//! Node `zlib` module — namespace only.
//!
//! The express dependency tree (`body-parser`) requires `zlib` at module load
//! and calls `createInflate`/`createGunzip`/`createBrotliDecompress` lazily, only
//! when decoding a compressed request body. node-js has no compression backend,
//! so those factories exist to satisfy the reference but throw if actually
//! invoked — an honest "unsupported", never a silently-wrong decode.

use fusevm::Value;

/// `zlib` module functions routed through `stdlib::call`.
pub const MODULE_METHODS: &[&str] = &[
    "createInflate",
    "createGunzip",
    "createBrotliDecompress",
    "createDeflate",
    "createGzip",
    "createBrotliCompress",
];

pub fn call(method: &str, _args: &[Value]) -> Option<Result<Value, String>> {
    if MODULE_METHODS.contains(&method) {
        return Some(Err(crate::host::type_error(&format!(
            "zlib.{method} is not supported in node-js (no compression backend)"
        ))));
    }
    None
}
