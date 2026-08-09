//! Node `Buffer` (global + `require('buffer').Buffer`). A Buffer is a plain
//! object tagged `@@native = "Buffer"` whose bytes live in a hidden `@@bytes`
//! array; `length` is an enumerable data property so `buf.length` reads directly.

use super::{arg_str, from_base64, from_hex, to_base64, to_hex};
use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// The statics on `Buffer`. node v26.7.0's
/// `Object.getOwnPropertyNames(Buffer).filter(n => typeof Buffer[n] === 'function')`
/// reports eleven; this is ten of them.
///
/// Three used to be missing, and the gap was not cosmetic: `safe-buffer`
/// feature-detects `Buffer.from && Buffer.alloc && Buffer.allocUnsafe &&
/// Buffer.allocUnsafeSlow` and, on a miss, exports its own legacy `SafeBuffer`
/// wrapper instead of the real `buffer` module. express's `res.send` takes
/// `Buffer` from `safe-buffer`, so every `res.json()` went through that wrapper
/// and died on its `Buffer(arg, …)` call.
///
/// The eleventh, `copyBytesFrom`, is deliberately absent rather than faked. It
/// copies a typed array's raw BYTES with `offset`/`length` counted in ELEMENTS,
/// which needs a per-kind little-endian serializer for all nine element kinds
/// (typed arrays are stored here as a `@@elems` array of NUMBERS, not bytes).
/// Listing it without that would advertise a method the dispatcher cannot
/// implement — the exact drift this list exists to prevent.
pub const STATIC_METHODS: &[&str] = &[
    "from",
    "alloc",
    "allocUnsafe",
    "allocUnsafeSlow",
    "concat",
    "isBuffer",
    "isEncoding",
    "byteLength",
    "compare",
    "of",
];

/// The methods a `Buffer` instance answers — the surface `instance_call`
/// dispatches, and the set installed as `@proto:Buffer:<m>` thunks on the real
/// `Buffer.prototype` object.
pub const INSTANCE_METHODS: &[&str] = &[
    "toString",
    "set",
    "toJSON",
    "equals",
    "slice",
    "subarray",
    "readUInt8",
    "includes",
    "indexOf",
    "lastIndexOf",
    "write",
    "copy",
    "fill",
    "compare",
    "readUInt16BE",
    "readUInt16LE",
    "writeUInt8",
    "writeUInt16BE",
    "writeUInt16LE",
    "readUInt32BE",
    "readUInt32LE",
    "readInt8",
    "readInt16BE",
    "readInt16LE",
    "readInt32BE",
    "readInt32LE",
    "writeUInt32BE",
    "writeUInt32LE",
    "writeInt32BE",
    "writeInt32LE",
    "at",
    "values",
    "keys",
    "entries",
];

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
                Some(JsObj::Object(p)) => {
                    p.get("lastModified").map(|x| h.to_number(x)).unwrap_or(0.0)
                }
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
            let typ = if args.len() > 2 {
                arg_str(args, 2)
            } else {
                String::new()
            };
            Ok(build_blob("Blob", &bytes[s..e], &typ, IndexMap::new()))
        }
        _ => Err(crate::host::type_error(&format!(
            "blob.{method} is not a function"
        ))),
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
        // The `Uint8Array` view properties a Buffer inherits in Node. `byteLength`
        // equals `length` because the element size is 1.
        m.insert("byteLength".into(), Value::Float(bytes.len() as f64));
        m.insert("byteOffset".into(), Value::Float(0.0));
        m.insert("BYTES_PER_ELEMENT".into(), Value::Float(1.0));
        let obj = h.new_object(m);
        // A Buffer is a real `Uint8Array` subclass instance in Node, so link it
        // to the actual `Buffer.prototype` object rather than leaving it a bare
        // tagged object with no `[[Prototype]]`.
        h.ensure_native_protos();
        if let Some(p) = h.native_proto("Buffer") {
            h.set_proto(&obj, p);
        }
        // The view metadata is real but non-enumerable; V8 keeps `length` and
        // friends off `Object.keys(buf)` (whose own keys are the byte indices).
        for k in ["length", "byteLength", "byteOffset", "BYTES_PER_ELEMENT"] {
            h.hide_prop(&obj, k);
        }
        obj
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

