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
use num_bigint::BigUint;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rsa::pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey};
use sha1::Sha1;
use sha2::{Sha256, Sha384, Sha512};
use signature::{SignatureEncoding, Signer, Verifier};
use spki::{DecodePublicKey, EncodePublicKey};
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
    // Asymmetric keys & signatures
    "generateKeyPairSync",
    "generateKeyPair",
    "createPrivateKey",
    "createPublicKey",
    "createSecretKey",
    "createSign",
    "createVerify",
    "sign",
    "verify",
    "publicEncrypt",
    "privateDecrypt",
    "privateEncrypt",
    "publicDecrypt",
    // Diffie-Hellman / ECDH
    "createDiffieHellman",
    "createDiffieHellmanGroup",
    "getDiffieHellman",
    "createECDH",
    "diffieHellman",
    // Primes
    "checkPrime",
    "checkPrimeSync",
    "generatePrime",
    "generatePrimeSync",
    // Misc
    "argon2",
    "argon2Sync",
    "getRandomValues",
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
        // ── Asymmetric key generation ───────────────────────────────────
        "generateKeyPairSync" => generate_key_pair(&arg_str(args, 0).to_ascii_lowercase(), args.get(1)),
        "generateKeyPair" => {
            let cb = trailing_cb(args);
            let res = generate_key_pair(&arg_str(args, 0).to_ascii_lowercase(), args.get(1));
            match cb {
                Some(cb) => {
                    match res {
                        Ok(pair) => with_host(|h| {
                            // Deliver (err=null, publicKey, privateKey).
                            let nullv = h.null();
                            let (pubk, prvk) = match h.get(&pair) {
                                Some(JsObj::Object(p)) => (
                                    p.get("publicKey").cloned().unwrap_or(Value::Undef),
                                    p.get("privateKey").cloned().unwrap_or(Value::Undef),
                                ),
                                _ => (Value::Undef, Value::Undef),
                            };
                            h.queue_micro(cb, vec![nullv, pubk, prvk]);
                        }),
                        Err(e) => {
                            let errv = with_host(|h| h.new_str(e));
                            with_host(|h| h.queue_micro(cb, vec![errv]));
                        }
                    }
                    Ok(Value::Undef)
                }
                None => res,
            }
        }
        "createPrivateKey" => create_private_key(args.first()),
        "createPublicKey" => create_public_key(args.first()),
        "createSecretKey" => Ok(secret_key_object(&val_bytes_at(args, 0))),
        // ── Sign / Verify (streaming instances) ─────────────────────────
        "createSign" => Ok(new_sign_verify("Sign", &arg_str(args, 0))),
        "createVerify" => Ok(new_sign_verify("Verify", &arg_str(args, 0))),
        // ── Sign / Verify (one-shot) ────────────────────────────────────
        "sign" => {
            let algo = arg_str(args, 0);
            let data = val_bytes_at(args, 1);
            let key = key_material(args.get(2).unwrap_or(&Value::Undef));
            match sign_data(&key, &algo, &data) {
                Ok(sig) => Ok(super::buffer::from_bytes(&sig)),
                Err(e) => Err(e),
            }
        }
        "verify" => {
            let algo = arg_str(args, 0);
            let data = val_bytes_at(args, 1);
            let key = key_material(args.get(2).unwrap_or(&Value::Undef));
            let sig = val_bytes_at(args, 3);
            match verify_data(&key, &algo, &data, &sig) {
                Ok(ok) => Ok(Value::Bool(ok)),
                Err(e) => Err(e),
            }
        }
        // ── RSA public/private encryption ───────────────────────────────
        "publicEncrypt" => rsa_public_op(args, true, true),
        "privateDecrypt" => rsa_public_op(args, false, false),
        "privateEncrypt" => rsa_private_encrypt(args),
        "publicDecrypt" => rsa_public_decrypt(args),
        // ── Diffie-Hellman ──────────────────────────────────────────────
        "createDiffieHellman" => create_diffie_hellman(args),
        "createDiffieHellmanGroup" | "getDiffieHellman" => diffie_hellman_group(&arg_str(args, 0)),
        "createECDH" => create_ecdh(&arg_str(args, 0)),
        "diffieHellman" => diffie_hellman_oneshot(args.first()),
        // ── Primes ──────────────────────────────────────────────────────
        "checkPrimeSync" => Ok(Value::Bool(check_prime(args.first()))),
        "checkPrime" => {
            let ok = check_prime(args.first());
            let cb = trailing_cb(args);
            match cb {
                Some(cb) => {
                    with_host(|h| {
                        let nullv = h.null();
                        h.queue_micro(cb, vec![nullv, Value::Bool(ok)]);
                    });
                    Ok(Value::Undef)
                }
                None => Ok(Value::Bool(ok)),
            }
        }
        "generatePrimeSync" => generate_prime(super::arg_num(args, 0) as usize, opts_object(args, 1)),
        "generatePrime" => {
            let res = generate_prime(super::arg_num(args, 0) as usize, opts_object(args, 1));
            let cb = trailing_cb(args);
            match (cb, res) {
                (Some(cb), Ok(v)) => {
                    with_host(|h| {
                        let nullv = h.null();
                        h.queue_micro(cb, vec![nullv, v]);
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
        // ── argon2 ──────────────────────────────────────────────────────
        "argon2Sync" => match argon2_hash(&arg_str(args, 0), args.get(1)) {
            Ok(out) => Ok(super::buffer::from_bytes(&out)),
            Err(e) => Err(e),
        },
        "argon2" => deliver_async(trailing_cb(args), argon2_hash(&arg_str(args, 0), args.get(1))),
        // ── WebCrypto getRandomValues ───────────────────────────────────
        "getRandomValues" => get_random_values(args.first()),
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

// ════════════════════════════════════════════════════════════════════════
//  Asymmetric cryptography: key generation, KeyObjects, sign/verify, RSA
//  encryption, Diffie-Hellman / ECDH, primes, argon2, X.509.
// ════════════════════════════════════════════════════════════════════════

/// A `KeyObject` for an asymmetric key: carries `type` (`private`/`public`),
/// `asymmetricKeyType`, and the PEM material in the hidden `@@pem`.
fn key_object(kind: &str, asym: &str, pem: &str) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("KeyObject"));
        m.insert("type".into(), h.new_str(kind));
        m.insert("asymmetricKeyType".into(), h.new_str(asym));
        m.insert("@@pem".into(), h.new_str(pem));
        h.new_object(m)
    })
}

/// A secret (symmetric) `KeyObject` wrapping raw bytes.
fn secret_key_object(bytes: &[u8]) -> Value {
    with_host(|h| {
        let arr = h.new_array(bytes.iter().map(|b| Value::Float(*b as f64)).collect());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("KeyObject"));
        m.insert("type".into(), h.new_str("secret"));
        m.insert("@@secret".into(), arr);
        h.new_object(m)
    })
}

/// Wrap DER bytes as a PEM block with the given label (64-char lines, LF).
fn pem_wrap(label: &str, der: &[u8]) -> String {
    let b64 = to_base64(der);
    let mut s = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        s.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        s.push('\n');
    }
    s.push_str(&format!("-----END {label}-----\n"));
    s
}

/// Extract the DER bytes from a single PEM block (any label).
fn pem_body(pem: &str) -> Vec<u8> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    super::from_base64(&body)
}

/// PKCS#8 PEM for a raw X25519 private key (OID 1.3.101.110).
fn x25519_private_pem(raw: &[u8]) -> String {
    let mut der = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x04, 0x22, 0x04,
        0x20,
    ];
    der.extend_from_slice(raw);
    pem_wrap("PRIVATE KEY", &der)
}

