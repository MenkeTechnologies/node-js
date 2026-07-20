//! Node `Buffer` (global + `require('buffer').Buffer`). A Buffer is a plain
//! object tagged `@@native = "Buffer"` whose bytes live in a hidden `@@bytes`
//! array; `length` is an enumerable data property so `buf.length` reads directly.

use super::{arg_str, from_base64, from_hex, to_base64, to_hex};
use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const STATIC_METHODS: &[&str] = &["from", "alloc", "allocUnsafe", "concat", "isBuffer", "byteLength"];

/// Free functions of the `buffer` module itself (`require('buffer').atob`, …), as
/// opposed to the `Buffer` constructor's static methods above. Needs the parent
/// `"buffer"` routing arm (see final report).
pub const MODULE_METHODS: &[&str] = &["atob", "btoa", "isAscii", "isUtf8", "transcode"];

/// Dispatch a `require('buffer').<method>` free function.
pub fn module_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        // atob: base64 → a binary (latin1) string.
        "atob" => {
            let s = arg_str(args, 0);
            let bytes = from_base64(&s);
            let bin: String = bytes.iter().map(|b| *b as char).collect();
            Ok(with_host(|h| h.new_str(bin)))
        }
        // btoa: a binary string → base64 (each char's low byte is one octet).
        "btoa" => {
            let s = arg_str(args, 0);
            let bytes: Vec<u8> = s.chars().map(|c| c as u32 as u8).collect();
            let b64 = to_base64(&bytes);
            Ok(with_host(|h| h.new_str(b64)))
        }
        "isAscii" => {
            let bytes = input_bytes(args.first());
            Ok(Value::Bool(bytes.iter().all(|b| *b < 0x80)))
        }
        "isUtf8" => {
            let bytes = input_bytes(args.first());
            Ok(Value::Bool(std::str::from_utf8(&bytes).is_ok()))
        }
        // transcode(source, fromEnc, toEnc): re-encode bytes between utf8/latin1/
        // ascii/utf16le (best-effort; hex/base64 are not transcode encodings).
        "transcode" => {
            let src = input_bytes(args.first());
            let from = arg_str(args, 1);
            let to = arg_str(args, 2);
            let s = bytes_to_string(&src, &from);
            let out = string_to_bytes(&s, &to);
            Ok(from_bytes(&out))
        }
        _ => return None,
    })
}

/// Raw bytes of a Buffer/Blob arg, or the UTF-8 bytes of a string arg.
fn input_bytes(v: Option<&Value>) -> Vec<u8> {
    match v {
        None => Vec::new(),
        Some(v) => {
            if let Some(s) = with_host(|h| h.as_str(v)) {
                s.into_bytes()
            } else {
                bytes_of(v)
            }
        }
    }
}

/// Interpret bytes under `enc` as a Rust string (for `transcode`).
fn bytes_to_string(bytes: &[u8], enc: &str) -> String {
    match enc.to_ascii_lowercase().as_str() {
        "ascii" | "latin1" | "binary" => bytes.iter().map(|b| *b as char).collect(),
        "utf16le" | "utf-16le" | "ucs2" | "ucs-2" => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Encode a Rust string into `enc` bytes (for `transcode`).
fn string_to_bytes(s: &str, enc: &str) -> Vec<u8> {
    match enc.to_ascii_lowercase().as_str() {
        "ascii" | "latin1" | "binary" => s.chars().map(|c| c as u32 as u8).collect(),
        "utf16le" | "utf-16le" | "ucs2" | "ucs-2" => {
            s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
        }
        _ => s.as_bytes().to_vec(),
    }
}

// ── Blob / File ──────────────────────────────────────────────────────────────
//
// A `Blob` is a native object tagged `@@native = "Blob"` (a `File` is `"File"`)
// whose bytes live in `@@bytes`, with `size`/`type` (and `File`'s `name`/
// `lastModified`) as readable data properties. Needs parent construct/instance
// wiring (see final report).

/// Concatenate one Blob-part's bytes: a string contributes its UTF-8 bytes, a
/// Buffer/Blob its raw bytes.
fn part_bytes(v: &Value) -> Vec<u8> {
    match with_host(|h| h.as_str(v)) {
        Some(s) => s.into_bytes(),
        None => bytes_of(v),
    }
}

/// Gather the byte payload from a `BlobPart[]` (the first constructor argument).
fn gather_parts(parts: &Value) -> Vec<u8> {
    let items = with_host(|h| match h.get(parts) {
        Some(JsObj::Array(it)) => it.clone(),
        _ => Vec::new(),
    });
    let mut out = Vec::new();
    for it in &items {
        out.extend(part_bytes(it));
    }
    out
}

/// The `type` string from an options bag (`{ type }`), or "".
fn opt_type(opts: Option<&Value>) -> String {
    match opts {
        Some(v) => with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => p.get("type").map(|x| h.str_of(x)).unwrap_or_default(),
            _ => String::new(),
        }),
        None => String::new(),
    }
}

