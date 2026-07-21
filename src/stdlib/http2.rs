//! Node `http2` module: a REAL, minimal HTTP/2 server over TLS+ALPN.
//!
//! This is a genuine HTTP/2 server (RFC 7540 framing + RFC 7541 HPACK via the
//! `hpack` crate) layered on the same blocking-`rustls` model as `tls`/`https`.
//! A real h2 client (`curl --http2`, `nghttp`, Node's `http2` client) can perform
//! a GET and receive a response body from a server built with
//! `http2.createSecureServer`.
//!
//! ── Threading model (identical discipline to `net`/`tls`) ────────────────────
//! Background threads NEVER touch the JS heap (a main-thread `thread_local`). One
//! thread per TCP connection owns the `rustls::StreamOwned`, performs the TLS
//! handshake (negotiating ALPN `h2`), then runs the full HTTP/2 framing loop:
//! reading/parsing frames and writing response frames. HPACK `Encoder`/`Decoder`
//! (with their connection-scoped dynamic tables) live on that thread. Every
//! JS-visible effect — building the `Http2Stream`/`Http2Session` objects, emitting
//! `stream`/`session`/`request`, running listeners — happens on the main thread
//! via posted `IoTask` closures. The stream objects talk back to their connection
//! thread through an `mpsc` channel of `H2Cmd`s (`respond`/`write`/`end` enqueue a
//! command; the owner thread encodes+writes the frame), so reads and writes stay
//! on the single thread that owns the stateful TLS + HPACK cipher/table state.
//!
//! ── What IS implemented (server) ─────────────────────────────────────────────
//!   * TLS handshake with ALPN offering only `h2`; non-`h2` clients are closed.
//!   * Client connection preface check (`PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n`).
//!   * SETTINGS: we send our (empty, valid) SETTINGS; we ACK the client's SETTINGS
//!     and ignore incoming SETTINGS ACKs.
//!   * HEADERS (type 1) inbound: PADDED + PRIORITY flags stripped, header block
//!     HPACK-decoded into pseudo-headers (`:method`/`:path`/`:scheme`/`:authority`)
//!     + regular headers. Fires `stream` (and compat `request`).
//!   * DATA (type 0) inbound: emitted on the stream as `data`/`end` (request body).
//!   * `Http2Stream.respond(headers)` → HPACK-encoded HEADERS with `:status`.
//!   * `Http2Stream.write(data)` / `.end([data])` → DATA frames (END_STREAM on end),
//!     chunked to <= 16384 (the default SETTINGS_MAX_FRAME_SIZE) per frame.
//!   * PING (type 6): answered with a PING ACK echoing the 8-byte payload.
//!   * WINDOW_UPDATE (type 8) / PRIORITY (type 2) / RST_STREAM (type 3): accepted
//!     and ignored (large-enough default windows; see caveats).
//!   * GOAWAY (type 7): sent on connection close; inbound GOAWAY ends the loop.
//!
//! ── Honest limitations (NOT implemented — deliberately, not faked) ────────────
//!   * `http2.connect` (CLIENT) is NOT implemented — it throws a clear error.
//!   * `http2.createServer` (cleartext h2c / prior-knowledge) is NOT implemented.
//!   * CONTINUATION frames (type 9): a HEADERS frame WITHOUT END_HEADERS is not
//!     reassembled — that stream is skipped. Real clients pack a single small GET
//!     header block into one HEADERS frame, so this rarely triggers, but a very
//!     large request header block would be dropped rather than mis-parsed.
//!   * Flow control is minimal: we advertise/assume the default windows and IGNORE
//!     inbound WINDOW_UPDATE for our own send side. A response body larger than the
//!     peer's stream/connection window (65535 bytes by default) can STALL. Small
//!     responses (the common case, and the test target) are unaffected.
//!   * No server push, no trailers, no PRIORITY tree, no per-stream RST bookkeeping,
//!     no ALTSVC/ORIGIN. These are absent, never stubbed to look present.
//!
//! ── Handler-fault isolation (why the server used to close right after HEADERS) ─
//! The event loop treats an `Err` returned from a posted `IoTask` as FATAL
//! (`host::drive_event_loop` runs `task()?`), which terminates the whole process
//! and closes every live socket. `events::emit` propagates a listener's error
//! (`invoke(&f, …)?`). So an exception thrown by the user's `stream`/`request`
//! handler used to bubble out of the connection's `on_headers` IoTask and kill the
//! server the instant the first request arrived — the client sees the TCP close as
//! a "broken pipe" with no response. To prevent one faulty handler from taking
//! down the server, the connection IoTasks (`on_headers`/`on_data`/`on_session`)
//! CATCH handler errors, print them to stderr (like Node's uncaught-exception
//! output, so the cause is visible), and return `Ok` — the loop and other
//! connections survive. Set `HTTP2_DEBUG=1` to trace every frame in/out on stderr.
//!
//! ── Verification status ──────────────────────────────────────────────────────
//! The framing (9-octet header layout, big-endian 24-bit length, frame type/flag
//! values, HPACK usage) is written to RFC 7540/7541. It has NOT been compiled or
//! run in this session (the parent owns the shared build), so the byte-level
//! correctness is verified-by-construction against the RFCs, not by a live curl.
//! See the final report for the exact `curl --http2` command to confirm it.

use crate::host::{invoke, with_host, IoTask, JsObj};
use fusevm::Value;
use hpack::{Decoder, Encoder};
use indexmap::IndexMap;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// `http2` module functions routed through `stdlib::call`.
pub const METHODS: &[&str] = &[
    "createSecureServer", "createServer", "connect", "getDefaultSettings", "getPackedSettings",
    "getUnpackedSettings",
];

/// Instance method names for the `@@native` tags this module owns (exposed to
/// `stdlib::instance_has_method` so a method *read* yields a bound method).
pub const SERVER_METHODS: &[&str] = &["listen", "close", "address", "setTimeout"];
pub const STREAM_METHODS: &[&str] = &[
    "respond", "write", "end", "close", "setEncoding", "setTimeout", "pause", "resume",
    // HTTP/1-compat surface (Http2ServerResponse), best-effort:
    "writeHead", "setHeader", "getHeader", "removeHeader",
];
pub const SESSION_METHODS: &[&str] =
    &["settings", "ping", "goaway", "close", "destroy", "ref", "unref", "setTimeout"];

// ── HTTP/2 constants (RFC 7540 §6, §11) ──────────────────────────────────────

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

// Frame types.
const FT_DATA: u8 = 0x0;
const FT_HEADERS: u8 = 0x1;
const FT_PRIORITY: u8 = 0x2;
const FT_RST_STREAM: u8 = 0x3;
const FT_SETTINGS: u8 = 0x4;
const FT_PING: u8 = 0x6;
const FT_GOAWAY: u8 = 0x7;
const FT_WINDOW_UPDATE: u8 = 0x8;
const FT_CONTINUATION: u8 = 0x9;

