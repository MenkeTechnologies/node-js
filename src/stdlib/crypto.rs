//! Node `crypto` module: `createHash(algo)` → a `Hash` instance
//! (`update(...).digest(enc)`), backed by the `md-5`/`sha1`/`sha2` crates.
//! The `Hash` is a plain object tagged `@@native = "Hash"` accumulating input in
//! a hidden `@@data` byte array until `digest` finalizes it.

use super::{arg_str, to_base64, to_hex};
use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use md5::{Digest as _, Md5};
use sha1::Sha1;
use sha2::{Sha256, Sha512};

pub const METHODS: &[&str] = &["createHash"];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "createHash" => {
            let algo = arg_str(args, 0).to_ascii_lowercase();
            if !matches!(algo.as_str(), "md5" | "sha1" | "sha256" | "sha512") {
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
        _ => return None,
    })
}

/// `Hash` instance methods: `update` (chainable) and `digest`.
pub fn instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
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
            let (algo, data) = with_host(|h| {
                let (mut algo, mut data) = (String::new(), Vec::new());
                if let Some(JsObj::Object(p)) = h.get(recv) {
                    algo = p.get("@@algo").map(|v| h.str_of(v)).unwrap_or_default();
                    if let Some(JsObj::Array(items)) = p.get("@@data").and_then(|v| h.get(v)) {
                        data = items.iter().map(|v| h.to_number(v) as u8).collect();
                    }
                }
                (algo, data)
            });
            let digest = digest(&algo, &data);
            let enc = if args.is_empty() { None } else { Some(arg_str(args, 0)) };
            Ok(match enc.as_deref() {
                Some("hex") => with_host(|h| h.new_str(to_hex(&digest))),
                Some("base64") | Some("base64url") => with_host(|h| h.new_str(to_base64(&digest))),
                Some("latin1") | Some("binary") => with_host(|h| h.new_str(digest.iter().map(|b| *b as char).collect::<String>())),
                _ => super::buffer::from_bytes(&digest),
            })
        }
        _ => Err(crate::host::type_error(&format!("hash.{method} is not a function"))),
    }
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

fn decode(s: &str, enc: &str) -> Vec<u8> {
    match enc.to_ascii_lowercase().as_str() {
        "hex" => super::from_hex(s),
        "base64" | "base64url" => super::from_base64(s),
        "ascii" | "latin1" | "binary" => s.chars().map(|c| c as u8).collect(),
        _ => s.as_bytes().to_vec(),
    }
}
