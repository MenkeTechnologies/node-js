//! Node `Buffer` (global + `require('buffer').Buffer`). A Buffer is a plain
//! object tagged `@@native = "Buffer"` whose bytes live in a hidden `@@bytes`
//! array SHARED with its `ArrayBuffer` (`@@buffer`, read as `buf.buffer`); the
//! Buffer is a window (`byteOffset`/`length`) onto it, so every reader goes
//! through `window_bytes_h`.

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
    "copyBytesFrom",
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
pub const MODULE_METHODS: &[&str] = &["atob", "btoa", "isAscii", "isLatin1", "isUtf8", "transcode"];

/// The data members of the `buffer` module, next to its functions.
/// `MAX_STRING_LENGTH` is V8's `String::kMaxLength` on a 64-bit build, the
/// figure node reports; node-js does not itself cap a string at it.
pub fn module_constant(name: &str) -> Option<Value> {
    const MAX_STRING_LENGTH: f64 = 536_870_888.0;
    match name {
        "kMaxLength" => Some(Value::Float(K_MAX_LENGTH)),
        "kStringMaxLength" => Some(Value::Float(MAX_STRING_LENGTH)),
        "INSPECT_MAX_BYTES" => Some(Value::Float(50.0)),
        "constants" => Some(with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("MAX_LENGTH".into(), Value::Float(K_MAX_LENGTH));
            m.insert("MAX_STRING_LENGTH".into(), Value::Float(MAX_STRING_LENGTH));
            h.new_object(m)
        })),
        _ => None,
    }
}

/// `atob`/`btoa`'s one argument: absent is `ERR_MISSING_ARGS`, anything else
/// goes through `ToString` (so `btoa(undefined)` encodes `"undefined"`, a
/// `toString` method runs, and a symbol throws).
fn string_input(args: &[Value]) -> Result<String, String> {
    let v = args.first().ok_or_else(|| {
        crate::host::coded_error(
            "TypeError",
            "ERR_MISSING_ARGS",
            "The \"input\" argument must be specified",
        )
    })?;
    let s = crate::host::to_string_value(v)?;
    Ok(with_host(|h| h.str_of(&s)))
}

/// The bytes `isAscii`/`isUtf8` inspect: an `ArrayBuffer` or a typed array
/// (a Buffer is one), read as the bytes it views. A string, a number or a
/// `DataView` is `ERR_INVALID_ARG_TYPE` — they were answered as if they were
/// byte sequences.
fn validated_bytes(args: &[Value]) -> Result<Vec<u8>, String> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    match super::native_tag(&v).as_deref() {
        Some("Buffer" | "TypedArray" | "ArrayBuffer") => Ok(view_bytes(&v).unwrap_or_default()),
        _ => Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"input\" argument must be an instance of ArrayBuffer, Buffer, or TypedArray. Received {}",
                super::received_desc(&v)
            ),
        )),
    }
}

/// WHATWG forgiving-base64 decode, with the two failures node's `atob`
/// distinguishes: a character outside the alphabet (or misplaced padding) is
/// "Invalid character", and a data length of `4n + 1` is "not correctly
/// encoded". ASCII whitespace is skipped, up to two trailing `=` are allowed
/// when they complete a quantum, and non-zero trailing bits are accepted.
fn forgiving_base64(s: &str) -> Result<Vec<u8>, String> {
    let invalid = || crate::host::dom_error("InvalidCharacterError", "Invalid character");
    let chars: Vec<char> = s
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' '))
        .collect();
    let pad = chars.iter().rev().take_while(|c| **c == '=').count();
    let data = &chars[..chars.len() - pad];
    let mut vals = Vec::with_capacity(data.len());
    for c in data {
        match B64_ALPHABET.find(*c) {
            Some(i) => vals.push(i as u32),
            None => return Err(invalid()),
        }
    }
    if pad > 2 {
        return Err(invalid());
    }
    if vals.len() % 4 == 1 {
        return Err(crate::host::dom_error(
            "InvalidCharacterError",
            "The string to be decoded is not correctly encoded.",
        ));
    }
    if pad > 0 && chars.len() % 4 != 0 {
        return Err(invalid());
    }
    let mut out = Vec::with_capacity(vals.len() * 3 / 4);
    for q in vals.chunks(4) {
        let n = q
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, v)| n | v << (18 - 6 * i));
        out.extend_from_slice(&n.to_be_bytes()[1..q.len()]);
    }
    Ok(out)
}

