//! Node `zlib` module — real DEFLATE / zlib / gzip via the `flate2` crate.
//!
//! The synchronous API (`gzipSync`/`gunzipSync`, `deflateSync`/`inflateSync`,
//! `deflateRawSync`/`inflateRawSync`) is backed by `flate2` at `Compression::default()`
//! (level 6, matching node's default). Round-trips and cross-decoding with the real
//! `node` binary hold: `gunzipSync(gzipSync(x)) === x`, and node's `zlib.gunzipSync`
//! decodes our gzip output (and vice-versa). Raw/zlib-framed output is byte-identical
//! to node for the same level; gzip framing differs only in the OS byte + mtime header
//! fields, which node itself treats as informational.
//!
//! Brotli is not provided by `flate2` and no brotli crate is available here, so
//! `brotliCompressSync`/`brotliDecompressSync` return an honest "not implemented"
//! error rather than a silently-wrong result. The streaming factories
//! (`createGzip`/`createInflate`/…) likewise error until a streaming backend exists.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use flate2::Compression;
use std::io::{Read, Write};

use super::buffer;

/// `zlib` module functions routed through `stdlib::call`.
pub const MODULE_METHODS: &[&str] = &[
    // Real, backed by flate2.
    "gzipSync",
    "gunzipSync",
    "deflateSync",
    "inflateSync",
    "deflateRawSync",
    "inflateRawSync",
    // Honest errors (no brotli backend).
    "brotliCompressSync",
    "brotliDecompressSync",
    // Honest errors (no streaming backend).
    "createInflate",
    "createGunzip",
    "createBrotliDecompress",
    "createDeflate",
    "createGzip",
    "createBrotliCompress",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    // Brotli and the streaming factories are unsupported — honest error, never a fake.
    if matches!(method, "brotliCompressSync" | "brotliDecompressSync") {
        return Some(Err(crate::host::type_error(
            "zlib brotli is not supported in node-js (no brotli backend)",
        )));
    }
    if method.starts_with("create") && MODULE_METHODS.contains(&method) {
        return Some(Err(crate::host::type_error(&format!(
            "zlib.{method} is not supported in node-js (no streaming backend)"
        ))));
    }

    // Compute the full output bytes BEFORE re-entering the host to allocate the
    // result Buffer, so the two `with_host` calls (input read, output alloc) never
    // nest into a double borrow.
    let out: Result<Vec<u8>, String> = match method {
        "gzipSync" => gzip(&input_bytes(args)),
        "gunzipSync" => gunzip(&input_bytes(args)),
        "deflateSync" => deflate(&input_bytes(args)),
        "inflateSync" => inflate(&input_bytes(args)),
        "deflateRawSync" => deflate_raw(&input_bytes(args)),
        "inflateRawSync" => inflate_raw(&input_bytes(args)),
        _ => return None,
    };
    Some(out.map(|bytes| buffer::from_bytes(&bytes)))
}

/// Input bytes of `args[0]`: a Buffer's backing `@@bytes`, else the utf-8 bytes of
/// its string coercion (node accepts a Buffer, TypedArray, DataView, or string).
fn input_bytes(args: &[Value]) -> Vec<u8> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    with_host(|h| match h.get(&v) {
        Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|b| h.get(b)) {
            Some(JsObj::Array(items)) => items.iter().map(|x| h.to_number(x) as u8).collect(),
            _ => h.str_of(&v).into_bytes(),
        },
        _ => h.str_of(&v).into_bytes(),
    })
}

/// Map an I/O error (a malformed compressed stream, etc.) to a node-style message.
fn io_err(e: std::io::Error) -> String {
    format!("Error: {e}")
}

fn gzip(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(input).map_err(io_err)?;
    enc.finish().map_err(io_err)
}

fn gunzip(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    GzDecoder::new(input).read_to_end(&mut out).map_err(io_err)?;
    Ok(out)
}

fn deflate(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(input).map_err(io_err)?;
    enc.finish().map_err(io_err)
}

fn inflate(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    ZlibDecoder::new(input).read_to_end(&mut out).map_err(io_err)?;
    Ok(out)
}

fn deflate_raw(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(input).map_err(io_err)?;
    enc.finish().map_err(io_err)
}

fn inflate_raw(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    DeflateDecoder::new(input).read_to_end(&mut out).map_err(io_err)?;
    Ok(out)
}
