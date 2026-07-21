//! Node `v8` module — a compatibility shim, NOT real V8 introspection.
//!
//! node-js has no V8: JS is lowered to `fusevm` bytecode and runs on a Rust heap
//! (see `host.rs`). There is therefore no V8 heap to measure and no V8 binary
//! serialization format to emit. This module exists so code that *calls* the `v8`
//! surface (metrics collectors, `v8.serialize`-based caches) keeps working, with
//! every deviation documented rather than faked:
//!
//!   - `getHeapStatistics` / `getHeapSpaceStatistics` / `getHeapCodeStatistics`
//!     return the correct *shape* (all the keys Node produces) with **zeroed**
//!     values. They are not read from any live allocator — reporting a fabricated
//!     non-zero heap size would be a lie, so the honest answer is 0.
//!   - `serialize` / `deserialize` round-trip through **JSON**, not V8's
//!     structured-clone binary format. The returned Buffer is UTF-8 JSON bytes and
//!     is byte-incompatible with Node's `v8.serialize`; it cannot carry cyclic
//!     graphs, `Map`/`Set`, typed arrays, `BigInt`, or `undefined` the way the
//!     real format does. Use it only for plain JSON-representable values.
//!   - `setFlagsFromString` is a no-op (there are no V8 flags to set).
//!   - `getHeapSnapshot` throws: node-js cannot produce a V8 heap snapshot.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const METHODS: &[&str] = &[
    "getHeapStatistics",
    "getHeapSpaceStatistics",
    "getHeapCodeStatistics",
    "serialize",
    "deserialize",
    "setFlagsFromString",
    "getHeapSnapshot",
    "cachedDataVersionTag",
];

/// A FIXED compatibility tag returned by `v8.cachedDataVersionTag()`. Node derives
/// this from the V8 version + build flags; node-js has no V8, so a single stable
/// constant is returned (callers use it only to invalidate a code cache when the
/// runtime changes — a constant is honest for a runtime that never emits V8 code
/// cache data in the first place).
const CACHED_DATA_VERSION_TAG: f64 = 3_527_742_766.0;

/// Methods dispatched on an `@@native = "Serializer"` object (JSON-backed shim;
/// reported to the parent for `instance_has_method` / `instance_call` wiring).
pub const SERIALIZER_METHODS: &[&str] = &["writeHeader", "writeValue", "releaseBuffer"];

/// Methods dispatched on an `@@native = "Deserializer"` object.
pub const DESERIALIZER_METHODS: &[&str] = &["readHeader", "readValue"];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "getHeapStatistics" => Ok(heap_statistics()),
        // No V8 heap spaces to enumerate.
        "getHeapSpaceStatistics" => Ok(with_host(|h| h.new_array(Vec::new()))),
        "getHeapCodeStatistics" => Ok(heap_code_statistics()),
        "serialize" => serialize(args),
        "deserialize" => deserialize(args),
        // Nothing to configure; accepted silently for compatibility.
        "setFlagsFromString" => Ok(Value::Undef),
        "getHeapSnapshot" => Err(crate::host::type_error(
            "v8.getHeapSnapshot is not supported: node-js does not run on V8",
        )),
        "cachedDataVersionTag" => Ok(Value::Float(CACHED_DATA_VERSION_TAG)),
        _ => return None,
    })
}

/// Non-function members of the `v8` namespace, exposed as constructor values so
/// `require('v8').Serializer` (etc.) resolve and `new` reaches `construct`.
/// Requires the parent to route `"v8"` into `stdlib::constant`.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "Serializer" | "Deserializer" | "DefaultSerializer" | "DefaultDeserializer" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin(name.into()))))
        }
        _ => None,
    }
}

/// `new v8.Serializer()` / `new v8.Deserializer(buffer)` (and the `Default*`
/// aliases). node-js has no V8 structured-clone binary format, so these are
/// JSON-backed shims: a `Serializer` accumulates ONE `writeValue`d value as JSON
/// and hands it back from `releaseBuffer` as a UTF-8 Buffer; a `Deserializer`
/// parses that JSON back with `readValue`. The granular byte writers/readers
/// (`writeUint32`/`writeDouble`/`writeRawBytes`/…) are NOT modeled — they only
/// make sense against the real binary layout (see the report's deferred list).
pub fn construct(name: &str, args: &[Value]) -> Result<Value, String> {
    match name {
        "Serializer" | "DefaultSerializer" => Ok(with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("@@native".into(), h.new_str("Serializer"));
            m.insert("@@json".into(), Value::Undef);
            h.new_object(m)
        })),
        "Deserializer" | "DefaultDeserializer" => {
            // Decode the incoming Buffer's bytes as UTF-8 JSON text.
            let json = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            Ok(with_host(|h| {
                let jv = h.new_str(json);
                let mut m = IndexMap::new();
                m.insert("@@native".into(), h.new_str("Deserializer"));
                m.insert("@@json".into(), jv);
                h.new_object(m)
            }))
        }
        _ => Err(crate::host::type_error(&format!(
            "v8.{name} is not a constructor"
        ))),
    }
}

