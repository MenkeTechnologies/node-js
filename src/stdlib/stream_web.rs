//! Node `stream/web` module: the WHATWG Streams API over the host object heap.
//!
//! Every stream, reader, writer and controller is an `@@native`-tagged
//! `JsObj::Object`; all internal state (queues, lock flags, callbacks, promises)
//! lives in hidden `@@`-prefixed props. There is no real backpressure scheduler
//! in this runtime, so the data-flow is modelled **synchronously**:
//!
//! * A `ReadableStream`'s `pull` callback is driven synchronously from
//!   `reader.read()` when the internal queue is empty. If `pull` enqueues (the
//!   common case) the `read()` promise resolves immediately; if the queue stays
//!   empty (a controller-fed stream such as a `TransformStream`'s readable) a
//!   genuine *pending* promise is returned and settled when a later `enqueue`,
//!   `close` or `error` occurs. This makes the queue/close/error data semantics
//!   correct even though timing is synchronous.
//! * `pipeTo` / `pipeThrough` / `tee` drain their source synchronously. An
//!   asynchronous-only `pull` (one that resolves a promise and enqueues *later*,
//!   with nothing on the queue) is treated as end-of-stream by those operations
//!   — the one place the synchronous model diverges from spec timing (documented,
//!   not faked: no chunk is invented).
//!
//! `Symbol.asyncIterator` on a `ReadableStream` is DEFERRED: the host does not
//! support async-iterator discovery on native objects. `getReader().read()` is
//! the fully-working consumption path.

use crate::host::{invoke, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::io::{Read, Write};

// ── class / tag registry ─────────────────────────────────────────────────────

/// The classes exported by `require('stream/web')`. Each resolves to a
/// `Builtin(name)` via `constant` and constructs via `construct`.
pub const CLASSES: &[&str] = &[
    "ReadableStream",
    "ReadableStreamDefaultReader",
    "ReadableStreamBYOBReader",
    "ReadableStreamDefaultController",
    "ReadableByteStreamController",
    "ReadableStreamBYOBRequest",
    "WritableStream",
    "WritableStreamDefaultWriter",
    "WritableStreamDefaultController",
    "TransformStream",
    "TransformStreamDefaultController",
    "ByteLengthQueuingStrategy",
    "CountQueuingStrategy",
    "TextEncoderStream",
    "TextDecoderStream",
    "CompressionStream",
    "DecompressionStream",
];

/// `stream/web` exports no free functions — everything is a class/constructor.
pub const METHODS: &[&str] = &[];

// `@@native` tags (== class names, so `instanceof`/`native_tag` line up).
pub const READABLE_STREAM_TAG: &str = "ReadableStream";
pub const RS_DEFAULT_READER_TAG: &str = "ReadableStreamDefaultReader";
pub const RS_BYOB_READER_TAG: &str = "ReadableStreamBYOBReader";
pub const RS_DEFAULT_CONTROLLER_TAG: &str = "ReadableStreamDefaultController";
pub const RS_BYTE_CONTROLLER_TAG: &str = "ReadableByteStreamController";
pub const RS_BYOB_REQUEST_TAG: &str = "ReadableStreamBYOBRequest";
pub const WRITABLE_STREAM_TAG: &str = "WritableStream";
pub const WS_DEFAULT_WRITER_TAG: &str = "WritableStreamDefaultWriter";
pub const WS_DEFAULT_CONTROLLER_TAG: &str = "WritableStreamDefaultController";
pub const TRANSFORM_STREAM_TAG: &str = "TransformStream";
pub const TS_DEFAULT_CONTROLLER_TAG: &str = "TransformStreamDefaultController";
pub const BYTE_LENGTH_STRATEGY_TAG: &str = "ByteLengthQueuingStrategy";
pub const COUNT_STRATEGY_TAG: &str = "CountQueuingStrategy";
pub const TEXT_ENCODER_STREAM_TAG: &str = "TextEncoderStream";
pub const TEXT_DECODER_STREAM_TAG: &str = "TextDecoderStream";
pub const COMPRESSION_STREAM_TAG: &str = "CompressionStream";
pub const DECOMPRESSION_STREAM_TAG: &str = "DecompressionStream";

// Instance-method lists (for `instance_has_method` wiring in mod.rs).
pub const READABLE_STREAM_METHODS: &[&str] = &["getReader", "cancel", "tee", "pipeTo", "pipeThrough"];
pub const RS_DEFAULT_READER_METHODS: &[&str] = &["read", "releaseLock", "cancel"];
pub const RS_BYOB_READER_METHODS: &[&str] = &["read", "releaseLock", "cancel"];
pub const RS_DEFAULT_CONTROLLER_METHODS: &[&str] = &["enqueue", "close", "error"];
pub const RS_BYTE_CONTROLLER_METHODS: &[&str] = &["enqueue", "close", "error"];
pub const RS_BYOB_REQUEST_METHODS: &[&str] = &["respond", "respondWithNewView"];
pub const WRITABLE_STREAM_METHODS: &[&str] = &["getWriter", "abort", "close"];
pub const WS_DEFAULT_WRITER_METHODS: &[&str] = &["write", "close", "abort", "releaseLock"];
pub const WS_DEFAULT_CONTROLLER_METHODS: &[&str] = &["error"];
pub const TS_DEFAULT_CONTROLLER_METHODS: &[&str] = &["enqueue", "terminate", "error"];
pub const STRATEGY_METHODS: &[&str] = &["size"];

/// True if `name` is one of the `stream/web` class constructors.
pub fn is_class(name: &str) -> bool {
    CLASSES.contains(&name)
}

/// `require('stream/web').<Class>` → the constructor value.
pub fn constant(name: &str) -> Option<Value> {
    if is_class(name) {
        Some(with_host(|h| h.alloc(JsObj::Builtin(name.to_string()))))
    } else {
        None
    }
}

/// The method list for a `stream/web` tag (for `instance_has_method`).
pub fn methods_for(tag: &str) -> &'static [&'static str] {
    match tag {
        READABLE_STREAM_TAG => READABLE_STREAM_METHODS,
        RS_DEFAULT_READER_TAG => RS_DEFAULT_READER_METHODS,
        RS_BYOB_READER_TAG => RS_BYOB_READER_METHODS,
        RS_DEFAULT_CONTROLLER_TAG => RS_DEFAULT_CONTROLLER_METHODS,
        RS_BYTE_CONTROLLER_TAG => RS_BYTE_CONTROLLER_METHODS,
        RS_BYOB_REQUEST_TAG => RS_BYOB_REQUEST_METHODS,
        WRITABLE_STREAM_TAG => WRITABLE_STREAM_METHODS,
        WS_DEFAULT_WRITER_TAG => WS_DEFAULT_WRITER_METHODS,
        WS_DEFAULT_CONTROLLER_TAG => WS_DEFAULT_CONTROLLER_METHODS,
        TS_DEFAULT_CONTROLLER_TAG => TS_DEFAULT_CONTROLLER_METHODS,
        BYTE_LENGTH_STRATEGY_TAG | COUNT_STRATEGY_TAG => STRATEGY_METHODS,
        _ => &[],
    }
}

