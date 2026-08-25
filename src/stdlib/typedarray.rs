//! JavaScript typed arrays (`Uint8Array`/`Int8Array`/…/`Float64Array`),
//! `ArrayBuffer`, `WeakRef`, and `TextEncoder`/`TextDecoder`.
//!
//! A typed array is a plain object tagged `@@native = "TypedArray"` carrying its
//! kind (`@@kind`), its elements as a hidden `@@elems` array of numbers, and the
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
    "toString",
    "values",
];

/// The eleven element kinds plus `ArrayBuffer` (which carries only a byte
/// length).
pub fn is_ctor(name: &str) -> bool {
    ELEMENT_KINDS.contains(&name) || name == "ArrayBuffer"
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
    // 7.1.15/7.1.16: the operand must already BE a BigInt — a Number throws,
    // which is what makes `new BigInt64Array(1)[0] = 1` a TypeError in node.
    let big = with_host(|h| match h.get(v) {
        Some(JsObj::BigInt(b)) => Some(b.clone()),
        _ => None,
    })
    .ok_or_else(|| crate::host::type_error("Cannot convert a Number value to a BigInt"))?;
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
    let field = match tag.as_str() {
        "TypedArray" => "@@elems",
        "Buffer" => "@@bytes",
        _ => return Vec::new(),
    };
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => match p.get(field).and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    })
}

/// Build a typed array of `kind` from already-coerced element values.
fn make(kind: &str, elems: Vec<Value>) -> Value {
    with_host(|h| {
        let bpe = bytes_per_element(kind);
        let len = elems.len();
        let arr = h.new_array(elems);
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("TypedArray"));
        m.insert("@@kind".into(), h.new_str(kind));
        m.insert("@@elems".into(), arr);
        m.insert("length".into(), Value::Float(len as f64));
        m.insert("byteLength".into(), Value::Float((len * bpe) as f64));
        // Every view reports where it starts in its backing store. A `Buffer`
        // already carried this; a typed array did not, so `u8.byteOffset` read
        // `undefined` where a Buffer read 0. Nothing here can produce a
        // non-zero offset yet — see the note on `.buffer` below.
        m.insert("byteOffset".into(), Value::Float(0.0));
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
        for k in ["length", "byteLength", "byteOffset", "BYTES_PER_ELEMENT"] {
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
        return Ok(with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("@@native".into(), h.new_str("ArrayBuffer"));
            m.insert("byteLength".into(), Value::Float(n as f64));
            h.new_object(m)
        }));
    }
    let elems = build_elems(kind, args)?;
    Ok(make(kind, elems))
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

/// `Uint8Array.from(iterable[, mapFn])` / `Uint8Array.of(...items)`.
pub fn static_call(kind: &str, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
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
    let field = match tag.as_str() {
        "TypedArray" => "@@elems",
        "Buffer" => "@@bytes",
        _ => return None,
    };
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => match p.get(field).and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => Some(items.iter().map(|x| h.to_number(x)).collect()),
            _ => None,
        },
        _ => None,
    })
}

/// The number of elements `v` exposes as integer-index own properties, for a
/// typed array (`@@elems`) or a `Buffer` (`@@bytes`); `None` for anything else.
///
/// Both index-membership questions — `obj.hasOwnProperty(i)` and `i in obj` —
/// must answer from this one place. They used to disagree: `hasOwnProperty`
/// carried a hand-rolled arm that understood `@@bytes` only, so it was right for
/// a Buffer and wrong for every other typed array, while the `in` operator knew
/// about neither and reported false for every valid index of both.
pub fn index_len(v: &Value) -> Option<usize> {
    let field = match super::native_tag(v)?.as_str() {
        "TypedArray" => "@@elems",
        "Buffer" => "@@bytes",
        _ => return None,
    };
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => match p.get(field).and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => Some(items.len()),
            _ => None,
        },
        _ => None,
    })
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

// ── element indexing (called from builtins::get_property/set_property) ────────

