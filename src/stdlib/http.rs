//! Node `http` module: an HTTP/1.1 server built on top of `net`.
//!
//! `http.createServer(requestListener)` returns a `net.Server` with an
//! `http`-specific per-connection hook. As bytes arrive on a connection (posted
//! by the socket reader thread and dispatched on the main thread by
//! `net::on_socket_data` → `http::feed`), we buffer them, parse complete
//! HTTP/1.1 requests, and for each one build an `IncomingMessage` (`req`) and a
//! `ServerResponse` (`res`) and call the user's `(req, res)` listener on the main
//! thread. `res.end` serializes a valid HTTP/1.1 response and writes it straight
//! back to the socket via `net::socket_write_id`, keeping the connection alive.

use crate::host::{invoke, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::collections::HashMap;

/// `http` module functions routed through `stdlib::call`.
pub const MODULE_METHODS: &[&str] = &["createServer"];

// ── per-connection parse state ───────────────────────────────────────────────

/// Main-thread state for one live HTTP connection, keyed by the `net` socket id.
struct HttpConn {
    /// The underlying `net.Socket` object.
    socket: Value,
    /// The user's `requestListener` (`(req, res) => …`).
    listener: Value,
    /// Bytes received but not yet consumed into a complete request.
    buf: Vec<u8>,
}

/// Main-thread state for one in-flight response, keyed by a fresh response id
/// (stored on the `res` object as `@@resid`).
struct ResState {
    /// Socket id to write the serialized response to.
    sock_id: u64,
    /// Status code (`writeHead`/`res.statusCode`).
    status: u16,
    /// Optional custom status message.
    message: Option<String>,
    /// Response headers in insertion order; name kept as given, matched
    /// case-insensitively.
    headers: Vec<(String, String)>,
    /// Accumulated body bytes (`write`/`end`).
    body: Vec<u8>,
}

thread_local! {
    static CONNS: std::cell::RefCell<HashMap<u64, HttpConn>> =
        std::cell::RefCell::new(HashMap::new());
    static RESPONSES: std::cell::RefCell<HashMap<u64, ResState>> =
        std::cell::RefCell::new(HashMap::new());
    static NEXT_RESID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn next_resid() -> u64 {
    NEXT_RESID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}

// ── module: http.createServer ────────────────────────────────────────────────

/// `stdlib::call` entry for `http.<method>`.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "createServer" => Some(Ok(create_server(args.first().cloned()))),
        _ => None,
    }
}

/// Build an HTTP server: a `net.Server` whose per-connection hook wires up the
/// request parser and remembers the `requestListener`.
pub fn create_server(request_listener: Option<Value>) -> Value {
    let server = super::net::create_server(None);
    let listener = request_listener.unwrap_or(Value::Undef);
    let hook = std::rc::Rc::new(move |_server: &Value, socket: &Value| -> Result<(), String> {
        let sock_id = socket_id_of(socket);
        CONNS.with(|c| {
            c.borrow_mut().insert(
                sock_id,
                HttpConn { socket: socket.clone(), listener: listener.clone(), buf: Vec::new() },
            );
        });
        Ok(())
    });
    super::net::set_conn_hook(&server, hook);
    server
}

fn socket_id_of(socket: &Value) -> u64 {
    with_host(|h| match h.get(socket) {
        Some(JsObj::Object(p)) => p.get("@@netid").map(|v| h.to_number(v) as u64).unwrap_or(0),
        _ => 0,
    })
}

/// Discard an HTTP connection when its socket closes.
pub fn drop_conn(sock_id: u64) {
    CONNS.with(|c| {
        c.borrow_mut().remove(&sock_id);
    });
}

// ── request parsing ──────────────────────────────────────────────────────────

/// Feed freshly received socket bytes into the HTTP parser. No-op for a socket
/// that is not an HTTP connection (plain `net`). Runs on the main thread.
pub fn feed(sock_id: u64, _socket: &Value, bytes: &[u8]) -> Result<(), String> {
    let is_http = CONNS.with(|c| c.borrow().contains_key(&sock_id));
    if !is_http {
        return Ok(());
    }
    CONNS.with(|c| c.borrow_mut().get_mut(&sock_id).unwrap().buf.extend_from_slice(bytes));

    // Parse and dispatch every complete request currently buffered (pipelining).
    loop {
        let (listener, socket, parsed) = CONNS.with(|c| {
            let mut c = c.borrow_mut();
            let conn = c.get_mut(&sock_id).unwrap();
            match parse_request(&conn.buf) {
                Some((req, consumed)) => {
                    conn.buf.drain(..consumed);
                    (conn.listener.clone(), conn.socket.clone(), Some(req))
                }
                None => (Value::Undef, Value::Undef, None),
            }
        });
        let Some(parsed) = parsed else { break };

        let req = build_incoming(&parsed);
        let res = build_response(sock_id);
        if with_host(|h| crate::host::is_callable(h, &listener)) {
            invoke(&listener, vec![req.clone(), res], None)?;
        }
        // Body streaming: emit any request body then `end` for downstream readers.
        if !parsed.body.is_empty() {
            let chunk = super::buffer::from_bytes(&parsed.body);
            super::events::instance_call(&req, "emit", vec![with_host(|h| h.new_str("data")), chunk])?;
        }
        super::events::instance_call(&req, "emit", vec![with_host(|h| h.new_str("end"))])?;
        let _ = socket; // (kept for future keep-alive bookkeeping)
    }
    Ok(())
}

