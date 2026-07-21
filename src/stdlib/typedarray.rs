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

pub const STATIC_METHODS: &[&str] = &["from", "of"];

/// The nine element kinds plus `ArrayBuffer` (which carries only a byte length).
pub fn is_ctor(name: &str) -> bool {
    matches!(
        name,
        "Uint8Array"
            | "Int8Array"
            | "Uint8ClampedArray"
            | "Int16Array"
            | "Uint16Array"
            | "Int32Array"
            | "Uint32Array"
            | "Float32Array"
            | "Float64Array"
            | "ArrayBuffer"
    )
}

/// Bytes per element for a typed-array kind.
fn bytes_per_element(kind: &str) -> usize {
    match kind {
        "Int8Array" | "Uint8Array" | "Uint8ClampedArray" => 1,
        "Int16Array" | "Uint16Array" => 2,
        "Int32Array" | "Uint32Array" | "Float32Array" => 4,
        "Float64Array" => 8,
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

/// Build a typed array of `kind` from already-coerced element values.
fn make(kind: &str, elems: Vec<f64>) -> Value {
    with_host(|h| {
        let bpe = bytes_per_element(kind);
        let len = elems.len();
        let arr = h.new_array(elems.into_iter().map(Value::Float).collect());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("TypedArray"));
        m.insert("@@kind".into(), h.new_str(kind));
        m.insert("@@elems".into(), arr);
        m.insert("length".into(), Value::Float(len as f64));
        m.insert("byteLength".into(), Value::Float((len * bpe) as f64));
        m.insert("BYTES_PER_ELEMENT".into(), Value::Float(bpe as f64));
        h.new_object(m)
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
fn build_elems(kind: &str, args: &[Value]) -> Result<Vec<f64>, String> {
    match args.first() {
        None | Some(Value::Undef) => Ok(Vec::new()),
        Some(Value::Int(_)) | Some(Value::Float(_)) => {
            let n = super::arg_num(args, 0).max(0.0) as usize;
            Ok(vec![0.0; n])
        }
        Some(v) => {
            // Another typed array / Buffer → copy its elements.
            if let Some(src) = elems_of(v) {
                return Ok(src.iter().map(|x| coerce(kind, *x)).collect());
            }
            // A plain array or arraylike → coerce each entry.
            let items = crate::host::iter_all(v).unwrap_or_default();
            Ok(items
                .iter()
                .map(|x| coerce(kind, with_host(|h| h.to_number(x))))
                .collect())
        }
    }
}

/// `Uint8Array.from(iterable[, mapFn])` / `Uint8Array.of(...items)`.
pub fn static_call(kind: &str, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "of" => Ok(make(
            kind,
            args.iter()
                .map(|x| coerce(kind, with_host(|h| h.to_number(x))))
                .collect(),
        )),
        "from" => from(kind, args),
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
        out.push(coerce(kind, with_host(|h| h.to_number(&mapped))));
    }
    Ok(make(kind, out))
}

/// The element values of a typed array / Buffer (`None` for anything else).
fn elems_of(v: &Value) -> Option<Vec<f64>> {
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

/// The `@@kind` of a typed-array receiver (defaults to `Uint8Array`).
fn kind_of(recv: &Value) -> String {
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
pub fn elem_set(recv: &Value, key: &str, val: &Value) -> bool {
    let Ok(i) = key.parse::<usize>() else {
        return false;
    };
    let kind = kind_of(recv);
    let n = coerce(&kind, with_host(|h| h.to_number(val)));
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get(recv) {
            if let Some(arr) = p.get("@@elems").cloned() {
                if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                    if i < items.len() {
                        items[i] = Value::Float(n);
                        return true;
                    }
                }
            }
        }
        false
    })
}

/// Typed-array instance methods.
pub fn instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let kind = kind_of(recv);
    let elems = elems_of(recv).unwrap_or_default();
    match method {
        "toString" | "join" => {
            let sep = if method == "join" && !args.is_empty() {
                super::arg_str(args, 0)
            } else {
                ",".into()
            };
            let parts: Vec<String> =
                with_host(|h| elems.iter().map(|n| h.str_of(&Value::Float(*n))).collect());
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
            let needle = super::arg_num(args, 0);
            Ok(Value::Float(
                elems
                    .iter()
                    .position(|x| *x == needle)
                    .map(|p| p as f64)
                    .unwrap_or(-1.0),
            ))
        }
        "includes" => {
            let needle = super::arg_num(args, 0);
            Ok(Value::Bool(elems.contains(&needle)))
        }
        "fill" => {
            let v = coerce(&kind, super::arg_num(args, 0));
            Ok(make(&kind, vec![v; elems.len()]))
        }
        "set" => {
            // `ta.set(src[, offset])` — write `src`'s values in place.
            let src = elems_of(&args.first().cloned().unwrap_or(Value::Undef))
                .or_else(|| {
                    Some(
                        crate::host::iter_all(&args.first().cloned().unwrap_or(Value::Undef))
                            .ok()?
                            .iter()
                            .map(|x| with_host(|h| h.to_number(x)))
                            .collect(),
                    )
                })
                .unwrap_or_default();
            let off = super::arg_num(args, 1).max(0.0) as usize;
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get(recv) {
                    if let Some(arr) = p.get("@@elems").cloned() {
                        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                            for (k, v) in src.iter().enumerate() {
                                if off + k < items.len() {
                                    items[off + k] = Value::Float(coerce(&kind, *v));
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
                s.as_bytes().iter().map(|b| *b as f64).collect(),
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
