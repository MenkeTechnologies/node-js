//! Node `https` module: HTTP/1.1 over real TLS.
//!
//! Server: `https.createServer(options, requestListener)` builds a `tls` server
//! (real rustls handshake, see `tls.rs`) whose per-connection hook attaches an
//! HTTP/1.1 request parser. As decrypted bytes arrive (`tls::on_socket_data` →
//! `https::feed`) we parse complete requests, build an `IncomingMessage` (`req`)
//! and a response object (`res`), and call the user's `(req, res)` listener.
//! `res.end` serializes an HTTP/1.1 response and writes it back through the TLS
//! channel via `tls::socket_write`.
//!
//! Client: `https.request(options[, cb])` / `https.get(url[, cb])` open a blocking
//! TLS connection on a background thread, write the request, read the full
//! response (the request forces `Connection: close`, so read-to-EOF is reliable),
//! parse it into an `IncomingMessage`, and fire `cb(res)` on the main thread.
//!
//! The HTTP/1.1 request parser and response serializer here are minimal
//! reimplementations of the private logic in `http.rs` (`parse_request`,
//! `serialize_response`). They are duplicated because that logic writes to `net`
//! sockets via `net::socket_write_id`, whereas an https response must go through
//! the TLS write channel. If `http::parse_request`/`ParsedReq`/`serialize_response`/
//! `ResState` were made `pub` (and response writing were sink-agnostic), https
//! could delegate to them instead.

use crate::host::{invoke, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;

/// `https` module functions routed through `stdlib::call`.
pub const MODULE_METHODS: &[&str] = &["createServer", "request", "get"];

/// Instance method names for this module's `@@native` tags (property reads that
/// yield a bound method), exposed to `stdlib::instance_has_method`.
pub const RESPONSE_METHODS: &[&str] = &[
    "writeHead", "setHeader", "getHeader", "getHeaderNames", "getHeaders", "hasHeader",
    "removeHeader", "write", "end", "flushHeaders",
];
pub const CLIENT_REQUEST_METHODS: &[&str] =
    &["write", "end", "setHeader", "getHeader", "removeHeader", "abort", "destroy", "setTimeout"];

// ── module-level non-function values (https.Agent / globalAgent) ─────────────

/// `https.Agent` / `https.globalAgent`: a minimal stub. node-js opens a fresh
/// connection per request (no pooling / keep-alive reuse), so an Agent carries no
/// behavior beyond being a constructible/inspectable object.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "Agent" => Some(with_host(|h| h.alloc(JsObj::Builtin("https.Agent".into())))),
        "globalAgent" => Some(with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("@@native".into(), h.new_str("Agent"));
            m.insert("maxSockets".into(), Value::Float(f64::INFINITY));
            m.insert("protocol".into(), h.new_str("https:"));
            h.new_object(m)
        })),
        _ => None,
    }
}

/// `stdlib::call` entry for `https.<method>`.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "createServer" => Some(create_server(args)),
        "request" => Some(request(args, false)),
        "get" => Some(request(args, true)),
        _ => None,
    }
}

// ── shared prop helpers ──────────────────────────────────────────────────────

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

