//! `Proxy` — the ECMAScript exotic object (10.5) whose essential internal
//! methods are redirected to a handler's traps.
//!
//! A Proxy is not a shape node-js could fake with a property map: every one of
//! its internal methods has to be diverted, so it is its own heap variant
//! (`JsObj::Proxy`) and this module is the single place the diversion happens.
//! The funnels the rest of the runtime already routes through —
//! `builtins::get_property` / `set_property` / `has_property` /
//! `delete_property` / `object_keys`, `host::invoke` / `construct_nt` — each
//! call into here first; when the handler has no trap for the operation, the
//! `no_trap` fallback re-runs the SAME funnel against the target, which is what
//! makes `new Proxy(t, {})` observationally indistinguishable from `t`.
//!
//! Not implemented, deliberately, and recorded in BUGS.md rather than faked: the
//! spec's trap-result *invariant* checks (10.5.x steps that throw when a trap
//! contradicts a non-configurable/non-extensible target property). node-js
//! reports the trap's answer as given. Every trap itself is real.

use crate::host::{self, with_host, JsObj};
use fusevm::Value;

/// `(target, handler)` when `v` is a Proxy — revoked or not.
pub fn parts(v: &Value) -> Option<(Value, Value)> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Proxy {
            target, handler, ..
        }) => Some((target.clone(), handler.clone())),
        _ => None,
    })
}

/// Whether `v` is a Proxy whose `[[ProxyHandler]]` is still live.
fn revoked(v: &Value) -> bool {
    with_host(|h| matches!(h.get(v), Some(JsObj::Proxy { revoked, .. }) if *revoked))
}

/// The proxy chain's ultimate non-proxy target — what `Array.isArray`,
/// `Object.prototype.toString` and `typeof` classify by (10.5.x defer those to
/// `[[ProxyTarget]]`, and a proxy of a proxy defers again).
pub fn ultimate_target(v: &Value) -> Option<Value> {
    let mut cur = parts(v)?.0;
    for _ in 0..100 {
        match parts(&cur) {
            Some((t, _)) => cur = t,
            None => return Some(cur),
        }
    }
    Some(cur)
}

/// V8's message for an operation attempted on a revoked proxy.
fn revoked_err(op: &str) -> String {
    host::type_error(&format!(
        "Cannot perform '{op}' on a proxy that has been revoked"
    ))
}

/// Resolve trap `name` on `v`'s handler.
///
/// `Ok(None)` means "not a proxy, or no such trap" — the caller runs its
/// ordinary path (against the target, for the no-trap case). A revoked proxy
/// and a non-callable trap both throw here, before any target work happens.
fn trap(v: &Value, name: &str) -> Result<Option<(Value, Value, Value)>, String> {
    let Some((target, handler)) = parts(v) else {
        return Ok(None);
    };
    if revoked(v) {
        return Err(revoked_err(name));
    }
    let t = crate::builtins::get_property(&handler, name)?;
    if matches!(t, Value::Undef) || with_host(|h| h.is_null(&t)) {
        return Ok(None);
    }
    if !with_host(|h| host::is_callable(h, &t)) {
        return Err(host::type_error(&format!(
            "'{}' returned for property '{name}' of object '#<Object>' is not a function",
            with_host(|h| h.str_of(&t))
        )));
    }
    Ok(Some((t, target, handler)))
}

/// The target of a proxy whose handler declines the operation (no trap), or
/// `None` when `v` is not a proxy at all. Errors on a revoked proxy.
fn no_trap(v: &Value, op: &str) -> Result<Option<Value>, String> {
    match parts(v) {
        None => Ok(None),
        Some((target, _)) if !revoked(v) => Ok(Some(target)),
        Some(_) => Err(revoked_err(op)),
    }
}

/// An internal property key as the JS value a trap receives: the SYMBOL for a
/// symbol-keyed property (`@@sym:7`, `@@iterator`), a string otherwise. A trap
/// that inspects its key argument must see what the script wrote.
pub fn key_value(k: &str) -> Value {
    with_host(|h| {
        if let Some(s) = h.symbol_of_key(k) {
            return s;
        }
        match k.strip_prefix("@@") {
            Some(name) if host::WELL_KNOWN_SYMBOLS.contains(&name) => h.well_known_symbol(name),
            _ => h.new_str(k),
        }
    })
}

fn call(t: &Value, handler: &Value, args: Vec<Value>) -> Result<Value, String> {
    host::invoke(t, args, Some(handler.clone()))
}

// ── the thirteen traps ───────────────────────────────────────────────────────

