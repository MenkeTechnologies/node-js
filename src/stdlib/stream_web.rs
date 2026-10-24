//! Node `stream/web` module: the WHATWG Streams API.
//!
//! The implementation is `webstreams.js`, a port of node v26.12.1's
//! `lib/internal/webstreams/` (its files unchanged, behind a prelude that
//! supplies the `primordials` and internals they import). It is evaluated
//! once, on the first read of any stream class, and every class — the
//! `stream/web` member and the global — is the port's constructor, so streams
//! are ordinary script objects: the spec's promise-driven `start`/`pull`/
//! `cancel`, backpressure through `highWaterMark`/`desiredSize`, byte streams
//! with BYOB readers and `byobRequest`, `tee`, and `pipeTo`/`pipeThrough`
//! with `signal`/`preventClose`/`preventAbort`/`preventCancel` are node's
//! code.
//!
//! This replaced a native reimplementation that ran `pull` synchronously
//! (a `pull` that returned a promise was not awaited), had no byte streams,
//! and drained `pipeTo`/`tee` eagerly.

use crate::host::{invoke, with_host, JsObj};
use fusevm::Value;

/// The members of `require('stream/web')`, in node's order (`lib/stream/web.js`).
pub const CLASSES: &[&str] = &[
    "ReadableStream",
    "ReadableStreamTee",
    "ReadableStreamDefaultReader",
    "ReadableStreamBYOBReader",
    "ReadableStreamBYOBRequest",
    "ReadableByteStreamController",
    "ReadableStreamDefaultController",
    "TransformStream",
    "TransformStreamDefaultController",
    "WritableStream",
    "WritableStreamDefaultWriter",
    "WritableStreamDefaultController",
    "ByteLengthQueuingStrategy",
    "CountQueuingStrategy",
    "TextEncoderStream",
    "TextDecoderStream",
    "CompressionStream",
    "DecompressionStream",
];

/// `stream/web` exports no free functions that dispatch natively.
pub const METHODS: &[&str] = &[];

/// True if `name` is a member of `require('stream/web')`.
pub fn is_class(name: &str) -> bool {
    CLASSES.contains(&name)
}

/// True if `name` is one of the stream classes node also binds as a global —
/// every member except `ReadableStreamTee`, which is a function.
pub fn is_global_class(name: &str) -> bool {
    name != "ReadableStreamTee" && is_class(name)
}

thread_local! {
    static PORT: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

/// The evaluated port: `{ web, isReadableStream, isWritableStream,
/// readableStreamCancel }`.
fn port() -> Result<Value, String> {
    if let Some(p) = PORT.with(|p| p.borrow().clone()) {
        return Ok(p);
    }
    let factory = crate::eval_in_global_scope(include_str!("webstreams.js"))?;
    let (node_error, promise_state) = with_host(|h| {
        (
            h.alloc(JsObj::Builtin("util.@@nodeError".into())),
            h.alloc(JsObj::Builtin("util.@@promiseState".into())),
        )
    });
    let made = invoke(&factory, vec![node_error, promise_state], None)?;
    PORT.with(|p| *p.borrow_mut() = Some(made.clone()));
    Ok(made)
}

/// `require('stream/web').<name>` (and the global of the same name).
pub fn constant(name: &str) -> Option<Value> {
    if !is_class(name) {
        return None;
    }
    let web = crate::builtins::get_property(&port().ok()?, "web").ok()?;
    crate::builtins::get_property(&web, name).ok()
}

/// One of the port's internals, by name.
fn internal(name: &str) -> Result<Value, String> {
    crate::builtins::get_property(&port()?, name)
}

/// The port's brand check `isReadableStream(v)` / `isWritableStream(v)`. No
/// stream exists before the port is first evaluated, so an unloaded port
/// answers `false` without loading it.
fn brand(check: &str, v: &Value) -> Result<bool, String> {
    if PORT.with(|p| p.borrow().is_none()) {
        return Ok(false);
    }
    let f = internal(check)?;
    let r = invoke(&f, vec![v.clone()], None)?;
    Ok(with_host(|h| h.truthy(&r)))
}

/// node's `isReadableStream`.
pub fn is_readable_stream(v: &Value) -> Result<bool, String> {
    brand("isReadableStream", v)
}

/// node's `isWritableStream`.
pub fn is_writable_stream(v: &Value) -> Result<bool, String> {
    brand("isWritableStream", v)
}

/// node's `readableStreamCancel(stream, reason)`: cancel without the lock
/// check `ReadableStream.prototype.cancel` makes.
pub fn readable_stream_cancel(args: &[Value]) -> Result<Value, String> {
    let f = internal("readableStreamCancel")?;
    invoke(&f, args.to_vec(), None)
}
