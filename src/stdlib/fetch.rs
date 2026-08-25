//! The WHATWG Fetch globals: `fetch`, `Headers`, `Request`, `Response`, `Blob`,
//! `FormData`, `AbortController` and `AbortSignal`.
//!
//! `fetch` is not a re-implementation of an HTTP client: it drives the SAME
//! exchange `http.request`/`https.request` already perform (`http::exchange` /
//! `https::exchange`), on a background thread, and hands the parsed result back
//! over the host's I/O channel — the pattern every other async native in this
//! frontend uses. That keeps one HTTP/1.1 wire implementation, one chunked
//! decoder and one TLS configuration in the process.
//!
//! A body is held as raw BYTES (`@@body`, a JS array of byte numbers) rather
//! than as a string, so `arrayBuffer()`/`bytes()` are exact for a binary
//! response and `text()` decodes UTF-8 once, at the point the caller asks for
//! text. A `Response` is fully buffered by the time the promise settles, so the
//! body accessors are already-settled promises — the streaming `body`
//! `ReadableStream` is the one part of the interface this does not provide.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// The classes this module constructs, as bare globals.
pub const CLASSES: &[&str] = &[
    "Headers",
    "Request",
    "Response",
    "Blob",
    "File",
    "FormData",
    "AbortController",
    "AbortSignal",
];

pub const HEADERS_METHODS: &[&str] = &[
    "get",
    "getSetCookie",
    "has",
    "set",
    "append",
    "delete",
    "forEach",
    "keys",
    "values",
    "entries",
];
pub const RESPONSE_METHODS: &[&str] = &[
    "text",
    "json",
    "arrayBuffer",
    "bytes",
    "blob",
    "formData",
    "clone",
];
pub const REQUEST_METHODS: &[&str] = &[
    "text",
    "json",
    "arrayBuffer",
    "bytes",
    "blob",
    "formData",
    "clone",
];
pub const BLOB_METHODS: &[&str] = &["text", "arrayBuffer", "bytes", "slice", "stream"];
pub const FORM_DATA_METHODS: &[&str] = &[
    "append", "delete", "get", "getAll", "has", "set", "forEach", "keys", "values", "entries",
];
pub const ABORT_CONTROLLER_METHODS: &[&str] = &["abort"];
pub const ABORT_SIGNAL_METHODS: &[&str] =
    &["throwIfAborted", "addEventListener", "removeEventListener"];

pub fn methods_for(tag: &str) -> &'static [&'static str] {
    match tag {
        "Headers" => HEADERS_METHODS,
        "Response" => RESPONSE_METHODS,
        "Request" => REQUEST_METHODS,
        "Blob" | "File" => BLOB_METHODS,
        "FormData" => FORM_DATA_METHODS,
        "AbortController" => ABORT_CONTROLLER_METHODS,
        "AbortSignal" => ABORT_SIGNAL_METHODS,
        _ => &[],
    }
}

pub fn is_class(name: &str) -> bool {
    CLASSES.contains(&name)
}

// ── small helpers ────────────────────────────────────────────────────────────

fn str_of(v: &Value) -> String {
    with_host(|h| h.str_of(v))
}

fn new_str(s: impl Into<String>) -> Value {
    let s = s.into();
    with_host(|h| h.new_str(s))
}

fn prop(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
    .filter(|v| !matches!(v, Value::Undef))
}

fn set_prop(recv: &Value, key: &str, val: Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), val);
        }
    });
}

/// Wrap a synchronous outcome in an already-settled Promise. Same shape as
/// `fs/promises`: the body of a fetched `Response` is already in memory, so
/// `text()`/`json()` have nothing to wait for.
fn settled(result: Result<Value, String>) -> Value {
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p).unwrap_or(0));
    match result {
        Ok(v) => crate::host::resolve_promise_val(id, v),
        Err(e) => {
            let ev = with_host(|h| crate::builtins::synth_error(h, &e));
            crate::host::reject_promise_val(id, ev);
        }
    }
    p
}

/// The bytes stored under `@@body`, as a Rust byte vector.
fn body_bytes(recv: &Value) -> Vec<u8> {
    let Some(arr) = prop(recv, "@@body") else {
        return Vec::new();
    };
    with_host(|h| match h.get(&arr) {
        Some(JsObj::Array(items)) => items.iter().map(|v| h.to_number(v) as u8).collect(),
        _ => Vec::new(),
    })
}

fn store_body(bytes: &[u8]) -> Value {
    with_host(|h| h.new_array(bytes.iter().map(|b| Value::Float(*b as f64)).collect()))
}

/// A `Uint8Array` over `bytes` — what `bytes()` answers and what
/// `arrayBuffer()` is built from.
fn to_uint8array(bytes: &[u8]) -> Result<Value, String> {
    let arr = with_host(|h| h.new_array(bytes.iter().map(|b| Value::Float(*b as f64)).collect()));
    super::typedarray::construct("Uint8Array", &[arr])
}

