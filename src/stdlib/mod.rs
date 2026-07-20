//! Node.js core modules implemented natively for node-js.
//!
//! A `require(spec)` (see `builtins::call_builtin_function`) resolves a supported
//! module to a `JsObj::Builtin("<module>")` namespace value — exactly the shape
//! of the built-in `console`/`Math` namespaces — so `mod.method(...)` dispatches
//! through `host::call_method` → `builtins::call_builtin_function("<module>.<method>")`
//! → `stdlib::call`, and `const { method } = require('mod')` reads the method as a
//! first-class `Builtin("mod.method")` via `namespace_property`.
//!
//! Every stdlib function is free-standing and acquires the thread-local `JsHost`
//! through `with_host` only around allocations (and releases it before any
//! re-entrant `host::invoke`), so callbacks (`fs` async, `EventEmitter.emit`,
//! `assert.throws`) never double-borrow the host. Stateful instances (`Buffer`,
//! crypto `Hash`, `EventEmitter`, `URL`) are plain objects carrying a hidden
//! `@@native` tag (filtered from enumeration/display like `@@iterator`); their
//! methods route through `instance_call` from `host::call_method`.

use crate::host::{with_host, JsObj};
use fusevm::Value;

pub mod assert;
pub mod buffer;
pub mod crypto;
pub mod events;
pub mod fs;
pub mod http;
pub mod net;
pub mod os;
pub mod path;
pub mod process;
pub mod stream;
pub mod tty;
pub mod url;
pub mod util;

/// Canonical namespace name a `require(spec)` resolves to (after stripping an
/// optional `node:` prefix), or `None` for an unsupported module.
pub fn resolve(spec: &str) -> Option<&'static str> {
    match spec.strip_prefix("node:").unwrap_or(spec) {
        "fs" => Some("fs"),
        "path" => Some("path"),
        "os" => Some("os"),
        "util" => Some("util"),
        "assert" => Some("assert"),
        "crypto" => Some("crypto"),
        "buffer" => Some("buffer"),
        "url" => Some("url"),
        "process" => Some("process"),
        "net" => Some("net"),
        "http" => Some("http"),
        "stream" => Some("stream"),
        "tty" => Some("tty"),
        // The `events` module's export IS the EventEmitter constructor, so
        // `require('events')` yields the ctor namespace directly.
        "events" => Some("EventEmitter"),
        _ => None,
    }
}

/// True if `qualified` (`namespace.method`) is a stdlib method that
/// `call_builtin_function` should route into `call` (extends `is_known_builtin`).
pub fn is_method(qualified: &str) -> bool {
    let Some((ns, m)) = qualified.split_once('.') else {
        return qualified == "assert";
    };
    match ns {
        "fs" => fs::METHODS.contains(&m),
        "path" => path::METHODS.contains(&m),
        "os" => os::METHODS.contains(&m),
        "util" => util::METHODS.contains(&m),
        "assert" => assert::METHODS.contains(&m),
        "crypto" => crypto::METHODS.contains(&m),
        "Buffer" => buffer::STATIC_METHODS.contains(&m),
        "buffer" => m == "Buffer",
        "url" => url::MODULE_METHODS.contains(&m) || m == "URL",
        "net" => net::MODULE_METHODS.contains(&m),
        "http" => http::MODULE_METHODS.contains(&m),
        "tty" => tty::METHODS.contains(&m),
        "process" => process::METHODS.contains(&m),
        "EventEmitter" => m == "EventEmitter",
        _ => false,
    }
}

/// Dispatch a resolved stdlib builtin (`assert`, or `namespace.method`). Returns
/// `None` if `name` is not a stdlib builtin (the caller falls through to the core
/// builtin table).
pub fn call(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    if name == "assert" {
        return Some(assert::assert_ok(args));
    }
    let (ns, m) = name.split_once('.')?;
    Some(match ns {
        "fs" => fs::call(m, args)?,
        "path" => path::call(m, args)?,
        "os" => os::call(m, args)?,
        "util" => util::call(m, args)?,
        "assert" => assert::call(m, args)?,
        "crypto" => crypto::call(m, args)?,
        "Buffer" => buffer::static_call(m, args)?,
        "buffer" if m == "Buffer" => Ok(with_host(|h| h.alloc(JsObj::Builtin("Buffer".into())))),
        "url" if m == "URL" => Ok(with_host(|h| h.alloc(JsObj::Builtin("URL".into())))),
        "url" => url::call(m, args)?,
        "net" => net::call(m, args)?,
        "http" => http::call(m, args)?,
        "tty" => tty::call(m, args)?,
        "process" => process::call(m, args)?,
        "EventEmitter" if m == "EventEmitter" => {
            Ok(with_host(|h| h.alloc(JsObj::Builtin("EventEmitter".into()))))
        }
        _ => return None,
    })
}

