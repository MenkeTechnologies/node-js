//! Node `dns` module, backed by the platform resolver via `std::net`.
//!
//! `std` exposes name resolution only through `ToSocketAddrs`, so every lookup
//! goes through `(hostname, 0).to_socket_addrs()` (port `0` is a placeholder —
//! only the resolved `IpAddr`s are used). There is no PTR/reverse API in `std`,
//! so `lookupService` is a documented best-effort echo. No addresses are ever
//! fabricated: when the host is offline or the name does not resolve, the error
//! callback fires with an `ENOTFOUND`-style `Error` — the correct behaviour.
//!
//! Callbacks are Node-style `(err, ...)`. Because node-js runs callbacks on the
//! main thread, they are invoked directly through `host::invoke`; the re-entrant
//! borrow discipline of `fs.rs` is preserved by building every argument inside a
//! `with_host` closure (which releases the host borrow when it returns) and only
//! then calling `invoke` outside any borrow.
//!
//! The `promises` sub-namespace is a nested object (see `constant`) whose members
//! are the `promiseLookup`/`promiseResolve4`/`promiseResolve6` routed methods, so
//! `dns.promises.lookup(host)` returns a Promise.

use super::arg_str;
use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::net::{IpAddr, ToSocketAddrs};

pub const METHODS: &[&str] = &[
    "lookup",
    "resolve",
    "resolve4",
    "resolve6",
    "lookupService",
    // `dns.promises.*` members, reachable through the `promises` nested object.
    "promiseLookup",
    "promiseResolve4",
    "promiseResolve6",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "lookup" => lookup(args),
        // Node's generic `dns.resolve` defaults to `A` (IPv4) records.
        "resolve" | "resolve4" => resolve_family(args, true),
        "resolve6" => resolve_family(args, false),
        "lookupService" => lookup_service(args),
        "promiseLookup" => promise_lookup(args),
        "promiseResolve4" => promise_resolve(args, true),
        "promiseResolve6" => promise_resolve(args, false),
        _ => return None,
    })
}

/// `dns.promises` nested object (and `dns.version`). Served through
/// `namespace_property` → `stdlib::constant` once the parent wires the `"dns"`
/// arm (see the module doc).
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "promises" => Some(with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("lookup".into(), h.alloc(JsObj::Builtin("dns.promiseLookup".into())));
            m.insert("resolve4".into(), h.alloc(JsObj::Builtin("dns.promiseResolve4".into())));
            m.insert("resolve6".into(), h.alloc(JsObj::Builtin("dns.promiseResolve6".into())));
            h.new_object(m)
        })),
        _ => None,
    }
}

/// `dns.lookup(hostname[, options|family], cb)`: resolve to a single address.
/// `cb(err)` on failure, else `cb(null, address, family)` with `family` 4 or 6.
fn lookup(args: &[Value]) -> Result<Value, String> {
    let hostname = arg_str(args, 0);
    let want = opt_family(args);
    let Some(cb) = args.last().cloned() else { return Ok(Value::Undef) };
    let chosen = resolve_addrs(&hostname).ok().and_then(|list| {
        list.into_iter().find(|ip| match want {
            Some(4) => ip.is_ipv4(),
            Some(6) => ip.is_ipv6(),
            _ => true,
        })
    });
    match chosen {
        Some(ip) => {
            let family = if ip.is_ipv4() { 4.0 } else { 6.0 };
            // Build the callback args, release the host borrow, then invoke.
            let (err, address) = with_host(|h| (h.null(), h.new_str(ip.to_string())));
            crate::host::invoke(&cb, vec![err, address, Value::Float(family)], None)?;
        }
        None => {
            let err = not_found(&hostname);
            crate::host::invoke(&cb, vec![err], None)?;
        }
    }
    Ok(Value::Undef)
}

/// `dns.resolve4`/`resolve6`: `cb(err, [addresses])` with the family filtered.
fn resolve_family(args: &[Value], want_v4: bool) -> Result<Value, String> {
    let hostname = arg_str(args, 0);
    let Some(cb) = args.last().cloned() else { return Ok(Value::Undef) };
    match resolve_addrs(&hostname) {
        Ok(list) => {
            let addrs: Vec<String> = list
                .into_iter()
                .filter(|ip| ip.is_ipv4() == want_v4)
                .map(|ip| ip.to_string())
                .collect();
            let (err, arr) = with_host(|h| {
                let items: Vec<Value> = addrs.into_iter().map(|s| h.new_str(s)).collect();
                (h.null(), h.new_array(items))
            });
            crate::host::invoke(&cb, vec![err, arr], None)?;
        }
        Err(_) => {
            let err = not_found(&hostname);
            crate::host::invoke(&cb, vec![err], None)?;
        }
    }
    Ok(Value::Undef)
}