/// Raw bytes of a value: Buffer bytes, else its UTF-8 string form.
fn value_bytes(v: Option<&Value>) -> Vec<u8> {
    let Some(v) = v else { return Vec::new() };
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

// ── server: per-connection parse state ───────────────────────────────────────

struct HttpsConn {
    listener: Value,
    buf: Vec<u8>,
}

struct ResState {
    sock_id: u64,
    status: u16,
    message: Option<String>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

thread_local! {
    static CONNS: std::cell::RefCell<HashMap<u64, HttpsConn>> =
        std::cell::RefCell::new(HashMap::new());
    static RESPONSES: std::cell::RefCell<HashMap<u64, ResState>> =
        std::cell::RefCell::new(HashMap::new());
    static NEXT_RESID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
    static CLIENT_REQS: std::cell::RefCell<HashMap<u64, ClientReq>> =
        std::cell::RefCell::new(HashMap::new());
    static NEXT_REQID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn next_resid() -> u64 {
    NEXT_RESID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}
fn next_reqid() -> u64 {
    NEXT_REQID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}

// ── https.createServer ───────────────────────────────────────────────────────

pub fn create_server(args: &[Value]) -> Result<Value, String> {
    let mut options: Option<Value> = None;
    let mut listener = Value::Undef;
    for a in args {
        if with_host(|h| crate::host::is_callable(h, a)) {
            listener = a.clone();
        } else if matches!(a, Value::Obj(_)) {
            options = Some(a.clone());
        }
    }
    let opts = options.ok_or_else(|| {
        crate::host::type_error("https.createServer requires an options object with `key` and `cert`")
    })?;
    let cert = value_bytes(get_prop(&opts, "cert").as_ref());
    let key = value_bytes(get_prop(&opts, "key").as_ref());
    if cert.is_empty() || key.is_empty() {
        return Err(crate::host::type_error("https.createServer requires `key` and `cert`"));
    }
    let config = super::tls::build_server_config(&cert, &key)?;

    // Per-connection hook: register the http parser for this socket id.
    let listener_for_hook = listener.clone();
    let hook: super::tls::ConnHook = std::rc::Rc::new(move |_server: &Value, _socket: &Value, sock_id: u64| {
        CONNS.with(|c| {
            c.borrow_mut()
                .insert(sock_id, HttpsConn { listener: listener_for_hook.clone(), buf: Vec::new() });
        });
        Ok(())
    });
    Ok(super::tls::create_server_with_config(config, hook, listener))
}

/// Discard an https connection when its TLS socket closes (called by `tls`).
pub fn drop_conn(sock_id: u64) {
    CONNS.with(|c| {
        c.borrow_mut().remove(&sock_id);
    });
}

// ── server request parsing (called from tls::on_socket_data) ─────────────────

/// Feed decrypted bytes into the https request parser. No-op for a socket that is
/// not an https connection. Runs on the main thread.
pub fn feed(sock_id: u64, _socket: &Value, bytes: &[u8]) -> Result<(), String> {
    let is_https = CONNS.with(|c| c.borrow().contains_key(&sock_id));
    if !is_https {
        return Ok(());
    }
    CONNS.with(|c| c.borrow_mut().get_mut(&sock_id).unwrap().buf.extend_from_slice(bytes));

    loop {
        let (listener, parsed) = CONNS.with(|c| {
            let mut c = c.borrow_mut();
            let conn = c.get_mut(&sock_id).unwrap();
            match parse_request(&conn.buf) {
                Some((req, consumed)) => {
                    conn.buf.drain(..consumed);
                    (conn.listener.clone(), Some(req))
                }
                None => (Value::Undef, None),
            }
        });
        let Some(parsed) = parsed else { break };

        let req = build_incoming(&parsed);
        let res = build_response(sock_id);
        if with_host(|h| crate::host::is_callable(h, &listener)) {
            invoke(&listener, vec![req.clone(), res], None)?;
        }
        if !parsed.body.is_empty() {
            let chunk = super::buffer::from_bytes(&parsed.body);
            super::events::instance_call(&req, "emit", vec![with_host(|h| h.new_str("data")), chunk])?;
        }
        super::events::instance_call(&req, "emit", vec![with_host(|h| h.new_str("end"))])?;
    }
    Ok(())
}

/// A fully parsed HTTP request. (Mirror of `http::ParsedReq`.)
struct ParsedReq {
    method: String,
    url: String,
    http_version: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Parse one complete request from `buf`, or `None` if more bytes are needed.
/// (Mirror of `http::parse_request`.)
fn parse_request(buf: &[u8]) -> Option<(ParsedReq, usize)> {
    let head_end = find_subslice(buf, b"\r\n\r\n")?;
    let head = &buf[..head_end];
    let body_start = head_end + 4;

    let head_str = String::from_utf8_lossy(head);
    let mut lines = head_str.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split(' ');
    let method = parts.next()?.to_string();
    let url = parts.next()?.to_string();
    let version = parts.next().unwrap_or("HTTP/1.1");
    let http_version = version.strip_prefix("HTTP/").unwrap_or("1.1").to_string();

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let name = k.trim().to_ascii_lowercase();
            let value = v.trim().to_string();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }
    if buf.len() < body_start + content_length {
        return None;
    }
    let body = buf[body_start..body_start + content_length].to_vec();
    Some((ParsedReq { method, url, http_version, headers, body }, body_start + content_length))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn build_incoming(req: &ParsedReq) -> Value {
    let headers_obj = with_host(|h| {
        let mut m = IndexMap::new();
        for (k, v) in &req.headers {
            m.insert(k.clone(), h.new_str(v.clone()));
        }
        h.new_object(m)
    });
    let mut extra = IndexMap::new();
    extra.insert("method".into(), with_host(|h| h.new_str(req.method.clone())));
    extra.insert("url".into(), with_host(|h| h.new_str(req.url.clone())));
    extra.insert("httpVersion".into(), with_host(|h| h.new_str(req.http_version.clone())));
    extra.insert("headers".into(), headers_obj);
    // Reuse http's IncomingMessage tag: only its EventEmitter surface is used, and
    // that routes through `http::instance_call` → `events`.
    super::tls::new_emitter_object("IncomingMessage", extra)
}

fn build_response(sock_id: u64) -> Value {
    let resid = next_resid();
    RESPONSES.with(|r| {
        r.borrow_mut().insert(
            resid,
            ResState { sock_id, status: 200, message: None, headers: Vec::new(), body: Vec::new() },
        );
    });
    let mut extra = IndexMap::new();
    extra.insert("@@resid".into(), Value::Float(resid as f64));
    extra.insert("statusCode".into(), Value::Float(200.0));
    super::tls::new_emitter_object("HTTPSServerResponse", extra)
}

// ── instance dispatch ────────────────────────────────────────────────────────

pub fn instance_call(tag: &str, recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if matches!(
        method,
        "on" | "addListener" | "prependListener" | "once" | "prependOnceListener" | "emit"
            | "removeListener" | "off" | "removeAllListeners" | "listenerCount" | "eventNames"
            | "setMaxListeners" | "getMaxListeners" | "listeners"
    ) {
        return super::events::instance_call(recv, method, args);
    }
    match tag {
        "HTTPSServerResponse" => response_call(recv, method, args),
        "HTTPSClientRequest" => client_request_call(recv, method, args),
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

fn resid_of(res: &Value) -> Option<u64> {
    u64_prop(res, "@@resid")
}

fn response_call(res: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    let Some(resid) = resid_of(res) else {
        return Err(crate::host::type_error("invalid ServerResponse"));
    };
    match method {
        "writeHead" => {
            let status = with_host(|h| args.first().map(|v| h.to_number(v)).unwrap_or(200.0)) as u16;
            let mut message: Option<String> = None;
            let mut headers_arg: Option<Value> = None;
            if let Some(a) = args.get(1) {
                if with_host(|h| h.as_str(a)).is_some() {
                    message = Some(with_host(|h| h.str_of(a)));
                } else if !matches!(a, Value::Undef) {
                    headers_arg = Some(a.clone());
                }
            }
            if let Some(a) = args.get(2) {
                if !matches!(a, Value::Undef) {
                    headers_arg = Some(a.clone());
                }
            }
            let header_pairs = headers_arg.map(|h| object_pairs(&h)).unwrap_or_default();
            RESPONSES.with(|r| {
                if let Some(st) = r.borrow_mut().get_mut(&resid) {
                    st.status = status;
                    st.message = message;
                    for (k, v) in header_pairs {
                        upsert_header(&mut st.headers, &k, v);
                    }
                }
            });
            set_prop(res, "statusCode", Value::Float(status as f64));
            Ok(res.clone())
        }
        "setHeader" => {
            let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            let v = with_host(|h| h.str_of(&args.get(1).cloned().unwrap_or(Value::Undef)));
            RESPONSES.with(|r| {
                if let Some(st) = r.borrow_mut().get_mut(&resid) {
                    upsert_header(&mut st.headers, &k, v);
                }
            });
            Ok(Value::Undef)
        }
        "getHeader" => {
            let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef))).to_ascii_lowercase();
            let val = RESPONSES.with(|r| {
                r.borrow()
                    .get(&resid)
                    .and_then(|st| st.headers.iter().find(|(hk, _)| hk.eq_ignore_ascii_case(&k)).map(|(_, v)| v.clone()))
            });
            Ok(val.map(|v| with_host(|h| h.new_str(v))).unwrap_or(Value::Undef))
        }
        "removeHeader" => {
            let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            RESPONSES.with(|r| {
                if let Some(st) = r.borrow_mut().get_mut(&resid) {
                    st.headers.retain(|(hk, _)| !hk.eq_ignore_ascii_case(&k));
                }
            });
            Ok(Value::Undef)
        }
        "flushHeaders" => Ok(Value::Undef),
        "write" => {
            let bytes = value_bytes(args.first());
            RESPONSES.with(|r| {
                if let Some(st) = r.borrow_mut().get_mut(&resid) {
                    st.body.extend_from_slice(&bytes);
                }
            });
            Ok(Value::Bool(true))
        }
        "end" => {
            if let Some(chunk) = args.first().filter(|v| !matches!(v, Value::Undef)) {
                let bytes = value_bytes(Some(chunk));
                RESPONSES.with(|r| {
                    if let Some(st) = r.borrow_mut().get_mut(&resid) {
                        st.body.extend_from_slice(&bytes);
                    }
                });
            }
            finish_response(res, resid)?;
            Ok(res.clone())
        }
        _ => Err(crate::host::type_error(&format!("res.{method} is not a function"))),
    }
}

fn finish_response(res: &Value, resid: u64) -> Result<(), String> {
    let js_status = u64_prop(res, "statusCode").map(|n| n as u16);
    let st = RESPONSES.with(|r| r.borrow_mut().remove(&resid));
    let Some(mut st) = st else { return Ok(()) };
    if let Some(s) = js_status {
        st.status = s;
    }
    let payload = serialize_response(&mut st);
    super::tls::socket_write(st.sock_id, &payload);
    super::tls::socket_end(st.sock_id);
    super::events::instance_call(res, "emit", vec![with_host(|h| h.new_str("finish"))])?;
    Ok(())
}

/// Serialize the HTTP/1.1 response bytes. (Mirror of `http::serialize_response`,
/// except the response always closes the connection — the TLS owner shuts down the
/// write half after `res.end`.)
fn serialize_response(st: &mut ResState) -> Vec<u8> {
    let reason = st.message.clone().unwrap_or_else(|| status_text(st.status).to_string());
    let mut out = format!("HTTP/1.1 {} {}\r\n", st.status, reason).into_bytes();
    let has = |name: &str| st.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
    let chunked = st
        .headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked"));
    for (k, v) in &st.headers {
        out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
    }
    if !chunked && !has("content-length") {
        out.extend_from_slice(format!("Content-Length: {}\r\n", st.body.len()).as_bytes());
    }
    if !has("connection") {
        out.extend_from_slice(b"Connection: close\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&st.body);
    out
}

fn upsert_header(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    if let Some(slot) = headers.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
        slot.1 = value;
    } else {
        headers.push((name.to_string(), value));
    }
}

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

fn status_text(code: u16) -> &'static str {
    for &(c, msg) in super::http::status_table() {
        if c == code {
            return msg;
        }
    }
    "OK"
}

// ── client: https.request / https.get ────────────────────────────────────────

/// State of an in-flight client request (`https.request`/`https.get`) until it is
/// dispatched by `.end()`.
struct ClientReq {
    host: String,
    port: u16,
    servername: String,
    reject_unauthorized: bool,
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// The `req` object (a `HTTPSClientRequest` emitter) for `response` events.
    request: Value,
    sent: bool,
}

/// `https.request(options[, cb])` / `https.get(url|options[, cb])`. Returns a
/// `ClientRequest` (a `HTTPSClientRequest` emitter). `get` auto-sends.
pub fn request(args: &[Value], is_get: bool) -> Result<Value, String> {
    let mut host = "localhost".to_string();
    let mut port: u16 = 443;
    let mut path = "/".to_string();
    let mut method = "GET".to_string();
    let mut servername: Option<String> = None;
    let mut reject_unauthorized = true;
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut cb: Option<Value> = None;

    for a in args {
        if with_host(|h| crate::host::is_callable(h, a)) {
            cb = Some(a.clone());
        } else if with_host(|h| h.as_str(a)).is_some() {
            // A URL string.
            let url = with_host(|h| h.str_of(a));
            parse_url(&url, &mut host, &mut port, &mut path);
        } else if matches!(a, Value::Obj(_)) {
            for key in ["hostname", "host"] {
                if let Some(v) = get_prop(a, key).filter(|v| with_host(|h| h.as_str(v)).is_some()) {
                    host = with_host(|h| h.str_of(&v));
                }
            }
            if let Some(v) = get_prop(a, "port") {
                let n = with_host(|h| h.to_number(&v));
                if !n.is_nan() {
                    port = n as u16;
                }
            }
            if let Some(v) = get_prop(a, "path").filter(|v| with_host(|h| h.as_str(v)).is_some()) {
                path = with_host(|h| h.str_of(&v));
            }
            if let Some(v) = get_prop(a, "method").filter(|v| with_host(|h| h.as_str(v)).is_some()) {
                method = with_host(|h| h.str_of(&v));
            }
            if let Some(v) = get_prop(a, "servername").filter(|v| with_host(|h| h.as_str(v)).is_some()) {
                servername = Some(with_host(|h| h.str_of(&v)));
            }
            if let Some(v) = get_prop(a, "rejectUnauthorized") {
                reject_unauthorized = with_host(|h| h.truthy(&v));
            }
            if let Some(hv) = get_prop(a, "headers").filter(|v| matches!(v, Value::Obj(_))) {
                for (k, val) in object_pairs(&hv) {
                    headers.push((k, val));
                }
            }
        }
    }
    if is_get {
        method = "GET".to_string();
    }
    let servername = servername.unwrap_or_else(|| host.clone());

    let reqid = next_reqid();
    let mut extra = IndexMap::new();
    extra.insert("@@reqid".into(), Value::Float(reqid as f64));
    extra.insert("method".into(), with_host(|h| h.new_str(method.clone())));
    extra.insert("path".into(), with_host(|h| h.new_str(path.clone())));
    let request = super::tls::new_emitter_object("HTTPSClientRequest", extra);
    // The `cb` passed to `request`/`get` is registered as the `response` listener
    // (Node semantics), so it fires exactly once when the response arrives.
    if let Some(cb) = cb {
        super::events::instance_call(&request, "on", vec![with_host(|h| h.new_str("response")), cb])?;
    }
    CLIENT_REQS.with(|c| {
        c.borrow_mut().insert(
            reqid,
            ClientReq {
                host,
                port,
                servername,
                reject_unauthorized,
                method,
                path,
                headers,
                body: Vec::new(),
                request: request.clone(),
                sent: false,
            },
        );
    });
    // `https.get` dispatches immediately; `https.request` waits for `.end()`.
    if is_get {
        dispatch_request(reqid)?;
    }
    Ok(request)
}

fn parse_url(url: &str, host: &mut String, port: &mut u16, path: &mut String) {
    let rest = url.strip_prefix("https://").unwrap_or(url);
    let (authority, p) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    *path = if p.is_empty() { "/".to_string() } else { p.to_string() };
    if let Some((h, port_str)) = authority.rsplit_once(':') {
        *host = h.to_string();
        if let Ok(n) = port_str.parse::<u16>() {
            *port = n;
        }
    } else {
        *host = authority.to_string();
        *port = 443;
    }
}

fn client_request_call(req: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    let reqid = u64_prop(req, "@@reqid");
    match method {
        "write" => {
            if let Some(id) = reqid {
                let bytes = value_bytes(args.first());
                CLIENT_REQS.with(|c| {
                    if let Some(r) = c.borrow_mut().get_mut(&id) {
                        r.body.extend_from_slice(&bytes);
                    }
                });
            }
            Ok(Value::Bool(true))
        }
        "end" => {
            if let Some(id) = reqid {
                if let Some(chunk) = args.first().filter(|v| !matches!(v, Value::Undef)) {
                    let bytes = value_bytes(Some(chunk));
                    CLIENT_REQS.with(|c| {
                        if let Some(r) = c.borrow_mut().get_mut(&id) {
                            r.body.extend_from_slice(&bytes);
                        }
                    });
                }
                dispatch_request(id)?;
            }
            Ok(req.clone())
        }
        "setHeader" => {
            if let Some(id) = reqid {
                let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
                let v = with_host(|h| h.str_of(&args.get(1).cloned().unwrap_or(Value::Undef)));
                CLIENT_REQS.with(|c| {
                    if let Some(r) = c.borrow_mut().get_mut(&id) {
                        upsert_header(&mut r.headers, &k, v);
                    }
                });
            }
            Ok(Value::Undef)
        }
        "getHeader" => {
            let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef))).to_ascii_lowercase();
            let val = reqid.and_then(|id| {
                CLIENT_REQS.with(|c| {
                    c.borrow()
                        .get(&id)
                        .and_then(|r| r.headers.iter().find(|(hk, _)| hk.eq_ignore_ascii_case(&k)).map(|(_, v)| v.clone()))
                })
            });
            Ok(val.map(|v| with_host(|h| h.new_str(v))).unwrap_or(Value::Undef))
        }
        "removeHeader" => {
            if let Some(id) = reqid {
                let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
                CLIENT_REQS.with(|c| {
                    if let Some(r) = c.borrow_mut().get_mut(&id) {
                        r.headers.retain(|(hk, _)| !hk.eq_ignore_ascii_case(&k));
                    }
                });
            }
            Ok(Value::Undef)
        }
        "abort" | "destroy" | "setTimeout" => Ok(req.clone()),
        _ => Err(crate::host::type_error(&format!("req.{method} is not a function"))),
    }
}

