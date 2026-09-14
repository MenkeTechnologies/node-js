//! JavaScript typed arrays (`Uint8Array`/`Int8Array`/…/`Float64Array`),
//! `ArrayBuffer`, `WeakRef`, and `TextEncoder`/`TextDecoder`.
//!
//! A typed array is a plain object tagged `@@native = "TypedArray"` carrying its
//! kind (`@@kind`), a window (`@@buffer`/`byteOffset`/`length`) onto the bytes
//! its `ArrayBuffer` owns, and the
//! enumerable `length`/`byteLength`/`BYTES_PER_ELEMENT` data properties JS code
//! reads directly. Element indexing (`ta[i]` get/set) is special-cased in
//! `builtins::get_property`/`set_property` via `elem_get`/`elem_set` here, which
//! also apply each kind's coercion (integer wrap / clamp / float).
//!
//! `WeakRef` holds a *strong* reference (`deref()` always returns the target) —
//! node-js has no GC of JS objects, so this is observably correct for the
//! express dependency tree (object-inspect/qs/side-channel only ever `deref()`).

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const STATIC_METHODS: &[&str] = &["from", "of", "isView"];

/// `Uint8Array`'s statics — the shared three plus the base64/hex pair, which no
/// other view has.
pub const UINT8_STATIC_METHODS: &[&str] = &["from", "of", "isView", "fromBase64", "fromHex"];

/// The four base64/hex methods `Uint8Array.prototype` owns on its own.
/// In the engine's own order, which `Object.getOwnPropertyNames` reports.
pub const UINT8_PROTOTYPE_METHODS: &[&str] = &["toBase64", "setFromBase64", "toHex", "setFromHex"];

/// The statics `<kind>` advertises.
pub fn static_methods(kind: &str) -> &'static [&'static str] {
    if kind == "Uint8Array" {
        UINT8_STATIC_METHODS
    } else {
        STATIC_METHODS
    }
}

/// The methods installed on the real `Uint8Array.prototype` object (as
/// `@proto:Uint8Array:<m>` thunks), so `Uint8Array.prototype.slice.call(x)`
/// keeps working now that the prototype is an object rather than a `Builtin`
/// namespace whose every property read synthesized a thunk.
pub const PROTOTYPE_METHODS: &[&str] = &[
    "at",
    "copyWithin",
    "entries",
    "every",
    "fill",
    "filter",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "forEach",
    "includes",
    "indexOf",
    "join",
    "keys",
    "lastIndexOf",
    "map",
    "reduce",
    "reduceRight",
    "reverse",
    "set",
    "slice",
    "some",
    "sort",
    "subarray",
    "toReversed",
    "toSorted",
    "toString",
    "values",
    "with",
];

/// The eleven element kinds plus the two buffer types.
pub fn is_ctor(name: &str) -> bool {
    ELEMENT_KINDS.contains(&name) || matches!(name, "ArrayBuffer" | "DataView")
}

/// The element kinds, each of which gets its own real prototype object whose
/// parent is the shared `%TypedArray%.prototype`. `Uint8Array` leads because
/// `Buffer.prototype` chains onto it.
///
/// `BigInt64Array`/`BigUint64Array` are here too, and they are not
/// interchangeable with the rest: their elements are BigInts, so a Number
/// written into one is a `TypeError` and a `Number`-kind view will not accept
/// one either (`coerce_val`).
pub const ELEMENT_KINDS: &[&str] = &[
    "Uint8Array",
    "Int8Array",
    "Uint8ClampedArray",
    "Int16Array",
    "Uint16Array",
    "Int32Array",
    "Uint32Array",
    "Float32Array",
    "Float64Array",
    // The 64-bit views store BigInt elements rather than Numbers.
    "BigInt64Array",
    "BigUint64Array",
];

/// Bytes per element for a typed-array kind.
pub fn bytes_per_element(kind: &str) -> usize {
    match kind {
        "Int8Array" | "Uint8Array" | "Uint8ClampedArray" => 1,
        "Int16Array" | "Uint16Array" => 2,
        "Int32Array" | "Uint32Array" | "Float32Array" => 4,
        "Float64Array" | "BigInt64Array" | "BigUint64Array" => 8,
        _ => 1,
    }
}

/// Coerce a JS number into the value stored for `kind` (integer wrap, unsigned
/// clamp, or float), mirroring the `ToInt8`/`ToUint8Clamp`/… abstract ops.
fn coerce(kind: &str, n: f64) -> f64 {
    match kind {
        "Int8Array" => (n as i64 as i8) as f64,
        "Uint8Array" => (n as i64 as u8) as f64,
        "Uint8ClampedArray" => {
            if n.is_nan() {
                0.0
            } else {
                n.round().clamp(0.0, 255.0)
            }
        }
        "Int16Array" => (n as i64 as i16) as f64,
        "Uint16Array" => (n as i64 as u16) as f64,
        "Int32Array" => (n as i64 as i32) as f64,
        "Uint32Array" => (n as i64 as u32) as f64,
        "Float32Array" => n as f32 as f64,
        _ => n, // Float64Array
    }
}

/// Whether `kind` stores BigInt elements rather than Numbers. The two 64-bit
/// views are the only ones: their elements do not fit an `f64` without loss, so
/// the whole element pipeline carries `Value` rather than `f64`.
pub fn is_bigint_kind(kind: &str) -> bool {
    matches!(kind, "BigInt64Array" | "BigUint64Array")
}

/// Coerce a JS value into the element `kind` stores. The numeric kinds go
/// through the `ToInt8`/`ToUint8Clamp`/… abstract ops as before; the 64-bit ones
/// wrap through `ToBigInt64`/`ToBigUint64` and keep a BigInt.
fn coerce_val(kind: &str, v: &Value) -> Result<Value, String> {
    if !is_bigint_kind(kind) {
        return Ok(Value::Float(coerce(kind, with_host(|h| h.to_number(v)))));
    }
    // 7.1.15/7.1.16 route through `ToBigInt`, which is not "must already be a
    // BigInt": a boolean, a string and any object that converts to one are all
    // accepted (`a[0] = '12'` stores `12n`), and only a Number is refused. The
    // check here was the identity test, so it rejected every one of those and
    // reported the same wrong text — node names the value it could not convert.
    let big = crate::builtins::to_bigint(v)?;
    Ok(with_host(|h| h.new_bigint(wrap_bigint(kind, big))))
}

/// `ToBigInt64` / `ToBigUint64` — wrap modulo 2^64 into the signed or unsigned
/// 64-bit range, which is what a 64-bit view stores.
fn wrap_bigint(kind: &str, b: num_bigint::BigInt) -> num_bigint::BigInt {
    use num_traits::cast::ToPrimitive;
    let modulus = num_bigint::BigInt::from(1u128 << 64);
    let mut m = b % &modulus;
    if m.sign() == num_bigint::Sign::Minus {
        m += &modulus;
    }
    // `m` is now in [0, 2^64); reinterpret it for the view's signedness.
    let raw = m.to_u64().unwrap_or(0);
    if kind == "BigInt64Array" {
        num_bigint::BigInt::from(raw as i64)
    } else {
        num_bigint::BigInt::from(raw)
    }
}

/// An element's BigInt, for ordering a 64-bit view. Zero for anything else,
/// which the numeric kinds never ask for.
fn bigint_of(v: &Value) -> num_bigint::BigInt {
    with_host(|h| match h.get(v) {
        Some(JsObj::BigInt(b)) => b.clone(),
        _ => num_bigint::BigInt::from(0),
    })
}

/// `indexOf`/`lastIndexOf`/`includes` element comparison. 23.2.3.x compare the
/// search element with the STORED one and do not coerce it, so a string never
/// matches a numeric element and a Number never matches a BigInt one.
///
/// `includes` differs from `indexOf` only in treating `NaN` as present
/// (SameValueZero vs strict equality), which `nan_matches` selects: node reports
/// `new Float64Array([NaN]).includes(NaN)` as true and `.indexOf(NaN)` as -1.
fn same_element(stored: &Value, needle: &Value, nan_matches: bool) -> bool {
    if nan_matches {
        if let (Value::Float(a), Value::Float(b)) = (stored, needle) {
            if a.is_nan() && b.is_nan() {
                return true;
            }
        }
    }
    with_host(|h| h.strict_eq(stored, needle))
}

/// The zero element of `kind` — what a freshly allocated view is filled with.
fn zero_of(kind: &str) -> Value {
    if is_bigint_kind(kind) {
        with_host(|h| h.new_bigint(num_bigint::BigInt::from(0)))
    } else {
        Value::Float(0.0)
    }
}

/// An element as an `f64`, for the numeric-kind comparisons (`sort`'s default
/// order, `indexOf`). A BigInt element answers its nearest `f64`, which is only
/// ever used where the kind is numeric.
fn num(v: &Value) -> f64 {
    with_host(|h| h.to_number(v))
}

/// The element values of a typed array / Buffer as stored — `Value`, not `f64`,
/// so a 64-bit view keeps its BigInts. `elems_of` is the numeric view of the
/// same data and stays, because `Buffer` reads bytes through it.
pub fn elem_values(v: &Value) -> Vec<Value> {
    let Some(tag) = super::native_tag(v) else {
        return Vec::new();
    };
    if tag == "TypedArray" {
        let kind = kind_of(v);
        let bpe = bytes_per_element(&kind);
        return (0..view_len(v))
            .map(|i| {
                view_bytes(v, i * bpe, bpe)
                    .map(|b| decode(&kind, &b))
                    .unwrap_or(Value::Undef)
            })
            .collect();
    }
    if tag != "Buffer" {
        return Vec::new();
    }
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    })
}

