//! Node `readline` module.
//!
//! * `'line'` and `'close'` events, and `for await (const line of rl)`: an
//!   Interface over `process.stdin` reads it in chunks from a macrotask pump
//!   (started by the first `'line'`/`'close'` listener or async iterator); over
//!   any other stream it listens to the stream's `'data'`/`'end'`. Lines split
//!   on `\r\n`, `\n` or a lone `\r`, an unterminated last line is delivered at
//!   end of input, then `'close'`. See "line events" below for the ordering.
//! * `interface.question(query, cb)` writes `query` to stdout and, before any
//!   line events have started, reads exactly ONE line from stdin synchronously
//!   and invokes `cb(line)`; once they have, the answer is the next line the
//!   pump delivers. `readline/promises` resolves a promise instead.
//! * `interface.write(data)` writes to the output; `prompt()` writes the stored
//!   prompt; `setPrompt`/`getPrompt` manage it; the module cursor helpers
//!   (`cursorTo`/`moveCursor`/`clearLine`/`clearScreenDown`) emit the
//!   corresponding ANSI control sequences to stdout.
//! * Not modelled: terminal mode (keypress decoding, history, line editing) —
//!   input is read as lines whatever it is attached to — and `pause()`/
//!   `resume()`, which return the Interface and change nothing.
//!
//! An Interface is a plain object tagged `@@native = "Interface"` carrying the
//! passed `@@input`/`@@output` streams, the current `@@prompt`, a hidden
//! `@@listeners` object of registered event callbacks, and the `@@rlid` that
//! keys its line state.

use crate::host::{is_callable, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};

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
    "@@asyncIterator",
];

/// The `readline/promises` surface: the same module, whose `createInterface`
/// builds an Interface with a promise-returning `question`.
pub const PROMISES_METHODS: &[&str] = &["createInterface"];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    if method.starts_with("@@") {
        return internal_call(method, args);
    }
    Some(match method {
        "createInterface" => Ok(create_interface(args, false)),
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
    Ok(create_interface(args, false))
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
/// `require('readline/promises').<method>`.
pub fn promises_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "createInterface" => Some(Ok(create_interface(args, true))),
        _ => None,
    }
}

fn create_interface(args: &[Value], promises: bool) -> Value {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
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
    let iface = with_host(|h| {
        let listeners = h.new_object(IndexMap::new());
        let prompt = h.new_str("> ");
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Interface"));
        m.insert("@@input".into(), input.clone());
        m.insert("@@output".into(), output);
        m.insert("@@prompt".into(), prompt);
        m.insert("@@listeners".into(), listeners);
        m.insert("@@rlid".into(), Value::Float(id as f64));
        if promises {
            let flag = h.new_str("1");
            m.insert("@@promises".into(), flag);
        }
        h.new_object(m)
    });
    if matches!(input, Value::Obj(_)) && !is_stdin(&input) {
        attach_stream(&iface, &input);
    }
    iface
}

