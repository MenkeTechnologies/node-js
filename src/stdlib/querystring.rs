//! Node `querystring` module, a port of `lib/querystring.js`: `parse`/`stringify`
//! (and their `decode`/`encode` aliases, which are the same functions), `escape`,
//! `unescape` and `unescapeBuffer`. The legacy
//! `application/x-www-form-urlencoded` rules, with `+` standing for a space —
//! distinct from the `qs` package express also ships.
//!
//! `parse` is node's single-pass scanner over UTF-16 code units, so multi-unit
//! separators, a `maxKeys` budget that counts PAIRS (empty ones included), the
//! "decode only if an escape was seen" rule and a custom `decodeURIComponent`
//! all behave as in node.

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
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Undef);
    Some(match method {
        "parse" | "decode" => parse(&arg(0), &arg(1), &arg(2), &arg(3)),
        "stringify" | "encode" => stringify(&arg(0), &arg(1), &arg(2), &arg(3)),
        "escape" => escape_value(&arg(0)).map(|s| with_host(|h| h.new_str(s))),
        "unescape" => {
            let decode_spaces = with_host(|h| h.truthy(&arg(1)));
            unescape(&arg(0), decode_spaces).map(|s| with_host(|h| h.new_str(s)))
        }
        "unescapeBuffer" => {
            let s = js_string(&arg(0));
            let decode_spaces = with_host(|h| h.truthy(&arg(1)));
            Ok(super::buffer::from_bytes(&unescape_buffer(
                &s,
                decode_spaces,
            )))
        }
        _ => return None,
    })
}

fn str_of(v: &Value) -> String {
    with_host(|h| h.str_of(v))
}

fn new_str(s: String) -> Value {
    with_host(|h| h.new_str(s))
}