// ── small object / prop helpers (each a single, non-nested `with_host`) ───────

fn get_prop(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

fn set_prop(recv: &Value, key: &str, val: Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), val);
        }
    });
}

fn remove_prop(recv: &Value, key: &str) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.shift_remove(key);
        }
    });
}

fn get_str(recv: &Value, key: &str) -> Option<String> {
    get_prop(recv, key).map(|v| with_host(|h| h.str_of(&v)))
}

fn state_of(stream: &Value) -> String {
    get_str(stream, "@@state").unwrap_or_else(|| "readable".into())
}

fn is_locked(stream: &Value) -> bool {
    get_prop(stream, "locked").map(|v| with_host(|h| h.truthy(&v))).unwrap_or(false)
}

fn is_callable_val(v: &Value) -> bool {
    with_host(|h| crate::host::is_callable(h, v))
}

/// A source/sink option that is a function, else `None`.
fn opt_cb(obj: &Value, key: &str) -> Option<Value> {
    get_prop(obj, key).filter(is_callable_val)
}

/// Build a `{ value, done }` iterator result.
fn iter_result(value: Value, done: bool) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("value".into(), value);
        m.insert("done".into(), Value::Bool(done));
        h.new_object(m)
    })
}

fn synth(msg: &str) -> Value {
    with_host(|h| crate::builtins::synth_error(h, msg))
}

fn new_pending() -> Value {
    with_host(|h| h.new_promise())
}

fn settle(p: &Value, val: Value) {
    if let Some(id) = with_host(|h| h.promise_id(p)) {
        crate::host::resolve_promise_val(id, val);
    }
}

fn settle_reject(p: &Value, err: Value) {
    if let Some(id) = with_host(|h| h.promise_id(p)) {
        crate::host::reject_promise_val(id, err);
    }
}

fn resolved(v: Value) -> Value {
    crate::host::promise_of(&v)
}

fn rejected(err: Value) -> Value {
    let p = new_pending();
    settle_reject(&p, err);
    p
}

/// Raw bytes of a chunk: a Buffer's `@@bytes`, a TypedArray's `@@elems`, else its
/// UTF-8 string form.
fn chunk_bytes(v: &Value) -> Vec<u8> {
    let via_field = with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => {
            let field = if p.contains_key("@@bytes") {
                Some("@@bytes")
            } else if p.contains_key("@@elems") {
                Some("@@elems")
            } else {
                None
            };
            field.and_then(|f| match p.get(f).and_then(|a| h.get(a)) {
                Some(JsObj::Array(items)) => Some(items.iter().map(|x| h.to_number(x) as u8).collect::<Vec<u8>>()),
                _ => None,
            })
        }
        _ => None,
    });
    via_field.unwrap_or_else(|| with_host(|h| h.str_of(v)).into_bytes())
}

/// A chunk's `byteLength` (its own `byteLength` prop, else its byte count).
fn chunk_byte_length(v: &Value) -> f64 {
    if let Some(bl) = get_prop(v, "byteLength") {
        return with_host(|h| h.to_number(&bl));
    }
    chunk_bytes(v).len() as f64
}

// ── array-prop helpers (queues / waiters) ─────────────────────────────────────

fn arr_push(recv: &Value, key: &str, val: Value) {
    with_host(|h| {
        let arr = match h.get(recv) {
            Some(JsObj::Object(p)) => p.get(key).cloned(),
            _ => None,
        };
        if let Some(a) = arr {
            if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                items.push(val);
            }
        }
    });
}

fn arr_shift(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| {
        let arr = match h.get(recv) {
            Some(JsObj::Object(p)) => p.get(key).cloned(),
            _ => None,
        }?;
        match h.get_mut(&arr) {
            Some(JsObj::Array(items)) if !items.is_empty() => Some(items.remove(0)),
            _ => None,
        }
    })
}

fn arr_len(recv: &Value, key: &str) -> usize {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => match p.get(key).and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.len(),
            _ => 0,
        },
        _ => 0,
    })
}

fn arr_get(recv: &Value, key: &str, i: usize) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => match p.get(key).and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.get(i).cloned(),
            _ => None,
        },
        _ => None,
    })
}

fn arr_drain(recv: &Value, key: &str) -> Vec<Value> {
    with_host(|h| {
        let arr = match h.get(recv) {
            Some(JsObj::Object(p)) => p.get(key).cloned(),
            _ => None,
        };
        match arr.as_ref().and_then(|a| h.get_mut(a)) {
            Some(JsObj::Array(items)) => std::mem::take(items),
            _ => Vec::new(),
        }
    })
}

