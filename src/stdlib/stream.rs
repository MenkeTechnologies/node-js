//! Node `stream` module: native base classes + module helper functions.
//!
//! The base classes (`Readable`/`Writable`/`Duplex`/`Transform`/`PassThrough`/
//! `Stream`) are EventEmitter-backed objects (same `@@native`/`@@on`/`@@once`
//! shape as `net` sockets) exposing the surface higher layers touch today:
//! `on`/`once`/`emit`, `write`/`end`, `push`/`read`, and a best-effort `pipe`.
//! `http`'s `req`/`res` are their own native objects (see `http.rs`); this module
//! exists so `require('stream')` yields the base constructors and the module
//! helper functions (`finished`, `pipeline`, `isReadable`, …).
//!
//! Lifecycle state is tracked with hidden boolean props set as terminal events
//! fire: `@@ended` (readable end), `@@finished` (writable finish), `@@destroyed`
//! (close/destroy), `@@errored` (error value), `@@disturbed` (read/resume/pipe).
//! `finished(stream, cb)` callbacks live in a `@@finished` array drained on the
//! first terminal event so the callback fires exactly once.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::Cell;

// Module-state default high-water marks (byte mode / object mode). Node v26
// defaults: 65536 bytes, 16 objects; `setDefaultHighWaterMark` mutates these.
thread_local! {
    static DEFAULT_HWM_BYTES: Cell<f64> = const { Cell::new(65536.0) };
    static DEFAULT_HWM_OBJ: Cell<f64> = const { Cell::new(16.0) };
}

/// The base classes exported by `require('stream')`.
pub const CLASSES: &[&str] = &[
    "Readable",
    "Writable",
    "Duplex",
    "Transform",
    "PassThrough",
    "Stream",
];

/// The module free-functions exported by `require('stream')`.
pub const METHODS: &[&str] = &[
    "finished",
    "pipeline",
    "addAbortSignal",
    "destroy",
    "isReadable",
    "isWritable",
    "isErrored",
    "isDestroyed",
    "isDisturbed",
    "getDefaultHighWaterMark",
    "setDefaultHighWaterMark",
];

/// True if `name` is one of the stream base-class constructors.
pub fn is_class(name: &str) -> bool {
    CLASSES.contains(&name)
}

/// `stream.<Class>` property (a constructor value), reachable via
/// `namespace_property` → `stdlib::constant`.
pub fn constant(name: &str) -> Option<Value> {
    if is_class(name) {
        Some(with_host(|h| h.alloc(JsObj::Builtin(name.to_string()))))
    } else {
        None
    }
}

/// `new Readable()` / `Writable` / `Duplex` / `Transform` / `PassThrough` /
/// `Stream`.
pub fn construct(name: &str) -> Value {
    // A `push`ed-data queue lives on the object as an array for `read`.
    let mut extra = IndexMap::new();
    let queue = with_host(|h| h.new_array(Vec::new()));
    extra.insert("@@queue".into(), queue);
    super::net::new_emitter_object(name, extra)
}

/// Module free-function dispatch (`stream.finished`, `stream.isReadable`, …).
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let s0 = || args.first().cloned().unwrap_or(Value::Undef);
    Some(match method {
        "getDefaultHighWaterMark" => Ok(get_default_hwm(args)),
        "setDefaultHighWaterMark" => Ok(set_default_hwm(args)),
        "isReadable" => Ok(Value::Bool(is_readable(&s0()))),
        "isWritable" => Ok(Value::Bool(is_writable(&s0()))),
        "isErrored" => Ok(Value::Bool(flag(&s0(), "@@errored"))),
        "isDestroyed" => Ok(Value::Bool(flag(&s0(), "@@destroyed"))),
        "isDisturbed" => Ok(Value::Bool(flag(&s0(), "@@disturbed"))),
        "destroy" => Ok(destroy_stream(args)),
        "finished" => Ok(finished(args)),
        "pipeline" => pipeline(args),
        "addAbortSignal" => Ok(add_abort_signal(args)),
        _ => return None,
    })
}

fn get_default_hwm(args: &[Value]) -> Value {
    let obj = args
        .first()
        .map(|v| with_host(|h| h.truthy(v)))
        .unwrap_or(false);
    let n = if obj {
        DEFAULT_HWM_OBJ.with(|c| c.get())
    } else {
        DEFAULT_HWM_BYTES.with(|c| c.get())
    };
    Value::Float(n)
}

fn set_default_hwm(args: &[Value]) -> Value {
    let obj = args
        .first()
        .map(|v| with_host(|h| h.truthy(v)))
        .unwrap_or(false);
    let val = super::arg_num(args, 1);
    if obj {
        DEFAULT_HWM_OBJ.with(|c| c.set(val));
    } else {
        DEFAULT_HWM_BYTES.with(|c| c.set(val));
    }
    Value::Undef
}