/// Build a typed array of `kind` from already-coerced element values.
fn make(kind: &str, elems: Vec<Value>) -> Value {
    let bpe = bytes_per_element(kind);
    let len = elems.len();
    let buf = new_array_buffer(len * bpe);
    let view = make_view(kind, &buf, 0, len);
    for (i, e) in elems.iter().enumerate() {
        write_view_bytes(&view, i * bpe, &encode(kind, e));
    }
    view
}

/// A typed array of `kind` over `buf`, `len` elements from `byte_off`.
fn make_view(kind: &str, buf: &Value, byte_off: usize, len: usize) -> Value {
    with_host(|h| {
        let bpe = bytes_per_element(kind);
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("TypedArray"));
        m.insert("@@kind".into(), h.new_str(kind));
        m.insert("@@buffer".into(), buf.clone());
        // `ta.buffer` is a non-enumerable accessor in node; a hidden own slot
        // reads identically and keeps it out of `Object.keys` and `inspect`.
        m.insert("buffer".into(), buf.clone());
        m.insert("length".into(), Value::Float(len as f64));
        m.insert("byteLength".into(), Value::Float((len * bpe) as f64));
        // Every view reports where it starts in its backing store. A `Buffer`
        // already carried this; a typed array did not, so `u8.byteOffset` read
        // `undefined` where a Buffer read 0. Nothing here can produce a
        // non-zero offset yet — see the note on `.buffer` below.
        m.insert("byteOffset".into(), Value::Float(byte_off as f64));
        m.insert("BYTES_PER_ELEMENT".into(), Value::Float(bpe as f64));
        let obj = h.new_object(m);
        // Link the instance to the real `Uint8Array.prototype` object so its
        // inherited methods resolve through the chain, exactly as a `Buffer`
        // already did. Without this a typed array was a bare tagged object and
        // `new Uint8Array([1]).every` was not even a function — the methods
        // existed on the prototype but nothing pointed at it.
        h.ensure_native_protos();
        if let Some(p) = h.native_proto(kind) {
            h.set_proto(&obj, p);
        }
        // View metadata is real but non-enumerable, as it is for a Buffer.
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

/// `new Uint8Array(...)` etc. `ArrayBuffer` is a byte container with only a
/// `byteLength`.
pub fn construct(kind: &str, args: &[Value]) -> Result<Value, String> {
    if kind == "ArrayBuffer" {
        let n = super::arg_num(args, 0).max(0.0) as usize;
        let ab = new_array_buffer(n);
        // `new ArrayBuffer(n, { maxByteLength })` is a RESIZABLE buffer, which
        // reports `resizable` and `maxByteLength` and accepts `resize`.
        if let Some(opts) = args.get(1) {
            let max = crate::builtins::get_property(opts, "maxByteLength").unwrap_or(Value::Undef);
            if !matches!(max, Value::Undef) {
                let m = with_host(|h| h.to_number(&max)).max(0.0) as usize;
                with_host(|h| {
                    if let Some(JsObj::Object(p)) = h.get_mut(&ab) {
                        p.insert("@@maxByteLength".into(), Value::Float(m as f64));
                        p.insert("maxByteLength".into(), Value::Float(m as f64));
                        p.insert("resizable".into(), Value::Bool(true));
                    }
                    h.hide_prop(&ab, "maxByteLength");
                    h.hide_prop(&ab, "resizable");
                });
            }
        }
        return Ok(ab);
    }
    // `new Uint8Array(buffer[, byteOffset[, length]])` — a VIEW onto an existing
    // buffer rather than a fresh copy. This is the form that makes two views
    // alias, and it did not exist: the argument fell through to the iterable
    // branch and produced an empty array.
    if let Some(first) = args.first() {
        if super::native_tag(first).as_deref() == Some("ArrayBuffer") {
            // A DETACHED buffer has no bytes to view.
            if is_detached(first) {
                return Err(crate::host::type_error(
                    "Cannot perform Construct on a detached ArrayBuffer",
                ));
            }
            let bpe = bytes_per_element(kind);
            let total = buffer_byte_length(first);
            let off = super::arg_num(args, 1).max(0.0) as usize;
            if off > total || off % bpe != 0 {
                return Err(crate::host::range_error(
                    "start offset of Uint8Array should be a multiple of element size",
                ));
            }
            let len = match args.get(2) {
                Some(Value::Undef) | None => (total - off) / bpe,
                Some(_) => super::arg_num(args, 2).max(0.0) as usize,
            };
            if off + len * bpe > total {
                return Err(crate::host::range_error("Invalid typed array length"));
            }
            return Ok(make_view(kind, first, off, len));
        }
    }
    let elems = build_elems(kind, args)?;
    Ok(make(kind, elems))
}

/// The methods a `DataView` instance exposes.
pub const DATAVIEW_METHODS: &[&str] = &[
    "getInt8",
    "getUint8",
    "getInt16",
    "getUint16",
    "getInt32",
    "getUint32",
    "getFloat32",
    "getFloat64",
    "getBigInt64",
    "getBigUint64",
    "setInt8",
    "setUint8",
    "setInt16",
    "setUint16",
    "setInt32",
    "setUint32",
    "setFloat32",
    "setFloat64",
    "setBigInt64",
    "setBigUint64",
];

/// `new DataView(buffer[, byteOffset[, byteLength]])`.
pub fn construct_dataview(args: &[Value]) -> Result<Value, String> {
    let buf = args.first().cloned().unwrap_or(Value::Undef);
    if super::native_tag(&buf).as_deref() != Some("ArrayBuffer") {
        return Err(crate::host::type_error(
            "First argument to DataView constructor must be an ArrayBuffer",
        ));
    }
    let total = buffer_byte_length(&buf);
    let off = super::arg_num(args, 1).max(0.0) as usize;
    if off > total {
        return Err(crate::host::range_error(
            "Start offset is outside the bounds of the buffer",
        ));
    }
    let len = match args.get(2) {
        Some(Value::Undef) | None => total - off,
        Some(_) => super::arg_num(args, 2).max(0.0) as usize,
    };
    if off + len > total {
        return Err(crate::host::range_error("Invalid DataView length"));
    }
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("DataView"));
        m.insert("@@buffer".into(), buf.clone());
        m.insert("buffer".into(), buf.clone());
        m.insert("byteOffset".into(), Value::Float(off as f64));
        m.insert("byteLength".into(), Value::Float(len as f64));
        let obj = h.new_object(m);
        for k in ["buffer", "byteOffset", "byteLength"] {
            h.hide_prop(&obj, k);
        }
        h.ensure_native_protos();
        if let Some(p) = h.ensure_ctor_proto("DataView") {
            h.set_proto(&obj, p);
        }
        obj
    }))
}

/// `dv.getUint16(off[, littleEndian])` and its siblings. A `DataView` defaults
/// to BIG-endian, unlike a typed array, which is the whole reason it exists.
pub fn dataview_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    if view_detached(recv) {
        return Err(detached_error("DataView.prototype", method, false));
    }
    let Some(spec) = method.get(3..) else {
        return Err(crate::host::type_error(&format!(
            "{method} is not a function"
        )));
    };
    let width = match spec {
        "Int8" | "Uint8" => 1,
        "Int16" | "Uint16" => 2,
        "Int32" | "Uint32" | "Float32" => 4,
        "Float64" | "BigInt64" | "BigUint64" => 8,
        _ => {
            return Err(crate::host::type_error(&format!(
                "{method} is not a function"
            )))
        }
    };
    let is_get = method.starts_with("get");
    // `ToIndex(requestIndex)` (25.3.1.1 step 3): NaN is 0 and a fraction
    // truncates toward zero, so `dv.getUint8(1.9)` reads index 1. A NEGATIVE
    // index was being clamped to 0 — `dv.getUint8(-2)` quietly read the first
    // byte where node reports the out-of-bounds RangeError.
    let requested = super::arg_num(args, 0);
    let requested = if requested.is_nan() {
        0.0
    } else {
        requested.trunc()
    };
    let span = with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("byteLength").map(|l| h.to_number(l)).unwrap_or(0.0),
        _ => 0.0,
    });
    if requested < 0.0 || requested + width as f64 > span {
        return Err(crate::host::range_error(
            "Offset is outside the bounds of the DataView",
        ));
    }
    let at = requested as usize;
    // The endianness flag is the LAST argument, and it is the second for a
    // getter but the third for a setter.
    let le = with_host(|h| {
        h.truthy(
            args.get(if is_get { 1 } else { 2 })
                .unwrap_or(&Value::Undef),
        )
    });
    if is_get {
        let mut b = view_bytes(recv, at, width).unwrap_or_else(|| vec![0; width]);
        if !le {
            b.reverse();
        }
        return Ok(match spec {
            "Int8" => Value::Float(b[0] as i8 as f64),
            "Uint8" => Value::Float(b[0] as f64),
            "Int16" => Value::Float(i16::from_le_bytes([b[0], b[1]]) as f64),
            "Uint16" => Value::Float(u16::from_le_bytes([b[0], b[1]]) as f64),
            "Int32" => Value::Float(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
            "Uint32" => Value::Float(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
            "Float32" => Value::Float(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
            "Float64" => Value::Float(f64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]))),
            "BigInt64" => {
                let raw = i64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
                with_host(|h| h.new_bigint(num_bigint::BigInt::from(raw)))
            }
            _ => {
                let raw = u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
                with_host(|h| h.new_bigint(num_bigint::BigInt::from(raw)))
            }
        });
    }
    let val = args.get(1).cloned().unwrap_or(Value::Undef);
    let mut b = match spec {
        "BigInt64" | "BigUint64" => {
            use num_traits::cast::ToPrimitive;
            // `setBigInt64`/`setBigUint64` take `ToBigInt(value)` (25.3.4.x via
            // `SetViewValue` step 5), the same conversion an element write does.
            let big = crate::builtins::to_bigint(&val)?;
            let raw = if spec == "BigInt64" {
                big.to_i64().unwrap_or(0) as u64
            } else {
                big.to_u64().unwrap_or(0)
            };
            raw.to_le_bytes().to_vec()
        }
        _ => {
            let n = with_host(|h| h.to_number(&val));
            match spec {
                "Int8" | "Uint8" => vec![n as i64 as u8],
                "Int16" | "Uint16" => (n as i64 as u16).to_le_bytes().to_vec(),
                "Int32" | "Uint32" => (n as i64 as u32).to_le_bytes().to_vec(),
                "Float32" => (n as f32).to_le_bytes().to_vec(),
                _ => n.to_le_bytes().to_vec(),
            }
        }
    };
    if !le {
        b.reverse();
    }
    write_view_bytes(recv, at, &b);
    Ok(Value::Undef)
}

