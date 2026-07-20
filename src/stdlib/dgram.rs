//! Node `dgram` module: real UDP sockets over `std::net::UdpSocket`.
//!
//! Threading model mirrors `net` (see `host::run_event_loop`): each bound socket
//! runs a `recv_from` loop on its own thread. That thread NEVER touches the JS
//! heap — it only moves raw datagram bytes and posts `IoTask` closures onto the
//! host channel. Every JS-visible effect (emitting `message`/`listening`/`close`,
//! running callbacks) happens on the main thread when the event loop runs the
//! posted closure. The `UdpSocket` is shared with the recv thread through an
//! `Arc` (both `recv_from` and `send_to` take `&self`). All main-thread records
//! live in a `thread_local`, so they need no locking of their own.

use crate::host::{invoke, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// `dgram` module functions routed through `stdlib::call`.
pub const MODULE_METHODS: &[&str] = &["createSocket"];

/// The `dgram.Socket` `@@native` tag.
pub const SOCKET_TAG: &str = "UdpSocket";

/// Instance methods on a `dgram.Socket` (for `instance_has_method` / dispatch).
pub const SOCKET_METHODS: &[&str] = &[
    "bind",
    "send",
    "close",
    "address",
    "setBroadcast",
    "setTTL",
    "setMulticastTTL",
    "setMulticastLoopback",
    "setMulticastInterface",
    "addMembership",
    "dropMembership",
    "addSourceSpecificMembership",
    "dropSourceSpecificMembership",
    "setRecvBufferSize",
    "setSendBufferSize",
    "getRecvBufferSize",
    "getSendBufferSize",
    "connect",
    "disconnect",
    "remoteAddress",
    "ref",
    "unref",
];

/// How long a `recv_from` blocks before the loop re-checks its stop flag.
const POLL: Duration = Duration::from_millis(200);

/// Main-thread record for a bound socket.
struct UdpRec {
    /// The JS socket object (a native emitter).
    emitter: Value,
    /// The live UDP socket, shared with the recv thread.
    socket: Arc<UdpSocket>,
    /// Set by `close` to stop the `recv_from` loop.
    stop: Arc<AtomicBool>,
}

#[derive(Default)]
struct DgramState {
    next_id: u64,
    sockets: HashMap<u64, UdpRec>,
}

thread_local! {
    static DGRAM: std::cell::RefCell<DgramState> = std::cell::RefCell::new(DgramState::default());
}

fn next_id() -> u64 {
    DGRAM.with(|s| {
        let mut s = s.borrow_mut();
        s.next_id += 1;
        s.next_id
    })
}

// ── object helpers ────────────────────────────────────────────────────────────

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

fn is_udp6(recv: &Value) -> bool {
    get_prop(recv, "@@udptype").map(|v| with_host(|h| h.str_of(&v))).as_deref() == Some("udp6")
}

/// The default bind/target host for this socket's address family.
fn default_bind_host(recv: &Value) -> &'static str {
    if is_udp6(recv) {
        "::"
    } else {
        "0.0.0.0"
    }
}

fn default_send_host(recv: &Value) -> &'static str {
    if is_udp6(recv) {
        "::1"
    } else {
        "127.0.0.1"
    }
}

fn is_num(v: &Value) -> bool {
    matches!(v, Value::Float(_) | Value::Int(_))
}

fn is_str(v: &Value) -> bool {
    matches!(v, Value::Str(_)) || with_host(|h| matches!(h.get(v), Some(JsObj::Str(_))))
}

/// Raw bytes of a `send` message argument: a Buffer's `@@bytes`, else a string's
/// UTF-8 (mirrors `net::value_bytes`).
fn value_bytes(v: &Value) -> Vec<u8> {
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

/// Delegate the EventEmitter methods to `events`; `None` for a non-emitter method.
fn emitter_dispatch(recv: &Value, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "on" | "addListener" | "prependListener" | "once" | "prependOnceListener" | "emit"
        | "removeListener" | "off" | "removeAllListeners" | "listeners" | "listenerCount"
        | "eventNames" | "setMaxListeners" | "getMaxListeners" => {
            Some(super::events::instance_call(recv, method, args.to_vec()))
        }
        _ => None,
    }
}

// ── module: dgram.createSocket ────────────────────────────────────────────────

