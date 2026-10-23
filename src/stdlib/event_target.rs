//! The WHATWG `Event` / `EventTarget` / `CustomEvent` globals — a port of
//! node's `lib/internal/event_target.js`, restricted to what a non-DOM target
//! can observe (there is no tree, so there is no capture or bubble PATH: every
//! dispatch runs at the target, `eventPhase` is `AT_TARGET` while listeners
//! run and `composedPath()` is `[target]`).
//!
//! `AbortSignal` is an `EventTarget` in node, and its `abort` listeners get a
//! real `Event`; `fetch::abort_signal` dispatches through [`dispatch`] so the
//! two share one listener list and one ordering rule.
//!
//! Storage: a target keeps `@@listeners`, a hidden object mapping an event type
//! to an ARRAY of listener records. A record is a plain hidden object carrying
//! `@@cb` (the callback or `handleEvent` object), `@@capture`, `@@once`, and
//! `@@handler` — the latter marks the slot an `on<type>` event-handler
//! attribute occupies: node's `defineEventHandler` registers that wrapper the
//! FIRST time the attribute is assigned and only swaps its callback afterwards,
//! which is why `signal.onabort = f` set before an `addEventListener` runs
//! first no matter how often it is reassigned. The wrapper reads the current
//! callback from the target's `@@on<type>` slot at dispatch time.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// The classes this module constructs, as bare globals.
pub const CLASSES: &[&str] = &["Event", "EventTarget", "CustomEvent"];

pub const EVENT_TARGET_METHODS: &[&str] =
    &["addEventListener", "removeEventListener", "dispatchEvent"];
/// `Event.prototype` members in node's definition order are methods and
/// accessors interleaved; the prototype builder lays out methods, then
/// accessors, then [`EVENT_LATE_METHODS`], so the two method groups bracket
/// the accessor block the way node's `initEvent`…`preventDefault` and
/// `composedPath`/`stopPropagation` do.
pub const EVENT_METHODS: &[&str] = &["initEvent", "stopImmediatePropagation", "preventDefault"];
pub const EVENT_LATE_METHODS: &[&str] = &["composedPath", "stopPropagation"];

/// `(name, settable)` for every `Event.prototype` accessor. `srcElement`,
/// `returnValue` and `cancelBubble` are COMPUTED from other slots (see
/// [`instance_call`]'s `@get@` arms); the rest read their own `@@<name>` slot.
pub const EVENT_ACCESSORS: &[(&str, bool)] = &[
    ("target", false),
    ("currentTarget", false),
    ("srcElement", false),
    ("type", false),
    ("cancelable", false),
    ("defaultPrevented", false),
    ("timeStamp", false),
    ("returnValue", true),
    ("bubbles", false),
    ("composed", false),
    ("eventPhase", false),
    ("cancelBubble", true),
    ("isTrusted", false),
];

/// The `eventPhase` constants, on the constructor (`Event.AT_TARGET`).
pub fn constant(name: &str) -> Option<Value> {
    Some(Value::Int(match name {
        "NONE" => 0,
        "CAPTURING_PHASE" => 1,
        "AT_TARGET" => 2,
        "BUBBLING_PHASE" => 3,
        _ => return None,
    }))
}

pub fn is_class(name: &str) -> bool {
    CLASSES.contains(&name)
}

// ── small helpers ────────────────────────────────────────────────────────────

fn slot(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

fn set_slot(recv: &Value, key: &str, val: Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), val);
        }
    });
}

fn flag(recv: &Value, key: &str) -> bool {
    matches!(slot(recv, key), Some(Value::Bool(true)))
}

fn is_object(v: &Value) -> bool {
    with_host(|h| matches!(h.get(v), Some(JsObj::Object(_)) | Some(JsObj::Array(_))))
}

/// Whether `v` carries the `Event` brand — its `@@type` slot — which a subclass
/// instance has too, since `super()` adopts the built event's slots.
fn is_event(v: &Value) -> bool {
    matches!(v, Value::Obj(_)) && slot(v, "@@type").is_some()
}

