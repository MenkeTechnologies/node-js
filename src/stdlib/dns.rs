//! Node `dns` module.
//!
//! `dns.lookup`/`dns.promises.lookup` use the platform resolver (`getaddrinfo`,
//! via `std::net::ToSocketAddrs`) exactly like Node — they honour `/etc/hosts`.
//! Every `resolve*`/`reverse` method is a real DNS query issued through
//! `hickory_resolver`'s blocking `Resolver`. Because Node runs its DNS callbacks
//! asynchronously, each query runs on a dedicated `std::thread`; when it finishes
//! it posts an `IoTask` back to the main thread which builds the JS record values
//! and fires the Node-style `(err, records)` callback (or settles the Promise).
//! `incr_handle`/`decr_handle` bracket the async op so the event loop stays alive.
//!
//! A per-thread override list (`setServers`) and the default result order are
//! module state; `dns.Resolver` instances carry their own `@@servers` list on the
//! instance object so instance methods query independent nameservers.

use super::arg_str;
use crate::host::{with_host, JsObj};
use fusevm::Value;
use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
use hickory_resolver::error::{ResolveError, ResolveErrorKind};
use hickory_resolver::proto::op::ResponseCode;
use hickory_resolver::proto::rr::rdata::caa::{Property as CaaProperty, Value as CaaValue};
use hickory_resolver::proto::rr::{Name, RData, RecordType};
use hickory_resolver::Resolver;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::net::{IpAddr, ToSocketAddrs};

pub const METHODS: &[&str] = &[
    // getaddrinfo-based (platform resolver, honours /etc/hosts)
    "lookup",
    "lookupService",
    // real DNS queries (callback form)
    "resolve",
    "resolve4",
    "resolve6",
    "resolveMx",
    "resolveTxt",
    "resolveCname",
    "resolveNs",
    "resolvePtr",
    "resolveSrv",
    "resolveSoa",
    "resolveNaptr",
    "resolveCaa",
    "resolveTlsa",
    "resolveAny",
    "reverse",
    // configuration
    "getServers",
    "setServers",
    "getDefaultResultOrder",
    "setDefaultResultOrder",
    // `dns.promises.*` members, reachable through the `promises` nested object.
    "promiseLookup",
    "promiseLookupService",
    "promiseResolve",
    "promiseResolve4",
    "promiseResolve6",
    "promiseResolveMx",
    "promiseResolveTxt",
    "promiseResolveCname",
    "promiseResolveNs",
    "promiseResolvePtr",
    "promiseResolveSrv",
    "promiseResolveSoa",
    "promiseResolveNaptr",
    "promiseResolveCaa",
    "promiseResolveTlsa",
    "promiseResolveAny",
    "promiseReverse",
];

/// Method surface of a `dns.Resolver` instance (wired by the parent via
/// `instance_has_method("Resolver", …)`).
pub const RESOLVER_METHODS: &[&str] = &[
    "getServers",
    "setServers",
    "resolve",
    "resolve4",
    "resolve6",
    "resolveMx",
    "resolveTxt",
    "resolveCname",
    "resolveNs",
    "resolvePtr",
    "resolveSrv",
    "resolveSoa",
    "resolveNaptr",
    "resolveCaa",
    "resolveTlsa",
    "resolveAny",
    "reverse",
    "cancel",
    "setLocalAddress",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "lookup" => lookup(args),
        "lookupService" => lookup_service(args),
        "resolve" => resolve_cb(args),
        "resolve4" => cb_query(Query::A, args),
        "resolve6" => cb_query(Query::Aaaa, args),
        "resolveMx" => cb_query(Query::Mx, args),
        "resolveTxt" => cb_query(Query::Txt, args),
        "resolveCname" => cb_query(Query::Cname, args),
        "resolveNs" => cb_query(Query::Ns, args),
        "resolvePtr" => cb_query(Query::Ptr, args),
        "resolveSrv" => cb_query(Query::Srv, args),
        "resolveSoa" => cb_query(Query::Soa, args),
        "resolveNaptr" => cb_query(Query::Naptr, args),
        "resolveCaa" => cb_query(Query::Caa, args),
        "resolveTlsa" => cb_query(Query::Tlsa, args),
        "resolveAny" => cb_query(Query::Any, args),
        "reverse" => cb_query(Query::Reverse, args),
        "getServers" => get_servers(),
        "setServers" => set_servers(args),
        "getDefaultResultOrder" => Ok(get_default_result_order()),
        "setDefaultResultOrder" => set_default_result_order(args),
        "promiseLookup" => promise_lookup(args),
        "promiseLookupService" => promise_lookup_service(args),
        "promiseResolve" => resolve_promise(args),
        "promiseResolve4" => promise_query(Query::A, args),
        "promiseResolve6" => promise_query(Query::Aaaa, args),
        "promiseResolveMx" => promise_query(Query::Mx, args),
        "promiseResolveTxt" => promise_query(Query::Txt, args),
        "promiseResolveCname" => promise_query(Query::Cname, args),
        "promiseResolveNs" => promise_query(Query::Ns, args),
        "promiseResolvePtr" => promise_query(Query::Ptr, args),
        "promiseResolveSrv" => promise_query(Query::Srv, args),
        "promiseResolveSoa" => promise_query(Query::Soa, args),
        "promiseResolveNaptr" => promise_query(Query::Naptr, args),
        "promiseResolveCaa" => promise_query(Query::Caa, args),
        "promiseResolveTlsa" => promise_query(Query::Tlsa, args),
        "promiseResolveAny" => promise_query(Query::Any, args),
        "promiseReverse" => promise_query(Query::Reverse, args),
        _ => return None,
    })
}

// ── module state (per-thread; node-js runs JS on one thread) ─────────────────

struct DnsState {
    /// `setServers` override; `None` = use the system resolver configuration.
    servers: Option<Vec<String>>,
    /// `getDefaultResultOrder` value (`"verbatim"` | `"ipv4first"`).
    order: String,
}

