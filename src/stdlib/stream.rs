//! Node `stream` module: minimal native base classes.
//!
//! For M1 these are EventEmitter-backed objects (same `@@native`/`@@on`/`@@once`
//! shape as `net` sockets) exposing just the surface higher layers touch today:
//! `on`/`once`/`emit`, `write`/`end`, `push`/`read`, and a best-effort `pipe`.
//! `http`'s `req`/`res` are their own native objects for M1 (see `http.rs`); this
//! module exists so `require('stream')` yields the base constructors. Wave C
//! expands buffering/backpressure/flowing-mode semantics.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// The base classes exported by `require('stream')`.
pub const CLASSES: &[&str] = &["Readable", "Writable", "Duplex", "Transform"];

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

/// `new Readable()` / `new Writable()` / `new Duplex()` / `new Transform()`.
pub fn construct(name: &str) -> Value {
    // A `push`ed-data queue lives on the object as an array for `read`.
    let mut extra = IndexMap::new();
    let queue = with_host(|h| h.new_array(Vec::new()));
    extra.insert("@@queue".into(), queue);
    super::net::new_emitter_object(name, extra)
}

/// Instance dispatch for a stream base class (`Readable`/`Writable`/`Duplex`/
/// `Transform`). EventEmitter methods are delegated to `events`.
pub fn instance_call(tag: &str, recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if matches!(
        method,
        "on" | "addListener" | "prependListener" | "once" | "prependOnceListener" | "emit"
            | "removeListener" | "off" | "removeAllListeners" | "listenerCount" | "eventNames"
    ) {
        return super::events::instance_call(recv, method, args);
    }
    let _ = tag;
    match method {
        "write" => {
            // Emit the chunk to `data` listeners (flowing-mode best effort).
            let chunk = args.first().cloned().unwrap_or(Value::Undef);
            super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("data")), chunk])?;
            Ok(Value::Bool(true))
        }
        "end" => {
            if let Some(chunk) = args.first().filter(|v| !matches!(v, Value::Undef)) {
                super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("data")), chunk.clone()])?;
            }
            super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("finish"))])?;
            super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("end"))])?;
            Ok(recv.clone())
        }
        "push" => {
            let chunk = args.first().cloned().unwrap_or(Value::Undef);
            // `push(null)` signals EOF.
            if with_host(|h| h.is_nullish(&chunk)) {
                super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("end"))])?;
                return Ok(Value::Bool(false));
            }
            // Enqueue and emit `data` for flowing consumers.
            if let Some(q) = queue_of(recv) {
                with_host(|h| {
                    if let Some(JsObj::Array(items)) = h.get_mut(&q) {
                        items.push(chunk.clone());
                    }
                });
            }
            super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("data")), chunk])?;
            Ok(Value::Bool(true))
        }
        "read" => {
            // Return (and dequeue) the next buffered chunk, or null.
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
            // Best effort: forward this stream's `data`/`end` into `dest.write`/
            // `dest.end` by wiring listeners is non-trivial without JS closures;
            // for M1 pump whatever is already queued, then return `dest`.
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
        "setEncoding" | "pause" | "resume" | "cork" | "uncork" | "destroy" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!("stream.{method} is not a function"))),
    }
}

fn queue_of(recv: &Value) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@queue").cloned(),
        _ => None,
    })
}
