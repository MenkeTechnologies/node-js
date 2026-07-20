//! Node `crypto` module.
//!
//! * `createHash(algo)` → a `Hash` instance (`update(...).digest(enc)`), backed
//!   by the `md-5`/`sha1`/`sha2` crates.
//! * `createHmac(algo, key)` → an `Hmac` instance (`update(...).digest(enc)`),
//!   backed by the `hmac` crate over the same digests.
//! * `randomBytes`, `randomUUID`, `randomInt` — CSPRNG output via `getrandom`.
//!
//! `Hash`/`Hmac` are plain objects tagged `@@native = "Hash"` / `"Hmac"`
//! accumulating input in a hidden `@@data` byte array until `digest` finalizes.

use super::{arg_str, to_base64, to_hex};
use crate::host::{is_callable, with_host, JsObj};
use cipher::block_padding::Pkcs7;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, StreamCipher};
use fusevm::Value;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use indexmap::IndexMap;
use md5::{Digest as _, Md5};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use subtle::ConstantTimeEq;

/// Cipher algorithms `createCipheriv`/`createDecipheriv` support (AES CBC/CTR).
const CIPHERS: &[&str] = &[
    "aes-128-cbc",
    "aes-192-cbc",
    "aes-256-cbc",
    "aes-128-ctr",
    "aes-192-ctr",
    "aes-256-ctr",
];

/// Digest names `createHash`/`createHmac`/`pbkdf2`/`hkdf` accept.
const HASHES: &[&str] = &["md5", "sha1", "sha256", "sha512"];

/// Standard EC curve names for `getCurves()`. Key generation over these is not
/// supported (no EC crate available); the list mirrors the common OpenSSL names
/// so feature-detection code sees them.
const CURVES: &[&str] = &[
    "prime256v1",
    "secp256k1",
    "secp384r1",
    "secp521r1",
    "secp224r1",
    "secp192k1",
    "secp256r1",
];

