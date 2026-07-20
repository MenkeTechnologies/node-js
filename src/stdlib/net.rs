//! Node `net` module: TCP `Server` and `Socket`.
//!
//! Threading model (see `host::run_event_loop`): each listener runs an `accept`
//! loop on its own thread and each accepted connection runs a `read` loop on its
//! own thread. Those threads NEVER touch the JS heap — they only move raw bytes
//! and post `IoTask` closures onto the host channel. Every JS-visible effect
//! (creating the `Socket` object, emitting `connection`/`data`/`end`/`close`,
//! calling listeners) happens on the main thread when the event loop runs the
//! posted closure. All shared Rust-side state (listener/socket records) lives in
//! a main-thread `thread_local`, so it needs no locking of its own; only the
//! write half of each `TcpStream` is shared with... nothing else, but is wrapped
//! for symmetry and future duplex use.

use crate::host::{invoke, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// `net` module functions routed through `stdlib::call`.
pub const MODULE_METHODS: &[&str] = &[
    "createServer",
    "connect",
    "createConnection",
    "isIP",
    "isIPv4",
    "isIPv6",
    "getDefaultAutoSelectFamily",
    "setDefaultAutoSelectFamily",
    "getDefaultAutoSelectFamilyAttemptTimeout",
    "setDefaultAutoSelectFamilyAttemptTimeout",
];

/// Instance methods of a `net.BlockList` (parent wires the `BlockList` tag to
/// `block_list_call` via `native_tag`/`instance_call`).
pub const BLOCKLIST_METHODS: &[&str] =
    &["addAddress", "addRange", "addSubnet", "check"];

/// A native-thread hook run on the main thread for each new connection. Set by
/// `http` to attach its request parser; `None` for a plain `net` server (which
/// just emits `connection` and calls its JS `connectionListener`).
type ConnHook = std::rc::Rc<dyn Fn(&Value, &Value) -> Result<(), String>>;

/// Main-thread record for a listening server.
struct ServerRec {
    /// The JS server object (a native emitter).
    emitter: Value,
    /// Set by `close` to stop the `accept` loop.
    stop: Arc<AtomicBool>,
    /// `http`'s per-connection setup hook (if this is an http server).
    conn_hook: Option<ConnHook>,
}

/// Main-thread record for a live connection.
struct SocketRec {
    /// The JS socket object (a native emitter).
    emitter: Value,
    /// Write half of the TCP stream (shared for future duplex; only the main
    /// thread writes to it today).
    write: Arc<Mutex<TcpStream>>,
}

#[derive(Default)]
struct NetState {
    next_id: u64,
    servers: HashMap<u64, ServerRec>,
    sockets: HashMap<u64, SocketRec>,
}

thread_local! {
    static NET: std::cell::RefCell<NetState> = std::cell::RefCell::new(NetState::default());
}

fn next_id() -> u64 {
    NET.with(|s| {
        let mut s = s.borrow_mut();
        s.next_id += 1;
        s.next_id
    })
}

// ── object construction ──────────────────────────────────────────────────────

/// Build a native emitter object (`@@native` tag + `@@on`/`@@once` listener maps)
/// carrying the given extra props. Shared shape with `events::new_emitter` so the
/// EventEmitter methods (`on`/`once`/`emit`/…) work verbatim.
pub fn new_emitter_object(tag: &str, mut extra: IndexMap<String, Value>) -> Value {
    with_host(|h| {
        let on = h.new_object(IndexMap::new());
        let once = h.new_object(IndexMap::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        m.insert("@@on".into(), on);
        m.insert("@@once".into(), once);
        for (k, v) in extra.drain(..) {
            m.insert(k, v);
        }
        h.new_object(m)
    })
}

fn get_prop(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

fn set_prop(recv: &Value, key: &str, val: Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), val);
        }
    });
}

fn u64_prop(recv: &Value, key: &str) -> Option<u64> {
    get_prop(recv, key).map(|v| with_host(|h| h.to_number(&v)) as u64)
}

