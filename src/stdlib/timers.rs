//! Node `timers` and `timers/promises` modules.
//!
//! `require('timers')` re-exports the SAME timer primitives that already exist as
//! globals (`setTimeout`/`setInterval`/`setImmediate` + their `clear*`), so this
//! module owns NO queue of its own: every method delegates straight to
//! `builtins::call_builtin_function`, which schedules onto the single shared
//! `JsHost.macrotasks` queue. `timers.foo(...)` is therefore observably identical
//! to the global `foo(...)`.
//!
//! `require('timers/promises')` returns the promise-based variants: `setTimeout`
//! and `setImmediate` resolve a Promise after the delay instead of invoking a
//! callback. They are built on the SAME two substrates — the global timer
//! scheduler and the `@@presolve:<id>` native-continuation convention that
//! `builtins.rs` uses for Promise resolve reactions — so no new mechanism is
//! introduced: a timer is scheduled whose callback is the promise's resolver.

use super::arg_num;
use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

// ── timer handle objects (`Timeout` / `Immediate`) ───────────────────────────

/// Methods on a `Timeout` (returned by `setTimeout`/`setInterval`). Node's
/// prototype carries exactly `refresh`, `unref`, `ref`, `hasRef`, `close`;
/// `valueOf`/`toString` back the primitive coercion described on
/// [`new_handle`].
pub const TIMEOUT_METHODS: &[&str] = &[
    "ref", "unref", "hasRef", "refresh", "close", "valueOf", "toString",
];

/// Methods on an `Immediate` (returned by `setImmediate`). Node's `Immediate`
/// prototype has no `refresh` — there is no countdown to restart.
pub const IMMEDIATE_METHODS: &[&str] = &["ref", "unref", "hasRef", "close", "valueOf", "toString"];

/// Build the handle object a `set*` call returns: an `@@native`-tagged object
/// (`"Timeout"` or `"Immediate"`) carrying the scheduler's timer id in a hidden
/// `@@timerId` slot.
///
/// Node's handles coerce to their integer id (`String(t)` is `"2"`), which is
/// what keeps `clearTimeout` working for code that stashed the return value in a
/// number. `valueOf`/`toString` reproduce that for both ToPrimitive hints —
/// Node reaches it via an own `Symbol.toPrimitive`, which this does not model.
pub fn new_handle(id: u64, tag: &'static str) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        m.insert("@@timerId".into(), Value::Float(id as f64));
        h.new_object(m)
    })
}

/// The timer id behind a handle object, or `None` for anything else.
///
/// Read straight off the heap slot rather than through `ToPrimitive`: the
/// `clear*` builtins run inside a `with_host` borrow, and coercing via a
/// `valueOf` call would re-enter `with_host` and panic on the `RefCell`.
pub fn handle_id(v: &Value) -> Option<u64> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get("@@timerId").map(|n| h.to_number(n) as u64),
        _ => None,
    })
}

/// Dispatch a method on a `Timeout`/`Immediate` handle.
///
/// `ref`/`unref`/`refresh` return the handle itself (Node chains them); they are
/// inert once the timer has fired or been cleared, since the scheduler entry is
/// gone. `close` is Node's alias for clearing the timer.
pub fn instance_call(recv: &Value, method: &str, _args: &[Value]) -> Result<Value, String> {
    let id = handle_id(recv).unwrap_or(0);
    match method {
        "ref" => {
            with_host(|h| h.set_timer_refed(id, true));
            Ok(recv.clone())
        }
        "unref" => {
            with_host(|h| h.set_timer_refed(id, false));
            Ok(recv.clone())
        }
        "hasRef" => Ok(Value::Bool(with_host(|h| h.timer_has_ref(id)))),
        "refresh" => {
            with_host(|h| h.refresh_timer(id));
            Ok(recv.clone())
        }
        "close" => {
            with_host(|h| h.cancel_timer(id));
            Ok(recv.clone())
        }
        "valueOf" => Ok(Value::Float(id as f64)),
        "toString" => Ok(with_host(|h| h.new_str(id.to_string()))),
        _ => Err(crate::host::type_error(&format!(
            "timeout.{method} is not a function"
        ))),
    }
}

// ── timers (callback API) ────────────────────────────────────────────────────

/// Methods of the `timers` module. Each name is also a global; `call` forwards to
/// the identical global implementation, so there is one timer queue, not two.
pub const METHODS: &[&str] = &[
    "setTimeout",
    "setInterval",
    "setImmediate",
    "clearTimeout",
    "clearInterval",
    "clearImmediate",
];

/// Dispatch a `timers.<method>` call by delegating to the matching global timer
/// builtin. `clearImmediate` has no distinct global handler (the loop cancels by
/// id regardless of kind), so it maps to `clearTimeout`.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let global = match method {
        "setTimeout" | "setInterval" | "setImmediate" | "clearTimeout" | "clearInterval" => method,
        // No separate global `clearImmediate` handler exists; cancellation is by
        // timer id in both cases, so route it through `clearTimeout`.
        "clearImmediate" => "clearTimeout",
        _ => return None,
    };
    Some(crate::builtins::call_builtin_function(
        global,
        args.to_vec(),
    ))
}

// ── timers/promises (Promise API) ────────────────────────────────────────────