/// `ab.resize(n)` on a resizable buffer — grows with zeros or truncates,
/// in place, so every view over it sees the new size.
pub fn buffer_resize(ab: &Value, args: &[Value]) -> Result<Value, String> {
    let max = with_host(|h| match h.get(ab) {
        Some(JsObj::Object(p)) => p.get("@@maxByteLength").map(|m| h.to_number(m) as usize),
        _ => None,
    })
    .ok_or_else(|| {
        crate::host::type_error(
            "ArrayBuffer.prototype.resize called on a non-resizable ArrayBuffer",
        )
    })?;
    let n = super::arg_num(args, 0).max(0.0) as usize;
    if n > max {
        return Err(crate::host::range_error("Invalid array buffer length"));
    }
    let store = store_of(ab);
    with_host(|h| {
        if let Some(a) = store {
            if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                items.resize(n, Value::Float(0.0));
            }
        }
        if let Some(JsObj::Object(p)) = h.get_mut(ab) {
            p.insert("byteLength".into(), Value::Float(n as f64));
        }
    });
    Ok(Value::Undef)
}

/// Overwrite an `ArrayBuffer`'s bytes wholesale, for a producer that computed
/// them outside the heap.
pub fn write_buffer_bytes(ab: &Value, bytes: &[u8]) {
    let Some(store) = store_of(ab) else { return };
    with_host(|h| {
        if let Some(JsObj::Array(items)) = h.get_mut(&store) {
            *items = bytes.iter().map(|b| Value::Float(*b as f64)).collect();
        }
        if let Some(JsObj::Object(p)) = h.get_mut(ab) {
            p.insert("byteLength".into(), Value::Float(bytes.len() as f64));
        }
    });
}

/// The heap array an `ArrayBuffer` keeps its bytes in, so another view can
/// share it rather than copy.
pub fn buffer_store(ab: &Value) -> Option<Value> {
    store_of(ab)
}

/// A COPY of an `ArrayBuffer`'s bytes, for the callers that only read.
pub fn buffer_bytes_snapshot(ab: &Value) -> Option<Vec<u8>> {
    let store = store_of(ab)?;
    with_host(|h| match h.get(&store) {
        Some(JsObj::Array(items)) => {
            Some(items.iter().map(|x| h.to_number(x) as i64 as u8).collect())
        }
        _ => None,
    })
}

/// An `ArrayBuffer`'s byte length, from its own store.
pub fn buffer_byte_length(ab: &Value) -> usize {
    with_host(|h| match h.get(ab) {
        Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.len(),
            _ => 0,
        },
        _ => 0,
    })
}

/// `ArrayBuffer.prototype.slice(begin[, end])` — a COPY of the byte range, as a
/// new buffer. Writes to it are not seen by views over the original.
pub fn buffer_slice(ab: &Value, args: &[Value]) -> Value {
    let total = buffer_byte_length(ab) as i64;
    let idx = |v: Option<&Value>, dflt: i64| -> usize {
        let n = match v {
            None | Some(Value::Undef) => dflt,
            Some(x) => with_host(|h| h.to_number(x)) as i64,
        };
        (if n < 0 { total + n } else { n }).clamp(0, total) as usize
    };
    let start = idx(args.first(), 0);
    let end = idx(args.get(1), total).max(start);
    let out = new_array_buffer(end - start);
    let src = with_host(|h| match h.get(ab) {
        Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items[start..end].to_vec(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    });
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get(&out) {
            if let Some(arr) = p.get("@@bytes").cloned() {
                if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                    *items = src;
                }
            }
        }
    });
    out
}

/// Element vector for a typed-array construction from its first argument:
/// a number → that many zeroed slots; an array/iterable/typed-array → its coerced
/// values; otherwise → empty.
fn build_elems(kind: &str, args: &[Value]) -> Result<Vec<Value>, String> {
    match args.first() {
        None | Some(Value::Undef) => Ok(Vec::new()),
        Some(Value::Int(_)) | Some(Value::Float(_)) => {
            let n = super::arg_num(args, 0).max(0.0) as usize;
            Ok(vec![zero_of(kind); n])
        }
        Some(v) => {
            // Another typed array / Buffer → copy its elements; anything else
            // iterable → coerce each entry.
            let items = match super::native_tag(v).as_deref() {
                Some("TypedArray") | Some("Buffer") => elem_values(v),
                _ => crate::host::iter_all(v).unwrap_or_default(),
            };
            items.iter().map(|x| coerce_val(kind, x)).collect()
        }
    }
}

// ── Uint8Array base64/hex (the "Uint8Array to/from base64" proposal) ──────────

/// How much of a trailing partial base64 chunk `fromBase64`/`setFromBase64`
/// accept. The default is `loose`, which is why an UNPADDED string decodes.
#[derive(Clone, Copy, PartialEq)]
enum LastChunk {
    Loose,
    Strict,
    StopBeforePartial,
}

/// Read the `{ alphabet, lastChunkHandling }` options object. Both reject an
/// unknown value with node's `invalid option <v>`, and a non-object that is not
/// `undefined` is `invalid_argument` — not the usual "must be an object".
fn base64_options(opt: Option<&Value>) -> Result<(bool, LastChunk), String> {
    let Some(o) = opt.filter(|v| !matches!(v, Value::Undef)) else {
        return Ok((false, LastChunk::Loose));
    };
    if !with_host(|h| matches!(h.get(o), Some(JsObj::Object(_)))) {
        return Err(crate::host::type_error("invalid_argument"));
    }
    let read = |k: &str| {
        with_host(|h| match h.get(o) {
            Some(JsObj::Object(p)) => p.get(k).filter(|v| !matches!(v, Value::Undef)).cloned(),
            _ => None,
        })
    };
    let url = match read("alphabet") {
        None => false,
        Some(v) => match with_host(|h| h.str_of(&v)).as_str() {
            "base64" => false,
            "base64url" => true,
            other => return Err(crate::host::type_error(&format!("invalid option {other}"))),
        },
    };
    let last = match read("lastChunkHandling") {
        None => LastChunk::Loose,
        Some(v) => match with_host(|h| h.str_of(&v)).as_str() {
            "loose" => LastChunk::Loose,
            "strict" => LastChunk::Strict,
            "stop-before-partial" => LastChunk::StopBeforePartial,
            other => return Err(crate::host::type_error(&format!("invalid option {other}"))),
        },
    };
    Ok((url, last))
}

const B64_BAD: &str =
    "SyntaxError: Found a character that cannot be part of a valid base64 string.";
const B64_SINGLE: &str =
    "SyntaxError: The base64 input terminates with a single character, excluding padding (=).";

/// Decode base64 STRICTLY, reporting how many characters were consumed.
///
/// The lenient decoder behind `atob` cannot serve here: this has to reject a
/// stray `=`, a wrong pad count and a character outside the selected alphabet,
/// and `stop-before-partial` needs the consumed count rather than just the
/// bytes. ASCII whitespace is skipped, which node also allows.
fn decode_base64_strict(s: &str, url: bool, last: LastChunk) -> Result<(Vec<u8>, usize), String> {
    let value = |c: char| -> Option<u32> {
        let table = if url {
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
        } else {
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        };
        table.find(c).map(|i| i as u32)
    };
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut chunk: Vec<u32> = Vec::new();
    let mut consumed = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == '=' {
            // Padding closes the chunk, and only a 2- or 3-character chunk may
            // be padded: `QQ=` and `AA===` are both errors.
            let pads = chars[i..].iter().filter(|c| **c == '=').count();
            let rest_ok = chars[i..]
                .iter()
                .all(|c| *c == '=' || c.is_ascii_whitespace());
            let want = 4 - chunk.len();
            if !rest_ok || chunk.len() < 2 || pads != want {
                return Err(B64_BAD.into());
            }
            out.extend(flush_base64_chunk(&chunk));
            return Ok((out, chars.len()));
        }
        let Some(v) = value(c) else {
            return Err(B64_BAD.into());
        };
        chunk.push(v);
        i += 1;
        if chunk.len() == 4 {
            out.extend(flush_base64_chunk(&chunk));
            chunk.clear();
            consumed = i;
        }
    }
    match chunk.len() {
        0 => Ok((out, consumed)),
        // A single leftover character encodes nothing at all.
        1 if last != LastChunk::StopBeforePartial => Err(B64_SINGLE.into()),
        _ if last == LastChunk::StopBeforePartial => Ok((out, consumed)),
        1 => Ok((out, consumed)),
        _ if last == LastChunk::Strict => Err(B64_SINGLE.into()),
        _ => {
            out.extend(flush_base64_chunk(&chunk));
            Ok((out, chars.len()))
        }
    }
}