// ── Headers ──────────────────────────────────────────────────────────────────
//
// The entry list is kept in insertion order with LOWERCASED names, which is what
// the Fetch standard's header list is and what makes `get`/`has` case-
// insensitive without a second index.

fn headers_entries(recv: &Value) -> Vec<(String, String)> {
    let Some(arr) = prop(recv, "@@entries") else {
        return Vec::new();
    };
    with_host(|h| match h.get(&arr) {
        Some(JsObj::Array(items)) => items
            .iter()
            .filter_map(|pair| match h.get(pair) {
                Some(JsObj::Array(kv)) if kv.len() == 2 => {
                    Some((h.str_of(&kv[0]), h.str_of(&kv[1])))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    })
}

fn set_headers_entries(recv: &Value, entries: &[(String, String)]) {
    let arr = with_host(|h| {
        let pairs: Vec<Value> = entries
            .iter()
            .map(|(k, v)| {
                let k = h.new_str(k.clone());
                let v = h.new_str(v.clone());
                h.new_array(vec![k, v])
            })
            .collect();
        h.new_array(pairs)
    });
    set_prop(recv, "@@entries", arr);
}

/// `new Headers(init)`. `init` is another `Headers`, an array of `[name, value]`
/// pairs, or a plain object.
pub fn construct_headers(args: &[Value]) -> Result<Value, String> {
    let obj = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Headers"));
        let empty = h.new_array(Vec::new());
        m.insert("@@entries".into(), empty);
        h.new_object(m)
    });
    with_host(|h| h.hide_prop(&obj, "@@entries"));
    if let Some(init) = args.first().filter(|v| !matches!(v, Value::Undef)) {
        for (k, v) in init_header_entries(init) {
            append_header(&obj, &k, &v);
        }
    }
    Ok(obj)
}

/// The `(name, value)` pairs a `HeadersInit` contributes, in order.
fn init_header_entries(init: &Value) -> Vec<(String, String)> {
    if super::native_tag(init).as_deref() == Some("Headers") {
        return headers_entries(init);
    }
    // An array of `[name, value]` pairs.
    if with_host(|h| matches!(h.get(init), Some(JsObj::Array(_)))) {
        return with_host(|h| match h.get(init) {
            Some(JsObj::Array(items)) => items
                .iter()
                .filter_map(|pair| match h.get(pair) {
                    Some(JsObj::Array(kv)) if kv.len() >= 2 => {
                        Some((h.str_of(&kv[0]).to_ascii_lowercase(), h.str_of(&kv[1])))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        });
    }
    // A plain object: every own enumerable string key.
    with_host(|h| match h.get(init) {
        Some(JsObj::Object(p)) => p
            .iter()
            .filter(|(k, _)| !k.starts_with("@@"))
            .map(|(k, v)| (k.to_ascii_lowercase(), h.str_of(v)))
            .collect(),
        _ => Vec::new(),
    })
}

fn append_header(recv: &Value, name: &str, value: &str) {
    let mut e = headers_entries(recv);
    e.push((name.to_ascii_lowercase(), value.to_string()));
    set_headers_entries(recv, &e);
}

fn set_header(recv: &Value, name: &str, value: &str) {
    let name = name.to_ascii_lowercase();
    let mut e = headers_entries(recv);
    match e.iter().position(|(k, _)| *k == name) {
        Some(i) => {
            e[i].1 = value.to_string();
            e.retain({
                let mut seen = 0;
                move |(k, _)| {
                    if *k != name {
                        return true;
                    }
                    seen += 1;
                    seen == 1
                }
            });
        }
        None => e.push((name, value.to_string())),
    }
    set_headers_entries(recv, &e);
}

/// `Headers.get`: every value for `name`, joined with `", "` — the standard's
/// combined value, which is why a repeated header reads back as one string.
fn get_header(recv: &Value, name: &str) -> Option<String> {
    let name = name.to_ascii_lowercase();
    let vals: Vec<String> = headers_entries(recv)
        .into_iter()
        .filter(|(k, _)| *k == name)
        .map(|(_, v)| v)
        .collect();
    (!vals.is_empty()).then(|| vals.join(", "))
}

pub fn headers_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "get" => Ok(match get_header(recv, &super::arg_str(args, 0)) {
            Some(v) => new_str(v),
            None => with_host(|h| h.null()),
        }),
        "getSetCookie" => {
            let vals: Vec<Value> = headers_entries(recv)
                .into_iter()
                .filter(|(k, _)| k == "set-cookie")
                .map(|(_, v)| new_str(v))
                .collect();
            Ok(with_host(|h| h.new_array(vals)))
        }
        "has" => {
            let name = super::arg_str(args, 0).to_ascii_lowercase();
            Ok(Value::Bool(
                headers_entries(recv).iter().any(|(k, _)| *k == name),
            ))
        }
        "set" => {
            set_header(recv, &super::arg_str(args, 0), &super::arg_str(args, 1));
            Ok(Value::Undef)
        }
        "append" => {
            append_header(recv, &super::arg_str(args, 0), &super::arg_str(args, 1));
            Ok(Value::Undef)
        }
        "delete" => {
            let name = super::arg_str(args, 0).to_ascii_lowercase();
            let mut e = headers_entries(recv);
            e.retain(|(k, _)| *k != name);
            set_headers_entries(recv, &e);
            Ok(Value::Undef)
        }
        // Iteration is over the SORTED, combined header list (the standard's
        // "sorted and combined" order), not insertion order.
        "forEach" => {
            let cb = args.first().cloned().unwrap_or(Value::Undef);
            for (k, v) in sorted_combined(recv) {
                let (kv, vv) = (new_str(k), new_str(v));
                crate::host::invoke(&cb, vec![vv, kv, recv.clone()], None)?;
            }
            Ok(Value::Undef)
        }
        "keys" | "values" | "entries" => {
            let items: Vec<Value> = sorted_combined(recv)
                .into_iter()
                .map(|(k, v)| match method {
                    "keys" => new_str(k),
                    "values" => new_str(v),
                    _ => {
                        let (kv, vv) = (new_str(k), new_str(v));
                        with_host(|h| h.new_array(vec![kv, vv]))
                    }
                })
                .collect();
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// The header list sorted by name with same-named values combined, which is the
/// order every `Headers` iteration method observes.
fn sorted_combined(recv: &Value) -> Vec<(String, String)> {
    let mut names: Vec<String> = Vec::new();
    for (k, _) in headers_entries(recv) {
        if !names.contains(&k) {
            names.push(k);
        }
    }
    names.sort();
    names
        .into_iter()
        .filter_map(|n| get_header(recv, &n).map(|v| (n, v)))
        .collect()
}

// ── Response / Request bodies ────────────────────────────────────────────────

fn build_response(
    status: u16,
    status_text: &str,
    headers: &[(String, String)],
    body: &[u8],
    url: &str,
) -> Value {
    let headers_obj = construct_headers(&[]).unwrap_or(Value::Undef);
    for (k, v) in headers {
        append_header(&headers_obj, k, v);
    }
    let body_arr = store_body(body);
    let obj = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Response"));
        m.insert("status".into(), Value::Float(status as f64));
        m.insert("statusText".into(), h.new_str(status_text.to_string()));
        // 2xx (and only 2xx) is `ok` — 304 is not.
        m.insert(
            "ok".into(),
            Value::Bool((200..300).contains(&(status as u32))),
        );
        m.insert("redirected".into(), Value::Bool(false));
        m.insert("type".into(), h.new_str("basic"));
        m.insert("url".into(), h.new_str(url.to_string()));
        m.insert("bodyUsed".into(), Value::Bool(false));
        m.insert("headers".into(), headers_obj);
        m.insert("@@body".into(), body_arr);
        h.new_object(m)
    });
    with_host(|h| h.hide_prop(&obj, "@@body"));
    obj
}

/// `new Response(body, init)`.
pub fn construct_response(args: &[Value]) -> Result<Value, String> {
    let body = body_init_bytes(args.first());
    let init = args.get(1).cloned().unwrap_or(Value::Undef);
    let status = match prop(&init, "status") {
        Some(v) => with_host(|h| h.to_number(&v)) as u16,
        None => 200,
    };
    let status_text = prop(&init, "statusText")
        .map(|v| str_of(&v))
        .unwrap_or_default();
    let mut headers: Vec<(String, String)> = Vec::new();
    if let Some(hv) = prop(&init, "headers") {
        headers = init_header_entries(&hv);
    }
    Ok(build_response(status, &status_text, &headers, &body, ""))
}

/// `new Request(input, init)`.
pub fn construct_request(args: &[Value]) -> Result<Value, String> {
    let input = args.first().cloned().unwrap_or(Value::Undef);
    let url = if super::native_tag(&input).as_deref() == Some("Request") {
        prop(&input, "url").map(|v| str_of(&v)).unwrap_or_default()
    } else {
        str_of(&input)
    };
    let init = args.get(1).cloned().unwrap_or(Value::Undef);
    let method = prop(&init, "method")
        .map(|v| str_of(&v).to_ascii_uppercase())
        .unwrap_or_else(|| "GET".into());
    let headers_obj = construct_headers(&[])?;
    if let Some(hv) = prop(&init, "headers") {
        for (k, v) in init_header_entries(&hv) {
            append_header(&headers_obj, &k, &v);
        }
    }
    let body = body_init_bytes(prop(&init, "body").as_ref());
    let body_arr = store_body(&body);
    let obj = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Request"));
        m.insert("url".into(), h.new_str(url));
        m.insert("method".into(), h.new_str(method));
        m.insert("headers".into(), headers_obj);
        m.insert("bodyUsed".into(), Value::Bool(false));
        m.insert("@@body".into(), body_arr);
        h.new_object(m)
    });
    with_host(|h| h.hide_prop(&obj, "@@body"));
    Ok(obj)
}

/// The bytes a `BodyInit` contributes: a string encodes as UTF-8, a typed array
/// / Buffer / `Blob` supplies its bytes, and anything else stringifies.
fn body_init_bytes(v: Option<&Value>) -> Vec<u8> {
    let Some(v) = v.filter(|v| !matches!(v, Value::Undef)) else {
        return Vec::new();
    };
    if with_host(|h| h.is_null(v)) {
        return Vec::new();
    }
    match super::native_tag(v).as_deref() {
        Some("Blob") | Some("File") | Some("Response") | Some("Request") => return body_bytes(v),
        Some("TypedArray") | Some("Buffer") => {
            if let Some(e) = super::typedarray::elems_of(v) {
                return e.iter().map(|x| *x as u8).collect();
            }
        }
        _ => {}
    }
    if super::native_tag(v).as_deref() == Some("URLSearchParams") {
        return str_of(v).into_bytes();
    }
    str_of(v).into_bytes()
}

/// The body accessors shared by `Response`, `Request` and `Blob`. Each answers a
/// settled Promise, because the body is already fully buffered.
pub fn body_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "text" => {
            set_prop(recv, "bodyUsed", Value::Bool(true));
            let s = String::from_utf8_lossy(&body_bytes(recv)).into_owned();
            Ok(settled(Ok(new_str(s))))
        }
        "json" => {
            set_prop(recv, "bodyUsed", Value::Bool(true));
            let s = String::from_utf8_lossy(&body_bytes(recv)).into_owned();
            let parsed = crate::builtins::call_builtin_function("JSON.parse", vec![new_str(s)]);
            Ok(settled(parsed))
        }
        "bytes" => {
            set_prop(recv, "bodyUsed", Value::Bool(true));
            Ok(settled(to_uint8array(&body_bytes(recv))))
        }
        "arrayBuffer" => {
            set_prop(recv, "bodyUsed", Value::Bool(true));
            let bytes = body_bytes(recv);
            let buf = with_host(|h| {
                let mut m = IndexMap::new();
                m.insert("@@native".into(), h.new_str("ArrayBuffer"));
                m.insert("byteLength".into(), Value::Float(bytes.len() as f64));
                let arr = h.new_array(bytes.iter().map(|b| Value::Float(*b as f64)).collect());
                m.insert("@@bytes".into(), arr);
                h.new_object(m)
            });
            with_host(|h| h.hide_prop(&buf, "@@bytes"));
            Ok(settled(Ok(buf)))
        }
        "blob" => {
            set_prop(recv, "bodyUsed", Value::Bool(true));
            let bytes = body_bytes(recv);
            let ct = prop(recv, "headers")
                .and_then(|h| get_header(&h, "content-type"))
                .unwrap_or_default();
            Ok(settled(Ok(new_blob(&bytes, &ct, None))))
        }
        "formData" => {
            set_prop(recv, "bodyUsed", Value::Bool(true));
            let s = String::from_utf8_lossy(&body_bytes(recv)).into_owned();
            let fd = construct_form_data(&[])?;
            // Only the urlencoded form is decoded here; a multipart body needs
            // the boundary parser this does not have.
            for pair in s.split('&').filter(|p| !p.is_empty()) {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                form_append(&fd, &percent_decode(k), &new_str(percent_decode(v)));
            }
            Ok(settled(Ok(fd)))
        }
        "clone" => {
            let bytes = body_bytes(recv);
            let clone = crate::builtins::deep_clone(recv);
            set_prop(&clone, "@@body", store_body(&bytes));
            set_prop(&clone, "bodyUsed", Value::Bool(false));
            Ok(clone)
        }
        "slice" => {
            let bytes = body_bytes(recv);
            let start = args
                .first()
                .map(|v| with_host(|h| h.to_number(v)) as usize)
                .unwrap_or(0)
                .min(bytes.len());
            let end = args
                .get(1)
                .filter(|v| !matches!(v, Value::Undef))
                .map(|v| with_host(|h| h.to_number(v)) as usize)
                .unwrap_or(bytes.len())
                .clamp(start, bytes.len());
            let ct = args.get(2).map(str_of).unwrap_or_default();
            Ok(new_blob(&bytes[start..end], &ct, None))
        }
        "stream" => Err(crate::host::type_error(
            "Response.body streaming is not implemented on this build",
        )),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// Throw `v` as the JS exception value (rather than as an internal message), so
/// `catch (e)` receives the object the spec says it should.
fn throw_js(v: Value) -> String {
    let msg = with_host(|h| h.str_of(&v));
    with_host(|h| h.exc = Some(v));
    msg
}

fn percent_decode(s: &str) -> String {
    let bytes = s.replace('+', " ").into_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ── Blob / File ──────────────────────────────────────────────────────────────

fn new_blob(bytes: &[u8], content_type: &str, file_name: Option<&str>) -> Value {
    let body = store_body(bytes);
    let obj = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert(
            "@@native".into(),
            h.new_str(if file_name.is_some() { "File" } else { "Blob" }),
        );
        m.insert("size".into(), Value::Float(bytes.len() as f64));
        m.insert("type".into(), h.new_str(content_type.to_ascii_lowercase()));
        if let Some(n) = file_name {
            m.insert("name".into(), h.new_str(n.to_string()));
            m.insert("lastModified".into(), Value::Float(0.0));
        }
        m.insert("@@body".into(), body);
        h.new_object(m)
    });
    with_host(|h| h.hide_prop(&obj, "@@body"));
    obj
}

