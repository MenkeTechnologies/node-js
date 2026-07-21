//! Node `console` module (`require('console')`), sharing the exact rendering the
//! global `console.*` uses: every argument is run through `JsHost::console_format`
//! (strings verbatim, everything else via `util.inspect`) and space-joined — the
//! same pipeline `builtins::print_line` drives — so module output is identical to
//! the global. `log`/`info`/`debug` go to stdout; `error`/`warn`/`trace`/`assert`
//! to stderr. `count`, `group` and `time` keep per-thread state here (a monotonic
//! `Instant` backs the timers, so `timeEnd` reports a real elapsed duration).

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::IsTerminal;
use std::time::Instant;

pub const METHODS: &[&str] = &[
    "log",
    "info",
    "debug",
    "error",
    "warn",
    "dir",
    "dirxml",
    "trace",
    "assert",
    "count",
    "countReset",
    "group",
    "groupCollapsed",
    "groupEnd",
    "time",
    "timeEnd",
    "timeLog",
    "table",
    "clear",
    "timeStamp",
    "profile",
    "profileEnd",
];

/// The instance method names for a `console.Console` object (`@@native = "Console"`),
/// wired by the parent `mod.rs`. Identical to the free-function surface.
pub const CONSOLE_METHODS: &[&str] = METHODS;

thread_local! {
    /// Current `console.group` nesting depth (2 spaces per level).
    static GROUP_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// `console.count` label → invocation tally.
    static COUNTS: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
    /// `console.time` label → start instant.
    static TIMERS: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
    /// Active output sink `(stdout, stderr)` for a `Console` instance call. When
    /// set, `emit` writes formatted lines to these streams instead of the process
    /// std streams, so a `new Console({stdout, stderr})` honors custom writables.
    static SINK: RefCell<Option<(Value, Value)>> = const { RefCell::new(None) };
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "log" | "info" | "debug" | "dirxml" => {
            emit(&format_args(args), false);
            Ok(Value::Undef)
        }
        "error" | "warn" => {
            emit(&format_args(args), true);
            Ok(Value::Undef)
        }
        // `console.dir` renders its first argument through the same inspector,
        // ignoring the (rarely-used) options argument.
        "dir" => {
            let s = with_host(|h| h.inspect(&args.first().cloned().unwrap_or(Value::Undef)));
            emit(&s, false);
            Ok(Value::Undef)
        }
        // `console.trace` prints a "Trace:"-prefixed message to stderr. A full
        // captured stack is not attached here (no cheap synchronous stack source
        // at this layer); the message content matches Node.
        "trace" => {
            let msg = format_args(args);
            let line = if msg.is_empty() {
                "Trace".to_string()
            } else {
                format!("Trace: {msg}")
            };
            emit(&line, true);
            Ok(Value::Undef)
        }
        // `console.assert(cond, ...msg)`: on a falsy condition, write
        // "Assertion failed" (plus any message) to stderr; otherwise no output.
        "assert" => {
            let ok = with_host(|h| h.truthy(&args.first().cloned().unwrap_or(Value::Undef)));
            if !ok {
                let msg = format_args(&args[1.min(args.len())..]);
                let line = if msg.is_empty() {
                    "Assertion failed".to_string()
                } else {
                    format!("Assertion failed: {msg}")
                };
                emit(&line, true);
            }
            Ok(Value::Undef)
        }
        "count" => {
            let label = label_arg(args, "default");
            let n = COUNTS.with(|c| {
                let mut m = c.borrow_mut();
                let e = m.entry(label.clone()).or_insert(0);
                *e += 1;
                *e
            });
            emit(&format!("{label}: {n}"), false);
            Ok(Value::Undef)
        }
        "countReset" => {
            let label = label_arg(args, "default");
            COUNTS.with(|c| c.borrow_mut().remove(&label));
            Ok(Value::Undef)
        }
        // `group`/`groupCollapsed` print their label (if any) then indent all
        // subsequent output one level; `groupEnd` pops a level.
        "group" | "groupCollapsed" => {
            if !args.is_empty() {
                emit(&format_args(args), false);
            }
            GROUP_DEPTH.with(|d| d.set(d.get() + 1));
            Ok(Value::Undef)
        }
        "groupEnd" => {
            GROUP_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
            Ok(Value::Undef)
        }
        "time" => {
            let label = label_arg(args, "default");
            TIMERS.with(|t| t.borrow_mut().insert(label, Instant::now()));
            Ok(Value::Undef)
        }
        "timeEnd" | "timeLog" => {
            let label = label_arg(args, "default");
            let elapsed = TIMERS.with(|t| {
                let m = t.borrow();
                m.get(&label).map(|start| start.elapsed())
            });
            match elapsed {
                Some(d) => {
                    let ms = d.as_secs_f64() * 1000.0;
                    // Any extra args after the label are appended, matching Node.
                    let extra = if args.len() > 1 {
                        format!(" {}", format_args(&args[1..]))
                    } else {
                        String::new()
                    };
                    emit(&format!("{label}: {ms:.3}ms{extra}"), false);
                    if method == "timeEnd" {
                        TIMERS.with(|t| t.borrow_mut().remove(&label));
                    }
                }
                None => emit(
                    &format!("Warning: No such label '{label}' for console.{method}()"),
                    true,
                ),
            }
            Ok(Value::Undef)
        }
        // `console.table(data[, properties])`: render an ASCII box table. Non-tabular
        // input (a primitive, a function, …) falls back to `console.log`.
        "table" => {
            match render_table(args) {
                Some(t) => emit(&t, false),
                None => emit(&format_args(args), false),
            }
            Ok(Value::Undef)
        }
        // `console.clear()`: emit the clear-screen sequence only to a TTY (a no-op
        // when output is redirected), matching Node.
        "clear" => {
            let is_tty = SINK.with(|s| s.borrow().is_some()) || std::io::stdout().is_terminal();
            if is_tty {
                emit("\u{1b}[2J\u{1b}[0f", false);
            }
            Ok(Value::Undef)
        }
        // Devtools-only timeline hooks — no timeline here, so no-ops (as in Node
        // when not under an inspector).
        "timeStamp" | "profile" | "profileEnd" => Ok(Value::Undef),
        _ => return None,
    })
}