// ── ReadableStream internals ──────────────────────────────────────────────────

/// Allocate a bare ReadableStream shell (state `readable`, empty queue/waiters).
fn new_readable_shell() -> Value {
    with_host(|h| {
        let queue = h.new_array(Vec::new());
        let waiters = h.new_array(Vec::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(READABLE_STREAM_TAG));
        m.insert("@@state".into(), h.new_str("readable"));
        m.insert("@@queue".into(), queue);
        m.insert("@@waiters".into(), waiters);
        m.insert("locked".into(), Value::Bool(false));
        h.new_object(m)
    })
}

/// A default (or byte) controller wired back to `stream`.
fn new_readable_controller(stream: &Value, byte: bool) -> Value {
    let tag = if byte { RS_BYTE_CONTROLLER_TAG } else { RS_DEFAULT_CONTROLLER_TAG };
    let ctrl = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        m.insert("@@stream".into(), stream.clone());
        m.insert("desiredSize".into(), Value::Float(1.0));
        if byte {
            m.insert("byobRequest".into(), h.null());
        }
        h.new_object(m)
    });
    set_prop(stream, "@@controller", ctrl.clone());
    ctrl
}

/// Enqueue `chunk`: hand it to the oldest pending reader if one is waiting, else
/// append to the internal queue.
fn stream_enqueue(stream: &Value, chunk: Value) {
    if state_of(stream) != "readable" {
        return;
    }
    if let Some(waiter) = arr_shift(stream, "@@waiters") {
        let r = iter_result(chunk, false);
        settle(&waiter, r);
    } else {
        arr_push(stream, "@@queue", chunk);
    }
}

/// Close the stream: settle every pending reader with `{done:true}`.
fn stream_close(stream: &Value) {
    if state_of(stream) != "readable" {
        return;
    }
    set_prop(stream, "@@state", with_host(|h| h.new_str("closed")));
    for waiter in arr_drain(stream, "@@waiters") {
        let r = iter_result(Value::Undef, true);
        settle(&waiter, r);
    }
    // Resolve a reader's `closed` promise if one is attached.
    if let Some(reader) = get_prop(stream, "@@reader") {
        if let Some(cp) = get_prop(&reader, "closed") {
            settle(&cp, Value::Undef);
        }
    }
}

/// Error the stream: reject every pending reader with `err`.
fn stream_error(stream: &Value, err: Value) {
    if state_of(stream) != "readable" {
        return;
    }
    set_prop(stream, "@@state", with_host(|h| h.new_str("errored")));
    set_prop(stream, "@@stored_error", err.clone());
    for waiter in arr_drain(stream, "@@waiters") {
        settle_reject(&waiter, err.clone());
    }
    if let Some(reader) = get_prop(stream, "@@reader") {
        if let Some(cp) = get_prop(&reader, "closed") {
            settle_reject(&cp, err.clone());
        }
    }
}

/// A settled `read()` promise if data / close / error is immediately available.
fn try_immediate_read(stream: &Value) -> Option<Value> {
    match state_of(stream).as_str() {
        "errored" => {
            let e = get_prop(stream, "@@stored_error").unwrap_or_else(|| synth("TypeError: stream errored"));
            Some(rejected(e))
        }
        _ if arr_len(stream, "@@queue") > 0 => {
            let chunk = arr_shift(stream, "@@queue").unwrap_or(Value::Undef);
            Some(resolved(iter_result(chunk, false)))
        }
        "closed" => Some(resolved(iter_result(Value::Undef, true))),
        _ => None,
    }
}

/// Drive one synchronous production step (a JS `pull` or a native tee-pull) when
/// the queue is empty.
fn drive_pull(stream: &Value) {
    if let Some(kind) = get_str(stream, "@@native_pull") {
        if kind == "tee" {
            tee_pull(stream);
        }
        return;
    }
    if let Some(pull) = opt_cb(stream, "@@pull") {
        let ctrl = get_prop(stream, "@@controller").unwrap_or(Value::Undef);
        // A `pull` that returns a promise (async pull) is accepted but its later
        // resolution is not awaited — synchronous enqueues are what drives data.
        if let Err(msg) = invoke(&pull, vec![ctrl], None) {
            stream_error(stream, synth(&msg));
        }
    }
}

/// `reader.read()` core: resolve immediately if possible, else drive `pull` once
/// and re-check, else return a pending promise settled by a later enqueue/close.
fn stream_read(stream: &Value) -> Value {
    if let Some(p) = try_immediate_read(stream) {
        return p;
    }
    drive_pull(stream);
    if let Some(p) = try_immediate_read(stream) {
        return p;
    }
    // Still readable and empty: a genuine pending read, settled on next enqueue.
    let p = new_pending();
    arr_push(stream, "@@waiters", p.clone());
    p
}

/// `stream.cancel(reason)` / `reader.cancel(reason)`: close and run `cancel`.
fn stream_cancel(stream: &Value, reason: Value) -> Value {
    if state_of(stream) == "readable" {
        // Discard buffered chunks, then close.
        let _ = arr_drain(stream, "@@queue");
        if let Some(cancel) = opt_cb(stream, "@@cancel") {
            if let Err(msg) = invoke(&cancel, vec![reason], None) {
                return rejected(synth(&msg));
            }
        }
        stream_close(stream);
    }
    resolved(Value::Undef)
}

// ── ReadableStream construction ───────────────────────────────────────────────