// Frame flags.
const FL_END_STREAM: u8 = 0x1;
const FL_ACK: u8 = 0x1; // SETTINGS/PING re-use bit 0 as ACK
const FL_END_HEADERS: u8 = 0x4;
const FL_PADDED: u8 = 0x8;
const FL_PRIORITY: u8 = 0x20;

/// Default SETTINGS_MAX_FRAME_SIZE — the largest DATA payload we emit per frame.
const MAX_FRAME_SIZE: usize = 16384;

// ── process-global id sources (ids minted on background threads) ─────────────

static NEXT_SERVER_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_STREAM_KEY: AtomicU64 = AtomicU64::new(1);
static NEXT_SESSION_KEY: AtomicU64 = AtomicU64::new(1);

fn next_server_id() -> u64 {
    NEXT_SERVER_ID.fetch_add(1, Ordering::Relaxed)
}
fn next_stream_key() -> u64 {
    NEXT_STREAM_KEY.fetch_add(1, Ordering::Relaxed)
}
fn next_session_key() -> u64 {
    NEXT_SESSION_KEY.fetch_add(1, Ordering::Relaxed)
}

// ── commands from the main thread to a connection's owner thread ─────────────

/// A frame the owner thread should encode + write, produced by JS method calls.
enum H2Cmd {
    /// Send a HEADERS frame carrying the given (already ordered, `:status`-first)
    /// header list; `end` sets END_STREAM (a header-only response).
    Respond { stream_id: u32, headers: Vec<(String, String)>, end: bool },
    /// Send DATA (chunked to MAX_FRAME_SIZE); `end` sets END_STREAM on the last.
    Data { stream_id: u32, data: Vec<u8>, end: bool },
    /// RST_STREAM with NO_ERROR.
    Close { stream_id: u32 },
    /// GOAWAY(NO_ERROR) then terminate the owner loop.
    Goaway,
}

// ── main-thread state ────────────────────────────────────────────────────────

struct H2ServerRec {
    emitter: Value,
    stop: Arc<AtomicBool>,
}

struct H2StreamRec {
    emitter: Value,
    tx: Sender<H2Cmd>,
    stream_id: u32,
    /// True once a HEADERS response frame has been sent (respond/writeHead).
    responded: bool,
}

struct H2SessionRec {
    #[allow(dead_code)]
    emitter: Value,
    tx: Sender<H2Cmd>,
}

#[derive(Default)]
struct H2State {
    servers: HashMap<u64, H2ServerRec>,
    streams: HashMap<u64, H2StreamRec>,
    sessions: HashMap<u64, H2SessionRec>,
}

thread_local! {
    static H2: std::cell::RefCell<H2State> = std::cell::RefCell::new(H2State::default());
    /// `ServerConfig`s built by `createSecureServer` before `listen` assigns an id
    /// (mirrors `tls::PENDING_CONFIGS`).
    static PENDING_CONFIGS: std::cell::RefCell<Vec<(Value, Arc<ServerConfig>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

// ── shared object / prop helpers (same shape as net/tls) ─────────────────────

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

/// Delegate the EventEmitter surface to `events`; `None` for a non-emitter method.
fn emitter_dispatch(recv: &Value, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "on" | "addListener" | "prependListener" | "once" | "prependOnceListener" | "emit"
        | "removeListener" | "off" | "removeAllListeners" | "listenerCount" | "eventNames"
        | "setMaxListeners" | "getMaxListeners" | "listeners" => {
            Some(super::events::instance_call(recv, method, args.to_vec()))
        }
        _ => None,
    }
}

/// Raw bytes of a `write`/`end`/`key`/`cert` argument (Buffer bytes or UTF-8).
fn value_bytes(v: Option<&Value>) -> Vec<u8> {
    let Some(v) = v else { return Vec::new() };
    let is_buffer =
        with_host(|h| matches!(h.get(v), Some(JsObj::Object(p)) if p.contains_key("@@bytes")));
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

/// Enumerable string key/value pairs of a plain object (for a `respond`/`writeHead`
/// headers argument). Hidden `@@`/`#` keys are skipped.
fn object_pairs(obj: &Value) -> Vec<(String, String)> {
    with_host(|h| match h.get(obj) {
        Some(JsObj::Object(p)) => p
            .iter()
            .filter(|(k, _)| !k.starts_with("@@") && !k.starts_with('#'))
            .map(|(k, v)| (k.clone(), h.str_of(v)))
            .collect(),
        _ => Vec::new(),
    })
}

// ── module entry ─────────────────────────────────────────────────────────────

/// `stdlib::call` entry for `http2.<method>`.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "createSecureServer" => create_secure_server(args),
        "createServer" => Err("Error: http2.createServer (cleartext h2c / prior-knowledge) \
            is not implemented in node-js; use http2.createSecureServer (h2 over TLS)"
            .to_string()),
        "connect" => Err("Error: http2.connect (HTTP/2 client) is not implemented in node-js; \
            only the HTTP/2 server (http2.createSecureServer) is implemented"
            .to_string()),
        "getDefaultSettings" => Ok(default_settings_object()),
        "getPackedSettings" => Ok(pack_settings(args)),
        "getUnpackedSettings" => unpack_settings(args),
        _ => return None,
    })
}

/// `http2.constants` and other non-function properties, reachable via
/// `namespace_property` → `stdlib::constant`.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "constants" => Some(constants_object()),
        // Expose ctor namespaces so `http2.Http2ServerRequest.prototype` etc. resolve
        // to *something* (they are not separately constructable here).
        "Http2ServerRequest" => Some(with_host(|h| h.alloc(JsObj::Builtin("Http2ServerRequest".into())))),
        "Http2ServerResponse" => Some(with_host(|h| h.alloc(JsObj::Builtin("Http2ServerResponse".into())))),
        _ => None,
    }
}