/// Build a `Blob`/`File` native object with the shared `@@bytes`/`size`/`type`
/// fields; `File` adds `name`/`lastModified`.
fn build_blob(tag: &str, bytes: &[u8], typ: &str, extra: IndexMap<String, Value>) -> Value {
    with_host(|h| {
        let arr = h.new_array(bytes.iter().map(|b| Value::Float(*b as f64)).collect());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag.to_string()));
        m.insert("@@bytes".into(), arr);
        m.insert("size".into(), Value::Float(bytes.len() as f64));
        m.insert("type".into(), h.new_str(typ.to_string()));
        for (k, v) in extra {
            m.insert(k, v);
        }
        h.new_object(m)
    })
}

/// `new Blob(parts[, options])`.
pub fn construct_blob(args: &[Value]) -> Result<Value, String> {
    let bytes = gather_parts(&args.first().cloned().unwrap_or(Value::Undef));
    let typ = opt_type(args.get(1));
    Ok(build_blob("Blob", &bytes, &typ, IndexMap::new()))
}

/// `new File(parts, name[, options])`.
pub fn construct_file(args: &[Value]) -> Result<Value, String> {
    let bytes = gather_parts(&args.first().cloned().unwrap_or(Value::Undef));
    let name = arg_str(args, 1);
    let typ = opt_type(args.get(2));
    // lastModified: options.lastModified or 0.
    let last_modified = args
        .get(2)
        .map(|v| {
            with_host(|h| match h.get(v) {
                Some(JsObj::Object(p)) => p.get("lastModified").map(|x| h.to_number(x)).unwrap_or(0.0),
                _ => 0.0,
            })
        })
        .unwrap_or(0.0);
    let extra = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("name".to_string(), h.new_str(name));
        m.insert("lastModified".to_string(), Value::Float(last_modified));
        m
    });
    Ok(build_blob("File", &bytes, &typ, extra))
}

/// Method names for `Blob`/`File` instances (parent `instance_has_method`).
pub const BLOB_METHODS: &[&str] = &["text", "arrayBuffer", "bytes", "slice"];

