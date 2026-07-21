//! Node `readline` module — a pragmatic, synchronous interface.
//!
//! Node's real `readline` is event-driven over a stream: it registers `'line'`,
//! `'close'`, and keypress handlers and fires them asynchronously as the input
//! stream produces data. node-js has no interactive stdin loop at this layer (the
//! event loop drives timers/promises/I-O-thread tasks, not a live TTY reader), so
//! this module implements the parts that CAN be honest without one:
//!
//! * REAL: `interface.question(query, cb)` writes `query` to stdout and reads
//!   exactly ONE line from stdin synchronously (`std::io::stdin().read_line`),
//!   then invokes `cb(line)` with the trimmed line. This is a genuine blocking
//!   read, matching the observable result of Node's `question` for a single
//!   prompt.
//! * REAL: `interface.write(data)` writes to stdout; `prompt()` writes the stored
//!   prompt; `setPrompt`/`getPrompt` manage it; the module cursor helpers
//!   (`cursorTo`/`moveCursor`/`clearLine`/`clearScreenDown`) emit the
//!   corresponding ANSI control sequences to stdout.
//! * NOT MODELED (documented, never faked): the asynchronous `'line'`/`'close'`
//!   event streaming. `interface.on('line', cb)` accepts and stores the listener
//!   (so it is not lost and chaining returns `this`), but node-js never
//!   asynchronously emits `'line'` — there is no background stdin reader. Use
//!   `question` for real line input. `close()`/`pause()`/`resume()` are no-ops.
//!
//! An Interface is a plain object tagged `@@native = "Interface"` carrying the
//! passed `@@input`/`@@output` streams (kept for fidelity; the real read/write
//! always uses process stdin/stdout), the current `@@prompt`, and a hidden
//! `@@listeners` object of registered event callbacks.

use crate::host::{is_callable, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::io::{self, Write};

pub const METHODS: &[&str] = &[
    "createInterface",
    "clearLine",
    "clearScreenDown",
    "cursorTo",
    "moveCursor",
    "emitKeypressEvents",
];

/// Methods dispatched on an `@@native = "Interface"` object (reported to the
/// parent for `instance_has_method` wiring).
pub const INTERFACE_METHODS: &[&str] = &[
    "question",
    "write",
    "close",
    "pause",
    "resume",
    "prompt",
    "setPrompt",
    "getPrompt",
    "on",
    "once",
    "addListener",
    "prependListener",
    "removeListener",
    "off",
    "removeAllListeners",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "createInterface" => Ok(create_interface(args)),
        // Cursor / line control: emit the ANSI sequence to stdout. Node passes the
        // target stream as the first arg; node-js writes to the real stdout (the
        // usual `process.stdout` target). Each returns `true` (write accepted).
        "cursorTo" => {
            let x = super::arg_num(args, 1);
            let y = args.get(2).filter(|v| !matches!(v, Value::Undef));
            let seq = match y {
                Some(yv) => format!(
                    "\x1b[{};{}H",
                    with_host(|h| h.to_number(yv)) as i64 + 1,
                    x as i64 + 1
                ),
                None => format!("\x1b[{}G", x as i64 + 1),
            };
            write_stdout(&seq);
            Ok(Value::Bool(true))
        }
        "moveCursor" => {
            let dx = super::arg_num(args, 1) as i64;
            let dy = super::arg_num(args, 2) as i64;
            let mut seq = String::new();
            if dx > 0 {
                seq.push_str(&format!("\x1b[{dx}C"));
            } else if dx < 0 {
                seq.push_str(&format!("\x1b[{}D", -dx));
            }
            if dy > 0 {
                seq.push_str(&format!("\x1b[{dy}B"));
            } else if dy < 0 {
                seq.push_str(&format!("\x1b[{}A", -dy));
            }
            write_stdout(&seq);
            Ok(Value::Bool(true))
        }
        "clearLine" => {
            let dir = super::arg_num(args, 1);
            // dir < 0 → to start (1K); dir > 0 → to end (0K); 0 → whole line (2K).
            let seq = if dir < 0.0 {
                "\x1b[1K"
            } else if dir > 0.0 {
                "\x1b[0K"
            } else {
                "\x1b[2K"
            };
            write_stdout(seq);
            Ok(Value::Bool(true))
        }
        "clearScreenDown" => {
            write_stdout("\x1b[0J");
            Ok(Value::Bool(true))
        }
        // `readline.emitKeypressEvents(stream)` normally attaches an input decoder
        // that makes `stream` emit `'keypress'` events. node-js has no background
        // TTY reader driving async input events (see the module docs), so there is
        // nothing to attach: an honest no-op rather than a fake key stream.
        "emitKeypressEvents" => Ok(Value::Undef),
        _ => return None,
    })
}

/// `new readline.Interface(options | input[, output])` — the class form of
/// `createInterface`, producing the same `@@native = "Interface"` object.
/// Requires the parent to route `"Interface"` construction into this fn.
pub fn construct(args: &[Value]) -> Result<Value, String> {
    Ok(create_interface(args))
}