pub const METHODS: &[&str] = &[
    "createHash",
    "createHmac",
    "randomBytes",
    "randomUUID",
    "randomInt",
    "pbkdf2Sync",
    "pbkdf2",
    "scryptSync",
    "scrypt",
    "hkdfSync",
    "hkdf",
    "createCipheriv",
    "createDecipheriv",
    "randomFillSync",
    "randomFill",
    "timingSafeEqual",
    "getHashes",
    "getCiphers",
    "getCurves",
    "getFips",
    "pseudoRandomBytes",
    "prng",
    "rng",
    "hash",
    "randomUUIDv7",
    "getCipherInfo",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "createHash" => {
            let algo = arg_str(args, 0).to_ascii_lowercase();
            if !supported(&algo) {
                return Some(Err(format!("Error: Digest method not supported: {algo}")));
            }
            Ok(with_host(|h| {
                let data = h.new_array(Vec::new());
                let mut m = IndexMap::new();
                m.insert("@@native".into(), h.new_str("Hash"));
                m.insert("@@algo".into(), h.new_str(algo));
                m.insert("@@data".into(), data);
                h.new_object(m)
            }))
        }
        "createHmac" => {
            let algo = arg_str(args, 0).to_ascii_lowercase();
            if !supported(&algo) {
                return Some(Err(format!("Error: Digest method not supported: {algo}")));
            }
            // Key may be a Buffer (raw bytes) or a string (utf8 by default).
            let key = key_bytes(args.get(1));
            Ok(with_host(|h| {
                let data = h.new_array(Vec::new());
                let keyv = h.new_array(key.iter().map(|b| Value::Float(*b as f64)).collect());
                let mut m = IndexMap::new();
                m.insert("@@native".into(), h.new_str("Hmac"));
                m.insert("@@algo".into(), h.new_str(algo));
                m.insert("@@key".into(), keyv);
                m.insert("@@data".into(), data);
                h.new_object(m)
            }))
        }
        "randomBytes" => {
            let n = super::arg_num(args, 0).max(0.0) as usize;
            let mut buf = vec![0u8; n];
            if let Err(e) = getrandom::getrandom(&mut buf) {
                return Some(Err(format!("Error: failed to generate random bytes: {e}")));
            }
            // Callback form: randomBytes(n, (err, buf) => ...). Build the Buffer,
            // release the host borrow, then queue the callback with (null, buf).
            let cb = args.get(1).cloned().filter(|v| with_host(|h| is_callable(h, v)));
            if let Some(cb) = cb {
                let bufv = super::buffer::from_bytes(&buf);
                with_host(|h| {
                    let nullv = h.null();
                    h.queue_micro(cb, vec![nullv, bufv]);
                });
                Ok(Value::Undef)
            } else {
                Ok(super::buffer::from_bytes(&buf))
            }
        }
        "randomUUID" => {
            let mut b = [0u8; 16];
            if let Err(e) = getrandom::getrandom(&mut b) {
                return Some(Err(format!("Error: failed to generate random bytes: {e}")));
            }
            // RFC 4122 v4: version nibble = 4, variant nibble ∈ [8..b].
            b[6] = (b[6] & 0x0f) | 0x40;
            b[8] = (b[8] & 0x3f) | 0x80;
            let h = to_hex(&b);
            let uuid = format!(
                "{}-{}-{}-{}-{}",
                &h[0..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..32],
            );
            Ok(with_host(|host| host.new_str(uuid)))
        }
        "randomInt" => {
            // randomInt([min, ]max) — uniform integer in [min, max).
            let (min, max) = if args.len() >= 2 {
                (super::arg_num(args, 0), super::arg_num(args, 1))
            } else {
                (0.0, super::arg_num(args, 0))
            };
            let (min, max) = (min as i64, max as i64);
            if max <= min {
                return Some(Err("Error: The value of \"max\" is out of range. It must be greater than the value of \"min\".".into()));
            }
            let range = (max - min) as u64;
            match random_below(range) {
                Ok(r) => Ok(Value::Float((min + r as i64) as f64)),
                Err(e) => Err(format!("Error: failed to generate random bytes: {e}")),
            }
        }
        // ── Key derivation ──────────────────────────────────────────────
        "pbkdf2Sync" => {
            let digest = arg_str(args, 4).to_ascii_lowercase();
            match pbkdf2_derive(&digest, &val_bytes_at(args, 0), &val_bytes_at(args, 1), super::arg_num(args, 2) as u32, super::arg_num(args, 3).max(0.0) as usize) {
                Ok(out) => Ok(super::buffer::from_bytes(&out)),
                Err(e) => Err(e),
            }
        }
        "pbkdf2" => {
            let digest = arg_str(args, 4).to_ascii_lowercase();
            let res = pbkdf2_derive(&digest, &val_bytes_at(args, 0), &val_bytes_at(args, 1), super::arg_num(args, 2) as u32, super::arg_num(args, 3).max(0.0) as usize);
            deliver_async(args.get(5).cloned(), res)
        }
        "scryptSync" => {
            let keylen = super::arg_num(args, 2).max(0.0) as usize;
            match scrypt_derive(&val_bytes_at(args, 0), &val_bytes_at(args, 1), keylen, opts_object(args, 3)) {
                Ok(out) => Ok(super::buffer::from_bytes(&out)),
                Err(e) => Err(e),
            }
        }
        "scrypt" => {
            let keylen = super::arg_num(args, 2).max(0.0) as usize;
            let res = scrypt_derive(&val_bytes_at(args, 0), &val_bytes_at(args, 1), keylen, opts_object(args, 3));
            deliver_async(trailing_cb(args), res)
        }
        "hkdfSync" => {
            let digest = arg_str(args, 0).to_ascii_lowercase();
            match hkdf_derive(&digest, &val_bytes_at(args, 1), &val_bytes_at(args, 2), &val_bytes_at(args, 3), super::arg_num(args, 4).max(0.0) as usize) {
                Ok(out) => Ok(super::buffer::from_bytes(&out)),
                Err(e) => Err(e),
            }
        }
        "hkdf" => {
            let digest = arg_str(args, 0).to_ascii_lowercase();
            let res = hkdf_derive(&digest, &val_bytes_at(args, 1), &val_bytes_at(args, 2), &val_bytes_at(args, 3), super::arg_num(args, 4).max(0.0) as usize);
            deliver_async(args.get(5).cloned(), res)
        }
        // ── Symmetric ciphers ───────────────────────────────────────────
        "createCipheriv" => make_cipher("Cipheriv", args),
        "createDecipheriv" => make_cipher("Decipheriv", args),
        // ── Random fill ─────────────────────────────────────────────────
        "randomFillSync" => random_fill(args),
        "randomFill" => {
            let cb = trailing_cb(args);
            let res = random_fill(args);
            match (cb, res) {
                (Some(cb), Ok(buf)) => {
                    with_host(|h| {
                        let nullv = h.null();
                        h.queue_micro(cb, vec![nullv, buf]);
                    });
                    Ok(Value::Undef)
                }
                (Some(cb), Err(e)) => {
                    let errv = with_host(|h| h.new_str(e));
                    with_host(|h| h.queue_micro(cb, vec![errv]));
                    Ok(Value::Undef)
                }
                (None, r) => r,
            }
        }
        // ── Constant-time compare ───────────────────────────────────────
        "timingSafeEqual" => {
            let a = val_bytes_at(args, 0);
            let b = val_bytes_at(args, 1);
            if a.len() != b.len() {
                return Some(Err("Error: Input buffers must have the same byte length".into()));
            }
            Ok(Value::Bool(a.ct_eq(&b).into()))
        }
        // ── Introspection ───────────────────────────────────────────────
        "getHashes" => Ok(with_host(|h| {
            let items: Vec<Value> = HASHES.iter().map(|s| h.new_str(*s)).collect();
            h.new_array(items)
        })),
        "getCiphers" => Ok(with_host(|h| {
            let items: Vec<Value> = CIPHERS.iter().map(|s| h.new_str(*s)).collect();
            h.new_array(items)
        })),
        "getCurves" => Ok(with_host(|h| {
            let items: Vec<Value> = CURVES.iter().map(|s| h.new_str(*s)).collect();
            h.new_array(items)
        })),
        // node v26 returns the number 0 (not the boolean false) from getFips().
        "getFips" => Ok(Value::Float(0.0)),
        "getCipherInfo" => cipher_info(&arg_str(args, 0).to_ascii_lowercase()),
        // ── randomBytes aliases (all return a Buffer of CSPRNG bytes) ────
        "pseudoRandomBytes" | "prng" | "rng" => {
            let n = super::arg_num(args, 0).max(0.0) as usize;
            let mut buf = vec![0u8; n];
            if let Err(e) = getrandom::getrandom(&mut buf) {
                return Some(Err(format!("Error: failed to generate random bytes: {e}")));
            }
            let cb = args.get(1).cloned().filter(|v| with_host(|h| is_callable(h, v)));
            if let Some(cb) = cb {
                let bufv = super::buffer::from_bytes(&buf);
                with_host(|h| {
                    let nullv = h.null();
                    h.queue_micro(cb, vec![nullv, bufv]);
                });
                Ok(Value::Undef)
            } else {
                Ok(super::buffer::from_bytes(&buf))
            }
        }
        // ── One-shot hash (node 21+) ────────────────────────────────────
        "hash" => {
            let algo = arg_str(args, 0).to_ascii_lowercase();
            if !supported(&algo) {
                return Some(Err(format!("Error: Digest method not supported: {algo}")));
            }
            // args[1] is the data (utf8 string or Buffer); there is no input-encoding param.
            let data = val_bytes_at(args, 1);
            // args[2] is the output encoding (default "hex"); "buffer" yields a Buffer.
            let out = digest(&algo, &data);
            let out_enc = if args.len() > 2 { arg_str(args, 2) } else { "hex".into() };
            Ok(if out_enc == "buffer" {
                super::buffer::from_bytes(&out)
            } else {
                encode_out(&out, Some(&out_enc))
            })
        }
        // ── time-ordered UUID v7 ────────────────────────────────────────
        "randomUUIDv7" => uuid_v7(),
        _ => return None,
    })
}

