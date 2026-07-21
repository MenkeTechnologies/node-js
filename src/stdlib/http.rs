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
use std::io::{Read, Write};
use std::net::TcpStream;

/// `http` module functions routed through `stdlib::call`.
pub const MODULE_METHODS: &[&str] = &[
    "createServer",
    "request",
    "get",
    "validateHeaderName",
    "validateHeaderValue",
    "setMaxIdleHTTPParsers",
    "setGlobalProxyFromEnv",
];

/// Instance methods of a client `ClientRequest` (parent wires the `ClientRequest`
/// tag via `native_tag`/`instance_call`).
pub const CLIENT_REQUEST_METHODS: &[&str] = &[
    "write",
    "end",
    "setHeader",
    "getHeader",
    "removeHeader",
    "abort",
    "destroy",
    "setTimeout",
    "flushHeaders",
];

/// The HTTP request methods Node exposes as `http.METHODS` (router derives its
/// per-verb helpers by lowercasing this list).
const METHODS: &[&str] = &[
    "ACL",
    "BIND",
    "CHECKOUT",
    "CONNECT",
    "COPY",
    "DELETE",
    "GET",
    "HEAD",
    "LINK",
    "LOCK",
    "M-SEARCH",
    "MERGE",
    "MKACTIVITY",
    "MKCALENDAR",
    "MKCOL",
    "MOVE",
    "NOTIFY",
    "OPTIONS",
    "PATCH",
    "POST",
    "PROPFIND",
    "PROPPATCH",
    "PURGE",
    "PUT",
    "QUERY",
    "REBIND",
    "REPORT",
    "SEARCH",
    "SOURCE",
    "SUBSCRIBE",
    "TRACE",
    "UNBIND",
    "UNLINK",
    "UNLOCK",
    "UNSUBSCRIBE",
];

/// Non-function `http` module constants (`http.METHODS`, `http.STATUS_CODES`),
/// reachable via `namespace_property` → `stdlib::constant`.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "METHODS" => Some(with_host(|h| {
            let items = METHODS.iter().map(|m| h.new_str(*m)).collect();
            h.new_array(items)
        })),
        "STATUS_CODES" => Some(with_host(|h| {
            let mut m = IndexMap::new();
            for (code, msg) in crate::stdlib::http::status_table() {
                m.insert(code.to_string(), h.new_str(*msg));
            }
            h.new_object(m)
        })),
        // The request/response constructors express augments
        // (`Object.create(http.IncomingMessage.prototype)`): exposed as builtin
        // ctor namespaces so `.prototype` resolves.
        "IncomingMessage" => Some(with_host(|h| {
            h.alloc(JsObj::Builtin("IncomingMessage".into()))
        })),
        "ServerResponse" => Some(with_host(|h| {
            h.alloc(JsObj::Builtin("ServerResponse".into()))
        })),
        // Client/server constructors exposed as builtin ctor namespaces so
        // `.prototype` resolves and `new http.X(...)` routes to `http::construct`.
        // `http.Server` uses a distinct builtin name to disambiguate from
        // `net.Server` in the shared `stdlib::construct`.
        "Agent" => Some(with_host(|h| h.alloc(JsObj::Builtin("Agent".into())))),
        "Server" => Some(with_host(|h| h.alloc(JsObj::Builtin("http.Server".into())))),
        "ClientRequest" => Some(with_host(|h| {
            h.alloc(JsObj::Builtin("ClientRequest".into()))
        })),
        "OutgoingMessage" => Some(with_host(|h| {
            h.alloc(JsObj::Builtin("OutgoingMessage".into()))
        })),
        "globalAgent" => Some(construct_agent(&[])),
        _ => None,
    }
}

/// `stdlib::construct` entry for `http` classes. Parent wires this in.
pub fn construct(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match name {
        "Agent" => Some(Ok(construct_agent(args))),
        // `new http.Server([requestListener])`.
        "http.Server" => Some(Ok(create_server(
            args.first()
                .cloned()
                .filter(|v| with_host(|h| crate::host::is_callable(h, v))),
        ))),
        _ => None,
    }
}

