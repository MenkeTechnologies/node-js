//! Node `events` module: `EventEmitter`. The emitter is an object tagged
//! `@@native = "EventEmitter"` with hidden `@@on`/`@@once` maps (event name →
//! listener array). `emit` collects listeners, releases the host borrow, then
//! invokes each so callbacks can re-enter the host.

use super::arg_str;
use crate::host::{call_method, invoke, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// Construct a fresh `EventEmitter`.
pub fn new_emitter() -> Value {
    with_host(|h| {
        let on = h.new_object(IndexMap::new());
        let once = h.new_object(IndexMap::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("EventEmitter"));
        m.insert("@@on".into(), on);
        m.insert("@@once".into(), once);
        h.new_object(m)
    })
}

/// The EventEmitter method names, exposed so `EventEmitter.prototype` can be
/// enumerated / copied (express does `mixin(app, EventEmitter.prototype)` to make
/// its `app` *function* an emitter).
pub const METHODS: &[&str] = &[
    "on",
    "addListener",
    "prependListener",
    "once",
    "prependOnceListener",
    "emit",
    "removeListener",
    "off",
    "removeAllListeners",
    "listenerCount",
    "listeners",
    "rawListeners",
    "eventNames",
    "setMaxListeners",
    "getMaxListeners",
];

/// The internal key an event name registers under.
///
/// `ToPropertyKey`, not `String(name)`: a SYMBOL event name is a distinct key
/// (`@@sym:<id>`), so `on(sym, f)` and `on("Symbol(desc)", f)` are different
/// events and `eventNames()` can hand the symbol itself back. Rendering it
/// collapsed the two and made every symbol listener unremovable by its symbol.
fn event_key(args: &[Value]) -> String {
    with_host(|h| h.property_key(args.first().unwrap_or(&Value::Undef)))
}

pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match method {
        // Both the event-name coercion and the listener lookup re-enter the host,
        // so they must resolve BEFORE the `with_host` that allocates the array —
        // nesting them inside it panics with `RefCell already borrowed`. That was
        // latent until `listeners` became reachable on a socket/request/stream.
        // `rawListeners` returns the once-WRAPPERS in node; once-listeners are
        // stored unwrapped here, so the two views coincide.
        "listeners" | "rawListeners" => {
            let items = listeners(recv, &event_key(&args));
            Ok(with_host(|h| h.new_array(items)))
        }
        // The cap is not enforced (nothing here warns on listener count), but it
        // must still read back what was set — `setMaxListeners` used to discard
        // the value and `getMaxListeners` always answered the default 10.
        "setMaxListeners" => {
            let n = args
                .first()
                .map(|v| with_host(|h| h.to_number(v)))
                .unwrap_or(10.0);
            with_host(|h| {
                let nv = Value::Float(n);
                match h.get_mut(recv) {
                    Some(JsObj::Object(p)) => {
                        p.insert("@@maxListeners".into(), nv);
                    }
                    _ => h.set_fn_prop(recv, "@@maxListeners", nv),
                }
            });
            Ok(recv.clone())
        }
        "getMaxListeners" => Ok(with_host(|h| match named_map(h, recv, "@@maxListeners") {
            Some(v @ (Value::Float(_) | Value::Int(_))) => v,
            _ => Value::Float(10.0),
        })),
        "on" | "addListener" | "prependListener" | "once" | "prependOnceListener" => {
            let once = matches!(method, "once" | "prependOnceListener");
            let prepend = method.starts_with("prepend");
            let name = event_key(&args);
            let f = args.get(1).cloned().unwrap_or(Value::Undef);
            // `newListener` fires BEFORE the listener is added, so a handler for
            // it sees the emitter without the new listener and can add its own
            // ahead of it. It was never emitted at all.
            if name != "newListener" && !listeners(recv, "newListener").is_empty() {
                let nv = with_host(|h| h.new_str(name.clone()));
                emit(recv, "newListener", &[nv, f.clone()])?;
            }
            add(
                recv,
                if once { "@@once" } else { "@@on" },
                &name,
                f,
                prepend,
            );
            Ok(recv.clone())
        }
        "emit" => emit(
            recv,
            &event_key(&args),
            &args.get(1..).map(|s| s.to_vec()).unwrap_or_default(),
        ),
        "removeListener" | "off" => {
            let name = event_key(&args);
            let f = args.get(1).cloned();
            let had = f
                .as_ref()
                .is_some_and(|f| listeners(recv, &name).iter().any(|l| l == f));
            remove(recv, &name, f.clone());
            // `removeListener` fires AFTER the removal, and only when one
            // actually happened. It was never emitted at all.
            if had && name != "removeListener" && !listeners(recv, "removeListener").is_empty() {
                let nv = with_host(|h| h.new_str(name.clone()));
                emit(recv, "removeListener", &[nv, f.unwrap_or(Value::Undef)])?;
            }
            Ok(recv.clone())
        }
        "removeAllListeners" => {
            let name = if args.is_empty() {
                None
            } else {
                Some(event_key(&args))
            };
            remove_all(recv, name.as_deref());
            Ok(recv.clone())
        }
        "listenerCount" => Ok(Value::Float(listeners(recv, &event_key(&args)).len() as f64)),
        // A SYMBOL event name comes back as the symbol itself, not as its
        // `Symbol(desc)` rendering — `emitter.on(sym, f)` then `eventNames()`
        // has to hand back something `emitter.off(name, f)` accepts. Strings
        // come first, then symbols, which is the own-key order node reports.
        "eventNames" => Ok(with_host(|h| {
            let mut keys: Vec<String> = Vec::new();
            if let Some(JsObj::Object(p)) = named_map(h, recv, "@@on").and_then(|v| h.get(&v)) {
                keys.extend(p.keys().cloned());
            }
            let (syms, strs): (Vec<String>, Vec<String>) = keys
                .into_iter()
                .partition(|k| crate::host::is_symbol_key(k));
            let mut names: Vec<Value> = strs.into_iter().map(|k| h.new_str(k)).collect();
            names.extend(
                syms.iter()
                    .filter_map(|k| h.symbol_of_key(k))
                    .collect::<Vec<Value>>(),
            );
            h.new_array(names)
        })),
        _ => Err(crate::host::type_error(&format!(
            "emitter.{method} is not a function"
        ))),
    }
}