thread_local! {
    static STATE: RefCell<DnsState> =
        RefCell::new(DnsState { servers: None, order: "verbatim".into() });
}

fn module_servers() -> Option<Vec<String>> {
    STATE.with(|s| s.borrow().servers.clone())
}

// ── non-function exports ─────────────────────────────────────────────────────

/// `dns.promises` nested object plus the numeric flags and string error codes.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "promises" => Some(promises_object()),
        "ADDRCONFIG" => Some(Value::Float(1024.0)),
        "V4MAPPED" => Some(Value::Float(2048.0)),
        "ALL" => Some(Value::Float(256.0)),
        _ => code_constant(name).map(|c| with_host(|h| h.new_str(c))),
    }
}

/// String error-code constants (`dns.NODATA === 'ENODATA'`, …).
fn code_constant(name: &str) -> Option<&'static str> {
    Some(match name {
        "NODATA" => "ENODATA",
        "FORMERR" => "EFORMERR",
        "SERVFAIL" => "ESERVFAIL",
        "NOTFOUND" => "ENOTFOUND",
        "NOTIMP" => "ENOTIMP",
        "REFUSED" => "EREFUSED",
        "BADQUERY" => "EBADQUERY",
        "BADNAME" => "EBADNAME",
        "BADFAMILY" => "EBADFAMILY",
        "BADRESP" => "EBADRESP",
        "CONNREFUSED" => "ECONNREFUSED",
        "TIMEOUT" => "ETIMEOUT",
        "EOF" => "EOF",
        "FILE" => "EFILE",
        "NOMEM" => "ENOMEM",
        "DESTRUCTION" => "EDESTRUCTION",
        "BADSTR" => "EBADSTR",
        "BADFLAGS" => "EBADFLAGS",
        "NONAME" => "ENONAME",
        "BADHINTS" => "EBADHINTS",
        "NOTINITIALIZED" => "ENOTINITIALIZED",
        "LOADIPHLPAPI" => "ELOADIPHLPAPI",
        "ADDRGETNETWORKPARAMS" => "EADDRGETNETWORKPARAMS",
        "CANCELLED" => "ECANCELLED",
        _ => return None,
    })
}

/// `dns.promises` member name → the `dns.*` builtin it routes to.
const PROMISE_MEMBERS: &[(&str, &str)] = &[
    ("lookup", "promiseLookup"),
    ("lookupService", "promiseLookupService"),
    ("resolve", "promiseResolve"),
    ("resolve4", "promiseResolve4"),
    ("resolve6", "promiseResolve6"),
    ("resolveMx", "promiseResolveMx"),
    ("resolveTxt", "promiseResolveTxt"),
    ("resolveCname", "promiseResolveCname"),
    ("resolveNs", "promiseResolveNs"),
    ("resolvePtr", "promiseResolvePtr"),
    ("resolveSrv", "promiseResolveSrv"),
    ("resolveSoa", "promiseResolveSoa"),
    ("resolveNaptr", "promiseResolveNaptr"),
    ("resolveCaa", "promiseResolveCaa"),
    ("resolveTlsa", "promiseResolveTlsa"),
    ("resolveAny", "promiseResolveAny"),
    ("reverse", "promiseReverse"),
    ("getServers", "getServers"),
    ("setServers", "setServers"),
    ("getDefaultResultOrder", "getDefaultResultOrder"),
    ("setDefaultResultOrder", "setDefaultResultOrder"),
];

/// Build the `dns.promises` namespace object: each member is a `Builtin`
/// reference routed back through `call`.
fn promises_object() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        for (key, target) in PROMISE_MEMBERS {
            let b = h.alloc(JsObj::Builtin(format!("dns.{target}")));
            m.insert((*key).into(), b);
        }
        h.new_object(m)
    })
}

// ── dns.lookup (getaddrinfo) ─────────────────────────────────────────────────

/// `dns.lookup(hostname[, options|family], cb)` → `cb(null, address, family)`.
fn lookup(args: &[Value]) -> Result<Value, String> {
    let hostname = arg_str(args, 0);
    let want = opt_family(args);
    let Some(cb) = args.last().cloned() else {
        return Ok(Value::Undef);
    };
    match pick_addr(&hostname, want) {
        Some(ip) => {
            let family = if ip.is_ipv4() { 4.0 } else { 6.0 };
            let (err, address) = with_host(|h| (h.null(), h.new_str(ip.to_string())));
            crate::host::invoke(&cb, vec![err, address, Value::Float(family)], None)?;
        }
        None => {
            let err = dns_error("ENOTFOUND", "getaddrinfo", &hostname);
            crate::host::invoke(&cb, vec![err], None)?;
        }
    }
    Ok(Value::Undef)
}

/// `dns.promises.lookup(hostname)` → Promise of `{ address, family }`.
fn promise_lookup(args: &[Value]) -> Result<Value, String> {
    let hostname = arg_str(args, 0);
    let want = opt_family(args);
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p)).unwrap_or(0);
    match pick_addr(&hostname, want) {
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
        None => {
            crate::host::reject_promise_val(id, dns_error("ENOTFOUND", "getaddrinfo", &hostname))
        }
    }
    Ok(p)
}

/// Resolve `hostname` through `getaddrinfo`, filtered by requested family.
fn pick_addr(hostname: &str, want: Option<u8>) -> Option<IpAddr> {
    (hostname, 0u16)
        .to_socket_addrs()
        .ok()?
        .map(|sa| sa.ip())
        .find(|ip| match want {
            Some(4) => ip.is_ipv4(),
            Some(6) => ip.is_ipv6(),
            _ => true,
        })
}