/// Methods of the `timers/promises` module (its namespace name carries no `.`, so
/// `stdlib::is_method` treats the whole `"timers/promises"` as the namespace).
///
/// `setInterval(delay[, value])` (an async iterator) is NOT implemented: the
/// `for await` machinery finds a native object's async iterator only via
/// `host::user_async_iterator_fn`, which needs a *callable* `@@asyncIterator`
/// stored property discoverable by `lookup_chain` — native-tagged objects
/// dispatch methods through the parent `instance_call` table, not stored
/// properties, so there is no way to expose it without editing `builtins.rs`/
/// `host.rs` (out of scope here).
pub const PROMISES_METHODS: &[&str] = &["setTimeout", "setImmediate", "setInterval"];

/// The async-iterator surface `timers/promises.setInterval` returns.
pub const INTERVAL_METHODS: &[&str] = &["next", "return", "@@asyncIterator"];

/// Dispatch a `timers/promises.<method>` call.
///
/// `setTimeout(delay[, value])` → a Promise that fulfills with `value` (undefined
/// if absent) after `delay` ms. `setImmediate([value])` → a Promise that fulfills
/// with `value` on the next loop turn. Any trailing `options` argument (Node's
/// `{ signal, ref }`) is accepted and ignored — abort/unref are not modeled.
pub fn promises_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "setTimeout" => {
            let delay = arg_num(args, 0);
            let value = args.get(1).cloned().unwrap_or(Value::Undef);
            Some(Ok(schedule_promise("setTimeout", Some(delay), value)))
        }
        "setImmediate" => {
            let value = args.first().cloned().unwrap_or(Value::Undef);
            Some(Ok(schedule_promise("setImmediate", None, value)))
        }
        // `setInterval(delay, value)` is an ASYNC ITERABLE, not a promise: it
        // yields `value` every `delay` for as long as it is iterated. It was
        // missing, so `for await (const v of setInterval(...))` had nothing to
        // call.
        "setInterval" => {
            let delay = arg_num(args, 0);
            let value = args.get(1).cloned().unwrap_or(Value::Undef);
            Some(Ok(interval_iterator(delay, value)))
        }
        _ => None,
    }
}

/// The object `timers/promises.setInterval` hands back: an async iterator that
/// resolves one `{ value, done: false }` per `delay`, and reports `done` once
/// `return()` has been called (which is what `break` inside `for await` does).
fn interval_iterator(delay: f64, value: Value) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("IntervalIterator"));
        m.insert("@@delay".into(), Value::Float(delay));
        m.insert("@@value".into(), value);
        m.insert("@@stopped".into(), Value::Bool(false));
        h.new_object(m)
    })
}

pub fn interval_call(recv: &Value, method: &str, _args: &[Value]) -> Result<Value, String> {
    let slot = |k: &str| {
        with_host(|h| match h.get(recv) {
            Some(JsObj::Object(p)) => p.get(k).cloned(),
            _ => None,
        })
    };
    match method {
        // An async iterable is its own iterator here, as node's is.
        "@@asyncIterator" => Ok(recv.clone()),
        "next" => {
            let stopped = slot("@@stopped").is_some_and(|v| with_host(|h| h.truthy(&v)));
            let value = slot("@@value").unwrap_or(Value::Undef);
            if stopped {
                let done = iter_result(Value::Undef, true);
                return Ok(resolved_promise(done));
            }
            let delay = slot("@@delay")
                .map(|v| with_host(|h| h.to_number(&v)))
                .unwrap_or(0.0);
            let result = iter_result(value, false);
            Ok(schedule_promise("setTimeout", Some(delay), result))
        }
        "return" => {
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get_mut(recv) {
                    p.insert("@@stopped".into(), Value::Bool(true));
                }
            });
            let done = iter_result(Value::Undef, true);
            Ok(resolved_promise(done))
        }
        _ => Err(crate::host::type_error(&format!(
            "intervalIterator.{method} is not a function"
        ))),
    }
}

/// `{ value, done }` — the iterator-result shape both arms return.
fn iter_result(value: Value, done: bool) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("value".into(), value);
        m.insert("done".into(), Value::Bool(done));
        h.new_object(m)
    })
}

/// A promise already fulfilled with `v`.
fn resolved_promise(v: Value) -> Value {
    let (promise, id) = with_host(|h| {
        let p = h.new_promise();
        let id = h.promise_id(&p).unwrap_or(0);
        (p, id)
    });
    crate::host::resolve_promise_val(id, v);
    promise
}

/// Allocate a pending Promise and schedule its resolution with `value` via the
/// existing global timer scheduler. The scheduled callback is a
/// `Builtin("@@presolve:<id>")` value — the same native continuation
/// `builtins.rs` invokes to fulfill a Promise — so when the timer fires it
/// resolves the Promise with the timer's extra argument (`value`).
fn schedule_promise(kind: &str, delay: Option<f64>, value: Value) -> Value {
    // Create the promise and grab its id for the resolver continuation.
    let (promise, id) = with_host(|h| {
        let p = h.new_promise();
        let id = h.promise_id(&p).unwrap_or(0);
        (p, id)
    });
    // The resolver: invoked with `[value]` when the timer fires.
    let resolver = with_host(|h| h.alloc(JsObj::Builtin(format!("@@presolve:{id}"))));
    // Route through the exact global scheduler. `setTimeout(cb, delay, value)`
    // and `setImmediate(cb, value)` pass `value` on to the callback as its first
    // argument, which `@@presolve:<id>` resolves the promise with.
    let timer_args = match delay {
        Some(d) => vec![resolver, Value::Float(d), value],
        None => vec![resolver, value],
    };
    // Ignore the returned timer id; the promise is the module's return value.
    let _ = crate::builtins::call_builtin_function(kind, timer_args);
    promise
}