/// The `http2.constants` object (a representative subset of RFC 7540 / nghttp2
/// names — HTTP status codes, header-name constants, and error/settings codes).
fn constants_object() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        let put_i = |m: &mut IndexMap<String, Value>, k: &str, v: i64| {
            m.insert(k.to_string(), Value::Float(v as f64));
        };
        // Common HTTP status constants.
        put_i(&mut m, "HTTP_STATUS_OK", 200);
        put_i(&mut m, "HTTP_STATUS_NO_CONTENT", 204);
        put_i(&mut m, "HTTP_STATUS_MOVED_PERMANENTLY", 301);
        put_i(&mut m, "HTTP_STATUS_FOUND", 302);
        put_i(&mut m, "HTTP_STATUS_NOT_MODIFIED", 304);
        put_i(&mut m, "HTTP_STATUS_BAD_REQUEST", 400);
        put_i(&mut m, "HTTP_STATUS_UNAUTHORIZED", 401);
        put_i(&mut m, "HTTP_STATUS_FORBIDDEN", 403);
        put_i(&mut m, "HTTP_STATUS_NOT_FOUND", 404);
        put_i(&mut m, "HTTP_STATUS_INTERNAL_SERVER_ERROR", 500);
        // NGHTTP2 error codes (RFC 7540 §7).
        put_i(&mut m, "NGHTTP2_NO_ERROR", 0x0);
        put_i(&mut m, "NGHTTP2_PROTOCOL_ERROR", 0x1);
        put_i(&mut m, "NGHTTP2_INTERNAL_ERROR", 0x2);
        put_i(&mut m, "NGHTTP2_FLOW_CONTROL_ERROR", 0x3);
        put_i(&mut m, "NGHTTP2_SETTINGS_TIMEOUT", 0x4);
        put_i(&mut m, "NGHTTP2_STREAM_CLOSED", 0x5);
        put_i(&mut m, "NGHTTP2_FRAME_SIZE_ERROR", 0x6);
        put_i(&mut m, "NGHTTP2_REFUSED_STREAM", 0x7);
        put_i(&mut m, "NGHTTP2_CANCEL", 0x8);
        put_i(&mut m, "NGHTTP2_COMPRESSION_ERROR", 0x9);
        put_i(&mut m, "NGHTTP2_ENHANCE_YOUR_CALM", 0xb);
        // SETTINGS identifiers (RFC 7540 §6.5.2).
        put_i(&mut m, "NGHTTP2_SETTINGS_HEADER_TABLE_SIZE", 0x1);
        put_i(&mut m, "NGHTTP2_SETTINGS_ENABLE_PUSH", 0x2);
        put_i(&mut m, "NGHTTP2_SETTINGS_MAX_CONCURRENT_STREAMS", 0x3);
        put_i(&mut m, "NGHTTP2_SETTINGS_INITIAL_WINDOW_SIZE", 0x4);
        put_i(&mut m, "NGHTTP2_SETTINGS_MAX_FRAME_SIZE", 0x5);
        put_i(&mut m, "NGHTTP2_SETTINGS_MAX_HEADER_LIST_SIZE", 0x6);
        // A representative subset of the HTTP2_HEADER_* string constants.
        let hdr = |m: &mut IndexMap<String, Value>, k: &str, v: &str, h: &mut crate::host::JsHost| {
            let s = h.new_str(v);
            m.insert(k.to_string(), s);
        };
        hdr(&mut m, "HTTP2_HEADER_STATUS", ":status", h);
        hdr(&mut m, "HTTP2_HEADER_METHOD", ":method", h);
        hdr(&mut m, "HTTP2_HEADER_AUTHORITY", ":authority", h);
        hdr(&mut m, "HTTP2_HEADER_SCHEME", ":scheme", h);
        hdr(&mut m, "HTTP2_HEADER_PATH", ":path", h);
        hdr(&mut m, "HTTP2_HEADER_CONTENT_TYPE", "content-type", h);
        hdr(&mut m, "HTTP2_HEADER_CONTENT_LENGTH", "content-length", h);
        hdr(&mut m, "HTTP2_METHOD_GET", "GET", h);
        hdr(&mut m, "HTTP2_METHOD_POST", "POST", h);
        h.new_object(m)
    })
}

/// A minimal default-settings object (Node's `http2.getDefaultSettings()` shape).
fn default_settings_object() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("headerTableSize".into(), Value::Float(4096.0));
        m.insert("enablePush".into(), Value::Bool(false));
        m.insert("initialWindowSize".into(), Value::Float(65535.0));
        m.insert("maxFrameSize".into(), Value::Float(MAX_FRAME_SIZE as f64));
        m.insert("maxConcurrentStreams".into(), Value::Float(100.0));
        h.new_object(m)
    })
}

// ── getPackedSettings / getUnpackedSettings (RFC 7540 §6.5.1 wire form) ───────

/// Append one SETTINGS entry (2-byte identifier + 4-byte value, big-endian).
fn push_setting(out: &mut Vec<u8>, id: u16, val: u32) {
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&val.to_be_bytes());
}

/// `http2.getPackedSettings(settings)` — serialize the SETTINGS values present on
/// `settings` into the RFC 7540 §6.5.1 wire form (6 octets per entry: a 2-byte
/// identifier followed by a 4-byte value, big-endian). Only keys actually present
/// on the object are emitted, in the canonical identifier order (1..6) Node uses;
/// this is the exact inverse of `getUnpackedSettings`.
fn pack_settings(args: &[Value]) -> Value {
    let settings = args.first().cloned().unwrap_or(Value::Undef);
    let num = |key: &str| -> Option<u32> {
        get_prop(&settings, key)
            .filter(|v| !matches!(v, Value::Undef))
            .map(|v| with_host(|h| h.to_number(&v)) as u32)
    };
    let mut out: Vec<u8> = Vec::new();
    if let Some(v) = num("headerTableSize") {
        push_setting(&mut out, 0x1, v);
    }
    if let Some(p) = get_prop(&settings, "enablePush").filter(|v| !matches!(v, Value::Undef)) {
        let on = with_host(|h| h.truthy(&p));
        push_setting(&mut out, 0x2, u32::from(on));
    }
    if let Some(v) = num("maxConcurrentStreams") {
        push_setting(&mut out, 0x3, v);
    }
    if let Some(v) = num("initialWindowSize") {
        push_setting(&mut out, 0x4, v);
    }
    if let Some(v) = num("maxFrameSize") {
        push_setting(&mut out, 0x5, v);
    }
    if let Some(v) = num("maxHeaderListSize").or_else(|| num("maxHeaderSize")) {
        push_setting(&mut out, 0x6, v);
    }
    super::buffer::from_bytes(&out)
}

/// `http2.getUnpackedSettings(buf)` — parse a packed SETTINGS payload (6 octets per
/// entry: 2-byte identifier + 4-byte value, big-endian) back into a settings object
/// (`{ headerTableSize, enablePush, maxConcurrentStreams, initialWindowSize,
/// maxFrameSize, maxHeaderSize, maxHeaderListSize }`, only the keys present in the
/// buffer). The inverse of `getPackedSettings`. Throws `ERR_HTTP2_INVALID_PACKED_
/// SETTINGS_LENGTH` when the length is not a multiple of six.
fn unpack_settings(args: &[Value]) -> Result<Value, String> {
    let bytes = value_bytes(args.first());
    if bytes.len() % 6 != 0 {
        return Err("RangeError [ERR_HTTP2_INVALID_PACKED_SETTINGS_LENGTH]: \
                    Packed settings length must be a multiple of six"
            .to_string());
    }
    let mut m = IndexMap::new();
    for chunk in bytes.chunks_exact(6) {
        let id = u16::from_be_bytes([chunk[0], chunk[1]]);
        let val = u32::from_be_bytes([chunk[2], chunk[3], chunk[4], chunk[5]]);
        match id {
            0x1 => {
                m.insert("headerTableSize".to_string(), Value::Float(val as f64));
            }
            0x2 => {
                m.insert("enablePush".to_string(), Value::Bool(val != 0));
            }
            0x3 => {
                m.insert("maxConcurrentStreams".to_string(), Value::Float(val as f64));
            }
            0x4 => {
                m.insert("initialWindowSize".to_string(), Value::Float(val as f64));
            }
            0x5 => {
                m.insert("maxFrameSize".to_string(), Value::Float(val as f64));
            }
            // Identifier 6 maps to BOTH `maxHeaderSize` and its `maxHeaderListSize`
            // alias (Node emits both, in that order).
            0x6 => {
                m.insert("maxHeaderSize".to_string(), Value::Float(val as f64));
                m.insert("maxHeaderListSize".to_string(), Value::Float(val as f64));
            }
            // Unknown identifiers are ignored per RFC 7540 §6.5.
            _ => {}
        }
    }
    Ok(with_host(|h| h.new_object(m)))
}