/// SPKI PEM for a raw X25519 public key.
fn x25519_public_pem(raw: &[u8]) -> String {
    let mut der = vec![
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x03, 0x21, 0x00,
    ];
    der.extend_from_slice(raw);
    pem_wrap("PUBLIC KEY", &der)
}

/// The trailing 32 raw bytes of an X25519 PKCS#8/SPKI PEM.
fn x25519_raw(pem: &str) -> Option<[u8; 32]> {
    let der = pem_body(pem);
    if der.len() < 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&der[der.len() - 32..]);
    Some(out)
}

/// The options object at `args[1]` for key generation.
fn keygen_encoding(opts: Option<&Value>, priv_side: bool) -> Option<String> {
    let o = opts?;
    let field = if priv_side { "privateKeyEncoding" } else { "publicKeyEncoding" };
    with_host(|h| {
        let JsObj::Object(p) = h.get(o)? else { return None };
        let enc = p.get(field)?;
        let JsObj::Object(e) = h.get(enc)? else { return None };
        e.get("format").map(|v| h.str_of(v))
    })
}

/// `generateKeyPairSync(type, opts)`: RSA / EC (P-256, P-384) / Ed25519 / X25519.
/// Returns `{ publicKey, privateKey }` — PEM strings when the matching
/// `*KeyEncoding.format` is `'pem'`, else `KeyObject`s.
fn generate_key_pair(kind: &str, opts: Option<&Value>) -> Result<Value, String> {
    let mut rng = rand_core::OsRng;
    let (asym, priv_pem, pub_pem) = match kind {
        "rsa" => {
            let bits = opt_num(opts.unwrap_or(&Value::Undef), &["modulusLength"], 2048.0) as usize;
            let sk = rsa::RsaPrivateKey::new(&mut rng, bits).map_err(|e| format!("Error: {e}"))?;
            let pk = rsa::RsaPublicKey::from(&sk);
            let priv_pem = sk.to_pkcs8_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))?.to_string();
            let pub_pem = pk.to_public_key_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))?;
            ("rsa", priv_pem, pub_pem)
        }
        "ec" => {
            let curve = opt_str(opts.unwrap_or(&Value::Undef), "namedCurve");
            match ec_curve_id(&curve) {
                Some("p256") => {
                    let sk = p256::SecretKey::random(&mut rng);
                    let priv_pem = sk.to_pkcs8_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))?.to_string();
                    let pub_pem = sk.public_key().to_public_key_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))?;
                    ("ec", priv_pem, pub_pem)
                }
                Some("p384") => {
                    let sk = p384::SecretKey::random(&mut rng);
                    let priv_pem = sk.to_pkcs8_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))?.to_string();
                    let pub_pem = sk.public_key().to_public_key_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))?;
                    ("ec", priv_pem, pub_pem)
                }
                _ => return Err(format!("Error: Unsupported EC curve: {curve}")),
            }
        }
        "ed25519" => {
            let sk = ed25519_dalek::SigningKey::generate(&mut rng);
            let priv_pem = sk.to_pkcs8_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))?.to_string();
            let pub_pem = sk.verifying_key().to_public_key_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))?;
            ("ed25519", priv_pem, pub_pem)
        }
        "x25519" => {
            let sk = x25519_dalek::StaticSecret::random_from_rng(rng);
            let pk = x25519_dalek::PublicKey::from(&sk);
            (
                "x25519",
                x25519_private_pem(&sk.to_bytes()),
                x25519_public_pem(pk.as_bytes()),
            )
        }
        _ => return Err(format!("Error: Unsupported key type: {kind}")),
    };
    let pub_is_pem = keygen_encoding(opts, false).as_deref() == Some("pem");
    let priv_is_pem = keygen_encoding(opts, true).as_deref() == Some("pem");
    let publik = if pub_is_pem {
        with_host(|h| h.new_str(pub_pem.clone()))
    } else {
        key_object("public", asym, &pub_pem)
    };
    let privat = if priv_is_pem {
        with_host(|h| h.new_str(priv_pem.clone()))
    } else {
        key_object("private", asym, &priv_pem)
    };
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("publicKey".into(), publik);
        m.insert("privateKey".into(), privat);
        h.new_object(m)
    }))
}

/// Map a Node EC curve name to an internal id.
fn ec_curve_id(name: &str) -> Option<&'static str> {
    match name {
        "P-256" | "prime256v1" | "secp256r1" => Some("p256"),
        "P-384" | "secp384r1" => Some("p384"),
        _ => None,
    }
}

/// A string option on an options object (empty if absent).
fn opt_str(obj: &Value, key: &str) -> String {
    with_host(|h| match h.get(obj) {
        Some(JsObj::Object(p)) => p.get(key).map(|v| h.str_of(v)).unwrap_or_default(),
        _ => String::new(),
    })
}

/// Detect the asymmetric type of a private-key PEM/DER by trial parsing.
fn detect_private(bytes: &[u8]) -> Option<&'static str> {
    let pem = std::str::from_utf8(bytes).ok();
    if pem.map(|p| rsa::RsaPrivateKey::from_pkcs8_pem(p).is_ok() || rsa::RsaPrivateKey::from_pkcs1_pem(p).is_ok()).unwrap_or(false)
        || rsa::RsaPrivateKey::from_pkcs8_der(bytes).is_ok()
    {
        return Some("rsa");
    }
    if pem.map(|p| p256::SecretKey::from_pkcs8_pem(p).is_ok()).unwrap_or(false) {
        return Some("ec");
    }
    if pem.map(|p| p384::SecretKey::from_pkcs8_pem(p).is_ok()).unwrap_or(false) {
        return Some("ec");
    }
    if pem.map(|p| ed25519_dalek::SigningKey::from_pkcs8_pem(p).is_ok()).unwrap_or(false) {
        return Some("ed25519");
    }
    if pem.map(|p| p.contains("PRIVATE KEY")).unwrap_or(false) && x25519_raw(pem.unwrap_or("")).is_some() {
        // X25519 PKCS#8 is a fixed 48-byte structure; distinguish by OID byte.
        let der = pem_body(pem.unwrap_or(""));
        if der.len() == 48 && der[9..12] == [0x2b, 0x65, 0x6e] {
            return Some("x25519");
        }
    }
    None
}

/// Detect the asymmetric type of a public-key PEM/DER by trial parsing.
fn detect_public(bytes: &[u8]) -> Option<&'static str> {
    let pem = std::str::from_utf8(bytes).ok();
    if pem.map(|p| rsa::RsaPublicKey::from_public_key_pem(p).is_ok()).unwrap_or(false) {
        return Some("rsa");
    }
    if pem.map(|p| p256::PublicKey::from_public_key_pem(p).is_ok()).unwrap_or(false) {
        return Some("ec");
    }
    if pem.map(|p| p384::PublicKey::from_public_key_pem(p).is_ok()).unwrap_or(false) {
        return Some("ec");
    }
    if pem.map(|p| ed25519_dalek::VerifyingKey::from_public_key_pem(p).is_ok()).unwrap_or(false) {
        return Some("ed25519");
    }
    if let Some(p) = pem {
        let der = pem_body(p);
        if der.len() == 44 && der[6..9] == [0x2b, 0x65, 0x6e] {
            return Some("x25519");
        }
    }
    None
}

/// `createPrivateKey(input)` → a private `KeyObject`.
fn create_private_key(input: Option<&Value>) -> Result<Value, String> {
    let bytes = input.map(key_material).unwrap_or_default();
    let asym = detect_private(&bytes).ok_or("Error: Failed to read private key")?;
    let pem = String::from_utf8_lossy(&bytes).into_owned();
    Ok(key_object("private", asym, &pem))
}

