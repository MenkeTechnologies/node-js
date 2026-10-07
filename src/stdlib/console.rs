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
            // A throwing directive aborts the call BEFORE anything is written,
            // which is what node does: `console.log('%j', 1n)` prints no line.
            match format_args(args) {
                Ok(s) => {
                    emit(&s, false);
                    Ok(Value::Undef)
                }
                Err(e) => Err(e),
            }
        }
        "error" | "warn" => match format_args(args) {
            Ok(s) => {
                emit(&s, true);
                Ok(Value::Undef)
            }
            Err(e) => Err(e),
        },
        // `console.dir` renders its first argument through the same inspector,
        // ignoring the (rarely-used) options argument.
        "dir" => match crate::host::inspect_js(&args.first().cloned().unwrap_or(Value::Undef)) {
            Ok(s) => {
                emit(&s, false);
                Ok(Value::Undef)
            }
            Err(e) => Err(e),
        },
        // `console.trace` prints a "Trace:"-prefixed message to stderr. A full
        // captured stack is not attached here (no cheap synchronous stack source
        // at this layer); the message content matches Node.
        "trace" => {
            let msg = match format_args(args) {
                Ok(s) => s,
                Err(e) => return Some(Err(e)),
            };
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
                let msg = match format_args(&args[1.min(args.len())..]) {
                    Ok(s) => s,
                    Err(e) => return Some(Err(e)),
                };
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
                match format_args(args) {
                    Ok(s) => emit(&s, false),
                    Err(e) => return Some(Err(e)),
                }
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
                        match format_args(&args[1..]) {
                            Ok(s) => format!(" {s}"),
                            Err(e) => return Some(Err(e)),
                        }
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
                Err(e) => return Some(Err(e)),
                Ok(Some(t)) => emit(&t, false),
                // `this.log(tabularData)`: only the data, never `properties`.
                Ok(None) => match format_args(&args[..args.len().min(1)]) {
                    Ok(s) => emit(&s, false),
                    Err(e) => return Some(Err(e)),
                },
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
fn format_args(args: &[Value]) -> Result<String, String> {
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
    with_host(|h| h.write_out(&format!("{out}\n"), stderr));
}

// ── console.table ────────────────────────────────────────────────────────────
//
// A port of node's `Console.prototype.table` (lib/internal/console/
// constructor.js) and of `lib/internal/cli_table.js`, which draws the box.

/// The key/value pairs a Map-shaped `console.table` source is read as.
type TableEntries = Vec<(Value, Value)>;

/// `console.table(data[, properties])` as node renders it, or `Ok(None)` when
/// `data` is not an object — the caller then logs it as `console.log` would.
fn render_table(args: &[Value]) -> Result<Option<String>, String> {
    let data = args.first().cloned().unwrap_or(Value::Undef);
    let properties = args.get(1).cloned().unwrap_or(Value::Undef);
    let properties: Option<Vec<Value>> = match with_host(|h| h.get(&properties).cloned()) {
        _ if matches!(properties, Value::Undef) => None,
        Some(JsObj::Array(items)) => Some(items),
        _ => {
            return Err(crate::host::coded_error(
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                &format!(
                    "The \"properties\" argument must be an instance of Array. Received {}",
                    crate::stdlib::received_desc(&properties)
                ),
            ))
        }
    };
    if with_host(|h| crate::host::is_primitive(h, &data) || h.is_null(&data)) {
        return Ok(None);
    }
    let index_array = |n: usize| -> Result<Vec<Option<String>>, String> {
        (0..n)
            .map(|i| table_inspect(&Value::Float(i as f64)).map(Some))
            .collect()
    };

    // A Map, or a Map/Set iterator, is read without being consumed
    // (`previewEntries`); a Map entries iterator and a Map are key/value.
    let view = with_host(|h| crate::builtins::collection_iterator_view(h, &data));
    let (map_entries, set_values): (Option<TableEntries>, Option<Vec<Value>>) =
        match (view, with_host(|h| h.get(&data).cloned())) {
            (Some(("Map Entries", rest)), _) => (Some(rest), None),
            // `previewEntries` of a Set entries iterator is the flat
            // `[v, v, …]` list, each value twice.
            (Some(("Set Entries", rest)), _) => (
                None,
                Some(rest.into_iter().flat_map(|(k, v)| [k, v]).collect()),
            ),
            (Some((_, rest)), _) => (None, Some(rest.into_iter().map(|(k, _)| k).collect())),
            (None, Some(JsObj::Map { entries, .. })) => {
                (Some(entries.values().cloned().collect()), None)
            }
            (None, Some(JsObj::Set { entries, .. })) => {
                (None, Some(entries.values().cloned().collect()))
            }
            _ => (None, None),
        };
    if let Some(pairs) = map_entries {
        let mut keys = Vec::with_capacity(pairs.len());
        let mut values = Vec::with_capacity(pairs.len());
        for (k, v) in &pairs {
            keys.push(Some(table_inspect(k)?));
            values.push(Some(table_inspect(v)?));
        }
        let head = ["(iteration index)", "Key", "Values"].map(str::to_string);
        return Ok(Some(cli_table(
            &head,
            &[index_array(pairs.len())?, keys, values],
        )));
    }
    if let Some(items) = set_values {
        let values = items
            .iter()
            .map(|v| table_inspect(v).map(Some))
            .collect::<Result<Vec<_>, _>>()?;
        let head = ["(iteration index)", "Values"].map(str::to_string);
        return Ok(Some(cli_table(&head, &[index_array(items.len())?, values])));
    }

    // One column per key, in `ObjectKeys` order of first appearance; a row
    // that lacks the key leaves its cell empty.
    let index_keys = object_keys(&data)?;
    let mut map: Vec<(String, Vec<Option<String>>)> = Vec::new();
    let mut has_primitives = false;
    let mut values_column: Vec<Option<String>> = Vec::new();
    for (i, k) in index_keys.iter().enumerate() {
        let item = crate::builtins::get_property(&data, k)?;
        let primitive = with_host(|h| h.is_nullish(&item) || crate::host::is_primitive(h, &item));
        if properties.is_none() && primitive {
            has_primitives = true;
            set_cell(&mut values_column, i, table_inspect(&item)?);
            continue;
        }
        let keys: Vec<String> = match &properties {
            Some(p) => p
                .iter()
                .map(crate::host::to_property_key)
                .collect::<Result<_, _>>()?,
            None => object_keys(&item)?,
        };
        for key in keys {
            let pos = match map.iter().position(|(k, _)| *k == key) {
                Some(p) => p,
                None => {
                    map.push((key.clone(), Vec::new()));
                    map.len() - 1
                }
            };
            let cell = if (primitive && properties.is_some()) || !has_own(&item, &key)? {
                String::new()
            } else {
                table_inspect(&crate::builtins::get_property(&item, &key)?)?
            };
            set_cell(&mut map[pos].1, i, cell);
        }
    }
    // `ObjectKeys(map)` of the null-prototype accumulator: integer keys first.
    let (mut ordered, rest): (Vec<_>, Vec<_>) = map
        .into_iter()
        .partition(|(k, _)| crate::host::array_index(k).is_some());
    ordered.sort_by_key(|(k, _)| crate::host::array_index(k));
    ordered.extend(rest);
    let mut head = vec!["(index)".to_string()];
    let mut columns = vec![index_keys.into_iter().map(Some).collect::<Vec<_>>()];
    for (k, col) in ordered {
        head.push(k);
        columns.push(col);
    }
    if has_primitives {
        head.push("Values".to_string());
        columns.push(values_column);
    }
    Ok(Some(cli_table(&head, &columns)))
}

/// `map[key][i] = cell`: a sparse column grows to `i + 1`, its gaps holes.
fn set_cell(column: &mut Vec<Option<String>>, i: usize, cell: String) {
    if column.len() <= i {
        column.resize(i + 1, None);
    }
    column[i] = Some(cell);
}

/// `ObjectKeys(v)` as strings.
fn object_keys(v: &Value) -> Result<Vec<String>, String> {
    let keys = crate::builtins::call_builtin_function("Object.keys", vec![v.clone()])?;
    Ok(with_host(|h| match h.get(&keys) {
        Some(JsObj::Array(items)) => items.iter().map(|k| h.str_of(k)).collect(),
        _ => Vec::new(),
    }))
}

fn has_own(v: &Value, key: &str) -> Result<bool, String> {
    let k = with_host(|h| h.new_str(key.to_string()));
    let r = crate::builtins::object_builtin_method(v, "hasOwnProperty", vec![k])?;
    Ok(with_host(|h| h.truthy(&r)))
}

/// The table's `_inspect`: an object with more than two own keys collapses to
/// `[Object]` (depth -1), everything else shows one level; at most three array
/// items; never wrapped.
fn table_inspect(v: &Value) -> Result<String, String> {
    // node's `isArray` here also admits typed arrays and Buffers.
    let view = matches!(
        crate::stdlib::native_tag(v).as_deref(),
        Some("TypedArray" | "Buffer")
    );
    let collapse = !view
        && with_host(|h| {
            !crate::host::is_primitive(h, v)
                && !h.is_nullish(v)
                && !matches!(h.get(v), Some(JsObj::Array(_)))
        })
        && object_keys(v)?.len() > 2;
    let opts = with_host(|h| {
        let mut m: IndexMap<String, Value> = IndexMap::new();
        m.insert(
            "depth".into(),
            Value::Float(if collapse { -1.0 } else { 0.0 }),
        );
        m.insert("maxArrayLength".into(), Value::Float(3.0));
        m.insert("breakLength".into(), Value::Float(f64::INFINITY));
        h.new_object(m)
    });
    let out = crate::stdlib::util::call("inspect", &[v.clone(), opts])
        .unwrap_or_else(|| Ok(with_host(|h| h.new_str(h.inspect(v)))))?;
    Ok(with_host(|h| h.str_of(&out)))
}

/// node's `getStringWidth` (the build without ICU data, which is the one
/// written out in `lib/internal/util/inspect.js`): VT escapes removed, NFC,
/// then one column per code point except the zero-width ones, two for the
/// East Asian full-width ranges.
fn string_width(s: &str) -> usize {
    use unicode_normalization::UnicodeNormalization;
    let s: String = crate::stdlib::util::strip_vt(s).nfc().collect();
    s.chars()
        .map(|c| {
            let code = c as u32;
            if is_full_width(code) {
                2
            } else if is_zero_width(code) {
                0
            } else {
                1
            }
        })
        .sum()
}

fn is_zero_width(code: u32) -> bool {
    code <= 0x1F
        || (0x7F..=0x9F).contains(&code)
        || (0x300..=0x36F).contains(&code)
        || (0x200B..=0x200F).contains(&code)
        || (0x20D0..=0x20FF).contains(&code)
        || (0xFE00..=0xFE0F).contains(&code)
        || (0xFE20..=0xFE2F).contains(&code)
        || (0xE0100..=0xE01EF).contains(&code)
}

fn is_full_width(code: u32) -> bool {
    code >= 0x1100
        && (code <= 0x115f
            || code == 0x2329
            || code == 0x232a
            || ((0x2e80..=0x3247).contains(&code) && code != 0x303f)
            || (0x3250..=0x4dbf).contains(&code)
            || (0x4e00..=0xa4c6).contains(&code)
            || (0xa960..=0xa97c).contains(&code)
            || (0xac00..=0xd7a3).contains(&code)
            || (0xf900..=0xfaff).contains(&code)
            || (0xfe10..=0xfe19).contains(&code)
            || (0xfe30..=0xfe6b).contains(&code)
            || (0xff01..=0xff60).contains(&code)
            || (0xffe0..=0xffe6).contains(&code)
            || (0x1b000..=0x1b001).contains(&code)
            || (0x1f200..=0x1f251).contains(&code)
            || (0x1f300..=0x1f64f).contains(&code)
            || (0x20000..=0x3fffd).contains(&code))
}

/// `cli_table(head, columns)`: every column as wide as its widest cell, each
/// cell LEFT-justified, a missing cell (a hole, or past the column's end)
/// blank.
fn cli_table(head: &[String], columns: &[Vec<Option<String>>]) -> String {
    let longest = columns.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths: Vec<usize> = head.iter().map(|h| string_width(h)).collect();
    let mut rows: Vec<Vec<String>> = vec![Vec::with_capacity(head.len()); longest];
    for (i, column) in columns.iter().enumerate() {
        for (j, row) in rows.iter_mut().enumerate() {
            let value = column.get(j).cloned().flatten().unwrap_or_default();
            widths[i] = widths[i].max(string_width(&value));
            row.push(value);
        }
    }
    let render_row = |row: &[String]| -> String {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, w)| format!("{cell}{}", " ".repeat(w.saturating_sub(string_width(cell)))))
            .collect();
        format!("│ {} │", cells.join(" │ "))
    };
    let divider: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
    let mut out = format!(
        "┌{}┐\n{}\n├{}┤\n",
        divider.join("┬"),
        render_row(head),
        divider.join("┼")
    );
    for row in &rows {
        out.push_str(&render_row(row));
        out.push('\n');
    }
    out.push_str(&format!("└{}┘", divider.join("┴")));
    out
}
