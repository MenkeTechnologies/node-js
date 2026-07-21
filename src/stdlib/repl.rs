//! Node `repl` module — `repl.start([options])`.
//!
//! node-js ALREADY has a real interactive REPL: `crate::repl::run()` (see
//! `src/repl.rs`), the reedline-based loop that `node --repl` (and bare `node`
//! on a TTY) drives from `main.rs`. It keeps one persistent host across lines,
//! accumulates continuation lines while delimiters stay open, and evaluates each
//! buffer via `crate::compile` + `crate::run_compiled`. `repl.start()` delegates
//! straight to that loop — it is the SAME real REPL, not a reimplementation.
//!
//! `repl.start()` blocks on stdin exactly like Node's: it hands control to the
//! interactive loop and only returns at EOF (Ctrl-D). This matches Node, where
//! `repl.start()` is normally the terminal action of a REPL-launcher script.
//!
//! LIMITATION (documented, never faked): `crate::repl::run()` calls
//! `host::reset_host()` and drives its own fresh persistent host, so lines typed
//! at the prompt do NOT see variables from the script that called `start()`, and
//! heap handles created before `start()` are not shared with the interactive
//! session. `start()` is therefore intended as the program's final statement
//! (the launcher pattern), which is how it is used in practice.
//!
//! The returned REPLServer is a plain object tagged `@@native = "REPLServer"`
//! carrying the resolved `prompt`/`input`/`output`/`useColors` options for
//! fidelity and a hidden `@@listeners` map. Its `close`/`on`/`once`/`write`/
//! `setPrompt` methods dispatch through `instance_call` — BUT ONLY IF the parent
//! `stdlib::mod` wires the `"REPLServer"` tag into `instance_has_method` +
//! `instance_call` (see the report). Because `start()` has already returned from
//! the (finished) interactive loop by the time these could be called, they are
//! best-effort post-hoc no-ops: `close` fires any stored `'exit'` listeners,
//! `on`/`once` store the listener and return `this`, `write` writes to stdout.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::io::{self, Write};

pub const METHODS: &[&str] = &["start", "isValidSyntax"];

/// Methods dispatched on an `@@native = "REPLServer"` object (reported to the
/// parent for `instance_has_method` / `instance_call` wiring). Without that
/// wiring a property read of these names yields `undefined`.
pub const REPLSERVER_METHODS: &[&str] = &[
    "close",
    "on",
    "once",
    "addListener",
    "prependListener",
    "removeListener",
    "off",
    "removeAllListeners",
    "write",
    "setPrompt",
    "displayPrompt",
    "defineCommand",
    "clearBufferedCommand",
    "pause",
    "resume",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        // Hand control to the real, existing interactive REPL loop. This blocks
        // until EOF, then returns a REPLServer-shaped object.
        "start" => {
            let opts = args.first().cloned().unwrap_or(Value::Undef);
            crate::repl::run();
            Ok(new_repl_server(&opts))
        }
        // `repl.isValidSyntax(code)` → whether `code` compiles cleanly. Real: it
        // runs `code` through node-js's own front end (`crate::compile`, the same
        // parser+compiler the module loader uses) and reports success/failure.
        "isValidSyntax" => Ok(Value::Bool(
            crate::compile(&super::arg_str(args, 0)).is_ok(),
        )),
        _ => return None,
    })
}

/// `new repl.REPLServer([options])` / `new repl.Recoverable(err)`.
///
/// * `REPLServer` — same object `start()` produces (see `new_repl_server`); the
///   constructor form does NOT auto-start the interactive loop (Node's does), so
///   this is a best-effort holder for the resolved options.
/// * `Recoverable` — Node wraps a syntax error the REPL should treat as "keep
///   reading more lines". We build a real `Error` (so `instanceof Error` holds)
///   carrying the original error as `.err`, matching Node's public shape.
pub fn construct(name: &str, args: &[Value]) -> Result<Value, String> {
    match name {
        "REPLServer" => Ok(new_repl_server(
            &args.first().cloned().unwrap_or(Value::Undef),
        )),
        "Recoverable" => {
            let inner = args.first().cloned().unwrap_or(Value::Undef);
            let msg = with_host(|h| h.str_of(&inner));
            let err =
                crate::builtins::construct_builtin("Error", vec![with_host(|h| h.new_str(msg))])?;
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get_mut(&err) {
                    p.insert("err".into(), inner);
                }
            });
            Ok(err)
        }
        _ => Err(crate::host::type_error(&format!(
            "repl.{name} is not a constructor"
        ))),
    }
}