/// `new ReadableStream(underlyingSource, strategy)`.
pub fn construct_readable(args: &[Value]) -> Result<Value, String> {
    let source = args.first().cloned().unwrap_or(Value::Undef);
    let byte = get_str(&source, "type").as_deref() == Some("bytes");

    let stream = new_readable_shell();
    if byte {
        set_prop(&stream, "@@type", with_host(|h| h.new_str("bytes")));
    }
    if let Some(pull) = opt_cb(&source, "pull") {
        set_prop(&stream, "@@pull", pull);
    }
    if let Some(cancel) = opt_cb(&source, "cancel") {
        set_prop(&stream, "@@cancel", cancel);
    }
    let ctrl = new_readable_controller(&stream, byte);

    if let Some(start) = opt_cb(&source, "start") {
        if let Err(msg) = invoke(&start, vec![ctrl], None) {
            stream_error(&stream, synth(&msg));
        }
    }
    Ok(stream)
}

/// `stream.getReader([{ mode }])`.
fn get_reader(stream: &Value, args: &[Value]) -> Result<Value, String> {
    if is_locked(stream) {
        return Err(crate::host::type_error("ReadableStream is locked"));
    }
    let byob = args
        .first()
        .and_then(|o| get_str(o, "mode"))
        .as_deref()
        == Some("byob");
    let tag = if byob { RS_BYOB_READER_TAG } else { RS_DEFAULT_READER_TAG };

    // The reader's `closed` promise reflects the stream's terminal state.
    let closed = new_pending();
    match state_of(stream).as_str() {
        "closed" => settle(&closed, Value::Undef),
        "errored" => settle_reject(
            &closed,
            get_prop(stream, "@@stored_error").unwrap_or_else(|| synth("TypeError: stream errored")),
        ),
        _ => {}
    }

    let reader = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        m.insert("@@stream".into(), stream.clone());
        m.insert("closed".into(), closed);
        h.new_object(m)
    });
    set_prop(stream, "locked", Value::Bool(true));
    set_prop(stream, "@@reader", reader.clone());
    Ok(reader)
}

fn reader_release(reader: &Value) {
    if let Some(stream) = get_prop(reader, "@@stream") {
        set_prop(&stream, "locked", Value::Bool(false));
        remove_prop(&stream, "@@reader");
    }
    remove_prop(reader, "@@stream");
}

// ── WritableStream internals ──────────────────────────────────────────────────

/// Accept one chunk into a writable's sink (a JS `write`, or a built-in codec /
/// transform dispatch keyed by `@@xform_kind`).
fn ws_accept_chunk(ws: &Value, chunk: Value) -> Result<(), String> {
    if let Some(kind) = get_str(ws, "@@xform_kind") {
        let readable = get_prop(ws, "@@readable");
        match kind.as_str() {
            "textencode" => {
                if let Some(r) = &readable {
                    let bytes = with_host(|h| h.str_of(&chunk)).into_bytes();
                    let buf = super::buffer::from_bytes(&bytes);
                    stream_enqueue(r, buf);
                }
            }
            "textdecode" => {
                if let Some(r) = &readable {
                    let bytes = chunk_bytes(&chunk);
                    let s = with_host(|h| h.new_str(String::from_utf8_lossy(&bytes).into_owned()));
                    stream_enqueue(r, s);
                }
            }
            k if k.starts_with("compress:") || k.starts_with("decompress:") => {
                let mut bytes = chunk_bytes(&chunk);
                arr_push_bytes(ws, &mut bytes);
            }
            "js" => {
                let ctrl = get_prop(ws, "@@tcontroller").unwrap_or(Value::Undef);
                if let Some(transform) = opt_cb(ws, "@@transform") {
                    invoke(&transform, vec![chunk, ctrl], None)?;
                } else if let Some(r) = &readable {
                    // Identity transform: pass the chunk straight through.
                    stream_enqueue(r, chunk);
                }
            }
            _ => {}
        }
        return Ok(());
    }
    if let Some(write) = opt_cb(ws, "@@write") {
        let ctrl = get_prop(ws, "@@controller").unwrap_or(Value::Undef);
        invoke(&write, vec![chunk, ctrl], None)?;
    }
    Ok(())
}

/// Append raw bytes onto a writable's `@@accum` byte buffer (for codec streams).
fn arr_push_bytes(ws: &Value, bytes: &mut Vec<u8>) {
    with_host(|h| {
        let accum = match h.get(ws) {
            Some(JsObj::Object(p)) => p.get("@@accum").cloned(),
            _ => None,
        };
        if let Some(a) = accum {
            if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                items.extend(bytes.drain(..).map(|b| Value::Float(b as f64)));
            }
        }
    });
}

fn ws_accum_bytes(ws: &Value) -> Vec<u8> {
    with_host(|h| match h.get(ws) {
        Some(JsObj::Object(p)) => match p.get("@@accum").and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.iter().map(|x| h.to_number(x) as u8).collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    })
}

/// Finish a writable's sink: run flush / codec, close a paired readable, mark
/// the writable closed and settle its `closed` promise.
fn ws_finish(ws: &Value) -> Result<(), String> {
    if let Some(kind) = get_str(ws, "@@xform_kind") {
        let readable = get_prop(ws, "@@readable");
        match kind.as_str() {
            "js" => {
                let ctrl = get_prop(ws, "@@tcontroller").unwrap_or(Value::Undef);
                if let Some(flush) = opt_cb(ws, "@@flush") {
                    invoke(&flush, vec![ctrl], None)?;
                }
                if let Some(r) = &readable {
                    stream_close(r);
                }
            }
            k if k.starts_with("compress:") || k.starts_with("decompress:") => {
                let data = ws_accum_bytes(ws);
                let out = run_codec(&kind, &data)?;
                if let Some(r) = &readable {
                    stream_enqueue(r, super::buffer::from_bytes(&out));
                    stream_close(r);
                }
            }
            _ => {
                if let Some(r) = &readable {
                    stream_close(r);
                }
            }
        }
    } else if let Some(close) = opt_cb(ws, "@@close") {
        invoke(&close, vec![], None)?;
    }
    set_prop(ws, "@@state", with_host(|h| h.new_str("closed")));
    if let Some(writer) = get_prop(ws, "@@writer") {
        if let Some(cp) = get_prop(&writer, "closed") {
            settle(&cp, Value::Undef);
        }
    }
    Ok(())
}