/// `Hash` instance methods: `update` (chainable) and `digest`.
pub fn instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    hashlike_call("Hash", recv, method, args)
}

/// `Hmac` instance methods: `update` (chainable) and `digest`.
pub fn hmac_instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    hashlike_call("Hmac", recv, method, args)
}

/// `Cipheriv`/`Decipheriv` instance methods: `update(data[,inEnc,outEnc])` and
/// `final([outEnc])`. Input is accumulated in `@@data`; the transform runs at
/// `final` (CBC needs the full block/padding stream, so `update` returns empty
/// and `final` returns the whole result — the standard `update()+final()`
/// concatenation is byte-identical to node).
pub fn cipher_instance_call(tag: &str, recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "update" => {
            let bytes = cipher_input_bytes(args);
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get(recv).cloned() {
                    if let Some(arr) = p.get("@@data").cloned() {
                        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                            items.extend(bytes.iter().map(|b| Value::Float(*b as f64)));
                        }
                    }
                }
            });
            let out_enc = if args.len() > 2 { Some(arg_str(args, 2)) } else { None };
            Ok(encode_out(&[], out_enc.as_deref()))
        }
        "final" => {
            let (algo, key, iv, data) = with_host(|h| {
                let (mut algo, mut key, mut iv, mut data) = (String::new(), Vec::new(), Vec::new(), Vec::new());
                if let Some(JsObj::Object(p)) = h.get(recv) {
                    algo = p.get("@@algo").map(|v| h.str_of(v)).unwrap_or_default();
                    if let Some(JsObj::Array(it)) = p.get("@@key").and_then(|v| h.get(v)) {
                        key = it.iter().map(|v| h.to_number(v) as u8).collect();
                    }
                    if let Some(JsObj::Array(it)) = p.get("@@iv").and_then(|v| h.get(v)) {
                        iv = it.iter().map(|v| h.to_number(v) as u8).collect();
                    }
                    if let Some(JsObj::Array(it)) = p.get("@@data").and_then(|v| h.get(v)) {
                        data = it.iter().map(|v| h.to_number(v) as u8).collect();
                    }
                }
                (algo, key, iv, data)
            });
            let out = cipher_crypt(&algo, &key, &iv, &data, tag == "Cipheriv")?;
            let out_enc = if args.is_empty() { None } else { Some(arg_str(args, 0)) };
            Ok(encode_out(&out, out_enc.as_deref()))
        }
        "setAutoPadding" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!("{}.{method} is not a function", tag.to_ascii_lowercase()))),
    }
}