/// A non-function constant on a stdlib namespace (`path.sep`, `os.EOL`,
/// `buffer.Buffer`, `url.URL`), reachable via `namespace_property`.
pub fn constant(ns: &str, name: &str) -> Option<Value> {
    match ns {
        "path" => path::constant(name),
        "os" => os::constant(name),
        "buffer" if name == "Buffer" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("Buffer".into()))))
        }
        "url" if name == "URL" => Some(with_host(|h| h.alloc(JsObj::Builtin("URL".into())))),
        "stream" => stream::constant(name),
        "process" => process::constant(name),
        "EventEmitter" if name == "EventEmitter" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("EventEmitter".into()))))
        }
        _ => None,
    }
}

/// Construct a stdlib class instance (`new URL(...)`, `new EventEmitter()`, and
/// `new Buffer(...)` legacy), reachable from `construct_builtin`. `None` if `name`
/// is not a stdlib constructor.
pub fn construct(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match name {
        "URL" => Some(url::construct(args)),
        "EventEmitter" => Some(Ok(events::new_emitter())),
        "Buffer" => Some(buffer::static_call("from", args).unwrap_or(Ok(Value::Undef))),
        n if stream::is_class(n) => Some(Ok(stream::construct(n))),
        _ => None,
    }
}

/// The hidden `@@native` instance tag of `recv` (`"Buffer"`/`"Hash"`/
/// `"EventEmitter"`/`"URL"`), or `None` for a non-native object.
pub fn native_tag(recv: &Value) -> Option<String> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@native").map(|v| h.str_of(v)),
        _ => None,
    })
}

/// Dispatch a method call on a native stdlib instance (`recv` carries a
/// `@@native` tag). Called from `host::call_method` before the generic object
/// method resolution.
pub fn instance_call(tag: &str, recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match tag {
        "Buffer" => buffer::instance_call(recv, method, &args),
        "Hash" => crypto::instance_call(recv, method, &args),
        "EventEmitter" => events::instance_call(recv, method, args),
        "URL" => url::instance_call(recv, method, &args),
        "Stats" => fs::stats_call(recv, method),
        "Server" | "Socket" => net::instance_call(tag, recv, method, args),
        "IncomingMessage" | "ServerResponse" => http::instance_call(tag, recv, method, args),
        "Readable" | "Writable" | "Duplex" | "Transform" => stream::instance_call(tag, recv, method, args),
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

// ── shared helpers ──────────────────────────────────────────────────────────

/// ToString of `args[i]` (empty string if absent).
pub(crate) fn arg_str(args: &[Value], i: usize) -> String {
    with_host(|h| args.get(i).map(|v| h.str_of(v)).unwrap_or_default())
}

/// ToNumber of `args[i]` (`NaN` if absent).
pub(crate) fn arg_num(args: &[Value], i: usize) -> f64 {
    with_host(|h| args.get(i).map(|v| h.to_number(v)).unwrap_or(f64::NAN))
}

/// Lowercase hex encoding of `bytes`.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

/// Decode a hex string to bytes (ignoring a trailing odd nibble, like Node).
pub(crate) fn from_hex(s: &str) -> Vec<u8> {
    let digits: Vec<u8> = s.bytes().filter_map(|c| (c as char).to_digit(16).map(|d| d as u8)).collect();
    digits.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0] << 4) | c[1]).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 encoding (with `=` padding) of `bytes`.
pub(crate) fn to_base64(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Decode a standard base64 string to bytes (ignores whitespace and padding).
pub(crate) fn from_base64(s: &str) -> Vec<u8> {
    let rev = |c: u8| -> Option<u32> { B64.iter().position(|&x| x == c).map(|p| p as u32) };
    let vals: Vec<u32> = s.bytes().filter_map(rev).collect();
    let mut out = Vec::new();
    for chunk in vals.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let n = (chunk[0] << 18)
            | (chunk[1] << 12)
            | (chunk.get(2).copied().unwrap_or(0) << 6)
            | chunk.get(3).copied().unwrap_or(0);
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}