/// Abort/error a writable: mark errored, error a paired readable, reject `closed`.
fn ws_abort(ws: &Value, reason: Value) -> Result<(), String> {
    if get_str(ws, "@@xform_kind").is_some() {
        if let Some(r) = get_prop(ws, "@@readable") {
            stream_error(&r, reason.clone());
        }
    } else if let Some(abort) = opt_cb(ws, "@@abort") {
        invoke(&abort, vec![reason.clone()], None)?;
    }
    set_prop(ws, "@@state", with_host(|h| h.new_str("errored")));
    set_prop(ws, "@@stored_error", reason.clone());
    if let Some(writer) = get_prop(ws, "@@writer") {
        if let Some(cp) = get_prop(&writer, "closed") {
            settle_reject(&cp, reason);
        }
    }
    Ok(())
}

/// `new WritableStream(underlyingSink, strategy)`.
pub fn construct_writable(args: &[Value]) -> Result<Value, String> {
    let sink = args.first().cloned().unwrap_or(Value::Undef);
    let ws = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(WRITABLE_STREAM_TAG));
        m.insert("@@state".into(), h.new_str("writable"));
        m.insert("locked".into(), Value::Bool(false));
        h.new_object(m)
    });
    for key in ["write", "close", "abort"] {
        if let Some(cb) = opt_cb(&sink, key) {
            set_prop(&ws, &format!("@@{key}"), cb);
        }
    }
    let ctrl = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(WS_DEFAULT_CONTROLLER_TAG));
        m.insert("@@stream".into(), ws.clone());
        h.new_object(m)
    });
    set_prop(&ws, "@@controller", ctrl.clone());
    if let Some(start) = opt_cb(&sink, "start") {
        if let Err(msg) = invoke(&start, vec![ctrl], None) {
            let _ = ws_abort(&ws, synth(&msg));
        }
    }
    Ok(ws)
}

/// `writable.getWriter()`.
fn get_writer(ws: &Value) -> Result<Value, String> {
    if is_locked(ws) {
        return Err(crate::host::type_error("WritableStream is locked"));
    }
    let ready = resolved(Value::Undef);
    let closed = new_pending();
    if state_of(ws) == "closed" {
        settle(&closed, Value::Undef);
    } else if state_of(ws) == "errored" {
        settle_reject(&closed, get_prop(ws, "@@stored_error").unwrap_or(Value::Undef));
    }
    let writer = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(WS_DEFAULT_WRITER_TAG));
        m.insert("@@stream".into(), ws.clone());
        m.insert("ready".into(), ready);
        m.insert("closed".into(), closed);
        m.insert("desiredSize".into(), Value::Float(1.0));
        h.new_object(m)
    });
    set_prop(ws, "locked", Value::Bool(true));
    set_prop(ws, "@@writer", writer.clone());
    Ok(writer)
}

fn writer_release(writer: &Value) {
    if let Some(ws) = get_prop(writer, "@@stream") {
        set_prop(&ws, "locked", Value::Bool(false));
        remove_prop(&ws, "@@writer");
    }
    remove_prop(writer, "@@stream");
}

// ── TransformStream ───────────────────────────────────────────────────────────

/// Build the readable + writable pair shared by `TransformStream` and the
/// built-in codec/text streams. `kind` selects the writable's sink behaviour.
fn build_transform_pair(kind: &str) -> (Value, Value) {
    let readable = new_readable_shell();
    let ws = with_host(|h| {
        let accum = h.new_array(Vec::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(WRITABLE_STREAM_TAG));
        m.insert("@@state".into(), h.new_str("writable"));
        m.insert("locked".into(), Value::Bool(false));
        m.insert("@@xform_kind".into(), h.new_str(kind));
        m.insert("@@accum".into(), accum);
        h.new_object(m)
    });
    set_prop(&ws, "@@readable", readable.clone());
    (readable, ws)
}

/// `new TransformStream(transformer, writableStrategy, readableStrategy)`.
pub fn construct_transform(args: &[Value]) -> Result<Value, String> {
    let transformer = args.first().cloned().unwrap_or(Value::Undef);
    let (readable, writable) = build_transform_pair("js");

    // The transform controller enqueues into the readable side.
    let tctrl = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(TS_DEFAULT_CONTROLLER_TAG));
        m.insert("@@readable".into(), readable.clone());
        m.insert("@@writable".into(), writable.clone());
        m.insert("desiredSize".into(), Value::Float(1.0));
        h.new_object(m)
    });
    set_prop(&writable, "@@tcontroller", tctrl.clone());
    if let Some(t) = opt_cb(&transformer, "transform") {
        set_prop(&writable, "@@transform", t);
    }
    if let Some(f) = opt_cb(&transformer, "flush") {
        set_prop(&writable, "@@flush", f);
    }

    let ts = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(TRANSFORM_STREAM_TAG));
        m.insert("readable".into(), readable);
        m.insert("writable".into(), writable);
        h.new_object(m)
    });

    if let Some(start) = opt_cb(&transformer, "start") {
        invoke(&start, vec![tctrl], None)?;
    }
    Ok(ts)
}

// ── text / codec transform streams ────────────────────────────────────────────

fn build_codec_stream(tag: &str, kind: &str, extra: Vec<(&str, Value)>) -> Value {
    let (readable, writable) = build_transform_pair(kind);
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        m.insert("readable".into(), readable);
        m.insert("writable".into(), writable);
        for (k, v) in extra {
            m.insert(k.to_string(), v);
        }
        h.new_object(m)
    })
}