/// The optional middle argument of `lookup`: numeric family or `{ family }`.
fn opt_family(args: &[Value]) -> Option<u8> {
    if args.len() < 3 {
        return None;
    }
    with_host(|h| match h.get(&args[1]) {
        Some(JsObj::Object(p)) => p.get("family").map(|f| h.to_number(f) as u8),
        _ => {
            let n = h.to_number(&args[1]);
            (n.is_finite() && n > 0.0).then_some(n as u8)
        }
    })
}

// ── lookupService (real reverse + service name) ──────────────────────────────

/// `dns.lookupService(address, port, cb)` → `cb(null, hostname, service)`.
fn lookup_service(args: &[Value]) -> Result<Value, String> {
    let addr = arg_str(args, 0);
    let port = super::arg_num(args, 1);
    let Some(cb) = args.last().cloned() else {
        return Ok(Value::Undef);
    };
    let servers = module_servers();
    with_host(|h| h.incr_handle());
    let tx = with_host(|h| h.io_sender());
    std::thread::spawn(move || {
        let result = run_lookup_service(&addr, port, servers.as_deref());
        let label = addr;
        let _ = tx.send(Box::new(move || {
            with_host(|h| h.decr_handle());
            let call_args = match result {
                Ok((host, service)) => {
                    let (n, hv, sv) =
                        with_host(|h| (h.null(), h.new_str(host), h.new_str(service)));
                    vec![n, hv, sv]
                }
                Err(code) => vec![dns_error(&code, "getnameinfo", &label)],
            };
            if let Err(e) = crate::host::invoke(&cb, call_args, None) {
                eprintln!("{e}");
            }
            Ok(())
        }));
    });
    Ok(Value::Undef)
}

/// `dns.promises.lookupService(address, port)` → Promise of `{ hostname, service }`.
fn promise_lookup_service(args: &[Value]) -> Result<Value, String> {
    let addr = arg_str(args, 0);
    let port = super::arg_num(args, 1);
    let servers = module_servers();
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p)).unwrap_or(0);
    with_host(|h| h.incr_handle());
    let tx = with_host(|h| h.io_sender());
    std::thread::spawn(move || {
        let result = run_lookup_service(&addr, port, servers.as_deref());
        let label = addr;
        let _ = tx.send(Box::new(move || {
            with_host(|h| h.decr_handle());
            match result {
                Ok((host, service)) => {
                    let obj = with_host(|h| {
                        let mut m = IndexMap::new();
                        m.insert("hostname".into(), h.new_str(host));
                        m.insert("service".into(), h.new_str(service));
                        h.new_object(m)
                    });
                    crate::host::resolve_promise_val(id, obj);
                }
                Err(code) => {
                    crate::host::reject_promise_val(id, dns_error(&code, "getnameinfo", &label))
                }
            }
            Ok(())
        }));
    });
    Ok(p)
}

fn run_lookup_service(
    addr: &str,
    port: f64,
    servers: Option<&[String]>,
) -> Result<(String, String), String> {
    let ip: IpAddr = addr.parse().map_err(|_| "EINVAL".to_string())?;
    let resolver = build_resolver(servers)?;
    let lookup = resolver
        .reverse_lookup(ip)
        .map_err(|e| err_code(&e).to_string())?;
    let host = lookup
        .iter()
        .next()
        .map(|n| name_str(n))
        .ok_or_else(|| "ENOTFOUND".to_string())?;
    Ok((host, port_service(port as u16)))
}

/// Well-known IANA port → service names (`getservbyport` parity for common
/// ports); unknown ports fall back to the numeric string.
fn port_service(port: u16) -> String {
    let name = match port {
        20 => "ftp-data",
        21 => "ftp",
        22 => "ssh",
        23 => "telnet",
        25 => "smtp",
        53 => "domain",
        67 => "bootps",
        68 => "bootpc",
        69 => "tftp",
        79 => "finger",
        80 => "http",
        110 => "pop3",
        111 => "sunrpc",
        119 => "nntp",
        123 => "ntp",
        143 => "imap",
        161 => "snmp",
        194 => "irc",
        389 => "ldap",
        443 => "https",
        445 => "microsoft-ds",
        465 => "submissions",
        514 => "shell",
        515 => "printer",
        587 => "submission",
        631 => "ipp",
        636 => "ldaps",
        993 => "imaps",
        995 => "pop3s",
        1194 => "openvpn",
        1433 => "ms-sql-s",
        3306 => "mysql",
        3389 => "ms-wbt-server",
        5432 => "postgresql",
        5060 => "sip",
        5061 => "sips",
        6379 => "redis",
        8080 => "http-alt",
        8443 => "https-alt",
        _ => return port.to_string(),
    };
    name.to_string()
}

// ── generic resolve dispatch ─────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Query {
    A,
    Aaaa,
    Mx,
    Txt,
    Cname,
    Ns,
    Ptr,
    Srv,
    Soa,
    Naptr,
    Caa,
    Tlsa,
    Any,
    Reverse,
}

impl Query {
    /// The `syscall` string Node reports on the error object.
    fn syscall(self) -> &'static str {
        match self {
            Query::A => "queryA",
            Query::Aaaa => "queryAaaa",
            Query::Mx => "queryMx",
            Query::Txt => "queryTxt",
            Query::Cname => "queryCname",
            Query::Ns => "queryNs",
            Query::Ptr => "queryPtr",
            Query::Srv => "querySrv",
            Query::Soa => "querySoa",
            Query::Naptr => "queryNaptr",
            Query::Caa => "queryCaa",
            Query::Tlsa => "queryTlsa",
            Query::Any => "queryAny",
            Query::Reverse => "getHostByAddr",
        }
    }
}

fn query_of(rrtype: &str) -> Option<Query> {
    Some(match rrtype {
        "A" => Query::A,
        "AAAA" => Query::Aaaa,
        "MX" => Query::Mx,
        "TXT" => Query::Txt,
        "CNAME" => Query::Cname,
        "NS" => Query::Ns,
        "PTR" => Query::Ptr,
        "SRV" => Query::Srv,
        "SOA" => Query::Soa,
        "NAPTR" => Query::Naptr,
        "CAA" => Query::Caa,
        "TLSA" => Query::Tlsa,
        "ANY" => Query::Any,
        _ => return None,
    })
}