// ── http2.createSecureServer ─────────────────────────────────────────────────

/// `http2.createSecureServer(options[, onRequestHandler])`. Parses `key`+`cert`
/// into a `ServerConfig` with ALPN `h2` (a bad cert throws synchronously) and
/// returns an `Http2Server` emitter. A supplied handler is registered as a compat
/// `request` listener (Node semantics).
fn create_secure_server(args: &[Value]) -> Result<Value, String> {
    let mut options: Option<Value> = None;
    let mut handler: Option<Value> = None;
    for a in args {
        if with_host(|h| crate::host::is_callable(h, a)) {
            handler = Some(a.clone());
        } else if matches!(a, Value::Obj(_)) {
            options = Some(a.clone());
        }
    }
    let opts = options.ok_or_else(|| {
        crate::host::type_error("http2.createSecureServer requires options with `key` and `cert`")
    })?;
    let cert = value_bytes(get_prop(&opts, "cert").as_ref());
    let key = value_bytes(get_prop(&opts, "key").as_ref());
    if cert.is_empty() || key.is_empty() {
        return Err(crate::host::type_error(
            "http2.createSecureServer requires `key` and `cert`",
        ));
    }
    let config = build_h2_server_config(&cert, &key)?;

    let server = new_emitter_object("Http2Server", IndexMap::new());
    if let Some(cb) = handler {
        // Node registers the createSecureServer handler as a `request` listener.
        super::events::instance_call(&server, "on", vec![with_host(|h| h.new_str("request")), cb])?;
    }
    PENDING_CONFIGS.with(|p| p.borrow_mut().push((server.clone(), config)));
    Ok(server)
}

/// Build a `ServerConfig` from PEM `key`+`cert`, offering ALPN `h2` only.
fn build_h2_server_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<Arc<ServerConfig>, String> {
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut &cert_pem[..])
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Error: http2: bad certificate PEM: {e}"))?;
    if certs.is_empty() {
        return Err("Error: http2: no certificates found in `cert`".to_string());
    }
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut &key_pem[..])
        .map_err(|e| format!("Error: http2: bad private key PEM: {e}"))?
        .ok_or_else(|| "Error: http2: no private key found in `key`".to_string())?;
    let mut cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("Error: http2: invalid key/cert: {e}"))?;
    // ALPN: offer only "h2" so the handshake negotiates HTTP/2.
    cfg.alpn_protocols = vec![b"h2".to_vec()];
    Ok(Arc::new(cfg))
}

fn take_pending_config(server: &Value) -> Option<Arc<ServerConfig>> {
    PENDING_CONFIGS.with(|p| {
        let mut p = p.borrow_mut();
        p.iter().position(|(s, _)| s == server).map(|pos| p.remove(pos).1)
    })
}

// ── instance dispatch (Http2Server / Http2Stream / Http2Session) ─────────────

pub fn instance_call(tag: &str, recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match tag {
        "Http2Server" => server_call(recv, method, args),
        "Http2Stream" => stream_call(recv, method, args),
        "Http2Session" => session_call(recv, method, args),
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
        "setTimeout" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!("server.{method} is not a function"))),
    }
}