/// `String(v)`: `ToString`, except that a symbol renders as `Symbol(desc)`.
fn js_string(v: &Value) -> String {
    match crate::host::to_string_value(v) {
        Ok(s) => str_of(&s),
        Err(_) => str_of(v),
    }
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn from_utf16(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
}

/// A callable member of an options bag, or `None`.
fn option_fn(options: &Value, name: &str) -> Result<Option<Value>, String> {
    if !with_host(|h| h.truthy(options)) {
        return Ok(None);
    }
    let f = crate::builtins::get_property(options, name)?;
    Ok(with_host(|h| crate::host::is_callable(h, &f)).then_some(f))
}

// ── escape / unescape ────────────────────────────────────────────────────────

/// `qsEscape`: an object is `String()`ed (its `toString` runs), any other
/// non-string is `+ ''`ed (a symbol throws), then every byte outside
/// `A-Za-z0-9-._~!'()*` of the UTF-8 is `%XX`.
fn escape_value(v: &Value) -> Result<String, String> {
    let s = crate::host::to_string_value(v)?;
    Ok(escape(&str_of(&s)))
}

fn escape(s: &str) -> String {
    const UNRESERVED: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if UNRESERVED.contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `qsUnescape(s, decodeSpaces)`: `decodeURIComponent(s)`, and when that
/// throws (a malformed or non-UTF-8 escape), `unescapeBuffer(s).toString()`.
fn unescape(v: &Value, decode_spaces: bool) -> Result<String, String> {
    match crate::builtins::call_builtin_function("decodeURIComponent", vec![v.clone()]) {
        Ok(d) => Ok(str_of(&d)),
        Err(e) if e.starts_with("URIError") => {
            with_host(|h| h.exc = None);
            let bytes = unescape_buffer(&js_string(v), decode_spaces);
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        Err(e) => Err(e),
    }
}

/// `querystring.unescapeBuffer` — each UTF-16 code unit is one output byte
/// (truncated, as node's `out[i] = charCode` stores it), `%XX` decodes to its
/// byte, and `+` is a space only when `decode_spaces`. A malformed escape stays
/// literal.
fn unescape_buffer(s: &str, decode_spaces: bool) -> Vec<u8> {
    let u = utf16(s);
    let hex = |c: u16| char::from_u32(c as u32).and_then(|c| c.to_digit(16));
    let mut out = Vec::with_capacity(u.len());
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        if c == u16::from(b'+') && decode_spaces {
            out.push(b' ');
            i += 1;
            continue;
        }
        if c == u16::from(b'%') && i + 2 < u.len() {
            if let (Some(hi), Some(lo)) = (hex(u[i + 1]), hex(u[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(c as u8);
        i += 1;
    }
    out
}

// ── parse ────────────────────────────────────────────────────────────────────

/// `addKeyVal`: decode a part only when an escape was seen in it (or the
/// decoder is custom), then set it, promoting a repeated key to an array.
fn add_key_val(
    obj: &mut IndexMap<String, Value>,
    key: &[u16],
    value: &[u16],
    encoded: (bool, bool),
    decode: &Option<Value>,
) -> Result<(), String> {
    let decode_part = |part: &[u16], enc: bool| -> Result<Value, String> {
        let s = new_str(from_utf16(part));
        if part.is_empty() || !enc {
            return Ok(s);
        }
        let decoded = match decode {
            Some(f) => crate::host::invoke(f, vec![s.clone()], None),
            None => unescape(&s, false).map(new_str),
        };
        // `decodeStr`: a throwing decoder falls back to `unescape(s, true)`,
        // and the exception it raised is caught, so its value is discarded.
        decoded.or_else(|_| {
            with_host(|h| h.exc = None);
            unescape(&s, true).map(new_str)
        })
    };
    let k = decode_part(key, encoded.0)?;
    let k = str_of(&crate::host::to_string_value(&k)?);
    let v = decode_part(value, encoded.1)?;
    match obj.get(&k).cloned() {
        None => {
            obj.insert(k, v);
        }
        Some(cur) => {
            let pushed = with_host(|h| match h.get_mut(&cur) {
                Some(JsObj::Array(items)) => {
                    items.push(v.clone());
                    true
                }
                _ => false,
            });
            if !pushed {
                let arr = with_host(|h| h.new_array(vec![cur, v]));
                obj.insert(k, arr);
            }
        }
    }
    Ok(())
}

/// `querystring.parse(str[, sep[, eq[, options]]])` → a null-prototype object.
/// A falsy `sep`/`eq` is the default; `options.maxKeys` (default 1000, `<= 0`
/// unlimited) budgets PAIRS, and `options.decodeURIComponent` replaces the
/// decoder.
fn parse(qs: &Value, sep: &Value, eq: &Value, options: &Value) -> Result<Value, String> {
    let mut obj: IndexMap<String, Value> = IndexMap::new();
    if let Some(qs) = with_host(|h| h.as_str(qs)).filter(|s| !s.is_empty()) {
        parse_into(&mut obj, &utf16(&qs), sep, eq, options)?;
    }
    // A NULL prototype, so `__proto__` or `constructor` in the input is an
    // ordinary own property.
    Ok(with_host(|h| {
        let o = h.new_object(obj);
        let null = h.null();
        h.set_proto(&o, null);
        o
    }))
}

fn parse_into(
    obj: &mut IndexMap<String, Value>,
    qs: &[u16],
    sep: &Value,
    eq: &Value,
    options: &Value,
) -> Result<(), String> {
    let codes = |v: &Value, default: &str| {
        if with_host(|h| h.truthy(v)) {
            utf16(&js_string(v))
        } else {
            utf16(default)
        }
    };
    let (sep_codes, eq_codes) = (codes(sep, "&"), codes(eq, "="));
    let (sep_len, eq_len) = (sep_codes.len(), eq_codes.len());
    let mut pairs = 1000.0;
    if with_host(|h| h.truthy(options)) {
        let max = crate::builtins::get_property(options, "maxKeys")?;
        if with_host(|h| h.type_of(&max)) == "number" {
            let n = with_host(|h| h.to_number(&max));
            pairs = if n > 0.0 { n } else { -1.0 };
        }
    }
    let decode = option_fn(options, "decodeURIComponent")?;
    let custom = decode.is_some();
    let plus: Vec<u16> = utf16(if custom { "%20" } else { " " });
    let is_hex = |c: u16| (c as u32) < 128 && (c as u8 as char).is_ascii_hexdigit();
    let (mut last_pos, mut sep_idx, mut eq_idx) = (0usize, 0usize, 0usize);
    let (mut key, mut value): (Vec<u16>, Vec<u16>) = (Vec::new(), Vec::new());
    let (mut key_enc, mut val_enc) = (custom, custom);
    let mut encode_check = 0;
    let mut i = 0;
    while i < qs.len() {
        let code = qs[i];
        if Some(&code) == sep_codes.get(sep_idx) {
            sep_idx += 1;
            if sep_idx == sep_len {
                let end = i + 1 - sep_idx;
                if eq_idx < eq_len {
                    if last_pos < end {
                        key.extend_from_slice(&qs[last_pos..end]);
                    } else if key.is_empty() {
                        pairs -= 1.0;
                        if pairs == 0.0 {
                            return Ok(());
                        }
                        last_pos = i + 1;
                        sep_idx = 0;
                        eq_idx = 0;
                        i += 1;
                        continue;
                    }
                } else if last_pos < end {
                    value.extend_from_slice(&qs[last_pos..end]);
                }
                add_key_val(obj, &key, &value, (key_enc, val_enc), &decode)?;
                pairs -= 1.0;
                if pairs == 0.0 {
                    return Ok(());
                }
                key_enc = custom;
                val_enc = custom;
                key.clear();
                value.clear();
                encode_check = 0;
                last_pos = i + 1;
                sep_idx = 0;
                eq_idx = 0;
            }
        } else {
            sep_idx = 0;
            if eq_idx < eq_len {
                if Some(&code) == eq_codes.get(eq_idx) {
                    eq_idx += 1;
                    if eq_idx == eq_len {
                        let end = i + 1 - eq_idx;
                        if last_pos < end {
                            key.extend_from_slice(&qs[last_pos..end]);
                        }
                        encode_check = 0;
                        last_pos = i + 1;
                    }
                    i += 1;
                    continue;
                }
                eq_idx = 0;
                if !key_enc {
                    if code == u16::from(b'%') {
                        encode_check = 1;
                        i += 1;
                        continue;
                    } else if encode_check > 0 {
                        if is_hex(code) {
                            encode_check += 1;
                            if encode_check == 3 {
                                key_enc = true;
                            }
                            i += 1;
                            continue;
                        }
                        encode_check = 0;
                    }
                }
                if code == u16::from(b'+') {
                    if last_pos < i {
                        key.extend_from_slice(&qs[last_pos..i]);
                    }
                    key.extend_from_slice(&plus);
                    last_pos = i + 1;
                }
                i += 1;
                continue;
            }
            if code == u16::from(b'+') {
                if last_pos < i {
                    value.extend_from_slice(&qs[last_pos..i]);
                }
                value.extend_from_slice(&plus);
                last_pos = i + 1;
            } else if !val_enc {
                if code == u16::from(b'%') {
                    encode_check = 1;
                } else if encode_check > 0 {
                    if is_hex(code) {
                        encode_check += 1;
                        if encode_check == 3 {
                            val_enc = true;
                        }
                    } else {
                        encode_check = 0;
                    }
                }
            }
        }
        i += 1;
    }
    if last_pos < qs.len() {
        if eq_idx < eq_len {
            key.extend_from_slice(&qs[last_pos..]);
        } else if sep_idx < sep_len {
            value.extend_from_slice(&qs[last_pos..]);
        }
    } else if eq_idx == 0 && key.is_empty() {
        return Ok(());
    }
    add_key_val(obj, &key, &value, (key_enc, val_enc), &decode)
}

// ── stringify ────────────────────────────────────────────────────────────────

/// `stringifyPrimitive`: the text a value contributes — a string itself, a
/// finite number, a bigint, a boolean; everything else (`null`, `NaN`, an
/// object, a symbol) is empty.
fn primitive_text(v: &Value) -> String {
    match with_host(|h| h.type_of(v)) {
        "string" | "bigint" | "boolean" => str_of(v),
        "number" if with_host(|h| h.to_number(v)).is_finite() => str_of(v),
        _ => String::new(),
    }
}

/// One key or value as `stringify` writes it: with the default encoder,
/// `encodeStringified` (a number below `1e21` is written bare, a larger one
/// encoded for its `+`); with a custom one, `encode(stringifyPrimitive(v))`.
fn convert(v: &Value, encode: &Option<Value>) -> Result<String, String> {
    let text = primitive_text(v);
    match encode {
        Some(f) => {
            let r = crate::host::invoke(f, vec![new_str(text)], None)?;
            Ok(str_of(&crate::host::to_string_value(&r)?))
        }
        None => {
            let ty = with_host(|h| h.type_of(v));
            Ok(match ty {
                "string" => escape(&text),
                "number" if with_host(|h| h.to_number(v)).abs() >= 1e21 => escape(&text),
                _ => text,
            })
        }
    }
}

/// `querystring.stringify(obj[, sep[, eq[, options]]])`: `Object.keys(obj)` in
/// order, each value read with `[[Get]]`, an array value one pair per element
/// (none for an empty array). A non-object is `''`.
fn stringify(obj: &Value, sep: &Value, eq: &Value, options: &Value) -> Result<Value, String> {
    let part = |v: &Value, default: &str| -> Result<String, String> {
        if with_host(|h| h.truthy(v)) {
            Ok(str_of(&crate::host::to_string_value(v)?))
        } else {
            Ok(default.to_string())
        }
    };
    let (sep, eq) = (part(sep, "&")?, part(eq, "=")?);
    let encode = option_fn(options, "encodeURIComponent")?;
    if with_host(|h| h.type_of(obj)) != "object" || with_host(|h| h.is_null(obj)) {
        return Ok(new_str(String::new()));
    }
    let keys = crate::builtins::call_builtin_function("Object.keys", vec![obj.clone()])?;
    let keys = with_host(|h| match h.get(&keys) {
        Some(JsObj::Array(items)) => items.clone(),
        _ => Vec::new(),
    });
    let mut fields = String::new();
    for k in keys {
        let v = crate::builtins::get_property(obj, &str_of(&k))?;
        let ks = convert(&k, &encode)? + &eq;
        let elems = with_host(|h| match h.get(&v) {
            Some(JsObj::Array(items)) => Some(items.clone()),
            _ => None,
        });
        match elems {
            Some(list) if list.is_empty() => continue,
            Some(list) => {
                if !fields.is_empty() {
                    fields += &sep;
                }
                for (j, e) in list.iter().enumerate() {
                    if j > 0 {
                        fields += &sep;
                    }
                    fields += &ks;
                    fields += &convert(e, &encode)?;
                }
            }
            None => {
                if !fields.is_empty() {
                    fields += &sep;
                }
                fields += &ks;
                fields += &convert(&v, &encode)?;
            }
        }
    }
    Ok(new_str(fields))
}