fn cb_query(q: Query, args: &[Value]) -> Result<Value, String> {
    let name = arg_str(args, 0);
    let Some(cb) = args.last().cloned() else {
        return Ok(Value::Undef);
    };
    start_query(q, name, module_servers(), cb);
    Ok(Value::Undef)
}

fn promise_query(q: Query, args: &[Value]) -> Result<Value, String> {
    let name = arg_str(args, 0);
    Ok(start_promise_query(q, name, module_servers()))
}

/// `dns.resolve(hostname[, rrtype], cb)`.
fn resolve_cb(args: &[Value]) -> Result<Value, String> {
    let name = arg_str(args, 0);
    let Some(cb) = args.last().cloned() else {
        return Ok(Value::Undef);
    };
    let rrtype = if args.len() > 2 {
        arg_str(args, 1)
    } else {
        "A".into()
    };
    let q = query_of(&rrtype).unwrap_or(Query::A);
    start_query(q, name, module_servers(), cb);
    Ok(Value::Undef)
}

/// `dns.promises.resolve(hostname[, rrtype])`.
fn resolve_promise(args: &[Value]) -> Result<Value, String> {
    let name = arg_str(args, 0);
    let rrtype = if args.len() > 1 {
        arg_str(args, 1)
    } else {
        "A".into()
    };
    let q = query_of(&rrtype).unwrap_or(Query::A);
    Ok(start_promise_query(q, name, module_servers()))
}

// ── async drivers (spawn thread, post IoTask back) ───────────────────────────

fn start_query(q: Query, name: String, servers: Option<Vec<String>>, cb: Value) {
    with_host(|h| h.incr_handle());
    let tx = with_host(|h| h.io_sender());
    std::thread::spawn(move || {
        let res = run_query(q, &name, servers.as_deref());
        let label = name;
        let _ = tx.send(Box::new(move || {
            with_host(|h| h.decr_handle());
            let call_args = match res {
                Ok(data) => {
                    let val = dns_to_value(data);
                    let n = with_host(|h| h.null());
                    vec![n, val]
                }
                Err(code) => vec![dns_error(&code, q.syscall(), &label)],
            };
            if let Err(e) = crate::host::invoke(&cb, call_args, None) {
                eprintln!("{e}");
            }
            Ok(())
        }));
    });
}

fn start_promise_query(q: Query, name: String, servers: Option<Vec<String>>) -> Value {
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p)).unwrap_or(0);
    with_host(|h| h.incr_handle());
    let tx = with_host(|h| h.io_sender());
    std::thread::spawn(move || {
        let res = run_query(q, &name, servers.as_deref());
        let label = name;
        let _ = tx.send(Box::new(move || {
            with_host(|h| h.decr_handle());
            match res {
                Ok(data) => crate::host::resolve_promise_val(id, dns_to_value(data)),
                Err(code) => {
                    crate::host::reject_promise_val(id, dns_error(&code, q.syscall(), &label))
                }
            }
            Ok(())
        }));
    });
    p
}

/// Callback-vs-Promise dispatch used by `Resolver` instance methods.
fn run_or_promise(
    q: Query,
    name: String,
    servers: Option<Vec<String>>,
    cb: Option<Value>,
    promises: bool,
) -> Value {
    if promises {
        start_promise_query(q, name, servers)
    } else if let Some(cb) = cb {
        start_query(q, name, servers, cb);
        Value::Undef
    } else {
        Value::Undef
    }
}

// ── the blocking query itself (runs on a worker thread) ──────────────────────

/// Plain-data (Send) record set produced by a worker thread; converted to JS
/// values on the main thread by `dns_to_value`.
enum DnsResult {
    Strings(Vec<String>),
    Mx(Vec<(u16, String)>),
    Txt(Vec<Vec<String>>),
    Srv(Vec<SrvRec>),
    Soa(SoaRec),
    Naptr(Vec<NaptrRec>),
    Caa(Vec<CaaRec>),
    Tlsa(Vec<TlsaRec>),
    Any(Vec<AnyRec>),
}

struct SoaRec {
    nsname: String,
    hostmaster: String,
    serial: u32,
    refresh: i32,
    retry: i32,
    expire: i32,
    minttl: u32,
}

struct SrvRec {
    priority: u16,
    weight: u16,
    port: u16,
    name: String,
}

struct NaptrRec {
    flags: String,
    service: String,
    regexp: String,
    replacement: String,
    order: u16,
    preference: u16,
}

struct CaaRec {
    critical: u16,
    tag: String,
    value: String,
}

struct TlsaRec {
    cert_usage: u8,
    selector: u8,
    matching: u8,
    data: Vec<u8>,
}

enum AnyRec {
    A(String, u32),
    Aaaa(String, u32),
    Cname(String),
    Mx(u16, String),
    Ns(String),
    Ptr(String),
    Txt(Vec<String>),
    Srv(SrvRec),
    Soa(SoaRec),
    Naptr(NaptrRec),
    Caa(CaaRec),
}

