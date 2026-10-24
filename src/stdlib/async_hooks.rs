//! Node `async_hooks` module — honest minimal implementation.
//!
//! There IS a real async-resource id graph, but only over the resources this
//! module itself creates:
//!
//!   - Every `new AsyncResource(type)` takes the next monotonically increasing
//!     `asyncId` (from 2; Node reserves 1 for the root context) and records the
//!     creating context's id as its `triggerAsyncId`.
//!   - `runInAsyncScope` makes that pair the current execution context for the
//!     duration of the call, so `executionAsyncId()` inside reports the
//!     resource and a resource constructed there inherits it as its parent.
//!     Nesting `runInAsyncScope` therefore builds a real parent chain.
//!
//! What is NOT modeled: node-js does not instrument timers, promises, sockets
//! or any other engine-level async resource, so `executionAsyncId()` inside a
//! `setTimeout`/`.then` callback reports the ROOT context (1) rather than a
//! per-callback id, and `triggerAsyncId()` there reports 0. Only the
//! `AsyncResource` graph above is real. Consequently:
//!
//!   - `createHook({ init, before, after, destroy })` returns a hook object with
//!     chainable `enable()`/`disable()`. The registered callbacks are stored
//!     nowhere and NEVER FIRE — node-js does not instrument async resource
//!     lifetimes. This is intentional; do not treat it as a gap to "fill" by
//!     faking hook invocations.
//!
//! `AsyncLocalStorage` is real across async boundaries: stores live in the
//! host's ASYNC CONTEXT FRAME (node's `AsyncContextFrame`), which every queued
//! task, timer and promise reaction captures when it is scheduled (a reaction:
//! when `.then`/`await` registers it) and restores while it runs. `run(store,
//! cb)` runs `cb` in a frame with the store set, `exit` in one without it,
//! `enterWith` replaces the current frame, and `AsyncLocalStorage.snapshot()` /
//! `bind()` and `AsyncResource` hold the frame they were made in.
//!
//! Instances are `@@native`-tagged objects (`AsyncLocalStorage` / `AsyncHook`)
//! dispatched through `instance_call`; the parent wires `construct`,
//! `native_tag`, `instance_has_method`, and `instance_call` (see the report).

use crate::host::{invoke, with_host};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::RefCell;

thread_local! {

    /// Monotonic async-id source. Node reserves 1 for the root execution
    /// context, so fresh resources start at 2.
    static NEXT_ASYNC_ID: RefCell<f64> = const { RefCell::new(2.0) };

    /// The execution-context stack as `(asyncId, triggerAsyncId)` pairs, rooted
    /// at Node's `(1, 0)`. `runInAsyncScope` pushes the resource's pair for the
    /// duration of the call, which is what makes `executionAsyncId()` inside the
    /// scope report the resource and a resource *created* in that scope inherit
    /// it as its `triggerAsyncId`.
    static EXEC_STACK: RefCell<Vec<(f64, f64)>> = const { RefCell::new(Vec::new()) };
}

/// The id of the currently-executing async context (Node's `executionAsyncId`).
pub fn execution_async_id() -> f64 {
    EXEC_STACK.with(|s| s.borrow().last().map(|p| p.0).unwrap_or(1.0))
}

/// The id of the context that *created* the currently-executing one.
pub fn trigger_async_id() -> f64 {
    EXEC_STACK.with(|s| s.borrow().last().map(|p| p.1).unwrap_or(0.0))
}

/// Take the next async id.
fn fresh_async_id() -> f64 {
    NEXT_ASYNC_ID.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n += 1.0;
        id
    })
}

/// Run `f` with `(async_id, trigger_id)` as the current execution context,
/// restoring the previous context even if `f` fails.
fn in_async_scope<T>(async_id: f64, trigger_id: f64, f: impl FnOnce() -> T) -> T {
    EXEC_STACK.with(|s| s.borrow_mut().push((async_id, trigger_id)));
    let r = f();
    EXEC_STACK.with(|s| {
        s.borrow_mut().pop();
    });
    r
}