// ── lifecycle-flag helpers ──────────────────────────────────────────────────

fn tag_of(recv: &Value) -> Option<String> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@native").map(|v| h.str_of(v)),
        _ => None,
    })
}

fn flag(recv: &Value, key: &str) -> bool {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).map(|v| h.truthy(v)).unwrap_or(false),
        _ => false,
    })
}

fn set_flag(recv: &Value, key: &str, v: Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), v);
        }
    });
}

fn is_readable(s: &Value) -> bool {
    let Some(t) = tag_of(s) else { return false };
    matches!(
        t.as_str(),
        "Readable" | "Duplex" | "Transform" | "PassThrough"
    ) && !flag(s, "@@destroyed")
        && !flag(s, "@@ended")
}

fn is_writable(s: &Value) -> bool {
    let Some(t) = tag_of(s) else { return false };
    matches!(
        t.as_str(),
        "Writable" | "Duplex" | "Transform" | "PassThrough"
    ) && !flag(s, "@@destroyed")
        && !flag(s, "@@finished")
}

// ── `finished` callback registry ────────────────────────────────────────────

fn add_finished(recv: &Value, cb: Value) {
    with_host(|h| {
        let existing = match h.get(recv) {
            Some(JsObj::Object(p)) => p.get("@@finished").cloned(),
            _ => None,
        };
        let arr = match existing {
            Some(a) if matches!(h.get(&a), Some(JsObj::Array(_))) => a,
            _ => {
                let a = h.new_array(Vec::new());
                if let Some(JsObj::Object(p)) = h.get_mut(recv) {
                    p.insert("@@finished".into(), a.clone());
                }
                a
            }
        };
        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
            items.push(cb);
        }
    });
}

fn take_finished(recv: &Value) -> Vec<Value> {
    with_host(|h| {
        let arr = match h.get_mut(recv) {
            Some(JsObj::Object(p)) => p.shift_remove("@@finished"),
            _ => None,
        };
        match arr {
            Some(av) => match h.get(&av) {
                Some(JsObj::Array(items)) => items.clone(),
                _ => Vec::new(),
            },
            None => Vec::new(),
        }
    })
}

/// Emit `name` (with `extra` args), set the matching lifecycle flag, and drain
/// `finished` callbacks on the first terminal event so each fires once.
fn emit_event(recv: &Value, name: &str, extra: Vec<Value>) -> Result<Value, String> {
    let mut a = vec![with_host(|h| h.new_str(name))];
    a.extend(extra.iter().cloned());
    let r = super::events::instance_call(recv, "emit", a)?;
    match name {
        "end" => set_flag(recv, "@@ended", Value::Bool(true)),
        "finish" => set_flag(recv, "@@finished", Value::Bool(true)),
        "close" => set_flag(recv, "@@destroyed", Value::Bool(true)),
        "error" => set_flag(
            recv,
            "@@errored",
            extra.first().cloned().unwrap_or(Value::Bool(true)),
        ),
        _ => {}
    }
    if matches!(name, "end" | "finish" | "close" | "error") {
        let cbs = take_finished(recv);
        let arg = if name == "error" {
            extra.first().cloned().unwrap_or(Value::Undef)
        } else {
            Value::Undef
        };
        for cb in cbs {
            crate::host::invoke(&cb, vec![arg.clone()], None)?;
        }
    }
    Ok(r)
}

// ── module free functions ───────────────────────────────────────────────────

/// `stream.finished(stream[, options], callback)` — invoke `callback(err)` once
/// when the stream ends/finishes/closes/errors. Fires immediately if the stream
/// has already reached a terminal state. Returns `undefined` (Node returns a
/// cleanup fn; not tracked — best-effort).
fn finished(args: &[Value]) -> Value {
    let stream = args.first().cloned().unwrap_or(Value::Undef);
    let cb = args
        .iter()
        .rev()
        .find(|v| with_host(|h| crate::host::is_callable(h, v)))
        .cloned()
        .unwrap_or(Value::Undef);
    if flag(&stream, "@@ended") || flag(&stream, "@@finished") || flag(&stream, "@@destroyed") {
        let _ = crate::host::invoke(&cb, vec![Value::Undef], None);
    } else {
        add_finished(&stream, cb);
    }
    Value::Undef
}

/// `stream.pipeline(source, ...transforms, dest[, callback])` — chain via
/// `.pipe()` and register `callback` on the destination's completion. Returns
/// the destination stream.
fn pipeline(args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err(crate::host::type_error(
            "pipeline requires at least one stream",
        ));
    }
    let cb_idx = args
        .iter()
        .rposition(|v| with_host(|h| crate::host::is_callable(h, v)));
    let (streams, cb) = match cb_idx {
        Some(i) if i == args.len() - 1 => (&args[..i], Some(args[i].clone())),
        _ => (args, None),
    };
    for w in streams.windows(2) {
        crate::host::call_method(&w[0], "pipe", vec![w[1].clone()])?;
    }
    let last = streams.last().cloned().unwrap_or(Value::Undef);
    if let Some(cb) = cb {
        add_finished(&last, cb);
    }
    Ok(last)
}