/// Read a hidden emitter field (`@@on`/`@@once`). Works for a plain emitter
/// object AND for a function/class receiver (express's `app` is a function whose
/// emitter maps live in the fn-prop side table).
fn named_map(h: &crate::host::JsHost, recv: &Value, which: &str) -> Option<Value> {
    match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(which).cloned(),
        Some(JsObj::Func(_)) | Some(JsObj::Class(_)) => h.fn_prop(recv, which),
        _ => None,
    }
}

/// Store a hidden emitter field, routing to props or the fn-prop table.
fn set_named_map(h: &mut crate::host::JsHost, recv: &Value, which: &str, val: Value) {
    match h.get(recv) {
        Some(JsObj::Func(_)) | Some(JsObj::Class(_)) => h.set_fn_prop(recv, which, val),
        _ => {
            if let Some(JsObj::Object(p)) = h.get_mut(recv) {
                p.insert(which.to_string(), val);
            }
        }
    }
}

/// Register `f` for `name`.
///
/// Every listener — `once` included — goes into the single ordered `@@on` list,
/// and `@@once` holds only a MARKER copy of the once-only ones. It used to be
/// two parallel queues that `listeners` concatenated `@@on`-then-`@@once`, so a
/// once-listener always fired last no matter when it was registered:
/// `e.once('a', first); e.on('a', second)` ran `second` first, and
/// `prependOnceListener` could not reach the front at all.
fn add(recv: &Value, which: &str, name: &str, f: Value, prepend: bool) {
    if which == "@@once" {
        // The marker records once-ness; order within it is never observed.
        add_to_list(recv, "@@once", name, f.clone(), false);
    }
    add_to_list(recv, "@@on", name, f, prepend);
}