/// Whether `v` is an `EventTarget`: it owns a `@@listeners` map. Every
/// constructor here and in `fetch::new_abort_signal` installs one.
fn is_event_target(v: &Value) -> bool {
    matches!(v, Value::Obj(_)) && slot(v, "@@listeners").is_some()
}

fn missing_args(what: &str) -> String {
    crate::host::coded_error(
        "TypeError",
        "ERR_MISSING_ARGS",
        &format!("The {what} must be specified"),
    )
}

fn invalid_this(class: &str) -> String {
    crate::host::coded_error(
        "TypeError",
        "ERR_INVALID_THIS",
        &format!("Value of \"this\" must be of type {class}"),
    )
}

/// Throw `v` as the JS exception value rather than as an internal message, so
/// `catch (e)` receives the object itself.
pub(crate) fn throw_value(v: Value) -> String {
    let msg = with_host(|h| h.str_of(&v));
    with_host(|h| h.exc = Some(v));
    msg
}

/// Node's `emitUncaughtException(err)`: `process.nextTick(() => { throw err })`.
/// A listener's exception never reaches `dispatchEvent`'s caller; it surfaces
/// on the loop as an `uncaughtException` after the remaining listeners ran.
fn rethrow_on_next_tick(msg: &str) {
    let err = with_host(|h| h.exc.take())
        .unwrap_or_else(|| with_host(|h| crate::builtins::synth_error(h, msg)));
    let Value::Obj(idx) = err else {
        return;
    };
    with_host(|h| {
        let thunk = h.alloc(JsObj::Builtin(format!("@@rethrow:{idx}")));
        h.queue_nexttick(thunk, Vec::new());
    });
}

/// The `@@rethrow:<idx>` thunk body: raise the stored error.
pub fn rethrow(idx: u32) -> Result<Value, String> {
    Err(throw_value(Value::Obj(idx)))
}

// ── construction ─────────────────────────────────────────────────────────────

/// A fresh, empty listener map for a target under construction.
pub fn new_listener_map() -> Value {
    with_host(|h| h.new_object(IndexMap::new()))
}

fn construct_event_target() -> Value {
    let listeners = new_listener_map();
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("EventTarget"));
        m.insert("@@listeners".into(), listeners);
        h.new_object(m)
    })
}

/// `new Event(type[, { bubbles, cancelable, composed }])`.
fn event_slots(tag: &str, args: &[Value]) -> Result<IndexMap<String, Value>, String> {
    if args.is_empty() {
        return Err(missing_args("\"type\" argument"));
    }
    let ty = with_host(|h| h.str_of(&args[0]));
    let init = args.get(1).cloned().unwrap_or(Value::Undef);
    let option = |name: &str| -> bool {
        if !is_object(&init) {
            return false;
        }
        let v = crate::builtins::get_property(&init, name).unwrap_or(Value::Undef);
        with_host(|h| h.truthy(&v))
    };
    let bubbles = option("bubbles");
    let cancelable = option("cancelable");
    let composed = option("composed");
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        m.insert("@@type".into(), h.new_str(ty));
        m.insert("@@bubbles".into(), Value::Bool(bubbles));
        m.insert("@@cancelable".into(), Value::Bool(cancelable));
        m.insert("@@composed".into(), Value::Bool(composed));
        m.insert("@@defaultPrevented".into(), Value::Bool(false));
        m.insert(
            "@@timeStamp".into(),
            Value::Float(super::perf_hooks::now_ms()),
        );
        m.insert("@@target".into(), h.null());
        m.insert("@@currentTarget".into(), h.null());
        m.insert("@@eventPhase".into(), Value::Int(0));
        m.insert("@@isTrusted".into(), Value::Bool(false));
        m.insert("@@propagationStopped".into(), Value::Bool(false));
        m.insert("@@stopImmediate".into(), Value::Bool(false));
        m.insert("@@dispatching".into(), Value::Bool(false));
        m
    }))
}