/// The handle of `recv`'s hidden `@@bytes` array, without copying it.
fn bytes_handle(recv: &Value) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@bytes").cloned(),
        _ => None,
    })
}

/// `buf[i]` — the byte at an integer index, or `undefined` past the end.
///
/// Reads through to the single element. Materialising the whole buffer here
/// (the old `bytes_of` call) made one indexed read O(len), so any loop over a
/// Buffer — including a request body arriving through `express.json()` — cost
/// O(len^2).
pub fn byte_get(recv: &Value, index: &str) -> Value {
    let i: usize = match index.parse() {
        Ok(i) => i,
        Err(_) => return Value::Undef,
    };
    let arr = match bytes_handle(recv) {
        Some(a) => a,
        None => return Value::Undef,
    };
    with_host(|h| match h.get(&arr) {
        Some(JsObj::Array(items)) => match items.get(i) {
            Some(v) => Value::Float(h.to_number(v)),
            None => Value::Undef,
        },
        _ => Value::Undef,
    })
}

/// `buf[i] = n` — write one byte (truncated to 8 bits, as a Uint8Array does).
/// Returns whether `recv` was a Buffer and the write landed.
///
/// Writes the single element in place; the old path copied the buffer out,
/// changed one byte, and wrote every byte back.
pub fn byte_set(recv: &Value, index: &str, val: &Value) -> bool {
    if super::native_tag(recv).as_deref() != Some("Buffer") {
        return false;
    }
    let i: usize = match index.parse() {
        Ok(i) => i,
        Err(_) => return false,
    };
    let arr = match bytes_handle(recv) {
        Some(a) => a,
        None => return false,
    };
    let b = with_host(|h| h.to_number(val)) as i64 as u8;
    with_host(|h| {
        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
            // Out of range writes are dropped, not appended.
            if let Some(slot) = items.get_mut(i) {
                *slot = Value::Float(b as f64);
            }
        }
    });
    true
}

pub fn static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "from" => from(args),
        "alloc" => {
            let n = super::arg_num(args, 0).max(0.0) as usize;
            // A string fill repeats to length n; a numeric fill is a single byte.
            let pat = if args.len() > 1 {
                fill_pattern(args, 1)
            } else {
                vec![0]
            };
            let bytes: Vec<u8> = if pat.is_empty() {
                vec![0u8; n]
            } else {
                (0..n).map(|i| pat[i % pat.len()]).collect()
            };
            Ok(from_bytes(&bytes))
        }
        // `allocUnsafeSlow` differs from `allocUnsafe` only in skipping Node's
        // shared pool — an allocator detail with no observable difference here,
        // where every Buffer already owns its bytes.
        "allocUnsafe" | "allocUnsafeSlow" => Ok(from_bytes(&vec![
            0u8;
            super::arg_num(args, 0).max(0.0)
                as usize
        ])),
        "concat" => concat(args),
        // `Buffer.of(...bytes)` — the `%TypedArray%.of` form: each argument is one
        // byte. Measured: `Buffer.of(1,2,3).toString('hex') === '010203'`,
        // `Buffer.of().length === 0`.
        "of" => Ok(from_bytes(
            &args
                .iter()
                .map(|v| crate::host::with_host(|h| h.to_number(v)) as u8)
                .collect::<Vec<u8>>(),
        )),
        // `Buffer.isEncoding(enc)` — case-insensitive over the encodings Node
        // accepts. Measured: `UTF8`, `UTF-8`, `ASCII` and `Hex` are all true;
        // `utf7`, `utf-16be`, `none` and `''` are all false.
        "isEncoding" => Ok(Value::Bool(matches!(
            super::arg_str(args, 0).to_ascii_lowercase().as_str(),
            "utf8"
                | "utf-8"
                | "ucs2"
                | "ucs-2"
                | "utf16le"
                | "utf-16le"
                | "latin1"
                | "binary"
                | "base64"
                | "base64url"
                | "hex"
                | "ascii"
        ))),
        // Static `Buffer.compare(a, b)` — the sort comparator form.
        "compare" => {
            let a = bytes_of(&args.first().cloned().unwrap_or(Value::Undef));
            let b = bytes_of(&args.get(1).cloned().unwrap_or(Value::Undef));
            Ok(Value::Float(match a.cmp(&b) {
                std::cmp::Ordering::Less => -1.0,
                std::cmp::Ordering::Equal => 0.0,
                std::cmp::Ordering::Greater => 1.0,
            }))
        }
        "isBuffer" => Ok(Value::Bool(
            super::native_tag(&args.first().cloned().unwrap_or(Value::Undef)).as_deref()
                == Some("Buffer"),
        )),
        "byteLength" => {
            // A Buffer/typed array/ArrayBuffer argument reports its VIEW size,
            // not its element count and not the length of a stringification.
            if let Some(n) = view_byte_length(&args.first().cloned().unwrap_or(Value::Undef)) {
                return Some(Ok(Value::Float(n)));
            }
            let enc = args
                .get(1)
                .map(|_| arg_str(args, 1))
                .unwrap_or_else(|| "utf8".into());
            Ok(Value::Float(
                decode_str(&arg_str(args, 0), &enc).len() as f64
            ))
        }
        _ => return None,
    })
}