fn add_to_list(recv: &Value, which: &str, name: &str, f: Value, prepend: bool) {
    with_host(|h| {
        // Lazily create the listener map (a mixed-in function emitter has none).
        let map = match named_map(h, recv, which) {
            Some(m) => m,
            None => {
                let m = h.new_object(IndexMap::new());
                set_named_map(h, recv, which, m.clone());
                m
            }
        };
        // Ensure `map[name]` is an array, then push.
        let arr = match h.get(&map) {
            Some(JsObj::Object(p)) => p.get(name).cloned(),
            _ => None,
        };
        let arr = match arr {
            Some(a) if matches!(h.get(&a), Some(JsObj::Array(_))) => a,
            _ => {
                let a = h.new_array(Vec::new());
                if let Some(JsObj::Object(p)) = h.get_mut(&map) {
                    p.insert(name.to_string(), a.clone());
                }
                a
            }
        };
        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
            // `prependListener` puts the handler FIRST; it was appending like
            // `on`, so the two were indistinguishable.
            if prepend {
                items.insert(0, f);
            } else {
                items.push(f);
            }
        }
    });
}

/// Drop the FIRST entry equal to `f` from one listener list, leaving any
/// duplicate registrations of the same function in place.
fn remove_first(recv: &Value, which: &str, name: &str, f: &Value) {
    with_host(|h| {
        let Some(map) = named_map(h, recv, which) else {
            return;
        };
        let arr = match h.get(&map) {
            Some(JsObj::Object(p)) => p.get(name).cloned(),
            _ => None,
        };
        let mut emptied = false;
        if let Some(JsObj::Array(items)) = arr.and_then(|a| h.get_mut(&a)) {
            if let Some(i) = items.iter().position(|x| x == f) {
                items.remove(i);
            }
            emptied = items.is_empty();
        }
        // An emptied list must take its KEY with it, or `eventNames()` keeps
        // reporting an event nothing is listening for.
        if emptied {
            if let Some(JsObj::Object(p)) = h.get_mut(&map) {
                p.shift_remove(name);
            }
        }
    });
}

fn listeners(recv: &Value, name: &str) -> Vec<Value> {
    with_host(|h| {
        // `@@on` alone: it holds every listener in registration order, and
        // `@@once` is only a marker copy of some of them.
        let mut out = Vec::new();
        if let Some(map) = named_map(h, recv, "@@on") {
            if let Some(JsObj::Object(p)) = h.get(&map) {
                if let Some(a) = p.get(name) {
                    if let Some(JsObj::Array(items)) = h.get(a) {
                        out.extend(items.iter().cloned());
                    }
                }
            }
        }
        out
    })
}

fn emit(recv: &Value, name: &str, args: &[Value]) -> Result<Value, String> {
    let to_call = listeners(recv, name);
    // An `error` event with no listener THROWS rather than being dropped. This
    // is how node surfaces a failed socket, stream or request, and swallowing
    // it turned every such failure into silence.
    if name == "error" && to_call.is_empty() {
        let err = args.first().cloned().unwrap_or(Value::Undef);
        if matches!(err, Value::Undef) {
            return Err(crate::host::plain_coded_error(
                "Error",
                "ERR_UNHANDLED_ERROR",
                "Unhandled error.",
            ));
        }
        let msg = with_host(|h| {
            h.exc = Some(err.clone());
            crate::builtins::error_string(h, &err)
        });
        return Err(msg);
    }
    // Once-listeners fire a single time. They live in BOTH lists now, so
    // clearing the marker also has to drop one matching entry apiece from the
    // ordered list — one, not all, so a function registered with `on` AND
    // `once` keeps its `on` registration, as in node.
    let expired = remove_all_of(recv, "@@once", Some(name));
    for f in &expired {
        remove_first(recv, "@@on", name, f);
    }
    let had = !to_call.is_empty();
    for f in to_call {
        invoke(&f, args.to_vec(), Some(recv.clone()))?;
    }
    // Settle any `events.once(emitter, name)` promise waiters for this event.
    resolve_waiters(recv, name, args);
    Ok(Value::Bool(had))
}

// ── `events.once` promise waiters ───────────────────────────────────────────
//
// `once(emitter, name)` returns a real Promise. We cannot register a Rust
// closure as a JS listener (listeners must be callable Values), so instead a
// pending promise is parked under the emitter's hidden `@@waiters` map keyed by
// event name; `emit` (above) settles them. On `error`, waiters of every other
// event reject with the error, mirroring Node's `once` semantics.