/// Delegate the EventEmitter methods (`on`/`once`/`emit`/…) to `events`; returns
/// `None` for a non-emitter method so the caller can handle it.
fn emitter_dispatch(recv: &Value, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "on" | "addListener" | "prependListener" | "once" | "prependOnceListener" | "emit"
        | "removeListener" | "off" | "removeAllListeners" | "listenerCount" | "eventNames" => {
            Some(super::events::instance_call(recv, method, args.to_vec()))
        }
        _ => None,
    }
}

// ── module: net.createServer ─────────────────────────────────────────────────

/// `stdlib::call` entry for `net.<method>`.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "createServer" => Some(Ok(create_server(args.first().cloned()))),
        "connect" | "createConnection" => Some(Ok(connect(args))),
        "isIP" => Some(Ok(Value::Float(is_ip(&arg_string(args, 0)) as f64))),
        "isIPv4" => Some(Ok(Value::Bool(is_ip(&arg_string(args, 0)) == 4))),
        "isIPv6" => Some(Ok(Value::Bool(is_ip(&arg_string(args, 0)) == 6))),
        "getDefaultAutoSelectFamily" => {
            Some(Ok(Value::Bool(AUTO_SELECT_FAMILY.with(|c| c.get()))))
        }
        "setDefaultAutoSelectFamily" => {
            let v = args.first().map(|a| with_host(|h| h.truthy(a))).unwrap_or(false);
            AUTO_SELECT_FAMILY.with(|c| c.set(v));
            Some(Ok(Value::Undef))
        }
        "getDefaultAutoSelectFamilyAttemptTimeout" => {
            Some(Ok(Value::Float(AUTO_SELECT_TIMEOUT.with(|c| c.get()))))
        }
        "setDefaultAutoSelectFamilyAttemptTimeout" => {
            let v = args.first().map(|a| with_host(|h| h.to_number(a))).unwrap_or(f64::NAN);
            if v.is_finite() && v >= 1.0 {
                AUTO_SELECT_TIMEOUT.with(|c| c.set(v));
            }
            Some(Ok(Value::Undef))
        }
        _ => None,
    }
}

thread_local! {
    /// `net` module default for `autoSelectFamily` (best-effort; not consulted by
    /// our connect path, which is single-address).
    static AUTO_SELECT_FAMILY: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// `net` module default for `autoSelectFamilyAttemptTimeout` (ms).
    static AUTO_SELECT_TIMEOUT: std::cell::Cell<f64> = const { std::cell::Cell::new(250.0) };
}

/// String value of `args[i]` (empty string if absent/undefined).
fn arg_string(args: &[Value], i: usize) -> String {
    match args.get(i) {
        Some(v) if !matches!(v, Value::Undef) => with_host(|h| h.str_of(v)),
        _ => String::new(),
    }
}

/// Node `net.isIP(input)` → `4`, `6`, or `0`. Pure parse (no DNS).
fn is_ip(input: &str) -> i32 {
    use std::net::{Ipv4Addr, Ipv6Addr};
    if input.parse::<Ipv4Addr>().is_ok() {
        4
    } else if input.parse::<Ipv6Addr>().is_ok() {
        6
    } else {
        0
    }
}

/// Non-function `net` namespace members: the class constructors, exposed as
/// builtin ctor namespaces so `.prototype` resolves and `new net.X(...)` routes
/// through `stdlib::construct` → `net::construct`. `Stream` is a legacy alias of
/// `Socket`. Reachable via `namespace_property` → `stdlib::constant` once the
/// parent adds a `"net" => net::constant(name)` arm.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "Server" | "Socket" | "Stream" | "SocketAddress" | "BlockList" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin(name.into()))))
        }
        _ => None,
    }
}

/// Build a `net.Server`. An optional `connectionListener` is stored on the object
/// and invoked (plus `connection` emitted) for every accepted socket.
pub fn create_server(connection_listener: Option<Value>) -> Value {
    let mut extra = IndexMap::new();
    if let Some(cb) = connection_listener.filter(|v| !matches!(v, Value::Undef)) {
        extra.insert("@@connListener".into(), cb);
    }
    new_emitter_object("Server", extra)
}

