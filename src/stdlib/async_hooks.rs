//! Node `async_hooks` module — honest minimal implementation.
//!
//! node-js has no per-async-resource id tracking and no async-context
//! propagation, so most of this module is a deliberate, documented no-op whose
//! only job is to let code that defensively imports `async_hooks` load and run
//! without crashing:
//!
//!   - `executionAsyncId()` returns a fixed `1`, `triggerAsyncId()` returns `0`.
//!     These are NOT real resource ids — there is no async-resource graph.
//!   - `createHook({ init, before, after, destroy })` returns a hook object with
//!     chainable `enable()`/`disable()`. The registered callbacks are stored
//!     nowhere and NEVER FIRE — node-js does not instrument async resource
//!     lifetimes. This is intentional; do not treat it as a gap to "fill" by
//!     faking hook invocations.
//!
//! What IS real is `AsyncLocalStorage` for the SYNCHRONOUS case: `run(store, cb)`
//! makes `getStore()` return `store` for the duration of `cb` (and restores the
//! previous store afterwards), and `enterWith(store)` sets the current store for
//! subsequent synchronous `getStore()` calls. Because there is no async-context
//! propagation, a store set with `enterWith` (or visible inside `run`) does NOT
//! automatically follow into `setTimeout`/Promise callbacks — cross-async
//! propagation is not modeled. Within straight-line synchronous code the store is
//! correct.
//!
//! Instances are `@@native`-tagged objects (`AsyncLocalStorage` / `AsyncHook`)
//! dispatched through `instance_call`; the parent wires `construct`,
//! `native_tag`, `instance_has_method`, and `instance_call` (see the report).

use crate::host::{invoke, with_host};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Per-`AsyncLocalStorage`-instance store stack, keyed by the instance's heap
    /// index. Push on `run`/`enterWith`, pop on `run` exit. The top is what
    /// `getStore()` returns. A stack (not a single slot) so nested `run` calls
    /// restore the enclosing store correctly.
    static STORES: RefCell<HashMap<u32, Vec<Value>>> = RefCell::new(HashMap::new());
}

/// Module-level callable members.
pub const METHODS: &[&str] = &["executionAsyncId", "triggerAsyncId", "createHook"];

/// Instance method names by native tag — for the parent's `instance_has_method`
/// so a method *read* (`als.run.bind(...)`) resolves before it is invoked.
pub const ALS_METHODS: &[&str] = &["getStore", "run", "enterWith", "exit", "disable"];
pub const HOOK_METHODS: &[&str] = &["enable", "disable"];

pub fn call(method: &str, _args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        // Fixed placeholders — there is no async-resource id graph.
        "executionAsyncId" => Ok(Value::Float(1.0)),
        "triggerAsyncId" => Ok(Value::Float(0.0)),
        // The hook object; its callbacks never fire (see module docs).
        "createHook" => Ok(new_hook()),
        _ => return None,
    })
}

/// Construct a stdlib class instance (`new AsyncLocalStorage()`). `None` for any
/// other name so the parent's `construct` can fall through.
pub fn construct(name: &str, _args: &[Value]) -> Option<Result<Value, String>> {
    match name {
        "AsyncLocalStorage" => Some(Ok(new_native("AsyncLocalStorage"))),
        _ => None,
    }
}

/// A fresh `@@native`-tagged object carrying `tag`.
fn new_native(tag: &'static str) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        h.new_object(m)
    })
}

/// The object returned by `createHook`. Its `enable`/`disable` are no-ops that
/// return the hook itself (Node's chainable API); no callbacks are ever invoked.
fn new_hook() -> Value {
    new_native("AsyncHook")
}

/// Dispatch a method on a native `async_hooks` instance.
pub fn instance_call(tag: &str, recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match tag {
        // A createHook() result: enable/disable are no-ops returning `this` so
        // `createHook(...).enable()` chains work. No hook callbacks fire.
        "AsyncHook" => match method {
            "enable" | "disable" => Ok(recv.clone()),
            _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
        },
        "AsyncLocalStorage" => als_call(recv, method, args),
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

/// The instance's heap index (its store-stack key), or `0` for a non-heap value.
fn key(recv: &Value) -> u32 {
    match recv {
        Value::Obj(i) => *i,
        _ => 0,
    }
}

fn als_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    let id = key(recv);
    match method {
        // The current store (top of this instance's stack), or undefined.
        "getStore" => Ok(STORES.with(|s| {
            s.borrow().get(&id).and_then(|v| v.last().cloned()).unwrap_or(Value::Undef)
        })),
        // run(store, callback, ...args): set the store, call the callback with the
        // remaining args, restore the previous store, return the callback result.
        "run" => {
            let store = args.first().cloned().unwrap_or(Value::Undef);
            let cb = args.get(1).cloned().unwrap_or(Value::Undef);
            let rest = args.get(2..).map(|s| s.to_vec()).unwrap_or_default();
            with_store(id, store, cb, rest)
        }
        // exit(callback, ...args): run the callback with the store unset (undefined
        // pushed) for its duration.
        "exit" => {
            let cb = args.first().cloned().unwrap_or(Value::Undef);
            let rest = args.get(1..).map(|s| s.to_vec()).unwrap_or_default();
            with_store(id, Value::Undef, cb, rest)
        }
        // enterWith(store): set the current store for subsequent synchronous
        // getStore() calls (not popped automatically; not propagated across async).
        "enterWith" => {
            let store = args.first().cloned().unwrap_or(Value::Undef);
            STORES.with(|s| s.borrow_mut().entry(id).or_default().push(store));
            Ok(Value::Undef)
        }
        // disable(): drop all stores for this instance.
        "disable" => {
            STORES.with(|s| {
                s.borrow_mut().remove(&id);
            });
            Ok(Value::Undef)
        }
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

/// Push `store`, invoke `cb` with `rest` (releasing every host borrow first, so
/// the callback may re-enter the host), then always pop — even on error.
fn with_store(id: u32, store: Value, cb: Value, rest: Vec<Value>) -> Result<Value, String> {
    STORES.with(|s| s.borrow_mut().entry(id).or_default().push(store));
    let r = invoke(&cb, rest, None);
    STORES.with(|s| {
        if let Some(v) = s.borrow_mut().get_mut(&id) {
            v.pop();
        }
    });
    r
}
