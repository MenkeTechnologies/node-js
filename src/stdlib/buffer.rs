//! Node `Buffer` (global + `require('buffer').Buffer`). A Buffer is a plain
//! object tagged `@@native = "Buffer"` whose bytes live in a hidden `@@bytes`
//! array; `length` is an enumerable data property so `buf.length` reads directly.

use super::{arg_str, from_base64, from_hex, to_base64, to_hex};
use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const STATIC_METHODS: &[&str] = &["from", "alloc", "allocUnsafe", "concat", "isBuffer", "byteLength"];

/// Build a Buffer value from raw bytes.
pub fn from_bytes(bytes: &[u8]) -> Value {
    with_host(|h| {
        let arr = h.new_array(bytes.iter().map(|b| Value::Float(*b as f64)).collect());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Buffer"));
        m.insert("@@bytes".into(), arr);
        m.insert("length".into(), Value::Float(bytes.len() as f64));
        h.new_object(m)
    })
}

fn bytes_of(recv: &Value) -> Vec<u8> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|v| h.get(v)) {
            Some(JsObj::Array(items)) => items.iter().map(|v| h.to_number(v) as u8).collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    })
}

pub fn static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "from" => from(args),
        "alloc" => {
            let n = super::arg_num(args, 0).max(0.0) as usize;
            let fill = if args.len() > 1 { super::arg_num(args, 1) as u8 } else { 0 };
            Ok(from_bytes(&vec![fill; n]))
        }
        "allocUnsafe" => Ok(from_bytes(&vec![0u8; super::arg_num(args, 0).max(0.0) as usize])),
        "concat" => concat(args),
        "isBuffer" => Ok(Value::Bool(super::native_tag(&args.first().cloned().unwrap_or(Value::Undef)).as_deref() == Some("Buffer"))),
        "byteLength" => {
            let enc = args.get(1).map(|_| arg_str(args, 1)).unwrap_or_else(|| "utf8".into());
            Ok(Value::Float(decode_str(&arg_str(args, 0), &enc).len() as f64))
        }
        _ => return None,
    })
}

fn from(args: &[Value]) -> Result<Value, String> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    // Array of byte values.
    let arr = with_host(|h| match h.get(&v) {
        Some(JsObj::Array(items)) => Some(items.iter().map(|x| h.to_number(x) as u8).collect::<Vec<u8>>()),
        _ => None,
    });
    if let Some(bytes) = arr {
        return Ok(from_bytes(&bytes));
    }
    // Another Buffer: copy.
    if super::native_tag(&v).as_deref() == Some("Buffer") {
        return Ok(from_bytes(&bytes_of(&v)));
    }
    // String with an optional encoding.
    let enc = if args.len() > 1 { arg_str(args, 1) } else { "utf8".into() };
    Ok(from_bytes(&decode_str(&arg_str(args, 0), &enc)))
}

fn concat(args: &[Value]) -> Result<Value, String> {
    let list = with_host(|h| match h.get(&args.first().cloned().unwrap_or(Value::Undef)) {
        Some(JsObj::Array(items)) => items.clone(),
        _ => Vec::new(),
    });
    let mut out = Vec::new();
    for b in &list {
        out.extend(bytes_of(b));
    }
    Ok(from_bytes(&out))
}

/// Buffer instance methods.
pub fn instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let bytes = bytes_of(recv);
    match method {
        "toString" => {
            let enc = if args.is_empty() { "utf8".into() } else { arg_str(args, 0) };
            Ok(with_host(|h| h.new_str(encode_bytes(&bytes, &enc))))
        }
        "toJSON" => Ok(with_host(|h| {
            let data = h.new_array(bytes.iter().map(|b| Value::Float(*b as f64)).collect());
            let mut m = IndexMap::new();
            m.insert("type".into(), h.new_str("Buffer"));
            m.insert("data".into(), data);
            h.new_object(m)
        })),
        "equals" => {
            let other = bytes_of(&args.first().cloned().unwrap_or(Value::Undef));
            Ok(Value::Bool(bytes == other))
        }
        "slice" | "subarray" => {
            let (s, e) = slice_bounds(args, bytes.len());
            Ok(from_bytes(&bytes[s..e]))
        }
        "readUInt8" => {
            let i = super::arg_num(args, 0).max(0.0) as usize;
            Ok(Value::Float(*bytes.get(i).unwrap_or(&0) as f64))
        }
        "includes" | "indexOf" => {
            let needle = decode_str(&arg_str(args, 0), "utf8");
            let pos = bytes.windows(needle.len().max(1)).position(|w| w == needle.as_slice());
            if method == "includes" {
                Ok(Value::Bool(pos.is_some()))
            } else {
                Ok(Value::Float(pos.map(|p| p as f64).unwrap_or(-1.0)))
            }
        }
        _ => Err(crate::host::type_error(&format!("buffer.{method} is not a function"))),
    }
}

fn slice_bounds(args: &[Value], len: usize) -> (usize, usize) {
    let norm = |n: f64| -> usize {
        if n < 0.0 {
            (len as f64 + n).max(0.0) as usize
        } else {
            (n as usize).min(len)
        }
    };
    let s = if args.is_empty() { 0 } else { norm(super::arg_num(args, 0)) };
    let e = if args.len() < 2 { len } else { norm(super::arg_num(args, 1)) };
    (s.min(e), e.max(s))
}

fn decode_str(s: &str, enc: &str) -> Vec<u8> {
    match enc.to_ascii_lowercase().as_str() {
        "hex" => from_hex(s),
        "base64" | "base64url" => from_base64(s),
        "ascii" | "latin1" | "binary" => s.chars().map(|c| c as u8).collect(),
        _ => s.as_bytes().to_vec(),
    }
}

fn encode_bytes(bytes: &[u8], enc: &str) -> String {
    match enc.to_ascii_lowercase().as_str() {
        "hex" => to_hex(bytes),
        "base64" | "base64url" => to_base64(bytes),
        "ascii" | "latin1" | "binary" => bytes.iter().map(|b| *b as char).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}