/// Shared `update`/`digest` for `Hash` and `Hmac` (both accumulate into `@@data`;
/// `digest` finalizes via a plain digest or an HMAC keyed by `@@key`).
fn hashlike_call(kind: &str, recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "update" => {
            let enc = if args.len() > 1 { arg_str(args, 1) } else { "utf8".into() };
            let bytes = decode(&arg_str(args, 0), &enc);
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get(recv).cloned() {
                    if let Some(arr) = p.get("@@data").cloned() {
                        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                            items.extend(bytes.iter().map(|b| Value::Float(*b as f64)));
                        }
                    }
                }
            });
            Ok(recv.clone())
        }
        "digest" => {
            let (algo, key, data) = with_host(|h| {
                let (mut algo, mut key, mut data) = (String::new(), Vec::new(), Vec::new());
                if let Some(JsObj::Object(p)) = h.get(recv) {
                    algo = p.get("@@algo").map(|v| h.str_of(v)).unwrap_or_default();
                    if let Some(JsObj::Array(items)) = p.get("@@data").and_then(|v| h.get(v)) {
                        data = items.iter().map(|v| h.to_number(v) as u8).collect();
                    }
                    if let Some(JsObj::Array(items)) = p.get("@@key").and_then(|v| h.get(v)) {
                        key = items.iter().map(|v| h.to_number(v) as u8).collect();
                    }
                }
                (algo, key, data)
            });
            let out = if kind == "Hmac" {
                hmac_digest(&algo, &key, &data)
            } else {
                digest(&algo, &data)
            };
            let enc = if args.is_empty() { None } else { Some(arg_str(args, 0)) };
            Ok(match enc.as_deref() {
                Some("hex") => with_host(|h| h.new_str(to_hex(&out))),
                Some("base64") | Some("base64url") => with_host(|h| h.new_str(to_base64(&out))),
                Some("latin1") | Some("binary") => with_host(|h| h.new_str(out.iter().map(|b| *b as char).collect::<String>())),
                _ => super::buffer::from_bytes(&out),
            })
        }
        _ => Err(crate::host::type_error(&format!("{}.{method} is not a function", kind.to_ascii_lowercase()))),
    }
}

