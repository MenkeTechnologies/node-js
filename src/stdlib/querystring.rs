//! Node `querystring` module: `parse`/`stringify` (with the `escape`/`unescape`
//! aliases `encode`/`decode`). Values are percent-decoded/encoded with `+`
//! standing for a space, the legacy `application/x-www-form-urlencoded` rules
//! Node's `querystring` uses (distinct from the `qs` package express also ships).

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const METHODS: &[&str] = &["parse", "stringify", "escape", "unescape", "encode", "decode"];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "parse" | "decode" => Ok(parse(&super::arg_str(args, 0), args)),
        "stringify" | "encode" => Ok(stringify(args)),
        "escape" => {
            // arg_str borrows the host; compute it BEFORE the new_str with_host.
            let s = super::arg_str(args, 0);
            Ok(with_host(|h| h.new_str(escape(&s))))
        }
        "unescape" => {
            let s = super::arg_str(args, 0);
            Ok(with_host(|h| h.new_str(unescape(&s))))
        }
        _ => return None,
    })
}

/// `querystring.parse(str[, sep[, eq]])` → an object of decoded key/value pairs.
/// A repeated key collects its values into an array, matching Node.
fn parse(s: &str, args: &[Value]) -> Value {
    let sep = args.get(1).map(|_| super::arg_str(args, 1)).filter(|s| !s.is_empty()).unwrap_or_else(|| "&".into());
    let eq = args.get(2).map(|_| super::arg_str(args, 2)).filter(|s| !s.is_empty()).unwrap_or_else(|| "=".into());
    let mut map: IndexMap<String, Value> = IndexMap::new();
    if !s.is_empty() {
        for pair in s.split(&sep) {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = match pair.split_once(&eq) {
                Some((k, v)) => (unescape(k), unescape(v)),
                None => (unescape(pair), String::new()),
            };
            let val = with_host(|h| h.new_str(v));
            // A repeated key promotes to (and then extends) an array.
            match map.get(&k).cloned() {
                Some(existing) => {
                    let is_arr = with_host(|h| matches!(h.get(&existing), Some(JsObj::Array(_))));
                    if is_arr {
                        with_host(|h| {
                            if let Some(JsObj::Array(items)) = h.get_mut(&existing) {
                                items.push(val);
                            }
                        });
                    } else {
                        let arr = with_host(|h| h.new_array(vec![existing, val]));
                        map.insert(k, arr);
                    }
                }
                None => {
                    map.insert(k, val);
                }
            }
        }
    }
    with_host(|h| h.new_object(map))
}

/// `querystring.stringify(obj[, sep[, eq]])`.
fn stringify(args: &[Value]) -> Value {
    let obj = args.first().cloned().unwrap_or(Value::Undef);
    let sep = args.get(1).map(|_| super::arg_str(args, 1)).filter(|s| !s.is_empty()).unwrap_or_else(|| "&".into());
    let eq = args.get(2).map(|_| super::arg_str(args, 2)).filter(|s| !s.is_empty()).unwrap_or_else(|| "=".into());
    let entries = with_host(|h| match h.get(&obj) {
        Some(JsObj::Object(p)) => p
            .iter()
            .filter(|(k, _)| !k.starts_with("@@"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    });
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in entries {
        let ek = escape(&k);
        // An array value emits one `key=elem` pair per element.
        let elems = with_host(|h| match h.get(&v) {
            Some(JsObj::Array(items)) => Some(items.iter().map(|x| h.str_of(x)).collect::<Vec<_>>()),
            _ => None,
        });
        match elems {
            Some(list) => {
                for e in list {
                    parts.push(format!("{ek}{eq}{}", escape(&e)));
                }
            }
            None => {
                let ev = with_host(|h| h.str_of(&v));
                parts.push(format!("{ek}{eq}{}", escape(&ev)));
            }
        }
    }
    with_host(|h| h.new_str(parts.join(&sep)))
}

/// `querystring.escape` — percent-encode (space → `%20`, like Node; NOT `+`).
fn escape(s: &str) -> String {
    const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if UNRESERVED.contains(&b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(char::from_digit((b >> 4) as u32, 16).unwrap().to_ascii_uppercase());
            out.push(char::from_digit((b & 0xf) as u32, 16).unwrap().to_ascii_uppercase());
        }
    }
    out
}

/// Reverse `escape` (`+` → space, `%XX` → byte). Malformed escapes pass through
/// literally, as Node's `querystring.unescape` does (it never throws).
fn unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
