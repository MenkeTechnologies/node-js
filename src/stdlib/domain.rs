//! Node `domain` module (deprecated in Node, implemented here with its real
//! error-trapping semantics). A `Domain` is an EventEmitter (same `@@native` +
//! `@@on`/`@@once` shape as `events`/`net`) whose defining behaviour is
//! `domain.run(fn)`: it runs `fn` and, if `fn` throws, emits the domain's
//! `'error'` event with the thrown value instead of propagating the throw.
//!
//! Scope of the port: `.run`/`.bind`/`.intercept` genuinely trap synchronous
//! throws from the function they wrap and route them to `'error'`. What is NOT
//! wired is Node's *implicit* interception of errors emitted by emitters passed
//! to `.add()` — node-js has no per-emitter active-domain hook, so `.add()`/
//! `.remove()` only track membership (best-effort) and do not auto-forward those
//! emitters' `'error'` events. Only code run through `.run`/`.bind`/`.intercept`
//! is actually protected. `.enter()`/`.exit()` maintain a thread-local domain
//! stack whose top is exposed as `domain.active`.

use crate::host::{is_callable, take_exc_or_error, type_error, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::RefCell;

/// Module-level methods (`require('domain').create()`).
pub const METHODS: &[&str] = &["create", "createDomain"];

/// Instance methods carried by a `Domain` beyond the shared EventEmitter surface
/// (which `instance_has_method` adds via the emitter set).
pub const DOMAIN_METHODS: &[&str] = &[
    "run",
    "add",
    "remove",
    "bind",
    "intercept",
    "enter",
    "exit",
    "dispose",
];

thread_local! {
    /// The stack of entered domains; the top is `domain.active`. `.run`/`.enter`
    /// push, `.exit`/`.run`-completion pop.
    static STACK: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
}

// ── construction ─────────────────────────────────────────────────────────────

/// A fresh `Domain` (emitter object tagged `"Domain"`), carrying a hidden
/// `@@members` array for `.add`/`.remove` bookkeeping.
pub fn new_domain() -> Value {
    let members = with_host(|h| h.new_array(Vec::new()));
    let mut extra = IndexMap::new();
    extra.insert("@@members".to_string(), members);
    super::net::new_emitter_object("Domain", extra)
}

/// `require('domain')` module dispatch.
pub fn call(method: &str, _args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "create" | "createDomain" => Some(Ok(new_domain())),
        _ => None,
    }
}

/// `new domain.Domain()` (Node prefers `domain.create()`, but the constructor
/// exists).
pub fn construct(_args: &[Value]) -> Result<Value, String> {
    Ok(new_domain())
}

/// `domain.active` — the current (top-of-stack) domain, or `null`.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "active" => Some(active().unwrap_or_else(|| with_host(|h| h.null()))),
        _ => None,
    }
}

// ── instance dispatch ────────────────────────────────────────────────────────

pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    // EventEmitter methods (`on`/`once`/`emit`/…) delegate to `events` verbatim.
    if let Some(r) = emitter_dispatch(recv, method, &args) {
        return r;
    }
    match method {
        "run" => {
            let f = args.first().cloned().unwrap_or(Value::Undef);
            let call_args = args.get(1..).map(|s| s.to_vec()).unwrap_or_default();
            domain_run(recv, &f, call_args)
        }
        "add" => {
            if let Some(e) = args.into_iter().next() {
                track(recv, e, true);
            }
            Ok(Value::Undef)
        }
        "remove" => {
            if let Some(e) = args.into_iter().next() {
                track(recv, e, false);
            }
            Ok(Value::Undef)
        }
        "bind" => Ok(make_wrapper(
            recv,
            args.into_iter().next().unwrap_or(Value::Undef),
            "@@bound",
        )),
        "intercept" => Ok(make_wrapper(
            recv,
            args.into_iter().next().unwrap_or(Value::Undef),
            "@@intercept",
        )),
        "enter" => {
            enter(recv);
            Ok(Value::Undef)
        }
        "exit" => {
            exit(recv);
            Ok(Value::Undef)
        }
        // Deprecated no-op (Node's `domain.dispose()` was removed as unsafe).
        "dispose" => Ok(Value::Undef),
        // Internal continuation for the wrappers produced by `.bind`/`.intercept`.
        "@@bound" => {
            let domain = get_prop(recv, "@@boundDomain").unwrap_or_else(|| recv.clone());
            let f = get_prop(recv, "@@boundFn").unwrap_or(Value::Undef);
            domain_run(&domain, &f, args)
        }
        "@@intercept" => {
            let domain = get_prop(recv, "@@boundDomain").unwrap_or_else(|| recv.clone());
            let f = get_prop(recv, "@@boundFn").unwrap_or(Value::Undef);
            let err = args.first().cloned().unwrap_or(Value::Undef);
            let is_err = with_host(|h| !matches!(err, Value::Undef) && !h.is_null(&err));
            if is_err {
                emit_error(&domain, err);
                Ok(Value::Undef)
            } else {
                let rest = args.get(1..).map(|s| s.to_vec()).unwrap_or_default();
                domain_run(&domain, &f, rest)
            }
        }
        _ => Err(type_error(&format!("domain.{method} is not a function"))),
    }
}