pub fn construct_blob(args: &[Value]) -> Result<Value, String> {
    let mut bytes = Vec::new();
    if let Some(parts) = args.first() {
        for p in crate::host::iter_all(parts).unwrap_or_default() {
            bytes.extend(body_init_bytes(Some(&p)));
        }
    }
    let ct = args
        .get(1)
        .and_then(|o| prop(o, "type"))
        .map(|v| str_of(&v))
        .unwrap_or_default();
    Ok(new_blob(&bytes, &ct, None))
}

/// `new File(parts, name[, options])` — a `Blob` that also carries a name.
pub fn construct_file(args: &[Value]) -> Result<Value, String> {
    let mut bytes = Vec::new();
    if let Some(parts) = args.first() {
        for p in crate::host::iter_all(parts).unwrap_or_default() {
            bytes.extend(body_init_bytes(Some(&p)));
        }
    }
    let name = super::arg_str(args, 1);
    let ct = args
        .get(2)
        .and_then(|o| prop(o, "type"))
        .map(|v| str_of(&v))
        .unwrap_or_default();
    Ok(new_blob(&bytes, &ct, Some(&name)))
}

// ── FormData ─────────────────────────────────────────────────────────────────

pub fn construct_form_data(_args: &[Value]) -> Result<Value, String> {
    let obj = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("FormData"));
        let empty = h.new_array(Vec::new());
        m.insert("@@entries".into(), empty);
        h.new_object(m)
    });
    with_host(|h| h.hide_prop(&obj, "@@entries"));
    Ok(obj)
}