// ── client: net.connect / net.createConnection / new net.Socket ──────────────

/// Build a bare `net.Socket` (a native emitter) with an id but no live stream, as
/// produced by `new net.Socket()`. `connect` is called separately.
pub fn new_socket() -> Value {
    let sock_id = next_id();
    let mut extra = IndexMap::new();
    extra.insert("@@netid".into(), Value::Float(sock_id as f64));
    extra.insert("connecting".into(), Value::Bool(false));
    new_emitter_object("Socket", extra)
}

/// `net.connect(...)` / `net.createConnection(...)`: build a `Socket` and start
/// the connection immediately. Returns the socket synchronously; the `connect`
/// event fires on the main thread once the TCP handshake completes.
pub fn connect(args: &[Value]) -> Value {
    let socket = new_socket();
    socket_connect(&socket, args);
    socket
}

/// Parse the `(port[, host])` / `(options)` argument shapes of `connect`,
/// returning `(port, host, connectListener)`.
fn parse_connect_args(args: &[Value]) -> (u16, String, Option<Value>) {
    let mut port: u16 = 0;
    let mut host = "localhost".to_string();
    let mut cb: Option<Value> = None;
    for a in args {
        if with_host(|h| crate::host::is_callable(h, a)) {
            cb = Some(a.clone());
        } else if with_host(|h| h.as_str(a)).is_some() {
            host = with_host(|h| h.str_of(a));
        } else if matches!(a, Value::Obj(_)) {
            if let Some(v) = get_prop(a, "port") {
                let n = with_host(|h| h.to_number(&v));
                if !n.is_nan() {
                    port = n as u16;
                }
            }
            for key in ["host", "hostname"] {
                if let Some(v) = get_prop(a, key).filter(|v| with_host(|h| h.as_str(v)).is_some()) {
                    host = with_host(|h| h.str_of(&v));
                }
            }
        } else {
            let n = with_host(|h| h.to_number(a));
            if !n.is_nan() {
                port = n as u16;
            }
        }
    }
    (port, host, cb)
}

/// Drive `socket.connect(...)`: register the optional `connectListener`, then
/// spawn the blocking `TcpStream::connect` on a background thread. On success the
/// posted `IoTask` (`on_connect`) builds the reader; on failure it emits `error`.
fn socket_connect(socket: &Value, args: &[Value]) {
    let (port, host, cb) = parse_connect_args(args);
    if let Some(cb) = cb {
        let _ = super::events::instance_call(
            socket,
            "on",
            vec![with_host(|h| h.new_str("connect")), cb],
        );
    }
    let sock_id = u64_prop(socket, "@@netid").unwrap_or_else(next_id);
    set_prop(socket, "@@netid", Value::Float(sock_id as f64));
    set_prop(socket, "connecting", Value::Bool(true));
    with_host(|h| h.incr_handle());

    let tx = with_host(|h| h.io_sender());
    let socket_val = socket.clone();
    std::thread::spawn(move || match TcpStream::connect((host.as_str(), port)) {
        Ok(stream) => {
            let _ = tx.send(Box::new(move || on_connect(sock_id, socket_val, stream)));
        }
        Err(e) => {
            let msg = format!("connect ECONNREFUSED {host}:{port}: {e}");
            let _ = tx.send(Box::new(move || on_connect_error(socket_val, msg)));
        }
    });
}

/// Main-thread completion of a successful client connect: register the socket,
/// spawn its reader (same loop the server side uses), then emit `connect`.
fn on_connect(sock_id: u64, socket: Value, stream: TcpStream) -> Result<(), String> {
    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => {
            with_host(|h| h.decr_handle());
            return Ok(());
        }
    };
    let write = Arc::new(Mutex::new(stream));
    NET.with(|s| {
        s.borrow_mut().sockets.insert(sock_id, SocketRec { emitter: socket.clone(), write });
    });
    set_prop(&socket, "connecting", Value::Bool(false));
    // The reader gets its own handle registration via `on_socket_close`'s
    // `decr_handle`; the `incr` from `socket_connect` covers this socket's life.
    let tx = with_host(|h| h.io_sender());
    std::thread::spawn(move || reader_loop(read_stream, sock_id, tx));
    super::events::instance_call(&socket, "emit", vec![with_host(|h| h.new_str("connect"))])?;
    Ok(())
}

