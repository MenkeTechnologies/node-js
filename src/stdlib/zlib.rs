//! Node `zlib` module — real DEFLATE / zlib / gzip / brotli / zstd + CRC-32.
//!
//! One-shot compression is backed by real codecs: `flate2` for deflate/zlib/gzip
//! (`Compression::default()` == level 6, matching node's default), the `brotli`
//! crate (quality 11, lgwin 22 — node's `BROTLI_DEFAULT_QUALITY`/`BROTLI_DEFAULT_WINDOW`)
//! for brotli, and `zstd` (level 3 == `ZSTD_CLEVEL_DEFAULT`) for zstd. Round-trips
//! and cross-decoding with the real `node` binary hold for every codec, and
//! `zlib.crc32` uses `crc32fast`.
//!
//! Every one-shot function ships in both flavours: the synchronous `*Sync` form
//! returns a Buffer directly, and the asynchronous form runs the same codec and
//! invokes `callback(err, buffer)` via a queued microtask (node's zlib async work
//! is off-thread, but the codecs here are fast enough to run inline before the
//! callback fires, so results are identical). `zlib.unzip`/`unzipSync` auto-detect
//! gzip vs zlib framing by magic bytes, matching node's `Unzip`.
//!
//! The streaming transform classes (`Deflate`, `Gunzip`, `BrotliCompress`, …) and
//! their `create*` factories still require a streaming Transform backend we don't
//! have here, so those return an honest "not supported" error rather than a
//! silently-wrong result.

use crate::host::with_host;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use flate2::Compression;
use fusevm::Value;
use std::io::{Read, Write};

use super::buffer;

/// `zlib` module functions routed through `stdlib::call`.
pub const MODULE_METHODS: &[&str] = &[
    // One-shot synchronous (return a Buffer).
    "gzipSync",
    "gunzipSync",
    "deflateSync",
    "inflateSync",
    "deflateRawSync",
    "inflateRawSync",
    "unzipSync",
    "brotliCompressSync",
    "brotliDecompressSync",
    "zstdCompressSync",
    "zstdDecompressSync",
    // One-shot asynchronous (invoke callback(err, buffer)).
    "gzip",
    "gunzip",
    "deflate",
    "inflate",
    "deflateRaw",
    "inflateRaw",
    "unzip",
    "brotliCompress",
    "brotliDecompress",
    "zstdCompress",
    "zstdDecompress",
    // CRC-32 checksum (node 22+): returns a number.
    "crc32",
    // Streaming factories — honest errors (no streaming Transform backend).
    "createDeflate",
    "createInflate",
    "createGzip",
    "createGunzip",
    "createDeflateRaw",
    "createInflateRaw",
    "createUnzip",
    "createBrotliCompress",
    "createBrotliDecompress",
    "createZstdCompress",
    "createZstdDecompress",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    // Streaming factories are unsupported — honest error, never a fake.
    if method.starts_with("create") {
        return Some(Err(crate::host::type_error(&format!(
            "zlib.{method} is not supported in node-js (no streaming backend)"
        ))));
    }

    // crc32(data[, value]) -> number, not a Buffer.
    if method == "crc32" {
        let data = input_bytes(args);
        let init = {
            let n = super::arg_num(args, 1);
            if n.is_nan() {
                0
            } else {
                n as i64 as u32
            }
        };
        return Some(Ok(Value::Float(crc32(&data, init) as f64)));
    }

    // Asynchronous variants: op(buffer[, options], callback).
    if is_async(method) {
        return Some(run_async(method, args));
    }

    // Synchronous variants: `<op>Sync` -> Buffer. Compute the full output BEFORE
    // re-entering the host to allocate the result Buffer, so the two `with_host`
    // calls (input read, output alloc) never nest into a double borrow.
    let base = method.strip_suffix("Sync")?;
    let out = oneshot(base, &input_bytes(args));
    Some(out.map(|bytes| buffer::from_bytes(&bytes)))
}

/// Names that take a trailing `callback(err, buffer)`.
fn is_async(method: &str) -> bool {
    matches!(
        method,
        "gzip"
            | "gunzip"
            | "deflate"
            | "inflate"
            | "deflateRaw"
            | "inflateRaw"
            | "unzip"
            | "brotliCompress"
            | "brotliDecompress"
            | "zstdCompress"
            | "zstdDecompress"
    )
}

/// Run `op` and invoke the trailing callback with `(err, buffer)`.
fn run_async(op: &str, args: &[Value]) -> Result<Value, String> {
    let Some(cb) = args.last().cloned() else {
        return Ok(Value::Undef);
    };
    let input = input_bytes(args);
    let (err, buf) = match oneshot(op, &input) {
        Ok(bytes) => (with_host(|h| h.null()), buffer::from_bytes(&bytes)),
        // An error-first callback receives an ERROR OBJECT, not the message:
        // `zlib.gunzip(garbage, cb)` gives an `err` whose `code` is
        // `Z_DATA_ERROR`. A bare string made that — and `err.message` — all
        // `undefined`.
        // `decode_err` has already built the coded Error and parked it as the
        // pending exception for the SYNC path; take that rather than rebuilding
        // one from the message, which carries no `code`/`errno`.
        Err(e) => (
            with_host(|h| {
                h.exc
                    .take()
                    .unwrap_or_else(|| crate::builtins::synth_error(h, &e))
            }),
            Value::Undef,
        ),
    };
    with_host(|h| h.queue_micro(cb, vec![err, buf]));
    Ok(Value::Undef)
}