fn run_query(q: Query, name: &str, servers: Option<&[String]>) -> Result<DnsResult, String> {
    let r = build_resolver(servers)?;
    let ec = |e: ResolveError| err_code(&e).to_string();
    match q {
        Query::A => {
            let l = r.ipv4_lookup(name).map_err(ec)?;
            Ok(DnsResult::Strings(
                l.iter().map(|a| a.to_string()).collect(),
            ))
        }
        Query::Aaaa => {
            let l = r.ipv6_lookup(name).map_err(ec)?;
            Ok(DnsResult::Strings(
                l.iter().map(|a| a.to_string()).collect(),
            ))
        }
        Query::Mx => {
            let l = r.mx_lookup(name).map_err(ec)?;
            Ok(DnsResult::Mx(
                l.iter()
                    .map(|m| (m.preference(), name_str(m.exchange())))
                    .collect(),
            ))
        }
        Query::Txt => {
            let l = r.txt_lookup(name).map_err(ec)?;
            let entries = l
                .iter()
                .map(|t| {
                    t.txt_data()
                        .iter()
                        .map(|b| String::from_utf8_lossy(b).into_owned())
                        .collect()
                })
                .collect();
            Ok(DnsResult::Txt(entries))
        }
        Query::Cname => {
            let l = r.lookup(name, RecordType::CNAME).map_err(ec)?;
            let names = l
                .iter()
                .filter_map(|d| match d {
                    RData::CNAME(c) => Some(name_str(c)),
                    _ => None,
                })
                .collect();
            Ok(DnsResult::Strings(names))
        }
        Query::Ns => {
            let l = r.ns_lookup(name).map_err(ec)?;
            Ok(DnsResult::Strings(l.iter().map(|n| name_str(n)).collect()))
        }
        Query::Ptr => {
            let l = r.lookup(name, RecordType::PTR).map_err(ec)?;
            let names = l
                .iter()
                .filter_map(|d| match d {
                    RData::PTR(p) => Some(name_str(p)),
                    _ => None,
                })
                .collect();
            Ok(DnsResult::Strings(names))
        }
        Query::Srv => {
            let l = r.srv_lookup(name).map_err(ec)?;
            Ok(DnsResult::Srv(l.iter().map(srv_rec).collect()))
        }
        Query::Soa => {
            let l = r.soa_lookup(name).map_err(ec)?;
            let soa = l.iter().next().ok_or_else(|| "ENODATA".to_string())?;
            Ok(DnsResult::Soa(soa_rec(soa)))
        }
        Query::Naptr => {
            let l = r.lookup(name, RecordType::NAPTR).map_err(ec)?;
            let recs = l
                .iter()
                .filter_map(|d| match d {
                    RData::NAPTR(n) => Some(naptr_rec(n)),
                    _ => None,
                })
                .collect();
            Ok(DnsResult::Naptr(recs))
        }
        Query::Caa => {
            let l = r.lookup(name, RecordType::CAA).map_err(ec)?;
            let recs = l
                .iter()
                .filter_map(|d| match d {
                    RData::CAA(c) => Some(caa_rec(c)),
                    _ => None,
                })
                .collect();
            Ok(DnsResult::Caa(recs))
        }
        Query::Tlsa => {
            let l = r.tlsa_lookup(name).map_err(ec)?;
            let recs = l
                .iter()
                .map(|t| TlsaRec {
                    cert_usage: t.cert_usage().into(),
                    selector: t.selector().into(),
                    matching: t.matching().into(),
                    data: t.cert_data().to_vec(),
                })
                .collect();
            Ok(DnsResult::Tlsa(recs))
        }
        Query::Any => {
            let l = r.lookup(name, RecordType::ANY).map_err(ec)?;
            let recs = l.records().iter().filter_map(any_rec).collect();
            Ok(DnsResult::Any(recs))
        }
        Query::Reverse => {
            let ip: IpAddr = name.parse().map_err(|_| "EINVAL".to_string())?;
            let l = r.reverse_lookup(ip).map_err(ec)?;
            Ok(DnsResult::Strings(l.iter().map(|n| name_str(n)).collect()))
        }
    }
}

// ── record → plain-data extractors (worker thread) ───────────────────────────

fn name_str(n: &Name) -> String {
    n.to_ascii().trim_end_matches('.').to_string()
}

fn srv_rec(s: &hickory_resolver::proto::rr::rdata::SRV) -> SrvRec {
    SrvRec {
        priority: s.priority(),
        weight: s.weight(),
        port: s.port(),
        name: name_str(s.target()),
    }
}

fn soa_rec(s: &hickory_resolver::proto::rr::rdata::SOA) -> SoaRec {
    SoaRec {
        nsname: name_str(s.mname()),
        hostmaster: name_str(s.rname()),
        serial: s.serial(),
        refresh: s.refresh(),
        retry: s.retry(),
        expire: s.expire(),
        minttl: s.minimum(),
    }
}

fn naptr_rec(n: &hickory_resolver::proto::rr::rdata::NAPTR) -> NaptrRec {
    NaptrRec {
        flags: String::from_utf8_lossy(n.flags()).into_owned(),
        service: String::from_utf8_lossy(n.services()).into_owned(),
        regexp: String::from_utf8_lossy(n.regexp()).into_owned(),
        replacement: name_str(n.replacement()),
        order: n.order(),
        preference: n.preference(),
    }
}

fn caa_rec(c: &hickory_resolver::proto::rr::rdata::CAA) -> CaaRec {
    CaaRec {
        critical: if c.issuer_critical() { 128 } else { 0 },
        tag: caa_tag(c.tag()),
        value: caa_value(c.value()),
    }
}

fn caa_tag(p: &CaaProperty) -> String {
    match p {
        CaaProperty::Issue => "issue".into(),
        CaaProperty::IssueWild => "issuewild".into(),
        CaaProperty::Iodef => "iodef".into(),
        CaaProperty::Unknown(s) => s.clone(),
    }
}

fn caa_value(v: &CaaValue) -> String {
    match v {
        CaaValue::Issuer(name, kvs) => {
            let mut s = name.as_ref().map(name_str).unwrap_or_default();
            for kv in kvs {
                s.push_str("; ");
                s.push_str(kv.key());
                s.push('=');
                s.push_str(kv.value());
            }
            s
        }
        CaaValue::Url(u) => u.to_string(),
        CaaValue::Unknown(b) => String::from_utf8_lossy(b).into_owned(),
    }
}