/// Main-thread completion of a failed client connect: release the handle and emit
/// `error` (Node emits an `Error` with `code: 'ECONNREFUSED'`).
fn on_connect_error(socket: Value, msg: String) -> Result<(), String> {
    with_host(|h| h.decr_handle());
    let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
    let err = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("message".into(), h.new_str(msg.clone()));
        m.insert("code".into(), h.new_str("ECONNREFUSED"));
        h.new_object(m)
    });
    super::events::instance_call(&socket, "emit", vec![with_host(|h| h.new_str("error")), err])?;
    Ok(())
}

// ── constructors: new net.Socket() / new net.Server() / SocketAddress / BlockList

/// `stdlib::construct` entry for `net` classes. Parent wires this into
/// `stdlib::construct`.
pub fn construct(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match name {
        "Socket" | "Stream" => Some(Ok(new_socket())),
        "Server" => Some(Ok(create_server(
            args.first().cloned().filter(|v| with_host(|h| crate::host::is_callable(h, v))),
        ))),
        "SocketAddress" => Some(Ok(socket_address(args))),
        "BlockList" => Some(Ok(new_block_list())),
        _ => None,
    }
}

/// `new net.SocketAddress({ address, port, family, flowlabel })` — a plain data
/// holder (Node exposes the fields as getters; a data object reads identically).
fn socket_address(args: &[Value]) -> Value {
    let opts = args.first().cloned().unwrap_or(Value::Undef);
    let mut address = String::new();
    let mut family = "ipv4".to_string();
    let mut have_family = false;
    let mut port = 0f64;
    let mut flowlabel = 0f64;
    if matches!(opts, Value::Obj(_)) {
        if let Some(v) = get_prop(&opts, "address").filter(|v| with_host(|h| h.as_str(v)).is_some()) {
            address = with_host(|h| h.str_of(&v));
        }
        if let Some(v) = get_prop(&opts, "family").filter(|v| with_host(|h| h.as_str(v)).is_some()) {
            family = with_host(|h| h.str_of(&v)).to_ascii_lowercase();
            have_family = true;
        }
        if let Some(v) = get_prop(&opts, "port") {
            let n = with_host(|h| h.to_number(&v));
            if !n.is_nan() {
                port = n;
            }
        }
        if let Some(v) = get_prop(&opts, "flowlabel") {
            let n = with_host(|h| h.to_number(&v));
            if !n.is_nan() {
                flowlabel = n;
            }
        }
    }
    if !have_family {
        family = if is_ip(&address) == 6 { "ipv6" } else { "ipv4" }.to_string();
    }
    if address.is_empty() {
        address = if family == "ipv6" { "::" } else { "127.0.0.1" }.to_string();
    }
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("SocketAddress"));
        m.insert("address".into(), h.new_str(address));
        m.insert("port".into(), Value::Float(port));
        m.insert("family".into(), h.new_str(family));
        m.insert("flowlabel".into(), Value::Float(flowlabel));
        h.new_object(m)
    })
}

// ── BlockList ────────────────────────────────────────────────────────────────

/// One `BlockList` rule. All comparisons happen in the integer domain (`u128`
/// covers both families; IPv4 is mapped into the low 32 bits).
enum BlockRule {
    /// Single address (family-tagged).
    Addr { v6: bool, val: u128 },
    /// Inclusive `[start, end]` range (family-tagged).
    Range { v6: bool, start: u128, end: u128 },
    /// CIDR subnet: `network`/`prefix` (family-tagged).
    Subnet { v6: bool, network: u128, prefix: u32 },
}