const B64_ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Dispatch a `require('buffer').<method>` free function.
pub fn module_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        // atob: forgiving base64 → a binary (latin1) string.
        "atob" => (|| {
            let bytes = forgiving_base64(&string_input(args)?)?;
            let bin: String = bytes.iter().map(|b| *b as char).collect();
            Ok(with_host(|h| h.new_str(bin)))
        })(),
        // btoa: a binary string → base64. A code point above U+00FF has no
        // octet, so it is an `InvalidCharacterError` — it used to encode the
        // character's low byte.
        "btoa" => (|| {
            let s = string_input(args)?;
            let bytes = s
                .chars()
                .map(|c| u8::try_from(c as u32))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|_| {
                    crate::host::dom_error("InvalidCharacterError", "Invalid character")
                })?;
            Ok(with_host(|h| h.new_str(to_base64(&bytes))))
        })(),
        "isAscii" => validated_bytes(args).map(|b| Value::Bool(b.is_ascii())),
        "isUtf8" => validated_bytes(args).map(|b| Value::Bool(std::str::from_utf8(&b).is_ok())),
        // isLatin1 takes a STRING in node v26: whether every code unit fits a
        // byte.
        "isLatin1" => {
            let v = args.first().cloned().unwrap_or(Value::Undef);
            match with_host(|h| h.as_str(&v)) {
                Some(s) => Ok(Value::Bool(s.chars().all(|c| (c as u32) < 0x100))),
                None => Err(crate::host::invalid_arg_type(
                    "input", "argument", "string", &v,
                )),
            }
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
///
/// Every Buffer is a VIEW: the bytes live in an `ArrayBuffer` (its `.buffer`)
/// and the Buffer shares that store. It used to own a private array with no
/// buffer behind it, so `buf.buffer` threw on every Buffer node-js made.
pub fn from_bytes(bytes: &[u8]) -> Value {
    let arr = with_host(|h| h.new_array(bytes.iter().map(|b| Value::Float(*b as f64)).collect()));
    let ab = crate::stdlib::typedarray::array_buffer_over(arr);
    share_array_buffer(&ab, 0, bytes.len())
}

// ── the allocation pool (`lib/buffer.js`) ────────────────────────────────────
//
// Node carves small Buffers out of one shared ArrayBuffer: `Buffer.from` of a
// string or an array-like, `Buffer.allocUnsafe` and `Buffer.concat` take a
// slice of `allocPool` and advance `poolOffset` past it, 8-byte aligned; a new
// pool is made when the slice does not fit. So `buf.byteOffset` is the slice's
// place in the pool, `buf.buffer.byteLength` is the pool's (64 KiB plus the
// 64 bytes `createUnsafeAlignedBuffer` over-allocates), and `buf.buffer
// .transfer()` throws, the pool being marked untransferable. `Buffer.alloc`,
// `allocUnsafeSlow` and `Buffer.of` never use the pool. `poolBase` — where the
// aligned pool starts inside its ArrayBuffer — depends on a real address in
// node and is 0 here, which is what node reports whenever the backing store
// is already 64-byte aligned.

/// `kPoolAlignment`, and so the padding `createUnsafeAlignedBuffer` adds.
const POOL_PADDING: usize = 64;

/// `Buffer.poolSize` as a script currently sees it (it is writable).
fn pool_size_value() -> f64 {
    let set = with_host(|h| h.builtin_static("Buffer", "poolSize"));
    match set {
        Some(v) => with_host(|h| h.to_number(&v)),
        None => 65536.0,
    }
}

/// `Buffer.poolSize >>> 1`, the size below which a Buffer is pooled.
fn pool_half() -> usize {
    let n = pool_size_value();
    let u = if n.is_finite() {
        n.trunc().rem_euclid(4294967296.0) as u32
    } else {
        0
    };
    (u >> 1) as usize
}

/// `createPool()`: a fresh untransferable pool of `Buffer.poolSize` bytes.
fn create_pool() {
    let size = pool_size_value().max(0.0) as usize;
    let arr = with_host(|h| h.new_array(vec![Value::Float(0.0); size + POOL_PADDING]));
    let ab = crate::stdlib::typedarray::array_buffer_over(arr);
    crate::stdlib::typedarray::mark_untransferable(&ab);
    with_host(|h| h.buffer_pool = Some((ab, size, 0)));
}

/// A Buffer of `bytes` sliced out of the pool, after checking that `reserve`
/// bytes fit (`fromStringFast` checks its upper estimate, not the length it
/// ends up writing).
fn pool_slice(reserve: usize, bytes: &[u8]) -> Value {
    let fits = with_host(
        |h| matches!(h.buffer_pool, Some((_, size, off)) if reserve <= size - off.min(size)),
    );
    if !fits {
        create_pool();
    }
    let Some((ab, size, off)) = with_host(|h| h.buffer_pool.clone()) else {
        return from_bytes(bytes);
    };
    if let Some(store) = crate::stdlib::typedarray::buffer_store(&ab) {
        with_host(|h| {
            if let Some(JsObj::Array(items)) = h.get_mut(&store) {
                for (i, b) in bytes.iter().enumerate() {
                    items[off + i] = Value::Float(*b as f64);
                }
            }
        });
    }
    let buf = share_array_buffer(&ab, off, bytes.len());
    // `alignPool()`: the next slice starts on an 8-byte boundary.
    let next = (off + bytes.len() + 7) & !7;
    with_host(|h| h.buffer_pool = Some((ab, size, next)));
    buf
}

/// `allocate(size)` / `fromArrayLike`: a pooled Buffer holding `bytes` when it
/// is non-empty and under half the pool size, an own allocation otherwise.
pub fn pooled(bytes: &[u8]) -> Value {
    if bytes.is_empty() || bytes.len() >= pool_half() {
        return from_bytes(bytes);
    }
    pool_slice(bytes.len(), bytes)
}

/// `fromString` / `fromStringFast`: `s` encoded under `enc`, pooled when its
/// length estimate (four bytes per UTF-16 unit, or the real byte length once
/// that estimate reaches half the pool) is under half the pool size.
fn pooled_string(s: &str, enc: &str) -> Value {
    let bytes = decode_str(s, enc);
    if s.is_empty() {
        return from_bytes(&bytes);
    }
    let max = pool_half();
    let units = s.encode_utf16().count();
    if units >= max {
        return from_bytes(&bytes);
    }
    let mut estimate = units * 4;
    if estimate >= max {
        estimate = bytes.len();
    }
    if estimate >= max {
        return from_bytes(&bytes);
    }
    pool_slice(estimate, &bytes)
}

/// A Buffer's window onto its byte store as `(byteOffset, length)`.
///
/// A Buffer built from its own bytes spans the whole store, so this is
/// `(0, len)` and every accessor behaves as it did. One built over an
/// `ArrayBuffer` SHARES that buffer's array and may start partway into it, which
/// is what makes `Buffer.from(ab, 2, 2)` alias rather than copy.
fn window(recv: &Value) -> (usize, usize) {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => window_h(h, p),
        _ => (0, 0),
    })
}