fn supported(algo: &str) -> bool {
    matches!(algo, "md5" | "sha1" | "sha256" | "sha512")
}

fn digest(algo: &str, data: &[u8]) -> Vec<u8> {
    match algo {
        "md5" => {
            let mut h = Md5::new();
            h.update(data);
            h.finalize().to_vec()
        }
        "sha1" => {
            let mut h = Sha1::new();
            h.update(data);
            h.finalize().to_vec()
        }
        "sha512" => {
            let mut h = Sha512::new();
            h.update(data);
            h.finalize().to_vec()
        }
        _ => {
            let mut h = Sha256::new();
            h.update(data);
            h.finalize().to_vec()
        }
    }
}

fn hmac_digest(algo: &str, key: &[u8], data: &[u8]) -> Vec<u8> {
    // HMAC accepts any key length, so `new_from_slice` never fails here.
    match algo {
        "md5" => {
            let mut m = Hmac::<Md5>::new_from_slice(key).expect("HMAC accepts any key length");
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
        "sha1" => {
            let mut m = Hmac::<Sha1>::new_from_slice(key).expect("HMAC accepts any key length");
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
        "sha512" => {
            let mut m = Hmac::<Sha512>::new_from_slice(key).expect("HMAC accepts any key length");
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
        _ => {
            let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
    }
}

/// The `createHmac` key argument as raw bytes: a Buffer's bytes, else the value's
/// utf8 string encoding.
fn key_bytes(v: Option<&Value>) -> Vec<u8> {
    v.map(val_bytes).unwrap_or_default()
}

/// A value's raw bytes: a Buffer/TypedArray's backing bytes, else its utf8
/// string encoding. Used by pbkdf2/scrypt/hkdf/cipher/timingSafeEqual inputs.
fn val_bytes(v: &Value) -> Vec<u8> {
    if super::native_tag(v).as_deref() == Some("Buffer") {
        return with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|b| h.get(b)) {
                Some(JsObj::Array(items)) => items.iter().map(|x| h.to_number(x) as u8).collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        });
    }
    with_host(|h| h.str_of(v)).into_bytes()
}

/// `val_bytes` for the arg at index `i` (`Value::Undef` → empty).
fn val_bytes_at(args: &[Value], i: usize) -> Vec<u8> {
    args.get(i).map(val_bytes).unwrap_or_default()
}

/// The trailing argument if it is a callback (async form detection).
fn trailing_cb(args: &[Value]) -> Option<Value> {
    args.last().cloned().filter(|v| with_host(|h| is_callable(h, v)))
}

/// The arg at index `i` if it is a plain (non-callable) options object.
fn opts_object(args: &[Value], i: usize) -> Option<Value> {
    let v = args.get(i)?.clone();
    let is_obj = with_host(|h| matches!(h.get(&v), Some(JsObj::Object(_))) && !is_callable(h, &v));
    is_obj.then_some(v)
}

/// Queue a derived-key result to an async callback as `(null, buf)` / `(err)`;
/// if there is no callback, return the value/error synchronously.
fn deliver_async(cb: Option<Value>, res: Result<Vec<u8>, String>) -> Result<Value, String> {
    match (cb.filter(|v| with_host(|h| is_callable(h, v))), res) {
        (Some(cb), Ok(out)) => {
            let bufv = super::buffer::from_bytes(&out);
            with_host(|h| {
                let nullv = h.null();
                h.queue_micro(cb, vec![nullv, bufv]);
            });
            Ok(Value::Undef)
        }
        (Some(cb), Err(e)) => {
            let errv = with_host(|h| h.new_str(e));
            with_host(|h| h.queue_micro(cb, vec![errv]));
            Ok(Value::Undef)
        }
        (None, Ok(out)) => Ok(super::buffer::from_bytes(&out)),
        (None, Err(e)) => Err(e),
    }
}

/// PBKDF2-HMAC derivation over a supported digest.
fn pbkdf2_derive(digest: &str, pass: &[u8], salt: &[u8], iters: u32, keylen: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; keylen];
    match digest {
        "sha1" => pbkdf2::pbkdf2_hmac::<Sha1>(pass, salt, iters, &mut out),
        "sha256" => pbkdf2::pbkdf2_hmac::<Sha256>(pass, salt, iters, &mut out),
        "sha512" => pbkdf2::pbkdf2_hmac::<Sha512>(pass, salt, iters, &mut out),
        "md5" => pbkdf2::pbkdf2_hmac::<Md5>(pass, salt, iters, &mut out),
        _ => return Err(format!("Error: Invalid digest: {digest}")),
    }
    Ok(out)
}

/// scrypt derivation. Reads node's `N`/`cost`, `r`/`blockSize`, `p`/
/// `parallelization` options (defaults 16384/8/1).
fn scrypt_derive(pass: &[u8], salt: &[u8], keylen: usize, opts: Option<Value>) -> Result<Vec<u8>, String> {
    let (mut n, mut r, mut p) = (16384.0f64, 8.0f64, 1.0f64);
    if let Some(o) = opts {
        n = opt_num(&o, &["N", "cost"], n);
        r = opt_num(&o, &["r", "blockSize"], r);
        p = opt_num(&o, &["p", "parallelization"], p);
    }
    let n = n as u64;
    if n < 2 || (n & (n - 1)) != 0 {
        return Err("Error: Invalid scrypt param: N must be a power of two > 1".into());
    }
    let params = scrypt::Params::new(n.trailing_zeros() as u8, r as u32, p as u32, keylen)
        .map_err(|e| format!("Error: {e}"))?;
    let mut out = vec![0u8; keylen];
    scrypt::scrypt(pass, salt, &params, &mut out).map_err(|e| format!("Error: {e}"))?;
    Ok(out)
}

/// HKDF (extract + expand) over a supported digest.
fn hkdf_derive(digest: &str, ikm: &[u8], salt: &[u8], info: &[u8], keylen: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; keylen];
    let ok = match digest {
        "sha1" => Hkdf::<Sha1>::new(Some(salt), ikm).expand(info, &mut out),
        "sha256" => Hkdf::<Sha256>::new(Some(salt), ikm).expand(info, &mut out),
        "sha512" => Hkdf::<Sha512>::new(Some(salt), ikm).expand(info, &mut out),
        "md5" => Hkdf::<Md5>::new(Some(salt), ikm).expand(info, &mut out),
        _ => return Err(format!("Error: Invalid digest: {digest}")),
    };
    ok.map_err(|_| "Error: Invalid key length".to_string())?;
    Ok(out)
}