/// `Blob`/`File` instance methods. `text`/`arrayBuffer`/`bytes` return already-
/// resolved Promises (Node's async accessors); `slice` returns a new `Blob`.
/// `arrayBuffer`/`bytes` resolve with a `Buffer` (this runtime's byte container)
/// rather than a bare `ArrayBuffer`/`Uint8Array`.
pub fn blob_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let bytes = bytes_of(recv);
    match method {
        "text" => {
            let s = String::from_utf8_lossy(&bytes).into_owned();
            let sv = with_host(|h| h.new_str(s));
            Ok(crate::host::promise_of(&sv))
        }
        "arrayBuffer" | "bytes" => {
            let buf = from_bytes(&bytes);
            Ok(crate::host::promise_of(&buf))
        }
        "slice" => {
            let (s, e) = slice_bounds(args, bytes.len());
            let typ = if args.len() > 2 { arg_str(args, 2) } else { String::new() };
            Ok(build_blob("Blob", &bytes[s..e], &typ, IndexMap::new()))
        }
        _ => Err(crate::host::type_error(&format!("blob.{method} is not a function"))),
    }
}

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
            // A string fill repeats to length n; a numeric fill is a single byte.
            let pat = if args.len() > 1 { fill_pattern(args, 1) } else { vec![0] };
            let bytes: Vec<u8> = if pat.is_empty() {
                vec![0u8; n]
            } else {
                (0..n).map(|i| pat[i % pat.len()]).collect()
            };
            Ok(from_bytes(&bytes))
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
        "includes" | "indexOf" | "lastIndexOf" => {
            let needle = decode_str(&arg_str(args, 0), "utf8");
            // An empty needle matches at 0 (indexOf) / len (lastIndexOf), like Node.
            let pos = if needle.is_empty() {
                Some(if method == "lastIndexOf" { bytes.len() } else { 0 })
            } else if method == "lastIndexOf" {
                bytes.windows(needle.len()).rposition(|w| w == needle.as_slice())
            } else {
                bytes.windows(needle.len()).position(|w| w == needle.as_slice())
            };
            if method == "includes" {
                Ok(Value::Bool(pos.is_some()))
            } else {
                Ok(Value::Float(pos.map(|p| p as f64).unwrap_or(-1.0)))
            }
        }
        // Lexicographic byte comparison → -1 / 0 / 1.
        "compare" => {
            let other = bytes_of(&args.first().cloned().unwrap_or(Value::Undef));
            Ok(Value::Float(match bytes.cmp(&other) {
                std::cmp::Ordering::Less => -1.0,
                std::cmp::Ordering::Equal => 0.0,
                std::cmp::Ordering::Greater => 1.0,
            }))
        }
        // Big-endian / little-endian integer reads.
        "readUInt16BE" => {
            let i = super::arg_num(args, 0).max(0.0) as usize;
            let v = ((*bytes.get(i).unwrap_or(&0) as u16) << 8) | *bytes.get(i + 1).unwrap_or(&0) as u16;
            Ok(Value::Float(v as f64))
        }
        "readUInt16LE" => {
            let i = super::arg_num(args, 0).max(0.0) as usize;
            let v = (*bytes.get(i).unwrap_or(&0) as u16) | ((*bytes.get(i + 1).unwrap_or(&0) as u16) << 8);
            Ok(Value::Float(v as f64))
        }
        // In-place writes: mutate the backing `@@bytes`, return the next offset.
        "writeUInt8" => {
            let mut b = bytes.clone();
            let off = super::arg_num(args, 1).max(0.0) as usize;
            if off < b.len() {
                b[off] = super::arg_num(args, 0) as u8;
            }
            set_bytes(recv, &b);
            Ok(Value::Float((off + 1) as f64))
        }
        "writeUInt16BE" | "writeUInt16LE" => {
            let mut b = bytes.clone();
            let val = super::arg_num(args, 0) as u16;
            let off = super::arg_num(args, 1).max(0.0) as usize;
            let (hi, lo) = ((val >> 8) as u8, (val & 0xff) as u8);
            let (b0, b1) = if method == "writeUInt16BE" { (hi, lo) } else { (lo, hi) };
            if off + 1 < b.len() {
                b[off] = b0;
                b[off + 1] = b1;
            }
            set_bytes(recv, &b);
            Ok(Value::Float((off + 2) as f64))
        }
        // write(string[, offset[, length]][, encoding]) — returns bytes written.
        "write" => {
            let mut b = bytes.clone();
            let src = decode_str(&arg_str(args, 0), "utf8");
            let off = if args.len() > 1 { super::arg_num(args, 1).max(0.0) as usize } else { 0 };
            let mut n = 0;
            for (k, &byte) in src.iter().enumerate() {
                if off + k < b.len() {
                    b[off + k] = byte;
                    n += 1;
                }
            }
            set_bytes(recv, &b);
            Ok(Value::Float(n as f64))
        }
        // fill(value[, start[, end]]) — value is a byte or a repeated string.
        "fill" => {
            let mut b = bytes.clone();
            let len = b.len();
            let start = if args.len() > 1 { super::arg_num(args, 1).max(0.0) as usize } else { 0 };
            let end = if args.len() > 2 { (super::arg_num(args, 2) as usize).min(len) } else { len };
            let pat = fill_pattern(args, 0);
            if !pat.is_empty() {
                for (k, slot) in b[start..end.max(start)].iter_mut().enumerate() {
                    *slot = pat[k % pat.len()];
                }
            }
            set_bytes(recv, &b);
            Ok(recv.clone())
        }
        // copy(target[, targetStart[, sourceStart[, sourceEnd]]]) — returns count.
        "copy" => {
            let target = args.first().cloned().unwrap_or(Value::Undef);
            let mut tb = bytes_of(&target);
            let tstart = if args.len() > 1 { super::arg_num(args, 1).max(0.0) as usize } else { 0 };
            let sstart = if args.len() > 2 { super::arg_num(args, 2).max(0.0) as usize } else { 0 };
            let send = if args.len() > 3 { (super::arg_num(args, 3) as usize).min(bytes.len()) } else { bytes.len() };
            let mut n = 0;
            for (k, &byte) in bytes[sstart..send.max(sstart)].iter().enumerate() {
                if tstart + k < tb.len() {
                    tb[tstart + k] = byte;
                    n += 1;
                }
            }
            set_bytes(&target, &tb);
            Ok(Value::Float(n as f64))
        }
        _ => Err(crate::host::type_error(&format!("buffer.{method} is not a function"))),
    }
}

/// The fill pattern at `args[idx]`: a string's utf-8 bytes, else a single byte.
fn fill_pattern(args: &[Value], idx: usize) -> Vec<u8> {
    match args.get(idx) {
        None => vec![0],
        Some(v) => {
            let is_str = matches!(v, Value::Str(_))
                || with_host(|h| matches!(h.get(v), Some(JsObj::Str(_))));
            if is_str {
                decode_str(&arg_str(args, idx), "utf8")
            } else {
                vec![super::arg_num(args, idx) as u8]
            }
        }
    }
}

/// Overwrite `recv`'s backing `@@bytes` array (for in-place buffer writes).
fn set_bytes(recv: &Value, new: &[u8]) {
    with_host(|h| {
        let arr = match h.get(recv) {
            Some(JsObj::Object(p)) => p.get("@@bytes").cloned(),
            _ => None,
        };
        if let Some(a) = arr {
            if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                *items = new.iter().map(|b| Value::Float(*b as f64)).collect();
            }
        }
    });
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