/// The status-code → reason-phrase table shared by `STATUS_CODES` and response
/// serialization.
pub fn status_table() -> &'static [(u16, &'static str)] {
    &[
        (200, "OK"),
        (201, "Created"),
        (202, "Accepted"),
        (204, "No Content"),
        (301, "Moved Permanently"),
        (302, "Found"),
        (303, "See Other"),
        (304, "Not Modified"),
        (307, "Temporary Redirect"),
        (308, "Permanent Redirect"),
        (400, "Bad Request"),
        (401, "Unauthorized"),
        (403, "Forbidden"),
        (404, "Not Found"),
        (405, "Method Not Allowed"),
        (406, "Not Acceptable"),
        (409, "Conflict"),
        (410, "Gone"),
        (411, "Length Required"),
        (413, "Payload Too Large"),
        (414, "URI Too Long"),
        (415, "Unsupported Media Type"),
        (422, "Unprocessable Entity"),
        (429, "Too Many Requests"),
        (500, "Internal Server Error"),
        (501, "Not Implemented"),
        (502, "Bad Gateway"),
        (503, "Service Unavailable"),
        (504, "Gateway Timeout"),
    ]
}

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
        "request" => Some(request(args, false)),
        "get" => Some(request(args, true)),
        "validateHeaderName" => Some(validate_header_name(args)),
        "validateHeaderValue" => Some(validate_header_value(args)),
        // No pooling/proxy substrate: accepted no-ops (match Node's `undefined`).
        "setMaxIdleHTTPParsers" | "setGlobalProxyFromEnv" => Some(Ok(Value::Undef)),
        _ => None,
    }
}

/// `http.validateHeaderName(name[, label])` — throws `ERR_INVALID_HTTP_TOKEN`
/// unless `name` is a valid RFC 7230 field-name token, else returns `undefined`.
fn validate_header_name(args: &[Value]) -> Result<Value, String> {
    let name = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
    if is_valid_token(&name) {
        Ok(Value::Undef)
    } else {
        Err(crate::host::type_error(&format!(
            "Header name must be a valid HTTP token [\"{name}\"]"
        )))
    }
}

/// `http.validateHeaderValue(name, value)` — throws if `value` is `undefined`
/// (`ERR_HTTP_INVALID_HEADER_VALUE`) or contains an invalid character
/// (`ERR_INVALID_CHAR`), else returns `undefined`.
fn validate_header_value(args: &[Value]) -> Result<Value, String> {
    let name = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
    let raw = args.get(1).cloned().unwrap_or(Value::Undef);
    if matches!(raw, Value::Undef) {
        return Err(crate::host::type_error(&format!(
            "Invalid value \"undefined\" for header \"{name}\""
        )));
    }
    let value = with_host(|h| h.str_of(&raw));
    if value.bytes().any(|b| b != b'\t' && (b < 0x20 || b == 0x7f)) {
        return Err(crate::host::type_error(&format!(
            "Invalid character in header content [\"{name}\"]"
        )));
    }
    Ok(Value::Undef)
}

/// RFC 7230 field-name token: one or more of the `tchar` set.
fn is_valid_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// `new http.Agent([options])` — a minimal connection-pool holder. We open a
/// fresh connection per request (no reuse), so the agent only carries its
/// configuration for the JS side to read back.
pub fn construct_agent(args: &[Value]) -> Value {
    let pairs = args
        .first()
        .filter(|v| matches!(v, Value::Obj(_)))
        .map(object_pairs)
        .unwrap_or_default();
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Agent"));
        m.insert("maxSockets".into(), Value::Float(f64::INFINITY));
        m.insert("maxFreeSockets".into(), Value::Float(256.0));
        m.insert("sockets".into(), h.new_object(IndexMap::new()));
        m.insert("requests".into(), h.new_object(IndexMap::new()));
        for (k, v) in pairs {
            m.insert(k, h.new_str(v));
        }
        h.new_object(m)
    })
}

