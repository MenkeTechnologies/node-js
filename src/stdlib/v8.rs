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

use crate::host::with_host;
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
];

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
        _ => return None,
    })
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