/// Map one record from an `ANY` response to its `AnyRec`, skipping unmodelled
/// record types.
fn any_rec(rec: &hickory_resolver::proto::rr::Record) -> Option<AnyRec> {
    let ttl = rec.ttl();
    Some(match rec.data()? {
        RData::A(a) => AnyRec::A(a.to_string(), ttl),
        RData::AAAA(a) => AnyRec::Aaaa(a.to_string(), ttl),
        RData::CNAME(c) => AnyRec::Cname(name_str(c)),
        RData::MX(m) => AnyRec::Mx(m.preference(), name_str(m.exchange())),
        RData::NS(n) => AnyRec::Ns(name_str(n)),
        RData::PTR(p) => AnyRec::Ptr(name_str(p)),
        RData::TXT(t) => AnyRec::Txt(
            t.txt_data()
                .iter()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .collect(),
        ),
        RData::SRV(s) => AnyRec::Srv(srv_rec(s)),
        RData::SOA(s) => AnyRec::Soa(soa_rec(s)),
        RData::NAPTR(n) => AnyRec::Naptr(naptr_rec(n)),
        RData::CAA(c) => AnyRec::Caa(caa_rec(c)),
        _ => return None,
    })
}

// ── DnsResult → JS Value (main thread) ───────────────────────────────────────

fn dns_to_value(res: DnsResult) -> Value {
    match res {
        DnsResult::Strings(v) => with_host(|h| {
            let items: Vec<Value> = v.into_iter().map(|s| h.new_str(s)).collect();
            h.new_array(items)
        }),
        DnsResult::Mx(v) => with_host(|h| {
            let items: Vec<Value> = v
                .into_iter()
                .map(|(pri, ex)| {
                    let mut m = IndexMap::new();
                    m.insert("exchange".into(), h.new_str(ex));
                    m.insert("priority".into(), Value::Float(pri as f64));
                    m.insert("type".into(), h.new_str("MX"));
                    h.new_object(m)
                })
                .collect();
            h.new_array(items)
        }),
        DnsResult::Txt(v) => with_host(|h| {
            let items: Vec<Value> = v
                .into_iter()
                .map(|chunks| {
                    let inner: Vec<Value> = chunks.into_iter().map(|s| h.new_str(s)).collect();
                    h.new_array(inner)
                })
                .collect();
            h.new_array(items)
        }),
        DnsResult::Srv(v) => with_host(|h| {
            let items: Vec<Value> = v.iter().map(|s| srv_obj(h, s)).collect();
            h.new_array(items)
        }),
        DnsResult::Soa(s) => with_host(|h| soa_obj(h, &s)),
        DnsResult::Naptr(v) => with_host(|h| {
            let items: Vec<Value> = v.iter().map(|n| naptr_obj(h, n)).collect();
            h.new_array(items)
        }),
        DnsResult::Caa(v) => with_host(|h| {
            let items: Vec<Value> = v.iter().map(|c| caa_obj(h, c)).collect();
            h.new_array(items)
        }),
        DnsResult::Tlsa(v) => {
            // Buffer values must be built outside the object's `with_host`.
            let mut items = Vec::with_capacity(v.len());
            for t in v {
                let data = super::buffer::from_bytes(&t.data);
                let obj = with_host(|h| {
                    let mut m = IndexMap::new();
                    m.insert("certUsage".into(), Value::Float(t.cert_usage as f64));
                    m.insert("selector".into(), Value::Float(t.selector as f64));
                    m.insert("match".into(), Value::Float(t.matching as f64));
                    m.insert("data".into(), data);
                    h.new_object(m)
                });
                items.push(obj);
            }
            with_host(|h| h.new_array(items))
        }
        DnsResult::Any(v) => with_host(|h| {
            let items: Vec<Value> = v.into_iter().map(|rec| any_obj(h, rec)).collect();
            h.new_array(items)
        }),
    }
}

fn srv_obj(h: &mut crate::host::JsHost, s: &SrvRec) -> Value {
    let mut m = IndexMap::new();
    m.insert("name".into(), h.new_str(s.name.clone()));
    m.insert("port".into(), Value::Float(s.port as f64));
    m.insert("priority".into(), Value::Float(s.priority as f64));
    m.insert("weight".into(), Value::Float(s.weight as f64));
    m.insert("type".into(), h.new_str("SRV"));
    h.new_object(m)
}

fn soa_obj(h: &mut crate::host::JsHost, s: &SoaRec) -> Value {
    let mut m = IndexMap::new();
    m.insert("nsname".into(), h.new_str(s.nsname.clone()));
    m.insert("hostmaster".into(), h.new_str(s.hostmaster.clone()));
    m.insert("serial".into(), Value::Float(s.serial as f64));
    m.insert("refresh".into(), Value::Float(s.refresh as f64));
    m.insert("retry".into(), Value::Float(s.retry as f64));
    m.insert("expire".into(), Value::Float(s.expire as f64));
    m.insert("minttl".into(), Value::Float(s.minttl as f64));
    h.new_object(m)
}

fn naptr_obj(h: &mut crate::host::JsHost, n: &NaptrRec) -> Value {
    let mut m = IndexMap::new();
    m.insert("flags".into(), h.new_str(n.flags.clone()));
    m.insert("service".into(), h.new_str(n.service.clone()));
    m.insert("regexp".into(), h.new_str(n.regexp.clone()));
    m.insert("replacement".into(), h.new_str(n.replacement.clone()));
    m.insert("order".into(), Value::Float(n.order as f64));
    m.insert("preference".into(), Value::Float(n.preference as f64));
    h.new_object(m)
}

fn caa_obj(h: &mut crate::host::JsHost, c: &CaaRec) -> Value {
    let mut m = IndexMap::new();
    m.insert("critical".into(), Value::Float(c.critical as f64));
    m.insert("type".into(), h.new_str("CAA"));
    m.insert(c.tag.clone(), h.new_str(c.value.clone()));
    h.new_object(m)
}