/// The 1-3 bytes a base64 chunk of 2, 3 or 4 sextets encodes.
fn flush_base64_chunk(chunk: &[u32]) -> Vec<u8> {
    let mut acc = 0u32;
    for v in chunk {
        acc = (acc << 6) | v;
    }
    let bytes = chunk.len() - 1;
    acc <<= 6 * (4 - chunk.len());
    let all = [(acc >> 16) as u8, (acc >> 8) as u8, acc as u8];
    all[..bytes].to_vec()
}

const HEX_BAD: &str = "SyntaxError: Input string must contain hex characters in even length";

/// Decode hex STRICTLY. Node reports the same message for an odd length and for
/// a non-hex character, so `"gg"` and `"0"` fail identically.
fn decode_hex_strict(s: &str) -> Result<Vec<u8>, String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() % 2 != 0 || !chars.iter().all(|c| c.is_ascii_hexdigit()) {
        return Err(HEX_BAD.into());
    }
    Ok(chars
        .chunks(2)
        .map(|p| {
            let hi = p[0].to_digit(16).expect("checked");
            let lo = p[1].to_digit(16).expect("checked");
            (hi * 16 + lo) as u8
        })
        .collect())
}

/// The string argument these six all take, rejecting anything else the way node
/// does rather than coercing it.
fn base64_input(args: &[Value]) -> Result<String, String> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    let is_str = matches!(v, Value::Str(_))
        || with_host(|h| matches!(h.get(&v), Some(crate::host::JsObj::Str(_))));
    if !is_str {
        return Err(crate::host::type_error("input argument must be a string"));
    }
    Ok(with_host(|h| h.str_of(&v)))
}

/// `Uint8Array.fromBase64` / `Uint8Array.fromHex`.
fn from_base64_static(method: &str, args: &[Value]) -> Result<Value, String> {
    let s = base64_input(args)?;
    let bytes = if method == "fromHex" {
        decode_hex_strict(&s)?
    } else {
        let (url, last) = base64_options(args.get(1))?;
        decode_base64_strict(&s, url, last)?.0
    };
    Ok(make(
        "Uint8Array",
        bytes.iter().map(|b| Value::Float(*b as f64)).collect(),
    ))
}

/// `Uint8Array.from(iterable[, mapFn])` / `Uint8Array.of(...items)`, and the
/// base64/hex statics.
pub fn static_call(kind: &str, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    // The base64/hex statics are on `Uint8Array` ONLY — no other view has them.
    if matches!(method, "fromBase64" | "fromHex") {
        if kind != "Uint8Array" {
            return None;
        }
        return Some(from_base64_static(method, args));
    }
    Some(match method {
        "of" => args
            .iter()
            .map(|x| coerce_val(kind, x))
            .collect::<Result<Vec<Value>, String>>()
            .map(|e| make(kind, e)),
        "from" => from(kind, args),
        // `ArrayBuffer.isView(x)` — true for a typed array or a Buffer (which is
        // a Uint8Array view), false for the backing ArrayBuffer itself.
        "isView" => Ok(Value::Bool(with_host(|h| {
            matches!(
                h.get(&args.first().cloned().unwrap_or(Value::Undef)),
                Some(crate::host::JsObj::Object(p))
                    if matches!(
                        p.get("@@native").map(|t| h.str_of(t)).as_deref(),
                        Some("TypedArray") | Some("Buffer") | Some("DataView")
                    )
            )
        }))),
        _ => return None,
    })
}

fn from(kind: &str, args: &[Value]) -> Result<Value, String> {
    let src = args.first().cloned().unwrap_or(Value::Undef);
    let map_fn = args
        .get(1)
        .cloned()
        .filter(|f| with_host(|h| crate::host::is_callable(h, f)));
    let items = if let Some(e) = elems_of(&src) {
        e.into_iter().map(Value::Float).collect()
    } else {
        crate::host::iter_all(&src).unwrap_or_default()
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, it) in items.into_iter().enumerate() {
        let mapped = match &map_fn {
            Some(f) => crate::host::invoke(f, vec![it, Value::Float(i as f64)], None)?,
            None => it,
        };
        out.push(coerce_val(kind, &mapped)?);
    }
    Ok(make(kind, out))
}

/// The element values of a typed array / Buffer (`None` for anything else).
pub fn elems_of(v: &Value) -> Option<Vec<f64>> {
    let tag = super::native_tag(v)?;
    if !matches!(tag.as_str(), "TypedArray" | "Buffer") {
        return None;
    }
    let vals = elem_values(v);
    Some(with_host(|h| vals.iter().map(|x| h.to_number(x)).collect()))
}

/// The number of elements `v` exposes as integer-index own properties, for a
/// typed array (its view length) or a `Buffer` (`@@bytes`); `None` otherwise.
///
/// Both index-membership questions — `obj.hasOwnProperty(i)` and `i in obj` —
/// must answer from this one place. They used to disagree: `hasOwnProperty`
/// carried a hand-rolled arm that understood `@@bytes` only, so it was right for
/// a Buffer and wrong for every other typed array, while the `in` operator knew
/// about neither and reported false for every valid index of both.
pub fn index_len(v: &Value) -> Option<usize> {
    match super::native_tag(v)?.as_str() {
        "TypedArray" => Some(view_len(v)),
        "Buffer" => with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|a| h.get(a)) {
                Some(JsObj::Array(items)) => Some(items.len()),
                _ => None,
            },
            _ => None,
        }),
        _ => None,
    }
}

/// Whether `key` is an in-range integer index of the typed array / Buffer `v`.
/// `None` when `v` is neither, so callers can fall through to their own logic.
pub fn has_index(v: &Value, key: &str) -> Option<bool> {
    let len = index_len(v)?;
    Some(key.parse::<usize>().map(|i| i < len).unwrap_or(false))
}

/// The `@@kind` of a typed-array receiver (defaults to `Uint8Array`).
pub fn kind_of(recv: &Value) -> String {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p
            .get("@@kind")
            .map(|v| h.str_of(v))
            .unwrap_or_else(|| "Uint8Array".into()),
        _ => "Uint8Array".into(),
    })
}

// ── backing store ────────────────────────────────────────────────────────────
//
// Every view — typed array or `DataView` — reads and writes THROUGH an
// `ArrayBuffer`, which owns the only copy of the bytes as a hidden `@@bytes`
// heap array. That is what makes two views over one buffer see each other's
// writes: `new Uint32Array(ab)[0]` reflects a byte written through
// `new Uint8Array(ab)`. Before this an `ArrayBuffer` carried nothing but a
// `byteLength` and each view owned a private element vector, so nothing was
// ever shared and `DataView` did not exist at all.

/// Allocate an `ArrayBuffer` of `n` zeroed bytes.
pub fn new_array_buffer(n: usize) -> Value {
    with_host(|h| {
        let arr = h.new_array(vec![Value::Float(0.0); n]);
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("ArrayBuffer"));
        m.insert("@@bytes".into(), arr);
        m.insert("byteLength".into(), Value::Float(n as f64));
        // `detached` is a prototype accessor in the spec; kept as a hidden own
        // property here so it reads back without appearing in `Object.keys` or
        // `console.log`, the same way `byteLength` is.
        m.insert("detached".into(), Value::Bool(false));
        // A FIXED buffer still reports both, as `false` and its own length —
        // they are prototype accessors in the spec, so they always answer.
        m.insert("resizable".into(), Value::Bool(false));
        m.insert("maxByteLength".into(), Value::Float(n as f64));
        let obj = h.new_object(m);
        for k in ["byteLength", "detached", "resizable", "maxByteLength"] {
            h.hide_prop(&obj, k);
        }
        // `ensure_ctor_proto` builds the prototype WITH a `constructor` slot, so
        // `ab.constructor.name` reports `ArrayBuffer` rather than `Object`.
        if let Some(p) = h.ensure_ctor_proto("ArrayBuffer") {
            h.set_proto(&obj, p);
        }
        obj
    })
}

/// Whether `ab` has been DETACHED — its bytes handed to another buffer by
/// `transfer`, or given away by `structuredClone`'s `transfer` option.
///
/// A detached buffer is not an empty one: reading a view over it answers
/// `undefined` and its `length` is 0, but every METHOD on that view throws.
pub fn is_detached(ab: &Value) -> bool {
    with_host(|h| match h.get(ab) {
        Some(JsObj::Object(p)) => p.get("detached").map(|v| h.truthy(v)).unwrap_or(false),
        _ => false,
    })
}

/// Whether `v` is a view whose backing buffer has been detached.
pub fn view_detached(v: &Value) -> bool {
    with_host(|h| view_detached_h(h, v))
}

/// `view_detached` for a caller that already holds the host borrow — the
/// iteration entry point runs under one, and re-entering aborts the process.
pub fn view_detached_h(h: &crate::host::JsHost, v: &Value) -> bool {
    let buf = match h.get(v) {
        Some(JsObj::Object(p)) => p.get("@@buffer").cloned(),
        _ => None,
    };
    match buf.and_then(|b| match h.get(&b) {
        Some(JsObj::Object(p)) => p.get("detached").cloned(),
        _ => None,
    }) {
        Some(d) => h.truthy(&d),
        None => false,
    }
}

/// Detach `ab`: drop its bytes and mark it, so every later read reports zero
/// length and every method over it throws.
pub fn detach_buffer(ab: &Value) {
    detach(ab)
}

fn detach(ab: &Value) {
    with_host(|h| {
        let empty = h.new_array(Vec::new());
        if let Some(JsObj::Object(p)) = h.get_mut(ab) {
            p.insert("@@bytes".into(), empty);
            p.insert("byteLength".into(), Value::Float(0.0));
            p.insert("detached".into(), Value::Bool(true));
        }
        h.hide_prop(ab, "byteLength");
        h.hide_prop(ab, "detached");
    });
}