/// `createPublicKey(input)` → a public `KeyObject`. Accepts a public key, a
/// private key/`KeyObject` (derives the public half), or a PEM/DER buffer.
fn create_public_key(input: Option<&Value>) -> Result<Value, String> {
    let bytes = input.map(key_material).unwrap_or_default();
    if let Some(asym) = detect_public(&bytes) {
        let pem = String::from_utf8_lossy(&bytes).into_owned();
        return Ok(key_object("public", asym, &pem));
    }
    // Derive the public key from a private key.
    if let Some(asym) = detect_private(&bytes) {
        let pem = public_pem_from_private(&bytes, asym)?;
        return Ok(key_object("public", asym, &pem));
    }
    Err("Error: Failed to read public key".into())
}

/// The SPKI public PEM derived from a private-key PEM/DER of a known type.
fn public_pem_from_private(bytes: &[u8], asym: &str) -> Result<String, String> {
    let pem = std::str::from_utf8(bytes).ok();
    let err = || "Error: Failed to derive public key".to_string();
    match asym {
        "rsa" => {
            let sk = pem
                .and_then(|p| rsa::RsaPrivateKey::from_pkcs8_pem(p).ok())
                .or_else(|| rsa::RsaPrivateKey::from_pkcs8_der(bytes).ok())
                .ok_or_else(err)?;
            rsa::RsaPublicKey::from(&sk).to_public_key_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))
        }
        "ec" => {
            if let Some(sk) = pem.and_then(|p| p256::SecretKey::from_pkcs8_pem(p).ok()) {
                return sk.public_key().to_public_key_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"));
            }
            let sk = pem.and_then(|p| p384::SecretKey::from_pkcs8_pem(p).ok()).ok_or_else(err)?;
            sk.public_key().to_public_key_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))
        }
        "ed25519" => {
            let sk = pem.and_then(|p| ed25519_dalek::SigningKey::from_pkcs8_pem(p).ok()).ok_or_else(err)?;
            sk.verifying_key().to_public_key_pem(LineEnding::LF).map_err(|e| format!("Error: {e}"))
        }
        "x25519" => {
            let raw = pem.and_then(x25519_raw).ok_or_else(err)?;
            let sk = x25519_dalek::StaticSecret::from(raw);
            Ok(x25519_public_pem(x25519_dalek::PublicKey::from(&sk).as_bytes()))
        }
        _ => Err(err()),
    }
}

/// Raw key material of a key argument: a `KeyObject`'s stored PEM, a `{ key }`
/// wrapper's inner key, a Buffer's bytes, or a PEM/DER string's bytes.
fn key_material(v: &Value) -> Vec<u8> {
    if super::native_tag(v).as_deref() == Some("KeyObject") {
        return with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => p.get("@@pem").map(|s| h.str_of(s)).unwrap_or_default(),
            _ => String::new(),
        })
        .into_bytes();
    }
    if super::native_tag(v).as_deref() != Some("Buffer") {
        let inner = with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => p.get("key").cloned(),
            _ => None,
        });
        if let Some(k) = inner {
            return key_material(&k);
        }
    }
    val_bytes(v)
}

/// A `Sign`/`Verify` streaming instance (`update(...).sign(key)` /
/// `update(...).verify(key, sig)`).
fn new_sign_verify(tag: &str, algo: &str) -> Value {
    with_host(|h| {
        let data = h.new_array(Vec::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str(tag));
        m.insert("@@algo".into(), h.new_str(algo.to_ascii_lowercase()));
        m.insert("@@data".into(), data);
        h.new_object(m)
    })
}

/// Normalize a Node signature-algorithm name to a bare digest (`sha256`).
fn digest_of(algo: &str) -> String {
    let a = algo.to_ascii_lowercase();
    let a = a.strip_prefix("rsa-").unwrap_or(&a);
    a.replace('-', "")
}

/// One-shot asymmetric sign over `data` with a private key (auto key-type).
fn sign_data(key: &[u8], algo: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let pem = std::str::from_utf8(key).ok();
    let d = digest_of(algo);
    if let Some(sk) = pem
        .and_then(|p| rsa::RsaPrivateKey::from_pkcs8_pem(p).ok())
        .or_else(|| pem.and_then(|p| rsa::RsaPrivateKey::from_pkcs1_pem(p).ok()))
        .or_else(|| rsa::RsaPrivateKey::from_pkcs8_der(key).ok())
    {
        return rsa_sign(&sk, &d, data);
    }
    if let Some(sk) = pem.and_then(|p| p256::ecdsa::SigningKey::from_pkcs8_pem(p).ok()) {
        let sig: p256::ecdsa::Signature = sk.try_sign(data).map_err(|e| format!("Error: {e}"))?;
        return Ok(sig.to_der().as_bytes().to_vec());
    }
    if let Some(sk) = pem.and_then(|p| p384::ecdsa::SigningKey::from_pkcs8_pem(p).ok()) {
        let sig: p384::ecdsa::Signature = sk.try_sign(data).map_err(|e| format!("Error: {e}"))?;
        return Ok(sig.to_der().as_bytes().to_vec());
    }
    if let Some(sk) = pem.and_then(|p| ed25519_dalek::SigningKey::from_pkcs8_pem(p).ok()) {
        return Ok(sk.sign(data).to_bytes().to_vec());
    }
    Err("Error: Invalid or unsupported private key for signing".into())
}

/// RSA PKCS#1 v1.5 signature over `data` for the selected digest.
fn rsa_sign(sk: &rsa::RsaPrivateKey, digest: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let sig = match digest {
        "sha256" => rsa::pkcs1v15::SigningKey::<Sha256>::new(sk.clone()).sign(data).to_vec(),
        "sha384" => rsa::pkcs1v15::SigningKey::<Sha384>::new(sk.clone()).sign(data).to_vec(),
        "sha512" => rsa::pkcs1v15::SigningKey::<Sha512>::new(sk.clone()).sign(data).to_vec(),
        _ => return Err(format!("Error: Unsupported digest for RSA signing: {digest}")),
    };
    Ok(sig)
}

/// One-shot asymmetric verify (auto key-type). ECDSA signatures are DER.
fn verify_data(key: &[u8], algo: &str, data: &[u8], sig: &[u8]) -> Result<bool, String> {
    let pem = std::str::from_utf8(key).ok();
    let d = digest_of(algo);
    if let Some(pk) = pem
        .and_then(|p| rsa::RsaPublicKey::from_public_key_pem(p).ok())
        .or_else(|| pem.and_then(|p| rsa::RsaPublicKey::from_pkcs1_pem(p).ok()))
        .or_else(|| pem.and_then(|p| rsa::RsaPrivateKey::from_pkcs8_pem(p).ok()).map(|s| rsa::RsaPublicKey::from(&s)))
    {
        return rsa_verify(&pk, &d, data, sig);
    }
    if let Some(vk) = pem
        .and_then(|p| p256::ecdsa::VerifyingKey::from_public_key_pem(p).ok())
        .or_else(|| pem.and_then(|p| p256::ecdsa::SigningKey::from_pkcs8_pem(p).ok()).map(|s| *s.verifying_key()))
    {
        let s = p256::ecdsa::Signature::from_der(sig).map_err(|e| format!("Error: {e}"))?;
        return Ok(vk.verify(data, &s).is_ok());
    }
    if let Some(vk) = pem
        .and_then(|p| p384::ecdsa::VerifyingKey::from_public_key_pem(p).ok())
        .or_else(|| pem.and_then(|p| p384::ecdsa::SigningKey::from_pkcs8_pem(p).ok()).map(|s| *s.verifying_key()))
    {
        let s = p384::ecdsa::Signature::from_der(sig).map_err(|e| format!("Error: {e}"))?;
        return Ok(vk.verify(data, &s).is_ok());
    }
    if let Some(vk) = pem
        .and_then(|p| ed25519_dalek::VerifyingKey::from_public_key_pem(p).ok())
        .or_else(|| pem.and_then(|p| ed25519_dalek::SigningKey::from_pkcs8_pem(p).ok()).map(|s| s.verifying_key()))
    {
        let s = ed25519_dalek::Signature::from_slice(sig).map_err(|e| format!("Error: {e}"))?;
        return Ok(vk.verify(data, &s).is_ok());
    }
    Err("Error: Invalid or unsupported public key for verifying".into())
}