/// `window` over a Buffer's own properties, with the host borrow held.
pub fn window_h(h: &crate::host::JsHost, p: &IndexMap<String, Value>) -> (usize, usize) {
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

/// The bytes a Buffer's window covers, with the host borrow held. Every reader
/// of a Buffer's bytes goes through here: the store is SHARED with its
/// `ArrayBuffer` and with every other view over it, so reading the whole store
/// returns bytes that are not the Buffer's.
pub fn window_bytes_h(h: &crate::host::JsHost, p: &IndexMap<String, Value>) -> Vec<u8> {
    let (off, len) = window_h(h, p);
    match p.get("@@bytes").and_then(|v| h.get(v)) {
        Some(JsObj::Array(items)) => items[off..off + len]
            .iter()
            .map(|v| h.to_number(v) as u8)
            .collect(),
        _ => Vec::new(),
    }
}

fn bytes_of(recv: &Value) -> Vec<u8> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => window_bytes_h(h, p),
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

/// `buffer.kMaxLength` — node v26's ceiling on a Buffer's size, 2^53 - 1.
const K_MAX_LENGTH: f64 = 9_007_199_254_740_991.0;

/// Node's `validateNumber(size, 'size', 0, kMaxLength)`, the check
/// `Buffer.alloc` / `allocUnsafe` / `allocUnsafeSlow` run before allocating: a
/// non-number is `ERR_INVALID_ARG_TYPE`, and a negative, too-large or NaN size is
/// `ERR_OUT_OF_RANGE`. Without it `Buffer.alloc(-1)` and `Buffer.alloc('x')`
/// quietly returned an empty buffer, and `Buffer.alloc(2 ** 53)` aborted the
/// process on a failed allocation instead of throwing.
fn validate_size(args: &[Value]) -> Result<usize, String> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    if with_host(|h| h.type_of(&v)) != "number" {
        return Err(crate::host::invalid_arg_type(
            "size", "argument", "number", &v,
        ));
    }
    let n = with_host(|h| h.to_number(&v));
    if n.is_nan() || !(0.0..=K_MAX_LENGTH).contains(&n) {
        return Err(crate::host::coded_error(
            "RangeError",
            "ERR_OUT_OF_RANGE",
            &format!(
                "The value of \"size\" is out of range. It must be >= 0 && <= {}. Received {}",
                crate::host::fmt_number(K_MAX_LENGTH),
                out_of_range_received(n)
            ),
        ));
    }
    Ok(n as usize)
}