fn form_entries(recv: &Value) -> Vec<(String, Value)> {
    let Some(arr) = prop(recv, "@@entries") else {
        return Vec::new();
    };
    with_host(|h| match h.get(&arr) {
        Some(JsObj::Array(items)) => items
            .iter()
            .filter_map(|pair| match h.get(pair) {
                Some(JsObj::Array(kv)) if kv.len() == 2 => Some((h.str_of(&kv[0]), kv[1].clone())),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    })
}

fn set_form_entries(recv: &Value, entries: &[(String, Value)]) {
    let arr = with_host(|h| {
        let pairs: Vec<Value> = entries
            .iter()
            .map(|(k, v)| {
                let k = h.new_str(k.clone());
                h.new_array(vec![k, v.clone()])
            })
            .collect();
        h.new_array(pairs)
    });
    set_prop(recv, "@@entries", arr);
}

fn form_append(recv: &Value, name: &str, value: &Value) {
    let mut e = form_entries(recv);
    e.push((name.to_string(), value.clone()));
    set_form_entries(recv, &e);
}

pub fn form_data_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let name = super::arg_str(args, 0);
    match method {
        "append" => {
            form_append(recv, &name, args.get(1).unwrap_or(&Value::Undef));
            Ok(Value::Undef)
        }
        "set" => {
            let mut e = form_entries(recv);
            e.retain(|(k, _)| *k != name);
            e.push((name, args.get(1).cloned().unwrap_or(Value::Undef)));
            set_form_entries(recv, &e);
            Ok(Value::Undef)
        }
        "delete" => {
            let mut e = form_entries(recv);
            e.retain(|(k, _)| *k != name);
            set_form_entries(recv, &e);
            Ok(Value::Undef)
        }
        "get" => Ok(form_entries(recv)
            .into_iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v)
            .unwrap_or_else(|| with_host(|h| h.null()))),
        "getAll" => {
            let vals: Vec<Value> = form_entries(recv)
                .into_iter()
                .filter(|(k, _)| *k == name)
                .map(|(_, v)| v)
                .collect();
            Ok(with_host(|h| h.new_array(vals)))
        }
        "has" => Ok(Value::Bool(
            form_entries(recv).iter().any(|(k, _)| *k == name),
        )),
        "forEach" => {
            let cb = args.first().cloned().unwrap_or(Value::Undef);
            for (k, v) in form_entries(recv) {
                let kv = new_str(k);
                crate::host::invoke(&cb, vec![v, kv, recv.clone()], None)?;
            }
            Ok(Value::Undef)
        }
        "keys" | "values" | "entries" => {
            let items: Vec<Value> = form_entries(recv)
                .into_iter()
                .map(|(k, v)| match method {
                    "keys" => new_str(k),
                    "values" => v,
                    _ => {
                        let kv = new_str(k);
                        with_host(|h| h.new_array(vec![kv, v]))
                    }
                })
                .collect();
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

// ── AbortController / AbortSignal ────────────────────────────────────────────

pub fn new_abort_signal() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("AbortSignal"));
        m.insert("aborted".into(), Value::Bool(false));
        m.insert("reason".into(), Value::Undef);
        m.insert("onabort".into(), h.null());
        h.new_object(m)
    })
}

