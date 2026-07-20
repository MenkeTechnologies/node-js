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
    Some(crate::builtins::call_builtin_function(global, args.to_vec()))
}

// ── timers/promises (Promise API) ────────────────────────────────────────────

/// Methods of the `timers/promises` module (its namespace name carries no `.`, so
/// `stdlib::is_method` treats the whole `"timers/promises"` as the namespace).
pub const PROMISES_METHODS: &[&str] = &["setTimeout", "setImmediate"];

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
        _ => None,
    }
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