/// Spawn the blocking TLS exchange for a client request. Runs on a background
/// thread; posts the parsed response to the main thread.
fn dispatch_request(reqid: u64) -> Result<(), String> {
    // Take the request state out; capture only `Send` data for the thread.
    let sent = CLIENT_REQS.with(|c| c.borrow().get(&reqid).map(|r| r.sent).unwrap_or(true));
    if sent {
        return Ok(());
    }
    CLIENT_REQS.with(|c| {
        if let Some(r) = c.borrow_mut().get_mut(&reqid) {
            r.sent = true;
        }
    });

    let (host, port, servername, reject, method, path, headers, body) = CLIENT_REQS.with(|c| {
        let b = c.borrow();
        let r = b.get(&reqid).unwrap();
        (
            r.host.clone(),
            r.port,
            r.servername.clone(),
            r.reject_unauthorized,
            r.method.clone(),
            r.path.clone(),
            r.headers.clone(),
            r.body.clone(),
        )
    });

    let config = super::tls::client_config(reject);
    let io_tx = with_host(|h| h.io_sender());
    with_host(|h| h.incr_handle());

    // Build the request bytes.
    let mut has_host = false;
    let mut has_len = false;
    let mut header_block = String::new();
    for (k, v) in &headers {
        if k.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        if k.eq_ignore_ascii_case("content-length") {
            has_len = true;
        }
        if k.eq_ignore_ascii_case("connection") {
            continue; // we force `close`
        }
        header_block.push_str(&format!("{k}: {v}\r\n"));
    }
    let host_header = if port == 443 { host.clone() } else { format!("{host}:{port}") };
    let mut request_bytes = format!("{method} {path} HTTP/1.1\r\n");
    if !has_host {
        request_bytes.push_str(&format!("Host: {host_header}\r\n"));
    }
    request_bytes.push_str(&header_block);
    if !has_len && !body.is_empty() {
        request_bytes.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request_bytes.push_str("Connection: close\r\n\r\n");
    let mut wire = request_bytes.into_bytes();
    wire.extend_from_slice(&body);

    std::thread::spawn(move || {
        let result = do_client_exchange(&host, port, &servername, config, &wire);
        match result {
            Ok(raw) => {
                let _ = io_tx.send(Box::new(move || deliver_response(reqid, raw)));
            }
            Err(msg) => {
                let _ = io_tx.send(Box::new(move || deliver_error(reqid, msg)));
            }
        }
    });
    Ok(())
}

/// The blocking TLS round-trip: connect, handshake, write the request, read the
/// full response to EOF. Returns the raw response bytes.
fn do_client_exchange(
    host: &str,
    port: u16,
    servername: &str,
    config: std::sync::Arc<rustls::ClientConfig>,
    request: &[u8],
) -> Result<Vec<u8>, String> {
    let server_name = ServerName::try_from(servername.to_string())
        .map_err(|_| format!("Error: tls: invalid servername '{servername}'"))?;
    let sock = TcpStream::connect((host, port))
        .map_err(|e| format!("Error: connect ECONNREFUSED {host}:{port}: {e}"))?;
    let conn = ClientConnection::new(config, server_name).map_err(|e| format!("Error: tls: {e}"))?;
    let mut stream = StreamOwned::new(conn, sock);
    stream.write_all(request).map_err(|e| format!("Error: https write: {e}"))?;
    stream.flush().map_err(|e| format!("Error: https flush: {e}"))?;
    let mut raw = Vec::new();
    // Read to EOF; a clean TLS close-notify surfaces as `Ok(0)`. A peer that drops
    // the TCP connection without close-notify yields an UnexpectedEof, which for a
    // `Connection: close` response we treat as end-of-body.
    let mut buf = [0u8; 16384];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                if raw.is_empty() {
                    return Err(format!("Error: https read: {e}"));
                }
                break;
            }
        }
    }
    Ok(raw)
}