pub fn construct_text_encoder_stream() -> Result<Value, String> {
    let enc = with_host(|h| h.new_str("utf-8"));
    Ok(build_codec_stream(TEXT_ENCODER_STREAM_TAG, "textencode", vec![("encoding", enc)]))
}

pub fn construct_text_decoder_stream(args: &[Value]) -> Result<Value, String> {
    let label = args
        .first()
        .map(|v| with_host(|h| h.str_of(v)))
        .filter(|s| !s.is_empty() && s.as_str() != "undefined")
        .unwrap_or_else(|| "utf-8".into());
    let extra = with_host(|h| {
        vec![
            ("encoding", h.new_str(label.to_lowercase())),
            ("fatal", Value::Bool(false)),
            ("ignoreBOM", Value::Bool(false)),
        ]
    });
    Ok(build_codec_stream(TEXT_DECODER_STREAM_TAG, "textdecode", extra))
}

/// `new CompressionStream(format)` — `format` ∈ {gzip, deflate, deflate-raw}.
pub fn construct_compression(args: &[Value], decompress: bool) -> Result<Value, String> {
    let format = super::arg_str(args, 0);
    if !matches!(format.as_str(), "gzip" | "deflate" | "deflate-raw") {
        return Err(crate::host::type_error(&format!("Unsupported compression format: '{format}'")));
    }
    let (tag, prefix) = if decompress {
        (DECOMPRESSION_STREAM_TAG, "decompress")
    } else {
        (COMPRESSION_STREAM_TAG, "compress")
    };
    Ok(build_codec_stream(tag, &format!("{prefix}:{format}"), Vec::new()))
}

/// Run a buffered codec over `data` (called on stream close).
fn run_codec(kind: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    use flate2::Compression;
    let io = |e: std::io::Error| format!("Error: {e}");
    match kind {
        "compress:gzip" => {
            let mut e = GzEncoder::new(Vec::new(), Compression::default());
            e.write_all(data).map_err(io)?;
            e.finish().map_err(io)
        }
        "compress:deflate" => {
            let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
            e.write_all(data).map_err(io)?;
            e.finish().map_err(io)
        }
        "compress:deflate-raw" => {
            let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
            e.write_all(data).map_err(io)?;
            e.finish().map_err(io)
        }
        "decompress:gzip" => {
            let mut out = Vec::new();
            GzDecoder::new(data).read_to_end(&mut out).map_err(io)?;
            Ok(out)
        }
        "decompress:deflate" => {
            let mut out = Vec::new();
            ZlibDecoder::new(data).read_to_end(&mut out).map_err(io)?;
            Ok(out)
        }
        "decompress:deflate-raw" => {
            let mut out = Vec::new();
            DeflateDecoder::new(data).read_to_end(&mut out).map_err(io)?;
            Ok(out)
        }
        _ => Err(crate::host::type_error("unknown codec")),
    }
}

// ── queuing strategies ────────────────────────────────────────────────────────

pub fn construct_strategy(tag: &str, args: &[Value]) -> Result<Value, String> {
    let hwm = args
        .first()
        .and_then(|o| get_prop(o, "highWaterMark"))
        .map(|v| with_host(|h| h.to_number(&v)))
        .unwrap_or(f64::NAN);
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        m.insert("highWaterMark".into(), Value::Float(hwm));
        h.new_object(m)
    }))
}

// ── byte-stream / BYOB (basic) ────────────────────────────────────────────────

/// A pulled value from a source (used by `pipeTo`/`tee`/BYOB drains).
enum Pulled {
    Chunk(Value),
    Done,
    Errored(Value),
}

/// One synchronous production step for a drain: return a chunk, or `Done`/`Errored`.
fn pull_one(stream: &Value) -> Pulled {
    if state_of(stream) == "errored" {
        return Pulled::Errored(get_prop(stream, "@@stored_error").unwrap_or(Value::Undef));
    }
    if let Some(c) = arr_shift(stream, "@@queue") {
        return Pulled::Chunk(c);
    }
    if state_of(stream) == "closed" {
        return Pulled::Done;
    }
    drive_pull(stream);
    if state_of(stream) == "errored" {
        return Pulled::Errored(get_prop(stream, "@@stored_error").unwrap_or(Value::Undef));
    }
    if let Some(c) = arr_shift(stream, "@@queue") {
        return Pulled::Chunk(c);
    }
    // Empty after a pull step: end the drain (cannot suspend synchronously).
    Pulled::Done
}

/// `byobReader.read(view)` — fills `view` from the next chunk (best-effort).
fn byob_read(reader: &Value, args: &[Value]) -> Value {
    let Some(stream) = get_prop(reader, "@@stream") else {
        return rejected(synth("TypeError: reader has no associated stream"));
    };
    let view = args.first().cloned().unwrap_or(Value::Undef);
    match pull_one(&stream) {
        Pulled::Chunk(c) => {
            let bytes = chunk_bytes(&c);
            // Return a fresh view over the copied bytes (does not reuse the
            // caller's ArrayBuffer — a documented BYOB simplification).
            resolved(iter_result(super::buffer::from_bytes(&bytes), false))
        }
        Pulled::Done => resolved(iter_result(view, true)),
        Pulled::Errored(e) => rejected(e),
    }
}

// ── tee ───────────────────────────────────────────────────────────────────────