pub fn construct(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match name {
        "EventTarget" => Ok(construct_event_target()),
        "Event" => event_slots("Event", args).map(|m| with_host(|h| h.new_object(m))),
        // `detail` defaults to `null`, not `undefined`.
        "CustomEvent" => event_slots("CustomEvent", args).map(|mut m| {
            let init = args.get(1).cloned().unwrap_or(Value::Undef);
            let detail = if is_object(&init) {
                crate::builtins::get_property(&init, "detail").unwrap_or(Value::Undef)
            } else {
                Value::Undef
            };
            with_host(|h| {
                let detail = if matches!(detail, Value::Undef) {
                    h.null()
                } else {
                    detail
                };
                m.insert("@@detail".into(), detail);
                h.new_object(m)
            })
        }),
        _ => return None,
    })
}

// ── listener records ─────────────────────────────────────────────────────────

/// The listener array for `ty` on `target`, created on first use.
fn listener_list(target: &Value, ty: &str, create: bool) -> Option<Value> {
    let map = slot(target, "@@listeners")?;
    let existing = with_host(|h| match h.get(&map) {
        Some(JsObj::Object(p)) => p.get(ty).cloned(),
        _ => None,
    });
    match existing {
        Some(list) => Some(list),
        None if create => Some(with_host(|h| {
            let list = h.new_array(Vec::new());
            if let Some(JsObj::Object(p)) = h.get_mut(&map) {
                p.insert(ty.to_string(), list.clone());
            }
            list
        })),
        None => None,
    }
}

fn list_items(list: &Value) -> Vec<Value> {
    with_host(|h| match h.get(list) {
        Some(JsObj::Array(items)) => items.clone(),
        _ => Vec::new(),
    })
}

fn new_record(cb: Value, capture: bool, once: bool, handler: bool) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@cb".into(), cb);
        m.insert("@@capture".into(), Value::Bool(capture));
        m.insert("@@once".into(), Value::Bool(once));
        m.insert("@@handler".into(), Value::Bool(handler));
        m.insert("@@removed".into(), Value::Bool(false));
        h.new_object(m)
    })
}

/// The record in `list` registered for `cb` at this `capture` flag, if any —
/// the identity `addEventListener` dedupes on and `removeEventListener` finds.
fn find_record(list: &Value, cb: &Value, capture: bool) -> Option<Value> {
    list_items(list).into_iter().find(|rec| {
        !flag(rec, "@@handler")
            && flag(rec, "@@capture") == capture
            && slot(rec, "@@cb")
                .map(|c| with_host(|h| h.strict_eq(&c, cb)))
                .unwrap_or(false)
    })
}

/// Drop `rec` from `list` and mark it removed, so an in-flight dispatch that
/// snapshotted the list skips it.
fn remove_record(list: &Value, rec: &Value) {
    set_slot(rec, "@@removed", Value::Bool(true));
    with_host(|h| {
        if let Some(JsObj::Array(items)) = h.get_mut(list) {
            items.retain(|r| r != rec);
        }
    });
}

/// `(capture, once, signal)` from an `addEventListener` options argument: a
/// boolean IS `capture`; an object carries the three named fields.
fn parse_options(opts: &Value) -> (bool, bool, Option<Value>) {
    if let Value::Bool(b) = opts {
        return (*b, false, None);
    }
    if !is_object(opts) {
        return (false, false, None);
    }
    // Each read may run a getter (and so re-enter the host), so it happens
    // before, not inside, the borrow that tests the value.
    let read = |name: &str| crate::builtins::get_property(opts, name).unwrap_or(Value::Undef);
    let capture = read("capture");
    let capture = with_host(|h| h.truthy(&capture));
    let once = read("once");
    let once = with_host(|h| h.truthy(&once));
    let signal = Some(read("signal")).filter(|s| !matches!(s, Value::Undef));
    (capture, once, signal)
}

