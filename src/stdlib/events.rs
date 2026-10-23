//! Node `events` module: `EventEmitter`. The emitter is an object tagged
//! `@@native = "EventEmitter"` with a hidden `@@on` map (event name → the
//! ordered RAW listener array: a `once` registration is stored as node's
//! `_onceWrap` wrapper, whose `.listener` is the function registered). `emit` collects listeners, releases the host borrow, then
//! invokes each so callbacks can re-enter the host.

use super::arg_str;
use crate::host::{call_method, invoke, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// Construct a fresh `EventEmitter`.
pub fn new_emitter() -> Value {
    with_host(|h| {
        let on = h.new_object(IndexMap::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("EventEmitter"));
        m.insert("@@on".into(), on);
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
        // `rawListeners` is the stored list, once-WRAPPERS included;
        // `listeners` unwraps each one (`unwrapListeners`).
        "rawListeners" => {
            let items = listeners(recv, &event_key(&args));
            Ok(with_host(|h| h.new_array(items)))
        }
        "listeners" => {
            let items = unwrapped_listeners(recv, &event_key(&args))?;
            Ok(with_host(|h| h.new_array(items)))
        }
        // The cap is not enforced (nothing here warns on listener count), but it
        // must still read back what was set — `setMaxListeners` used to discard
        // the value and `getMaxListeners` always answered the default 10.
        "setMaxListeners" => {
            // `validateNumber(n, 'setMaxListeners', 0)`.
            let arg = args.first().cloned().unwrap_or(Value::Undef);
            let n = match arg {
                Value::Float(n) => n,
                Value::Int(n) => n as f64,
                _ => {
                    return Err(crate::host::invalid_arg_type(
                        "setMaxListeners",
                        "argument",
                        "number",
                        &arg,
                    ))
                }
            };
            if n.is_nan() || n < 0.0 {
                return Err(crate::host::coded_error(
                    "RangeError",
                    "ERR_OUT_OF_RANGE",
                    &format!(
                        "The value of \"setMaxListeners\" is out of range. It must be >= 0. Received {}",
                        crate::host::fmt_number(n)
                    ),
                ));
            }
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
            let event = args.first().cloned().unwrap_or(Value::Undef);
            let mut f = args.get(1).cloned().unwrap_or(Value::Undef);
            check_listener(&f)?;
            // `once` registers `_onceWrap(this, type, listener)` in the
            // listener's place, so `rawListeners` sees the wrapper.
            if once {
                f = once_wrap(recv, &event, &f)?;
            }
            // `newListener` fires BEFORE the listener is added, so a handler for
            // it sees the emitter without the new listener and can add its own
            // ahead of it. It is handed `listener.listener ?? listener`.
            if name != "newListener" && !listeners(recv, "newListener").is_empty() {
                let shown = original_listener(&f)?;
                emit(recv, "newListener", &[event, shown])?;
            }
            add_to_list(recv, &name, f, prepend);
            warn_max_listeners(recv, &name, args.first().unwrap_or(&Value::Undef));
            Ok(recv.clone())
        }
        "emit" => emit(
            recv,
            &event_key(&args),
            &args.get(1..).map(|s| s.to_vec()).unwrap_or_default(),
        ),
        "removeListener" | "off" => {
            let event = args.first().cloned().unwrap_or(Value::Undef);
            let f = args.get(1).cloned().unwrap_or(Value::Undef);
            check_listener(&f)?;
            remove_listener(recv, &event_key(&args), event, &f)?;
            Ok(recv.clone())
        }
        "removeAllListeners" => {
            if args.is_empty() {
                remove_all_listeners(recv, None)?;
            } else {
                let event = args.first().cloned().unwrap_or(Value::Undef);
                remove_all_listeners(recv, Some((event_key(&args), event)))?;
            }
            Ok(recv.clone())
        }
        // `listenerCount(type, listener)` counts the registrations of one
        // function, a once-wrapper counting for the function it wraps.
        "listenerCount" => {
            let list = listeners(recv, &event_key(&args));
            let wanted = args.get(1).cloned().unwrap_or(Value::Undef);
            if with_host(|h| h.is_nullish(&wanted)) {
                return Ok(Value::Float(list.len() as f64));
            }
            let mut matching = 0;
            for l in &list {
                if *l == wanted || crate::builtins::get_property(l, "listener")? == wanted {
                    matching += 1;
                }
            }
            Ok(Value::Float(matching as f64))
        }
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

/// `_addListener`'s leak check: once an event has more listeners than the
/// emitter's cap (`setMaxListeners`, default 10; `0` disables it), emit ONE
/// `MaxListenersExceededWarning` for that event. The cap used to be stored and
/// never consulted.
fn warn_max_listeners(recv: &Value, key: &str, event: &Value) {
    let max = with_host(|h| match named_map(h, recv, "@@maxListeners") {
        Some(v @ (Value::Float(_) | Value::Int(_))) => h.to_number(&v),
        _ => 10.0,
    });
    let count = listeners(recv, key).len();
    if max <= 0.0 || count as f64 <= max {
        return;
    }
    let marker = format!("@@warned:{key}");
    if with_host(|h| named_map(h, recv, &marker).is_some()) {
        return;
    }
    with_host(|h| match h.get_mut(recv) {
        Some(JsObj::Object(p)) => {
            p.insert(marker, Value::Bool(true));
        }
        _ => h.set_fn_prop(recv, &marker, Value::Bool(true)),
    });
    let (event, target) = with_host(|h| {
        let event = h.str_of(event);
        let target = if crate::host::is_callable(h, recv) {
            format!("[Function: {}]", h.callable_name(recv))
        } else {
            match h.ctor_name(recv) {
                n if n.is_empty() => "[EventEmitter]".to_string(),
                n => format!("[{n}]"),
            }
        };
        (event, target)
    });
    super::process::emit_warning(
        "MaxListenersExceededWarning",
        None,
        &format!(
            "Possible EventEmitter memory leak detected. {count} {event} listeners added to {target}. \
             MaxListeners is {}. Use emitter.setMaxListeners() to increase limit",
            crate::host::fmt_number(max)
        ),
        None,
    );
}

/// Read a hidden emitter field (`@@on`, `@@maxListeners`). Works for a plain emitter
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

/// Register `f` for `name`. Every listener — a once-wrapper included — goes
/// into the one ordered `@@on` list, so registration order is firing order.
fn add_to_list(recv: &Value, name: &str, f: Value, prepend: bool) {
    with_host(|h| {
        // Lazily create the listener map (a mixed-in function emitter has none).
        let map = match named_map(h, recv, "@@on") {
            Some(m) => m,
            None => {
                let m = h.new_object(IndexMap::new());
                set_named_map(h, recv, "@@on", m.clone());
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
            // `prependListener` puts the handler FIRST.
            if prepend {
                items.insert(0, f);
            } else {
                items.push(f);
            }
        }
    });
}

/// Drop the entry at `index` from `name`'s listener list.
fn remove_at(recv: &Value, name: &str, index: usize) {
    with_host(|h| {
        let Some(map) = named_map(h, recv, "@@on") else {
            return;
        };
        let arr = match h.get(&map) {
            Some(JsObj::Object(p)) => p.get(name).cloned(),
            _ => None,
        };
        let mut emptied = false;
        if let Some(JsObj::Array(items)) = arr.and_then(|a| h.get_mut(&a)) {
            if index < items.len() {
                items.remove(index);
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

/// The RAW listener list for `name` (`rawListeners`): once-wrappers as stored.
fn listeners(recv: &Value, name: &str) -> Vec<Value> {
    with_host(|h| {
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

/// `unwrapListeners`: each entry's `.listener`, or the entry itself.
fn unwrapped_listeners(recv: &Value, name: &str) -> Result<Vec<Value>, String> {
    listeners(recv, name)
        .iter()
        .map(original_listener)
        .collect()
}

/// `listener.listener || listener` — the function a once-wrapper stands for.
fn original_listener(f: &Value) -> Result<Value, String> {
    let inner = crate::builtins::get_property(f, "listener")?;
    Ok(if with_host(|h| h.truthy(&inner)) {
        inner
    } else {
        f.clone()
    })
}

/// `_onceWrap(target, type, listener)` from `lib/events.js`, verbatim: a
/// closure that removes itself before its first call and ignores any later
/// one, carrying the wrapped function as `.listener`.
const ONCE_WRAP_SRC: &str = "(function _onceWrap(target, type, listener) {\n\
  let fired = false;\n\
  function wrapper(...args) {\n\
    if (fired) return;\n\
    fired = true;\n\
    target.removeListener(type, wrapper);\n\
    return Reflect.apply(listener, target, args);\n\
  }\n\
  wrapper.listener = listener;\n\
  return wrapper;\n\
})";

fn once_wrap(target: &Value, event: &Value, f: &Value) -> Result<Value, String> {
    thread_local! {
        static FACTORY: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    }
    let factory = match FACTORY.with(|c| c.borrow().clone()) {
        Some(f) => f,
        None => {
            let made = crate::eval_in_global_scope(ONCE_WRAP_SRC)?;
            FACTORY.with(|c| *c.borrow_mut() = Some(made.clone()));
            made
        }
    };
    invoke(
        &factory,
        vec![target.clone(), event.clone(), f.clone()],
        None,
    )
}

/// `removeListener(type, listener)`, ported from `lib/events.js`. A lone
/// listener is matched by itself or its `.listener`, and the `removeListener`
/// event is handed `list.listener || listener`; in a longer list the LAST
/// match goes, and the event is handed `listener` exactly as passed — which
/// is the wrapper when a once-wrapper removes itself.
fn remove_listener(recv: &Value, name: &str, event: Value, f: &Value) -> Result<(), String> {
    let list = listeners(recv, name);
    let removed = if list.len() == 1 {
        let inner = if list[0] == *f {
            None
        } else {
            Some(crate::builtins::get_property(&list[0], "listener")?)
        };
        match inner {
            None => Some(original_listener(&list[0])?),
            Some(inner) if inner == *f => Some(original_listener(&list[0])?),
            Some(_) => None,
        }
        .inspect(|_| remove_at(recv, name, 0))
    } else {
        let mut position = None;
        for (i, l) in list.iter().enumerate().rev() {
            if *l == *f || crate::builtins::get_property(l, "listener")? == *f {
                position = Some(i);
                break;
            }
        }
        position.map(|i| {
            remove_at(recv, name, i);
            f.clone()
        })
    };
    // `removeListener` fires AFTER the removal, and only when one happened.
    if let Some(shown) = removed {
        if !listeners(recv, "removeListener").is_empty() {
            emit(recv, "removeListener", &[event, shown])?;
        }
    }
    Ok(())
}

/// `removeAllListeners([type])`, ported from `lib/events.js`. With nobody
/// listening for `removeListener` the lists are simply dropped; otherwise
/// every listener goes through `removeListener` (LIFO within an event), every
/// event but `removeListener` first, so each removal is announced.
fn remove_all_listeners(recv: &Value, event: Option<(String, Value)>) -> Result<(), String> {
    if listeners(recv, "removeListener").is_empty() {
        with_host(|h| {
            if let Some(map) = named_map(h, recv, "@@on") {
                if let Some(JsObj::Object(p)) = h.get_mut(&map) {
                    match &event {
                        Some((name, _)) => {
                            p.shift_remove(name);
                        }
                        None => p.clear(),
                    }
                }
            }
        });
        return Ok(());
    }
    let Some((name, event)) = event else {
        let keys: Vec<(String, Value)> = with_host(|h| {
            let names: Vec<String> = match named_map(h, recv, "@@on").and_then(|m| h.get(&m)) {
                Some(JsObj::Object(p)) => p.keys().cloned().collect(),
                _ => Vec::new(),
            };
            // `ReflectOwnKeys` order: string keys, then symbols.
            let (syms, strs): (Vec<String>, Vec<String>) = names
                .into_iter()
                .partition(|k| crate::host::is_symbol_key(k));
            strs.into_iter()
                .chain(syms)
                .filter(|k| k != "removeListener")
                .map(|k| {
                    let v = h.symbol_of_key(&k).unwrap_or_else(|| h.new_str(k.clone()));
                    (k, v)
                })
                .collect()
        });
        for key in keys {
            remove_all_listeners(recv, Some(key))?;
        }
        let rl = with_host(|h| h.new_str("removeListener"));
        remove_all_listeners(recv, Some(("removeListener".into(), rl)))?;
        return remove_all_listeners(recv, None);
    };
    for f in listeners(recv, &name).iter().rev() {
        remove_listener(recv, &name, event.clone(), f)?;
    }
    Ok(())
}

/// `checkListener`: every registration and removal refuses a non-function.
fn check_listener(f: &Value) -> Result<(), String> {
    if with_host(|h| crate::host::is_callable(h, f)) {
        return Ok(());
    }
    Err(crate::host::invalid_arg_type(
        "listener", "argument", "function", f,
    ))
}

fn emit(recv: &Value, name: &str, args: &[Value]) -> Result<Value, String> {
    // `EventEmitter.errorMonitor` listeners see every `error` FIRST, without
    // counting as a handler: an error with only a monitor still throws.
    if name == "error" {
        let monitor = with_host(|h| {
            let sym = h.internal_symbol("events.errorMonitor");
            h.property_key(&sym)
        });
        if !listeners(recv, &monitor).is_empty() {
            emit(recv, &monitor, args)?;
        }
    }
    let to_call = listeners(recv, name);
    // An `error` event with no listener THROWS rather than being dropped. This
    // is how node surfaces a failed socket, stream or request, and swallowing
    // it turned every such failure into silence.
    if name == "error" && to_call.is_empty() {
        let err = args.first().cloned().unwrap_or(Value::Undef);
        // Only an `Error` is thrown as itself; anything else is wrapped in
        // ERR_UNHANDLED_ERROR quoting its inspection, with the value kept as
        // `context` (lib/events.js).
        let error_ctor = crate::builtins::global_binding("Error").unwrap_or(Value::Undef);
        if !crate::host::instance_of(&err, &error_ctor).unwrap_or(false) {
            let shown = crate::host::inspect_js(&err)?;
            let msg = crate::host::coded_error(
                "Error",
                "ERR_UNHANDLED_ERROR",
                &format!("Unhandled error. ({shown})"),
            );
            let wrapped = with_host(|h| crate::builtins::synth_error(h, &msg));
            crate::builtins::set_property_pub(&wrapped, "context", err)?;
            with_host(|h| h.exc = Some(wrapped));
            return Err(msg);
        }
        let msg = with_host(|h| {
            h.exc = Some(err.clone());
            crate::builtins::error_string(h, &err)
        });
        return Err(msg);
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
            let found = match unwrapped_listeners(&emitter, &arg_str(args, 1)) {
                Ok(found) => found,
                Err(e) => return Some(Err(e)),
            };
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
            let native = h.new_str("EventEmitter");
            if let Some(JsObj::Object(p)) = h.get_mut(&emitter) {
                p.entry("@@native".to_string()).or_insert(native);
                p.insert("@@on".to_string(), on);
            }
        }
    });
    emitter
}

/// `events.addAbortListener(signal, listener)`: a one-time `abort` listener on
/// the signal, or — if it is already aborted — `listener` on a microtask. The
/// returned disposable is a plain object: this runtime has no `Symbol.dispose`.
fn add_abort_listener(args: &[Value]) -> Value {
    let signal = args.first().cloned().unwrap_or(Value::Undef);
    let listener = args.get(1).cloned().unwrap_or(Value::Undef);
    let aborted = crate::builtins::get_property(&signal, "aborted").unwrap_or(Value::Undef);
    if with_host(|h| h.truthy(&aborted)) {
        with_host(|h| h.queue_micro(listener, Vec::new()));
    } else {
        let name = with_host(|h| h.new_str("abort"));
        let once = with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("once".into(), Value::Bool(true));
            h.new_object(m)
        });
        let _ = call_method(&signal, "addEventListener", vec![name, listener, once]);
    }
    with_host(|h| h.new_object(IndexMap::new()))
}