/// Build an HTTP server: a `net.Server` whose per-connection hook wires up the
/// request parser and remembers the `requestListener`.
pub fn create_server(request_listener: Option<Value>) -> Value {
    let server = super::net::create_server(None);
    let listener = request_listener.unwrap_or(Value::Undef);
    let hook = std::rc::Rc::new(
        move |_server: &Value, socket: &Value| -> Result<(), String> {
            let sock_id = socket_id_of(socket);
            CONNS.with(|c| {
                c.borrow_mut().insert(
                    sock_id,
                    HttpConn {
                        socket: socket.clone(),
                        listener: listener.clone(),
                        buf: Vec::new(),
                    },
                );
            });
            Ok(())
        },
    );
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
    CONNS.with(|c| {
        c.borrow_mut()
            .get_mut(&sock_id)
            .unwrap()
            .buf
            .extend_from_slice(bytes)
    });

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
            super::events::instance_call(
                &req,
                "emit",
                vec![with_host(|h| h.new_str("data")), chunk],
            )?;
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
        ParsedReq {
            method,
            url,
            http_version,
            headers,
            body,
        },
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
    extra.insert(
        "method".into(),
        with_host(|h| h.new_str(req.method.clone())),
    );
    extra.insert("url".into(), with_host(|h| h.new_str(req.url.clone())));
    extra.insert(
        "httpVersion".into(),
        with_host(|h| h.new_str(req.http_version.clone())),
    );
    extra.insert("headers".into(), headers_obj);
    super::net::new_emitter_object("IncomingMessage", extra)
}

// ── ServerResponse (res) ─────────────────────────────────────────────────────