/// `ArrayBuffer.prototype.transfer([newLength])` and `transferToFixedLength`.
///
/// A fresh buffer takes the bytes — truncated or zero-padded to `newLength` —
/// and the receiver is detached. The two differ only in whether the result may
/// still grow.
pub fn buffer_transfer(ab: &Value, args: &[Value], fixed: bool) -> Result<Value, String> {
    let method = if fixed {
        "transferToFixedLength"
    } else {
        "transfer"
    };
    if is_detached(ab) {
        return Err(crate::host::type_error(&format!(
            "Cannot perform ArrayBuffer.prototype.{method} on a detached ArrayBuffer"
        )));
    }
    let old = byte_len_of(ab);
    let new_len = match args.first().filter(|v| !matches!(v, Value::Undef)) {
        Some(v) => with_host(|h| h.to_number(v)).max(0.0) as usize,
        None => old,
    };
    let mut bytes = view_bytes_of_buffer(ab, old);
    bytes.resize(new_len, 0);
    let out = new_array_buffer(new_len);
    write_buffer_bytes(&out, &bytes);
    if !fixed {
        // `transfer` keeps the source's resizability; `transferToFixedLength`
        // never does.
        let resizable = with_host(|h| match h.get(ab) {
            Some(JsObj::Object(p)) => p.contains_key("@@maxByteLength"),
            _ => false,
        });
        if resizable {
            let max = with_host(|h| match h.get(ab) {
                Some(JsObj::Object(p)) => p.get("@@maxByteLength").cloned(),
                _ => None,
            });
            if let Some(max) = max {
                with_host(|h| {
                    if let Some(JsObj::Object(p)) = h.get_mut(&out) {
                        p.insert("@@maxByteLength".into(), max);
                    }
                });
            }
        }
    }
    detach(ab);
    Ok(out)
}

/// An ArrayBuffer's `byteLength`.
fn byte_len_of(ab: &Value) -> usize {
    with_host(|h| match h.get(ab) {
        Some(JsObj::Object(p)) => {
            p.get("byteLength").map(|l| h.to_number(l)).unwrap_or(0.0) as usize
        }
        _ => 0,
    })
}

/// An ArrayBuffer's bytes.
fn view_bytes_of_buffer(ab: &Value, n: usize) -> Vec<u8> {
    let Some(store) = store_of(ab) else {
        return Vec::new();
    };
    with_host(|h| match h.get(&store) {
        Some(JsObj::Array(items)) => items
            .iter()
            .take(n)
            .map(|x| h.to_number(x) as i64 as u8)
            .collect(),
        _ => Vec::new(),
    })
}

/// The TypeError a method over a DETACHED buffer throws. Node names the method
/// and distinguishes a view's from a DataView's from the buffer's own.
pub fn detached_error(label: &str, method: &str, buffer_only: bool) -> String {
    let tail = if buffer_only {
        "a detached ArrayBuffer"
    } else {
        "a detached or out-of-bounds ArrayBuffer"
    };
    // A symbol-keyed member reports the name of the function it ALIASES, the way
    // node does everywhere else (`Set.prototype.keys` reports `values`):
    // `[...detachedView]` says `%TypedArray%.prototype.values`, never
    // `.@@iterator`, which is this frontend's internal spelling for
    // `Symbol.iterator` and not a name any script wrote.
    let method = match method {
        "@@iterator" => "values",
        other => other,
    };
    crate::host::type_error(&format!("Cannot perform {label}.{method} on {tail}"))
}

/// The heap array holding an `ArrayBuffer`'s bytes.
fn store_of(ab: &Value) -> Option<Value> {
    with_host(|h| match h.get(ab) {
        Some(JsObj::Object(p)) => p.get("@@bytes").cloned(),
        _ => None,
    })
}

/// A view's `(buffer, byteOffset)`.
fn view_base(v: &Value) -> Option<(Value, usize)> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => {
            let buf = p.get("@@buffer").cloned()?;
            let off = p.get("byteOffset").map(|o| h.to_number(o)).unwrap_or(0.0);
            Some((buf, off.max(0.0) as usize))
        }
        _ => None,
    })
}

/// `n` bytes of `v`'s buffer starting at its `byteOffset + at`.
pub fn view_bytes(v: &Value, at: usize, n: usize) -> Option<Vec<u8>> {
    let (buf, off) = view_base(v)?;
    let store = store_of(&buf)?;
    with_host(|h| match h.get(&store) {
        Some(JsObj::Array(items)) => {
            let start = off + at;
            if start + n > items.len() {
                return None;
            }
            Some(
                items[start..start + n]
                    .iter()
                    .map(|x| h.to_number(x) as i64 as u8)
                    .collect(),
            )
        }
        _ => None,
    })
}

/// Write `bytes` into `v`'s buffer at its `byteOffset + at`. False when the
/// range does not fit.
pub fn write_view_bytes(v: &Value, at: usize, bytes: &[u8]) -> bool {
    let Some((buf, off)) = view_base(v) else {
        return false;
    };
    let Some(store) = store_of(&buf) else {
        return false;
    };
    with_host(|h| match h.get_mut(&store) {
        Some(JsObj::Array(items)) => {
            let start = off + at;
            if start + bytes.len() > items.len() {
                return false;
            }
            for (i, b) in bytes.iter().enumerate() {
                items[start + i] = Value::Float(*b as f64);
            }
            true
        }
        _ => false,
    })
}

/// Decode one element of `kind` from its `bytes` (native byte order, which on
/// every architecture this runs on is little-endian).
fn decode(kind: &str, b: &[u8]) -> Value {
    match kind {
        "Int8Array" => Value::Float(b[0] as i8 as f64),
        "Uint8Array" | "Uint8ClampedArray" => Value::Float(b[0] as f64),
        "Int16Array" => Value::Float(i16::from_le_bytes([b[0], b[1]]) as f64),
        "Uint16Array" => Value::Float(u16::from_le_bytes([b[0], b[1]]) as f64),
        "Int32Array" => Value::Float(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
        "Uint32Array" => Value::Float(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
        "Float32Array" => Value::Float(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
        "BigInt64Array" => {
            let raw = i64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
            with_host(|h| h.new_bigint(num_bigint::BigInt::from(raw)))
        }
        "BigUint64Array" => {
            let raw = u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
            with_host(|h| h.new_bigint(num_bigint::BigInt::from(raw)))
        }
        _ => Value::Float(f64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]))),
    }
}

/// Encode one already-coerced element of `kind` into its bytes.
fn encode(kind: &str, v: &Value) -> Vec<u8> {
    if is_bigint_kind(kind) {
        use num_traits::cast::ToPrimitive;
        let b = bigint_of(v);
        let raw = if kind == "BigInt64Array" {
            b.to_i64().unwrap_or(0) as u64
        } else {
            b.to_u64().unwrap_or(0)
        };
        return raw.to_le_bytes().to_vec();
    }
    let n = num(v);
    match kind {
        "Int8Array" => vec![n as i64 as i8 as u8],
        "Uint8Array" | "Uint8ClampedArray" => vec![n as i64 as u8],
        "Int16Array" => (n as i64 as i16).to_le_bytes().to_vec(),
        "Uint16Array" => (n as i64 as u16).to_le_bytes().to_vec(),
        "Int32Array" => (n as i64 as i32).to_le_bytes().to_vec(),
        "Uint32Array" => (n as i64 as u32).to_le_bytes().to_vec(),
        "Float32Array" => (n as f32).to_le_bytes().to_vec(),
        _ => n.to_le_bytes().to_vec(),
    }
}

/// The elements of a typed-array view, decoded with a host borrow ALREADY
/// held. `host` reads views from inside `&self` methods (inspect, key
/// enumeration, iteration) where re-entering through `with_host` would panic on
/// the outstanding borrow.
pub fn elems_with_host(h: &crate::host::JsHost, v: &Value) -> Vec<Value> {
    if let Some(JsObj::Object(p)) = h.get(v) {
        if let Some(arr) = p.get("@@bytes") {
            return match h.get(arr) {
                Some(JsObj::Array(items)) => items.clone(),
                _ => Vec::new(),
            };
        }
    }
    let Some((kind, raws)) = raw_elems(h, v) else {
        return Vec::new();
    };
    // A 64-bit element is a BigInt, which needs an allocation this borrow
    // cannot make; `elems_mut_host` is the reader for callers that can.
    if is_bigint_kind(&kind) {
        return vec![Value::Undef; raws.len()];
    }
    raws.iter().map(|b| decode(&kind, b)).collect()
}

/// The raw bytes of every element of a view, with the host borrow already held.
/// The shared half of the three readers below.
fn raw_elems(h: &crate::host::JsHost, v: &Value) -> Option<(String, Vec<Vec<u8>>)> {
    let JsObj::Object(p) = h.get(v)? else {
        return None;
    };
    let kind = p
        .get("@@kind")
        .map(|k| h.str_of(k))
        .unwrap_or_else(|| "Uint8Array".into());
    let bpe = bytes_per_element(&kind);
    // A view over a DETACHED buffer has no elements. Its own `length` still
    // holds the old count, so `util.inspect` showed `Uint8Array(4) [0,0,0,0]`
    // over a buffer with no bytes left.
    let len = if view_detached_h(h, v) {
        0
    } else {
        p.get("length").map(|l| h.to_number(l)).unwrap_or(0.0) as usize
    };
    let off = p.get("byteOffset").map(|o| h.to_number(o)).unwrap_or(0.0) as usize;
    let store = match p.get("@@buffer").and_then(|b| h.get(b)) {
        Some(JsObj::Object(bp)) => bp.get("@@bytes").and_then(|a| h.get(a)),
        _ => None,
    };
    let JsObj::Array(bytes) = store? else {
        return None;
    };
    let out = (0..len)
        .map(|i| {
            let start = off + i * bpe;
            if start + bpe > bytes.len() {
                return vec![0u8; bpe];
            }
            bytes[start..start + bpe]
                .iter()
                .map(|x| h.to_number(x) as i64 as u8)
                .collect()
        })
        .collect();
    Some((kind, out))
}