/// Read the first present, finite numeric property among `keys` from an options
/// object, else `default`.
fn opt_num(obj: &Value, keys: &[&str], default: f64) -> f64 {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get(obj) {
            for k in keys {
                if let Some(v) = p.get(*k) {
                    let n = h.to_number(v);
                    if !n.is_nan() {
                        return n;
                    }
                }
            }
        }
        default
    })
}

/// Build a `Cipheriv`/`Decipheriv` instance object from `(algo, key, iv)`.
fn make_cipher(tag: &str, args: &[Value]) -> Result<Value, String> {
    let algo = arg_str(args, 0).to_ascii_lowercase();
    if !CIPHERS.contains(&algo.as_str()) {
        return Err(format!("Error: Unknown cipher: {algo}"));
    }
    let key = val_bytes_at(args, 1);
    let iv = val_bytes_at(args, 2);
    let want_key = key_len(&algo);
    if key.len() != want_key {
        return Err("Error: Invalid key length".into());
    }
    if iv.len() != 16 {
        return Err("Error: Invalid initialization vector".into());
    }
    Ok(with_host(|h| {
        let keyv = h.new_array(key.iter().map(|b| Value::Float(*b as f64)).collect());
        let ivv = h.new_array(iv.iter().map(|b| Value::Float(*b as f64)).collect());
        let data = h.new_array(Vec::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        m.insert("@@algo".into(), h.new_str(algo));
        m.insert("@@key".into(), keyv);
        m.insert("@@iv".into(), ivv);
        m.insert("@@data".into(), data);
        h.new_object(m)
    }))
}

/// Cipher `update` input: a Buffer's bytes, else the string decoded by the input
/// encoding (arg 1, default utf8).
fn cipher_input_bytes(args: &[Value]) -> Vec<u8> {
    if super::native_tag(args.first().unwrap_or(&Value::Undef)).as_deref() == Some("Buffer") {
        return val_bytes_at(args, 0);
    }
    let enc = if args.len() > 1 { arg_str(args, 1) } else { "utf8".into() };
    decode(&arg_str(args, 0), &enc)
}

/// AES-CBC (Pkcs7) / AES-CTR transform. `encrypt` selects direction (CTR is
/// symmetric so the flag is unused there).
fn cipher_crypt(algo: &str, key: &[u8], iv: &[u8], data: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    const KEYERR: &str = "Error: Invalid key length";
    const DECERR: &str = "Error: error:1C800064:Provider routines::bad decrypt";
    match (algo, encrypt) {
        ("aes-128-cbc", true) => Ok(cbc::Encryptor::<aes::Aes128>::new_from_slices(key, iv).map_err(|_| KEYERR.to_string())?.encrypt_padded_vec_mut::<Pkcs7>(data)),
        ("aes-192-cbc", true) => Ok(cbc::Encryptor::<aes::Aes192>::new_from_slices(key, iv).map_err(|_| KEYERR.to_string())?.encrypt_padded_vec_mut::<Pkcs7>(data)),
        ("aes-256-cbc", true) => Ok(cbc::Encryptor::<aes::Aes256>::new_from_slices(key, iv).map_err(|_| KEYERR.to_string())?.encrypt_padded_vec_mut::<Pkcs7>(data)),
        ("aes-128-cbc", false) => cbc::Decryptor::<aes::Aes128>::new_from_slices(key, iv).map_err(|_| KEYERR.to_string())?.decrypt_padded_vec_mut::<Pkcs7>(data).map_err(|_| DECERR.to_string()),
        ("aes-192-cbc", false) => cbc::Decryptor::<aes::Aes192>::new_from_slices(key, iv).map_err(|_| KEYERR.to_string())?.decrypt_padded_vec_mut::<Pkcs7>(data).map_err(|_| DECERR.to_string()),
        ("aes-256-cbc", false) => cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv).map_err(|_| KEYERR.to_string())?.decrypt_padded_vec_mut::<Pkcs7>(data).map_err(|_| DECERR.to_string()),
        ("aes-128-ctr", _) => {
            let mut buf = data.to_vec();
            ctr::Ctr128BE::<aes::Aes128>::new_from_slices(key, iv).map_err(|_| KEYERR.to_string())?.apply_keystream(&mut buf);
            Ok(buf)
        }
        ("aes-192-ctr", _) => {
            let mut buf = data.to_vec();
            ctr::Ctr128BE::<aes::Aes192>::new_from_slices(key, iv).map_err(|_| KEYERR.to_string())?.apply_keystream(&mut buf);
            Ok(buf)
        }
        ("aes-256-ctr", _) => {
            let mut buf = data.to_vec();
            ctr::Ctr128BE::<aes::Aes256>::new_from_slices(key, iv).map_err(|_| KEYERR.to_string())?.apply_keystream(&mut buf);
            Ok(buf)
        }
        _ => Err(format!("Error: Unsupported cipher: {algo}")),
    }
}