/// RSA PKCS#1 v1.5 verify for the selected digest.
fn rsa_verify(pk: &rsa::RsaPublicKey, digest: &str, data: &[u8], sig: &[u8]) -> Result<bool, String> {
    let signature = match rsa::pkcs1v15::Signature::try_from(sig) {
        Ok(s) => s,
        Err(_) => return Ok(false),
    };
    let ok = match digest {
        "sha256" => rsa::pkcs1v15::VerifyingKey::<Sha256>::new(pk.clone()).verify(data, &signature).is_ok(),
        "sha384" => rsa::pkcs1v15::VerifyingKey::<Sha384>::new(pk.clone()).verify(data, &signature).is_ok(),
        "sha512" => rsa::pkcs1v15::VerifyingKey::<Sha512>::new(pk.clone()).verify(data, &signature).is_ok(),
        _ => return Err(format!("Error: Unsupported digest for RSA verifying: {digest}")),
    };
    Ok(ok)
}

/// `Sign`/`Verify` instance dispatch.
pub fn sign_verify_instance_call(tag: &str, recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "update" => {
            let enc = if args.len() > 1 { arg_str(args, 1) } else { "utf8".into() };
            let bytes = if super::native_tag(args.first().unwrap_or(&Value::Undef)).as_deref() == Some("Buffer") {
                val_bytes_at(args, 0)
            } else {
                decode(&arg_str(args, 0), &enc)
            };
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
        "sign" => {
            let (algo, data) = sign_verify_state(recv);
            let key = key_material(args.first().unwrap_or(&Value::Undef));
            let sig = sign_data(&key, &algo, &data)?;
            let out_enc = if args.len() > 1 { Some(arg_str(args, 1)) } else { None };
            Ok(encode_out(&sig, out_enc.as_deref()))
        }
        "verify" => {
            let (algo, data) = sign_verify_state(recv);
            let key = key_material(args.first().unwrap_or(&Value::Undef));
            let sig = if args.len() > 2 {
                decode(&arg_str(args, 1), &arg_str(args, 2))
            } else {
                val_bytes_at(args, 1)
            };
            Ok(Value::Bool(verify_data(&key, &algo, &data, &sig)?))
        }
        _ => Err(crate::host::type_error(&format!("{}.{method} is not a function", tag.to_ascii_lowercase()))),
    }
}

/// Read the `@@algo`/`@@data` of a `Sign`/`Verify` instance.
fn sign_verify_state(recv: &Value) -> (String, Vec<u8>) {
    with_host(|h| {
        let (mut algo, mut data) = (String::new(), Vec::new());
        if let Some(JsObj::Object(p)) = h.get(recv) {
            algo = p.get("@@algo").map(|v| h.str_of(v)).unwrap_or_default();
            if let Some(JsObj::Array(items)) = p.get("@@data").and_then(|v| h.get(v)) {
                data = items.iter().map(|v| h.to_number(v) as u8).collect();
            }
        }
        (algo, data)
    })
}

/// Read the numeric `padding`/oaepHash options from a key argument object.
fn rsa_padding(v: &Value) -> (i64, String) {
    let padding = opt_num(v, &["padding"], 4.0) as i64; // RSA_PKCS1_OAEP_PADDING
    let oaep = {
        let h = opt_str(v, "oaepHash");
        if h.is_empty() { "sha1".to_string() } else { h.to_ascii_lowercase() }
    };
    (padding, oaep)
}

/// Parse an RSA public key from PEM/DER (or derive from a private key).
fn parse_rsa_public(key: &[u8]) -> Result<rsa::RsaPublicKey, String> {
    let pem = std::str::from_utf8(key).ok();
    pem.and_then(|p| rsa::RsaPublicKey::from_public_key_pem(p).ok())
        .or_else(|| pem.and_then(|p| rsa::RsaPublicKey::from_pkcs1_pem(p).ok()))
        .or_else(|| rsa::RsaPublicKey::from_public_key_der(key).ok())
        .or_else(|| pem.and_then(|p| rsa::RsaPrivateKey::from_pkcs8_pem(p).ok()).map(|s| rsa::RsaPublicKey::from(&s)))
        .ok_or_else(|| "Error: Failed to parse RSA public key".into())
}

/// Parse an RSA private key from PEM/DER.
fn parse_rsa_private(key: &[u8]) -> Result<rsa::RsaPrivateKey, String> {
    let pem = std::str::from_utf8(key).ok();
    pem.and_then(|p| rsa::RsaPrivateKey::from_pkcs8_pem(p).ok())
        .or_else(|| pem.and_then(|p| rsa::RsaPrivateKey::from_pkcs1_pem(p).ok()))
        .or_else(|| rsa::RsaPrivateKey::from_pkcs8_der(key).ok())
        .ok_or_else(|| "Error: Failed to parse RSA private key".into())
}

/// `publicEncrypt` / `privateDecrypt`: OAEP by default, PKCS#1 v1.5 when
/// `padding == RSA_PKCS1_PADDING (1)`.
fn rsa_public_op(args: &[Value], _public: bool, encrypt: bool) -> Result<Value, String> {
    let key_arg = args.first().cloned().unwrap_or(Value::Undef);
    let key = key_material(&key_arg);
    let (padding, oaep) = rsa_padding(&key_arg);
    let data = val_bytes_at(args, 1);
    let out = if encrypt {
        let pk = parse_rsa_public(&key)?;
        let mut rng = rand_core::OsRng;
        if padding == 1 {
            pk.encrypt(&mut rng, rsa::Pkcs1v15Encrypt, &data)
        } else if oaep == "sha256" {
            pk.encrypt(&mut rng, rsa::Oaep::new::<Sha256>(), &data)
        } else {
            pk.encrypt(&mut rng, rsa::Oaep::new::<Sha1>(), &data)
        }
        .map_err(|e| format!("Error: {e}"))?
    } else {
        let sk = parse_rsa_private(&key)?;
        if padding == 1 {
            sk.decrypt(rsa::Pkcs1v15Encrypt, &data)
        } else if oaep == "sha256" {
            sk.decrypt(rsa::Oaep::new::<Sha256>(), &data)
        } else {
            sk.decrypt(rsa::Oaep::new::<Sha1>(), &data)
        }
        .map_err(|e| format!("Error: {e}"))?
    };
    Ok(super::buffer::from_bytes(&out))
}

/// `privateEncrypt`: raw RSA PKCS#1 v1.5 (type 1) block signed with the
/// private key.
fn rsa_private_encrypt(args: &[Value]) -> Result<Value, String> {
    let key = key_material(args.first().unwrap_or(&Value::Undef));
    let data = val_bytes_at(args, 1);
    let sk = parse_rsa_private(&key)?;
    let out = sk
        .sign(rsa::Pkcs1v15Sign::new_unprefixed(), &data)
        .map_err(|e| format!("Error: {e}"))?;
    Ok(super::buffer::from_bytes(&out))
}