/// `new console.Console(stdout[, stderr])` or `new console.Console({stdout, stderr})`
/// → an object tagged `@@native = "Console"` carrying its target streams. Parent
/// `mod.rs` wires construction and `instance_call`.
pub fn construct(args: &[Value]) -> Result<Value, String> {
    let first = args.first().cloned().unwrap_or(Value::Undef);
    // Options form: a plain object exposing a `stdout` property.
    let is_options = with_host(|h| match h.get(&first) {
        Some(JsObj::Object(m)) => m.contains_key("stdout"),
        _ => false,
    });
    let (stdout, stderr) = if is_options {
        let out = crate::builtins::get_property(&first, "stdout").unwrap_or(Value::Undef);
        let err = match crate::builtins::get_property(&first, "stderr") {
            Ok(Value::Undef) | Err(_) => out.clone(),
            Ok(v) => v,
        };
        (out, err)
    } else {
        let err = args
            .get(1)
            .cloned()
            .filter(|v| !matches!(v, Value::Undef))
            .unwrap_or_else(|| first.clone());
        (first, err)
    };
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Console"));
        m.insert("@@stdout".into(), stdout);
        m.insert("@@stderr".into(), stderr);
        h.new_object(m)
    }))
}

/// Dispatch a method on a `Console` instance: install the instance's streams as the
/// active output sink, run the same formatting/state logic the free functions use,
/// then restore the previous sink.
pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    let streams = with_host(|h| match h.get(recv) {
        Some(JsObj::Object(m)) => Some((
            m.get("@@stdout").cloned().unwrap_or(Value::Undef),
            m.get("@@stderr").cloned().unwrap_or(Value::Undef),
        )),
        _ => None,
    });
    let prev = SINK.with(|s| s.borrow_mut().take());
    SINK.with(|s| *s.borrow_mut() = streams);
    let r = call(method, &args).unwrap_or(Ok(Value::Undef));
    SINK.with(|s| *s.borrow_mut() = prev);
    r
}

/// Space-join every argument through the shared console formatter (identical to
/// the global `console.log` path in `builtins::print_line`).
fn format_args(args: &[Value]) -> String {
    // console.log(...args) === util.format(...args): printf substitution when the
    // first arg is a format string, else inspect-and-join.
    super::util::format(args)
}

/// The label argument (`args[0]`) as a string, or `fallback` when absent.
fn label_arg(args: &[Value], fallback: &str) -> String {
    match args.first() {
        Some(v) => with_host(|h| h.str_of(v)),
        None => fallback.to_string(),
    }
}

/// Write a line with the current `console.group` indentation applied to every
/// physical line. Routes to the active `Console` instance sink stream if one is
/// installed, else to the process stdout/stderr.
fn emit(line: &str, stderr: bool) {
    let depth = GROUP_DEPTH.with(|d| d.get());
    let out = if depth == 0 {
        line.to_string()
    } else {
        let pad = "  ".repeat(depth);
        format!("{pad}{}", line.replace('\n', &format!("\n{pad}")))
    };
    // A `Console` instance with a real (heap-object) stream: write through it.
    let stream = SINK.with(|s| {
        s.borrow().as_ref().and_then(|(o, e)| {
            let target = if stderr { e } else { o };
            matches!(target, Value::Obj(_)).then(|| target.clone())
        })
    });
    if let Some(stream) = stream {
        let payload = with_host(|h| h.new_str(format!("{out}\n")));
        if crate::host::call_method(&stream, "write", vec![payload]).is_ok() {
            return;
        }
    }
    if stderr {
        eprintln!("{out}");
    } else {
        println!("{out}");
    }
}

