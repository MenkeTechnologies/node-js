//! Node `querystring` module: `parse`/`stringify` (with the `escape`/`unescape`
//! aliases `encode`/`decode`). Values are percent-decoded/encoded with `+`
//! standing for a space, the legacy `application/x-www-form-urlencoded` rules
//! Node's `querystring` uses (distinct from the `qs` package express also ships).

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const METHODS: &[&str] = &[
    "parse",
    "stringify",
    "escape",
    "unescape",
    "encode",
    "decode",
    "unescapeBuffer",
];

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
        // `querystring.unescapeBuffer(str[, decodeSpaces])` → a Buffer of the raw
        // decoded bytes. `+` is decoded to a space only when `decodeSpaces` is true
        // (Node's default is false).
        "unescapeBuffer" => {
            let s = super::arg_str(args, 0);
            let decode_spaces = matches!(args.get(1), Some(Value::Bool(true)));
            Ok(super::buffer::from_bytes(&unescape_buffer(
                &s,
                decode_spaces,
            )))
        }
        _ => return None,
    })
}

/// `querystring.parse(str[, sep[, eq]])` → an object of decoded key/value pairs.
/// A repeated key collects its values into an array, matching Node.
///
/// An explicitly-passed `undefined` separator means "use the default", not the
/// STRING `"undefined"` — `body-parser` calls
/// `parse(body, undefined, undefined, { maxKeys })`, and coercing those to text
/// made the whole body one key.
fn parse(s: &str, args: &[Value]) -> Value {
    let sep = args
        .get(1)
        .filter(|v| !matches!(v, Value::Undef))
        .map(|_| super::arg_str(args, 1))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "&".into());
    let eq = args
        .get(2)
        .filter(|v| !matches!(v, Value::Undef))
        .map(|_| super::arg_str(args, 2))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "=".into());
    // `maxKeys` (options.maxKeys, default 1000; 0 means unlimited) caps how many
    // DISTINCT keys are kept. It was ignored entirely, so a hostile query string
    // could allocate without bound — which is the reason node has the cap.
    let max_keys = args
        .get(3)
        .filter(|v| !matches!(v, Value::Undef))
        .and_then(|o| crate::builtins::get_property(o, "maxKeys").ok())
        .filter(|v| !matches!(v, Value::Undef))
        .map(|v| with_host(|h| h.to_number(&v)))
        .filter(|n| n.is_finite() && *n >= 0.0)
        .map(|n| n as usize)
        .unwrap_or(1000);
    let mut map: IndexMap<String, Value> = IndexMap::new();
    if !s.is_empty() {
        for pair in s.split(&sep) {
            if pair.is_empty() {
                continue;
            }
            if max_keys != 0 && map.len() >= max_keys {
                break;
            }
            let (k, v) = match pair.split_once(&eq) {
                Some((k, v)) => (unescape_form(k), unescape_form(v)),
                None => (unescape_form(pair), String::new()),
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
    // The result has a NULL prototype, so a `__proto__` or `constructor` key in
    // the query string is an ordinary own property rather than a reference to
    // something inherited. It was inheriting `Object.prototype`.
    with_host(|h| {
        let obj = h.new_object(map);
        let null = h.null();
        h.set_proto(&obj, null);
        obj
    })
}

/// The serialized form of one `stringify` value.
///
/// Only a string, number, bigint or boolean has one; `null`, `undefined`, an
/// object and a symbol all serialize to the EMPTY string, which is why
/// `stringify({ a: null })` is `a=`. This used to run everything through
/// `String(v)`, so a null came out as the text "null" and an object as
/// "[object Object]" — both of which parse back as data.
fn stringify_value(v: &Value) -> String {
    with_host(|h| match v {
        Value::Bool(_) | Value::Int(_) | Value::Float(_) => h.str_of(v),
        Value::Str(_) => h.str_of(v),
        Value::Obj(_) => match h.get(v) {
            Some(JsObj::Str(_)) | Some(JsObj::BigInt(_)) => h.str_of(v),
            _ => String::new(),
        },
        _ => String::new(),
    })
}

/// `querystring.stringify(obj[, sep[, eq]])`.
fn stringify(args: &[Value]) -> Value {
    let obj = args.first().cloned().unwrap_or(Value::Undef);
    let sep = args
        .get(1)
        .filter(|v| !matches!(v, Value::Undef))
        .map(|_| super::arg_str(args, 1))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "&".into());
    let eq = args
        .get(2)
        .filter(|v| !matches!(v, Value::Undef))
        .map(|_| super::arg_str(args, 2))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "=".into());
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
            Some(JsObj::Array(items)) => Some(items.clone()),
            _ => None,
        });
        match elems {
            Some(list) => {
                // Each element goes through the same primitive-only rule as a
                // scalar value, so a null or object element is an empty string.
                for e in list {
                    parts.push(format!("{ek}{eq}{}", escape(&stringify_value(&e))));
                }
            }
            None => {
                parts.push(format!("{ek}{eq}{}", escape(&stringify_value(&v))));
            }
        }
    }
    with_host(|h| h.new_str(parts.join(&sep)))
}

/// `querystring.unescapeBuffer` core — decode `%XX` to raw bytes (and `+` to a
/// space when `decode_spaces`), leaving malformed escapes literal.
fn unescape_buffer(s: &str, decode_spaces: bool) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' if decode_spaces => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => {
                let hi = (b[i + 1] as char).to_digit(16);
                let lo = (b[i + 2] as char).to_digit(16);
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
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// `querystring.escape` — percent-encode (space → `%20`, like Node; NOT `+`).
fn escape(s: &str) -> String {
    const UNRESERVED: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if UNRESERVED.contains(&b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(
                char::from_digit((b >> 4) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
            out.push(
                char::from_digit((b & 0xf) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
        }
    }
    out
}

/// Reverse `escape` (`+` → space, `%XX` → byte). Malformed escapes pass through
/// literally, as Node's `querystring.unescape` does (it never throws).
/// `querystring.unescape(str)` — percent-decoding only.
///
/// A `+` stays a `+`. Only `parse` treats it as a space, because that is a
/// form-encoding rule about the pair syntax, not about percent-escapes; this
/// decoded it too, so `querystring.unescape('a+b')` gave `'a b'`.
fn unescape(s: &str) -> String {
    unescape_inner(s, false)
}

/// The parse-side decoder, which DOES read `+` as a space.
fn unescape_form(s: &str) -> String {
    unescape_inner(s, true)
}

fn unescape_inner(s: &str, plus_is_space: bool) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' if plus_is_space => {
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