/// Park `promise` to be resolved when `name` next fires on `recv`.
fn add_waiter(recv: &Value, name: &str, promise: Value) {
    with_host(|h| {
        let map = match waiter_map(h, recv) {
            Some(m) => m,
            None => {
                let m = h.new_object(IndexMap::new());
                if let Some(JsObj::Object(p)) = h.get_mut(recv) {
                    p.insert("@@waiters".into(), m.clone());
                }
                m
            }
        };
        let existing = match h.get(&map) {
            Some(JsObj::Object(p)) => p.get(name).cloned(),
            _ => None,
        };
        let arr = match existing {
            Some(a) if matches!(h.get(&a), Some(JsObj::Array(_))) => a,
            _ => {
                let a = h.new_array(Vec::new());
                if let Some(JsObj::Object(p)) = h.get_mut(&map) {
                    p.insert(name.to_string(), a.clone());
                }
                a
            }
        };
        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
            items.push(promise);
        }
    });
}

fn waiter_map(h: &crate::host::JsHost, recv: &Value) -> Option<Value> {
    match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@waiters").cloned(),
        _ => None,
    }
}

/// Remove and return the promises parked on `name`.
fn take_waiters(recv: &Value, name: &str) -> Vec<Value> {
    with_host(|h| {
        let Some(map) = waiter_map(h, recv) else {
            return Vec::new();
        };
        let arr = match h.get_mut(&map) {
            Some(JsObj::Object(p)) => p.shift_remove(name),
            _ => None,
        };
        let Some(arr) = arr else { return Vec::new() };
        match h.get(&arr) {
            Some(JsObj::Array(items)) => items.clone(),
            _ => Vec::new(),
        }
    })
}

/// Remove and return every parked promise except those on `keep`.
fn take_waiters_except(recv: &Value, keep: &str) -> Vec<Value> {
    with_host(|h| {
        let Some(map) = waiter_map(h, recv) else {
            return Vec::new();
        };
        let keys: Vec<String> = match h.get(&map) {
            Some(JsObj::Object(p)) => p.keys().filter(|k| k.as_str() != keep).cloned().collect(),
            _ => Vec::new(),
        };
        let mut out = Vec::new();
        for k in keys {
            let arr = match h.get_mut(&map) {
                Some(JsObj::Object(p)) => p.shift_remove(&k),
                _ => None,
            };
            if let Some(arr) = arr {
                if let Some(JsObj::Array(items)) = h.get(&arr) {
                    out.extend(items.iter().cloned());
                }
            }
        }
        out
    })
}

fn resolve_waiters(recv: &Value, name: &str, args: &[Value]) {
    let waiting = take_waiters(recv, name);
    if !waiting.is_empty() {
        let arr = with_host(|h| h.new_array(args.to_vec()));
        for p in &waiting {
            if let Some(id) = with_host(|h| h.promise_id(p)) {
                crate::host::resolve_promise_val(id, arr.clone());
            }
        }
    }
    if name == "error" {
        let err = args.first().cloned().unwrap_or(Value::Undef);
        for p in take_waiters_except(recv, "error") {
            if let Some(id) = with_host(|h| h.promise_id(&p)) {
                crate::host::reject_promise_val(id, err.clone());
            }
        }
    }
}

// ── static module functions (`require('events').once`, `.listenerCount`, …) ──

/// Static functions on the `events` module namespace. `EventEmitter` (the
/// self-ref ctor) and `EventEmitterAsyncResource` are handled by the parent;
/// `on` (async iterator) is deferred (see module docs / final report).
pub const STATIC_METHODS: &[&str] = &[
    "once",
    "listenerCount",
    "getEventListeners",
    "getMaxListeners",
    "setMaxListeners",
    "addAbortListener",
    "init",
];

/// Dispatch a static `events.<method>(...)`. Returns `None` for names this
/// module does not own (e.g. `EventEmitter`) so the parent's specific arm wins.
pub fn static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let emitter = args.first().cloned().unwrap_or(Value::Undef);
    Some(match method {
        "once" => Ok(once_static(emitter, &arg_str(args, 1))),
        "listenerCount" => Ok(Value::Float(
            listeners(&emitter, &arg_str(args, 1)).len() as f64
        )),
        // `listeners` takes the host, so calling it INSIDE `with_host` borrowed
        // the same RefCell twice and aborted the process with "RefCell already
        // borrowed" — a Rust panic, not a throw, so no JS `try` caught it.
        // Collect first, then borrow to build the array.
        "getEventListeners" => {
            let found = listeners(&emitter, &arg_str(args, 1));
            Ok(with_host(|h| h.new_array(found)))
        }
        // No per-emitter cap is tracked; report Node's default and accept sets.
        "getMaxListeners" => Ok(Value::Float(10.0)),
        "setMaxListeners" => Ok(Value::Undef),
        "addAbortListener" => Ok(add_abort_listener(args)),
        "init" => Ok(init_emitter(emitter)),
        _ => return None,
    })
}