/// How node's `ERR_OUT_OF_RANGE` shows a numeric input: an integer beyond 2^32
/// through `addNumericalSeparator` (`_` every three characters from the right,
/// applied to the printed text, so `1e21` becomes `1e_+21` exactly as in node),
/// anything else as `inspect` would.
pub(crate) fn out_of_range_received(n: f64) -> String {
    let shown = crate::host::fmt_number(n);
    if n.fract() != 0.0 || n.abs() <= 4_294_967_296.0 {
        return shown;
    }
    let start = usize::from(shown.starts_with('-'));
    let mut i = shown.len();
    let mut groups = String::new();
    while i >= start + 4 {
        groups = format!("_{}{groups}", &shown[i - 3..i]);
        i -= 3;
    }
    format!("{}{groups}", &shown[..i])
}

pub fn static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "from" => from(args),
        "copyBytesFrom" => copy_bytes_from(args),
        "alloc" => {
            let n = match validate_size(args) {
                Ok(n) => n,
                Err(e) => return Some(Err(e)),
            };
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
        // `allocUnsafe` slices the shared pool (`allocate`); `allocUnsafeSlow`
        // never does.
        "allocUnsafe" => validate_size(args).map(|n| pooled(&vec![0u8; n])),
        "allocUnsafeSlow" => validate_size(args).map(|n| from_bytes(&vec![0u8; n])),
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
            let a = match uint8_arg(args.first(), "buf1") {
                Ok(b) => b,
                Err(e) => return Some(Err(e)),
            };
            let b = match uint8_arg(args.get(1), "buf2") {
                Ok(b) => b,
                Err(e) => return Some(Err(e)),
            };
            Ok(ordering_value(a.cmp(&b)))
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
        // A typed array contributes the bytes it views, not its truncated
        // elements: `hash.update(new Uint16Array([0x4142]))` hashes `42 41`.
        Some("TypedArray") => crate::stdlib::typedarray::raw_view_bytes(v),
        Some("Buffer") | Some("ArrayBuffer") => bytes_like(v),
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
        return Ok(pooled(&bytes));
    }
    // A string, with an optional encoding.
    if with_host(|h| h.as_str(&v)).is_some() || matches!(v, Value::Str(_)) {
        let enc = if args.len() > 1 {
            arg_str(args, 1)
        } else {
            "utf8".into()
        };
        return Ok(pooled_string(&arg_str(args, 0), &enc));
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
                return Ok(pooled(&out));
            }
        }
        // `fromObject`'s second form: what `buf.toJSON()` produced,
        // `{ type: 'Buffer', data: [...] }`, rebuilds the bytes of `data`.
        let ty = crate::builtins::get_property(&v, "type").unwrap_or(Value::Undef);
        let data = crate::builtins::get_property(&v, "data").unwrap_or(Value::Undef);
        if with_host(|h| h.as_str(&ty).is_some_and(|t| t == "Buffer"))
            && with_host(|h| matches!(h.get(&data), Some(JsObj::Array(_))))
        {
            return Ok(pooled(&bytes_like(&data).unwrap_or_default()));
        }
    }
    // Anything else is a TypeError, not a stringification: `Buffer.from(5)`
    // used to produce the single byte `0x35` (the digit "5") and
    // `Buffer.from(null)` the four bytes of `"null"`.
    // It is `lib/buffer.js` throwing `new ERR_INVALID_ARG_TYPE(...)`, a JS-layer
    // NodeError, so `String(err)` brackets the code.
    Err(crate::host::coded_error(
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
    // `lib/buffer.js` `Buffer.concat(list, length)`: `validateArray(list)`; an
    // empty list is an empty Buffer before `length` is looked at; `length` is
    // `validateOffset`ed; each part must be a Uint8Array; the result is exactly
    // `length` bytes, truncated or zero-filled.
    let list_v = args.first().cloned().unwrap_or(Value::Undef);
    let list = match with_host(|h| h.get(&list_v).cloned()) {
        Some(JsObj::Array(items)) => items,
        _ => {
            return Err(crate::host::coded_error(
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                &format!(
                    "The \"list\" argument must be an instance of Array. Received {}",
                    super::received_desc(&list_v)
                ),
            ))
        }
    };
    if list.is_empty() {
        return Ok(from_bytes(&[]));
    }
    let length = match args.get(1) {
        None | Some(Value::Undef) => None,
        Some(v) => Some(validate_offset(v, "length")?),
    };
    let mut out = Vec::new();
    for (i, b) in list.iter().enumerate() {
        if super::util_types::ta_kind(b).as_deref() != Some("Uint8Array") {
            return Err(crate::host::coded_error(
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                &format!(
                    "The \"list[{i}]\" argument must be an instance of Buffer or Uint8Array. Received {}",
                    super::received_desc(b)
                ),
            ));
        }
        out.extend(bytes_like(b).unwrap_or_default());
    }
    if let Some(n) = length {
        out.resize(n, 0);
    }
    Ok(pooled(&out))
}

/// `validateOffset(value, name)` from `lib/buffer.js`: a Number, an integer,
/// and within `[0, kMaxLength]`.
fn validate_offset(v: &Value, name: &str) -> Result<usize, String> {
    validate_offset_in(v, name, K_MAX_LENGTH)
}

/// `validateOffset(value, name, 0, max)`: [`validate_offset`] with an explicit
/// upper bound.
fn validate_offset_in(v: &Value, name: &str, max: f64) -> Result<usize, String> {
    if with_host(|h| h.type_of(v)) != "number" {
        return Err(crate::host::invalid_arg_type(name, "argument", "number", v));
    }
    let n = with_host(|h| h.to_number(v));
    let out_of_range = |must: String| {
        crate::host::coded_error(
            "RangeError",
            "ERR_OUT_OF_RANGE",
            &format!(
                "The value of \"{name}\" is out of range. It must be {must}. Received {}",
                out_of_range_received(n)
            ),
        )
    };
    if n.is_nan() || n.fract() != 0.0 {
        return Err(out_of_range("an integer".to_string()));
    }
    if !(0.0..=max).contains(&n) {
        return Err(out_of_range(format!(
            ">= 0 && <= {}",
            crate::host::fmt_number(max)
        )));
    }
    Ok(n as usize)
}

/// The bytes of a `compare` operand, which `lib/buffer.js` requires to be a
/// Uint8Array (a Buffer is one): anything else is `ERR_INVALID_ARG_TYPE` naming
/// the parameter, never a comparison against the bytes of its stringification.
fn uint8_arg(v: Option<&Value>, name: &str) -> Result<Vec<u8>, String> {
    let v = v.cloned().unwrap_or(Value::Undef);
    if super::util_types::ta_kind(&v).as_deref() != Some("Uint8Array") {
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"{name}\" argument must be an instance of Buffer or Uint8Array. Received {}",
                super::received_desc(&v)
            ),
        ));
    }
    Ok(view_bytes(&v).unwrap_or_default())
}