/// The bytes of any byte-like source: a JS array of byte values, another
/// `Buffer`, ANY typed array, or an `ArrayBuffer`. `None` for anything else
/// (a string, which every caller handles with its own encoding rules).
///
/// A `Buffer` is a `Uint8Array` subclass, so every place that accepts a Buffer
/// accepts a typed array too. Each of `Buffer.from`, `Buffer.concat`,
/// `buf.equals` and `buf.indexOf` had its own notion of "byte source" and all
/// four understood `@@bytes` only: passing a `Uint8Array` made `from` fall
/// through to the STRING path and produce the bytes of `"[object Object]"`,
/// made `concat` contribute nothing, and made `equals`/`indexOf` silently miss.
/// Routing them all through one helper is what keeps them from drifting again.
///
/// Element values are truncated to a byte each, which is what Node does:
/// `Buffer.from(new Int32Array([1, 2, 300]))` is `<Buffer 01 02 2c>`.
pub fn bytes_like(v: &Value) -> Option<Vec<u8>> {
    // A Buffer or a typed array of any kind: its ELEMENTS, truncated.
    if let Some(elems) = crate::stdlib::typedarray::elems_of(v) {
        return Some(elems.iter().map(|x| *x as i64 as u8).collect());
    }
    // An `ArrayBuffer` carries only a byte length in this model, so it reads as
    // that many zero bytes. Node would share the memory; nothing here can
    // observe the difference until typed arrays get a real backing store.
    if super::native_tag(v).as_deref() == Some("ArrayBuffer") {
        let n = with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => p.get("byteLength").map(|b| h.to_number(b) as usize),
            _ => None,
        });
        return n.map(|n| vec![0u8; n]);
    }
    // A plain JS array of byte values.
    with_host(|h| match h.get(v) {
        Some(JsObj::Array(items)) => {
            Some(items.iter().map(|x| h.to_number(x) as i64 as u8).collect())
        }
        _ => None,
    })
}

/// The `byteLength` a `Buffer`/typed array/`ArrayBuffer` reports, which is the
/// VIEW's size in bytes rather than its element count — `Buffer.byteLength(new
/// Int32Array([1,2,3]))` is 12, not 3. Verified against node v26.7.0.
fn view_byte_length(v: &Value) -> Option<f64> {
    match super::native_tag(v).as_deref() {
        Some("Buffer") | Some("TypedArray") | Some("ArrayBuffer") => {
            with_host(|h| match h.get(v) {
                Some(JsObj::Object(p)) => p.get("byteLength").map(|b| h.to_number(b)),
                _ => None,
            })
        }
        _ => None,
    }
}

fn from(args: &[Value]) -> Result<Value, String> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    // Any byte-like source (array of bytes, Buffer, typed array, ArrayBuffer).
    if let Some(bytes) = bytes_like(&v) {
        return Ok(from_bytes(&bytes));
    }
    // String with an optional encoding.
    let enc = if args.len() > 1 {
        arg_str(args, 1)
    } else {
        "utf8".into()
    };
    Ok(from_bytes(&decode_str(&arg_str(args, 0), &enc)))
}

