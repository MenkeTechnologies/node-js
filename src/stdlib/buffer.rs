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
    "writeInt8",
    "writeInt16BE",
    "writeInt16LE",
    "readFloatBE",
    "readFloatLE",
    "writeFloatBE",
    "writeFloatLE",
    "readDoubleBE",
    "readDoubleLE",
    "writeDoubleBE",
    "writeDoubleLE",
    "readBigInt64BE",
    "readBigInt64LE",
    "readBigUInt64BE",
    "readBigUInt64LE",
    "writeBigInt64BE",
    "writeBigInt64LE",
    "writeBigUInt64BE",
    "writeBigUInt64LE",
    "readIntBE",
    "readIntLE",
    "readUIntBE",
    "readUIntLE",
    "writeIntBE",
    "writeIntLE",
    "writeUIntBE",
    "writeUIntLE",
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
    "swap16",
    "swap32",
    "swap64",
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
///
/// `transcode` is ICU, not `Buffer.from`, so its `ascii` arm SUBSTITUTES rather
/// than truncates: node renders `transcode(Buffer.from('aÿ'),'utf8','ascii')` as
/// `61 3f` — an unrepresentable character becomes `?`.
fn string_to_bytes(s: &str, enc: &str) -> Vec<u8> {
    match enc.to_ascii_lowercase().as_str() {
        "ascii" => s
            .chars()
            .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
            .collect(),
        "latin1" | "binary" => s.chars().map(|c| c as u32 as u8).collect(),
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

/// A Buffer that is a WINDOW onto an existing `ArrayBuffer`'s store, sharing
/// its bytes rather than copying them.
pub fn share_array_buffer(ab: &Value, off: usize, len: usize) -> Value {
    let store = crate::stdlib::typedarray::buffer_store(ab);
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Buffer"));
        m.insert(
            "@@bytes".into(),
            store.unwrap_or_else(|| h.new_array(Vec::new())),
        );
        m.insert("@@buffer".into(), ab.clone());
        m.insert("buffer".into(), ab.clone());
        m.insert("length".into(), Value::Float(len as f64));
        m.insert("byteLength".into(), Value::Float(len as f64));
        m.insert("byteOffset".into(), Value::Float(off as f64));
        m.insert("BYTES_PER_ELEMENT".into(), Value::Float(1.0));
        let obj = h.new_object(m);
        h.ensure_native_protos();
        if let Some(p) = h.native_proto("Buffer") {
            h.set_proto(&obj, p);
        }
        for k in [
            "buffer",
            "length",
            "byteLength",
            "byteOffset",
            "BYTES_PER_ELEMENT",
        ] {
            h.hide_prop(&obj, k);
        }
        obj
    })
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

/// A Buffer's window onto its byte store as `(byteOffset, length)`.
///
/// A Buffer built from its own bytes spans the whole store, so this is
/// `(0, len)` and every accessor behaves as it did. One built over an
/// `ArrayBuffer` SHARES that buffer's array and may start partway into it, which
/// is what makes `Buffer.from(ab, 2, 2)` alias rather than copy.
fn window(recv: &Value) -> (usize, usize) {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => {
            let off = p.get("byteOffset").map(|o| h.to_number(o)).unwrap_or(0.0) as usize;
            let store = match p.get("@@bytes").and_then(|v| h.get(v)) {
                Some(JsObj::Array(items)) => items.len(),
                _ => 0,
            };
            let len = p
                .get("length")
                .map(|l| h.to_number(l) as usize)
                .unwrap_or(store);
            (off.min(store), len.min(store.saturating_sub(off)))
        }
        _ => (0, 0),
    })
}