/// `publicDecrypt`: recover a `privateEncrypt` block via raw modular
/// exponentiation `s^e mod n`, then strip PKCS#1 type-1 padding.
fn rsa_public_decrypt(args: &[Value]) -> Result<Value, String> {
    use rsa::traits::PublicKeyParts;
    let key = key_material(args.first().unwrap_or(&Value::Undef));
    let ct = val_bytes_at(args, 1);
    let pk = parse_rsa_public(&key)?;
    let n = BigUint::from_bytes_be(&pk.n().to_bytes_be());
    let e = BigUint::from_bytes_be(&pk.e().to_bytes_be());
    let k = pk.n().to_bytes_be().len();
    let s = BigUint::from_bytes_be(&ct);
    let m = s.modpow(&e, &n);
    let mut em = m.to_bytes_be();
    while em.len() < k {
        em.insert(0, 0);
    }
    // EM = 0x00 0x01 0xFF..0xFF 0x00 || message
    if em.len() < 11 || em[0] != 0x00 || em[1] != 0x01 {
        return Err("Error: error:0200006E:rsa routines::padding check failed".into());
    }
    let sep = em[2..].iter().position(|&b| b == 0x00).map(|i| i + 2);
    match sep {
        Some(i) => Ok(super::buffer::from_bytes(&em[i + 1..])),
        None => Err("Error: error:0200006E:rsa routines::padding check failed".into()),
    }
}

// ── Diffie-Hellman (finite-field) ───────────────────────────────────────

/// RFC 2409/3526 MODP group primes (hex), generator 2.
const MODP_GROUPS: &[(&str, &str)] = &[
    ("modp1", MODP1),
    ("modp2", MODP2),
    ("modp5", MODP5),
    ("modp14", MODP14),
    ("modp15", MODP15),
    ("modp16", MODP16),
    ("modp17", MODP17),
    ("modp18", MODP18),
];

/// Byte-array property of an object as raw bytes.
fn obj_bytes(recv: &Value, key: &str) -> Vec<u8> {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get(recv) {
            if let Some(JsObj::Array(it)) = p.get(key).and_then(|v| h.get(v)) {
                return it.iter().map(|v| h.to_number(v) as u8).collect();
            }
        }
        Vec::new()
    })
}

/// Store raw bytes as a hidden byte-array property.
fn set_obj_bytes(recv: &Value, key: &str, bytes: &[u8]) {
    with_host(|h| {
        let arr = h.new_array(bytes.iter().map(|b| Value::Float(*b as f64)).collect());
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), arr);
        }
    });
}

/// Build a `DiffieHellman` instance from a prime + generator.
fn dh_object(prime: &[u8], gen: &[u8]) -> Value {
    with_host(|h| {
        let pv = h.new_array(prime.iter().map(|b| Value::Float(*b as f64)).collect());
        let gv = h.new_array(gen.iter().map(|b| Value::Float(*b as f64)).collect());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("DiffieHellman"));
        m.insert("@@prime".into(), pv);
        m.insert("@@gen".into(), gv);
        h.new_object(m)
    })
}

/// `createDiffieHellman(primeLength)` or `createDiffieHellman(prime[, generator])`.
fn create_diffie_hellman(args: &[Value]) -> Result<Value, String> {
    // Numeric first arg → generate a prime of that bit length; generator 2.
    let first = args.first().cloned().unwrap_or(Value::Undef);
    if matches!(first, Value::Int(_) | Value::Float(_)) {
        let bits = super::arg_num(args, 0) as usize;
        let prime = gen_prime(bits)?;
        return Ok(dh_object(&prime.to_bytes_be(), &[2]));
    }
    let prime = val_bytes_at(args, 0);
    let gen = if args.len() > 1 {
        match args.get(1) {
            Some(Value::Int(_)) | Some(Value::Float(_)) => {
                let g = super::arg_num(args, 1) as u64;
                BigUint::from(g).to_bytes_be()
            }
            _ => val_bytes_at(args, 1),
        }
    } else {
        vec![2]
    };
    Ok(dh_object(&prime, &gen))
}

/// `getDiffieHellman(group)` / `createDiffieHellmanGroup(group)`.
fn diffie_hellman_group(name: &str) -> Result<Value, String> {
    let hex = MODP_GROUPS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, h)| *h)
        .ok_or_else(|| format!("Error: Unknown group: {name}"))?;
    Ok(dh_object(&super::from_hex(hex), &[2]))
}

/// `DiffieHellman` instance dispatch.
pub fn dh_instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let enc = |args: &[Value], i: usize| -> Option<String> {
        if args.len() > i {
            let s = arg_str(args, i);
            if s.is_empty() { None } else { Some(s) }
        } else {
            None
        }
    };
    match method {
        "generateKeys" => {
            let p = BigUint::from_bytes_be(&obj_bytes(recv, "@@prime"));
            let g = BigUint::from_bytes_be(&obj_bytes(recv, "@@gen"));
            let priv_key = dh_random_priv(&p)?;
            let pub_key = g.modpow(&priv_key, &p);
            set_obj_bytes(recv, "@@priv", &priv_key.to_bytes_be());
            set_obj_bytes(recv, "@@pub", &pub_key.to_bytes_be());
            Ok(encode_out(&pub_key.to_bytes_be(), enc(args, 0).as_deref()))
        }
        "computeSecret" => {
            let other = if args.len() > 1 && !arg_str(args, 1).is_empty() && !matches!(args.first(), Some(v) if super::native_tag(v).as_deref() == Some("Buffer")) {
                decode(&arg_str(args, 0), &arg_str(args, 1))
            } else {
                val_bytes_at(args, 0)
            };
            let p = BigUint::from_bytes_be(&obj_bytes(recv, "@@prime"));
            let priv_key = BigUint::from_bytes_be(&obj_bytes(recv, "@@priv"));
            let their_pub = BigUint::from_bytes_be(&other);
            let secret = their_pub.modpow(&priv_key, &p);
            let out_enc = if args.len() > 2 { enc(args, 2) } else { None };
            Ok(encode_out(&secret.to_bytes_be(), out_enc.as_deref()))
        }
        "getPrime" => Ok(encode_out(&obj_bytes(recv, "@@prime"), enc(args, 0).as_deref())),
        "getGenerator" => Ok(encode_out(&obj_bytes(recv, "@@gen"), enc(args, 0).as_deref())),
        "getPublicKey" => Ok(encode_out(&obj_bytes(recv, "@@pub"), enc(args, 0).as_deref())),
        "getPrivateKey" => Ok(encode_out(&obj_bytes(recv, "@@priv"), enc(args, 0).as_deref())),
        "setPublicKey" => {
            set_obj_bytes(recv, "@@pub", &val_bytes_at(args, 0));
            Ok(recv.clone())
        }
        "setPrivateKey" => {
            set_obj_bytes(recv, "@@priv", &val_bytes_at(args, 0));
            Ok(recv.clone())
        }
        _ => Err(crate::host::type_error(&format!("dh.{method} is not a function"))),
    }
}

/// A random DH private exponent in `[2, p-2]`.
fn dh_random_priv(p: &BigUint) -> Result<BigUint, String> {
    let nbytes = p.to_bytes_be().len();
    let mut buf = vec![0u8; nbytes];
    getrandom::getrandom(&mut buf).map_err(|e| format!("Error: {e}"))?;
    let two = BigUint::from(2u32);
    let modulus = p - &two; // p-2 range size
    let x = BigUint::from_bytes_be(&buf) % &modulus;
    Ok(x + &two)
}

// ── ECDH ────────────────────────────────────────────────────────────────

/// `createECDH(curve)` — P-256 / P-384.
fn create_ecdh(curve: &str) -> Result<Value, String> {
    let id = ec_curve_id(curve).ok_or_else(|| format!("Error: Unsupported curve: {curve}"))?;
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("ECDH"));
        m.insert("@@curve".into(), h.new_str(id));
        h.new_object(m)
    }))
}