/// A fully parsed HTTP request.
struct ParsedReq {
    method: String,
    url: String,
    http_version: String,
    /// Lowercased header name → value (last wins, like Node for simple headers).
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Try to parse one complete request from `buf`. Returns the request and the
/// number of bytes consumed, or `None` if more bytes are needed.
fn parse_request(buf: &[u8]) -> Option<(ParsedReq, usize)> {
    // Header block ends at the first CRLFCRLF.
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

    // Need the full body before dispatching.
    if buf.len() < body_start + content_length {
        return None;
    }
    let body = buf[body_start..body_start + content_length].to_vec();
    Some((
        ParsedReq { method, url, http_version, headers, body },
        body_start + content_length,
    ))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

// ── IncomingMessage (req) ────────────────────────────────────────────────────

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
    super::net::new_emitter_object("IncomingMessage", extra)
}

// ── ServerResponse (res) ─────────────────────────────────────────────────────

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
    super::net::new_emitter_object("ServerResponse", extra)
}

fn resid_of(res: &Value) -> Option<u64> {
    with_host(|h| match h.get(res) {
        Some(JsObj::Object(p)) => p.get("@@resid").map(|v| h.to_number(v) as u64),
        _ => None,
    })
}

/// Instance dispatch for `IncomingMessage`/`ServerResponse` (EventEmitter methods
/// handled by `net`'s shared emitter delegation via `stdlib::instance_call`).
pub fn instance_call(tag: &str, recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    // EventEmitter surface is shared with `net` sockets.
    if matches!(
        method,
        "on" | "addListener" | "prependListener" | "once" | "prependOnceListener" | "emit"
            | "removeListener" | "off" | "removeAllListeners" | "listenerCount" | "eventNames"
    ) {
        return super::events::instance_call(recv, method, args);
    }
    match tag {
        "IncomingMessage" => Err(crate::host::type_error(&format!("req.{method} is not a function"))),
        "ServerResponse" => response_call(recv, method, args),
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

fn response_call(res: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    let Some(resid) = resid_of(res) else {
        return Err(crate::host::type_error("invalid ServerResponse"));
    };
    match method {
        "writeHead" => {
            let status = with_host(|h| args.first().map(|v| h.to_number(v)).unwrap_or(200.0)) as u16;
            // Second arg is either the status message (string) or the headers obj.
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
            // Mirror onto the JS prop so `res.statusCode` reads reflect it.
            set_res_prop(res, "statusCode", Value::Float(status as f64));
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

/// Serialize and write the response, then emit `finish`.
fn finish_response(res: &Value, resid: u64) -> Result<(), String> {
    // Reconcile status with a possible `res.statusCode = n` assignment.
    let js_status = with_host(|h| match h.get(res) {
        Some(JsObj::Object(p)) => p.get("statusCode").map(|v| h.to_number(v) as u16),
        _ => None,
    });
    let st = RESPONSES.with(|r| r.borrow_mut().remove(&resid));
    let Some(mut st) = st else { return Ok(()) };
    if let Some(s) = js_status {
        st.status = s;
    }
    let payload = serialize_response(&mut st);
    super::net::socket_write_id(st.sock_id, &payload);
    super::events::instance_call(res, "emit", vec![with_host(|h| h.new_str("finish"))])?;
    Ok(())
}

/// Build the raw HTTP/1.1 response bytes: status line, headers (adding
/// `Content-Length` and `Connection: keep-alive` if the user did not set framing
/// headers), CRLFCRLF, then the body.
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
        out.extend_from_slice(b"Connection: keep-alive\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&st.body);
    out
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Insert or replace a header (case-insensitive name match), preserving order.
fn upsert_header(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    if let Some(slot) = headers.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
        slot.1 = value;
    } else {
        headers.push((name.to_string(), value));
    }
}

/// Enumerable string key/value pairs of a plain object (for the `writeHead`
/// headers argument).
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

/// Set a JS-visible prop on the response object.
fn set_res_prop(res: &Value, key: &str, val: Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(res) {
            p.insert(key.to_string(), val);
        }
    });
}

/// Raw bytes of a `write`/`end` argument: a Buffer's bytes, or a string's UTF-8.
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

/// Standard reason phrase for common status codes (default `OK`).
fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "OK",
    }
}