pub fn construct_abort_controller(_args: &[Value]) -> Result<Value, String> {
    let signal = new_abort_signal();
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("AbortController"));
        m.insert("signal".into(), signal);
        h.new_object(m)
    }))
}

/// The `DOMException`-shaped reason an abort carries. node's `AbortSignal`
/// rejects with a `DOMException` whose `name` is `AbortError`/`TimeoutError`;
/// `synth_error` only knows the ECMAScript error classes, so the name, the
/// constructor label and the `stack` head are stamped on afterwards.
fn dom_exception(name: &str, message: &str) -> Value {
    let e = with_host(|h| crate::builtins::synth_error(h, &format!("Error: {message}")));
    with_host(|h| {
        let n = h.new_str(name.to_string());
        let stack = h.new_str(format!("{name}: {message}"));
        if let Some(JsObj::Object(p)) = h.get_mut(&e) {
            p.insert("name".into(), n);
            p.insert("stack".into(), stack);
        }
    });
    e
}

/// Fire an `AbortSignal.timeout` deadline: abort the signal at heap index `idx`
/// with a `TimeoutError`. Reached from the `@@aborttimeout:<idx>` thunk.
pub fn fire_timeout_abort(idx: u32) -> Result<Value, String> {
    let signal = Value::Obj(idx);
    let e = dom_exception("TimeoutError", "The operation was aborted due to timeout");
    abort_signal(&signal, e)?;
    Ok(Value::Undef)
}