/// A non-function member of the `repl` namespace, reachable via
/// `namespace_property` IF the parent routes `"repl"` into `stdlib::constant`.
///
/// * `repl.REPLServer` — the server class. node-js has no first-class exposed
///   REPLServer constructor (the server object is produced by `start()`), so
///   this is documented-only and returns `None`; use `repl.start()`.
/// * `repl.writer` — Node's default output formatter (`util.inspect`). We expose
///   it as the `util.inspect` builtin so `repl.writer(value)` formats identically
///   to the REPL's own result rendering.
///
/// Requires the parent to route `"repl"` into `stdlib::constant`.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "REPLServer" | "Recoverable" => Some(with_host(|h| h.alloc(JsObj::Builtin(name.into())))),
        "writer" => Some(with_host(|h| {
            h.alloc(JsObj::Builtin("util.inspect".into()))
        })),
        _ => None,
    }
}

/// Build the REPLServer object returned by `start()`. Plain object with a
/// `@@native = "REPLServer"` tag, the resolved options, and a `@@listeners` map.
fn new_repl_server(opts: &Value) -> Value {
    let prompt = opt_str(opts, "prompt").unwrap_or_else(|| "> ".to_string());
    let input = opt_prop(opts, "input").unwrap_or(Value::Undef);
    let output = opt_prop(opts, "output").unwrap_or(Value::Undef);
    let use_colors = opt_prop(opts, "useColors").unwrap_or(Value::Bool(true));
    with_host(|h| {
        let listeners = h.new_object(IndexMap::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("REPLServer"));
        let p = h.new_str(prompt);
        m.insert("@@prompt".into(), p);
        m.insert("input".into(), input);
        m.insert("output".into(), output);
        m.insert("useColors".into(), use_colors);
        m.insert("@@listeners".into(), listeners);
        h.new_object(m)
    })
}

/// Dispatch a method on a REPLServer instance (`@@native = "REPLServer"`).
/// The interactive loop has already exited by the time these run, so they are
/// best-effort. Requires parent wiring of the `"REPLServer"` tag to be reachable.
pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match method {
        // Fire any stored `'exit'` listeners, then resolve to undefined.
        "close" => {
            emit(recv, "exit", &[])?;
            Ok(Value::Undef)
        }
        "on" | "once" | "addListener" | "prependListener" => {
            if let (Some(ev), Some(cb)) = (args.first(), args.get(1)) {
                let event = with_host(|h| h.str_of(ev));
                store_listener(recv, &event, cb.clone());
            }
            Ok(recv.clone())
        }
        "removeListener" | "off" | "removeAllListeners" => Ok(recv.clone()),
        "write" => {
            let data = with_host(|h| args.first().map(|v| h.str_of(v)).unwrap_or_default());
            let mut out = io::stdout();
            let _ = out.write_all(data.as_bytes());
            let _ = out.flush();
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
            Ok(recv.clone())
        }
        // The loop is finished; these have nothing live to act on.
        "displayPrompt" | "clearBufferedCommand" | "pause" | "resume" => Ok(recv.clone()),
        // Accept a custom command definition without erroring (no live loop to
        // register it against). Returns the server for chaining.
        "defineCommand" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// Invoke every listener stored under `recv`'s `@@listeners[event]`.
fn emit(recv: &Value, event: &str, cb_args: &[Value]) -> Result<(), String> {
    let listeners = with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@listeners").cloned(),
        _ => None,
    });
    let Some(listeners) = listeners else {
        return Ok(());
    };
    // Snapshot the callbacks (release the host before invoking).
    let cbs: Vec<Value> = with_host(|h| match h.get(&listeners) {
        Some(JsObj::Object(p)) => match p.get(event).map(|a| h.get(a)) {
            Some(Some(JsObj::Array(items))) => items.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    });
    for cb in cbs {
        crate::host::invoke(&cb, cb_args.to_vec(), None)?;
    }
    Ok(())
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

/// An own property of `v` if `v` is a plain object, else `None`.
fn opt_prop(v: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

/// String value of an own property of `v` (if present and `v` is an object).
fn opt_str(v: &Value, key: &str) -> Option<String> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get(key).map(|pv| h.str_of(pv)),
        _ => None,
    })
}