/// Dispatch a method on an Interface instance (`@@native = "Interface"`).
pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match method {
        // REAL synchronous single-line read: write the query, read one stdin line,
        // invoke the callback with it. Returns undefined (Node's callback form).
        "question" => {
            let query = with_host(|h| args.first().map(|v| h.str_of(v)).unwrap_or_default());
            write_stdout(&query);
            // Once input is flowing as line events, the answer is the next
            // line the stream delivers rather than a read of its own.
            if with_state(recv, |s| s.pumping) {
                if read_hidden(recv, "@@promises") == "1" {
                    let (promise, id) = with_host(|h| {
                        let p = h.new_promise();
                        let id = h.promise_id(&p).unwrap_or(0);
                        (p, id)
                    });
                    with_state(recv, |s| s.question = Some(QuestionReply::Promise(id)));
                    return Ok(promise);
                }
                let cb = args
                    .iter()
                    .rev()
                    .find(|v| with_host(|h| is_callable(h, v)))
                    .cloned();
                if let Some(cb) = cb {
                    with_state(recv, |s| s.question = Some(QuestionReply::Callback(cb)));
                }
                return Ok(Value::Undef);
            }
            let line = read_line();
            // `require('readline/promises')` builds an Interface whose
            // `question` RESOLVES with the line instead of taking a callback.
            // The two forms are the same read; only the handoff differs.
            if read_hidden(recv, "@@promises") == "1" {
                let line_val = with_host(|h| h.new_str(line));
                return crate::builtins::promise_resolve_pub(line_val);
            }
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
            write_output(recv, &data);
            Ok(Value::Undef)
        }
        "prompt" => {
            let p = read_hidden(recv, "@@prompt");
            write_output(recv, &p);
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
        // `read_hidden` takes the host, so reading it INSIDE `with_host` borrowed
        // the same RefCell twice and aborted the process — a Rust panic, not a
        // throw, so no JS `try` could catch it. Read first, then borrow.
        "getPrompt" => {
            let prompt = read_hidden(recv, "@@prompt");
            Ok(with_host(|h| h.new_str(prompt)))
        }
        // Listener registration, under `@@listeners[event]`. A `'line'` or
        // `'close'` listener on standard input starts the pump that feeds them.
        "on" | "once" | "addListener" | "prependListener" => {
            if let (Some(ev), Some(cb)) = (args.first(), args.get(1)) {
                let event = with_host(|h| h.str_of(ev));
                store_listener(
                    recv,
                    &event,
                    cb.clone(),
                    method == "once",
                    method == "prependListener",
                );
                if event == "line" || event == "close" {
                    ensure_flowing(recv);
                }
            }
            Ok(recv.clone())
        }
        "removeListener" | "off" => {
            if let (Some(ev), Some(cb)) = (args.first(), args.get(1)) {
                let event = with_host(|h| h.str_of(ev));
                remove_listener(recv, &event, cb);
            }
            Ok(recv.clone())
        }
        "removeAllListeners" => {
            let event = args
                .first()
                .filter(|v| !matches!(v, Value::Undef))
                .map(|v| with_host(|h| h.str_of(v)));
            clear_listeners(recv, event.as_deref());
            Ok(recv.clone())
        }
        "close" => {
            close(recv)?;
            Ok(Value::Undef)
        }
        "@@asyncIterator" => Ok(async_iterator(recv)),
        // Reading is driven by the pump and the stream's own events; there is
        // no separate flow switch to flip.
        "pause" | "resume" => Ok(recv.clone()),
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

/// The `@@listeners` object of `recv`.
fn listeners_obj(recv: &Value) -> Option<Value> {
    opt_prop(recv, "@@listeners")
}

/// Add `cb` to `recv`'s `@@listeners[event]`, an array of `[callback, once]`
/// pairs (created on demand); `prepend` puts it first.
fn store_listener(recv: &Value, event: &str, cb: Value, once: bool, prepend: bool) {
    let Some(listeners) = listeners_obj(recv) else {
        return;
    };
    with_host(|h| {
        let entry = h.new_array(vec![cb, Value::Bool(once)]);
        let arr = match h.get(&listeners) {
            Some(JsObj::Object(p)) => p.get(event).cloned(),
            _ => None,
        };
        let arr = arr.filter(|a| matches!(h.get(a), Some(JsObj::Array(_))));
        match arr {
            Some(a) => {
                if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                    if prepend {
                        items.insert(0, entry);
                    } else {
                        items.push(entry);
                    }
                }
            }
            None => {
                let a = h.new_array(vec![entry]);
                if let Some(JsObj::Object(p)) = h.get_mut(&listeners) {
                    p.insert(event.to_string(), a);
                }
            }
        }
    });
}

/// The `(callback, once)` pairs registered for `event`, in call order.
fn listener_entries(recv: &Value, event: &str) -> Vec<(Value, bool)> {
    let Some(listeners) = listeners_obj(recv) else {
        return Vec::new();
    };
    with_host(|h| {
        let arr = match h.get(&listeners) {
            Some(JsObj::Object(p)) => p.get(event).cloned(),
            _ => None,
        };
        let Some(Some(JsObj::Array(items))) = arr.map(|a| h.get(&a).cloned()) else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|e| match h.get(e) {
                Some(JsObj::Array(pair)) if pair.len() == 2 => {
                    Some((pair[0].clone(), h.truthy(&pair[1])))
                }
                _ => None,
            })
            .collect()
    })
}

/// Remove the first registration of `cb` for `event`.
fn remove_listener(recv: &Value, event: &str, cb: &Value) {
    let Some(listeners) = listeners_obj(recv) else {
        return;
    };
    with_host(|h| {
        let arr = match h.get(&listeners) {
            Some(JsObj::Object(p)) => p.get(event).cloned(),
            _ => None,
        };
        let Some(arr) = arr else { return };
        let pos = match h.get(&arr) {
            Some(JsObj::Array(items)) => items.iter().position(|e| match h.get(e) {
                Some(JsObj::Array(pair)) => pair.first().is_some_and(|f| h.strict_eq(f, cb)),
                _ => false,
            }),
            _ => None,
        };
        if let (Some(i), Some(JsObj::Array(items))) = (pos, h.get_mut(&arr)) {
            items.remove(i);
        }
    });
}