/// Build one `resolveAny` record object (`{ type, … }`) per Node's shapes.
fn any_obj(h: &mut crate::host::JsHost, rec: AnyRec) -> Value {
    let mut m = IndexMap::new();
    match rec {
        AnyRec::A(addr, ttl) => {
            m.insert("type".into(), h.new_str("A"));
            m.insert("address".into(), h.new_str(addr));
            m.insert("ttl".into(), Value::Float(ttl as f64));
        }
        AnyRec::Aaaa(addr, ttl) => {
            m.insert("type".into(), h.new_str("AAAA"));
            m.insert("address".into(), h.new_str(addr));
            m.insert("ttl".into(), Value::Float(ttl as f64));
        }
        AnyRec::Cname(v) => {
            m.insert("type".into(), h.new_str("CNAME"));
            m.insert("value".into(), h.new_str(v));
        }
        AnyRec::Mx(pri, ex) => {
            m.insert("type".into(), h.new_str("MX"));
            m.insert("exchange".into(), h.new_str(ex));
            m.insert("priority".into(), Value::Float(pri as f64));
        }
        AnyRec::Ns(v) => {
            m.insert("type".into(), h.new_str("NS"));
            m.insert("value".into(), h.new_str(v));
        }
        AnyRec::Ptr(v) => {
            m.insert("type".into(), h.new_str("PTR"));
            m.insert("value".into(), h.new_str(v));
        }
        AnyRec::Txt(chunks) => {
            let entries: Vec<Value> = chunks.into_iter().map(|s| h.new_str(s)).collect();
            let arr = h.new_array(entries);
            m.insert("type".into(), h.new_str("TXT"));
            m.insert("entries".into(), arr);
        }
        AnyRec::Srv(s) => {
            m.insert("type".into(), h.new_str("SRV"));
            m.insert("priority".into(), Value::Float(s.priority as f64));
            m.insert("weight".into(), Value::Float(s.weight as f64));
            m.insert("port".into(), Value::Float(s.port as f64));
            m.insert("name".into(), h.new_str(s.name));
        }
        AnyRec::Soa(s) => return soa_with_type(h, &s),
        AnyRec::Naptr(n) => {
            m.insert("type".into(), h.new_str("NAPTR"));
            m.insert("flags".into(), h.new_str(n.flags));
            m.insert("service".into(), h.new_str(n.service));
            m.insert("regexp".into(), h.new_str(n.regexp));
            m.insert("replacement".into(), h.new_str(n.replacement));
            m.insert("order".into(), Value::Float(n.order as f64));
            m.insert("preference".into(), Value::Float(n.preference as f64));
        }
        AnyRec::Caa(c) => {
            m.insert("type".into(), h.new_str("CAA"));
            m.insert("critical".into(), Value::Float(c.critical as f64));
            m.insert(c.tag, h.new_str(c.value));
        }
    }
    h.new_object(m)
}

fn soa_with_type(h: &mut crate::host::JsHost, s: &SoaRec) -> Value {
    let mut m = IndexMap::new();
    m.insert("type".into(), h.new_str("SOA"));
    m.insert("nsname".into(), h.new_str(s.nsname.clone()));
    m.insert("hostmaster".into(), h.new_str(s.hostmaster.clone()));
    m.insert("serial".into(), Value::Float(s.serial as f64));
    m.insert("refresh".into(), Value::Float(s.refresh as f64));
    m.insert("retry".into(), Value::Float(s.retry as f64));
    m.insert("expire".into(), Value::Float(s.expire as f64));
    m.insert("minttl".into(), Value::Float(s.minttl as f64));
    h.new_object(m)
}

// ── resolver construction / error mapping ────────────────────────────────────

/// Build a blocking `Resolver`. With an explicit server list, use those
/// nameservers (UDP/TCP, port 53); otherwise the system configuration.
fn build_resolver(servers: Option<&[String]>) -> Result<Resolver, String> {
    match servers {
        Some(list) if !list.is_empty() => {
            let ips: Vec<IpAddr> = list.iter().filter_map(|s| parse_ip(s)).collect();
            if ips.is_empty() {
                return Err("EBADFAMILY".into());
            }
            let group = NameServerConfigGroup::from_ips_clear(&ips, 53, true);
            let cfg = ResolverConfig::from_parts(None, vec![], group);
            Resolver::new(cfg, ResolverOpts::default()).map_err(|e| e.to_string())
        }
        _ => Resolver::from_system_conf()
            .or_else(|_| Resolver::new(ResolverConfig::default(), ResolverOpts::default()))
            .map_err(|e| e.to_string()),
    }
}

/// Parse a bare IP or an `ip:port` / `[ipv6]:port` string to its `IpAddr`.
fn parse_ip(s: &str) -> Option<IpAddr> {
    s.parse::<IpAddr>()
        .ok()
        .or_else(|| s.parse::<std::net::SocketAddr>().ok().map(|sa| sa.ip()))
}

/// The DNS system nameservers (deduplicated), as IP strings.
fn system_servers() -> Vec<String> {
    match hickory_resolver::system_conf::read_system_conf() {
        Ok((cfg, _)) => {
            let mut out: Vec<String> = Vec::new();
            for ns in cfg.name_servers() {
                let ip = ns.socket_addr.ip().to_string();
                if !out.contains(&ip) {
                    out.push(ip);
                }
            }
            out
        }
        Err(_) => Vec::new(),
    }
}

/// Map a `hickory` resolve error to the Node/c-ares error code string.
fn err_code(e: &ResolveError) -> &'static str {
    match e.kind() {
        ResolveErrorKind::NoRecordsFound { response_code, .. } => {
            if *response_code == ResponseCode::NXDomain {
                "ENOTFOUND"
            } else {
                "ENODATA"
            }
        }
        ResolveErrorKind::NoConnections => "ESERVFAIL",
        ResolveErrorKind::Timeout => "ETIMEOUT",
        _ => "ESERVFAIL",
    }
}