/// `ta[i]` read: the element at char/index `i`, or `None` if `i` is out of range
/// or not an integer index.
pub fn elem_get(recv: &Value, key: &str) -> Option<Value> {
    let i: usize = key.parse().ok()?;
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => match p.get("@@elems").and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.get(i).cloned(),
            _ => None,
        },
        _ => None,
    })
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
    Ok(with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get(recv) {
            if let Some(arr) = p.get("@@elems").cloned() {
                if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                    if i < items.len() {
                        items[i] = n;
                        return true;
                    }
                }
            }
        }
        false
    }))
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
/// whichever hidden array backs it — `@@elems` for a typed array, `@@bytes` for
/// a `Buffer`.
fn write_elems(recv: &Value, kind: &str, vals: &[Value]) -> Result<(), String> {
    let field = match super::native_tag(recv).as_deref() {
        Some("Buffer") => "@@bytes",
        _ => "@@elems",
    };
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
pub fn instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let kind = kind_of(recv);
    // Elements travel as `Value`, not `f64`: a 64-bit view's are BigInts, and
    // rounding them through a double is exactly the loss those views exist to
    // avoid. The numeric kinds still hold `Value::Float`, so nothing about them
    // changes.
    let elems = elem_values(recv);
    // The callback-taking methods share one shape: invoke `cb(value, index,
    // receiver)` per element. They are inherited by `Buffer` too, which is why
    // they must live here rather than in either concrete type.
    let call_cb = |i: usize, v: &Value| -> Result<Value, String> {
        crate::host::invoke(
            &args.first().cloned().unwrap_or(Value::Undef),
            vec![v.clone(), Value::Float(i as f64), recv.clone()],
            None,
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
            let cmp = args.first().cloned().unwrap_or(Value::Undef);
            if with_host(|h| crate::host::is_callable(h, &cmp)) {
                // A user comparator goes through the same fallible merge sort
                // `Array.prototype.sort` uses: O(n log n) rather than the
                // insertion sort this was, and a comparator returning NaN keeps
                // the pair's order (23.2.4.1 step 3: NaN is +0) instead of
                // swapping, which the `<= 0.0` break got wrong.
                crate::builtins::sort_values(&mut out, Some(&cmp))?;
            } else {
                // A typed array sorts NUMERICALLY by default, unlike `Array`
                // which sorts by string. Verified against node v26.7.0:
                // `new Uint8Array([10,9,1]).sort()` is `1,9,10` while
                // `[10,9,1].sort()` is `1,10,9`.
                // A BigInt element cannot be ordered through an `f64` without
                // collapsing values more than 2^53 apart, so the 64-bit views
                // compare the integers themselves.
                if is_bigint_kind(&kind) {
                    let keys: Vec<num_bigint::BigInt> = out.iter().map(bigint_of).collect();
                    let mut idx: Vec<usize> = (0..out.len()).collect();
                    idx.sort_by(|a, b| keys[*a].cmp(&keys[*b]));
                    out = idx.into_iter().map(|i| out[i].clone()).collect();
                } else {
                    out.sort_by(|a, b| {
                        num(a)
                            .partial_cmp(&num(b))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
            }
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
            Ok(Value::Float(
                elems
                    .iter()
                    .rposition(|x| same_element(x, &needle, false))
                    .map(|p| p as f64)
                    .unwrap_or(-1.0),
            ))
        }
        "keys" | "values" | "entries" => {
            let items: Vec<Value> = with_host(|h| match method {
                "keys" => (0..elems.len()).map(|i| Value::Float(i as f64)).collect(),
                "values" => elems.clone(),
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
            Ok(make(&kind, elems[s.min(e)..e.max(s)].to_vec()))
        }
        "indexOf" => {
            let needle = args.first().cloned().unwrap_or(Value::Undef);
            Ok(Value::Float(
                elems
                    .iter()
                    .position(|x| same_element(x, &needle, false))
                    .map(|p| p as f64)
                    .unwrap_or(-1.0),
            ))
        }
        "includes" => {
            let needle = args.first().cloned().unwrap_or(Value::Undef);
            Ok(Value::Bool(
                elems.iter().any(|x| same_element(x, &needle, true)),
            ))
        }
        "fill" => {
            let v = coerce_val(&kind, args.first().unwrap_or(&Value::Undef))?;
            Ok(make(&kind, vec![v; elems.len()]))
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
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get(recv) {
                    if let Some(arr) = p.get("@@elems").cloned() {
                        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                            for (k, v) in src.into_iter().enumerate() {
                                if off + k < items.len() {
                                    items[off + k] = v;
                                }
                            }
                        }
                    }
                }
            });
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
