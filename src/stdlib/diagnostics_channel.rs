//! Node `diagnostics_channel` — in-process publish/subscribe named channels.
//!
//! A channel is a plain object tagged `@@native = "Channel"` carrying its name
//! (`@@name`, also exposed as the enumerable `name`), a hidden `@@subs` array of
//! subscriber callbacks, and a `hasSubscribers` data property kept in sync as
//! subscribers come and go. Channels are interned by name in a thread-local
//! registry so `channel('x') === channel('x')` and the module-level
//! `subscribe/unsubscribe/hasSubscribers(name, …)` operate on the very same
//! object a caller obtains from `channel(name)` — matching Node's guarantee that
//! a channel is a single shared instance per name.
//!
//! Scope of fidelity: this is an **in-process** implementation. `publish(msg)`
//! synchronously invokes each subscriber with `(message, channelName)` (Node's
//! contract). It does not span worker threads or processes, and it does not
//! implement the `tracingChannel` / async-context store surface. The registry is
//! process-lifetime; handles are only meaningful within a single program run
//! (they index the live heap).

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// name → the single interned Channel object for that name.
    static CHANNELS: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
}

pub const METHODS: &[&str] = &["channel", "subscribe", "unsubscribe", "hasSubscribers"];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let name = super::arg_str(args, 0);
    Some(match method {
        "channel" => Ok(get_or_create(&name)),
        // Module-level subscribe/unsubscribe address a channel by name, creating
        // it on demand so a later `channel(name)` sees the same subscribers.
        "subscribe" => {
            let ch = get_or_create(&name);
            add_sub(&ch, args.get(1).cloned().unwrap_or(Value::Undef));
            Ok(Value::Undef)
        }
        "unsubscribe" => {
            let ch = get_or_create(&name);
            Ok(Value::Bool(remove_sub(&ch, &args.get(1).cloned().unwrap_or(Value::Undef))))
        }
        "hasSubscribers" => Ok(Value::Bool(sub_count(&get_or_create(&name)) > 0)),
        _ => return None,
    })
}

/// Instance dispatch for a Channel object (`@@native = "Channel"`).
pub fn instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "subscribe" => {
            add_sub(recv, args.first().cloned().unwrap_or(Value::Undef));
            Ok(Value::Undef)
        }
        "unsubscribe" => Ok(Value::Bool(remove_sub(recv, &args.first().cloned().unwrap_or(Value::Undef)))),
        "publish" => publish(recv, args.first().cloned().unwrap_or(Value::Undef)),
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

/// The interned channel for `name`, creating it (empty) on first use.
fn get_or_create(name: &str) -> Value {
    if let Some(ch) = CHANNELS.with(|c| c.borrow().get(name).cloned()) {
        return ch;
    }
    let ch = with_host(|h| {
        let subs = h.new_array(Vec::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Channel"));
        m.insert("@@name".into(), h.new_str(name));
        m.insert("@@subs".into(), subs);
        m.insert("name".into(), h.new_str(name));
        m.insert("hasSubscribers".into(), Value::Bool(false));
        h.new_object(m)
    });
    CHANNELS.with(|c| c.borrow_mut().insert(name.to_string(), ch.clone()));
    ch
}

/// The channel's `@@subs` array handle, if any.
fn subs_array(ch: &Value) -> Option<Value> {
    with_host(|h| match h.get(ch) {
        Some(JsObj::Object(p)) => p.get("@@subs").cloned(),
        _ => None,
    })
}

/// Current subscriber count.
fn sub_count(ch: &Value) -> usize {
    match subs_array(ch) {
        Some(arr) => with_host(|h| match h.get(&arr) {
            Some(JsObj::Array(items)) => items.len(),
            _ => 0,
        }),
        None => 0,
    }
}

/// Append a subscriber callback and refresh `hasSubscribers`.
fn add_sub(ch: &Value, cb: Value) {
    if let Some(arr) = subs_array(ch) {
        with_host(|h| {
            if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                items.push(cb);
            }
        });
        refresh_has(ch);
    }
}

/// Remove the first subscriber with the same heap identity as `cb`; returns
/// whether one was removed.
fn remove_sub(ch: &Value, cb: &Value) -> bool {
    let removed = match subs_array(ch) {
        Some(arr) => with_host(|h| {
            if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                if let Some(i) = items.iter().position(|x| same_ref(x, cb)) {
                    items.remove(i);
                    return true;
                }
            }
            false
        }),
        None => false,
    };
    if removed {
        refresh_has(ch);
    }
    removed
}

/// Synchronously invoke every subscriber with `(message, channelName)`.
fn publish(ch: &Value, msg: Value) -> Result<Value, String> {
    // Snapshot the subscribers + the name in one borrow, then invoke outside it —
    // a subscriber may re-enter the host (subscribe/publish/allocate).
    let subs: Vec<Value> = with_host(|h| match h.get(ch) {
        Some(JsObj::Object(p)) => match p.get("@@subs").and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    });
    let name_val = with_host(|h| match h.get(ch) {
        Some(JsObj::Object(p)) => p.get("@@name").cloned().unwrap_or(Value::Undef),
        _ => Value::Undef,
    });
    for cb in subs {
        crate::host::invoke(&cb, vec![msg.clone(), name_val.clone()], None)?;
    }
    Ok(Value::Undef)
}

/// Update the `hasSubscribers` data property to reflect the current count.
fn refresh_has(ch: &Value) {
    let has = sub_count(ch) > 0;
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(ch) {
            p.insert("hasSubscribers".into(), Value::Bool(has));
        }
    });
}

/// Heap-identity comparison for two reference values (subscriber callbacks are
/// always heap objects — functions or bound methods).
fn same_ref(a: &Value, b: &Value) -> bool {
    matches!((a, b), (Value::Obj(x), Value::Obj(y)) if x == y)
}