// ── core: run a function inside the domain, trapping throws ───────────────────

/// Enter the domain, invoke `f(..call_args)`, exit. A throw is caught, the real
/// thrown value is emitted as the domain's `'error'` (never propagated), and
/// `undefined` is returned — the defining `domain` behaviour.
fn domain_run(domain: &Value, f: &Value, call_args: Vec<Value>) -> Result<Value, String> {
    if !with_host(|h| is_callable(h, f)) {
        return Err(type_error("domain.run requires a function"));
    }
    enter(domain);
    let r = crate::host::invoke(f, call_args, None);
    exit(domain);
    match r {
        Ok(v) => Ok(v),
        Err(e) => {
            // Recover the live thrown value (a real Error object when the code
            // did `throw new Error(...)`), then clear the host error/signal state
            // so execution continues past the trapped throw.
            let err = take_exc_or_error(&e);
            with_host(|h| h.signal = None);
            emit_error(domain, err);
            Ok(Value::Undef)
        }
    }
}

/// Emit the domain's `'error'` event carrying `err`.
fn emit_error(domain: &Value, err: Value) {
    let name = with_host(|h| h.new_str("error"));
    let _ = super::events::instance_call(domain, "emit", vec![name, err]);
}

/// Build the reusable wrapper returned by `.bind`/`.intercept`: a `BoundMethod`
/// over a `Domain`-tagged holder object that stores the target fn + owning
/// domain. Invoking it routes back through `instance_call` at `kind`.
fn make_wrapper(domain: &Value, f: Value, kind: &str) -> Value {
    let mut extra = IndexMap::new();
    extra.insert("@@boundFn".to_string(), f);
    extra.insert("@@boundDomain".to_string(), domain.clone());
    let holder = super::net::new_emitter_object("Domain", extra);
    with_host(|h| {
        h.alloc(JsObj::BoundMethod {
            recv: holder,
            name: kind.to_string(),
        })
    })
}

// ── domain stack (`enter`/`exit`/`active`) ───────────────────────────────────

fn enter(domain: &Value) {
    STACK.with(|s| s.borrow_mut().push(domain.clone()));
}

fn exit(domain: &Value) {
    STACK.with(|s| {
        let mut s = s.borrow_mut();
        // Node's `exit` removes this domain (and any it stacked above); the
        // best-effort port drops the topmost occurrence of it.
        if let Some(pos) = s.iter().rposition(|x| x == domain) {
            s.remove(pos);
        }
    });
}

fn active() -> Option<Value> {
    STACK.with(|s| s.borrow().last().cloned())
}

// ── `.add`/`.remove` membership bookkeeping (best-effort) ────────────────────

fn track(recv: &Value, emitter: Value, add: bool) {
    with_host(|h| {
        let arr = match h.get(recv) {
            Some(JsObj::Object(p)) => p.get("@@members").cloned(),
            _ => None,
        };
        if let Some(a) = arr {
            if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                if add {
                    if !items.iter().any(|x| x == &emitter) {
                        items.push(emitter);
                    }
                } else if let Some(pos) = items.iter().position(|x| x == &emitter) {
                    items.remove(pos);
                }
            }
        }
    });
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn get_prop(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

/// EventEmitter method delegation (shared shape with `net`/`http` emitters).
fn emitter_dispatch(recv: &Value, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "on"
        | "once"
        | "emit"
        | "addListener"
        | "prependListener"
        | "prependOnceListener"
        | "removeListener"
        | "off"
        | "removeAllListeners"
        | "listeners"
        | "listenerCount"
        | "eventNames"
        | "setMaxListeners"
        | "getMaxListeners" => Some(super::events::instance_call(recv, method, args.to_vec())),
        _ => None,
    }
}