/// `dns.lookupService(address, port, cb)`. `std` has no reverse (PTR) resolver,
/// so this is best-effort: the address is echoed as the hostname and the port as
/// the service. A real PTR lookup would require a dedicated resolver crate — a
/// documented limitation.
fn lookup_service(args: &[Value]) -> Result<Value, String> {
    let addr = arg_str(args, 0);
    let port = super::arg_num(args, 1);
    let Some(cb) = args.last().cloned() else { return Ok(Value::Undef) };
    let service = if port.is_finite() { (port as u64).to_string() } else { String::new() };
    let (err, host_v, svc_v) = with_host(|h| (h.null(), h.new_str(addr), h.new_str(service)));
    crate::host::invoke(&cb, vec![err, host_v, svc_v], None)?;
    Ok(Value::Undef)
}

/// `dns.promises.lookup(hostname)` → Promise of `{ address, family }`.
fn promise_lookup(args: &[Value]) -> Result<Value, String> {
    let hostname = arg_str(args, 0);
    let want = opt_family(args);
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p)).unwrap_or(0);
    let chosen = resolve_addrs(&hostname).ok().and_then(|list| {
        list.into_iter().find(|ip| match want {
            Some(4) => ip.is_ipv4(),
            Some(6) => ip.is_ipv6(),
            _ => true,
        })
    });
    match chosen {
        Some(ip) => {
            let family = if ip.is_ipv4() { 4.0 } else { 6.0 };
            let obj = with_host(|h| {
                let mut m = IndexMap::new();
                m.insert("address".into(), h.new_str(ip.to_string()));
                m.insert("family".into(), Value::Float(family));
                h.new_object(m)
            });
            crate::host::resolve_promise_val(id, obj);
        }
        None => crate::host::reject_promise_val(id, not_found(&hostname)),
    }
    Ok(p)
}

/// `dns.promises.resolve4`/`resolve6` → Promise of `[addresses]`.
fn promise_resolve(args: &[Value], want_v4: bool) -> Result<Value, String> {
    let hostname = arg_str(args, 0);
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p)).unwrap_or(0);
    match resolve_addrs(&hostname) {
        Ok(list) => {
            let addrs: Vec<String> = list
                .into_iter()
                .filter(|ip| ip.is_ipv4() == want_v4)
                .map(|ip| ip.to_string())
                .collect();
            let arr = with_host(|h| {
                let items: Vec<Value> = addrs.into_iter().map(|s| h.new_str(s)).collect();
                h.new_array(items)
            });
            crate::host::resolve_promise_val(id, arr);
        }
        Err(_) => crate::host::reject_promise_val(id, not_found(&hostname)),
    }
    Ok(p)
}

/// Resolve `hostname` to the full address list via the platform resolver.
fn resolve_addrs(hostname: &str) -> std::io::Result<Vec<IpAddr>> {
    (hostname, 0u16).to_socket_addrs().map(|it| it.map(|sa| sa.ip()).collect())
}

/// An `ENOTFOUND` `Error` value for a failed resolution (built through the
/// shared error synthesizer so it carries the right prototype/`message`).
fn not_found(hostname: &str) -> Value {
    with_host(|h| crate::builtins::synth_error(h, &format!("Error: ENOTFOUND getaddrinfo {hostname}")))
}

/// The requested address family for `lookup`: the optional middle argument may be
/// a numeric family (`4`/`6`) or an options object `{ family }`. `None` = any.
fn opt_family(args: &[Value]) -> Option<u8> {
    if args.len() < 3 {
        return None;
    }
    with_host(|h| match h.get(&args[1]) {
        Some(JsObj::Object(p)) => p.get("family").map(|f| h.to_number(f) as u8),
        _ => {
            let n = h.to_number(&args[1]);
            if n.is_finite() && n > 0.0 {
                Some(n as u8)
            } else {
                None
            }
        }
    })
}