/// `target.addEventListener(type, listener[, options])`.
pub fn add_listener(target: &Value, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 {
        return Err(missing_args("\"type\" and \"listener\" arguments"));
    }
    let ty = with_host(|h| h.str_of(&args[0]));
    let listener = args[1].clone();
    // A nullish listener is accepted and ignored, with node's one-time-per-call
    // warning rather than an error.
    if with_host(|h| h.is_nullish(&listener)) {
        let shown = if with_host(|h| h.is_null(&listener)) {
            "null"
        } else {
            "undefined"
        };
        super::process::emit_warning(
            "AddEventListenerArgumentTypeWarning",
            None,
            &format!("addEventListener called with {shown} which has no effect."),
            None,
        );
        return Ok(Value::Undef);
    }
    let callable = with_host(|h| crate::host::is_callable(h, &listener));
    if !callable && !is_object(&listener) {
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"listener\" argument must be an instance of EventListener. Received {}",
                super::received_desc(&listener)
            ),
        ));
    }
    let (capture, once, signal) = parse_options(&args.get(2).cloned().unwrap_or(Value::Undef));
    // An already-aborted signal means the listener is never registered.
    if let Some(sig) = &signal {
        if matches!(slot(sig, "@@aborted"), Some(Value::Bool(true))) {
            return Ok(Value::Undef);
        }
    }
    let list = listener_list(target, &ty, true).ok_or_else(|| invalid_this("EventTarget"))?;
    if find_record(&list, &listener, capture).is_some() {
        return Ok(Value::Undef);
    }
    let rec = new_record(listener.clone(), capture, once, false);
    with_host(|h| {
        if let Some(JsObj::Array(items)) = h.get_mut(&list) {
            items.push(rec);
        }
    });
    // `{ signal }`: the signal's abort removes this listener. The removal is a
    // once-listener on the signal whose callback is a native thunk keyed by a
    // hidden record of (target, type, listener, capture).
    if let Some(sig) = signal {
        if is_event_target(&sig) {
            let record = with_host(|h| {
                let mut m = IndexMap::new();
                m.insert("@@target".into(), target.clone());
                m.insert("@@type".into(), h.new_str(ty.clone()));
                m.insert("@@cb".into(), listener);
                m.insert("@@capture".into(), Value::Bool(capture));
                h.new_object(m)
            });
            if let Value::Obj(idx) = record {
                let thunk = with_host(|h| h.alloc(JsObj::Builtin(format!("@@evunlisten:{idx}"))));
                let abort = with_host(|h| h.new_str("abort"));
                let once = with_host(|h| {
                    let mut m = IndexMap::new();
                    m.insert("once".into(), Value::Bool(true));
                    h.new_object(m)
                });
                add_listener(&sig, &[abort, thunk, once])?;
            }
        }
    }
    Ok(Value::Undef)
}

/// The `@@evunlisten:<idx>` thunk body: remove the listener the record at
/// heap index `idx` describes.
pub fn unlisten(idx: u32) -> Result<Value, String> {
    let record = Value::Obj(idx);
    let Some(target) = slot(&record, "@@target") else {
        return Ok(Value::Undef);
    };
    let ty = slot(&record, "@@type").unwrap_or(Value::Undef);
    let cb = slot(&record, "@@cb").unwrap_or(Value::Undef);
    let capture = slot(&record, "@@capture").unwrap_or(Value::Bool(false));
    remove_listener(&target, &[ty, cb, capture])
}

/// `target.removeEventListener(type, listener[, options])`.
pub fn remove_listener(target: &Value, args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 {
        return Err(missing_args("\"type\" and \"listener\" arguments"));
    }
    let ty = with_host(|h| h.str_of(&args[0]));
    let (capture, _, _) = parse_options(&args.get(2).cloned().unwrap_or(Value::Undef));
    if let Some(list) = listener_list(target, &ty, false) {
        if let Some(rec) = find_record(&list, &args[1], capture) {
            remove_record(&list, &rec);
        }
    }
    Ok(Value::Undef)
}