/// `ECDH` instance dispatch.
pub fn ecdh_instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let curve = obj_str(recv, "@@curve");
    match method {
        "generateKeys" => {
            let (priv_b, pub_b) = ecdh_generate(&curve)?;
            set_obj_bytes(recv, "@@priv", &priv_b);
            set_obj_bytes(recv, "@@pub", &pub_b);
            let out_enc = if args.len() > 1 { Some(arg_str(args, 1)) } else { None };
            Ok(encode_out(&pub_b, out_enc.as_deref()))
        }
        "computeSecret" => {
            let other = if args.len() > 1 && !arg_str(args, 1).is_empty() && super::native_tag(args.first().unwrap_or(&Value::Undef)).as_deref() != Some("Buffer") {
                decode(&arg_str(args, 0), &arg_str(args, 1))
            } else {
                val_bytes_at(args, 0)
            };
            let secret = ecdh_compute(&curve, &obj_bytes(recv, "@@priv"), &other)?;
            let out_enc = if args.len() > 2 { Some(arg_str(args, 2)) } else { None };
            Ok(encode_out(&secret, out_enc.as_deref()))
        }
        "getPublicKey" => {
            let out_enc = if args.len() > 1 { Some(arg_str(args, 1)) } else { None };
            Ok(encode_out(&obj_bytes(recv, "@@pub"), out_enc.as_deref()))
        }
        "getPrivateKey" => {
            let out_enc = if !args.is_empty() { Some(arg_str(args, 0)) } else { None };
            Ok(encode_out(&obj_bytes(recv, "@@priv"), out_enc.as_deref()))
        }
        "setPrivateKey" => {
            let priv_b = val_bytes_at(args, 0);
            let pub_b = ecdh_public_from_private(&curve, &priv_b)?;
            set_obj_bytes(recv, "@@priv", &priv_b);
            set_obj_bytes(recv, "@@pub", &pub_b);
            Ok(recv.clone())
        }
        _ => Err(crate::host::type_error(&format!("ecdh.{method} is not a function"))),
    }
}

/// Generate an ECDH keypair; public key is the uncompressed SEC1 point.
fn ecdh_generate(curve: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut rng = rand_core::OsRng;
    match curve {
        "p256" => {
            let sk = p256::SecretKey::random(&mut rng);
            let pubk = sk.public_key().to_encoded_point(false).as_bytes().to_vec();
            Ok((sk.to_bytes().to_vec(), pubk))
        }
        "p384" => {
            let sk = p384::SecretKey::random(&mut rng);
            let pubk = sk.public_key().to_encoded_point(false).as_bytes().to_vec();
            Ok((sk.to_bytes().to_vec(), pubk))
        }
        _ => Err(format!("Error: Unsupported curve: {curve}")),
    }
}

/// The uncompressed SEC1 public point for a raw ECDH private scalar.
fn ecdh_public_from_private(curve: &str, priv_b: &[u8]) -> Result<Vec<u8>, String> {
    match curve {
        "p256" => {
            let sk = p256::SecretKey::from_slice(priv_b).map_err(|e| format!("Error: {e}"))?;
            Ok(sk.public_key().to_encoded_point(false).as_bytes().to_vec())
        }
        "p384" => {
            let sk = p384::SecretKey::from_slice(priv_b).map_err(|e| format!("Error: {e}"))?;
            Ok(sk.public_key().to_encoded_point(false).as_bytes().to_vec())
        }
        _ => Err(format!("Error: Unsupported curve: {curve}")),
    }
}

/// ECDH shared secret (raw X coordinate) from a private scalar + peer point.
fn ecdh_compute(curve: &str, priv_b: &[u8], pub_b: &[u8]) -> Result<Vec<u8>, String> {
    match curve {
        "p256" => {
            let sk = p256::SecretKey::from_slice(priv_b).map_err(|e| format!("Error: {e}"))?;
            let pk = p256::PublicKey::from_sec1_bytes(pub_b).map_err(|e| format!("Error: {e}"))?;
            let shared = p256::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine());
            Ok(shared.raw_secret_bytes().to_vec())
        }
        "p384" => {
            let sk = p384::SecretKey::from_slice(priv_b).map_err(|e| format!("Error: {e}"))?;
            let pk = p384::PublicKey::from_sec1_bytes(pub_b).map_err(|e| format!("Error: {e}"))?;
            let shared = p384::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine());
            Ok(shared.raw_secret_bytes().to_vec())
        }
        _ => Err(format!("Error: Unsupported curve: {curve}")),
    }
}

/// String property helper.
fn obj_str(recv: &Value, key: &str) -> String {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).map(|v| h.str_of(v)).unwrap_or_default(),
        _ => String::new(),
    })
}

/// `crypto.diffieHellman({ privateKey, publicKey })` one-shot (EC / X25519).
fn diffie_hellman_oneshot(opts: Option<&Value>) -> Result<Value, String> {
    let o = opts.ok_or("Error: options object required")?;
    let priv_v = with_host(|h| match h.get(o) {
        Some(JsObj::Object(p)) => p.get("privateKey").cloned(),
        _ => None,
    })
    .ok_or("Error: privateKey required")?;
    let pub_v = with_host(|h| match h.get(o) {
        Some(JsObj::Object(p)) => p.get("publicKey").cloned(),
        _ => None,
    })
    .ok_or("Error: publicKey required")?;
    let priv_bytes = key_material(&priv_v);
    let pub_bytes = key_material(&pub_v);
    let asym = detect_private(&priv_bytes).ok_or("Error: unsupported private key")?;
    let priv_pem = std::str::from_utf8(&priv_bytes).ok();
    let pub_pem = std::str::from_utf8(&pub_bytes).ok();
    let secret = match asym {
        "ec" => {
            if let Some(sk) = priv_pem.and_then(|p| p256::SecretKey::from_pkcs8_pem(p).ok()) {
                let pk = pub_pem.and_then(|p| p256::PublicKey::from_public_key_pem(p).ok()).ok_or("Error: bad public key")?;
                p256::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine()).raw_secret_bytes().to_vec()
            } else {
                let sk = priv_pem.and_then(|p| p384::SecretKey::from_pkcs8_pem(p).ok()).ok_or("Error: bad private key")?;
                let pk = pub_pem.and_then(|p| p384::PublicKey::from_public_key_pem(p).ok()).ok_or("Error: bad public key")?;
                p384::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine()).raw_secret_bytes().to_vec()
            }
        }
        "x25519" => {
            let sraw = priv_pem.and_then(x25519_raw).ok_or("Error: bad private key")?;
            let praw = pub_pem.and_then(x25519_raw).ok_or("Error: bad public key")?;
            let sk = x25519_dalek::StaticSecret::from(sraw);
            let pk = x25519_dalek::PublicKey::from(praw);
            sk.diffie_hellman(&pk).as_bytes().to_vec()
        }
        _ => return Err("Error: diffieHellman requires EC or X25519 keys".into()),
    };
    Ok(super::buffer::from_bytes(&secret))
}

// ── Primes ──────────────────────────────────────────────────────────────

/// A key/number argument as a `BigUint` (BigInt, Buffer big-endian, or number).
fn arg_biguint(v: Option<&Value>) -> BigUint {
    let Some(v) = v else { return BigUint::from(0u32) };
    if let Some(b) = with_host(|h| match h.get(v) {
        Some(JsObj::BigInt(b)) => b.to_biguint(),
        _ => None,
    }) {
        return b;
    }
    if super::native_tag(v).as_deref() == Some("Buffer") {
        return BigUint::from_bytes_be(&val_bytes(v));
    }
    let n = with_host(|h| h.to_number(v));
    BigUint::from(n.max(0.0) as u64)
}

/// `checkPrimeSync(candidate)` — probabilistic primality.
fn check_prime(v: Option<&Value>) -> bool {
    let n = arg_biguint(v);
    num_prime::nt_funcs::is_prime(&n, None).probably()
}