/// Build an `Error` carrying `code`/`syscall`/`hostname`, message
/// `"<syscall> <code> <hostname>"` (c-ares parity).
fn dns_error(code: &str, syscall: &str, host: &str) -> Value {
    let msg = format!("Error: {syscall} {code} {host}");
    with_host(|h| {
        let e = crate::builtins::synth_error(h, &msg);
        let cv = h.new_str(code.to_string());
        let sv = h.new_str(syscall.to_string());
        let hv = h.new_str(host.to_string());
        if let Some(JsObj::Object(p)) = h.get_mut(&e) {
            p.insert("code".into(), cv);
            p.insert("syscall".into(), sv);
            p.insert("hostname".into(), hv);
        }
        e
    })
}

// ── configuration methods ────────────────────────────────────────────────────

fn get_servers() -> Result<Value, String> {
    let list = module_servers().unwrap_or_else(system_servers);
    Ok(with_host(|h| {
        let items: Vec<Value> = list.into_iter().map(|s| h.new_str(s)).collect();
        h.new_array(items)
    }))
}

fn set_servers(args: &[Value]) -> Result<Value, String> {
    let list = servers_from_arg(args.first());
    STATE.with(|s| s.borrow_mut().servers = Some(list));
    Ok(Value::Undef)
}

fn servers_from_arg(v: Option<&Value>) -> Vec<String> {
    with_host(|h| match v.and_then(|v| h.get(v)) {
        Some(JsObj::Array(items)) => items.iter().map(|x| h.str_of(x)).collect(),
        _ => Vec::new(),
    })
}

fn get_default_result_order() -> Value {
    let order = STATE.with(|s| s.borrow().order.clone());
    with_host(|h| h.new_str(order))
}

fn set_default_result_order(args: &[Value]) -> Result<Value, String> {
    let order = arg_str(args, 0);
    if order == "ipv4first" || order == "verbatim" || order == "ipv6first" {
        STATE.with(|s| s.borrow_mut().order = order);
    }
    Ok(Value::Undef)
}

// ── dns.Resolver instance ────────────────────────────────────────────────────

/// `new dns.Resolver()` → callback-style resolver instance.
pub fn construct_resolver(_args: &[Value]) -> Value {
    make_resolver_obj(false)
}

/// `new dns.promises.Resolver()` → Promise-returning resolver instance.
pub fn construct_resolver_promises(_args: &[Value]) -> Value {
    make_resolver_obj(true)
}

fn make_resolver_obj(promises: bool) -> Value {
    let empty = with_host(|h| h.new_array(Vec::new()));
    let tag = with_host(|h| h.new_str("Resolver"));
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), tag);
        m.insert("@@servers".into(), empty);
        if promises {
            m.insert("@@promises".into(), Value::Bool(true));
        }
        h.new_object(m)
    })
}

/// Dispatch an instance method on a `dns.Resolver` receiver.
pub fn resolver_instance_call(
    recv: &Value,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    let promises = recv_promises(recv);
    let servers = recv_servers(recv);

    match method {
        "getServers" => {
            let list = servers.clone().unwrap_or_else(system_servers);
            return Ok(with_host(|h| {
                let items: Vec<Value> = list.into_iter().map(|s| h.new_str(s)).collect();
                h.new_array(items)
            }));
        }
        "setServers" => {
            let list = servers_from_arg(args.first());
            set_recv_servers(recv, &list);
            return Ok(Value::Undef);
        }
        "cancel" | "setLocalAddress" => return Ok(Value::Undef),
        _ => {}
    }

    if method == "resolve" {
        let name = super::arg_str(&args, 0);
        let has_rrtype = if promises {
            args.len() > 1
        } else {
            args.len() > 2
        };
        let rrtype = if has_rrtype {
            super::arg_str(&args, 1)
        } else {
            "A".to_string()
        };
        let q = query_of(&rrtype).unwrap_or(Query::A);
        let cb = if promises { None } else { args.last().cloned() };
        return Ok(run_or_promise(q, name, servers, cb, promises));
    }

    if let Some(q) = query_for_method(method) {
        let name = super::arg_str(&args, 0);
        let cb = if promises { None } else { args.last().cloned() };
        return Ok(run_or_promise(q, name, servers, cb, promises));
    }

    Err(format!("TypeError: resolver.{method} is not a function"))
}

fn query_for_method(method: &str) -> Option<Query> {
    Some(match method {
        "resolve4" => Query::A,
        "resolve6" => Query::Aaaa,
        "resolveMx" => Query::Mx,
        "resolveTxt" => Query::Txt,
        "resolveCname" => Query::Cname,
        "resolveNs" => Query::Ns,
        "resolvePtr" => Query::Ptr,
        "resolveSrv" => Query::Srv,
        "resolveSoa" => Query::Soa,
        "resolveNaptr" => Query::Naptr,
        "resolveCaa" => Query::Caa,
        "resolveTlsa" => Query::Tlsa,
        "resolveAny" => Query::Any,
        "reverse" => Query::Reverse,
        _ => return None,
    })
}

fn recv_promises(recv: &Value) -> bool {
    with_host(|h| {
        matches!(
            h.get(recv),
            Some(JsObj::Object(p)) if matches!(p.get("@@promises"), Some(Value::Bool(true)))
        )
    })
}

fn recv_servers(recv: &Value) -> Option<Vec<String>> {
    with_host(|h| {
        let JsObj::Object(p) = h.get(recv)? else {
            return None;
        };
        let arr = p.get("@@servers")?;
        let JsObj::Array(items) = h.get(arr)? else {
            return None;
        };
        let list: Vec<String> = items.iter().map(|x| h.str_of(x)).collect();
        if list.is_empty() {
            None
        } else {
            Some(list)
        }
    })
}

fn set_recv_servers(recv: &Value, list: &[String]) {
    let arr = with_host(|h| {
        let items: Vec<Value> = list.iter().map(|s| h.new_str(s.clone())).collect();
        h.new_array(items)
    });
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert("@@servers".into(), arr);
        }
    });
}