/// The elements of a view with a MUTABLE host borrow held, so the two 64-bit
/// kinds can allocate their BigInts. This is the complete reader; the `&self`
/// one below cannot allocate and so answers `undefined` for those two kinds.
pub fn elems_mut_host(h: &mut crate::host::JsHost, v: &Value) -> Vec<Value> {
    if let Some(JsObj::Object(p)) = h.get(v) {
        if let Some(arr) = p.get("@@bytes").cloned() {
            return match h.get(&arr) {
                Some(JsObj::Array(items)) => items.clone(),
                _ => Vec::new(),
            };
        }
    }
    let Some((kind, raws)) = raw_elems(h, v) else {
        return Vec::new();
    };
    raws.iter()
        .map(|b| {
            if !is_bigint_kind(&kind) {
                return decode(&kind, b);
            }
            let raw = u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
            h.new_bigint(if kind == "BigInt64Array" {
                num_bigint::BigInt::from(raw as i64)
            } else {
                num_bigint::BigInt::from(raw)
            })
        })
        .collect()
}

/// Every element rendered for display, for `util.inspect` — which holds a
/// shared borrow and so cannot allocate the BigInt a 64-bit element would need
/// as a `Value`. Elements are always primitives, so a string loses nothing.
pub fn elems_display(h: &crate::host::JsHost, v: &Value) -> Vec<String> {
    let Some((kind, raws)) = raw_elems(h, v) else {
        return Vec::new();
    };
    raws.iter()
        .map(|b| {
            if !is_bigint_kind(&kind) {
                return h.inspect(&decode(&kind, b));
            }
            let raw = u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
            if kind == "BigInt64Array" {
                format!("{}n", raw as i64)
            } else {
                format!("{raw}n")
            }
        })
        .collect()
}

/// The element count a view exposes, from its own `length` slot.
fn view_len(v: &Value) -> usize {
    // A view over a DETACHED buffer has length 0 — its own `length` property
    // still holds the old count, which is why this cannot just read it.
    if view_detached(v) {
        return 0;
    }
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get("length").map(|l| h.to_number(l)).unwrap_or(0.0) as usize,
        _ => 0,
    })
}

// ── element indexing (called from builtins::get_property/set_property) ────────

/// `ta[i]` read: the element at char/index `i`, or `None` if `i` is out of range
/// or not an integer index.
pub fn elem_get(recv: &Value, key: &str) -> Option<Value> {
    let i: usize = key.parse().ok()?;
    if i >= view_len(recv) {
        return None;
    }
    let kind = kind_of(recv);
    let bpe = bytes_per_element(&kind);
    let bytes = view_bytes(recv, i * bpe, bpe)?;
    Some(decode(&kind, &bytes))
}

/// `ta[i] = v` write (coerced to the kind). Returns true if `i` is a valid index.
pub fn elem_set(recv: &Value, key: &str, val: &Value) -> Result<bool, String> {
    let Ok(i) = key.parse::<usize>() else {
        return Ok(false);
    };
    let kind = kind_of(recv);
    // Coerced through the element type, so writing a Number into a 64-bit view
    // throws rather than storing an un-typed element.
    let n = coerce_val(&kind, val)?;
    if i >= view_len(recv) {
        return Ok(false);
    }
    let bpe = bytes_per_element(&kind);
    Ok(write_view_bytes(recv, i * bpe, &encode(&kind, &n)))
}

/// Build a result of the same "species" as `recv`: a `Buffer` receiver yields a
/// `Buffer`, every other typed array yields its own kind. Node picks the result
/// type from the receiver's constructor, so `Buffer.from([1]).map(f)` is a
/// Buffer and `new Int32Array([1]).map(f)` is an `Int32Array`.
fn species(recv: &Value, kind: &str, elems: Vec<Value>) -> Value {
    if super::native_tag(recv).as_deref() == Some("Buffer") {
        let bytes: Vec<u8> = elems.iter().map(|x| num(x) as i64 as u8).collect();
        return super::buffer::from_bytes(&bytes);
    }
    make(kind, elems)
}

/// Overwrite `recv`'s elements in place, for the methods that mutate and return
/// the receiver (`fill`, `reverse`, `sort`, `copyWithin`). Writes through to
/// whichever store backs it — the `ArrayBuffer` for a typed array, `@@bytes` for
/// a `Buffer`.
fn write_elems(recv: &Value, kind: &str, vals: &[Value]) -> Result<(), String> {
    if super::native_tag(recv).as_deref() == Some("TypedArray") {
        let bpe = bytes_per_element(kind);
        let coerced: Vec<Value> = vals
            .iter()
            .map(|v| coerce_val(kind, v))
            .collect::<Result<_, _>>()?;
        for (i, v) in coerced.iter().enumerate() {
            write_view_bytes(recv, i * bpe, &encode(kind, v));
        }
        return Ok(());
    }
    let field = "@@bytes";
    // Coerce OUTSIDE the host borrow: `coerce_val` re-enters the host to read a
    // BigInt and to allocate the wrapped one.
    let coerced: Vec<Value> = vals
        .iter()
        .map(|v| coerce_val(kind, v))
        .collect::<Result<_, _>>()?;
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get(recv) {
            if let Some(arr) = p.get(field).cloned() {
                if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                    for (i, v) in coerced.into_iter().enumerate() {
                        if i < items.len() {
                            items[i] = v;
                        }
                    }
                }
            }
        }
    });
    Ok(())
}