/// `generatePrimeSync(size[, {bigint}])` — a random prime of `size` bits.
fn generate_prime(bits: usize, opts: Option<Value>) -> Result<Value, String> {
    let p = gen_prime(bits)?;
    let want_bigint = opts
        .map(|o| with_host(|h| matches!(h.get(&o), Some(JsObj::Object(m)) if m.get("bigint").map(|v| h.truthy(v)).unwrap_or(false))))
        .unwrap_or(false);
    if want_bigint {
        Ok(with_host(|h| h.alloc(JsObj::BigInt(num_bigint::BigInt::from(p)))))
    } else {
        // Node returns an ArrayBuffer; this runtime's ArrayBuffer carries no
        // backing bytes, so a Buffer (also a byte view) is returned instead.
        Ok(super::buffer::from_bytes(&p.to_bytes_be()))
    }
}

/// A random probable prime of `bits` bits (top bit set, odd, then next_prime).
fn gen_prime(bits: usize) -> Result<BigUint, String> {
    if bits < 2 {
        return Err("Error: size must be >= 2".into());
    }
    let nbytes = bits.div_ceil(8);
    let mut buf = vec![0u8; nbytes];
    getrandom::getrandom(&mut buf).map_err(|e| format!("Error: {e}"))?;
    let excess = nbytes * 8 - bits;
    buf[0] &= 0xffu8 >> excess;
    buf[0] |= 0x80u8 >> excess;
    let last = nbytes - 1;
    buf[last] |= 1;
    let start = BigUint::from_bytes_be(&buf);
    num_prime::nt_funcs::next_prime(&start, None).ok_or_else(|| "Error: prime generation failed".into())
}

// ── argon2 ──────────────────────────────────────────────────────────────

/// `argon2Sync(algorithm, options)` → raw tag bytes.
fn argon2_hash(algo: &str, opts: Option<&Value>) -> Result<Vec<u8>, String> {
    let o = opts.cloned().ok_or("Error: argon2 options required")?;
    let msg = prop_bytes(&o, "message");
    let salt = prop_bytes(&o, "nonce");
    let secret = prop_bytes(&o, "secret");
    let taglen = opt_num(&o, &["tagLength"], 32.0).max(4.0) as usize;
    let mem = opt_num(&o, &["memory"], 65536.0) as u32;
    let passes = opt_num(&o, &["passes"], 3.0) as u32;
    let par = opt_num(&o, &["parallelism"], 4.0) as u32;
    let algorithm = match algo.to_ascii_lowercase().as_str() {
        "argon2d" => argon2::Algorithm::Argon2d,
        "argon2i" => argon2::Algorithm::Argon2i,
        _ => argon2::Algorithm::Argon2id,
    };
    let params = argon2::Params::new(mem, passes, par, Some(taglen)).map_err(|e| format!("Error: {e}"))?;
    let ctx = if secret.is_empty() {
        argon2::Argon2::new(algorithm, argon2::Version::V0x13, params)
    } else {
        argon2::Argon2::new_with_secret(&secret, algorithm, argon2::Version::V0x13, params).map_err(|e| format!("Error: {e}"))?
    };
    let mut out = vec![0u8; taglen];
    ctx.hash_password_into(&msg, &salt, &mut out).map_err(|e| format!("Error: {e}"))?;
    Ok(out)
}

/// The bytes of a Buffer/typed-array/string property of an object.
fn prop_bytes(obj: &Value, key: &str) -> Vec<u8> {
    let v = with_host(|h| match h.get(obj) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    });
    v.map(|x| val_bytes(&x)).unwrap_or_default()
}

// ── WebCrypto getRandomValues ───────────────────────────────────────────

/// `getRandomValues(typedArray)` — fill in place with CSPRNG bytes, return it.
fn get_random_values(v: Option<&Value>) -> Result<Value, String> {
    let ta = v.cloned().ok_or("Error: argument must be an integer-type TypedArray")?;
    let tag = super::native_tag(&ta);
    if tag.as_deref() == Some("Buffer") {
        let len = val_bytes(&ta).len();
        let mut rnd = vec![0u8; len];
        getrandom::getrandom(&mut rnd).map_err(|e| format!("Error: {e}"))?;
        set_obj_bytes_named(&ta, "@@bytes", &rnd);
        return Ok(ta);
    }
    if tag.as_deref() != Some("TypedArray") {
        return Err("Error: argument must be an integer-type TypedArray".into());
    }
    let kind = obj_str(&ta, "@@kind");
    if kind.starts_with("Float") {
        return Err("Error: The provided ArrayBufferView is of type 'Float', which is not an integer array type".into());
    }
    let len = with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get(&ta) {
            if let Some(JsObj::Array(it)) = p.get("@@elems").and_then(|v| h.get(v)) {
                return it.len();
            }
        }
        0
    });
    let bpe = match kind.as_str() {
        "Int16Array" | "Uint16Array" => 2,
        "Int32Array" | "Uint32Array" => 4,
        _ => 1,
    };
    let mut raw = vec![0u8; len * bpe];
    getrandom::getrandom(&mut raw).map_err(|e| format!("Error: {e}"))?;
    let elems: Vec<Value> = (0..len)
        .map(|i| {
            let mut acc: u64 = 0;
            for j in 0..bpe {
                acc |= (raw[i * bpe + j] as u64) << (8 * j);
            }
            Value::Float(ta_coerce(&kind, acc))
        })
        .collect();
    with_host(|h| {
        let arr = match h.get(&ta) {
            Some(JsObj::Object(p)) => p.get("@@elems").cloned(),
            _ => None,
        };
        if let Some(a) = arr {
            if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                *items = elems;
            }
        }
    });
    Ok(ta)
}

/// Coerce a raw little-endian integer into a typed-array element value.
fn ta_coerce(kind: &str, raw: u64) -> f64 {
    match kind {
        "Int8Array" => (raw as i8) as f64,
        "Int16Array" => (raw as i16) as f64,
        "Int32Array" => (raw as i32) as f64,
        "Uint16Array" => (raw as u16) as f64,
        "Uint32Array" => (raw as u32) as f64,
        _ => (raw as u8) as f64, // Uint8Array / Uint8ClampedArray
    }
}

/// Store raw bytes into a named byte-array property (for Buffer `@@bytes`).
fn set_obj_bytes_named(recv: &Value, key: &str, bytes: &[u8]) {
    with_host(|h| {
        let arr = match h.get(recv) {
            Some(JsObj::Object(p)) => p.get(key).cloned(),
            _ => None,
        };
        if let Some(a) = arr {
            if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                *items = bytes.iter().map(|b| Value::Float(*b as f64)).collect();
            }
        }
    });
}

// ── KeyObject instance methods ──────────────────────────────────────────

/// `KeyObject` instance dispatch (`export({type,format})`, `equals`).
pub fn key_object_instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "export" => {
            // Secret key: raw bytes (Buffer) unless format 'jwk' (unsupported).
            let secret = obj_bytes(recv, "@@secret");
            if !secret.is_empty() {
                return Ok(super::buffer::from_bytes(&secret));
            }
            let pem = obj_str(recv, "@@pem");
            let format = args
                .first()
                .map(|o| opt_str(o, "format"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "pem".into());
            if format == "der" {
                Ok(super::buffer::from_bytes(&pem_body(&pem)))
            } else {
                Ok(with_host(|h| h.new_str(pem)))
            }
        }
        "equals" => {
            let other = args.first().map(key_material).unwrap_or_default();
            let mine = obj_str(recv, "@@pem").into_bytes();
            Ok(Value::Bool(mine == other))
        }
        _ => Err(crate::host::type_error(&format!("keyObject.{method} is not a function"))),
    }
}

// ── X.509 certificates ──────────────────────────────────────────────────