/// Native pull for a tee branch: serve from the shared buffer at the branch's
/// cursor, pulling one more chunk from the source when the cursor runs ahead.
fn tee_pull(branch: &Value) {
    let Some(shared) = get_prop(branch, "@@tee_shared") else { return };
    let idx = get_prop(branch, "@@tee_index").map(|v| with_host(|h| h.to_number(&v)) as usize).unwrap_or(0);

    if let Some(chunk) = arr_get(&shared, "@@buf", idx) {
        set_prop(branch, "@@tee_index", Value::Float((idx + 1) as f64));
        stream_enqueue(branch, chunk);
        return;
    }
    let done = get_prop(&shared, "@@done").map(|v| with_host(|h| h.truthy(&v))).unwrap_or(false);
    if done {
        stream_close(branch);
        return;
    }
    let Some(source) = get_prop(&shared, "@@source") else {
        stream_close(branch);
        return;
    };
    match pull_one(&source) {
        Pulled::Chunk(c) => {
            arr_push(&shared, "@@buf", c.clone());
            set_prop(branch, "@@tee_index", Value::Float((idx + 1) as f64));
            stream_enqueue(branch, c);
        }
        Pulled::Done => {
            set_prop(&shared, "@@done", Value::Bool(true));
            stream_close(branch);
        }
        Pulled::Errored(e) => stream_error(branch, e),
    }
}

/// `stream.tee()` → `[branch1, branch2]`, lazily sharing the source.
fn tee(stream: &Value) -> Value {
    set_prop(stream, "locked", Value::Bool(true));
    let shared = with_host(|h| {
        let buf = h.new_array(Vec::new());
        let mut m = IndexMap::new();
        m.insert("@@source".into(), stream.clone());
        m.insert("@@buf".into(), buf);
        m.insert("@@done".into(), Value::Bool(false));
        h.new_object(m)
    });
    let make_branch = || {
        let b = new_readable_shell();
        new_readable_controller(&b, false);
        set_prop(&b, "@@native_pull", with_host(|h| h.new_str("tee")));
        set_prop(&b, "@@tee_shared", shared.clone());
        set_prop(&b, "@@tee_index", Value::Float(0.0));
        b
    };
    let b1 = make_branch();
    let b2 = make_branch();
    with_host(|h| h.new_array(vec![b1, b2]))
}

// ── pipeTo / pipeThrough ──────────────────────────────────────────────────────

/// `stream.pipeTo(destWritable)` — synchronously drain the source into the sink,
/// then close it. Returns a resolved (or rejected on error) promise.
fn pipe_to(stream: &Value, dest: &Value) -> Value {
    set_prop(stream, "locked", Value::Bool(true));
    set_prop(dest, "locked", Value::Bool(true));
    loop {
        match pull_one(stream) {
            Pulled::Chunk(c) => {
                if let Err(msg) = ws_accept_chunk(dest, c) {
                    return rejected(synth(&msg));
                }
            }
            Pulled::Done => break,
            Pulled::Errored(e) => {
                let _ = ws_abort(dest, e.clone());
                return rejected(e);
            }
        }
    }
    if let Err(msg) = ws_finish(dest) {
        return rejected(synth(&msg));
    }
    resolved(Value::Undef)
}

/// `stream.pipeThrough({ writable, readable })` — pipe into `writable`, return
/// `readable`. The pipe runs synchronously so `readable` is already fed on return.
fn pipe_through(stream: &Value, args: &[Value]) -> Result<Value, String> {
    let transform = args.first().cloned().unwrap_or(Value::Undef);
    let writable = get_prop(&transform, "writable").ok_or_else(|| {
        crate::host::type_error("pipeThrough argument must have a writable and readable")
    })?;
    let readable = get_prop(&transform, "readable").ok_or_else(|| {
        crate::host::type_error("pipeThrough argument must have a writable and readable")
    })?;
    let _ = pipe_to(stream, &writable);
    Ok(readable)
}

// ── construct dispatch ────────────────────────────────────────────────────────

/// `new <Class>(...)` for every `stream/web` class. `None` if `name` is not ours.
pub fn construct(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match name {
        "ReadableStream" => construct_readable(args),
        "WritableStream" => construct_writable(args),
        "TransformStream" => construct_transform(args),
        "TextEncoderStream" => construct_text_encoder_stream(),
        "TextDecoderStream" => construct_text_decoder_stream(args),
        "CompressionStream" => construct_compression(args, false),
        "DecompressionStream" => construct_compression(args, true),
        "ByteLengthQueuingStrategy" => construct_strategy(BYTE_LENGTH_STRATEGY_TAG, args),
        "CountQueuingStrategy" => construct_strategy(COUNT_STRATEGY_TAG, args),
        // The controller/reader classes are normally handed out by the streams
        // above; direct construction yields a bare tagged shell so `instanceof`
        // and manual wiring work.
        "ReadableStreamDefaultReader" => Ok(bare(RS_DEFAULT_READER_TAG)),
        "ReadableStreamBYOBReader" => Ok(bare(RS_BYOB_READER_TAG)),
        "ReadableStreamDefaultController" => Ok(bare(RS_DEFAULT_CONTROLLER_TAG)),
        "ReadableByteStreamController" => Ok(bare(RS_BYTE_CONTROLLER_TAG)),
        "ReadableStreamBYOBRequest" => Ok(bare(RS_BYOB_REQUEST_TAG)),
        "WritableStreamDefaultWriter" => Ok(bare(WS_DEFAULT_WRITER_TAG)),
        "WritableStreamDefaultController" => Ok(bare(WS_DEFAULT_CONTROLLER_TAG)),
        "TransformStreamDefaultController" => Ok(bare(TS_DEFAULT_CONTROLLER_TAG)),
        _ => return None,
    })
}

fn bare(tag: &str) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        h.new_object(m)
    })
}

// ── instance dispatch ─────────────────────────────────────────────────────────