// ── console.table ────────────────────────────────────────────────────────────

/// Render `console.table(data[, properties])` as a box-drawn ASCII table, or
/// `None` when `data` is not a tabular value (array/object) — the caller then
/// falls back to `console.log`.
fn render_table(args: &[Value]) -> Option<String> {
    let data = args.first().cloned().unwrap_or(Value::Undef);
    let restrict: Option<Vec<String>> =
        with_host(|h| match h.get(args.get(1).unwrap_or(&Value::Undef)) {
            Some(JsObj::Array(items)) => Some(items.iter().map(|v| h.str_of(v)).collect()),
            _ => None,
        });
    // (index label, row value) for each row.
    let entries: Vec<(String, Value)> = with_host(|h| match h.get(&data) {
        Some(JsObj::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v.clone()))
            .collect(),
        Some(JsObj::Object(m)) => m
            .iter()
            .filter(|(k, _)| !k.starts_with("@@"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        _ => Vec::new(),
    });
    if with_host(|h| !matches!(h.get(&data), Some(JsObj::Array(_) | JsObj::Object(_)))) {
        return None;
    }

    // Discover columns (union of tabular rows' keys) and whether any row is a bare
    // value (needing the trailing "Values" column).
    let mut columns: Vec<String> = Vec::new();
    let mut has_values = false;
    for (_, val) in &entries {
        match row_keys(val) {
            Some(keys) => {
                for k in keys {
                    if !columns.contains(&k) {
                        columns.push(k);
                    }
                }
            }
            None => has_values = true,
        }
    }
    if let Some(r) = &restrict {
        columns = r.clone();
        has_values = false;
    }

    // Header + body as a grid of already-rendered cell strings.
    let mut header = Vec::with_capacity(columns.len() + 2);
    header.push("(index)".to_string());
    header.extend(columns.iter().cloned());
    if has_values {
        header.push("Values".to_string());
    }

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(entries.len());
    for (idx, val) in &entries {
        let is_primitive = row_keys(val).is_none();
        let mut row = Vec::with_capacity(header.len());
        row.push(idx.clone());
        for col in &columns {
            match row_get(val, col) {
                Some(cell) => row.push(with_host(|h| h.inspect(&cell))),
                None => row.push(String::new()),
            }
        }
        if has_values {
            row.push(if is_primitive {
                with_host(|h| h.inspect(val))
            } else {
                String::new()
            });
        }
        rows.push(row);
    }

    Some(draw_table(&header, &rows))
}

/// The own tabular keys of a row value (`Some` for arrays/objects), or `None` when
/// the row is a primitive (rendered in the "Values" column).
fn row_keys(val: &Value) -> Option<Vec<String>> {
    with_host(|h| match h.get(val) {
        Some(JsObj::Array(items)) => Some((0..items.len()).map(|i| i.to_string()).collect()),
        Some(JsObj::Object(m)) => {
            Some(m.keys().filter(|k| !k.starts_with("@@")).cloned().collect())
        }
        _ => None,
    })
}

/// Read column `key` from a row value, if present.
fn row_get(val: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(val) {
        Some(JsObj::Array(items)) => key
            .parse::<usize>()
            .ok()
            .and_then(|i| items.get(i).cloned()),
        Some(JsObj::Object(m)) => m.get(key).cloned(),
        _ => None,
    })
}

/// Draw the box-drawing table from a header row and body rows.
fn draw_table(header: &[String], rows: &[Vec<String>]) -> String {
    let ncols = header.len();
    let mut widths = vec![0usize; ncols];
    for (i, cell) in header.iter().enumerate() {
        widths[i] = cell.chars().count();
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

    let rule = |left: &str, mid: &str, right: &str| -> String {
        let mut s = String::from(left);
        for (i, w) in widths.iter().enumerate() {
            if i > 0 {
                s.push_str(mid);
            }
            s.push_str(&"─".repeat(w + 2));
        }
        s.push_str(right);
        s
    };
    let render_row = |cells: &[String]| -> String {
        let mut s = String::from("│");
        for (i, w) in widths.iter().enumerate() {
            let cell = cells.get(i).map(String::as_str).unwrap_or("");
            s.push(' ');
            s.push_str(&pad_center(cell, *w));
            s.push_str(" │");
        }
        s
    };

    let mut lines = Vec::with_capacity(rows.len() + 4);
    lines.push(rule("┌", "┬", "┐"));
    lines.push(render_row(header));
    lines.push(rule("├", "┼", "┤"));
    for row in rows {
        lines.push(render_row(row));
    }
    lines.push(rule("└", "┴", "┘"));
    lines.join("\n")
}

/// Center `s` within `w` columns (extra space biased to the right, as Node does).
fn pad_center(s: &str, w: usize) -> String {
    let len = s.chars().count();
    if len >= w {
        return s.to_string();
    }
    let total = w - len;
    let left = total / 2;
    let right = total - left;
    format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
}