/// Module-level callable members.
pub const METHODS: &[&str] = &["executionAsyncId", "triggerAsyncId", "createHook"];

/// Instance method names by native tag — for the parent's `instance_has_method`
/// so a method *read* (`als.run.bind(...)`) resolves before it is invoked.
pub const ALS_METHODS: &[&str] = &["getStore", "run", "enterWith", "exit", "disable"];
pub const HOOK_METHODS: &[&str] = &["enable", "disable"];
pub const RESOURCE_METHODS: &[&str] = &[
    "runInAsyncScope",
    "emitDestroy",
    "asyncId",
    "triggerAsyncId",
    "bind",
];

/// Static members on the `AsyncResource` constructor itself.
pub const RESOURCE_STATIC_METHODS: &[&str] = &["bind"];

/// `AsyncResource.bind(fn[, type[, thisArg]])` — with no async-context graph to
/// capture there is nothing to restore, so the bound function IS `fn` (bound to
/// `thisArg` when one is given). Node's own semantics reduce to this whenever no
/// context is active.
pub fn static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "bind" => {
            let f = args.first().cloned().unwrap_or(Value::Undef);
            let bound = bind_to(f, args.get(2).cloned());
            let frame = with_host(|h| h.async_frame.clone());
            Some(frame_runner(frame, Some(bound)))
        }
        _ => None,
    }
}

/// `fn` itself, or a `fn.bind(thisArg)` when a non-nullish receiver is supplied.
fn bind_to(f: Value, this: Option<Value>) -> Value {
    match this.filter(|t| !with_host(|h| h.is_nullish(t))) {
        Some(t) => with_host(|h| {
            h.alloc(crate::host::JsObj::BoundFunc {
                target: f,
                this: t,
                args: Vec::new(),
            })
        }),
        None => f,
    }
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "@@runInFrame" => run_in_frame(args),
        "executionAsyncId" => Ok(Value::Float(execution_async_id())),
        "triggerAsyncId" => Ok(Value::Float(trigger_async_id())),
        // The hook object; its callbacks never fire (see module docs).
        "createHook" => Ok(new_hook()),
        _ => return None,
    })
}