/// `new X509Certificate(pemOrDer)` → an `X509Certificate` instance.
pub fn construct_x509(args: &[Value]) -> Result<Value, String> {
    use x509_cert::der::{Decode, DecodePem, Encode};
    let bytes = val_bytes_at(args, 0);
    let is_pem = bytes.starts_with(b"-----BEGIN");
    let cert = if is_pem {
        x509_cert::Certificate::from_pem(&bytes).map_err(|e| format!("Error: {e}"))?
    } else {
        x509_cert::Certificate::from_der(&bytes).map_err(|e| format!("Error: {e}"))?
    };
    // The canonical DER (for fingerprint + raw).
    let der = cert.to_der().map_err(|e| format!("Error: {e}"))?;
    let fp = {
        let d = digest("sha1", &der);
        d.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
    };
    let subject = x509_name_node(&cert.tbs_certificate.subject.to_string());
    let issuer = x509_name_node(&cert.tbs_certificate.issuer.to_string());
    let not_before = x509_time(&cert.tbs_certificate.validity.not_before);
    let not_after = x509_time(&cert.tbs_certificate.validity.not_after);
    let serial = cert.tbs_certificate.serial_number.as_bytes().iter().map(|b| format!("{b:02X}")).collect::<String>();
    let spki_der = cert.tbs_certificate.subject_public_key_info.to_der().map_err(|e| format!("Error: {e}"))?;
    let pub_pem = pem_wrap("PUBLIC KEY", &spki_der);
    let pub_key = create_public_key(Some(&with_host(|h| h.new_str(pub_pem)))).unwrap_or(Value::Undef);
    let cert_pem = pem_wrap("CERTIFICATE", &der);
    let raw = super::buffer::from_bytes(&der);
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("X509Certificate"));
        m.insert("subject".into(), h.new_str(subject));
        m.insert("issuer".into(), h.new_str(issuer));
        m.insert("validFrom".into(), h.new_str(not_before));
        m.insert("validTo".into(), h.new_str(not_after));
        m.insert("serialNumber".into(), h.new_str(serial));
        m.insert("fingerprint".into(), h.new_str(fp));
        m.insert("publicKey".into(), pub_key);
        m.insert("raw".into(), raw);
        m.insert("@@pem".into(), h.new_str(cert_pem));
        h.new_object(m)
    }))
}

/// `X509Certificate` instance dispatch (`toString`, `toLegacyObject`).
pub fn x509_instance_call(recv: &Value, method: &str, _args: &[Value]) -> Result<Value, String> {
    match method {
        "toString" => Ok(with_host(|h| h.new_str(obj_str(recv, "@@pem")))),
        _ => Err(crate::host::type_error(&format!("x509Certificate.{method} is not a function"))),
    }
}

/// Convert an RFC 4514 name string ("O=Org,CN=Name") to Node's newline,
/// most-significant-last form ("CN=Name\nO=Org").
fn x509_name_node(rfc4514: &str) -> String {
    rfc4514.split(',').map(|s| s.trim()).rev().collect::<Vec<_>>().join("\n")
}

/// Render an X.509 time in OpenSSL's `%b %e %H:%M:%S %Y GMT` form.
fn x509_time(t: &x509_cert::time::Time) -> String {
    const MON: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let dt = t.to_date_time();
    let mon = MON.get(dt.month().saturating_sub(1) as usize).copied().unwrap_or("Jan");
    format!(
        "{} {:2} {:02}:{:02}:{:02} {} GMT",
        mon,
        dt.day(),
        dt.hour(),
        dt.minutes(),
        dt.seconds(),
        dt.year()
    )
}

// RFC 2409/3526 MODP group primes (generator 2).
const MODP1: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A63A3620FFFFFFFFFFFFFFFF";
const MODP2: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381FFFFFFFFFFFFFFFF";
const MODP5: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA237327FFFFFFFFFFFFFFFF";
const MODP14: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF";
const MODP15: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A93AD2CAFFFFFFFFFFFFFFFF";
const MODP16: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A92108011A723C12A787E6D788719A10BDBA5B2699C327186AF4E23C1A946834B6150BDA2583E9CA2AD44CE8DBBBC2DB04DE8EF92E8EFC141FBECAA6287C59474E6BC05D99B2964FA090C3A2233BA186515BE7ED1F612970CEE2D7AFB81BDD762170481CD0069127D5B05AA993B4EA988D8FDDC186FFB7DC90A6C08F4DF435C934063199FFFFFFFFFFFFFFFF";
const MODP17: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A92108011A723C12A787E6D788719A10BDBA5B2699C327186AF4E23C1A946834B6150BDA2583E9CA2AD44CE8DBBBC2DB04DE8EF92E8EFC141FBECAA6287C59474E6BC05D99B2964FA090C3A2233BA186515BE7ED1F612970CEE2D7AFB81BDD762170481CD0069127D5B05AA993B4EA988D8FDDC186FFB7DC90A6C08F4DF435C93402849236C3FAB4D27C7026C1D4DCB2602646DEC9751E763DBA37BDF8FF9406AD9E530EE5DB382F413001AEB06A53ED9027D831179727B0865A8918DA3EDBEBCF9B14ED44CE6CBACED4BB1BDB7F1447E6CC254B332051512BD7AF426FB8F401378CD2BF5983CA01C64B92ECF032EA15D1721D03F482D7CE6E74FEF6D55E702F46980C82B5A84031900B1C9E59E7C97FBEC7E8F323A97A7E36CC88BE0F1D45B7FF585AC54BD407B22B4154AACC8F6D7EBF48E1D814CC5ED20F8037E0A79715EEF29BE32806A1D58BB7C5DA76F550AA3D8A1FBFF0EB19CCB1A313D55CDA56C9EC2EF29632387FE8D76E3C0468043E8F663F4860EE12BF2D5B0B7474D6E694F91E6DCC4024FFFFFFFFFFFFFFFF";
const MODP18: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A92108011A723C12A787E6D788719A10BDBA5B2699C327186AF4E23C1A946834B6150BDA2583E9CA2AD44CE8DBBBC2DB04DE8EF92E8EFC141FBECAA6287C59474E6BC05D99B2964FA090C3A2233BA186515BE7ED1F612970CEE2D7AFB81BDD762170481CD0069127D5B05AA993B4EA988D8FDDC186FFB7DC90A6C08F4DF435C93402849236C3FAB4D27C7026C1D4DCB2602646DEC9751E763DBA37BDF8FF9406AD9E530EE5DB382F413001AEB06A53ED9027D831179727B0865A8918DA3EDBEBCF9B14ED44CE6CBACED4BB1BDB7F1447E6CC254B332051512BD7AF426FB8F401378CD2BF5983CA01C64B92ECF032EA15D1721D03F482D7CE6E74FEF6D55E702F46980C82B5A84031900B1C9E59E7C97FBEC7E8F323A97A7E36CC88BE0F1D45B7FF585AC54BD407B22B4154AACC8F6D7EBF48E1D814CC5ED20F8037E0A79715EEF29BE32806A1D58BB7C5DA76F550AA3D8A1FBFF0EB19CCB1A313D55CDA56C9EC2EF29632387FE8D76E3C0468043E8F663F4860EE12BF2D5B0B7474D6E694F91E6DBE115974A3926F12FEE5E438777CB6A932DF8CD8BEC4D073B931BA3BC832B68D9DD300741FA7BF8AFC47ED2576F6936BA424663AAB639C5AE4F5683423B4742BF1C978238F16CBE39D652DE3FDB8BEFC848AD922222E04A4037C0713EB57A81A23F0C73473FC646CEA306B4BCBC8862F8385DDFA9D4B7FA2C087E879683303ED5BDD3A062B3CF5B3A278A66D2A13F83F44F82DDF310EE074AB6A364597E899A0255DC164F31CC50846851DF9AB48195DED7EA1B1D510BD7EE74D73FAF36BC31ECFA268359046F4EB879F924009438B481C6CD7889A002ED5EE382BC9190DA6FC026E479558E4475677E9AA9E3050E2765694DFC81F56E880B96E7160C980DD98EDD3DFFFFFFFFFFFFFFFFF";