/// `removeAllListeners([event])`.
fn clear_listeners(recv: &Value, event: Option<&str>) {
    let Some(listeners) = listeners_obj(recv) else {
        return;
    };
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(&listeners) {
            match event {
                Some(e) => {
                    p.shift_remove(e);
                }
                None => p.clear(),
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

/// Write to the interface's configured `output` stream, falling back to stdout
/// when it has none.
///
/// `createInterface({input, output})` records `@@output` and Node writes
/// `prompt()` and `write()` THROUGH it — the option exists so a caller can
/// capture or redirect that text. Both went straight to `io::stdout()` here, so
/// an interface given its own output printed to the process's stdout anyway and
/// the supplied stream never saw a byte.
fn write_output(recv: &Value, s: &str) {
    let out = opt_prop(recv, "@@output").unwrap_or(Value::Undef);
    if matches!(out, Value::Obj(_)) {
        let payload = with_host(|h| h.new_str(s.to_string()));
        if crate::host::call_method(&out, "write", vec![payload]).is_ok() {
            return;
        }
    }
    write_stdout(s);
}

/// An own property of `v` if `v` is a plain object, else `None`.
fn opt_prop(v: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

// ── line events ──────────────────────────────────────────────────────────────
//
// An Interface splits its input into lines and emits `'line'` for each, then
// `'close'` when the input ends, as node's does. Standard input is read by a
// PUMP: a macrotask that reads one chunk (blocking, up to 64 KiB, through
// Rust's shared stdin buffer so `question` and this never skip each other's
// bytes) and re-arms itself until end of input. Any other input is a stream
// object, whose `'data'`/`'end'` events are listened to. A chunk's lines are
// emitted synchronously, one after another, as node emits them.

/// Per-Interface line state, keyed by the Interface's `@@rlid`.
#[derive(Default)]
struct LineState {
    /// Bytes received but not yet ended by a line break.
    pending: Vec<u8>,
    /// The last chunk ended in `\r`, so a `\n` opening the next one is part of
    /// that line break rather than an empty line.
    after_cr: bool,
    /// Complete lines not yet emitted.
    to_emit: VecDeque<String>,
    /// A chain of line emissions is in progress.
    emitting: bool,
    /// The input has ended: `'close'` follows the last line.
    ended: bool,
    closed: bool,
    /// The stdin pump has been started.
    pumping: bool,
    /// Async iteration: lines no `next()` has taken yet, and the promises of
    /// the `next()` calls waiting for one.
    iterating: bool,
    buffered: VecDeque<String>,
    waiters: VecDeque<u32>,
    /// A `question` waiting for the next line: its callback, or (for
    /// `readline/promises`) the id of the promise it returned.
    question: Option<QuestionReply>,
}

enum QuestionReply {
    Callback(Value),
    Promise(u32),
}

thread_local! {
    static LINES: std::cell::RefCell<HashMap<u64, LineState>> =
        std::cell::RefCell::new(HashMap::new());
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn rl_id(recv: &Value) -> u64 {
    opt_prop(recv, "@@rlid")
        .map(|v| with_host(|h| h.to_number(&v)) as u64)
        .unwrap_or(0)
}

fn with_state<R>(recv: &Value, f: impl FnOnce(&mut LineState) -> R) -> R {
    let id = rl_id(recv);
    LINES.with(|m| f(m.borrow_mut().entry(id).or_default()))
}

/// Whether `input` is `process.stdin`.
fn is_stdin(input: &Value) -> bool {
    with_host(|h| match h.get(input) {
        Some(JsObj::Object(p)) => {
            p.get("@@native").map(|v| h.str_of(v)).as_deref() == Some("WriteStream")
                && p.get("fd").map(|v| h.to_number(v)) == Some(0.0)
        }
        _ => false,
    })
}

/// Start reading standard input, once, if that is this Interface's input.
fn ensure_flowing(recv: &Value) {
    let input = opt_prop(recv, "@@input").unwrap_or(Value::Undef);
    if !is_stdin(&input) {
        return;
    }
    let start = with_state(recv, |s| {
        !std::mem::replace(&mut s.pumping, true) && !s.closed
    });
    if start {
        schedule_pump(recv);
    }
}

fn schedule_pump(recv: &Value) {
    with_host(|h| {
        let cb = h.alloc(JsObj::Builtin("readline.@@pump".into()));
        h.add_timer(-1.0, cb, vec![recv.clone()], None);
    });
}

/// One turn of the stdin pump.
fn pump(recv: &Value) -> Result<(), String> {
    if with_state(recv, |s| s.closed) {
        return Ok(());
    }
    let mut buf = vec![0u8; 65536];
    let n = io::stdin().lock().read(&mut buf).unwrap_or(0);
    if n == 0 {
        return on_end(recv);
    }
    schedule_pump(recv);
    on_data(recv, &buf[..n])
}

/// Split a chunk into lines on `\r\n`, `\n` or a lone `\r`, keeping the
/// unterminated tail for the next chunk.
fn on_data(recv: &Value, bytes: &[u8]) -> Result<(), String> {
    let start = with_state(recv, |s| {
        let mut i = 0;
        if s.after_cr && bytes.first() == Some(&b'\n') {
            i = 1;
        }
        s.after_cr = false;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => {
                    let line = String::from_utf8_lossy(&s.pending).into_owned();
                    s.pending.clear();
                    s.to_emit.push_back(line);
                }
                b'\r' => {
                    let line = String::from_utf8_lossy(&s.pending).into_owned();
                    s.pending.clear();
                    s.to_emit.push_back(line);
                    match bytes.get(i + 1) {
                        Some(b'\n') => i += 1,
                        None => s.after_cr = true,
                        _ => {}
                    }
                }
                b => s.pending.push(b),
            }
            i += 1;
        }
        !s.to_emit.is_empty() && !std::mem::replace(&mut s.emitting, true)
    });
    if start {
        emit_lines(recv)?;
    }
    Ok(())
}

/// The input ended: what is left unterminated is the last line, then `'close'`.
fn on_end(recv: &Value) -> Result<(), String> {
    let (start, close_now) = with_state(recv, |s| {
        s.ended = true;
        if !s.pending.is_empty() {
            let line = String::from_utf8_lossy(&s.pending).into_owned();
            s.pending.clear();
            s.to_emit.push_back(line);
        }
        let start = !s.to_emit.is_empty() && !std::mem::replace(&mut s.emitting, true);
        (start, s.to_emit.is_empty() && !s.emitting)
    });
    if start {
        emit_lines(recv)?;
    } else if close_now {
        close(recv)?;
    }
    Ok(())
}

/// Emit every queued line, in order and synchronously — node emits a chunk's
/// lines in one go, so the microtasks a `'line'` listener queues run after the
/// whole chunk — then `'close'` if the input has ended. A line arriving while
/// this runs (a listener feeding the input) joins the same loop.
fn emit_lines(recv: &Value) -> Result<(), String> {
    while let Some(line) = with_state(recv, |s| s.to_emit.pop_front()) {
        if let Err(e) = deliver(recv, line) {
            with_state(recv, |s| s.emitting = false);
            return Err(e);
        }
    }
    let close_now = with_state(recv, |s| {
        s.emitting = false;
        s.ended
    });
    if close_now {
        close(recv)?;
    }
    Ok(())
}

/// Hand one line to a waiting `question`, or else to the `'line'` listeners
/// and any async iterator.
fn deliver(recv: &Value, line: String) -> Result<(), String> {
    match with_state(recv, |s| s.question.take()) {
        Some(QuestionReply::Callback(cb)) => {
            let v = with_host(|h| h.new_str(line));
            crate::host::invoke(&cb, vec![v], None)?;
            return Ok(());
        }
        Some(QuestionReply::Promise(id)) => {
            let v = with_host(|h| h.new_str(line));
            crate::host::resolve_promise_val(id, v);
            return Ok(());
        }
        None => {}
    }
    let v = with_host(|h| h.new_str(line.clone()));
    emit(recv, "line", vec![v.clone()])?;
    let waiter = with_state(recv, |s| {
        if !s.iterating {
            return None;
        }
        let w = s.waiters.pop_front();
        if w.is_none() {
            s.buffered.push_back(line);
        }
        w
    });
    if let Some(id) = waiter {
        crate::host::resolve_promise_val(id, iter_result(v, false));
    }
    Ok(())
}

/// `rl.close()`, and the end of input: emit `'close'` once and finish any
/// async iteration.
fn close(recv: &Value) -> Result<(), String> {
    let waiters = with_state(recv, |s| {
        if std::mem::replace(&mut s.closed, true) {
            return None;
        }
        Some(std::mem::take(&mut s.waiters))
    });
    let Some(waiters) = waiters else {
        return Ok(());
    };
    emit(recv, "close", Vec::new())?;
    for id in waiters {
        crate::host::resolve_promise_val(id, iter_result(Value::Undef, true));
    }
    Ok(())
}

/// Call the listeners registered for `event`, `once` ones removed first.
fn emit(recv: &Value, event: &str, args: Vec<Value>) -> Result<(), String> {
    let entries = listener_entries(recv, event);
    for (cb, once) in entries {
        if once {
            remove_listener(recv, event, &cb);
        }
        crate::host::invoke(&cb, args.clone(), Some(recv.clone()))?;
    }
    Ok(())
}

/// `{ value, done }`.
fn iter_result(value: Value, done: bool) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("value".into(), value);
        m.insert("done".into(), Value::Bool(done));
        h.new_object(m)
    })
}