fn build_response(sock_id: u64) -> Value {
    let resid = next_resid();
    RESPONSES.with(|r| {
        r.borrow_mut().insert(
            resid,
            ResState {
                sock_id,
                status: 200,
                message: None,
                headers: Vec::new(),
                body: Vec::new(),
            },
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
pub fn instance_call(
    tag: &str,
    recv: &Value,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    // EventEmitter surface is shared with `net` sockets.
    if matches!(
        method,
        "on" | "addListener"
            | "prependListener"
            | "once"
            | "prependOnceListener"
            | "emit"
            | "removeListener"
            | "off"
            | "removeAllListeners"
            | "listenerCount"
            | "eventNames"
    ) {
        return super::events::instance_call(recv, method, args);
    }
    match tag {
        "IncomingMessage" => Err(crate::host::type_error(&format!(
            "req.{method} is not a function"
        ))),
        "ServerResponse" => response_call(recv, method, args),
        "ClientRequest" => client_request_call(recv, method, args),
        // Minimal Agent: no live sockets to tear down.
        "Agent" => match method {
            "destroy" => Ok(Value::Undef),
            "getName" => Ok(with_host(|h| h.new_str("localhost:80:"))),
            _ => Err(crate::host::type_error(&format!(
                "agent.{method} is not a function"
            ))),
        },
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

fn response_call(res: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    let Some(resid) = resid_of(res) else {
        return Err(crate::host::type_error("invalid ServerResponse"));
    };
    match method {
        "writeHead" => {
            let status =
                with_host(|h| args.first().map(|v| h.to_number(v)).unwrap_or(200.0)) as u16;
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
            let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)))
                .to_ascii_lowercase();
            let val = RESPONSES.with(|r| {
                r.borrow().get(&resid).and_then(|st| {
                    st.headers
                        .iter()
                        .find(|(hk, _)| hk.eq_ignore_ascii_case(&k))
                        .map(|(_, v)| v.clone())
                })
            });
            Ok(val
                .map(|v| with_host(|h| h.new_str(v)))
                .unwrap_or(Value::Undef))
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
        _ => Err(crate::host::type_error(&format!(
            "res.{method} is not a function"
        ))),
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
    let reason = st
        .message
        .clone()
        .unwrap_or_else(|| status_text(st.status).to_string());
    let mut out = format!("HTTP/1.1 {} {}\r\n", st.status, reason).into_bytes();

    let has = |name: &str| st.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
    let chunked = st.headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
    });

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
    if let Some(slot) = headers
        .iter_mut()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
    {
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

// ── client: http.request / http.get ──────────────────────────────────────────
//
// Mirrors the `https` client (https.rs), but over a plain `TcpStream` on port 80.
// `.end()` (or `http.get` immediately) spawns a background thread that connects,
// writes the request, reads the full response to EOF (the request forces
// `Connection: close`), parses it, and posts an `IoTask` that emits `response`
// with an `IncomingMessage` (then its body `data`/`end`) on the main thread.

/// State of an in-flight client request until `.end()` dispatches it.
struct ClientReq {
    host: String,
    port: u16,
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// The `req` object (a `ClientRequest` emitter) for `response`/`error` events.
    request: Value,
    sent: bool,
}

thread_local! {
    static CLIENT_REQS: std::cell::RefCell<HashMap<u64, ClientReq>> =
        std::cell::RefCell::new(HashMap::new());
    static NEXT_REQID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn next_reqid() -> u64 {
    NEXT_REQID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}

fn get_prop(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

fn u64_prop(recv: &Value, key: &str) -> Option<u64> {
    get_prop(recv, key).map(|v| with_host(|h| h.to_number(&v)) as u64)
}

/// `http.request(options|url[, cb])` / `http.get(url|options[, cb])`. Returns a
/// `ClientRequest` (a `ClientRequest` emitter). `get` auto-sends via `.end()`.
pub fn request(args: &[Value], is_get: bool) -> Result<Value, String> {
    let mut host = "localhost".to_string();
    let mut port: u16 = 80;
    let mut path = "/".to_string();
    let mut method = "GET".to_string();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut cb: Option<Value> = None;

    for a in args {
        if with_host(|h| crate::host::is_callable(h, a)) {
            cb = Some(a.clone());
        } else if with_host(|h| h.as_str(a)).is_some() {
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
            if let Some(v) = get_prop(a, "method").filter(|v| with_host(|h| h.as_str(v)).is_some())
            {
                method = with_host(|h| h.str_of(&v));
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

    let reqid = next_reqid();
    let mut extra = IndexMap::new();
    extra.insert("@@reqid".into(), Value::Float(reqid as f64));
    extra.insert("method".into(), with_host(|h| h.new_str(method.clone())));
    extra.insert("path".into(), with_host(|h| h.new_str(path.clone())));
    let request = super::net::new_emitter_object("ClientRequest", extra);
    // The `cb` is registered as the `response` listener (Node semantics).
    if let Some(cb) = cb {
        super::events::instance_call(
            &request,
            "on",
            vec![with_host(|h| h.new_str("response")), cb],
        )?;
    }
    CLIENT_REQS.with(|c| {
        c.borrow_mut().insert(
            reqid,
            ClientReq {
                host,
                port,
                method,
                path,
                headers,
                body: Vec::new(),
                request: request.clone(),
                sent: false,
            },
        );
    });
    if is_get {
        dispatch_request(reqid)?;
    }
    Ok(request)
}

/// Parse an `http://host[:port][/path]` URL into its parts (defaults port 80).
fn parse_url(url: &str, host: &mut String, port: &mut u16, path: &mut String) {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (authority, p) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    *path = if p.is_empty() {
        "/".to_string()
    } else {
        p.to_string()
    };
    if let Some((h, port_str)) = authority.rsplit_once(':') {
        *host = h.to_string();
        if let Ok(n) = port_str.parse::<u16>() {
            *port = n;
        }
    } else {
        *host = authority.to_string();
        *port = 80;
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
            let k = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)))
                .to_ascii_lowercase();
            let val = reqid.and_then(|id| {
                CLIENT_REQS.with(|c| {
                    c.borrow().get(&id).and_then(|r| {
                        r.headers
                            .iter()
                            .find(|(hk, _)| hk.eq_ignore_ascii_case(&k))
                            .map(|(_, v)| v.clone())
                    })
                })
            });
            Ok(val
                .map(|v| with_host(|h| h.new_str(v)))
                .unwrap_or(Value::Undef))
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
        "abort" | "destroy" | "setTimeout" | "flushHeaders" => Ok(req.clone()),
        _ => Err(crate::host::type_error(&format!(
            "req.{method} is not a function"
        ))),
    }
}

/// Spawn the blocking HTTP exchange for a client request on a background thread;
/// the parsed response is posted back to the main thread.
fn dispatch_request(reqid: u64) -> Result<(), String> {
    let sent = CLIENT_REQS.with(|c| c.borrow().get(&reqid).map(|r| r.sent).unwrap_or(true));
    if sent {
        return Ok(());
    }
    CLIENT_REQS.with(|c| {
        if let Some(r) = c.borrow_mut().get_mut(&reqid) {
            r.sent = true;
        }
    });

    let (host, port, method, path, headers, body) = CLIENT_REQS.with(|c| {
        let b = c.borrow();
        let r = b.get(&reqid).unwrap();
        (
            r.host.clone(),
            r.port,
            r.method.clone(),
            r.path.clone(),
            r.headers.clone(),
            r.body.clone(),
        )
    });

    let io_tx = with_host(|h| h.io_sender());
    with_host(|h| h.incr_handle());

    // Build the request bytes (force `Connection: close` for read-to-EOF).
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
            continue;
        }
        header_block.push_str(&format!("{k}: {v}\r\n"));
    }
    let host_header = if port == 80 {
        host.clone()
    } else {
        format!("{host}:{port}")
    };
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

    std::thread::spawn(move || match do_client_exchange(&host, port, &wire) {
        Ok(raw) => {
            let _ = io_tx.send(Box::new(move || deliver_response(reqid, raw)));
        }
        Err(msg) => {
            let _ = io_tx.send(Box::new(move || deliver_error(reqid, msg)));
        }
    });
    Ok(())
}

/// The blocking TCP round-trip: connect, write the request, read to EOF.
fn do_client_exchange(host: &str, port: u16, request: &[u8]) -> Result<Vec<u8>, String> {
    let mut stream = TcpStream::connect((host, port))
        .map_err(|e| format!("Error: connect ECONNREFUSED {host}:{port}: {e}"))?;
    stream
        .write_all(request)
        .map_err(|e| format!("Error: http write: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("Error: http flush: {e}"))?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 16384];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                if raw.is_empty() {
                    return Err(format!("Error: http read: {e}"));
                }
                break;
            }
        }
    }
    Ok(raw)
}

/// Parse the raw response and emit `response` (with an `IncomingMessage`), then
/// the body `data`/`end`. Runs on the main thread.
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
    let res = super::net::new_emitter_object("IncomingMessage", extra);

    super::events::instance_call(
        &entry.request,
        "emit",
        vec![with_host(|h| h.new_str("response")), res.clone()],
    )?;
    if !body.is_empty() {
        let chunk = super::buffer::from_bytes(&body);
        super::events::instance_call(&res, "emit", vec![with_host(|h| h.new_str("data")), chunk])?;
    }
    super::events::instance_call(&res, "emit", vec![with_host(|h| h.new_str("end"))])?;
    Ok(())
}

/// Emit `error` on the request when the exchange fails. Runs on the main thread.
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
        super::events::instance_call(
            &entry.request,
            "emit",
            vec![with_host(|h| h.new_str("error")), err],
        )?;
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
    let version = sp
        .next()
        .unwrap_or("HTTP/1.1")
        .strip_prefix("HTTP/")
        .unwrap_or("1.1")
        .to_string();
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
    let body = if chunked {
        decode_chunked(raw_body)
    } else {
        raw_body.to_vec()
    };
    (status, message, version, headers, body)
}

/// Decode an HTTP/1.1 chunked body (best-effort; stops at the terminating
/// 0-chunk or when the input is exhausted).
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
        let next = chunk_end + 2;
        if next >= data.len() {
            break;
        }
        data = &data[next..];
    }
    out
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