/// Mark a signal aborted and run its `onabort` / `abort` listeners.
fn abort_signal(signal: &Value, reason: Value) -> Result<(), String> {
    if matches!(prop(signal, "aborted"), Some(Value::Bool(true))) {
        return Ok(());
    }
    let reason = if matches!(reason, Value::Undef) {
        dom_exception("AbortError", "This operation was aborted")
    } else {
        reason
    };
    set_prop(signal, "aborted", Value::Bool(true));
    set_prop(signal, "reason", reason);
    if let Some(cb) = prop(signal, "onabort") {
        if with_host(|h| crate::host::is_callable(h, &cb)) {
            crate::host::invoke(&cb, vec![Value::Undef], Some(signal.clone()))?;
        }
    }
    if let Some(list) = prop(signal, "@@abortListeners") {
        for cb in crate::host::iter_all(&list).unwrap_or_default() {
            crate::host::invoke(&cb, vec![Value::Undef], Some(signal.clone()))?;
        }
    }
    Ok(())
}

pub fn abort_controller_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "abort" => {
            let signal = prop(recv, "signal").unwrap_or(Value::Undef);
            abort_signal(&signal, args.first().cloned().unwrap_or(Value::Undef))?;
            Ok(Value::Undef)
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

pub fn abort_signal_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "throwIfAborted" => {
            if matches!(prop(recv, "aborted"), Some(Value::Bool(true))) {
                let reason = prop(recv, "reason").unwrap_or(Value::Undef);
                return Err(throw_js(reason));
            }
            Ok(Value::Undef)
        }
        "addEventListener" => {
            if super::arg_str(args, 0) == "abort" {
                let cb = args.get(1).cloned().unwrap_or(Value::Undef);
                let existing = prop(recv, "@@abortListeners");
                let list = match existing {
                    Some(l) => l,
                    None => {
                        let l = with_host(|h| h.new_array(Vec::new()));
                        set_prop(recv, "@@abortListeners", l.clone());
                        with_host(|h| h.hide_prop(recv, "@@abortListeners"));
                        l
                    }
                };
                with_host(|h| {
                    if let Some(JsObj::Array(items)) = h.get_mut(&list) {
                        items.push(cb);
                    }
                });
            }
            Ok(Value::Undef)
        }
        "removeEventListener" => Ok(Value::Undef),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// `AbortSignal.abort(reason)` / `AbortSignal.timeout(ms)`.
pub fn abort_signal_static(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "abort" => {
            let s = new_abort_signal();
            let r = abort_signal(&s, args.first().cloned().unwrap_or(Value::Undef));
            Some(r.map(|_| s))
        }
        "timeout" => {
            let s = new_abort_signal();
            // The abort is a real scheduled macrotask: a native continuation
            // thunk (the `@@`-prefixed convention `@@presolve:<id>` already
            // uses) carries the signal's heap index to the timer callback.
            let ms = super::arg_num(args, 0);
            let Value::Obj(idx) = s else {
                return Some(Ok(s));
            };
            let cb = with_host(|h| h.alloc(JsObj::Builtin(format!("@@aborttimeout:{idx}"))));
            with_host(|h| h.add_timer(ms, cb, Vec::new(), None));
            Some(Ok(s))
        }
        _ => None,
    }
}