/// `stdlib::call` entry for `dgram.<method>`.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "createSocket" => Some(Ok(create_socket(args))),
        _ => None,
    }
}

/// `dgram.createSocket(type[, callback])` or `dgram.createSocket(options[, callback])`.
/// `type` is `'udp4'`/`'udp6'`; an options object reads its `.type`. A `callback`
/// is registered as a one-time-per-`emit` `'message'` listener (Node semantics).
pub fn create_socket(args: &[Value]) -> Value {
    let first = args.first().cloned().unwrap_or(Value::Undef);
    let sock_type = if is_str(&first) {
        with_host(|h| h.str_of(&first))
    } else {
        // options object: read `.type`.
        with_host(|h| match h.get(&first) {
            Some(JsObj::Object(p)) => p.get("type").map(|v| h.str_of(v)),
            _ => None,
        })
        .unwrap_or_else(|| "udp4".to_string())
    };
    let sock_type = if sock_type == "udp6" { "udp6" } else { "udp4" };

    let mut extra = IndexMap::new();
    extra.insert("@@udptype".into(), with_host(|h| h.new_str(sock_type)));
    let socket = super::net::new_emitter_object(SOCKET_TAG, extra);

    // A trailing callback becomes a `message` listener.
    if let Some(cb) = args.get(1).filter(|v| with_host(|h| crate::host::is_callable(h, v))) {
        let _ = super::events::instance_call(
            &socket,
            "on",
            vec![with_host(|h| h.new_str("message")), cb.clone()],
        );
    }
    socket
}

// ── instance dispatch ─────────────────────────────────────────────────────────

pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if let Some(r) = emitter_dispatch(recv, method, &args) {
        return r;
    }
    match method {
        "bind" => socket_bind(recv, &args),
        "send" => socket_send(recv, &args),
        "close" => socket_close(recv, &args),
        "address" => socket_address(recv),
        // Best-effort socket options: applied to the live `UdpSocket` where std
        // exposes them, otherwise accepted no-ops (multicast/buffer sizing).
        "setBroadcast" => {
            let on = with_host(|h| h.truthy(args.first().unwrap_or(&Value::Undef)));
            if let Some(sock) = live_socket(recv) {
                sock.set_broadcast(on).ok();
            }
            Ok(recv.clone())
        }
        "setTTL" | "setMulticastTTL" => {
            let ttl = with_host(|h| h.to_number(args.first().unwrap_or(&Value::Undef))) as u32;
            if let Some(sock) = live_socket(recv) {
                if method == "setTTL" {
                    sock.set_ttl(ttl).ok();
                } else {
                    sock.set_multicast_ttl_v4(ttl).ok();
                }
            }
            Ok(args.first().cloned().unwrap_or(Value::Undef))
        }
        "getRecvBufferSize" | "getSendBufferSize" => Ok(Value::Float(65536.0)),
        // Accepted no-ops: multicast membership, buffer sizing, connect/disconnect,
        // ref counting. Documented as best-effort — std::net exposes no portable
        // API for most, and the datagram path does not need them.
        "setMulticastLoopback" | "setMulticastInterface" | "addMembership" | "dropMembership"
        | "addSourceSpecificMembership" | "dropSourceSpecificMembership" | "setRecvBufferSize"
        | "setSendBufferSize" | "connect" | "disconnect" | "remoteAddress" | "ref" | "unref" => {
            Ok(recv.clone())
        }
        _ => Err(crate::host::type_error(&format!("socket.{method} is not a function"))),
    }
}

/// The live `UdpSocket` for `recv`, if it is currently bound.
fn live_socket(recv: &Value) -> Option<Arc<UdpSocket>> {
    let id = u64_prop(recv, "@@dgramid")?;
    DGRAM.with(|s| s.borrow().sockets.get(&id).map(|r| r.socket.clone()))
}

// ── bind ──────────────────────────────────────────────────────────────────────