thread_local! {
    static BLOCK_LISTS: std::cell::RefCell<HashMap<u64, Vec<BlockRule>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// `new net.BlockList()` — a `@@native`-tagged holder whose rules live in the
/// main-thread `BLOCK_LISTS` registry keyed by `@@blid`.
fn new_block_list() -> Value {
    let id = next_id();
    BLOCK_LISTS.with(|b| {
        b.borrow_mut().insert(id, Vec::new());
    });
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("BlockList"));
        m.insert("@@blid".into(), Value::Float(id as f64));
        h.new_object(m)
    })
}

/// Parse an IP string into `(is_ipv6, u128)`. IPv4 lands in the low 32 bits.
fn ip_to_u128(s: &str) -> Option<(bool, u128)> {
    use std::net::{Ipv4Addr, Ipv6Addr};
    if let Ok(v4) = s.parse::<Ipv4Addr>() {
        return Some((false, u32::from(v4) as u128));
    }
    if let Ok(v6) = s.parse::<Ipv6Addr>() {
        return Some((true, u128::from(v6)));
    }
    None
}

/// `BlockList` instance dispatch (parent routes the `BlockList` tag here).
pub fn block_list_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    let Some(id) = u64_prop(recv, "@@blid") else {
        return Err(crate::host::type_error("invalid BlockList"));
    };
    match method {
        "addAddress" => {
            let addr = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            if let Some((v6, val)) = ip_to_u128(&addr) {
                BLOCK_LISTS.with(|b| {
                    if let Some(rules) = b.borrow_mut().get_mut(&id) {
                        rules.push(BlockRule::Addr { v6, val });
                    }
                });
            }
            Ok(Value::Undef)
        }
        "addRange" => {
            let start = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            let end = with_host(|h| h.str_of(&args.get(1).cloned().unwrap_or(Value::Undef)));
            if let (Some((v6, s)), Some((_, e))) = (ip_to_u128(&start), ip_to_u128(&end)) {
                BLOCK_LISTS.with(|b| {
                    if let Some(rules) = b.borrow_mut().get_mut(&id) {
                        rules.push(BlockRule::Range { v6, start: s.min(e), end: s.max(e) });
                    }
                });
            }
            Ok(Value::Undef)
        }
        "addSubnet" => {
            let net = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            let prefix = with_host(|h| h.to_number(&args.get(1).cloned().unwrap_or(Value::Undef))) as u32;
            if let Some((v6, network)) = ip_to_u128(&net) {
                BLOCK_LISTS.with(|b| {
                    if let Some(rules) = b.borrow_mut().get_mut(&id) {
                        rules.push(BlockRule::Subnet { v6, network, prefix });
                    }
                });
            }
            Ok(Value::Undef)
        }
        "check" => {
            let addr = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            let Some((v6, val)) = ip_to_u128(&addr) else { return Ok(Value::Bool(false)) };
            let blocked = BLOCK_LISTS.with(|b| {
                b.borrow().get(&id).map(|rules| rules.iter().any(|r| rule_matches(r, v6, val))).unwrap_or(false)
            });
            Ok(Value::Bool(blocked))
        }
        _ => Err(crate::host::type_error(&format!("blocklist.{method} is not a function"))),
    }
}

/// Whether a query address (`v6`/`val`) is covered by a rule (family must match).
fn rule_matches(rule: &BlockRule, q_v6: bool, q_val: u128) -> bool {
    match rule {
        BlockRule::Addr { v6, val } => *v6 == q_v6 && *val == q_val,
        BlockRule::Range { v6, start, end } => *v6 == q_v6 && q_val >= *start && q_val <= *end,
        BlockRule::Subnet { v6, network, prefix } => {
            if *v6 != q_v6 {
                return false;
            }
            let bits = if q_v6 { 128 } else { 32 };
            let p = (*prefix).min(bits);
            if p == 0 {
                return true;
            }
            let shift = bits - p;
            (q_val >> shift) == (*network >> shift)
        }
    }
}