/// Construct a stdlib class instance (`new AsyncLocalStorage()`). `None` for any
/// other name so the parent's `construct` can fall through.
pub fn construct(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match name {
        "AsyncLocalStorage" => Some(Ok(new_native("AsyncLocalStorage"))),
        // `new AsyncResource(type[, options])` takes the next monotonic async id
        // and records the creating context as its `triggerAsyncId`, so a graph
        // built by nesting `runInAsyncScope` calls has real parent links.
        // `options.triggerAsyncId` overrides the inherited parent, as in Node.
        "AsyncResource" => {
            let r = new_native("AsyncResource");
            let frame_id = hold_frame(with_host(|h| h.async_frame.clone()));
            let trigger = args
                .get(1)
                .and_then(|o| {
                    with_host(|h| match h.get(o) {
                        Some(crate::host::JsObj::Object(p)) => p
                            .get("triggerAsyncId")
                            .filter(|v| !matches!(v, Value::Undef))
                            .map(|v| h.to_number(v)),
                        _ => None,
                    })
                })
                .unwrap_or_else(execution_async_id);
            with_host(|h| {
                if let Some(crate::host::JsObj::Object(p)) = h.get_mut(&r) {
                    p.insert("@@asyncId".into(), Value::Float(fresh_async_id()));
                    p.insert("@@triggerAsyncId".into(), Value::Float(trigger));
                    // The async context frame the resource was created in;
                    // `runInAsyncScope` and `bind` run in it.
                    p.insert("@@frame".into(), Value::Float(frame_id));
                }
            });
            Some(Ok(r))
        }
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

/// A hidden numeric slot on a native instance (`@@asyncId`), or `fallback`.
fn hidden_num(recv: &Value, key: &str, fallback: f64) -> f64 {
    with_host(|h| match h.get(recv) {
        Some(crate::host::JsObj::Object(p)) => {
            p.get(key).map(|v| h.to_number(v)).unwrap_or(fallback)
        }
        _ => fallback,
    })
}

/// The object returned by `createHook`. Its `enable`/`disable` are no-ops that
/// return the hook itself (Node's chainable API); no callbacks are ever invoked.
fn new_hook() -> Value {
    new_native("AsyncHook")
}

/// Dispatch a method on a native `async_hooks` instance.
pub fn instance_call(
    tag: &str,
    recv: &Value,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    match tag {
        // A createHook() result: enable/disable are no-ops returning `this` so
        // `createHook(...).enable()` chains work. No hook callbacks fire.
        "AsyncHook" => match method {
            "enable" | "disable" => Ok(recv.clone()),
            _ => Err(crate::host::type_error(&format!(
                "{method} is not a function"
            ))),
        },
        "AsyncLocalStorage" => als_call(recv, method, args),
        "AsyncResource" => resource_call(recv, method, args),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// An `AsyncResource` instance. Each carries a real monotonic `asyncId` and the
/// `triggerAsyncId` of the context that created it, and `runInAsyncScope` makes
/// that pair the current execution context for the duration of the call — so
/// `executionAsyncId()` inside the callback reports the resource, and a resource
/// constructed there records this one as its parent. `emitDestroy` still has
/// nothing to destroy (no `destroy` hooks fire; see the module docs).
fn resource_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match method {
        // runInAsyncScope(fn[, thisArg[, ...args]]) === fn.apply(thisArg, args),
        // run under this resource's async context.
        "runInAsyncScope" => {
            let f = args.first().cloned().unwrap_or(Value::Undef);
            // Pass the receiver through verbatim (Node uses `ReflectApply`), so
            // an explicit `null` gets the same sloppy-mode coercion `fn.call(null)`
            // already applies rather than silently becoming "no receiver".
            let this = args.get(1).cloned();
            let rest = args.get(2..).map(|s| s.to_vec()).unwrap_or_default();
            let id = hidden_num(recv, "@@asyncId", 1.0);
            let trigger = hidden_num(recv, "@@triggerAsyncId", 0.0);
            let frame = resource_frame(recv);
            in_async_scope(id, trigger, || {
                crate::host::in_frame(frame, || invoke(&f, rest, this))
            })
        }
        "bind" => {
            let f = args.first().cloned().unwrap_or(Value::Undef);
            let bound = bind_to(f, args.get(1).cloned());
            frame_runner(resource_frame(recv), Some(bound))
        }
        "emitDestroy" => Ok(recv.clone()),
        "asyncId" => Ok(Value::Float(hidden_num(recv, "@@asyncId", 1.0))),
        "triggerAsyncId" => Ok(Value::Float(hidden_num(recv, "@@triggerAsyncId", 0.0))),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
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
        // The store this instance has in the current async context frame.
        "getStore" => Ok(with_host(|h| {
            h.async_frame
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Undef)
        })),
        // run(store, callback, ...args): set the store, call the callback with the
        // remaining args, restore the previous store, return the callback result.
        "run" => {
            let store = args.first().cloned().unwrap_or(Value::Undef);
            let cb = args.get(1).cloned().unwrap_or(Value::Undef);
            let rest = args.get(2..).map(|s| s.to_vec()).unwrap_or_default();
            let frame = crate::host::frame_with(id, Some(store));
            crate::host::in_frame(frame, || invoke(&cb, rest, None))
        }
        // exit(callback, ...args): the callback runs with no store.
        "exit" => {
            let cb = args.first().cloned().unwrap_or(Value::Undef);
            let rest = args.get(1..).map(|s| s.to_vec()).unwrap_or_default();
            let frame = crate::host::frame_with(id, None);
            crate::host::in_frame(frame, || invoke(&cb, rest, None))
        }
        // enterWith(store): the store for the rest of the current execution and
        // everything it schedules.
        "enterWith" | "disable" => {
            let store =
                (method == "enterWith").then(|| args.first().cloned().unwrap_or(Value::Undef));
            let frame = crate::host::frame_with(id, store);
            with_host(|h| h.async_frame = frame);
            Ok(Value::Undef)
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// The frame an `AsyncResource` was created in.
fn resource_frame(recv: &Value) -> crate::host::AsyncFrame {
    let id = hidden_num(recv, "@@frame", -1.0);
    if id < 0.0 {
        return with_host(|h| h.async_frame.clone());
    }
    HELD_FRAMES
        .with(|f| f.borrow().get(id as usize).cloned())
        .unwrap_or_default()
}

/// Static members on the `AsyncLocalStorage` constructor.
pub const ALS_STATIC_METHODS: &[&str] = &["bind", "snapshot"];

/// `AsyncLocalStorage.snapshot()` — a function that runs its argument in the
/// frame current now — and `AsyncLocalStorage.bind(fn)` — `fn`, run in the
/// frame current now.
pub fn als_static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let frame = with_host(|h| h.async_frame.clone());
    Some(match method {
        "snapshot" => frame_runner(frame, None),
        "bind" => frame_runner(frame, Some(args.first().cloned().unwrap_or(Value::Undef))),
        _ => return None,
    })
}

thread_local! {
    /// Frames handed out to `snapshot`/`bind`/`AsyncResource`, by handle.
    static HELD_FRAMES: RefCell<Vec<crate::host::AsyncFrame>> = const { RefCell::new(Vec::new()) };
}

fn hold_frame(frame: crate::host::AsyncFrame) -> f64 {
    HELD_FRAMES.with(|f| {
        let mut f = f.borrow_mut();
        f.push(frame);
        (f.len() - 1) as f64
    })
}

/// A JS function over a held frame: with `target`, `target` bound to it (its
/// `this` and arguments passed through); without, node's snapshot runner
/// `(fn, ...args) => fn(...args)` in that frame.
fn frame_runner(frame: crate::host::AsyncFrame, target: Option<Value>) -> Result<Value, String> {
    thread_local! {
        static FACTORY: RefCell<Option<Value>> = const { RefCell::new(None) };
    }
    let factory = match FACTORY.with(|f| f.borrow().clone()) {
        Some(f) => f,
        None => {
            let f = crate::eval_internal(
                "(function (runner, id, target) {\n\
                   if (target === undefined) return function runInAsyncScope(fn, ...args) { return runner(id, fn, undefined, ...args); };\n\
                   return function (...args) { return runner(id, target, this, ...args); };\n\
                 })",
            )?;
            FACTORY.with(|c| *c.borrow_mut() = Some(f.clone()));
            f
        }
    };
    let id = Value::Float(hold_frame(frame));
    let runner = with_host(|h| {
        h.alloc(crate::host::JsObj::Builtin(
            "async_hooks.@@runInFrame".into(),
        ))
    });
    invoke(
        &factory,
        vec![runner, id, target.unwrap_or(Value::Undef)],
        None,
    )
}

/// `async_hooks.@@runInFrame(id, fn, this, ...args)`: `fn` called in held frame `id`.
pub fn run_in_frame(args: &[Value]) -> Result<Value, String> {
    let id = with_host(|h| h.to_number(args.first().unwrap_or(&Value::Undef))) as usize;
    let frame = HELD_FRAMES
        .with(|f| f.borrow().get(id).cloned())
        .unwrap_or_default();
    let f = args.get(1).cloned().unwrap_or(Value::Undef);
    let this = args.get(2).cloned().filter(|t| !matches!(t, Value::Undef));
    let rest = args.get(3..).map(|s| s.to_vec()).unwrap_or_default();
    crate::host::in_frame(frame, || invoke(&f, rest, this))
}