/// A non-function member of the `readline` namespace (reachable via
/// `namespace_property` IF the parent routes `"readline"` into `stdlib::constant`).
/// `readline.Interface` is the interface constructor.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "Interface" => Some(with_host(|h| h.alloc(JsObj::Builtin("Interface".into())))),
        _ => None,
    }
}

/// `readline.createInterface(options | input[, output])` → an Interface object.
fn create_interface(args: &[Value]) -> Value {
    // Options object form `{ input, output }` vs positional `(input, output)`.
    let (input, output) = match args.first() {
        Some(o) if opt_prop(o, "input").is_some() => (
            opt_prop(o, "input").unwrap_or(Value::Undef),
            opt_prop(o, "output").unwrap_or(Value::Undef),
        ),
        _ => (
            args.first().cloned().unwrap_or(Value::Undef),
            args.get(1).cloned().unwrap_or(Value::Undef),
        ),
    };
    with_host(|h| {
        let listeners = h.new_object(IndexMap::new());
        let prompt = h.new_str("> ");
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Interface"));
        m.insert("@@input".into(), input);
        m.insert("@@output".into(), output);
        m.insert("@@prompt".into(), prompt);
        m.insert("@@listeners".into(), listeners);
        h.new_object(m)
    })
}

/// Dispatch a method on an Interface instance (`@@native = "Interface"`).
pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match method {
        // REAL synchronous single-line read: write the query, read one stdin line,
        // invoke the callback with it. Returns undefined (Node's callback form).
        "question" => {
            let query = with_host(|h| args.first().map(|v| h.str_of(v)).unwrap_or_default());
            write_stdout(&query);
            let line = read_line();
            // The callback is the last callable argument (Node: `question(q, cb)`
            // or `question(q, options, cb)`).
            // The `.find` predicate receives `&&Value`; deref once so `is_callable`
            // sees a `&Value`. `find` yields `Option<&Value>`, cloned to `Value`.
            let cb = args
                .iter()
                .rev()
                .find(|v| with_host(|h| is_callable(h, v)))
                .cloned();
            if let Some(cb) = cb {
                let line_val = with_host(|h| h.new_str(line));
                crate::host::invoke(&cb, vec![line_val], None)?;
            }
            Ok(Value::Undef)
        }
        "write" => {
            let data = with_host(|h| args.first().map(|v| h.str_of(v)).unwrap_or_default());
            write_stdout(&data);
            Ok(Value::Undef)
        }
        "prompt" => {
            let p = read_hidden(recv, "@@prompt");
            write_stdout(&p);
            Ok(Value::Undef)
        }
        "setPrompt" => {
            let p = with_host(|h| args.first().map(|v| h.str_of(v)).unwrap_or_default());
            with_host(|h| {
                let pv = h.new_str(p);
                if let Some(JsObj::Object(m)) = h.get_mut(recv) {
                    m.insert("@@prompt".into(), pv);
                }
            });
            Ok(Value::Undef)
        }
        "getPrompt" => Ok(with_host(|h| h.new_str(read_hidden(recv, "@@prompt")))),
        // Listener registration: stored under `@@listeners[event]` so it is not
        // lost and chaining returns `this`. node-js does NOT asynchronously emit
        // `'line'`/`'close'` (no background stdin reader) — use `question` for
        // real input.
        "on" | "once" | "addListener" | "prependListener" => {
            if let (Some(ev), Some(cb)) = (args.first(), args.get(1)) {
                let event = with_host(|h| h.str_of(ev));
                store_listener(recv, &event, cb.clone());
            }
            Ok(recv.clone())
        }
        "removeListener" | "off" | "removeAllListeners" => Ok(recv.clone()),
        // No interactive loop to tear down / pause; honest no-ops.
        "close" | "pause" | "resume" => Ok(Value::Undef),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// Read the `key` hidden string property of `recv`.
fn read_hidden(recv: &Value, key: &str) -> String {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).map(|v| h.str_of(v)).unwrap_or_default(),
        _ => String::new(),
    })
}

/// Append `cb` to `recv`'s `@@listeners[event]` array (created on demand).
fn store_listener(recv: &Value, event: &str, cb: Value) {
    let listeners = with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@listeners").cloned(),
        _ => None,
    });
    let Some(listeners) = listeners else { return };
    with_host(|h| {
        let arr = match h.get(&listeners) {
            Some(JsObj::Object(p)) => p.get(event).cloned(),
            _ => None,
        };
        let arr = arr.filter(|a| matches!(h.get(a), Some(JsObj::Array(_))));
        match arr {
            Some(a) => {
                if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                    items.push(cb);
                }
            }
            None => {
                let a = h.new_array(vec![cb]);
                if let Some(JsObj::Object(p)) = h.get_mut(&listeners) {
                    p.insert(event.to_string(), a);
                }
            }
        }
    });
}

/// Read one line from stdin, stripping the trailing CR/LF. EOF yields "".
fn read_line() -> String {
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    line
}

/// Write `s` to real stdout and flush (this is explicit program output — a
/// readline prompt / write — not informational chatter).
fn write_stdout(s: &str) {
    let mut out = io::stdout();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

/// An own property of `v` if `v` is a plain object, else `None`.
fn opt_prop(v: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}