/// `socket.bind([port][, address][, callback])`. Binds on the main thread (so a
/// bind error surfaces from the call), registers the socket as a live handle, and
/// spawns the `recv_from` loop. The `listening` event + callback fire
/// asynchronously via a posted `IoTask`.
fn socket_bind(recv: &Value, args: &[Value]) -> Result<Value, String> {
    // Argument shapes: (), (port), (port, cb), (port, addr), (port, addr, cb),
    // and an options-object first arg `{ port, address }`.
    let mut port: u16 = 0;
    let mut host = default_bind_host(recv).to_string();
    let mut cb: Option<Value> = None;

    if let Some(first) = args.first() {
        if is_num(first) {
            port = with_host(|h| h.to_number(first)) as u16;
        } else if with_host(|h| matches!(h.get(first), Some(JsObj::Object(p)) if !p.contains_key("@@native"))) {
            // Options object `{ port, address }`.
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get(first) {
                    if let Some(pv) = p.get("port") {
                        port = h.to_number(pv) as u16;
                    }
                    if let Some(av) = p.get("address").map(|v| h.str_of(v)) {
                        host = av;
                    }
                }
            });
        }
    }
    for a in args.iter().skip(1) {
        if is_str(a) {
            host = with_host(|h| h.str_of(a));
        } else if with_host(|h| crate::host::is_callable(h, a)) {
            cb = Some(a.clone());
        }
    }

    do_bind(recv, &host, port)?;

    // Fire `listening` + callback asynchronously on the main thread.
    let socket = recv.clone();
    let _ = with_host(|h| h.io_sender()).send(Box::new(move || {
        if let Some(cb) = cb {
            super::events::instance_call(&socket, "once", vec![with_host(|h| h.new_str("listening")), cb])?;
        }
        super::events::instance_call(&socket, "emit", vec![with_host(|h| h.new_str("listening"))])?;
        Ok(())
    }));
    Ok(recv.clone())
}

/// Bind the socket (idempotent: returns the existing socket if already bound),
/// register its record, take an event-loop handle, and spawn the recv loop.
fn do_bind(recv: &Value, host: &str, port: u16) -> Result<Arc<UdpSocket>, String> {
    if let Some(sock) = live_socket(recv) {
        return Ok(sock);
    }
    let socket = UdpSocket::bind((host, port)).map_err(|e| format!("Error: bind EADDRINUSE: {e}"))?;
    // A read timeout lets the recv loop re-check its stop flag on a quiet socket.
    socket.set_read_timeout(Some(POLL)).ok();
    let socket = Arc::new(socket);

    let id = next_id();
    set_prop(recv, "@@dgramid", Value::Float(id as f64));
    let stop = Arc::new(AtomicBool::new(false));
    DGRAM.with(|s| {
        s.borrow_mut().sockets.insert(
            id,
            UdpRec { emitter: recv.clone(), socket: socket.clone(), stop: stop.clone() },
        );
    });
    with_host(|h| h.incr_handle());

    // Spawn the recv loop: raw datagrams → posted IoTasks. Never touches the host.
    let tx = with_host(|h| h.io_sender());
    let recv_sock = socket.clone();
    std::thread::spawn(move || recv_loop(recv_sock, id, stop, tx));

    Ok(socket)
}

/// Background reader: blocking `recv_from` loop posting `message` events. Runs off
/// the main thread and only moves `Send` data (bytes, addr, port) into the closure.
fn recv_loop(
    socket: Arc<UdpSocket>,
    id: u64,
    stop: Arc<AtomicBool>,
    tx: std::sync::mpsc::Sender<crate::host::IoTask>,
) {
    let mut buf = [0u8; 65536];
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        match socket.recv_from(&mut buf) {
            Ok((n, src)) => {
                let bytes = buf[..n].to_vec();
                let address = src.ip().to_string();
                let port = src.port();
                let family = if src.is_ipv6() { "IPv6" } else { "IPv4" };
                let _ = tx.send(Box::new(move || on_message(id, bytes, address, port, family)));
            }
            // A read-timeout (or non-blocking would-block) just re-checks `stop`.
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                continue;
            }
            Err(_) => break,
        }
    }
}

/// Main-thread delivery of one datagram: emit `message` with `(msg, rinfo)` where
/// `rinfo = { address, family, port, size }` (matching Node).
fn on_message(id: u64, bytes: Vec<u8>, address: String, port: u16, family: &'static str) -> Result<(), String> {
    let socket = DGRAM.with(|s| s.borrow().sockets.get(&id).map(|r| r.emitter.clone()));
    let Some(socket) = socket else { return Ok(()) };

    let size = bytes.len();
    let msg = super::buffer::from_bytes(&bytes);
    let rinfo = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("address".into(), h.new_str(address));
        m.insert("family".into(), h.new_str(family));
        m.insert("port".into(), Value::Float(port as f64));
        m.insert("size".into(), Value::Float(size as f64));
        h.new_object(m)
    });
    super::events::instance_call(&socket, "emit", vec![with_host(|h| h.new_str("message")), msg, rinfo])?;
    Ok(())
}