/// `events.once(emitter, name)` → a Promise resolving with the event args (or
/// rejecting with the error if `error` fires first).
fn once_static(emitter: Value, name: &str) -> Value {
    let p = with_host(|h| h.new_promise());
    add_waiter(&emitter, name, p.clone());
    p
}

/// `EventEmitter.init(emitter)` — ensure the hidden emitter maps exist on
/// `emitter` (used when mixing the emitter surface into a plain object).
fn init_emitter(emitter: Value) -> Value {
    with_host(|h| {
        let has = matches!(h.get(&emitter), Some(JsObj::Object(p)) if p.contains_key("@@on"));
        if !has {
            let on = h.new_object(IndexMap::new());
            let once = h.new_object(IndexMap::new());
            let native = h.new_str("EventEmitter");
            if let Some(JsObj::Object(p)) = h.get_mut(&emitter) {
                p.entry("@@native".to_string()).or_insert(native);
                p.insert("@@on".to_string(), on);
                p.insert("@@once".to_string(), once);
            }
        }
    });
    emitter
}

/// `events.addAbortListener(signal, listener)` — best-effort: register a
/// one-time `abort` listener if `signal` is emitter-like. `AbortSignal` is not
/// modeled natively, so this is a no-op for plain signals. Returns a disposable
/// placeholder object.
fn add_abort_listener(args: &[Value]) -> Value {
    let signal = args.first().cloned().unwrap_or(Value::Undef);
    let listener = args.get(1).cloned().unwrap_or(Value::Undef);
    let name = with_host(|h| h.new_str("abort"));
    let _ = call_method(&signal, "once", vec![name, listener]);
    with_host(|h| h.new_object(IndexMap::new()))
}

fn remove(recv: &Value, name: &str, f: Option<Value>) {
    let Some(f) = f else { return };
    with_host(|h| {
        for which in ["@@on", "@@once"] {
            if let Some(map) = named_map(h, recv, which) {
                let arr = match h.get(&map) {
                    Some(JsObj::Object(p)) => p.get(name).cloned(),
                    _ => None,
                };
                if let Some(a) = arr {
                    let now_empty = if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                        if let Some(pos) = items.iter().position(|x| x == &f) {
                            items.remove(pos);
                        }
                        items.is_empty()
                    } else {
                        false
                    };
                    // Node drops an event key once its last listener is removed,
                    // so `eventNames()` no longer lists it.
                    if now_empty {
                        if let Some(JsObj::Object(p)) = h.get_mut(&map) {
                            p.shift_remove(name);
                        }
                    }
                }
            }
        }
    });
}

fn remove_all(recv: &Value, name: Option<&str>) {
    remove_all_of(recv, "@@on", name);
    remove_all_of(recv, "@@once", name);
}

/// Drop a whole listener list (or every list), returning what was in it so the
/// caller can mirror the removal into the other map.
fn remove_all_of(recv: &Value, which: &str, name: Option<&str>) -> Vec<Value> {
    with_host(|h| {
        let mut dropped = Vec::new();
        if let Some(map) = named_map(h, recv, which) {
            let arrays: Vec<Value> = match h.get(&map) {
                Some(JsObj::Object(p)) => match name {
                    Some(n) => p.get(n).cloned().into_iter().collect(),
                    None => p.values().cloned().collect(),
                },
                _ => Vec::new(),
            };
            for a in arrays {
                if let Some(JsObj::Array(items)) = h.get(&a) {
                    dropped.extend(items.iter().cloned());
                }
            }
            if let Some(JsObj::Object(p)) = h.get_mut(&map) {
                match name {
                    Some(n) => {
                        p.shift_remove(n);
                    }
                    None => p.clear(),
                }
            }
        }
        dropped
    })
}