fn bytes_of(recv: &Value) -> Vec<u8> {
    let (off, len) = window(recv);
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|v| h.get(v)) {
            Some(JsObj::Array(items)) => items[off..off + len]
                .iter()
                .map(|v| h.to_number(v) as u8)
                .collect(),
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
    let (off, len) = window(recv);
    if i >= len {
        return Value::Undef;
    }
    with_host(|h| match h.get(&arr) {
        Some(JsObj::Array(items)) => match items.get(off + i) {
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
    // Indexed through the Buffer's WINDOW, so one sharing an ArrayBuffer writes
    // where its own view starts rather than at the store's origin.
    let (off, len) = window(recv);
    if i >= len {
        return true;
    }
    with_host(|h| {
        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
            // Out of range writes are dropped, not appended.
            if let Some(slot) = items.get_mut(off + i) {
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
            // alloc(size[, fill[, encoding]]).
            let pat = if args.len() > 1 {
                let enc = if args.len() > 2 {
                    arg_str(args, 2)
                } else {
                    "utf8".into()
                };
                fill_pattern(args, 1, &enc)
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
/// The bytes of a byte VIEW — a `Buffer`, any typed array, a `DataView` or an
/// `ArrayBuffer` — and nothing else.
///
/// Narrower than `bytes_like`, which also accepts a plain JS array of byte
/// values. The APIs that take "a Buffer, TypedArray, DataView or string" want
/// exactly this set: an array argument is a TypeError in node, so widening to
/// it would trade one divergence for another.
pub fn view_bytes(v: &Value) -> Option<Vec<u8>> {
    match super::native_tag(v).as_deref() {
        // A `DataView` exposes no elements, so its bytes come straight from the
        // window it holds onto its buffer. It is deliberately absent from
        // `bytes_like`: `Buffer.from(dataView)` is EMPTY in node, because
        // `Buffer.from` wants something array-like and a DataView has no
        // `length`.
        Some("DataView") => {
            let n = with_host(|h| match h.get(v) {
                Some(JsObj::Object(p)) => p.get("byteLength").map(|l| h.to_number(l) as usize),
                _ => None,
            })
            .unwrap_or(0);
            crate::stdlib::typedarray::view_bytes(v, 0, n)
        }
        Some("Buffer") | Some("TypedArray") | Some("ArrayBuffer") => bytes_like(v),
        _ => None,
    }
}

pub fn bytes_like(v: &Value) -> Option<Vec<u8>> {
    // A Buffer or a typed array of any kind: its ELEMENTS, truncated.
    if let Some(elems) = crate::stdlib::typedarray::elems_of(v) {
        return Some(elems.iter().map(|x| *x as i64 as u8).collect());
    }
    // An `ArrayBuffer` is handled by `from`, which SHARES its store rather than
    // copying — reaching here would produce a detached copy.
    if super::native_tag(v).as_deref() == Some("ArrayBuffer") {
        return Some(crate::stdlib::typedarray::buffer_bytes_snapshot(v).unwrap_or_default());
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
    // `Buffer.from(arrayBuffer[, byteOffset[, length]])` SHARES the buffer's
    // memory — a write through the Buffer is visible through every other view.
    // It used to copy zero bytes, because an ArrayBuffer had no store at all.
    if super::native_tag(&v).as_deref() == Some("ArrayBuffer") {
        let total = crate::stdlib::typedarray::buffer_byte_length(&v);
        let off = (super::arg_num(args, 1).max(0.0) as usize).min(total);
        let len = match args.get(2) {
            Some(Value::Undef) | None => total - off,
            Some(_) => (super::arg_num(args, 2).max(0.0) as usize).min(total - off),
        };
        return Ok(share_array_buffer(&v, off, len));
    }
    // Any byte-like source (array of bytes, Buffer, typed array).
    if let Some(bytes) = bytes_like(&v) {
        return Ok(from_bytes(&bytes));
    }
    // A string, with an optional encoding.
    if with_host(|h| h.as_str(&v)).is_some() || matches!(v, Value::Str(_)) {
        let enc = if args.len() > 1 {
            arg_str(args, 1)
        } else {
            "utf8".into()
        };
        return Ok(from_bytes(&decode_str(&arg_str(args, 0), &enc)));
    }
    // A `DataView` yields an EMPTY buffer: `Buffer.from` wants something
    // array-like, and a DataView carries `byteLength` but no `length`.
    if super::native_tag(&v).as_deref() == Some("DataView") {
        return Ok(from_bytes(&[]));
    }
    // An ARRAY-LIKE object — anything with a numeric `length` — contributes its
    // index properties, each coerced to a byte. `Buffer.from({length: 2})` is
    // two zero bytes in node.
    if matches!(v, Value::Obj(_)) {
        let len = crate::builtins::get_property(&v, "length").unwrap_or(Value::Undef);
        if !matches!(len, Value::Undef) {
            let n = with_host(|h| h.to_number(&len));
            if n.is_finite() && n >= 0.0 {
                let n = n as usize;
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    let e =
                        crate::builtins::get_property(&v, &i.to_string()).unwrap_or(Value::Undef);
                    let b = with_host(|h| h.to_number(&e));
                    out.push(if b.is_finite() { b as i64 as u8 } else { 0 });
                }
                return Ok(from_bytes(&out));
            }
        }
    }
    // Anything else is a TypeError, not a stringification: `Buffer.from(5)`
    // used to produce the single byte `0x35` (the digit "5") and
    // `Buffer.from(null)` the four bytes of `"null"`.
    Err(crate::host::plain_coded_error(
        "TypeError",
        "ERR_INVALID_ARG_TYPE",
        &format!(
            "The first argument must be of type string or an instance of \
Buffer, ArrayBuffer, or Array or an Array-like Object. Received {}",
            received_label(&v)
        ),
    ))
}

/// How node names a rejected `Buffer.from` argument: `null`, `type number (5)`,
/// or `an instance of Object`.
fn received_label(v: &Value) -> String {
    if matches!(v, Value::Undef) {
        return "undefined".into();
    }
    if with_host(|h| h.is_null(v)) {
        return "null".into();
    }
    let ty = with_host(|h| h.type_of(v));
    if ty == "object" || ty == "function" {
        let ctor = with_host(|h| h.ctor_name(v));
        let ctor = if ctor.is_empty() {
            "Object".into()
        } else {
            ctor
        };
        return format!("an instance of {ctor}");
    }
    let shown = with_host(|h| h.inspect(v));
    format!("type {ty} ({shown})")
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
        // toString([encoding[, start[, end]]]) — the range was ignored, so every
        // partial read (`buf.toString('utf8', 1, 3)`) returned the WHOLE buffer.
        "toString" => {
            let enc = match args.first() {
                None | Some(Value::Undef) => "utf8".into(),
                _ => arg_str(args, 0),
            };
            let len = bytes.len();
            let clamp = |i: usize| -> usize {
                let n = super::arg_num(args, i);
                if n.is_nan() {
                    0
                } else {
                    n.clamp(0.0, len as f64) as usize
                }
            };
            let start = if args.len() > 1 { clamp(1) } else { 0 };
            let end = if args.len() > 2 { clamp(2) } else { len };
            // An inverted range is empty, not reversed.
            let slice = if start < end {
                &bytes[start..end]
            } else {
                &[][..]
            };
            Ok(with_host(|h| h.new_str(encode_bytes(slice, &enc))))
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
            let i = read_offset(args, 1, bytes.len())?;
            Ok(Value::Float(bytes[i] as f64))
        }
        // indexOf/lastIndexOf/includes(value[, byteOffset][, encoding]) — both
        // trailing arguments used to be ignored, so a search always started at 0
        // and always read the needle as UTF-8.
        "includes" | "indexOf" | "lastIndexOf" => {
            let len = bytes.len();
            let last = method == "lastIndexOf";
            // `byteOffset` is a string when it is really the encoding.
            let (from, enc) = match args.get(1) {
                None | Some(Value::Undef) => (None, arg_str(args, 2)),
                Some(v) if with_host(|h| h.as_str(v)).is_some() => (None, arg_str(args, 1)),
                _ => (Some(super::arg_num(args, 1)), arg_str(args, 2)),
            };
            let enc = if enc.is_empty() { "utf8".into() } else { enc };
            // The needle is a string, a byte value, or another Buffer.
            let target = args.first().cloned().unwrap_or(Value::Undef);
            let needle = match &target {
                Value::Int(_) | Value::Float(_) => vec![super::arg_num(args, 0) as u8],
                // A Buffer or any other typed array searches by its bytes.
                _ if bytes_like(&target).is_some() => bytes_like(&target).unwrap_or_default(),
                _ => decode_str(&arg_str(args, 0), &enc),
            };
            // A negative offset counts back from the end; NaN is 0. Out of range
            // means "no room to match" forwards, and "whole buffer" backwards.
            let from = from.map(|n| {
                if n.is_nan() {
                    0
                } else if n < 0.0 {
                    (len as f64 + n).max(0.0) as usize
                } else {
                    (n as usize).min(len)
                }
            });
            // An empty needle matches at the offset itself, clamped to the length.
            let pos = if needle.is_empty() {
                Some(from.unwrap_or(if last { len } else { 0 }).min(len))
            } else if last {
                // lastIndexOf searches at or before the offset, so the match may
                // start at `from` itself and run past it.
                let hi = (from.unwrap_or(len) + needle.len()).min(len);
                bytes[..hi]
                    .windows(needle.len())
                    .rposition(|w| w == needle.as_slice())
            } else {
                let lo = from.unwrap_or(0);
                bytes[lo..]
                    .windows(needle.len())
                    .position(|w| w == needle.as_slice())
                    .map(|p| p + lo)
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
            let i = read_offset(args, 2, bytes.len())?;
            let v = ((bytes[i] as u16) << 8) | bytes[i + 1] as u16;
            Ok(Value::Float(v as f64))
        }
        "readUInt16LE" => {
            let i = read_offset(args, 2, bytes.len())?;
            let v = (bytes[i] as u16) | ((bytes[i + 1] as u16) << 8);
            Ok(Value::Float(v as f64))
        }
        // 32-bit and signed reads. `readIntXX` reinterprets the same bytes as
        // two's complement.
        "readUInt32BE" | "readUInt32LE" | "readInt32BE" | "readInt32LE" => {
            let i = read_offset(args, 4, bytes.len())?;
            let at = |k: usize| bytes[i + k] as u32;
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
            let i = read_offset(args, 1, bytes.len())?;
            Ok(Value::Float(bytes[i] as i8 as f64))
        }
        "readInt16BE" | "readInt16LE" => {
            let i = read_offset(args, 2, bytes.len())?;
            let at = |k: usize| bytes[i + k] as u16;
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
        // IEEE-754 reads/writes. `f32`/`f64` go through their raw bit patterns,
        // so the endianness handling is the same byte reversal as the integers.
        "readFloatBE" | "readFloatLE" => {
            let i = read_offset(args, 4, bytes.len())?;
            let mut raw = [0u8; 4];
            raw.copy_from_slice(&bytes[i..i + 4]);
            if method.ends_with("LE") {
                raw.reverse();
            }
            Ok(Value::Float(f32::from_be_bytes(raw) as f64))
        }
        "readDoubleBE" | "readDoubleLE" => {
            let i = read_offset(args, 8, bytes.len())?;
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes[i..i + 8]);
            if method.ends_with("LE") {
                raw.reverse();
            }
            Ok(Value::Float(f64::from_be_bytes(raw)))
        }
        "writeFloatBE" | "writeFloatLE" => {
            let mut raw = (super::arg_num(args, 0) as f32).to_be_bytes();
            if method.ends_with("LE") {
                raw.reverse();
            }
            let off = super::arg_num(args, 1).max(0.0) as usize;
            store_bytes(recv, &bytes, off, &raw)?;
            Ok(Value::Float((off + 4) as f64))
        }
        "writeDoubleBE" | "writeDoubleLE" => {
            let mut raw = super::arg_num(args, 0).to_be_bytes();
            if method.ends_with("LE") {
                raw.reverse();
            }
            let off = super::arg_num(args, 1).max(0.0) as usize;
            store_bytes(recv, &bytes, off, &raw)?;
            Ok(Value::Float((off + 8) as f64))
        }
        // 64-bit integers, which exceed `f64`'s exact range and so are BigInts
        // on both sides.
        "readBigInt64BE" | "readBigInt64LE" | "readBigUInt64BE" | "readBigUInt64LE" => {
            let i = read_offset(args, 8, bytes.len())?;
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes[i..i + 8]);
            if method.ends_with("LE") {
                raw.reverse();
            }
            let n = if method.starts_with("readBigInt") {
                num_bigint::BigInt::from(i64::from_be_bytes(raw))
            } else {
                num_bigint::BigInt::from(u64::from_be_bytes(raw))
            };
            Ok(with_host(|h| h.alloc(JsObj::BigInt(n))))
        }
        "writeBigInt64BE" | "writeBigInt64LE" | "writeBigUInt64BE" | "writeBigUInt64LE" => {
            let v = args.first().cloned().unwrap_or(Value::Undef);
            let n = with_host(|h| match h.get(&v) {
                Some(JsObj::BigInt(b)) => b.clone(),
                _ => num_bigint::BigInt::from(h.to_number(&v) as i64),
            });
            // Both signed and unsigned store the same 64 bits; the sign is only
            // a question of how they are read back.
            let bits = num_traits::ToPrimitive::to_i64(&n)
                .map(|x| x as u64)
                .or_else(|| num_traits::ToPrimitive::to_u64(&n))
                .unwrap_or(0);
            let mut raw = bits.to_be_bytes();
            if method.ends_with("LE") {
                raw.reverse();
            }
            let off = super::arg_num(args, 1).max(0.0) as usize;
            store_bytes(recv, &bytes, off, &raw)?;
            Ok(Value::Float((off + 8) as f64))
        }
        // The variable-width family: `byteLength` is an argument (1..=6), which
        // is why these cannot share the fixed-width arms above.
        "readIntBE" | "readIntLE" | "readUIntBE" | "readUIntLE" => {
            let off = super::arg_num(args, 0).max(0.0) as usize;
            let width = (super::arg_num(args, 1).max(1.0) as usize).min(6);
            if off + width > bytes.len() {
                return Err(range_error_out_of_bounds());
            }
            let mut acc: u64 = 0;
            for k in 0..width {
                let b = if method.ends_with("BE") {
                    bytes[off + k]
                } else {
                    bytes[off + width - 1 - k]
                };
                acc = (acc << 8) | b as u64;
            }
            let signed = method.starts_with("readInt");
            let out = if signed {
                // Sign-extend from the top bit of the width-th byte.
                let shift = 64 - (width * 8);
                ((acc << shift) as i64 >> shift) as f64
            } else {
                acc as f64
            };
            Ok(Value::Float(out))
        }
        "writeIntBE" | "writeIntLE" | "writeUIntBE" | "writeUIntLE" => {
            let val = super::arg_num(args, 0) as i64 as u64;
            let off = super::arg_num(args, 1).max(0.0) as usize;
            let width = (super::arg_num(args, 2).max(1.0) as usize).min(6);
            let mut raw: Vec<u8> = (0..width)
                .map(|k| (val >> (8 * (width - 1 - k))) as u8)
                .collect();
            if method.ends_with("LE") {
                raw.reverse();
            }
            store_bytes(recv, &bytes, off, &raw)?;
            Ok(Value::Float((off + width) as f64))
        }
        "writeInt8" => {
            let off = super::arg_num(args, 1).max(0.0) as usize;
            store_bytes(recv, &bytes, off, &[super::arg_num(args, 0) as i64 as u8])?;
            Ok(Value::Float((off + 1) as f64))
        }
        "writeInt16BE" | "writeInt16LE" => {
            let val = super::arg_num(args, 0) as i64 as u16;
            let mut raw = val.to_be_bytes();
            if method.ends_with("LE") {
                raw.reverse();
            }
            let off = super::arg_num(args, 1).max(0.0) as usize;
            store_bytes(recv, &bytes, off, &raw)?;
            Ok(Value::Float((off + 2) as f64))
        }
        // In-place writes: mutate the backing `@@bytes`, return the next offset.
        "writeUInt8" => {
            let off = super::arg_num(args, 1).max(0.0) as usize;
            store_bytes(recv, &bytes, off, &[super::arg_num(args, 0) as u8])?;
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
            let _ = &mut b;
            store_bytes(recv, &bytes, off, &[b0, b1])?;
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
            let _ = &mut b;
            store_bytes(recv, &bytes, off, &out)?;
            Ok(Value::Float((off + 4) as f64))
        }
        // write(string[, offset[, length]][, encoding]) — returns bytes written.
        // `length` and `encoding` used to be ignored entirely: every write was
        // UTF-8 and ran to the end of the buffer.
        "write" => {
            let mut b = bytes.clone();
            let Some((off, max, enc)) = write_args(args, b.len()) else {
                return Err(crate::host::range_error(&format!(
                    "The value of \"offset\" is out of range. It must be >= 0 && <= {}. Received {}",
                    b.len(),
                    super::arg_num(args, 1)
                )));
            };
            let src = truncate_chars(&arg_str(args, 0), &enc, max);
            let n = src.len().min(b.len().saturating_sub(off));
            b[off..off + n].copy_from_slice(&src[..n]);
            set_bytes(recv, &b);
            Ok(Value::Float(n as f64))
        }
        // swap16/32/64 reverse each 2/4/8-byte group IN PLACE and return the
        // same Buffer, so `b.swap16()` mutates `b`. A length that is not a whole
        // number of groups is a RangeError rather than a partial swap.
        "swap16" | "swap32" | "swap64" => {
            let group = match method {
                "swap16" => 2,
                "swap32" => 4,
                _ => 8,
            };
            if bytes.len() % group != 0 {
                return Err(crate::host::coded_error(
                    "RangeError",
                    "ERR_INVALID_BUFFER_SIZE",
                    &format!("Buffer size must be a multiple of {}-bits", group * 8),
                ));
            }
            let mut b = bytes.clone();
            for c in b.chunks_mut(group) {
                c.reverse();
            }
            set_bytes(recv, &b);
            Ok(recv.clone())
        }
        // fill(value[, start[, end]]) — value is a byte or a repeated string.
        // fill(value[, offset[, end]][, encoding]). A STRING in the `offset` or
        // `end` slot is the encoding, and node then resets the range to the
        // whole buffer rather than shifting the remaining arguments left — so
        // `fill('41','hex',1,3)` fills all of it, not `1..3`.
        "fill" => {
            let mut b = bytes.clone();
            let len = b.len();
            let (start, end, enc) = if arg_is_str(args, 1) {
                (0, len, arg_str(args, 1))
            } else if arg_is_str(args, 2) {
                let s = (super::arg_num(args, 1).max(0.0) as usize).min(len);
                (s, len, arg_str(args, 2))
            } else {
                let s = if args.len() > 1 {
                    (super::arg_num(args, 1).max(0.0) as usize).min(len)
                } else {
                    0
                };
                let e = if args.len() > 2 {
                    (super::arg_num(args, 2).max(0.0) as usize).min(len)
                } else {
                    len
                };
                let enc = if args.len() > 3 {
                    arg_str(args, 3)
                } else {
                    "utf8".into()
                };
                (s, e, enc)
            };
            let pat = fill_pattern(args, 0, &enc);
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
fn fill_pattern(args: &[Value], idx: usize, enc: &str) -> Vec<u8> {
    match args.get(idx) {
        None => vec![0],
        Some(v) => {
            let is_str = matches!(v, Value::Str(_))
                || with_host(|h| matches!(h.get(v), Some(JsObj::Str(_))));
            if is_str {
                decode_str(&arg_str(args, idx), enc)
            } else {
                vec![super::arg_num(args, idx) as u8]
            }
        }
    }
}

/// Whether `args[i]` is a string — the test that separates a positional
/// `offset`/`end` from a trailing `encoding` in `fill`/`write`/`indexOf`.
fn arg_is_str(args: &[Value], i: usize) -> bool {
    args.get(i)
        .is_some_and(|v| with_host(|h| h.as_str(v)).is_some())
}

/// Overwrite `recv`'s backing `@@bytes` array (for in-place buffer writes).
/// Write `out` into the buffer's backing bytes at `off`.
///
/// A write that would run past the end is a `RangeError`, as in node. The
/// fixed-width writers used to skip it silently and still return the advanced
/// offset, so a caller writing past the end was told it had succeeded and the
/// bytes were simply lost.
fn store_bytes(recv: &Value, bytes: &[u8], off: usize, out: &[u8]) -> Result<(), String> {
    if off + out.len() > bytes.len() {
        return Err(range_error_out_of_bounds());
    }
    let mut b = bytes.to_vec();
    b[off..off + out.len()].copy_from_slice(out);
    set_bytes(recv, &b);
    Ok(())
}

fn range_error_out_of_bounds() -> String {
    crate::host::plain_coded_error(
        "RangeError",
        "ERR_OUT_OF_RANGE",
        "Attempt to access memory outside buffer bounds",
    )
}

fn set_bytes(recv: &Value, new: &[u8]) {
    let (off, _) = window(recv);
    with_host(|h| {
        let arr = match h.get(recv) {
            Some(JsObj::Object(p)) => p.get("@@bytes").cloned(),
            _ => None,
        };
        if let Some(a) = arr {
            if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                // Only the window is rewritten, so a Buffer sharing an
                // ArrayBuffer never clobbers bytes outside its own view.
                for (i, b) in new.iter().enumerate() {
                    if off + i < items.len() {
                        items[off + i] = Value::Float(*b as f64);
                    }
                }
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

/// String → bytes under a Node buffer encoding.
///
/// The `utf16le` family and `base64url` used to fall through to the UTF-8 arm,
/// which is silent corruption rather than a missing feature: `Buffer.from('abc',
/// 'utf16le')` produced the 3 bytes `616263` instead of node's 6 bytes
/// `610062006300`, and every `byteLength`/`write`/`fill` that funnels through
/// here inherited the wrong count.
pub(crate) fn decode_str(s: &str, enc: &str) -> Vec<u8> {
    match enc.to_ascii_lowercase().as_str() {
        "hex" => from_hex(s),
        "base64" | "base64url" => from_base64(s),
        // One byte per UTF-16 CODE UNIT, not per code point: node writes
        // `Buffer.from("\u{1D4B3}","latin1")` as the low bytes of the surrogate
        // pair (`35 b3`), two bytes, not the one low byte of U+1D4B3. Encoding
        // does NOT mask to 7 bits even for `ascii` — only decoding does.
        "ascii" | "latin1" | "binary" => s.encode_utf16().map(|u| u as u8).collect(),
        // A JS string IS UTF-16, so this encoding is the identity on its code
        // units, written little-endian — not a transcode of the UTF-8 bytes.
        "utf16le" | "utf-16le" | "ucs2" | "ucs-2" => {
            s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
        }
        _ => s.as_bytes().to_vec(),
    }
}

pub(crate) fn encode_bytes(bytes: &[u8], enc: &str) -> String {
    match enc.to_ascii_lowercase().as_str() {
        "hex" => to_hex(bytes),
        "base64" => to_base64(bytes),
        "base64url" => super::to_base64url(bytes),
        // Decoding `ascii` masks off the high bit (node: `Buffer.from([0xff])
        // .toString('ascii')` is `U+007F`); `latin1` keeps the whole byte.
        "ascii" => bytes.iter().map(|b| (*b & 0x7f) as char).collect(),
        "latin1" | "binary" => bytes.iter().map(|b| *b as char).collect(),
        // A trailing odd byte has no code unit and is dropped, as node does.
        "utf16le" | "utf-16le" | "ucs2" | "ucs-2" => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            crate::utf16::to_string_lossy(&units)
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Resolve the `(offset, length, encoding)` triple of `buf.write(string[,
/// offset[, length]][, encoding])`, whose trailing arguments are positional-
/// or-encoding depending on their runtime type (node's own `Buffer.prototype
/// .write` does exactly this dispatch).
///
/// `args[0]` is the string; this reads from `args[1]` on. Returns `None` when
/// the offset is out of range, which is a `RangeError` at the call site.
fn write_args(args: &[Value], len: usize) -> Option<(usize, usize, String)> {
    let num = |i: usize| super::arg_num(args, i);
    // write(string) / write(string, encoding)
    if args.len() < 2 {
        return Some((0, len, "utf8".into()));
    }
    if arg_is_str(args, 1) {
        return Some((0, len, arg_str(args, 1)));
    }
    let off = num(1);
    if !(0.0..=len as f64).contains(&off) {
        return None;
    }
    let off = off as usize;
    // write(string, offset) / write(string, offset, encoding)
    if args.len() < 3 {
        return Some((off, len - off, "utf8".into()));
    }
    if arg_is_str(args, 2) {
        return Some((off, len - off, arg_str(args, 2)));
    }
    let max = len - off;
    let n = (num(2).max(0.0) as usize).min(max);
    let enc = if args.len() > 3 {
        arg_str(args, 3)
    } else {
        "utf8".into()
    };
    Some((off, n, enc))
}

/// Truncate `bytes` to at most `max`, never splitting a multi-byte character.
///
/// `buf.write` writes whole characters only: node reports 2, not 4, for
/// `Buffer.alloc(4).write('é€')` — the 2-byte `é` fits and the 3-byte `€` is
/// dropped whole rather than half-written. Only the variable-width encodings
/// need this; the fixed-width ones are already aligned by construction.
fn truncate_chars(s: &str, enc: &str, max: usize) -> Vec<u8> {
    let bytes = decode_str(s, enc);
    if bytes.len() <= max {
        return bytes;
    }
    match enc.to_ascii_lowercase().as_str() {
        "utf16le" | "utf-16le" | "ucs2" | "ucs-2" => bytes[..max - max % 2].to_vec(),
        "hex" | "base64" | "base64url" | "ascii" | "latin1" | "binary" => bytes[..max].to_vec(),
        _ => {
            let mut end = max;
            while end > 0 && (bytes[end] & 0xC0) == 0x80 {
                end -= 1;
            }
            bytes[..end].to_vec()
        }
    }
}

/// The validated byte offset for a fixed-width `buf.readXxx(offset)`.
///
/// Every read used to be `arg_num(args, 0).max(0.0) as usize` with
/// `bytes.get(i).unwrap_or(&0)` behind it, so an out-of-range read silently
/// produced zeroes instead of throwing — and a negative offset silently became
/// 0. Node raises one of two coded errors, and which one depends on whether the
/// buffer could hold the value at all:
///
/// ```text
/// Buffer.alloc(4).readUInt8(5)     RangeError [ERR_OUT_OF_RANGE]
///     The value of "offset" is out of range. It must be >= 0 and <= 3. Received 5
/// Buffer.alloc(0).readUInt8(0)     RangeError [ERR_BUFFER_OUT_OF_BOUNDS]
///     Attempt to access memory outside buffer bounds
/// ```
fn read_offset(args: &[Value], size: usize, len: usize) -> Result<usize, String> {
    if len < size {
        return Err(crate::host::coded_error(
            "RangeError",
            "ERR_BUFFER_OUT_OF_BOUNDS",
            "Attempt to access memory outside buffer bounds",
        ));
    }
    let max = len - size;
    let raw = match args.first() {
        None | Some(Value::Undef) => 0.0,
        Some(_) => super::arg_num(args, 0),
    };
    // A non-integer offset is its own rejection, with a different tail than the
    // range one — `readUInt8(1.5)` is "It must be an integer", not a bound.
    if raw.fract() != 0.0 || raw.is_nan() {
        return Err(crate::host::coded_error(
            "RangeError",
            "ERR_OUT_OF_RANGE",
            &format!(
                "The value of \"offset\" is out of range. It must be an integer. Received {}",
                crate::host::fmt_number(raw)
            ),
        ));
    }
    let off = raw;
    if off < 0.0 || off > max as f64 {
        return Err(crate::host::coded_error(
            "RangeError",
            "ERR_OUT_OF_RANGE",
            &format!(
                "The value of \"offset\" is out of range. It must be >= 0 and <= {max}. \
                 Received {}",
                crate::host::fmt_number(raw)
            ),
        ));
    }
    Ok(off as usize)
}