/// Required key length in bytes for a supported cipher name.
fn key_len(algo: &str) -> usize {
    if algo.starts_with("aes-128") {
        16
    } else if algo.starts_with("aes-192") {
        24
    } else {
        32
    }
}

/// Encode bytes for a cipher `update`/`final` (or one-shot `hash`) output.
fn encode_out(bytes: &[u8], enc: Option<&str>) -> Value {
    match enc {
        Some("hex") => with_host(|h| h.new_str(to_hex(bytes))),
        Some("base64") | Some("base64url") => with_host(|h| h.new_str(to_base64(bytes))),
        Some("latin1") | Some("binary") => with_host(|h| h.new_str(bytes.iter().map(|b| *b as char).collect::<String>())),
        Some("utf8") | Some("utf-8") => with_host(|h| h.new_str(String::from_utf8_lossy(bytes).into_owned())),
        _ => super::buffer::from_bytes(bytes),
    }
}

/// `getCipherInfo(name)` → `{ name, nid, blockSize, ivLength, mode, keyLength }`.
fn cipher_info(algo: &str) -> Result<Value, String> {
    if !CIPHERS.contains(&algo) {
        return Ok(Value::Undef);
    }
    let nid = match algo {
        "aes-128-cbc" => 419,
        "aes-192-cbc" => 423,
        "aes-256-cbc" => 427,
        "aes-128-ctr" => 904,
        "aes-192-ctr" => 905,
        _ => 906,
    };
    let (mode, block) = if algo.ends_with("ctr") { ("ctr", 1) } else { ("cbc", 16) };
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("mode".into(), h.new_str(mode));
        m.insert("name".into(), h.new_str(algo));
        m.insert("nid".into(), Value::Float(nid as f64));
        m.insert("keyLength".into(), Value::Float(key_len(algo) as f64));
        m.insert("blockSize".into(), Value::Float(block as f64));
        m.insert("ivLength".into(), Value::Float(16.0));
        h.new_object(m)
    }))
}