fn concat(args: &[Value]) -> Result<Value, String> {
    let list = with_host(
        |h| match h.get(&args.first().cloned().unwrap_or(Value::Undef)) {
            Some(JsObj::Array(items)) => items.clone(),
            _ => Vec::new(),
        },
    );
    let mut out = Vec::new();
    for b in &list {
        // Each part may be a Buffer OR any other typed array.
        out.extend(bytes_like(b).unwrap_or_default());
    }
    Ok(from_bytes(&out))
}

/// Buffer instance methods.
pub fn instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let bytes = bytes_of(recv);
    match method {
        "toString" => {
            let enc = if args.is_empty() {
                "utf8".into()
            } else {
                arg_str(args, 0)
            };
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
            // Comparable against any typed array, not just another Buffer.
            let other = bytes_like(&args.first().cloned().unwrap_or(Value::Undef));
            Ok(Value::Bool(other.is_some_and(|o| bytes == o)))
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
            // The needle is a string, a byte value, or another Buffer.
            let target = args.first().cloned().unwrap_or(Value::Undef);
            let needle = match &target {
                Value::Int(_) | Value::Float(_) => vec![super::arg_num(args, 0) as u8],
                // A Buffer or any other typed array searches by its bytes.
                _ if bytes_like(&target).is_some() => bytes_like(&target).unwrap_or_default(),
                _ => decode_str(&arg_str(args, 0), "utf8"),
            };
            // An empty needle matches at 0 (indexOf) / len (lastIndexOf), like Node.
            let pos = if needle.is_empty() {
                Some(if method == "lastIndexOf" {
                    bytes.len()
                } else {
                    0
                })
            } else if method == "lastIndexOf" {
                bytes
                    .windows(needle.len())
                    .rposition(|w| w == needle.as_slice())
            } else {
                bytes
                    .windows(needle.len())
                    .position(|w| w == needle.as_slice())
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
            let v = ((*bytes.get(i).unwrap_or(&0) as u16) << 8)
                | *bytes.get(i + 1).unwrap_or(&0) as u16;
            Ok(Value::Float(v as f64))
        }
        "readUInt16LE" => {
            let i = super::arg_num(args, 0).max(0.0) as usize;
            let v = (*bytes.get(i).unwrap_or(&0) as u16)
                | ((*bytes.get(i + 1).unwrap_or(&0) as u16) << 8);
            Ok(Value::Float(v as f64))
        }
        // 32-bit and signed reads. `readIntXX` reinterprets the same bytes as
        // two's complement.
        "readUInt32BE" | "readUInt32LE" | "readInt32BE" | "readInt32LE" => {
            let i = super::arg_num(args, 0).max(0.0) as usize;
            let at = |k: usize| *bytes.get(i + k).unwrap_or(&0) as u32;
            let v = if method.ends_with("BE") {
                (at(0) << 24) | (at(1) << 16) | (at(2) << 8) | at(3)
            } else {
                at(0) | (at(1) << 8) | (at(2) << 16) | (at(3) << 24)
            };
            Ok(Value::Float(if method.starts_with("readInt") {
                v as i32 as f64
            } else {
                v as f64
            }))
        }
        "readInt8" => {
            let i = super::arg_num(args, 0).max(0.0) as usize;
            Ok(Value::Float(*bytes.get(i).unwrap_or(&0) as i8 as f64))
        }
        "readInt16BE" | "readInt16LE" => {
            let i = super::arg_num(args, 0).max(0.0) as usize;
            let at = |k: usize| *bytes.get(i + k).unwrap_or(&0) as u16;
            let v = if method.ends_with("BE") {
                (at(0) << 8) | at(1)
            } else {
                at(0) | (at(1) << 8)
            };
            Ok(Value::Float(v as i16 as f64))
        }
        // `buf[i]` by method: `at` accepts a negative index like Array.prototype.at.
        "at" => {
            let i = super::arg_num(args, 0);
            let idx = if i < 0.0 { i + bytes.len() as f64 } else { i };
            Ok(match bytes.get(idx.max(-1.0) as usize) {
                Some(b) if idx >= 0.0 => Value::Float(*b as f64),
                _ => Value::Undef,
            })
        }
        // Iteration helpers: a Buffer is an index/byte collection.
        "values" | "keys" | "entries" => {
            let items: Vec<Value> = with_host(|h| match method {
                "keys" => (0..bytes.len()).map(|i| Value::Float(i as f64)).collect(),
                "values" => bytes.iter().map(|b| Value::Float(*b as f64)).collect(),
                _ => bytes
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        h.new_array(vec![Value::Float(i as f64), Value::Float(*b as f64)])
                    })
                    .collect(),
            });
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
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
            let (b0, b1) = if method == "writeUInt16BE" {
                (hi, lo)
            } else {
                (lo, hi)
            };
            if off + 1 < b.len() {
                b[off] = b0;
                b[off + 1] = b1;
            }
            set_bytes(recv, &b);
            Ok(Value::Float((off + 2) as f64))
        }
        "writeUInt32BE" | "writeUInt32LE" | "writeInt32BE" | "writeInt32LE" => {
            let mut b = bytes.clone();
            let val = super::arg_num(args, 0) as i64 as u32;
            let off = super::arg_num(args, 1).max(0.0) as usize;
            let be = [
                (val >> 24) as u8,
                (val >> 16) as u8,
                (val >> 8) as u8,
                val as u8,
            ];
            let out: Vec<u8> = if method.ends_with("BE") {
                be.to_vec()
            } else {
                be.iter().rev().copied().collect()
            };
            if off + 3 < b.len() {
                b[off..off + 4].copy_from_slice(&out);
            }
            set_bytes(recv, &b);
            Ok(Value::Float((off + 4) as f64))
        }
        // write(string[, offset[, length]][, encoding]) — returns bytes written.
        "write" => {
            let mut b = bytes.clone();
            let src = decode_str(&arg_str(args, 0), "utf8");
            let off = if args.len() > 1 {
                super::arg_num(args, 1).max(0.0) as usize
            } else {
                0
            };
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
            let start = if args.len() > 1 {
                super::arg_num(args, 1).max(0.0) as usize
            } else {
                0
            };
            let end = if args.len() > 2 {
                (super::arg_num(args, 2) as usize).min(len)
            } else {
                len
            };
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
            let tstart = if args.len() > 1 {
                super::arg_num(args, 1).max(0.0) as usize
            } else {
                0
            };
            let sstart = if args.len() > 2 {
                super::arg_num(args, 2).max(0.0) as usize
            } else {
                0
            };
            let send = if args.len() > 3 {
                (super::arg_num(args, 3) as usize).min(bytes.len())
            } else {
                bytes.len()
            };
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
        // `TypedArray.prototype.set(source[, offset])` — copy `source`'s bytes in
        // at `offset`, throwing when they would not fit (as Node does).
        "set" => {
            let src = bytes_of(&args.first().cloned().unwrap_or(Value::Undef));
            let offset = if args.len() > 1 {
                super::arg_num(args, 1).max(0.0) as usize
            } else {
                0
            };
            if offset + src.len() > bytes.len() {
                return Err(crate::host::range_error("offset is out of bounds"));
            }
            let mut out = bytes.clone();
            out[offset..offset + src.len()].copy_from_slice(&src);
            set_bytes(recv, &out);
            Ok(Value::Undef)
        }
        // A Buffer IS a `Uint8Array`, so anything it does not implement itself
        // falls through to the shared typed-array behaviour it inherits —
        // `every`, `map`, `filter`, `forEach`, `reduce`, `sort` and the rest.
        // These used to READ as functions (the thunks resolve through
        // `Uint8Array.prototype` on the chain) but throw on call, because
        // dispatch landed here and stopped. A method the typed arrays do not
        // have either still reports the Buffer-shaped error below.
        _ if crate::stdlib::typedarray::PROTOTYPE_METHODS.contains(&method) => {
            crate::stdlib::typedarray::instance_call(recv, method, args)
        }
        _ => Err(crate::host::type_error(&format!(
            "buffer.{method} is not a function"
        ))),
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
    let s = if args.is_empty() {
        0
    } else {
        norm(super::arg_num(args, 0))
    };
    let e = if args.len() < 2 {
        len
    } else {
        norm(super::arg_num(args, 1))
    };
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

pub(crate) fn encode_bytes(bytes: &[u8], enc: &str) -> String {
    match enc.to_ascii_lowercase().as_str() {
        "hex" => to_hex(bytes),
        "base64" | "base64url" => to_base64(bytes),
        "ascii" | "latin1" | "binary" => bytes.iter().map(|b| *b as char).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}