/// Dispatch a method on a `Serializer`/`Deserializer` instance.
pub fn instance_call(
    tag: &str,
    recv: &Value,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    match (tag, method) {
        // Serializer: `writeHeader` is a no-op (no binary header to emit).
        ("Serializer", "writeHeader") => Ok(Value::Undef),
        ("Serializer", "writeValue") => {
            let json = crate::builtins::call_builtin_function(
                "JSON.stringify",
                vec![args.first().cloned().unwrap_or(Value::Undef)],
            )?;
            let s = with_host(|h| h.str_of(&json));
            with_host(|h| {
                let sv = h.new_str(s);
                if let Some(JsObj::Object(p)) = h.get_mut(recv) {
                    p.insert("@@json".into(), sv);
                }
            });
            Ok(Value::Bool(true))
        }
        ("Serializer", "releaseBuffer") => {
            let s = with_host(|h| match h.get(recv) {
                Some(JsObj::Object(p)) => match p.get("@@json") {
                    Some(Value::Undef) | None => String::new(),
                    Some(v) => h.str_of(v),
                },
                _ => String::new(),
            });
            Ok(super::buffer::from_bytes(s.as_bytes()))
        }
        // Deserializer: `readHeader` is a no-op; `readValue` parses the JSON back.
        ("Deserializer", "readHeader") => Ok(Value::Undef),
        ("Deserializer", "readValue") => {
            let sv = with_host(|h| match h.get(recv) {
                Some(JsObj::Object(p)) => p.get("@@json").cloned().unwrap_or(Value::Undef),
                _ => Value::Undef,
            });
            crate::builtins::call_builtin_function("JSON.parse", vec![sv])
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// `v8.getHeapStatistics()` — the full key set Node returns, all zeroed. These are
/// NOT measured from a live V8 heap (node-js has none); see the module docs.
fn heap_statistics() -> Value {
    zeros_object(&[
        "total_heap_size",
        "total_heap_size_executable",
        "total_physical_size",
        "total_available_size",
        "used_heap_size",
        "heap_size_limit",
        "malloced_memory",
        "peak_malloced_memory",
        "does_zap_garbage",
        "number_of_native_contexts",
        "number_of_detached_contexts",
        "total_global_handles_size",
        "used_global_handles_size",
        "external_memory",
    ])
}

/// `v8.getHeapCodeStatistics()` — shape-correct, zeroed (no V8 code space here).
fn heap_code_statistics() -> Value {
    zeros_object(&[
        "code_and_metadata_size",
        "bytecode_and_metadata_size",
        "external_script_source_size",
        "cpu_profiler_metadata_size",
    ])
}

/// Build an object mapping each key to `0`.
fn zeros_object(keys: &[&str]) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        for k in keys {
            m.insert((*k).to_string(), Value::Float(0.0));
        }
        h.new_object(m)
    })
}

/// `v8.serialize(value)` — JSON round-trip into a Buffer (NOT the V8 binary
/// structured-clone format; see module docs).
fn serialize(args: &[Value]) -> Result<Value, String> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    let json = crate::builtins::call_builtin_function("JSON.stringify", vec![v])?;
    let s = with_host(|h| h.str_of(&json));
    let sval = with_host(|h| h.new_str(s));
    // Encode the JSON text as a UTF-8 Buffer.
    super::buffer::static_call("from", std::slice::from_ref(&sval)).unwrap_or(Ok(Value::Undef))
}

/// `v8.deserialize(buffer)` — parse the Buffer's UTF-8 JSON back into a value
/// (the inverse of this module's `serialize`, not Node's).
fn deserialize(args: &[Value]) -> Result<Value, String> {
    // `str_of` on a native Buffer decodes its bytes as UTF-8 (see host.rs).
    let s = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
    let sval = with_host(|h| h.new_str(s));
    crate::builtins::call_builtin_function("JSON.parse", vec![sval])
}