/// `randomFillSync`/`randomFill` core: fill `buf[offset..offset+size]` with
/// CSPRNG bytes in place, returning the same Buffer.
fn random_fill(args: &[Value]) -> Result<Value, String> {
    let buf = args.first().cloned().unwrap_or(Value::Undef);
    if super::native_tag(&buf).as_deref() != Some("Buffer") {
        return Err("Error: The \"buf\" argument must be a Buffer".into());
    }
    let cur = val_bytes(&buf);
    let len = cur.len();
    let offset = if args.len() > 1 { super::arg_num(args, 1).max(0.0) as usize } else { 0 };
    let offset = offset.min(len);
    let size = if args.len() > 2 { super::arg_num(args, 2).max(0.0) as usize } else { len - offset };
    let end = (offset + size).min(len);
    let mut rnd = vec![0u8; end.saturating_sub(offset)];
    if let Err(e) = getrandom::getrandom(&mut rnd) {
        return Err(format!("Error: failed to generate random bytes: {e}"));
    }
    let mut out = cur;
    out[offset..end].copy_from_slice(&rnd);
    with_host(|h| {
        let arr = match h.get(&buf) {
            Some(JsObj::Object(p)) => p.get("@@bytes").cloned(),
            _ => None,
        };
        if let Some(a) = arr {
            if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                *items = out.iter().map(|b| Value::Float(*b as f64)).collect();
            }
        }
    });
    Ok(buf)
}

/// `randomUUIDv7()` — RFC 9562 v7: 48-bit unix-ms timestamp prefix, version 7,
/// variant, and CSPRNG tail.
fn uuid_v7() -> Result<Value, String> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut b = [0u8; 16];
    if let Err(e) = getrandom::getrandom(&mut b) {
        return Err(format!("Error: failed to generate random bytes: {e}"));
    }
    b[0..6].copy_from_slice(&ms.to_be_bytes()[2..8]);
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = to_hex(&b);
    let uuid = format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]);
    Ok(with_host(|host| host.new_str(uuid)))
}

/// A uniform random `u64` in `[0, range)` via rejection sampling over 8 CSPRNG
/// bytes (discards the biased tail so the distribution stays exactly uniform).
fn random_below(range: u64) -> Result<u64, getrandom::Error> {
    // Largest multiple of `range` that fits in u64; values at/above it are biased.
    let limit = u64::MAX - (u64::MAX % range);
    loop {
        let mut b = [0u8; 8];
        getrandom::getrandom(&mut b)?;
        let n = u64::from_le_bytes(b);
        if n < limit {
            return Ok(n % range);
        }
    }
}

fn decode(s: &str, enc: &str) -> Vec<u8> {
    match enc.to_ascii_lowercase().as_str() {
        "hex" => super::from_hex(s),
        "base64" | "base64url" => super::from_base64(s),
        "ascii" | "latin1" | "binary" => s.chars().map(|c| c as u8).collect(),
        _ => s.as_bytes().to_vec(),
    }
}