/// Module-internal entry points reached through `readline.@@…` builtins: the
/// stdin pump (a timer callback) and the listeners attached to a stream input.
fn internal_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let recv = args.first().cloned().unwrap_or(Value::Undef);
    let r = match method {
        "@@pump" => pump(&recv),
        "@@feed" => {
            let chunk = args.get(1).cloned().unwrap_or(Value::Undef);
            let bytes = super::buffer::view_bytes(&chunk)
                .unwrap_or_else(|| with_host(|h| h.str_of(&chunk)).into_bytes());
            on_data(&recv, &bytes)
        }
        "@@end" => on_end(&recv),
        "@@close" => close(&recv),
        _ => return None,
    };
    Some(r.map(|_| Value::Undef))
}

/// Listen to a stream input's `'data'` and `'end'`, as node's Interface does
/// from the moment it is created.
fn attach_stream(recv: &Value, input: &Value) {
    let has_on = crate::builtins::get_property(input, "on")
        .map(|f| with_host(|h| is_callable(h, &f)))
        .unwrap_or(false);
    if !has_on {
        return;
    }
    for (event, internal) in [("data", "readline.@@feed"), ("end", "readline.@@end")] {
        let (name, cb) = with_host(|h| {
            let target = h.alloc(JsObj::Builtin(internal.into()));
            let undef = Value::Undef;
            let cb = h.alloc(JsObj::BoundFunc {
                target,
                this: undef,
                args: vec![recv.clone()],
            });
            (h.new_str(event), cb)
        });
        let _ = crate::host::call_method(input, "on", vec![name, cb]);
    }
}

