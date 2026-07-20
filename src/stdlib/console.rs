//! Node `console` module (`require('console')`), sharing the exact rendering the
//! global `console.*` uses: every argument is run through `JsHost::console_format`
//! (strings verbatim, everything else via `util.inspect`) and space-joined — the
//! same pipeline `builtins::print_line` drives — so module output is identical to
//! the global. `log`/`info`/`debug` go to stdout; `error`/`warn`/`trace`/`assert`
//! to stderr. `count`, `group` and `time` keep per-thread state here (a monotonic
//! `Instant` backs the timers, so `timeEnd` reports a real elapsed duration).

use crate::host::with_host;
use fusevm::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Instant;

pub const METHODS: &[&str] = &[
    "log",
    "info",
    "debug",
    "error",
    "warn",
    "dir",
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
];

thread_local! {
    /// Current `console.group` nesting depth (2 spaces per level).
    static GROUP_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// `console.count` label → invocation tally.
    static COUNTS: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
    /// `console.time` label → start instant.
    static TIMERS: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "log" | "info" | "debug" => {
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
            let line = if msg.is_empty() { "Trace".to_string() } else { format!("Trace: {msg}") };
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
                    let extra = if args.len() > 1 { format!(" {}", format_args(&args[1..])) } else { String::new() };
                    emit(&format!("{label}: {ms:.3}ms{extra}"), false);
                    if method == "timeEnd" {
                        TIMERS.with(|t| t.borrow_mut().remove(&label));
                    }
                }
                None => emit(&format!("Warning: No such label '{label}' for console.{method}()"), true),
            }
            Ok(Value::Undef)
        }
        _ => return None,
    })
}

/// Space-join every argument through the shared console formatter (identical to
/// the global `console.log` path in `builtins::print_line`).
fn format_args(args: &[Value]) -> String {
    with_host(|h| args.iter().map(|a| h.console_format(a)).collect::<Vec<_>>().join(" "))
}

/// The label argument (`args[0]`) as a string, or `fallback` when absent.
fn label_arg(args: &[Value], fallback: &str) -> String {
    match args.first() {
        Some(v) => with_host(|h| h.str_of(v)),
        None => fallback.to_string(),
    }
}

/// Write a line with the current `console.group` indentation applied to every
/// physical line, to stdout or stderr.
fn emit(line: &str, stderr: bool) {
    let depth = GROUP_DEPTH.with(|d| d.get());
    let out = if depth == 0 {
        line.to_string()
    } else {
        let pad = "  ".repeat(depth);
        format!("{pad}{}", line.replace('\n', &format!("\n{pad}")))
    };
    if stderr {
        eprintln!("{out}");
    } else {
        println!("{out}");
    }
}