/// `stream.destroy(stream[, err])` — emit `error` (if `err` given) then `close`
/// and mark the stream destroyed.
fn destroy_stream(args: &[Value]) -> Value {
    let stream = args.first().cloned().unwrap_or(Value::Undef);
    if flag(&stream, "@@destroyed") {
        return stream;
    }
    if let Some(e) = args.get(1).cloned() {
        if !with_host(|h| h.is_nullish(&e)) {
            let _ = emit_event(&stream, "error", vec![e]);
        }
    }
    let _ = emit_event(&stream, "close", vec![]);
    set_flag(&stream, "@@destroyed", Value::Bool(true));
    stream
}

/// `stream.addAbortSignal(signal, stream)` — best-effort: `AbortSignal` is not
/// modeled in this runtime, so this returns `stream` unchanged.
fn add_abort_signal(args: &[Value]) -> Value {
    args.get(1).cloned().unwrap_or(Value::Undef)
}

/// Instance dispatch for a stream base class. EventEmitter methods are delegated
/// to `events`; `emit` routes through `emit_event` for lifecycle tracking.
pub fn instance_call(
    tag: &str,
    recv: &Value,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    let _ = tag;
    if method == "emit" {
        let name = args
            .first()
            .map(|v| with_host(|h| h.str_of(v)))
            .unwrap_or_default();
        let extra = args.get(1..).map(|s| s.to_vec()).unwrap_or_default();
        return emit_event(recv, &name, extra);
    }
    if matches!(
        method,
        "on" | "addListener"
            | "prependListener"
            | "once"
            | "prependOnceListener"
            | "removeListener"
            | "off"
            | "removeAllListeners"
            | "listenerCount"
            | "eventNames"
    ) {
        return super::events::instance_call(recv, method, args);
    }
    match method {
        "write" => {
            let chunk = args.first().cloned().unwrap_or(Value::Undef);
            emit_event(recv, "data", vec![chunk])?;
            Ok(Value::Bool(true))
        }
        "end" => {
            if let Some(chunk) = args.first().filter(|v| !matches!(v, Value::Undef)) {
                emit_event(recv, "data", vec![chunk.clone()])?;
            }
            emit_event(recv, "finish", vec![])?;
            emit_event(recv, "end", vec![])?;
            Ok(recv.clone())
        }
        "push" => {
            let chunk = args.first().cloned().unwrap_or(Value::Undef);
            if with_host(|h| h.is_nullish(&chunk)) {
                emit_event(recv, "end", vec![])?;
                return Ok(Value::Bool(false));
            }
            if let Some(q) = queue_of(recv) {
                with_host(|h| {
                    if let Some(JsObj::Array(items)) = h.get_mut(&q) {
                        items.push(chunk.clone());
                    }
                });
            }
            emit_event(recv, "data", vec![chunk])?;
            Ok(Value::Bool(true))
        }
        "read" => {
            set_flag(recv, "@@disturbed", Value::Bool(true));
            if let Some(q) = queue_of(recv) {
                let next = with_host(|h| match h.get_mut(&q) {
                    Some(JsObj::Array(items)) if !items.is_empty() => Some(items.remove(0)),
                    _ => None,
                });
                if let Some(v) = next {
                    return Ok(v);
                }
            }
            Ok(with_host(|h| h.null()))
        }
        "pipe" => {
            set_flag(recv, "@@disturbed", Value::Bool(true));
            let dest = args.first().cloned().unwrap_or(Value::Undef);
            if let Some(q) = queue_of(recv) {
                let items = with_host(|h| match h.get(&q) {
                    Some(JsObj::Array(items)) => items.clone(),
                    _ => Vec::new(),
                });
                for chunk in items {
                    crate::host::call_method(&dest, "write", vec![chunk])?;
                }
            }
            Ok(dest)
        }
        "destroy" => {
            if !flag(recv, "@@destroyed") {
                if let Some(e) = args.first().filter(|v| !matches!(v, Value::Undef)) {
                    let _ = emit_event(recv, "error", vec![e.clone()]);
                }
                let _ = emit_event(recv, "close", vec![]);
                set_flag(recv, "@@destroyed", Value::Bool(true));
            }
            Ok(recv.clone())
        }
        "resume" => {
            set_flag(recv, "@@disturbed", Value::Bool(true));
            Ok(recv.clone())
        }
        "setEncoding" | "pause" | "cork" | "uncork" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!(
            "stream.{method} is not a function"
        ))),
    }
}

fn queue_of(recv: &Value) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@queue").cloned(),
        _ => None,
    })
}