/// Parse the raw response and emit `response` (with an `IncomingMessage`) then the
/// body `data`/`end`. Runs on the main thread.
fn deliver_response(reqid: u64, raw: Vec<u8>) -> Result<(), String> {
    let entry = CLIENT_REQS.with(|c| c.borrow_mut().remove(&reqid));
    with_host(|h| h.decr_handle());
    let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
    let Some(entry) = entry else { return Ok(()) };

    let (status, message, http_version, headers, body) = parse_response(&raw);

    let headers_obj = with_host(|h| {
        let mut m = IndexMap::new();
        for (k, v) in &headers {
            m.insert(k.clone(), h.new_str(v.clone()));
        }
        h.new_object(m)
    });
    let mut extra = IndexMap::new();
    extra.insert("statusCode".into(), Value::Float(status as f64));
    extra.insert("statusMessage".into(), with_host(|h| h.new_str(message)));
    extra.insert("httpVersion".into(), with_host(|h| h.new_str(http_version)));
    extra.insert("headers".into(), headers_obj);
    let res = super::tls::new_emitter_object("IncomingMessage", extra);

    // Fire `response` (the `cb` from `request`/`get` was registered as a listener).
    super::events::instance_call(&entry.request, "emit", vec![with_host(|h| h.new_str("response")), res.clone()])?;
    if !body.is_empty() {
        let chunk = super::buffer::from_bytes(&body);
        super::events::instance_call(&res, "emit", vec![with_host(|h| h.new_str("data")), chunk])?;
    }
    super::events::instance_call(&res, "emit", vec![with_host(|h| h.new_str("end"))])?;
    Ok(())
}