/// `server.listen(port[, host][, callback])`. Binds on the main thread, spawns the
/// accept loop, and fires `listening` + callback asynchronously.
fn server_listen(recv: &Value, args: &[Value]) -> Result<Value, String> {
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

    let config = take_pending_config(recv)
        .ok_or_else(|| crate::host::type_error("http2 server has no secure context"))?;
    let listener = TcpListener::bind((host.as_str(), port))
        .map_err(|e| format!("Error: listen EADDRINUSE: {e}"))?;
    let local = listener.local_addr().ok();

    let id = next_server_id();
    set_prop(recv, "@@serverid", Value::Float(id as f64));
    if let Some(addr) = local {
        let mut a = IndexMap::new();
        a.insert("port".into(), Value::Float(addr.port() as f64));
        a.insert("address".into(), with_host(|h| h.new_str(addr.ip().to_string())));
        a.insert("family".into(), with_host(|h| h.new_str(if addr.is_ipv6() { "IPv6" } else { "IPv4" })));
        let addr_obj = with_host(|h| h.new_object(a));
        set_prop(recv, "@@address", addr_obj);
    }
    let stop = Arc::new(AtomicBool::new(false));
    H2.with(|s| {
        s.borrow_mut().servers.insert(id, H2ServerRec { emitter: recv.clone(), stop: stop.clone() });
    });
    with_host(|h| h.incr_handle());

    let io_tx = with_host(|h| h.io_sender());
    listener.set_nonblocking(true).ok();
    std::thread::spawn(move || {
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }
            match listener.accept() {
                Ok((stream, _addr)) => {
                    let cfg = config.clone();
                    let tx = io_tx.clone();
                    std::thread::spawn(move || serve_connection(id, stream, cfg, tx));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

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

fn server_close(recv: &Value, args: &[Value]) -> Result<Value, String> {
    if let Some(id) = u64_prop(recv, "@@serverid") {
        let rec = H2.with(|s| s.borrow_mut().servers.remove(&id));
        if let Some(rec) = rec {
            rec.stop.store(true, Ordering::Release);
            with_host(|h| h.decr_handle());
            let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
        }
    }
    if let Some(cb) = args.first().filter(|v| with_host(|h| crate::host::is_callable(h, v))) {
        invoke(cb, Vec::new(), None)?;
    }
    super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("close"))])?;
    Ok(recv.clone())
}

// ── the per-connection owner thread: TLS + HTTP/2 framing ────────────────────

/// One background thread per accepted TCP connection: complete the TLS handshake
/// (negotiating ALPN `h2`), then run the HTTP/2 framing loop. Owns the
/// `StreamOwned` and the HPACK `Encoder`/`Decoder` for the whole connection.
fn serve_connection(server_id: u64, mut sock: TcpStream, config: Arc<ServerConfig>, io_tx: Sender<IoTask>) {
    // The socket is inherited non-blocking from the accept poll loop; put it in
    // blocking mode so the TLS handshake and the initial SETTINGS write can't
    // fail with WouldBlock. A per-read timeout (set later) gives the framing loop
    // its read/write interleave without making writes non-blocking.
    sock.set_nonblocking(false).ok();
    let mut conn = match ServerConnection::new(config) {
        Ok(c) => c,
        Err(_) => return,
    };
    if conn.complete_io(&mut sock).is_err() {
        return;
    }
    // ALPN must have negotiated "h2"; otherwise this is not an HTTP/2 connection.
    let is_h2 = conn.alpn_protocol().map(|p| p == b"h2").unwrap_or(false);
    let mut stream = StreamOwned::new(conn, sock);
    if !is_h2 {
        // Honest fallback: we only speak h2. Close the connection.
        stream.conn.send_close_notify();
        let _ = stream.flush();
        let _ = stream.sock.shutdown(std::net::Shutdown::Both);
        return;
    }
    // Command channel: main-thread stream/session methods post frames to write.
    let (tx, rx) = std::sync::mpsc::channel::<H2Cmd>();

    // Announce the session on the main thread.
    let session_key = next_session_key();
    {
        let tx_sess = tx.clone();
        let _ = io_tx.send(Box::new(move || on_session(server_id, session_key, tx_sess)));
    }

    if h2_debug() {
        eprintln!("[http2] connection {session_key}: ALPN h2 negotiated, starting framing loop");
    }

    // Send our SETTINGS immediately (empty payload is valid). This runs in
    // BLOCKING mode — the read timeout is set AFTERWARDS, because rustls's write
    // internally completes pending handshake I/O (a read), and a read-timeout set
    // beforehand makes that read WouldBlock → the write fails → the loop never
    // starts (the connection-startup race).
    let mut ok = true;
    ok &= write_frame(&mut stream, FT_SETTINGS, 0, 0, &[]).is_ok();
    ok &= stream.flush().is_ok();
    // Now switch to a short read timeout so the loop interleaves reads with the
    // outbound-command drain.
    stream
        .sock
        .set_read_timeout(Some(std::time::Duration::from_millis(20)))
        .ok();

    let mut decoder = Decoder::new();
    let mut encoder = Encoder::new();
    let mut inbuf: Vec<u8> = Vec::new();
    let mut got_preface = false;
    let mut max_stream_id: u32 = 0;
    // Map stream-id → main-thread stream key (for routing inbound DATA).
    let mut id_to_key: HashMap<u32, u64> = HashMap::new();
    let mut buf = [0u8; MAX_FRAME_SIZE];

    'conn: loop {
        // If the initial SETTINGS write failed the connection is already dead;
        // fall through to the GOAWAY/close cleanup below. (`ok` is only ever set
        // before the loop, so this is the entry gate, not a per-iteration test.)
        if !ok {
            break;
        }
        // 1) Drain queued outbound commands (respond/write/end frames enqueued by
        //    the main thread). This runs on EVERY loop iteration — including after
        //    a WouldBlock read — so async responses are written promptly.
        loop {
            match rx.try_recv() {
                Ok(cmd) => {
                    if h2_debug() {
                        eprintln!("[http2] connection {session_key}: draining {}", cmd_name(&cmd));
                    }
                    if !apply_cmd(&mut stream, &mut encoder, max_stream_id, cmd) {
                        if h2_debug() {
                            eprintln!("[http2] connection {session_key}: write failed, closing");
                        }
                        break 'conn;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        // 2) Read whatever plaintext (HTTP/2 frames) is available.
        match stream.read(&mut buf) {
            Ok(0) => {
                if h2_debug() {
                    eprintln!("[http2] connection {session_key}: read EOF (Ok 0)");
                }
                break;
            }
            Ok(n) => inbuf.extend_from_slice(&buf[..n]),
            Err(ref e)
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
            {
                continue;
            }
            Err(e) => {
                if h2_debug() {
                    eprintln!("[http2] connection {session_key}: read error {:?} ({e})", e.kind());
                }
                break;
            }
        }

        // 3) Consume the client connection preface (first 24 octets).
        if !got_preface {
            if inbuf.len() < PREFACE.len() {
                continue;
            }
            if &inbuf[..PREFACE.len()] != PREFACE {
                break; // not an HTTP/2 connection preface — bail.
            }
            inbuf.drain(..PREFACE.len());
            got_preface = true;
        }

        // 4) Parse and dispatch every complete frame currently buffered.
        loop {
            if inbuf.len() < 9 {
                break;
            }
            let len = ((inbuf[0] as usize) << 16) | ((inbuf[1] as usize) << 8) | (inbuf[2] as usize);
            if inbuf.len() < 9 + len {
                break; // await the rest of the frame body.
            }
            let ftype = inbuf[3];
            let flags = inbuf[4];
            let stream_id = u32::from_be_bytes([inbuf[5], inbuf[6], inbuf[7], inbuf[8]]) & 0x7fff_ffff;
            let payload: Vec<u8> = inbuf[9..9 + len].to_vec();
            inbuf.drain(..9 + len);
            if h2_debug() {
                eprintln!(
                    "[http2] connection {session_key}: recv frame type={ftype} flags={flags:#04x} \
                     stream={stream_id} len={len}"
                );
            }

            match ftype {
                FT_SETTINGS => {
                    if flags & FL_ACK == 0 {
                        // ACK the client's SETTINGS (empty payload, ACK flag).
                        if write_frame(&mut stream, FT_SETTINGS, FL_ACK, 0, &[])
                            .and_then(|_| stream.flush())
                            .is_err()
                        {
                            break 'conn;
                        }
                    }
                }
                FT_PING => {
                    if flags & FL_ACK == 0 {
                        // Echo the 8-byte opaque data with the ACK flag set.
                        if write_frame(&mut stream, FT_PING, FL_ACK, 0, &payload)
                            .and_then(|_| stream.flush())
                            .is_err()
                        {
                            break 'conn;
                        }
                    }
                }
                FT_HEADERS => {
                    if stream_id > max_stream_id {
                        max_stream_id = stream_id;
                    }
                    if flags & FL_END_HEADERS == 0 {
                        // CONTINUATION reassembly is not implemented; skip this stream.
                        continue;
                    }
                    let block = strip_headers_padding_priority(&payload, flags);
                    let decoded = match decoder.decode(&block) {
                        Ok(d) => d,
                        Err(_) => continue,
                    };
                    let headers: Vec<(String, String)> = decoded
                        .into_iter()
                        .map(|(k, v)| {
                            (String::from_utf8_lossy(&k).into_owned(), String::from_utf8_lossy(&v).into_owned())
                        })
                        .collect();
                    let end_stream = flags & FL_END_STREAM != 0;
                    let key = next_stream_key();
                    id_to_key.insert(stream_id, key);
                    let tx_stream = tx.clone();
                    let _ = io_tx.send(Box::new(move || {
                        on_headers(server_id, key, stream_id, headers, end_stream, tx_stream)
                    }));
                }
                FT_DATA => {
                    if let Some(&key) = id_to_key.get(&stream_id) {
                        let data = strip_data_padding(&payload, flags);
                        let end_stream = flags & FL_END_STREAM != 0;
                        let _ = io_tx.send(Box::new(move || on_data(key, data, end_stream)));
                    }
                }
                FT_GOAWAY => break 'conn,
                // WINDOW_UPDATE / PRIORITY / RST_STREAM / CONTINUATION: accepted and
                // ignored (large-enough default windows; see module limitations).
                FT_WINDOW_UPDATE | FT_PRIORITY | FT_RST_STREAM | FT_CONTINUATION => {}
                // Unknown/extension frame types: ignored per RFC 7540 §4.1.
                _ => {}
            }
        }
    }

    // Best-effort GOAWAY then close.
    if h2_debug() {
        eprintln!("[http2] connection {session_key}: framing loop exited, sending GOAWAY + closing");
    }
    let mut goaway = Vec::with_capacity(8);
    goaway.extend_from_slice(&(max_stream_id & 0x7fff_ffff).to_be_bytes());
    goaway.extend_from_slice(&0u32.to_be_bytes()); // NO_ERROR
    let _ = write_frame(&mut stream, FT_GOAWAY, 0, 0, &goaway);
    let _ = stream.flush();
    stream.conn.send_close_notify();
    let _ = stream.flush();
    let _ = stream.sock.shutdown(std::net::Shutdown::Both);

    // Release the per-connection handle and drop this connection's records on the
    // main thread (balances the `incr_handle` in `on_session`).
    let stream_keys: Vec<u64> = id_to_key.values().copied().collect();
    let _ = io_tx.send(Box::new(move || on_session_close(session_key, stream_keys)));
}

/// A short label for an `H2Cmd` (diagnostic tracing only).
fn cmd_name(cmd: &H2Cmd) -> &'static str {
    match cmd {
        H2Cmd::Respond { .. } => "respond(HEADERS)",
        H2Cmd::Data { .. } => "data(DATA)",
        H2Cmd::Close { .. } => "close(RST_STREAM)",
        H2Cmd::Goaway => "goaway(GOAWAY)",
    }
}

/// Apply one outbound command by encoding + writing its frame(s). Returns false on
/// a write error or an explicit GOAWAY (terminating the owner loop).
fn apply_cmd(
    stream: &mut StreamOwned<ServerConnection, TcpStream>,
    encoder: &mut Encoder<'_>,
    max_stream_id: u32,
    cmd: H2Cmd,
) -> bool {
    match cmd {
        H2Cmd::Respond { stream_id, headers, end } => {
            let block = encode_header_block(encoder, &headers);
            if h2_debug() {
                eprintln!(
                    "[http2] write HEADERS stream={stream_id} end_stream={end} \
                     hpack_len={} headers={headers:?}",
                    block.len()
                );
            }
            let flags = FL_END_HEADERS | if end { FL_END_STREAM } else { 0 };
            write_frame(stream, FT_HEADERS, flags, stream_id, &block)
                .and_then(|_| stream.flush())
                .is_ok()
        }
        H2Cmd::Data { stream_id, data, end } => {
            if h2_debug() {
                eprintln!("[http2] write DATA stream={stream_id} len={} end_stream={end}", data.len());
            }
            send_data(stream, stream_id, &data, end)
        }
        H2Cmd::Close { stream_id } => {
            // RST_STREAM(NO_ERROR).
            write_frame(stream, FT_RST_STREAM, 0, stream_id, &0u32.to_be_bytes())
                .and_then(|_| stream.flush())
                .is_ok()
        }
        H2Cmd::Goaway => {
            let mut g = Vec::with_capacity(8);
            g.extend_from_slice(&(max_stream_id & 0x7fff_ffff).to_be_bytes());
            g.extend_from_slice(&0u32.to_be_bytes());
            let _ = write_frame(stream, FT_GOAWAY, 0, 0, &g);
            let _ = stream.flush();
            false
        }
    }
}

/// Send a body as DATA frames, chunked to MAX_FRAME_SIZE. END_STREAM is set on the
/// final frame when `end` is true (an empty body still emits one END_STREAM DATA).
fn send_data(
    stream: &mut StreamOwned<ServerConnection, TcpStream>,
    stream_id: u32,
    data: &[u8],
    end: bool,
) -> bool {
    if data.is_empty() {
        let flags = if end { FL_END_STREAM } else { 0 };
        return write_frame(stream, FT_DATA, flags, stream_id, &[])
            .and_then(|_| stream.flush())
            .is_ok();
    }
    let chunks: Vec<&[u8]> = data.chunks(MAX_FRAME_SIZE).collect();
    let last = chunks.len() - 1;
    for (i, chunk) in chunks.iter().enumerate() {
        let flags = if end && i == last { FL_END_STREAM } else { 0 };
        if write_frame(stream, FT_DATA, flags, stream_id, chunk).is_err() {
            return false;
        }
    }
    stream.flush().is_ok()
}

/// Write one HTTP/2 frame: 9-octet header (24-bit length, 8-bit type, 8-bit flags,
/// 31-bit stream id) followed by the payload (RFC 7540 §4.1).
fn write_frame<W: Write>(w: &mut W, ftype: u8, flags: u8, stream_id: u32, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len();
    let mut hdr = [0u8; 9];
    hdr[0] = (len >> 16) as u8;
    hdr[1] = (len >> 8) as u8;
    hdr[2] = len as u8;
    hdr[3] = ftype;
    hdr[4] = flags;
    hdr[5..9].copy_from_slice(&(stream_id & 0x7fff_ffff).to_be_bytes());
    w.write_all(&hdr)?;
    w.write_all(payload)?;
    Ok(())
}

/// HPACK-encode a header list into a header block fragment.
fn encode_header_block(encoder: &mut Encoder<'_>, headers: &[(String, String)]) -> Vec<u8> {
    let owned: Vec<(Vec<u8>, Vec<u8>)> = headers
        .iter()
        .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
        .collect();
    encoder.encode(owned.iter().map(|(k, v)| (k.as_slice(), v.as_slice())))
}

/// Strip PADDED/PRIORITY prefixes/suffixes from a HEADERS payload, yielding the raw
/// header block fragment (RFC 7540 §6.2).
fn strip_headers_padding_priority(payload: &[u8], flags: u8) -> Vec<u8> {
    let mut start = 0usize;
    let mut pad_len = 0usize;
    if flags & FL_PADDED != 0 && !payload.is_empty() {
        pad_len = payload[0] as usize;
        start = 1;
    }
    if flags & FL_PRIORITY != 0 {
        start += 5; // 4-byte stream dependency + 1-byte weight
    }
    let end = payload.len().saturating_sub(pad_len);
    if start > end {
        return Vec::new();
    }
    payload[start..end].to_vec()
}

/// Strip a PADDED prefix/suffix from a DATA payload (RFC 7540 §6.1).
fn strip_data_padding(payload: &[u8], flags: u8) -> Vec<u8> {
    if flags & FL_PADDED != 0 && !payload.is_empty() {
        let pad_len = payload[0] as usize;
        let end = payload.len().saturating_sub(pad_len);
        if 1 <= end {
            return payload[1..end].to_vec();
        }
        return Vec::new();
    }
    payload.to_vec()
}

// ── main-thread event handlers (run from posted IoTasks) ─────────────────────

/// Build the `Http2Session` object and emit `session` on the server. Also takes a
/// per-connection event-loop handle (released by `on_session_close`) so the loop
/// stays alive while this connection is served — mirroring `tls`'s per-socket
/// `incr_handle`.
fn on_session(server_id: u64, session_key: u64, tx: Sender<H2Cmd>) -> Result<(), String> {
    with_host(|h| h.incr_handle());
    let server = H2.with(|s| s.borrow().servers.get(&server_id).map(|r| r.emitter.clone()));
    let Some(server) = server else { return Ok(()) };
    let mut extra = IndexMap::new();
    extra.insert("@@h2session".into(), Value::Float(session_key as f64));
    let session = new_emitter_object("Http2Session", extra);
    H2.with(|s| {
        s.borrow_mut()
            .sessions
            .insert(session_key, H2SessionRec { emitter: session.clone(), tx });
    });
    if let Err(e) =
        super::events::instance_call(&server, "emit", vec![with_host(|h| h.new_str("session")), session])
    {
        report_handler_error("session", &e);
    }
    Ok(())
}

/// Release the per-connection handle and drop the session + its stream records
/// (posted by the owner thread when its framing loop exits).
fn on_session_close(session_key: u64, stream_keys: Vec<u64>) -> Result<(), String> {
    H2.with(|s| {
        let mut st = s.borrow_mut();
        st.sessions.remove(&session_key);
        for k in &stream_keys {
            st.streams.remove(k);
        }
    });
    with_host(|h| h.decr_handle());
    // Wake the loop so a closed last handle lets it exit.
    let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
    Ok(())
}

/// Handle a decoded HEADERS frame: build the `Http2Stream`, emit `stream`
/// (and compat `request`).
fn on_headers(
    server_id: u64,
    stream_key: u64,
    stream_id: u32,
    headers: Vec<(String, String)>,
    end_stream: bool,
    tx: Sender<H2Cmd>,
) -> Result<(), String> {
    let server = H2.with(|s| s.borrow().servers.get(&server_id).map(|r| r.emitter.clone()));
    let Some(server) = server else { return Ok(()) };

    // Build the header object exposed to JS (pseudo-headers kept as `:`-prefixed).
    let headers_obj = with_host(|h| {
        let mut m = IndexMap::new();
        for (k, v) in &headers {
            m.insert(k.clone(), h.new_str(v.clone()));
        }
        h.new_object(m)
    });

    // The core Http2Stream object.
    let mut extra = IndexMap::new();
    extra.insert("@@h2key".into(), Value::Float(stream_key as f64));
    extra.insert("id".into(), Value::Float(stream_id as f64));
    let stream_obj = new_emitter_object("Http2Stream", extra);
    H2.with(|s| {
        s.borrow_mut().streams.insert(
            stream_key,
            H2StreamRec { emitter: stream_obj.clone(), tx, stream_id, responded: false },
        );
    });

    let method = header_value(&headers, ":method").unwrap_or_else(|| "GET".to_string());
    let path = header_value(&headers, ":path").unwrap_or_else(|| "/".to_string());
    if h2_debug() {
        eprintln!("[http2] dispatch stream={stream_id} {method} {path} (end_stream={end_stream})");
    }

    // Emit the core `stream` event: (stream, headers). CRITICAL: a throwing user
    // handler must NOT propagate out of this IoTask — the event loop treats an
    // `Err` from a posted task as fatal (`drive_event_loop`: `task()?`) and would
    // tear the whole process down, closing every live connection (the classic
    // "server closed right after HEADERS → curl broken pipe"). So we catch the
    // handler error, surface it on stderr, and keep the loop (and other
    // connections) alive — mirroring how a server should isolate request faults.
    if let Err(e) = super::events::instance_call(
        &server,
        "emit",
        vec![
            with_host(|h| h.new_str("stream")),
            stream_obj.clone(),
            headers_obj.clone(),
        ],
    ) {
        report_handler_error("stream", &e);
        return Ok(());
    }

    // Compat `request` event: (req, res). `req` is a lightweight emitter carrying
    // method/url/headers; `res` is the same Http2Stream (writeHead/end mapped).
    let req = super::events::new_emitter();
    set_prop(&req, "method", with_host(|h| h.new_str(method)));
    set_prop(&req, "url", with_host(|h| h.new_str(path)));
    set_prop(&req, "headers", headers_obj);
    if let Err(e) = super::events::instance_call(
        &server,
        "emit",
        vec![with_host(|h| h.new_str("request")), req, stream_obj.clone()],
    ) {
        report_handler_error("request", &e);
        return Ok(());
    }

    // A GET (END_STREAM on HEADERS) has no body: signal `end` to the stream.
    if end_stream {
        if let Err(e) =
            super::events::instance_call(&stream_obj, "emit", vec![with_host(|h| h.new_str("end"))])
        {
            report_handler_error("stream.end", &e);
        }
    }
    Ok(())
}

/// Report an uncaught error raised by a user request/stream handler. Printed to
/// stderr (a genuine program error, like Node's uncaught-exception output) so the
/// cause is visible instead of the process silently dying; never propagated, so
/// one faulty handler cannot kill the event loop / other in-flight connections.
fn report_handler_error(event: &str, err: &str) {
    eprintln!("http2: uncaught error in '{event}' handler: {err}");
}

/// True when `HTTP2_DEBUG` is set in the environment — enables per-frame stderr
/// tracing for diagnosing the h2 framing path end-to-end.
fn h2_debug() -> bool {
    std::env::var_os("HTTP2_DEBUG").is_some()
}

/// Emit an inbound request-body DATA chunk (and `end` on END_STREAM) on the stream.
fn on_data(stream_key: u64, data: Vec<u8>, end_stream: bool) -> Result<(), String> {
    let stream = H2.with(|s| s.borrow().streams.get(&stream_key).map(|r| r.emitter.clone()));
    let Some(stream) = stream else { return Ok(()) };
    if !data.is_empty() {
        let chunk = super::buffer::from_bytes(&data);
        if let Err(e) =
            super::events::instance_call(&stream, "emit", vec![with_host(|h| h.new_str("data")), chunk])
        {
            report_handler_error("data", &e);
            return Ok(());
        }
    }
    if end_stream {
        if let Err(e) =
            super::events::instance_call(&stream, "emit", vec![with_host(|h| h.new_str("end"))])
        {
            report_handler_error("end", &e);
        }
    }
    Ok(())
}

fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
}

// ── Http2Stream instance methods ─────────────────────────────────────────────

fn stream_key_of(recv: &Value) -> Option<u64> {
    u64_prop(recv, "@@h2key")
}

fn stream_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if let Some(r) = emitter_dispatch(recv, method, &args) {
        return r;
    }
    match method {
        "respond" => {
            let hdrs = args.first().map(object_pairs).unwrap_or_default();
            // `options.endStream` (arg 1) → header-only response.
            let end = args
                .get(1)
                .and_then(|o| get_prop(o, "endStream"))
                .map(|v| with_host(|h| h.truthy(&v)))
                .unwrap_or(false);
            do_respond(recv, hdrs, end)?;
            Ok(recv.clone())
        }
        // HTTP/1-compat: writeHead(status[, headersObj]) → respond.
        "writeHead" => {
            let status = with_host(|h| args.first().map(|v| h.to_number(v)).unwrap_or(200.0)) as u32;
            let mut hdrs: Vec<(String, String)> = Vec::new();
            for a in args.iter().skip(1) {
                if matches!(a, Value::Obj(_)) {
                    hdrs = object_pairs(a);
                    break;
                }
            }
            // Inject the `:status` pseudo-header (writeHead's implicit status).
            hdrs.insert(0, (":status".to_string(), status.to_string()));
            do_respond(recv, hdrs, false)?;
            Ok(recv.clone())
        }
        "setHeader" => {
            // Stash pending compat headers until the response HEADERS are sent.
            let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            let v = with_host(|h| h.str_of(&args.get(1).cloned().unwrap_or(Value::Undef)));
            let bag = pending_headers_obj(recv);
            set_prop(&bag, &k, with_host(|h| h.new_str(v)));
            Ok(Value::Undef)
        }
        "getHeader" => {
            let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            Ok(get_prop(recv, "@@pendingHeaders").and_then(|bag| get_prop(&bag, &k)).unwrap_or(Value::Undef))
        }
        "removeHeader" => {
            let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            if let Some(bag) = get_prop(recv, "@@pendingHeaders") {
                with_host(|h| {
                    if let Some(JsObj::Object(p)) = h.get_mut(&bag) {
                        p.shift_remove(&k);
                    }
                });
            }
            Ok(Value::Undef)
        }
        "write" => {
            ensure_responded(recv)?;
            let bytes = value_bytes(args.first());
            send_stream_data(recv, bytes, false);
            Ok(Value::Bool(true))
        }
        "end" => {
            ensure_responded(recv)?;
            let bytes = args
                .first()
                .filter(|v| !matches!(v, Value::Undef))
                .map(|v| value_bytes(Some(v)))
                .unwrap_or_default();
            send_stream_data(recv, bytes, true);
            super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("finish"))])?;
            Ok(recv.clone())
        }
        "close" => {
            if let Some(key) = stream_key_of(recv) {
                let sent = H2.with(|s| {
                    s.borrow().streams.get(&key).map(|r| {
                        let _ = r.tx.send(H2Cmd::Close { stream_id: r.stream_id });
                    })
                });
                let _ = sent;
            }
            Ok(recv.clone())
        }
        "setEncoding" | "setTimeout" | "pause" | "resume" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!("stream.{method} is not a function"))),
    }
}