/// Attach an `http`-style per-connection hook to a server object (called by
/// `http::create_server`). Stored in the main-thread registry, keyed by the
/// server's assigned id once it starts listening — so we stash it on the object
/// until `listen` registers the record.
pub fn set_conn_hook(server: &Value, hook: ConnHook) {
    // Marker so `listen` knows to move the hook into the `ServerRec`.
    set_prop(server, "@@httpMode", Value::Bool(true));
    PENDING_HOOKS.with(|p| p.borrow_mut().push((server.clone(), hook)));
}

thread_local! {
    /// Hooks registered before `listen` assigns a server id.
    static PENDING_HOOKS: std::cell::RefCell<Vec<(Value, ConnHook)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn take_pending_hook(server: &Value) -> Option<ConnHook> {
    PENDING_HOOKS.with(|p| {
        let mut p = p.borrow_mut();
        p.iter().position(|(s, _)| s == server).map(|pos| p.remove(pos).1)
    })
}

// ── instance methods (Server / Socket) ───────────────────────────────────────

pub fn instance_call(tag: &str, recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match tag {
        "Server" => server_call(recv, method, args),
        "Socket" => socket_call(recv, method, args),
        "BlockList" => block_list_call(recv, method, args),
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

fn server_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if let Some(r) = emitter_dispatch(recv, method, &args) {
        return r;
    }
    match method {
        "listen" => server_listen(recv, &args),
        "close" => server_close(recv, &args),
        "address" => Ok(get_prop(recv, "@@address").unwrap_or(Value::Undef)),
        _ => Err(crate::host::type_error(&format!("server.{method} is not a function"))),
    }
}

/// `server.listen(port[, host][, callback])`. Binds on the main thread (so bind
/// errors surface synchronously), then spawns the `accept` loop thread. The
/// `listening` event + callback fire asynchronously via a posted `IoTask`.
fn server_listen(recv: &Value, args: &[Value]) -> Result<Value, String> {
    // Argument shapes: (port), (port, cb), (port, host), (port, host, cb).
    let port = with_host(|h| args.first().map(|v| h.to_number(v)).unwrap_or(0.0)) as u16;
    let mut host = "0.0.0.0".to_string();
    let mut cb: Option<Value> = None;
    for a in &args[1.min(args.len())..] {
        if with_host(|h| h.as_str(a)).is_some() {
            host = with_host(|h| h.str_of(a));
        } else if with_host(|h| crate::host::is_callable(h, a)) {
            cb = Some(a.clone());
        }
    }

    let listener = TcpListener::bind((host.as_str(), port))
        .map_err(|e| format!("Error: listen EADDRINUSE: {e}"))?;
    let local = listener.local_addr().ok();

    // Assign an id and register the server as a live handle.
    let id = next_id();
    set_prop(recv, "@@netid", Value::Float(id as f64));
    if let Some(addr) = local {
        let mut a = IndexMap::new();
        a.insert("port".into(), Value::Float(addr.port() as f64));
        a.insert("address".into(), with_host(|h| h.new_str(addr.ip().to_string())));
        a.insert("family".into(), with_host(|h| h.new_str(if addr.is_ipv6() { "IPv6" } else { "IPv4" })));
        let addr_obj = with_host(|h| h.new_object(a));
        set_prop(recv, "@@address", addr_obj);
    }
    let conn_hook = take_pending_hook(recv);
    let stop = Arc::new(AtomicBool::new(false));
    NET.with(|s| {
        s.borrow_mut().servers.insert(
            id,
            ServerRec { emitter: recv.clone(), stop: stop.clone(), conn_hook },
        );
    });
    with_host(|h| h.incr_handle());

    // Spawn the accept loop. Non-blocking + short poll so `close` can stop it.
    let tx = with_host(|h| h.io_sender());
    listener.set_nonblocking(true).ok();
    std::thread::spawn(move || {
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }
            match listener.accept() {
                Ok((stream, _addr)) => {
                    let tx2 = tx.clone();
                    let _ = tx.send(Box::new(move || on_connection(id, stream, tx2)));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

    // Fire `listening` + callback asynchronously on the main thread.
    let server = recv.clone();
    let _ = with_host(|h| h.io_sender()).send(Box::new(move || {
        super::events::instance_call(&server, "emit", vec![with_host(|h| h.new_str("listening"))])?;
        if let Some(cb) = cb {
            invoke(&cb, Vec::new(), None)?;
        }
        Ok(())
    }));
    Ok(recv.clone())
}

/// `server.close([cb])`: stop accepting, drop the handle, wake the loop.
fn server_close(recv: &Value, args: &[Value]) -> Result<Value, String> {
    if let Some(id) = u64_prop(recv, "@@netid") {
        let rec = NET.with(|s| s.borrow_mut().servers.remove(&id));
        if let Some(rec) = rec {
            rec.stop.store(true, Ordering::Release);
            with_host(|h| h.decr_handle());
            // Wake the blocking loop so it can re-evaluate `open_handles`.
            let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
        }
    }
    // `close` callback registers as a one-shot `close` listener in Node.
    if let Some(cb) = args.first().filter(|v| with_host(|h| crate::host::is_callable(h, v))) {
        invoke(cb, Vec::new(), None)?;
    }
    super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("close"))])?;
    Ok(recv.clone())
}

fn socket_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if let Some(r) = emitter_dispatch(recv, method, &args) {
        return r;
    }
    match method {
        "write" => {
            if let Some(id) = u64_prop(recv, "@@netid") {
                socket_write_id(id, &value_bytes(args.first()));
            }
            Ok(Value::Bool(true))
        }
        "end" => {
            if let Some(id) = u64_prop(recv, "@@netid") {
                if let Some(chunk) = args.first().filter(|v| !matches!(v, Value::Undef)) {
                    socket_write_id(id, &value_bytes(Some(chunk)));
                }
                socket_shutdown(id);
            }
            Ok(recv.clone())
        }
        "destroy" => {
            if let Some(id) = u64_prop(recv, "@@netid") {
                socket_shutdown(id);
            }
            Ok(recv.clone())
        }
        "connect" => {
            socket_connect(recv, &args);
            Ok(recv.clone())
        }
        "address" => Ok(get_prop(recv, "@@address").unwrap_or(Value::Undef)),
        "setEncoding" | "setTimeout" | "setNoDelay" | "setKeepAlive" | "ref" | "unref" | "pause" | "resume" => {
            // Accepted no-ops for M1 (curl needs none of these).
            Ok(recv.clone())
        }
        _ => Err(crate::host::type_error(&format!("socket.{method} is not a function"))),
    }
}

/// Raw bytes of a `write`/`end` argument: a Buffer's bytes, or a string's UTF-8.
fn value_bytes(v: Option<&Value>) -> Vec<u8> {
    let Some(v) = v else { return Vec::new() };
    // Buffer instance: read its `@@bytes` array.
    let is_buffer = with_host(|h| matches!(h.get(v), Some(JsObj::Object(p)) if p.contains_key("@@bytes")));
    if is_buffer {
        return with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|b| h.get(b)) {
                Some(JsObj::Array(items)) => items.iter().map(|x| h.to_number(x) as u8).collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        });
    }
    with_host(|h| h.str_of(v)).into_bytes()
}