/// Order `elems` the way `%TypedArray%.prototype.sort` (23.2.3.29) does, with
/// `cmp` as the optional user comparator. Shared with `toSorted` (23.2.3.33),
/// which is the same ordering over a copy.
fn sort_elements(elems: &mut Vec<Value>, kind: &str, cmp: Option<&Value>) -> Result<(), String> {
    let cmp = cmp.cloned().unwrap_or(Value::Undef);
    if with_host(|h| crate::host::is_callable(h, &cmp)) {
        // A user comparator goes through the same fallible merge sort
        // `Array.prototype.sort` uses: O(n log n) rather than the insertion sort
        // this was, and a comparator returning NaN keeps the pair's order
        // (23.2.4.1 step 3: NaN is +0) instead of swapping, which the `<= 0.0`
        // break got wrong.
        return crate::builtins::sort_values(elems, Some(&cmp));
    }
    // A typed array sorts NUMERICALLY by default, unlike `Array` which sorts by
    // string. Verified against node v26.7.0: `new Uint8Array([10,9,1]).sort()`
    // is `1,9,10` while `[10,9,1].sort()` is `1,10,9`.
    // A BigInt element cannot be ordered through an `f64` without collapsing
    // values more than 2^53 apart, so the 64-bit views compare the integers
    // themselves.
    if is_bigint_kind(kind) {
        let keys: Vec<num_bigint::BigInt> = elems.iter().map(bigint_of).collect();
        let mut idx: Vec<usize> = (0..elems.len()).collect();
        idx.sort_by(|a, b| keys[*a].cmp(&keys[*b]));
        *elems = idx.into_iter().map(|i| elems[i].clone()).collect();
    } else {
        elems.sort_by(|a, b| {
            num(a)
                .partial_cmp(&num(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    Ok(())
}

/// Resolve a relative index argument against `len` (negative counts from the
/// end), clamped into range — the `RelativeIndex` coercion the typed-array
/// methods share.
fn rel_index(args: &[Value], idx: usize, len: usize, default: usize) -> usize {
    if args.len() <= idx {
        return default;
    }
    let n = super::arg_num(args, idx);
    if n < 0.0 {
        (len as f64 + n).max(0.0) as usize
    } else {
        (n as usize).min(len)
    }
}

/// Typed-array instance methods.
/// `toBase64` / `toHex` / `setFromBase64` / `setFromHex` — the `Uint8Array`
/// half of the base64/hex proposal. All four are brand-checked to `Uint8Array`:
/// every other view, and an ordinary array, is an incompatible receiver.
fn base64_instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let kind = kind_of(recv);
    if kind != "Uint8Array" {
        // Node renders a non-view receiver by its brand and a WRONG view as
        // `undefined`, which reads oddly but is what it prints.
        // A typed array of the WRONG element kind renders as `undefined` here,
        // which reads oddly but is what node prints; every other receiver is
        // rendered the way the other brand checks render one, and never reaches
        // this arm — the dispatcher's guard catches it first.
        return Err(crate::host::type_error(&format!(
            "Method Uint8Array.prototype.{method} called on incompatible receiver undefined"
        )));
    }
    let bytes: Vec<u8> = elem_values(recv)
        .iter()
        .map(|v| with_host(|h| h.to_number(v)) as u8)
        .collect();
    match method {
        "toBase64" => {
            let (url, _) = base64_options(args.first())?;
            let omit = args
                .first()
                .filter(|v| !matches!(v, Value::Undef))
                .map(|o| {
                    with_host(|h| match h.get(o) {
                        Some(JsObj::Object(p)) => {
                            p.get("omitPadding").map(|v| h.truthy(v)).unwrap_or(false)
                        }
                        _ => false,
                    })
                })
                .unwrap_or(false);
            // The url alphabet only swaps the two characters — it does NOT drop
            // the padding, which `to_base64url` does for the `atob` callers.
            let mut s = super::to_base64(&bytes);
            if url {
                s = s.replace('+', "-").replace('/', "_");
            }
            if omit {
                s = s.trim_end_matches('=').to_string();
            }
            Ok(with_host(|h| h.new_str(s)))
        }
        "toHex" => Ok(with_host(|h| h.new_str(super::to_hex(&bytes)))),
        // `setFrom*` writes as much as FITS and reports how far it got, so a
        // short target is not an error — it stops at the last whole chunk.
        "setFromBase64" | "setFromHex" => {
            let s = base64_input(args)?;
            let (decoded, read) = if method == "setFromHex" {
                let d = decode_hex_strict(&s)?;
                let fits = d.len().min(bytes.len());
                (d[..fits].to_vec(), fits * 2)
            } else {
                let (url, last) = base64_options(args.get(1))?;
                // Decode only as much as the target can hold: whole 4-character
                // chunks, plus the final partial one when it still fits.
                let whole = (bytes.len() / 3) * 4;
                let head: String = s.chars().take(whole).collect();
                let (mut d, mut consumed) = decode_base64_strict(&head, url, last)?;
                if d.len() < bytes.len() {
                    let (full, full_read) = decode_base64_strict(&s, url, last)?;
                    if full.len() <= bytes.len() {
                        d = full;
                        consumed = full_read;
                    }
                }
                (d, consumed)
            };
            write_view_bytes(recv, 0, &decoded);
            Ok(with_host(|h| {
                let mut m = IndexMap::new();
                m.insert("read".to_string(), Value::Float(read as f64));
                m.insert("written".to_string(), Value::Float(decoded.len() as f64));
                h.new_object(m)
            }))
        }
        _ => unreachable!("caller gates the method name"),
    }
}

pub fn instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    // Every method over a DETACHED buffer throws, naming itself. An element
    // read and `length` answer zero instead, which is why this is a per-method
    // guard rather than a check inside the element accessors.
    if view_detached(recv) {
        return Err(detached_error("%TypedArray%.prototype", method, false));
    }
    if matches!(
        method,
        "toBase64" | "toHex" | "setFromBase64" | "setFromHex"
    ) {
        return base64_instance_call(recv, method, args);
    }
    let kind = kind_of(recv);
    // Elements travel as `Value`, not `f64`: a 64-bit view's are BigInts, and
    // rounding them through a double is exactly the loss those views exist to
    // avoid. The numeric kinds still hold `Value::Float`, so nothing about them
    // changes.
    let elems = elem_values(recv);
    // The callback-taking methods share one shape: invoke `cb(value, index,
    // receiver)` per element. They are inherited by `Buffer` too, which is why
    // they must live here rather than in either concrete type.
    // `forEach(fn, thisArg)` and its siblings bind `thisArg` as the callback's
    // `this`; it was being dropped, so `this` inside the callback was undefined.
    let this_arg = args.get(1).filter(|v| !matches!(v, Value::Undef)).cloned();
    let call_cb = |i: usize, v: &Value| -> Result<Value, String> {
        crate::host::invoke(
            &args.first().cloned().unwrap_or(Value::Undef),
            vec![v.clone(), Value::Float(i as f64), recv.clone()],
            this_arg.clone(),
        )
    };
    match method {
        "every" => {
            for (i, v) in elems.iter().enumerate() {
                let r = call_cb(i, v)?;
                if !with_host(|h| h.truthy(&r)) {
                    return Ok(Value::Bool(false));
                }
            }
            Ok(Value::Bool(true))
        }
        "some" => {
            for (i, v) in elems.iter().enumerate() {
                let r = call_cb(i, v)?;
                if with_host(|h| h.truthy(&r)) {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        "forEach" => {
            for (i, v) in elems.iter().enumerate() {
                call_cb(i, v)?;
            }
            Ok(Value::Undef)
        }
        "map" => {
            let mut out = Vec::with_capacity(elems.len());
            for (i, v) in elems.iter().enumerate() {
                let r = call_cb(i, v)?;
                out.push(coerce_val(&kind, &r)?);
            }
            Ok(species(recv, &kind, out))
        }
        "filter" => {
            let mut out = Vec::new();
            for (i, v) in elems.iter().enumerate() {
                let r = call_cb(i, v)?;
                if with_host(|h| h.truthy(&r)) {
                    out.push(v.clone());
                }
            }
            Ok(species(recv, &kind, out))
        }
        "find" | "findIndex" | "findLast" | "findLastIndex" => {
            let last = method.starts_with("findLast");
            let idxs: Vec<usize> = if last {
                (0..elems.len()).rev().collect()
            } else {
                (0..elems.len()).collect()
            };
            for i in idxs {
                let r = call_cb(i, &elems[i])?;
                if with_host(|h| h.truthy(&r)) {
                    return Ok(if method.ends_with("Index") {
                        Value::Float(i as f64)
                    } else {
                        elems[i].clone()
                    });
                }
            }
            Ok(if method.ends_with("Index") {
                Value::Float(-1.0)
            } else {
                Value::Undef
            })
        }
        "reduce" | "reduceRight" => {
            let right = method == "reduceRight";
            let order: Vec<usize> = if right {
                (0..elems.len()).rev().collect()
            } else {
                (0..elems.len()).collect()
            };
            let cb = args.first().cloned().unwrap_or(Value::Undef);
            let mut it = order.into_iter();
            let mut acc = if args.len() >= 2 {
                args[1].clone()
            } else {
                match it.next() {
                    Some(i) => elems[i].clone(),
                    None => {
                        return Err(crate::host::type_error(
                            "Reduce of empty array with no initial value",
                        ))
                    }
                }
            };
            for i in it {
                acc = crate::host::invoke(
                    &cb,
                    vec![acc, elems[i].clone(), Value::Float(i as f64), recv.clone()],
                    None,
                )?;
            }
            Ok(acc)
        }
        "reverse" => {
            let mut out = elems.clone();
            out.reverse();
            write_elems(recv, &kind, &out)?;
            Ok(recv.clone())
        }
        "sort" => {
            let mut out = elems.clone();
            sort_elements(&mut out, &kind, args.first())?;
            write_elems(recv, &kind, &out)?;
            Ok(recv.clone())
        }
        "copyWithin" => {
            let len = elems.len();
            let target = rel_index(args, 0, len, 0);
            let start = rel_index(args, 1, len, 0);
            let end = rel_index(args, 2, len, len);
            let src: Vec<Value> = elems[start.min(end)..end.max(start)].to_vec();
            let mut out = elems.clone();
            for (k, v) in src.iter().enumerate() {
                if target + k < len {
                    out[target + k] = v.clone();
                }
            }
            write_elems(recv, &kind, &out)?;
            Ok(recv.clone())
        }
        "at" => {
            let n = super::arg_num(args, 0);
            let i = if n < 0.0 { elems.len() as f64 + n } else { n };
            if i < 0.0 || i >= elems.len() as f64 {
                return Ok(Value::Undef);
            }
            Ok(elems[i as usize].clone())
        }
        "lastIndexOf" => {
            let needle = args.first().cloned().unwrap_or(Value::Undef);
            let from = (args.len() > 1).then(|| super::arg_num(args, 1));
            let found = crate::builtins::search_start_last(from, elems.len()).and_then(|start| {
                elems[..=start]
                    .iter()
                    .rposition(|x| same_element(x, &needle, false))
            });
            Ok(Value::Float(found.map(|p| p as f64).unwrap_or(-1.0)))
        }
        // `%TypedArray%.prototype[Symbol.iterator]` IS `values` (23.2.3.35), so
        // it dispatches here rather than reporting itself missing:
        // `Uint8Array.prototype[Symbol.iterator].call(ta)` threw
        // `@@iterator is not a function`.
        "keys" | "values" | "entries" | "@@iterator" => {
            let items: Vec<Value> = with_host(|h| match method {
                "keys" => (0..elems.len()).map(|i| Value::Float(i as f64)).collect(),
                "values" | "@@iterator" => elems.clone(),
                _ => elems
                    .iter()
                    .enumerate()
                    .map(|(i, v)| h.new_array(vec![Value::Float(i as f64), v.clone()]))
                    .collect(),
            });
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
        }
        "toString" | "join" => {
            let sep = if method == "join" && !args.is_empty() {
                super::arg_str(args, 0)
            } else {
                ",".into()
            };
            let parts: Vec<String> = with_host(|h| elems.iter().map(|n| h.str_of(n)).collect());
            Ok(with_host(|h| h.new_str(parts.join(&sep))))
        }
        "slice" | "subarray" => {
            let len = elems.len();
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
            let (lo, hi) = (s.min(e), e.max(s));
            // 23.2.3.30: `subarray` is a VIEW over the same buffer — writes
            // through it are seen by the original. `slice` copies (23.2.3.27).
            if method == "subarray" && super::native_tag(recv).as_deref() == Some("TypedArray") {
                if let Some((buf, off)) = view_base(recv) {
                    let bpe = bytes_per_element(&kind);
                    return Ok(make_view(&kind, &buf, off + lo * bpe, hi - lo));
                }
            }
            Ok(species(recv, &kind, elems[lo..hi].to_vec()))
        }
        "indexOf" => {
            let needle = args.first().cloned().unwrap_or(Value::Undef);
            let start = crate::builtins::search_start(super::arg_num(args, 1), elems.len());
            Ok(Value::Float(
                elems
                    .iter()
                    .skip(start)
                    .position(|x| same_element(x, &needle, false))
                    .map(|p| (p + start) as f64)
                    .unwrap_or(-1.0),
            ))
        }
        "includes" => {
            let needle = args.first().cloned().unwrap_or(Value::Undef);
            let start = crate::builtins::search_start(super::arg_num(args, 1), elems.len());
            Ok(Value::Bool(
                elems
                    .iter()
                    .skip(start)
                    .any(|x| same_element(x, &needle, true)),
            ))
        }
        // 23.2.3.9: `fill` writes THROUGH the view and answers the receiver. It
        // was building a fresh array instead, so the write was invisible —
        // `u.fill(9)` left `u` untouched, `u.fill(9) === u` was false, and a
        // second view onto the same `ArrayBuffer` saw none of it. The `start`
        // and `end` arguments were dropped too, so `fill(9, 1, 2)` overwrote the
        // whole array rather than one element.
        "fill" => {
            let len = elems.len();
            let v = coerce_val(&kind, args.first().unwrap_or(&Value::Undef))?;
            let start = rel_index(args, 1, len, 0);
            let end = rel_index(args, 2, len, len);
            let mut out = elems.clone();
            for slot in out.iter_mut().take(end).skip(start) {
                *slot = v.clone();
            }
            write_elems(recv, &kind, &out)?;
            Ok(recv.clone())
        }
        // The change-by-copy trio (23.2.3.32-34). Each answers a NEW view of the
        // receiver's own element kind — `TypedArrayCreateSameType`, not the
        // species path — so a `Buffer` receiver yields a `Uint8Array`, which is
        // what node reports.
        "toReversed" | "toSorted" => {
            let mut out = elems.clone();
            if method == "toReversed" {
                out.reverse();
            } else {
                sort_elements(&mut out, &kind, args.first())?;
            }
            Ok(make(&kind, out))
        }
        "with" => {
            let len = elems.len();
            let n = super::arg_num(args, 0);
            let i = if n < 0.0 { len as f64 + n } else { n };
            if !(0.0..len as f64).contains(&i) {
                return Err("RangeError: Invalid typed array index".into());
            }
            let mut out = elems.clone();
            out[i as usize] = coerce_val(&kind, args.get(1).unwrap_or(&Value::Undef))?;
            Ok(make(&kind, out))
        }
        "set" => {
            // `ta.set(src[, offset])` — write `src`'s values in place.
            let arg = args.first().cloned().unwrap_or(Value::Undef);
            let src = match super::native_tag(&arg).as_deref() {
                Some("TypedArray") | Some("Buffer") => elem_values(&arg),
                _ => crate::host::iter_all(&arg).unwrap_or_default(),
            };
            let off = super::arg_num(args, 1).max(0.0) as usize;
            // Coerced outside the host borrow: a 64-bit element allocates.
            let src: Vec<Value> = src
                .iter()
                .map(|v| coerce_val(&kind, v))
                .collect::<Result<_, _>>()?;
            let bpe = bytes_per_element(&kind);
            let len = view_len(recv);
            for (k, v) in src.into_iter().enumerate() {
                if off + k < len {
                    write_view_bytes(recv, (off + k) * bpe, &encode(&kind, &v));
                }
            }
            Ok(Value::Undef)
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

// ── WeakRef (strong-ref approximation) ────────────────────────────────────────

pub fn construct_weakref(args: &[Value]) -> Result<Value, String> {
    let target = args.first().cloned().unwrap_or(Value::Undef);
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("WeakRef"));
        m.insert("@@target".into(), target);
        h.new_object(m)
    }))
}

pub fn weakref_call(recv: &Value, method: &str) -> Result<Value, String> {
    match method {
        "deref" => Ok(with_host(|h| match h.get(recv) {
            Some(JsObj::Object(p)) => p.get("@@target").cloned().unwrap_or(Value::Undef),
            _ => Value::Undef,
        })),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

// ── FinalizationRegistry (no-GC approximation) ────────────────────────────────
//
// This VM holds every value strongly (see `WeakRef` above), so a registered
// target is never reclaimed and the cleanup callback never fires. The ECMAScript
// spec permits an implementation to never call cleanup callbacks, so this is a
// conformant approximation: the constructor and `register`/`unregister` enforce
// their type checks and `unregister`'s bookkeeping exactly, only the (optional)
// callback invocation is absent. Registered unregister-tokens are tracked in a
// hidden `@@fr_tokens` array so `unregister` returns the correct boolean.

/// Whether `v` is an Object (a valid `register` target / unregister token) — a
/// heap value that is not one of the primitive-wrapper heap variants.
fn is_object_value(v: &Value) -> bool {
    matches!(v, Value::Obj(_))
        && with_host(|h| {
            !matches!(
                h.get(v),
                Some(JsObj::Str(_))
                    | Some(JsObj::Symbol { .. })
                    | Some(JsObj::BigInt(_))
                    | Some(JsObj::Null)
            )
        })
}

pub fn construct_finalization_registry(args: &[Value]) -> Result<Value, String> {
    let cb = args.first().cloned().unwrap_or(Value::Undef);
    if !with_host(|h| crate::host::is_callable(h, &cb)) {
        return Err(crate::host::type_error(
            "FinalizationRegistry: cleanup must be callable",
        ));
    }
    Ok(with_host(|h| {
        let tokens = h.new_array(Vec::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("FinalizationRegistry"));
        m.insert("@@fr_cb".into(), cb);
        m.insert("@@fr_tokens".into(), tokens);
        h.new_object(m)
    }))
}

pub fn finalization_registry_call(
    recv: &Value,
    method: &str,
    args: &[Value],
) -> Result<Value, String> {
    match method {
        "register" => {
            let target = args.first().cloned().unwrap_or(Value::Undef);
            let held = args.get(1).cloned().unwrap_or(Value::Undef);
            let token = args.get(2).cloned().unwrap_or(Value::Undef);
            if !is_object_value(&target) {
                // V8's wording is `invalid target`; the "must be an object"
                // phrasing was this file's own, not any engine's.
                return Err(crate::host::type_error(
                    "FinalizationRegistry.prototype.register: invalid target",
                ));
            }
            if with_host(|h| h.strict_eq(&target, &held)) {
                return Err(crate::host::type_error(
                    "FinalizationRegistry.prototype.register: target and holdings must not be same",
                ));
            }
            // A supplied unregister token must be an object; record it so a later
            // `unregister` can find (and drop) this registration.
            if !matches!(token, Value::Undef) {
                if !is_object_value(&token) {
                    return Err(crate::host::type_error(&format!(
                        "Invalid unregisterToken ('{}')",
                        with_host(|h| h.str_of(&token))
                    )));
                }
                with_host(|h| {
                    let toks = registry_tokens(h, recv);
                    if let Some(JsObj::Array(items)) = h.get_mut(&toks) {
                        items.push(token);
                    }
                });
            }
            Ok(Value::Undef)
        }
        "unregister" => {
            let token = args.first().cloned().unwrap_or(Value::Undef);
            if !is_object_value(&token) {
                // V8 names the token and does not mention the method.
                return Err(crate::host::type_error(&format!(
                    "Invalid unregisterToken ('{}')",
                    with_host(|h| h.str_of(&token))
                )));
            }
            Ok(Value::Bool(with_host(|h| {
                let toks = registry_tokens(h, recv);
                let kept: Vec<Value> = match h.get(&toks) {
                    Some(JsObj::Array(items)) => items
                        .iter()
                        .filter(|t| !h.strict_eq(t, &token))
                        .cloned()
                        .collect(),
                    _ => Vec::new(),
                };
                let removed = match h.get(&toks) {
                    Some(JsObj::Array(items)) => items.len() != kept.len(),
                    _ => false,
                };
                if let Some(JsObj::Array(items)) = h.get_mut(&toks) {
                    *items = kept;
                }
                removed
            })))
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// The hidden `@@fr_tokens` array backing a `FinalizationRegistry`.
fn registry_tokens(h: &crate::host::JsHost, recv: &Value) -> Value {
    match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@fr_tokens").cloned().unwrap_or(Value::Undef),
        _ => Value::Undef,
    }
}

// ── TextEncoder / TextDecoder ─────────────────────────────────────────────────

pub fn construct_text_encoder() -> Result<Value, String> {
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("TextEncoder"));
        m.insert("encoding".into(), h.new_str("utf-8"));
        h.new_object(m)
    }))
}

pub fn text_encoder_call(_recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        // `encode(str)` → a Uint8Array of the UTF-8 bytes.
        "encode" => {
            let s = super::arg_str(args, 0);
            Ok(make(
                "Uint8Array",
                s.as_bytes()
                    .iter()
                    .map(|b| Value::Float(*b as f64))
                    .collect(),
            ))
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

pub fn construct_text_decoder(args: &[Value]) -> Result<Value, String> {
    let label = if args.is_empty() {
        "utf-8".to_string()
    } else {
        super::arg_str(args, 0)
    };
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("TextDecoder"));
        m.insert("encoding".into(), h.new_str(label.to_ascii_lowercase()));
        h.new_object(m)
    }))
}

pub fn text_decoder_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        // `decode(bytes)` → a string from the buffer's UTF-8 (or latin1) bytes.
        "decode" => {
            let bytes: Vec<u8> = elems_of(&args.first().cloned().unwrap_or(Value::Undef))
                .unwrap_or_default()
                .iter()
                .map(|n| *n as u8)
                .collect();
            let enc = with_host(|h| match h.get(recv) {
                Some(JsObj::Object(p)) => p
                    .get("encoding")
                    .map(|v| h.str_of(v))
                    .unwrap_or_else(|| "utf-8".into()),
                _ => "utf-8".into(),
            });
            let s = match enc.as_str() {
                "latin1" | "iso-8859-1" | "ascii" => bytes.iter().map(|b| *b as char).collect(),
                _ => String::from_utf8_lossy(&bytes).into_owned(),
            };
            Ok(with_host(|h| h.new_str(s)))
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}