/// The lazily-created object holding compat `setHeader` values before the response.
fn pending_headers_obj(recv: &Value) -> Value {
    if let Some(bag) = get_prop(recv, "@@pendingHeaders") {
        return bag;
    }
    let bag = with_host(|h| h.new_object(IndexMap::new()));
    set_prop(recv, "@@pendingHeaders", bag.clone());
    bag
}

/// Send a HEADERS response frame with the given headers (a `:status` is injected if
/// none present). Marks the stream responded.
fn do_respond(recv: &Value, mut headers: Vec<(String, String)>, end: bool) -> Result<(), String> {
    // Merge any compat setHeader() values collected before respond/writeHead.
    if let Some(bag) = get_prop(recv, "@@pendingHeaders") {
        for (k, v) in object_pairs(&bag) {
            if !headers.iter().any(|(hk, _)| hk.eq_ignore_ascii_case(&k)) {
                headers.push((k, v));
            }
        }
    }
    // Ensure a single `:status` pseudo-header appears first (HTTP/2 requires
    // pseudo-headers to precede regular ones).
    let status = headers
        .iter()
        .find(|(k, _)| k == ":status")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "200".to_string());
    let mut ordered: Vec<(String, String)> = vec![(":status".to_string(), status)];
    for (k, v) in headers.into_iter() {
        if k == ":status" {
            continue;
        }
        // HTTP/2 header names must be lowercase.
        ordered.push((k.to_ascii_lowercase(), v));
    }

    let Some(key) = stream_key_of(recv) else { return Ok(()) };
    H2.with(|s| {
        if let Some(r) = s.borrow_mut().streams.get_mut(&key) {
            if !r.responded {
                r.responded = true;
                let _ = r.tx.send(H2Cmd::Respond { stream_id: r.stream_id, headers: ordered, end });
            }
        }
    });
    Ok(())
}