/// `[[Get]]`. `Ok(None)` → not a proxy; the caller proceeds normally.
pub fn get(v: &Value, key: &str, receiver: &Value) -> Result<Option<Value>, String> {
    if let Some((t, target, handler)) = trap(v, "get")? {
        let k = key_value(key);
        return call(&t, &handler, vec![target, k, receiver.clone()]).map(Some);
    }
    match no_trap(v, "get")? {
        Some(target) => crate::builtins::get_property_recv(&target, key, receiver).map(Some),
        None => Ok(None),
    }
}

/// `[[Set]]`. `Ok(true)` means the write was handled here.
pub fn set(v: &Value, key: &str, val: &Value, receiver: &Value) -> Result<bool, String> {
    if let Some((t, target, handler)) = trap(v, "set")? {
        let k = key_value(key);
        call(&t, &handler, vec![target, k, val.clone(), receiver.clone()])?;
        return Ok(true);
    }
    match no_trap(v, "set")? {
        Some(target) => {
            crate::builtins::set_property_pub(&target, key, val.clone())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// `[[HasProperty]]` (`key in proxy`).
pub fn has(v: &Value, key: &str) -> Result<Option<bool>, String> {
    if let Some((t, target, handler)) = trap(v, "has")? {
        let k = key_value(key);
        let r = call(&t, &handler, vec![target, k])?;
        return Ok(Some(with_host(|h| h.truthy(&r))));
    }
    match no_trap(v, "has")? {
        Some(target) => crate::builtins::has_property(&target, key).map(Some),
        None => Ok(None),
    }
}

/// `[[Delete]]`.
pub fn delete(v: &Value, key: &str) -> Result<Option<bool>, String> {
    if let Some((t, target, handler)) = trap(v, "deleteProperty")? {
        let k = key_value(key);
        let r = call(&t, &handler, vec![target, k])?;
        return Ok(Some(with_host(|h| h.truthy(&r))));
    }
    match no_trap(v, "deleteProperty")? {
        Some(target) => crate::builtins::delete_property(&target, key).map(Some),
        None => Ok(None),
    }
}

/// `[[OwnPropertyKeys]]`, as INTERNAL key strings (so a symbol key comes back as
/// `@@sym:<id>` — the form the rest of the runtime indexes by).
pub fn own_keys(v: &Value) -> Result<Option<Vec<String>>, String> {
    if let Some((t, target, handler)) = trap(v, "ownKeys")? {
        let r = call(&t, &handler, vec![target])?;
        let items = with_host(|h| h.iter_vec(&r))?;
        let mut out = Vec::with_capacity(items.len());
        for k in items {
            out.push(host::to_property_key(&k)?);
        }
        return Ok(Some(out));
    }
    match no_trap(v, "ownKeys")? {
        Some(target) => {
            let mut keys = with_host(|h| h.own_key_names(&target, false));
            keys.extend(with_host(|h| {
                h.own_symbol_keys(&target)
                    .iter()
                    .map(|s| h.property_key(s))
                    .collect::<Vec<_>>()
            }));
            Ok(Some(keys))
        }
        None => Ok(None),
    }
}

/// `[[GetOwnProperty]]` — the descriptor object (or `undefined`).
pub fn get_own_descriptor(v: &Value, key: &str) -> Result<Option<Value>, String> {
    if let Some((t, target, handler)) = trap(v, "getOwnPropertyDescriptor")? {
        let k = key_value(key);
        return call(&t, &handler, vec![target, k]).map(Some);
    }
    match no_trap(v, "getOwnPropertyDescriptor")? {
        Some(target) => {
            let k = key_value(key);
            crate::builtins::own_descriptor_pub(&target, k).map(Some)
        }
        None => Ok(None),
    }
}

/// `[[DefineOwnProperty]]`.
pub fn define_property(v: &Value, key: &str, desc: &Value) -> Result<bool, String> {
    if let Some((t, target, handler)) = trap(v, "defineProperty")? {
        let k = key_value(key);
        call(&t, &handler, vec![target, k, desc.clone()])?;
        return Ok(true);
    }
    match no_trap(v, "defineProperty")? {
        Some(target) => {
            let k = key_value(key);
            crate::builtins::define_property_pub(&target, k, desc.clone())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// `[[GetPrototypeOf]]`.
pub fn get_prototype_of(v: &Value) -> Result<Option<Value>, String> {
    if let Some((t, target, handler)) = trap(v, "getPrototypeOf")? {
        return call(&t, &handler, vec![target]).map(Some);
    }
    match no_trap(v, "getPrototypeOf")? {
        Some(target) => Ok(Some(crate::builtins::prototype_of(&target))),
        None => Ok(None),
    }
}

/// `[[SetPrototypeOf]]`.
pub fn set_prototype_of(v: &Value, proto: &Value) -> Result<bool, String> {
    if let Some((t, target, handler)) = trap(v, "setPrototypeOf")? {
        call(&t, &handler, vec![target, proto.clone()])?;
        return Ok(true);
    }
    match no_trap(v, "setPrototypeOf")? {
        Some(target) => {
            with_host(|h| h.set_proto(&target, proto.clone()));
            Ok(true)
        }
        None => Ok(false),
    }
}

/// `[[IsExtensible]]`.
pub fn is_extensible(v: &Value) -> Result<Option<bool>, String> {
    if let Some((t, target, handler)) = trap(v, "isExtensible")? {
        let r = call(&t, &handler, vec![target])?;
        return Ok(Some(with_host(|h| h.truthy(&r))));
    }
    match no_trap(v, "isExtensible")? {
        Some(target) => Ok(Some(with_host(|h| h.is_extensible(&target)))),
        None => Ok(None),
    }
}

/// `[[PreventExtensions]]`.
pub fn prevent_extensions(v: &Value) -> Result<bool, String> {
    if let Some((t, target, handler)) = trap(v, "preventExtensions")? {
        call(&t, &handler, vec![target])?;
        return Ok(true);
    }
    match no_trap(v, "preventExtensions")? {
        Some(target) => {
            with_host(|h| h.prevent_extensions(&target));
            Ok(true)
        }
        None => Ok(false),
    }
}

/// `[[Call]]`.
pub fn apply(v: &Value, args: Vec<Value>, this: Option<Value>) -> Result<Option<Value>, String> {
    if let Some((t, target, handler)) = trap(v, "apply")? {
        let this_arg = this.unwrap_or(Value::Undef);
        let list = with_host(|h| h.new_array(args));
        return call(&t, &handler, vec![target, this_arg, list]).map(Some);
    }
    match no_trap(v, "apply")? {
        Some(target) => host::invoke(&target, args, this).map(Some),
        None => Ok(None),
    }
}

/// `[[Construct]]`.
pub fn construct(v: &Value, args: Vec<Value>, new_target: &Value) -> Result<Option<Value>, String> {
    if let Some((t, target, handler)) = trap(v, "construct")? {
        let list = with_host(|h| h.new_array(args));
        return call(&t, &handler, vec![target, list, new_target.clone()]).map(Some);
    }
    match no_trap(v, "construct")? {
        Some(target) => host::construct_nt(&target, args, new_target.clone()).map(Some),
        None => Ok(None),
    }
}

// ── enumeration built on the traps ───────────────────────────────────────────

/// The own keys of a proxy that are ENUMERABLE string keys — `Object.keys`,
/// `for-in`'s own half, object spread and `JSON.stringify` all need this shape.
/// 10.5.11 defines it as `ownKeys` filtered by each key's `[[GetOwnProperty]]`,
/// so both traps really do run, in that order.
pub fn own_enum_string_keys(v: &Value) -> Result<Vec<String>, String> {
    let Some(keys) = own_keys(v)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for k in keys {
        if host::is_symbol_key(&k) {
            continue;
        }
        let Some(d) = get_own_descriptor(v, &k)? else {
            continue;
        };
        let enumerable = with_host(|h| match h.get(&d) {
            Some(JsObj::Object(p)) => p.get("enumerable").map(|e| h.truthy(e)).unwrap_or(false),
            _ => false,
        });
        if enumerable {
            out.push(k);
        }
    }
    Ok(out)
}

/// `(key, value)` for every own enumerable string key — spread / `Object.assign`
/// / `Object.entries` / `JSON.stringify`. Each value is read through the `get`
/// trap, as the spec's `CreateDataPropertyOrThrow(…, Get(from, key))` requires.
pub fn own_enum_entries(v: &Value) -> Result<Vec<(String, Value)>, String> {
    let keys = own_enum_string_keys(v)?;
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        let val = get(v, &k, v)?.unwrap_or(Value::Undef);
        out.push((k, val));
    }
    Ok(out)
}

/// Whether the proxy chain bottoms out in an Array — the shape `IsArray` and
/// `Array.prototype[Symbol.iterator]` both key off.
fn wraps_array(v: &Value) -> bool {
    match ultimate_target(v) {
        Some(t) => with_host(|h| matches!(h.get(&t), Some(JsObj::Array(_)))),
        None => false,
    }
}

/// `[...proxy]` / `for (… of proxy)`. `Ok(None)` → not a proxy.
///
/// Three cases, in the order `GetIterator` reaches them:
/// a user `Symbol.iterator` read THROUGH the `get` trap; an array target, whose
/// `Array.prototype[Symbol.iterator]` observably does `Get(O, "length")` then
/// `Get(O, i)` (so a `get` trap that lies about either is honored); and anything
/// else (Map/Set/string/generator target), which iterates as the target does.
pub fn iterate(v: &Value) -> Result<Option<Vec<Value>>, String> {
    if parts(v).is_none() {
        return Ok(None);
    }
    let array_backed = wraps_array(v);
    let iter_fn = get(v, "@@iterator", v)?.unwrap_or(Value::Undef);
    // node-js models `Array.prototype[Symbol.iterator]` as a thunk BOUND to the
    // array it was read off, where the real method is generic over `this`. Read
    // through a proxy, that thunk would walk the TARGET and ignore every answer
    // the `get` trap gave — so an array-backed proxy still holding the default
    // falls through to the length-driven walk, which is what the generic method
    // observably does. A user-installed iterator is an ordinary function value
    // and keeps the fast path.
    let default_array_iter =
        array_backed && with_host(|h| matches!(h.get(&iter_fn), Some(JsObj::BoundMethod { .. })));
    if !default_array_iter && with_host(|h| host::is_callable(h, &iter_fn)) {
        let iterator = host::invoke(&iter_fn, Vec::new(), Some(v.clone()))?;
        return host::drain_iterator(&iterator).map(Some);
    }
    if array_backed {
        let len_v = get(v, "length", v)?.unwrap_or(Value::Undef);
        let len = with_host(|h| h.to_number(&len_v));
        let len = if len.is_finite() && len > 0.0 {
            len as usize
        } else {
            0
        };
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            out.push(get(v, &i.to_string(), v)?.unwrap_or(Value::Undef));
        }
        return Ok(Some(out));
    }
    let target = no_trap(v, "get")?.expect("checked it is a proxy");
    host::iter_all(&target).map(Some)
}

/// The plain value `JSON.stringify` serializes a proxy as. `SerializeJSONArray`
/// and `SerializeJSONObject` both read every member through `[[Get]]`, so the
/// snapshot is taken through the traps rather than off the target.
pub fn json_snapshot(v: &Value) -> Result<Value, String> {
    if wraps_array(v) {
        let items = iterate(v)?.unwrap_or_default();
        return Ok(with_host(|h| h.new_array(items)));
    }
    let entries = own_enum_entries(v)?;
    Ok(with_host(|h| {
        let mut m = indexmap::IndexMap::new();
        for (k, val) in entries {
            m.insert(k, val);
        }
        h.new_object(m)
    }))
}

// ── construction ─────────────────────────────────────────────────────────────

/// `new Proxy(target, handler)` (10.5.14 `ProxyCreate`).
pub fn create(args: &[Value]) -> Result<Value, String> {
    let target = args.first().cloned().unwrap_or(Value::Undef);
    let handler = args.get(1).cloned().unwrap_or(Value::Undef);
    let ok = |v: &Value| {
        with_host(|h| matches!(v, Value::Obj(_)) && !h.is_null(v) && !host::is_primitive(h, v))
    };
    if !ok(&target) || !ok(&handler) {
        return Err(host::type_error(
            "Cannot create proxy with a non-object as target or handler",
        ));
    }
    Ok(with_host(|h| {
        h.alloc(JsObj::Proxy {
            target,
            handler,
            revoked: false,
        })
    }))
}

/// `Proxy.revocable(target, handler)` → `{ proxy, revoke }`. The revoker is a
/// builtin thunk keyed by the proxy's heap index, so calling it twice is the
/// no-op the spec asks for rather than a second teardown.
pub fn revocable(args: &[Value]) -> Result<Value, String> {
    let proxy = create(args)?;
    let idx = match proxy {
        Value::Obj(i) => i,
        _ => unreachable!("create returns a heap object"),
    };
    let revoke = with_host(|h| h.alloc(JsObj::Builtin(format!("@@prevoke:{idx}"))));
    Ok(with_host(|h| {
        let mut m = indexmap::IndexMap::new();
        m.insert("proxy".to_string(), proxy);
        m.insert("revoke".to_string(), revoke);
        h.new_object(m)
    }))
}

/// Run a `@@prevoke:<idx>` thunk: mark the proxy dead so every trap throws.
///
/// The target handle is KEPT rather than nulled as 10.5.15 step 5 words it,
/// because `typeof` is fixed at creation by whether the target was callable and
/// V8 still answers `'function'` for a revoked proxy of a function. Nothing can
/// read the target through the proxy anymore — `revoked` is checked before any
/// trap or fallback runs.
pub fn revoke(idx: u32) -> Value {
    with_host(|h| {
        if let Some(JsObj::Proxy { revoked, .. }) = h.get_mut(&Value::Obj(idx)) {
            *revoked = true;
        }
    });
    Value::Undef
}