// ── fetch ────────────────────────────────────────────────────────────────────

/// `fetch(input[, init])` — a Promise for a fully buffered `Response`.
pub fn fetch(args: &[Value]) -> Result<Value, String> {
    let promise = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&promise).unwrap_or(0));

    let input = args.first().cloned().unwrap_or(Value::Undef);
    let init = args.get(1).cloned().unwrap_or(Value::Undef);

    // `input` is a URL string, a `URL`, or a `Request` (whose method/headers/
    // body seed the request, and `init` overrides them).
    let from_request = super::native_tag(&input).as_deref() == Some("Request");
    let url = if from_request {
        prop(&input, "url").map(|v| str_of(&v)).unwrap_or_default()
    } else {
        str_of(&input)
    };

    let mut method = if from_request {
        prop(&input, "method")
            .map(|v| str_of(&v))
            .unwrap_or_else(|| "GET".into())
    } else {
        "GET".into()
    };
    if let Some(m) = prop(&init, "method") {
        method = str_of(&m);
    }
    method = method.to_ascii_uppercase();

    let mut headers: Vec<(String, String)> = Vec::new();
    if from_request {
        if let Some(h) = prop(&input, "headers") {
            headers = headers_entries(&h);
        }
    }
    if let Some(hv) = prop(&init, "headers") {
        headers.extend(init_header_entries(&hv));
    }

    let body = match prop(&init, "body") {
        Some(b) => body_init_bytes(Some(&b)),
        None if from_request => body_bytes(&input),
        None => Vec::new(),
    };

    // An already-aborted signal rejects before any connection is made.
    if let Some(sig) = prop(&init, "signal") {
        if matches!(prop(&sig, "aborted"), Some(Value::Bool(true))) {
            let reason = prop(&sig, "reason")
                .unwrap_or_else(|| dom_exception("AbortError", "This operation was aborted"));
            crate::host::reject_promise_val(id, reason);
            return Ok(promise);
        }
    }

    let Some(target) = parse_target(&url) else {
        crate::host::reject_promise_val(id, fetch_failed("unknown scheme"));
        return Ok(promise);
    };

    let wire = build_request_bytes(&target, &method, &headers, &body);
    let io_tx = with_host(|h| h.io_sender());
    with_host(|h| h.incr_handle());
    let url_for_response = url.clone();
    std::thread::spawn(move || {
        let raw = if target.tls {
            let config = crate::stdlib::tls::client_config(true);
            crate::stdlib::https::exchange(&target.host, target.port, &target.host, config, &wire)
        } else {
            crate::stdlib::http::exchange(&target.host, target.port, &wire)
        };
        let _ = io_tx.send(Box::new(move || {
            with_host(|h| h.decr_handle());
            match raw {
                Ok(raw) => {
                    let (status, message, _v, headers, body) =
                        crate::stdlib::http::parse_raw_response(&raw);
                    let resp = build_response(status, &message, &headers, &body, &url_for_response);
                    crate::host::resolve_promise_val(id, resp);
                }
                Err(msg) => crate::host::reject_promise_val(id, fetch_failed(&msg)),
            }
            Ok(())
        }));
    });
    Ok(promise)
}

/// node reports EVERY fetch transport failure as `TypeError: fetch failed` and
/// puts the detail on `.cause` as a nested `Error`, so the reason is available
/// without the message itself varying by platform.
fn fetch_failed(cause: &str) -> Value {
    with_host(|h| {
        let e = crate::builtins::synth_error(h, "TypeError: fetch failed");
        let c = crate::builtins::synth_error(h, &format!("Error: {cause}"));
        if let Some(JsObj::Object(p)) = h.get_mut(&e) {
            p.insert("cause".into(), c);
        }
        e
    })
}