/// Install the event-handler attribute slot for `on<type>` on `target` if this
/// is the first assignment (`defineEventHandler`'s wrapper). The callback is
/// read from `@@on<type>` at dispatch time, so a reassignment keeps the slot's
/// position in the listener order.
pub fn ensure_handler_slot(target: &Value, ty: &str) {
    let Some(list) = listener_list(target, ty, true) else {
        return;
    };
    if list_items(&list).iter().any(|rec| flag(rec, "@@handler")) {
        return;
    }
    let rec = new_record(Value::Undef, false, false, true);
    with_host(|h| {
        if let Some(JsObj::Array(items)) = h.get_mut(&list) {
            items.push(rec);
        }
    });
}

/// `target.dispatchEvent(event)` — run the listeners for `event.type` in
/// registration order, then report whether `preventDefault()` was NOT called.
pub fn dispatch(target: &Value, args: &[Value]) -> Result<Value, String> {
    if !is_event_target(target) {
        return Err(invalid_this("EventTarget"));
    }
    let Some(event) = args.first() else {
        return Err(missing_args("\"event\" argument"));
    };
    if !is_event(event) {
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"event\" argument must be an instance of Event. Received {}",
                super::received_desc(event)
            ),
        ));
    }
    let ty = slot(event, "@@type")
        .map(|v| with_host(|h| h.str_of(&v)))
        .unwrap_or_default();
    if flag(event, "@@dispatching") {
        return Err(crate::host::coded_error(
            "Error",
            "ERR_EVENT_RECURSION",
            &format!("The event \"{ty}\" is already being dispatched"),
        ));
    }
    set_slot(event, "@@target", target.clone());
    set_slot(event, "@@currentTarget", target.clone());
    set_slot(event, "@@eventPhase", Value::Int(2));
    set_slot(event, "@@dispatching", Value::Bool(true));

    // A snapshot: a listener added during dispatch does not run this time, one
    // removed during dispatch is skipped by its `@@removed` mark.
    let records = listener_list(target, &ty, false)
        .map(|list| (list_items(&list), list))
        .unwrap_or_else(|| (Vec::new(), Value::Undef));
    for rec in &records.0 {
        if flag(event, "@@stopImmediate") {
            break;
        }
        if flag(rec, "@@removed") {
            continue;
        }
        if flag(rec, "@@once") {
            remove_record(&records.1, rec);
        }
        let cb = if flag(rec, "@@handler") {
            slot(target, &format!("@@on{ty}")).unwrap_or(Value::Undef)
        } else {
            slot(rec, "@@cb").unwrap_or(Value::Undef)
        };
        let result = if with_host(|h| crate::host::is_callable(h, &cb)) {
            crate::host::invoke(&cb, vec![event.clone()], Some(target.clone()))
        } else if flag(rec, "@@handler") {
            // An event-handler attribute holding a non-function does nothing.
            Ok(Value::Undef)
        } else {
            // An `EventListener` object: node's wrapper is `if (listener
            // .handleEvent) handleEvent.apply(listener, args)`, so a missing
            // `handleEvent` is silently nothing and a non-callable one is the
            // `apply` TypeError, raised on the next tick like any listener error.
            match crate::builtins::get_property(&cb, "handleEvent") {
                Ok(h) if !with_host(|host| host.truthy(&h)) => Ok(Value::Undef),
                Ok(_) => crate::host::call_method(&cb, "handleEvent", vec![event.clone()]),
                Err(e) => Err(e),
            }
        };
        if let Err(msg) = result {
            rethrow_on_next_tick(&msg);
        }
    }

    set_slot(event, "@@eventPhase", Value::Int(0));
    set_slot(event, "@@currentTarget", with_host(|h| h.null()));
    set_slot(event, "@@dispatching", Value::Bool(false));
    Ok(Value::Bool(!flag(event, "@@defaultPrevented")))
}