/// Dispatch a codec op name (no `Sync` suffix) to its byte transform.
fn oneshot(op: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    match op {
        "gzip" => gzip(input),
        "gunzip" => gunzip(input),
        "deflate" => deflate(input),
        "inflate" => inflate(input),
        "deflateRaw" => deflate_raw(input),
        "inflateRaw" => inflate_raw(input),
        "unzip" => unzip(input),
        "brotliCompress" => brotli_compress(input),
        "brotliDecompress" => brotli_decompress(input),
        "zstdCompress" => zstd_compress(input),
        "zstdDecompress" => zstd_decompress(input),
        _ => Err(format!("Error: unknown zlib op '{op}'")),
    }
}

/// Input bytes of `args[0]`: a Buffer's backing `@@bytes`, else the utf-8 bytes of
/// its string coercion (node accepts a Buffer, TypedArray, DataView, or string).
fn input_bytes(args: &[Value]) -> Vec<u8> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    // Only a Buffer's own `@@bytes` was read, so a TypedArray or DataView —
    // both of which this comment already promised to accept — was stringified
    // and compressed as the text `[object Object]`.
    match super::buffer::view_bytes(&v) {
        Some(b) => b,
        None => with_host(|h| h.str_of(&v)).into_bytes(),
    }
}

/// Map an I/O error (a malformed compressed stream, etc.) to a node-style message.
fn io_err(e: std::io::Error) -> String {
    format!("Error: {e}")
}

/// A decode failure, worded and coded as zlib does. Node carries `err.code`
/// (`Z_DATA_ERROR`) and `err.errno` (-3), which callers branch on; a plain
/// message left both `undefined` and reported Rust's wording — `corrupt
/// deflate stream` for a header zlib calls an `incorrect header check`.
///
/// The two cases are told apart by looking at the HEADER: a stream whose magic
/// is wrong never decodes, while one that passes the header and then runs out
/// is a truncation, which zlib reports as a BUF error instead.
fn decode_err(truncated: bool) -> String {
    let (code, errno, msg) = if truncated {
        ("Z_BUF_ERROR", -5, "unexpected end of file")
    } else {
        ("Z_DATA_ERROR", -3, "incorrect header check")
    };
    let e = crate::builtins::make_error_pub("Error", msg);
    for (k, v) in [
        ("errno", Value::Float(errno as f64)),
        ("code", with_host(|h| h.new_str(code.to_string()))),
    ] {
        let _ = crate::builtins::set_property_pub(&e, k, v);
    }
    with_host(|h| h.exc = Some(e));
    format!("Error: {msg}")
}

/// Whether `input` carries a well-formed header for `kind`, so a failure after
/// it is a truncation rather than garbage.
fn header_ok(kind: &str, input: &[u8]) -> bool {
    match kind {
        "gzip" => input.len() >= 2 && input[0] == 0x1f && input[1] == 0x8b,
        // RFC 1950: CM must be 8 and the two header bytes are a multiple of 31.
        "zlib" => {
            input.len() >= 2
                && input[0] & 0x0f == 8
                && (u16::from(input[0]) * 256 + u16::from(input[1])) % 31 == 0
        }
        // A raw deflate stream has no header to check.
        _ => true,
    }
}

fn gzip(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(input).map_err(io_err)?;
    enc.finish().map_err(io_err)
}

fn gunzip(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    GzDecoder::new(input)
        .read_to_end(&mut out)
        .map_err(|_| decode_err(header_ok("gzip", input)))?;
    Ok(out)
}

fn deflate(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(input).map_err(io_err)?;
    enc.finish().map_err(io_err)
}

fn inflate(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    ZlibDecoder::new(input)
        .read_to_end(&mut out)
        .map_err(|_| decode_err(header_ok("zlib", input)))?;
    Ok(out)
}

fn deflate_raw(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(input).map_err(io_err)?;
    enc.finish().map_err(io_err)
}

fn inflate_raw(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    DeflateDecoder::new(input)
        .read_to_end(&mut out)
        .map_err(io_err)?;
    Ok(out)
}

/// `unzip`: auto-detect gzip (magic `1f 8b`) vs zlib framing, like node's `Unzip`.
fn unzip(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.starts_with(&[0x1f, 0x8b]) {
        gunzip(input)
    } else {
        inflate(input)
    }
}

fn brotli_compress(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        // buffer_size 4096, quality 11, lgwin 22 (node defaults).
        let mut enc = brotli::CompressorWriter::new(&mut out, 4096, 11, 22);
        enc.write_all(input).map_err(io_err)?;
        // Drop flushes/finalizes the stream at end of scope.
    }
    Ok(out)
}

fn brotli_decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    brotli::Decompressor::new(input, 4096)
        .read_to_end(&mut out)
        .map_err(io_err)?;
    Ok(out)
}

fn zstd_compress(input: &[u8]) -> Result<Vec<u8>, String> {
    // level 3 == ZSTD_CLEVEL_DEFAULT (node's default).
    zstd::encode_all(input, 3).map_err(io_err)
}

fn zstd_decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    zstd::decode_all(input).map_err(io_err)
}

/// CRC-32 (IEEE) of `data`, seeded with `init` (node's `zlib.crc32(data, value)`).
fn crc32(data: &[u8], init: u32) -> u32 {
    let mut h = crc32fast::Hasher::new_with_initial(init);
    h.update(data);
    h.finalize()
}