// ── send ──────────────────────────────────────────────────────────────────────

/// `socket.send(msg[, offset, length], port[, address][, callback])`. Auto-binds
/// to an ephemeral port on the socket's address family if not yet bound (Node
/// semantics), then `send_to` the bytes. The callback fires with `null` on
/// success (asynchronously, on the main thread).
fn socket_send(recv: &Value, args: &[Value]) -> Result<Value, String> {
    let msg = args.first().cloned().unwrap_or(Value::Undef);
    let full = value_bytes(&msg);

    // Collect the leading numeric args after `msg`: either `[port]` or
    // `[offset, length, port]` (Node distinguishes by count).
    let mut nums: Vec<f64> = Vec::new();
    let mut i = 1;
    while i < args.len() && is_num(&args[i]) {
        nums.push(with_host(|h| h.to_number(&args[i])));
        i += 1;
    }
    let (offset, length, port) = if nums.len() >= 3 {
        (nums[0].max(0.0) as usize, nums[1].max(0.0) as usize, nums[2] as u16)
    } else if let Some(p) = nums.first() {
        (0usize, full.len(), *p as u16)
    } else {
        return Err(crate::host::type_error("Port should be > 0 and < 65536"));
    };

    // Trailing args: optional address (string) then optional callback.
    let mut address = default_send_host(recv).to_string();
    let mut cb: Option<Value> = None;
    for a in args.iter().skip(i) {
        if is_str(a) {
            address = with_host(|h| h.str_of(a));
        } else if with_host(|h| crate::host::is_callable(h, a)) {
            cb = Some(a.clone());
        }
    }

    // Slice the payload to [offset, offset+length).
    let end = offset.saturating_add(length).min(full.len());
    let start = offset.min(full.len());
    let data = &full[start..end.max(start)];

    // Auto-bind to an ephemeral port on the socket's family if needed.
    let socket = do_bind(recv, default_bind_host(recv), 0)?;
    socket
        .send_to(data, (address.as_str(), port))
        .map_err(|e| format!("Error: send {e}"))?;

    // The send callback fires asynchronously with `(null)`.
    if let Some(cb) = cb {
        let _ = with_host(|h| h.io_sender()).send(Box::new(move || {
            let nul = with_host(|h| h.null());
            invoke(&cb, vec![nul], None)?;
            Ok(())
        }));
    }
    Ok(Value::Undef)
}

// ── close / address ───────────────────────────────────────────────────────────

/// `socket.close([callback])`: stop the recv loop, drop the handle, emit `close`,
/// and wake the event loop so a closed last handle lets it exit.
fn socket_close(recv: &Value, args: &[Value]) -> Result<Value, String> {
    if let Some(id) = u64_prop(recv, "@@dgramid") {
        let rec = DGRAM.with(|s| s.borrow_mut().sockets.remove(&id));
        if let Some(rec) = rec {
            rec.stop.store(true, Ordering::Release);
            with_host(|h| h.decr_handle());
            // Wake the blocking loop so it can re-evaluate `open_handles`.
            let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
        }
    }
    // A `close` callback registers as a one-shot `close` listener in Node.
    if let Some(cb) = args.first().filter(|v| with_host(|h| crate::host::is_callable(h, v))) {
        invoke(cb, Vec::new(), None)?;
    }
    super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("close"))])?;
    Ok(Value::Undef)
}

/// `socket.address()` → `{ address, family, port }` from `local_addr`. Throws if
/// the socket is not bound (matching Node's `Not running` error).
fn socket_address(recv: &Value) -> Result<Value, String> {
    let socket = live_socket(recv).ok_or_else(|| "Error: getsockname EBADF: bad file descriptor".to_string())?;
    let addr = socket.local_addr().map_err(|e| format!("Error: getsockname {e}"))?;
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("address".into(), h.new_str(addr.ip().to_string()));
        m.insert("family".into(), h.new_str(if addr.is_ipv6() { "IPv6" } else { "IPv4" }));
        m.insert("port".into(), Value::Float(addr.port() as f64));
        h.new_object(m)
    }))
}