// ── main-thread I/O dispatch (run from posted IoTasks) ────────────────────────

/// A newly accepted connection: build the `Socket`, register it, spawn its
/// reader, then emit `connection` and run the server's listener/hook. Runs on the
/// main thread.
fn on_connection(server_id: u64, stream: TcpStream, tx: std::sync::mpsc::Sender<crate::host::IoTask>) -> Result<(), String> {
    // Server gone (closed before this event drained): drop the connection.
    let server = NET.with(|s| s.borrow().servers.get(&server_id).map(|r| r.emitter.clone()));
    let Some(server) = server else { return Ok(()) };

    // Reader gets an independent handle; writes go through the original stream.
    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    let write = Arc::new(Mutex::new(stream));

    let sock_id = next_id();
    let mut extra = IndexMap::new();
    extra.insert("@@netid".into(), Value::Float(sock_id as f64));
    let socket = new_emitter_object("Socket", extra);
    NET.with(|s| {
        s.borrow_mut().sockets.insert(sock_id, SocketRec { emitter: socket.clone(), write });
    });
    with_host(|h| h.incr_handle());

    // Reader thread: raw bytes → posted IoTasks. Never touches the host.
    std::thread::spawn(move || reader_loop(read_stream, sock_id, tx));

    // Emit `connection` + run the server's connection handling.
    super::events::instance_call(&server, "emit", vec![with_host(|_h| socket.clone())])?;
    let hook = NET.with(|s| s.borrow().servers.get(&server_id).and_then(|r| r.conn_hook.clone()));
    if let Some(hook) = hook {
        hook(&server, &socket)?;
    } else if let Some(cb) = get_prop(&server, "@@connListener") {
        invoke(&cb, vec![socket.clone()], None)?;
    }
    Ok(())
}