// ── dispatch ─────────────────────────────────────────────────────────────────

pub fn instance_call(
    tag: &str,
    recv: &Value,
    method: &str,
    args: &[Value],
) -> Result<Value, String> {
    match tag {
        "EventTarget" => event_target_call(recv, method, args),
        _ => event_call(recv, method, args),
    }
}

/// The three `EventTarget.prototype` methods. `AbortSignal` routes here too.
pub fn event_target_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    if !is_event_target(recv) {
        return Err(invalid_this("EventTarget"));
    }
    match method {
        "addEventListener" => add_listener(recv, args),
        "removeEventListener" => remove_listener(recv, args),
        "dispatchEvent" => dispatch(recv, args),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

fn event_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    if !is_event(recv) {
        return Err(invalid_this("Event"));
    }
    match method {
        // `initEvent` is a no-op on an event being dispatched.
        "initEvent" => {
            if args.is_empty() {
                return Err(missing_args("\"type\" argument"));
            }
            if !flag(recv, "@@dispatching") {
                let ty = with_host(|h| h.str_of(&args[0]));
                let bubbles = args
                    .get(1)
                    .map(|v| with_host(|h| h.truthy(v)))
                    .unwrap_or(false);
                let cancelable = args
                    .get(2)
                    .map(|v| with_host(|h| h.truthy(v)))
                    .unwrap_or(false);
                set_slot(recv, "@@type", with_host(|h| h.new_str(ty)));
                set_slot(recv, "@@bubbles", Value::Bool(bubbles));
                set_slot(recv, "@@cancelable", Value::Bool(cancelable));
            }
            Ok(Value::Undef)
        }
        "stopImmediatePropagation" => {
            set_slot(recv, "@@propagationStopped", Value::Bool(true));
            set_slot(recv, "@@stopImmediate", Value::Bool(true));
            Ok(Value::Undef)
        }
        "stopPropagation" => {
            set_slot(recv, "@@propagationStopped", Value::Bool(true));
            Ok(Value::Undef)
        }
        // Only a cancelable event can be default-prevented.
        "preventDefault" => {
            if flag(recv, "@@cancelable") {
                set_slot(recv, "@@defaultPrevented", Value::Bool(true));
            }
            Ok(Value::Undef)
        }
        // There is no tree: the path is the target alone, and only while the
        // event is being dispatched.
        "composedPath" => {
            let path = if flag(recv, "@@dispatching") {
                vec![slot(recv, "@@target").unwrap_or(Value::Undef)]
            } else {
                Vec::new()
            };
            Ok(with_host(|h| h.new_array(path)))
        }
        // The computed accessors (no `@@<name>` slot of their own).
        "@get@srcElement" => Ok(slot(recv, "@@target").unwrap_or(Value::Undef)),
        "@get@returnValue" => Ok(Value::Bool(!flag(recv, "@@defaultPrevented"))),
        "@get@cancelBubble" => Ok(Value::Bool(flag(recv, "@@propagationStopped"))),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// An `Event` accessor SETTER: `cancelBubble = truthy` is `stopPropagation()`,
/// `returnValue = falsy` is `preventDefault()`. Neither stores the value.
pub fn accessor_written(recv: &Value, key: &str, value: &Value) -> Result<(), String> {
    if !is_event(recv) {
        return Err(invalid_this("Event"));
    }
    let truthy = with_host(|h| h.truthy(value));
    match key {
        "cancelBubble" if truthy => set_slot(recv, "@@propagationStopped", Value::Bool(true)),
        "returnValue" if !truthy && flag(recv, "@@cancelable") => {
            set_slot(recv, "@@defaultPrevented", Value::Bool(true))
        }
        _ => {}
    }
    Ok(())
}