struct Target {
    host: String,
    port: u16,
    path: String,
    tls: bool,
}

fn parse_target(url: &str) -> Option<Target> {
    // Only `http`/`https` are fetchable here; every other scheme (`file:`,
    // `data:`, `blob:`) rejects as an unknown scheme.
    let (tls, rest) = match url.strip_prefix("https://") {
        Some(r) => (true, r),
        None => (false, url.strip_prefix("http://")?),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let authority = authority.split('@').next_back().unwrap_or(authority);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(n) => (h.to_string(), n),
            Err(_) => (authority.to_string(), if tls { 443 } else { 80 }),
        },
        None => (authority.to_string(), if tls { 443 } else { 80 }),
    };
    if host.is_empty() {
        return None;
    }
    Some(Target {
        host,
        port,
        path,
        tls,
    })
}

fn build_request_bytes(
    target: &Target,
    method: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Vec<u8> {
    let mut has_host = false;
    let mut has_len = false;
    let mut block = String::new();
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        if k.eq_ignore_ascii_case("content-length") {
            has_len = true;
        }
        if k.eq_ignore_ascii_case("connection") {
            continue;
        }
        block.push_str(&format!("{k}: {v}\r\n"));
    }
    let default_port = if target.tls { 443 } else { 80 };
    let host_header = if target.port == default_port {
        target.host.clone()
    } else {
        format!("{}:{}", target.host, target.port)
    };
    let mut req = format!("{method} {} HTTP/1.1\r\n", target.path);
    if !has_host {
        req.push_str(&format!("Host: {host_header}\r\n"));
    }
    req.push_str(&block);
    if !has_len && !body.is_empty() {
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    // Every exchange reads to EOF, so the connection must not be kept alive.
    req.push_str("Connection: close\r\n\r\n");
    let mut wire = req.into_bytes();
    wire.extend_from_slice(body);
    wire
}

// ── dispatch ─────────────────────────────────────────────────────────────────

pub fn instance_call(
    tag: &str,
    recv: &Value,
    method: &str,
    args: &[Value],
) -> Result<Value, String> {
    match tag {
        "Headers" => headers_call(recv, method, args),
        "FormData" => form_data_call(recv, method, args),
        "AbortController" => abort_controller_call(recv, method, args),
        "AbortSignal" => abort_signal_call(recv, method, args),
        _ => body_call(recv, method, args),
    }
}

pub fn construct(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match name {
        "Headers" => construct_headers(args),
        "Request" => construct_request(args),
        "Response" => construct_response(args),
        "Blob" => construct_blob(args),
        "File" => construct_file(args),
        "FormData" => construct_form_data(args),
        "AbortController" => construct_abort_controller(args),
        "AbortSignal" => Err(crate::host::type_error(
            "Illegal constructor: use AbortSignal.abort() or AbortSignal.timeout()",
        )),
        _ => return None,
    })
}

/// The static methods of these classes (`Response.json`, `AbortSignal.abort`, …).
pub fn static_call(ns: &str, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match ns {
        "AbortSignal" => abort_signal_static(method, args),
        "Response" => match method {
            "json" => {
                let body = crate::builtins::call_builtin_function(
                    "JSON.stringify",
                    vec![args.first()?.clone()],
                )
                .map(|v| str_of(&v))
                .unwrap_or_default();
                let init = args.get(1).cloned().unwrap_or(Value::Undef);
                let status = match prop(&init, "status") {
                    Some(v) => with_host(|h| h.to_number(&v)) as u16,
                    None => 200,
                };
                let mut headers =
                    vec![("content-type".to_string(), "application/json".to_string())];
                if let Some(hv) = prop(&init, "headers") {
                    headers.extend(init_header_entries(&hv));
                }
                Some(Ok(build_response(
                    status,
                    "",
                    &headers,
                    body.as_bytes(),
                    "",
                )))
            }
            "error" => Some(Ok(build_response(0, "", &[], &[], ""))),
            "redirect" => {
                let loc = super::arg_str(args, 0);
                let status = match args.get(1) {
                    Some(v) => with_host(|h| h.to_number(v)) as u16,
                    None => 302,
                };
                Some(Ok(build_response(
                    status,
                    "",
                    &[("location".into(), loc)],
                    &[],
                    "",
                )))
            }
            _ => None,
        },
        _ => None,
    }
}

pub const RESPONSE_STATICS: &[&str] = &["json", "error", "redirect"];
pub const ABORT_SIGNAL_STATICS: &[&str] = &["abort", "timeout"];