/// The async iterator `for await (const line of rl)` reads: it takes the lines
/// emitted from the moment it is created, and ends at `'close'`. Leaving the
/// loop early (`return()`) closes the Interface, as in node.
pub const ITERATOR_METHODS: &[&str] = &["next", "return", "@@asyncIterator"];

fn async_iterator(recv: &Value) -> Value {
    with_state(recv, |s| s.iterating = true);
    ensure_flowing(recv);
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("ReadlineIterator"));
        m.insert("@@iface".into(), recv.clone());
        h.new_object(m)
    })
}

pub fn iterator_call(recv: &Value, method: &str) -> Result<Value, String> {
    let iface = opt_prop(recv, "@@iface").unwrap_or(Value::Undef);
    match method {
        "@@asyncIterator" => Ok(recv.clone()),
        "next" => {
            let ready = with_state(&iface, |s| match s.buffered.pop_front() {
                Some(line) => Some(Some(line)),
                None if s.closed => Some(None),
                None => None,
            });
            match ready {
                Some(Some(line)) => {
                    let v = with_host(|h| h.new_str(line));
                    crate::builtins::promise_resolve_pub(iter_result(v, false))
                }
                Some(None) => crate::builtins::promise_resolve_pub(iter_result(Value::Undef, true)),
                None => {
                    let (promise, id) = with_host(|h| {
                        let p = h.new_promise();
                        let id = h.promise_id(&p).unwrap_or(0);
                        (p, id)
                    });
                    with_state(&iface, |s| s.waiters.push_back(id));
                    Ok(promise)
                }
            }
        }
        // Leaving the loop closes the Interface on the next tick, so the code
        // after the loop runs before `'close'` fires, as in node.
        "return" => {
            with_host(|h| {
                let cb = h.alloc(JsObj::Builtin("readline.@@close".into()));
                h.queue_nexttick(cb, vec![iface.clone()]);
            });
            crate::builtins::promise_resolve_pub(iter_result(Value::Undef, true))
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}