/// A byte-order comparison as the `-1` / `0` / `1` that `compare` returns.
fn ordering_value(o: std::cmp::Ordering) -> Value {
    Value::Float(match o {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    })
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
            // `otherBuffer` must be a Uint8Array (a Buffer is one); any other
            // view or value is `ERR_INVALID_ARG_TYPE`, never a plain `false`.
            let other = uint8_arg(args.first(), "otherBuffer")?;
            Ok(Value::Bool(bytes == other))
        }
        // Both are a window onto the SAME memory in node (`slice` is the
        // deprecated alias): a write through the result shows in the parent.
        "slice" | "subarray" => {
            let (s, e) = slice_bounds(args, bytes.len());
            let (off, _) = window(recv);
            let ab = with_host(|h| match h.get(recv) {
                Some(JsObj::Object(p)) => p.get("@@buffer").cloned(),
                _ => None,
            });
            Ok(match ab {
                Some(ab) => share_array_buffer(&ab, off + s, e - s),
                None => from_bytes(&bytes[s..e]),
            })
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
            // The needle is a number (its `>>> 0` low byte, so 353 finds 97),
            // a string in `enc`, or a Uint8Array's bytes; anything else — an
            // array, a boolean, `null`, a `Uint16Array` — is the TypeError
            // `bidirectionalIndexOf` throws.
            let target = args.first().cloned().unwrap_or(Value::Undef);
            let needle = match with_host(|h| h.type_of(&target)) {
                "number" => {
                    let n = super::arg_num(args, 0);
                    vec![crate::host::to_int32(n) as u32 as u8]
                }
                "string" => decode_str(&arg_str(args, 0), &enc),
                _ if super::util_types::ta_kind(&target).as_deref() == Some("Uint8Array") => {
                    view_bytes(&target).unwrap_or_default()
                }
                _ => {
                    return Err(crate::host::coded_error(
                        "TypeError",
                        "ERR_INVALID_ARG_TYPE",
                        &format!(
                            "The \"value\" argument must be one of type number or string or an instance of Buffer or Uint8Array. Received {}",
                            super::received_desc(&target)
                        ),
                    ))
                }
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
        // compare(target[, targetStart[, targetEnd[, sourceStart[, sourceEnd]]]])
        // — `lib/buffer.js`: each bound is `validateOffset`ed (an end against its
        // own buffer's length), an empty source range is `0`/`-1` and an empty
        // target range `1` before any byte is read, and the two ranges are then
        // compared lexicographically. The bounds used to be ignored entirely.
        "compare" => {
            let target = uint8_arg(args.first(), "target")?;
            let bound = |i: usize, name: &str, default: usize, max: f64| match args.get(i) {
                None | Some(Value::Undef) => Ok(default),
                Some(v) => validate_offset_in(v, name, max),
            };
            let t_start = bound(1, "targetStart", 0, K_MAX_LENGTH)?;
            let t_end = bound(2, "targetEnd", target.len(), target.len() as f64)?;
            let s_start = bound(3, "sourceStart", 0, K_MAX_LENGTH)?;
            let s_end = bound(4, "sourceEnd", bytes.len(), bytes.len() as f64)?;
            if s_start >= s_end {
                return Ok(Value::Float(if t_start >= t_end { 0.0 } else { -1.0 }));
            }
            if t_start >= t_end {
                return Ok(Value::Float(1.0));
            }
            Ok(ordering_value(
                bytes[s_start..s_end].cmp(&target[t_start..t_end]),
            ))
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
        // A Buffer's `Symbol.iterator` IS `values`, inherited from
        // `%TypedArray%.prototype`, so it dispatches here rather than reporting
        // itself missing.
        "values" | "keys" | "entries" | "@@iterator" => {
            // They are `%TypedArray%.prototype`'s, so over a DETACHED buffer
            // they throw where Buffer's own methods read zero bytes.
            if crate::stdlib::typedarray::view_detached(recv) {
                let name = if method == "@@iterator" {
                    "values"
                } else {
                    method
                };
                return Err(crate::stdlib::typedarray::detached_error(
                    "%TypedArray%.prototype",
                    name,
                    false,
                ));
            }
            let items: Vec<Value> = with_host(|h| match method {
                "keys" => (0..bytes.len()).map(|i| Value::Float(i as f64)).collect(),
                "values" | "@@iterator" => bytes.iter().map(|b| Value::Float(*b as f64)).collect(),
                _ => bytes
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        h.new_array(vec![Value::Float(i as f64), Value::Float(*b as f64)])
                    })
                    .collect(),
            });
            Ok(with_host(|h| {
                h.alloc_iter(JsObj::Iter {
                    items,
                    idx: 0,
                    array: None,
                    brand: Some("Array Iterator"),
                })
            }))
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
            let off = write_offset(args, 4, bytes.len())?;
            store_bytes(recv, &bytes, off, &raw)?;
            Ok(Value::Float((off + 4) as f64))
        }
        "writeDoubleBE" | "writeDoubleLE" => {
            let mut raw = super::arg_num(args, 0).to_be_bytes();
            if method.ends_with("LE") {
                raw.reverse();
            }
            let off = write_offset(args, 8, bytes.len())?;
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
            let n = big_write_value(&v, method.starts_with("writeBigInt"))?;
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
            let off = write_offset(args, 8, bytes.len())?;
            store_bytes(recv, &bytes, off, &raw)?;
            Ok(Value::Float((off + 8) as f64))
        }
        // The variable-width family: `byteLength` is an argument (1..=6), which
        // is why these cannot share the fixed-width arms above.
        "readIntBE" | "readIntLE" | "readUIntBE" | "readUIntLE" => {
            let width = variable_byte_length(args.get(1))?;
            let off = read_offset(args, width, bytes.len())?;
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
        // `lib/internal/buffer.js`'s writers: `value = +value`, `checkInt` against
        // the width's range, then the offset's `checkBounds`. A value outside the
        // range used to wrap silently (`writeUInt8(300)` stored 0x2c) and an
        // out-of-range offset could be negative or fractional.
        "writeIntBE" | "writeIntLE" | "writeUIntBE" | "writeUIntLE" | "writeInt8"
        | "writeUInt8" | "writeInt16BE" | "writeInt16LE" | "writeUInt16BE" | "writeUInt16LE"
        | "writeInt32BE" | "writeInt32LE" | "writeUInt32BE" | "writeUInt32LE" => {
            let width = match method {
                "writeIntBE" | "writeIntLE" | "writeUIntBE" | "writeUIntLE" => {
                    variable_byte_length(args.get(2))?
                }
                "writeInt8" | "writeUInt8" => 1,
                m if m.contains("16") => 2,
                _ => 4,
            };
            let value = super::arg_num(args, 0);
            let bits = (width * 8) as i32;
            let (min, max) = if method.starts_with("writeInt") {
                (-(2f64.powi(bits - 1)), 2f64.powi(bits - 1) - 1.0)
            } else {
                (0.0, 2f64.powi(bits) - 1.0)
            };
            check_int_value(value, min, max, width - 1)?;
            let off = write_offset(args, width, bytes.len())?;
            // NaN stores 0, as `value as i64` gives.
            let val = value as i64 as u64;
            let mut raw: Vec<u8> = (0..width)
                .map(|k| (val >> (8 * (width - 1 - k))) as u8)
                .collect();
            if method.ends_with("LE") {
                raw.reverse();
            }
            store_bytes(recv, &bytes, off, &raw)?;
            Ok(Value::Float((off + width) as f64))
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
            let src = crate::stdlib::typedarray::set_source_bytes(args.first())?;
            let off = super::arg_num(args, 1);
            let off = if off.is_nan() { 0.0 } else { off.trunc() };
            if off < 0.0 || off + src.len() as f64 > bytes.len() as f64 {
                return Err(crate::host::range_error("offset is out of bounds"));
            }
            let offset = off as usize;
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

/// Overwrite the bytes a byte VIEW (or an `ArrayBuffer`) exposes, from its
/// first byte — the write-side twin of [`view_bytes`], so an API that fills a
/// caller's view (`randomFillSync`, `readSync`) reaches every view kind and
/// the memory it aliases.
pub fn write_view(v: &Value, bytes: &[u8]) {
    match super::native_tag(v).as_deref() {
        Some("Buffer" | "ArrayBuffer") => set_bytes(v, bytes),
        Some("TypedArray" | "DataView") => {
            crate::stdlib::typedarray::write_view_bytes(v, 0, bytes);
        }
        _ => {}
    }
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
    checked_offset(args.first(), size, len)
}

/// [`read_offset`] for a write, whose offset is the SECOND argument.
/// The BigInt a `writeBigInt64*` / `writeBigUInt64*` stores: `checkInt`
/// against the 64-bit range (a Number is compared too, as node's mixed `>`
/// does), and then a non-BigInt is refused by the BigInt arithmetic that
/// follows in node.
fn big_write_value(v: &Value, signed: bool) -> Result<num_bigint::BigInt, String> {
    use num_bigint::BigInt;
    let (min, max_excl) = if signed {
        (
            -(BigInt::from(1u8) << 63usize),
            BigInt::from(1u8) << 63usize,
        )
    } else {
        (BigInt::from(0u8), BigInt::from(1u8) << 64usize)
    };
    let range = if signed {
        ">= -(2n ** 63n) and < 2n ** 63n"
    } else {
        ">= 0n and < 2n ** 64n"
    };
    let out_of_range = |received: String| {
        crate::host::coded_error(
            "RangeError",
            "ERR_OUT_OF_RANGE",
            &format!(
                "The value of \"value\" is out of range. It must be {range}. Received {received}"
            ),
        )
    };
    match with_host(|h| h.get(v).cloned()) {
        Some(JsObj::BigInt(b)) => {
            if b < min || b >= max_excl {
                let mut shown = b.to_string();
                if b.magnitude() > &num_bigint::BigUint::from(1u64 << 32) {
                    shown = grouped_digits(&shown);
                }
                return Err(out_of_range(format!("{shown}n")));
            }
            Ok(b)
        }
        _ => {
            let n = with_host(|h| h.to_number(v));
            let (lo, hi) = if signed {
                (-(2f64.powi(63)), 2f64.powi(63))
            } else {
                (0.0, 2f64.powi(64))
            };
            if n < lo || n >= hi {
                return Err(out_of_range(out_of_range_received(n)));
            }
            Err(crate::host::type_error(
                "Cannot mix BigInt and other types, use explicit conversions",
            ))
        }
    }
}

/// `addNumericalSeparator`: `_` every three digits from the right.
fn grouped_digits(shown: &str) -> String {
    let start = usize::from(shown.starts_with('-'));
    let mut i = shown.len();
    let mut groups = String::new();
    while i >= start + 4 {
        groups = format!("_{}{groups}", &shown[i - 3..i]);
        i -= 3;
    }
    format!("{}{groups}", &shown[..i])
}

fn write_offset(args: &[Value], size: usize, len: usize) -> Result<usize, String> {
    checked_offset(args.get(1), size, len)
}

/// `checkInt`'s value test from `lib/internal/buffer.js`: `value` (already
/// `+value`) must lie in `[min, max]`. `byte_len` is node's `byteLength`
/// argument there — the width minus one — which picks between the literal
/// bounds and the power-of-two spelling used past 32 bits.
fn check_int_value(value: f64, min: f64, max: f64, byte_len: usize) -> Result<(), String> {
    if !(value > max || value < min) {
        return Ok(());
    }
    let range = if byte_len > 3 {
        let bits = (byte_len + 1) * 8;
        if min == 0.0 {
            format!(">= 0 and < 2 ** {bits}")
        } else {
            format!(">= -(2 ** {}) and < 2 ** {}", bits - 1, bits - 1)
        }
    } else {
        format!(
            ">= {} and <= {}",
            crate::host::fmt_number(min),
            crate::host::fmt_number(max)
        )
    };
    Err(crate::host::coded_error(
        "RangeError",
        "ERR_OUT_OF_RANGE",
        &format!(
            "The value of \"value\" is out of range. It must be {range}. Received {}",
            out_of_range_received(value)
        ),
    ))
}

/// The `byteLength` of the variable-width `readIntLE`/`writeUIntBE` family:
/// `validateNumber`, then node's `boundsError(byteLength, 6, 'byteLength')`.
fn variable_byte_length(v: Option<&Value>) -> Result<usize, String> {
    let v = v.cloned().unwrap_or(Value::Undef);
    if with_host(|h| h.type_of(&v)) != "number" {
        return Err(crate::host::invalid_arg_type(
            "byteLength",
            "argument",
            "number",
            &v,
        ));
    }
    let n = with_host(|h| h.to_number(&v));
    if n.fract() != 0.0 || n.is_nan() {
        return Err(crate::host::coded_error(
            "RangeError",
            "ERR_OUT_OF_RANGE",
            &format!(
                "The value of \"byteLength\" is out of range. It must be an integer. Received {}",
                crate::host::fmt_number(n)
            ),
        ));
    }
    if !(1.0..=6.0).contains(&n) {
        return Err(crate::host::coded_error(
            "RangeError",
            "ERR_OUT_OF_RANGE",
            &format!(
                "The value of \"byteLength\" is out of range. It must be >= 1 and <= 6. Received {}",
                crate::host::fmt_number(n)
            ),
        ));
    }
    Ok(n as usize)
}

fn checked_offset(arg: Option<&Value>, size: usize, len: usize) -> Result<usize, String> {
    // `validateNumber(offset, 'offset')`: a present non-number is a type error.
    if let Some(v) = arg {
        if !matches!(v, Value::Undef) && with_host(|h| h.type_of(v)) != "number" {
            return Err(crate::host::invalid_arg_type(
                "offset", "argument", "number", v,
            ));
        }
    }
    if len < size {
        return Err(crate::host::coded_error(
            "RangeError",
            "ERR_BUFFER_OUT_OF_BOUNDS",
            "Attempt to access memory outside buffer bounds",
        ));
    }
    let max = len - size;
    let raw = match arg {
        None | Some(Value::Undef) => 0.0,
        Some(v) => with_host(|h| h.to_number(v)),
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

/// `Buffer.copyBytesFrom(view[, offset[, length]])`, a port of `lib/buffer.js`:
/// a NEW Buffer holding a copy of the bytes behind `view`'s elements
/// `[offset, offset + length)` — `offset`/`length` count ELEMENTS, so a
/// `Uint16Array` contributes two bytes per element. It was absent.
fn copy_bytes_from(args: &[Value]) -> Result<Value, String> {
    let view = args.first().cloned().unwrap_or(Value::Undef);
    let Some(kind) = super::util_types::ta_kind(&view) else {
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"view\" argument must be an instance of TypedArray. Received {}",
                super::received_desc(&view)
            ),
        ));
    };
    let bytes = view_bytes(&view).unwrap_or_default();
    let bpe = crate::stdlib::typedarray::bytes_per_element(&kind);
    let len = bytes.len() / bpe;
    if len == 0 {
        return Ok(from_bytes(&[]));
    }
    let arg = |i: usize| args.get(i).filter(|v| !matches!(v, Value::Undef));
    let offset = match arg(1) {
        Some(v) => super::validate_integer(v, "offset", 0.0)? as usize,
        None => 0,
    };
    if offset >= len {
        return Ok(from_bytes(&[]));
    }
    let end = match arg(2) {
        Some(v) => {
            (offset as f64 + super::validate_integer(v, "length", 0.0)?).min(len as f64) as usize
        }
        None => len,
    };
    // `fromArrayLike(new Uint8Array(…))`: pooled like any array-like.
    Ok(pooled(&bytes[offset * bpe..end * bpe]))
}