/// Background reader: blocking `read` loop posting `data`/`end`/`close` events.
fn reader_loop(mut stream: TcpStream, sock_id: u64, tx: std::sync::mpsc::Sender<crate::host::IoTask>) {
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => {
                let _ = tx.send(Box::new(move || on_socket_end(sock_id)));
                break;
            }
            Ok(n) => {
                let bytes = buf[..n].to_vec();
                let _ = tx.send(Box::new(move || on_socket_data(sock_id, bytes)));
            }
            Err(_) => {
                let _ = tx.send(Box::new(move || on_socket_close(sock_id)));
                break;
            }
        }
    }
}

fn on_socket_data(sock_id: u64, bytes: Vec<u8>) -> Result<(), String> {
    let socket = NET.with(|s| s.borrow().sockets.get(&sock_id).map(|r| r.emitter.clone()));
    let Some(socket) = socket else { return Ok(()) };
    // Feed the http parser first (if this socket is an http connection).
    super::http::feed(sock_id, &socket, &bytes)?;
    // Then emit `data` to any JS listeners (as a Buffer, like Node).
    let chunk = super::buffer::from_bytes(&bytes);
    super::events::instance_call(&socket, "emit", vec![with_host(|h| h.new_str("data")), chunk])?;
    Ok(())
}

fn on_socket_end(sock_id: u64) -> Result<(), String> {
    let socket = NET.with(|s| s.borrow().sockets.get(&sock_id).map(|r| r.emitter.clone()));
    if let Some(socket) = socket {
        super::events::instance_call(&socket, "emit", vec![with_host(|h| h.new_str("end"))])?;
    }
    on_socket_close(sock_id)
}

fn on_socket_close(sock_id: u64) -> Result<(), String> {
    let rec = NET.with(|s| s.borrow_mut().sockets.remove(&sock_id));
    super::http::drop_conn(sock_id);
    if let Some(rec) = rec {
        super::events::instance_call(&rec.emitter, "emit", vec![with_host(|h| h.new_str("close"))])?;
        with_host(|h| h.decr_handle());
        // Wake the loop so a closed last handle lets it exit.
        let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
    }
    Ok(())
}

// ── writes (used by http::ServerResponse and net Socket) ──────────────────────

/// Write raw bytes to a live socket by id (no-op if it has closed).
pub fn socket_write_id(sock_id: u64, data: &[u8]) {
    let write = NET.with(|s| s.borrow().sockets.get(&sock_id).map(|r| r.write.clone()));
    if let Some(write) = write {
        if let Ok(mut stream) = write.lock() {
            let _ = stream.write_all(data);
            let _ = stream.flush();
        }
    }
}

/// Shut down the write half of a socket (`socket.end()`), signaling EOF to peer.
fn socket_shutdown(sock_id: u64) {
    let write = NET.with(|s| s.borrow().sockets.get(&sock_id).map(|r| r.write.clone()));
    if let Some(write) = write {
        if let Ok(stream) = write.lock() {
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
    }
}