/// Method dispatch for every `stream/web` native instance.
pub fn instance_call(tag: &str, recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match tag {
        READABLE_STREAM_TAG => match method {
            "getReader" => get_reader(recv, &args),
            "cancel" => Ok(stream_cancel(recv, args.first().cloned().unwrap_or(Value::Undef))),
            "tee" => Ok(tee(recv)),
            "pipeTo" => Ok(pipe_to(recv, &args.first().cloned().unwrap_or(Value::Undef))),
            "pipeThrough" => pipe_through(recv, &args),
            _ => unknown(tag, method),
        },
        RS_DEFAULT_READER_TAG => match method {
            "read" => match get_prop(recv, "@@stream") {
                Some(stream) => Ok(stream_read(&stream)),
                None => Ok(rejected(synth("TypeError: reader has no associated stream"))),
            },
            "releaseLock" => {
                reader_release(recv);
                Ok(Value::Undef)
            }
            "cancel" => match get_prop(recv, "@@stream") {
                Some(stream) => Ok(stream_cancel(&stream, args.first().cloned().unwrap_or(Value::Undef))),
                None => Ok(resolved(Value::Undef)),
            },
            _ => unknown(tag, method),
        },
        RS_BYOB_READER_TAG => match method {
            "read" => Ok(byob_read(recv, &args)),
            "releaseLock" => {
                reader_release(recv);
                Ok(Value::Undef)
            }
            "cancel" => match get_prop(recv, "@@stream") {
                Some(stream) => Ok(stream_cancel(&stream, args.first().cloned().unwrap_or(Value::Undef))),
                None => Ok(resolved(Value::Undef)),
            },
            _ => unknown(tag, method),
        },
        RS_DEFAULT_CONTROLLER_TAG | RS_BYTE_CONTROLLER_TAG => {
            let stream = get_prop(recv, "@@stream").unwrap_or(Value::Undef);
            match method {
                "enqueue" => {
                    stream_enqueue(&stream, args.first().cloned().unwrap_or(Value::Undef));
                    Ok(Value::Undef)
                }
                "close" => {
                    stream_close(&stream);
                    Ok(Value::Undef)
                }
                "error" => {
                    stream_error(&stream, args.first().cloned().unwrap_or(Value::Undef));
                    Ok(Value::Undef)
                }
                _ => unknown(tag, method),
            }
        }
        RS_BYOB_REQUEST_TAG => match method {
            // BYOB request is exposed for completeness; respond is a no-op in the
            // synchronous byte model (the reader copies bytes itself).
            "respond" | "respondWithNewView" => Ok(Value::Undef),
            _ => unknown(tag, method),
        },
        WRITABLE_STREAM_TAG => match method {
            "getWriter" => get_writer(recv),
            "close" => match ws_finish(recv) {
                Ok(()) => Ok(resolved(Value::Undef)),
                Err(msg) => Ok(rejected(synth(&msg))),
            },
            "abort" => match ws_abort(recv, args.first().cloned().unwrap_or(Value::Undef)) {
                Ok(()) => Ok(resolved(Value::Undef)),
                Err(msg) => Ok(rejected(synth(&msg))),
            },
            _ => unknown(tag, method),
        },
        WS_DEFAULT_WRITER_TAG => {
            let ws = get_prop(recv, "@@stream").unwrap_or(Value::Undef);
            match method {
                "write" => {
                    if state_of(&ws) == "errored" {
                        return Ok(rejected(get_prop(&ws, "@@stored_error").unwrap_or(Value::Undef)));
                    }
                    match ws_accept_chunk(&ws, args.first().cloned().unwrap_or(Value::Undef)) {
                        Ok(()) => Ok(resolved(Value::Undef)),
                        Err(msg) => Ok(rejected(synth(&msg))),
                    }
                }
                "close" => match ws_finish(&ws) {
                    Ok(()) => Ok(resolved(Value::Undef)),
                    Err(msg) => Ok(rejected(synth(&msg))),
                },
                "abort" => match ws_abort(&ws, args.first().cloned().unwrap_or(Value::Undef)) {
                    Ok(()) => Ok(resolved(Value::Undef)),
                    Err(msg) => Ok(rejected(synth(&msg))),
                },
                "releaseLock" => {
                    writer_release(recv);
                    Ok(Value::Undef)
                }
                _ => unknown(tag, method),
            }
        }
        WS_DEFAULT_CONTROLLER_TAG => match method {
            "error" => {
                let ws = get_prop(recv, "@@stream").unwrap_or(Value::Undef);
                let _ = ws_abort(&ws, args.first().cloned().unwrap_or(Value::Undef));
                Ok(Value::Undef)
            }
            _ => unknown(tag, method),
        },
        TS_DEFAULT_CONTROLLER_TAG => {
            let readable = get_prop(recv, "@@readable").unwrap_or(Value::Undef);
            match method {
                "enqueue" => {
                    stream_enqueue(&readable, args.first().cloned().unwrap_or(Value::Undef));
                    Ok(Value::Undef)
                }
                "terminate" => {
                    stream_close(&readable);
                    Ok(Value::Undef)
                }
                "error" => {
                    let e = args.first().cloned().unwrap_or(Value::Undef);
                    stream_error(&readable, e.clone());
                    if let Some(ws) = get_prop(recv, "@@writable") {
                        let _ = ws_abort(&ws, e);
                    }
                    Ok(Value::Undef)
                }
                _ => unknown(tag, method),
            }
        }
        BYTE_LENGTH_STRATEGY_TAG => match method {
            "size" => Ok(Value::Float(chunk_byte_length(&args.first().cloned().unwrap_or(Value::Undef)))),
            _ => unknown(tag, method),
        },
        COUNT_STRATEGY_TAG => match method {
            "size" => Ok(Value::Float(1.0)),
            _ => unknown(tag, method),
        },
        _ => unknown(tag, method),
    }
}

fn unknown(tag: &str, method: &str) -> Result<Value, String> {
    Err(crate::host::type_error(&format!("{tag}.{method} is not a function")))
}