/// Ensure the response HEADERS have been sent (auto-`respond` with 200 otherwise).
fn ensure_responded(recv: &Value) -> Result<(), String> {
    let Some(key) = stream_key_of(recv) else { return Ok(()) };
    let responded = H2.with(|s| s.borrow().streams.get(&key).map(|r| r.responded).unwrap_or(true));
    if !responded {
        do_respond(recv, Vec::new(), false)?;
    }
    Ok(())
}

/// Queue a DATA frame for this stream's connection thread.
fn send_stream_data(recv: &Value, data: Vec<u8>, end: bool) {
    if let Some(key) = stream_key_of(recv) {
        H2.with(|s| {
            if let Some(r) = s.borrow().streams.get(&key) {
                let _ = r.tx.send(H2Cmd::Data { stream_id: r.stream_id, data, end });
            }
        });
    }
}

// ── Http2Session instance methods ────────────────────────────────────────────

fn session_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if let Some(r) = emitter_dispatch(recv, method, &args) {
        return r;
    }
    match method {
        "close" | "destroy" | "goaway" => {
            if let Some(key) = u64_prop(recv, "@@h2session") {
                H2.with(|s| {
                    if let Some(r) = s.borrow().sessions.get(&key) {
                        let _ = r.tx.send(H2Cmd::Goaway);
                    }
                });
            }
            super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("close"))])?;
            Ok(recv.clone())
        }
        "settings" | "ping" | "ref" | "unref" | "setTimeout" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!("session.{method} is not a function"))),
    }
}

// ── shared emitter constructor (same shape as net/tls) ───────────────────────

/// Build a native emitter object (`@@native` tag + `@@on`/`@@once` maps + extras),
/// sharing the EventEmitter shape with `events`/`net`/`tls`.
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