fn deliver_error(reqid: u64, msg: String) -> Result<(), String> {
    let entry = CLIENT_REQS.with(|c| c.borrow_mut().remove(&reqid));
    with_host(|h| h.decr_handle());
    let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
    if let Some(entry) = entry {
        let err = with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("message".into(), h.new_str(msg.clone()));
            h.new_object(m)
        });
        super::events::instance_call(&entry.request, "emit", vec![with_host(|h| h.new_str("error")), err])?;
    }
    Ok(())
}

/// Parse a raw HTTP/1.1 response into `(status, message, version, headers, body)`.
/// Handles `Transfer-Encoding: chunked` and plain (Content-Length / to-EOF) bodies.
fn parse_response(raw: &[u8]) -> (u16, String, String, Vec<(String, String)>, Vec<u8>) {
    let head_end = find_subslice(raw, b"\r\n\r\n").unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let body_start = (head_end + 4).min(raw.len());
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut sp = status_line.splitn(3, ' ');
    let version = sp.next().unwrap_or("HTTP/1.1").strip_prefix("HTTP/").unwrap_or("1.1").to_string();
    let status = sp.next().and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
    let message = sp.next().unwrap_or("").to_string();

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut chunked = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let name = k.trim().to_ascii_lowercase();
            let value = v.trim().to_string();
            if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            }
            headers.push((name, value));
        }
    }
    let raw_body = &raw[body_start..];
    let body = if chunked { decode_chunked(raw_body) } else { raw_body.to_vec() };
    (status, message, version, headers, body)
}

/// Decode an HTTP/1.1 chunked body (best-effort; stops at the terminating 0-chunk
/// or when the input is exhausted).
fn decode_chunked(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(nl) = find_subslice(data, b"\r\n") {
        let size_line = String::from_utf8_lossy(&data[..nl]);
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let chunk_start = nl + 2;
        let chunk_end = (chunk_start + size).min(data.len());
        out.extend_from_slice(&data[chunk_start..chunk_end]);
        // Advance past the chunk and its trailing CRLF.
        let next = chunk_end + 2;
        if next >= data.len() {
            break;
        }
        data = &data[next..];
    }
    out
}
