//! Node `punycode` module — a faithful implementation of the RFC 3492 Bootstring
//! algorithm with the Punycode parameter set. The module is deprecated in Node
//! but still present; the codec is pure and deterministic (no host state beyond
//! allocating the returned string/array), so it round-trips independently of any
//! network or locale.
//!
//! Surface:
//!   * `encode(str)` / `decode(str)` — the raw label codec (no `xn--` prefix).
//!   * `toASCII(domain)` / `toUnicode(domain)` — per dot-separated label, with the
//!     `xn--` ACE prefix convention.
//!   * `ucs2Decode`/`ucs2Encode` — the code-point split/join. Node exposes these as
//!     `punycode.ucs2.decode` / `.encode`; that nested object is built by
//!     `constant("ucs2")`.
//!
//! Bootstring parameters (RFC 3492 §5): base 36, tmin 1, tmax 26, skew 38,
//! damp 700, initial_bias 72, initial_n 128, delimiter `-`.
//!
//! RFC-3492 sample verifications reasoned through against this implementation.
//! For "mañana": basic run "maana" (b=5) → "maana-"; the single non-basic ñ
//! (U+00F1=241) with delta=678 emits digits 15→'p', 19→'t', 0→'a' ⇒ "maana-pta".
//! Hence toASCII("mañana.com") === "xn--maana-pta.com" and the inverse
//! toUnicode round-trips. Likewise toASCII("bücher") === "xn--bcher-kva".

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

// ── Bootstring parameters ────────────────────────────────────────────────────
const BASE: u32 = 36;
const TMIN: u32 = 1;
const TMAX: u32 = 26;
const SKEW: u32 = 38;
const DAMP: u32 = 700;
const INITIAL_BIAS: u32 = 72;
const INITIAL_N: u32 = 128;

/// The dot separators Node accepts (ASCII `.` plus the ideographic/full-width/
/// halfwidth dots); all normalize to `.` in the output.
const DOTS: &[char] = &['\u{2E}', '\u{3002}', '\u{FF0E}', '\u{FF61}'];

pub const METHODS: &[&str] = &[
    "encode",
    "decode",
    "toASCII",
    "toUnicode",
    "ucs2Decode",
    "ucs2Encode",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let input = super::arg_str(args, 0);
    Some(match method {
        "encode" => encode_js(&input),
        "decode" => decode_js(&input),
        "toASCII" => Ok(with_host(|h| h.new_str(to_ascii(&input)))),
        "toUnicode" => Ok(with_host(|h| h.new_str(to_unicode(&input)))),
        "ucs2Decode" => Ok(ucs2_decode_val(&input)),
        "ucs2Encode" => Ok(ucs2_encode_val(args.first())),
        _ => return None,
    })
}

/// `punycode.ucs2` nested object and `punycode.version`, served through
/// `stdlib::constant` (needs the parent `"punycode" => punycode::constant(name)`
/// arm). Its `decode`/`encode` are the routed `ucs2Decode`/`ucs2Encode` methods.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "ucs2" => Some(with_host(|h| {
            let mut m = IndexMap::new();
            m.insert(
                "decode".into(),
                h.alloc(JsObj::Builtin("punycode.ucs2Decode".into())),
            );
            m.insert(
                "encode".into(),
                h.alloc(JsObj::Builtin("punycode.ucs2Encode".into())),
            );
            h.new_object(m)
        })),
        "version" => Some(with_host(|h| h.new_str("2.3.1"))),
        _ => None,
    }
}

// ── JS-facing wrappers ───────────────────────────────────────────────────────

fn encode_js(s: &str) -> Result<Value, String> {
    let cps: Vec<u32> = s.chars().map(|c| c as u32).collect();
    match encode(&cps) {
        Ok(out) => Ok(with_host(|h| h.new_str(out))),
        Err(e) => Err(crate::host::range_error(&e)),
    }
}

fn decode_js(s: &str) -> Result<Value, String> {
    match decode(s) {
        Ok(cps) => {
            let out: String = cps.iter().filter_map(|&c| char::from_u32(c)).collect();
            Ok(with_host(|h| h.new_str(out)))
        }
        Err(e) => Err(crate::host::range_error(&e)),
    }
}

/// `punycode.ucs2.decode(str)` → array of code-point numbers. node-js strings are
/// Rust `String` (full Unicode scalars), so `chars()` already yields code points.
fn ucs2_decode_val(s: &str) -> Value {
    with_host(|h| {
        let items: Vec<Value> = s.chars().map(|c| Value::Float(c as u32 as f64)).collect();
        h.new_array(items)
    })
}

/// `punycode.ucs2.encode(codePoints)` → string.
fn ucs2_encode_val(arg: Option<&Value>) -> Value {
    let cps: Vec<u32> = match arg {
        Some(v) => with_host(|h| match h.get(v) {
            Some(JsObj::Array(items)) => items.iter().map(|x| h.to_number(x) as u32).collect(),
            _ => Vec::new(),
        }),
        None => Vec::new(),
    };
    let out: String = cps.iter().filter_map(|&c| char::from_u32(c)).collect();
    with_host(|h| h.new_str(out))
}

// ── domain-level ToASCII / ToUnicode ─────────────────────────────────────────

fn map_labels(domain: &str, f: impl Fn(&str) -> String) -> String {
    domain
        .split(|c| DOTS.contains(&c))
        .map(f)
        .collect::<Vec<_>>()
        .join(".")
}

fn to_ascii(domain: &str) -> String {
    map_labels(domain, |label| {
        // Only labels carrying non-ASCII get the `xn--` ACE form.
        if label.chars().any(|c| (c as u32) >= 0x80) {
            let cps: Vec<u32> = label.chars().map(|c| c as u32).collect();
            match encode(&cps) {
                Ok(enc) => format!("xn--{enc}"),
                Err(_) => label.to_string(),
            }
        } else {
            label.to_string()
        }
    })
}

fn to_unicode(domain: &str) -> String {
    map_labels(domain, |label| {
        // Case-insensitive `xn--` prefix ⇒ decode the remainder.
        let lower = label.to_lowercase();
        match lower.strip_prefix("xn--") {
            Some(rest) => match decode(rest) {
                Ok(cps) => cps.iter().filter_map(|&c| char::from_u32(c)).collect(),
                Err(_) => label.to_string(),
            },
            None => label.to_string(),
        }
    })
}

// ── core Bootstring codec (RFC 3492) ─────────────────────────────────────────

/// Bias adaptation (RFC 3492 §6.1).
fn adapt(mut delta: u32, num_points: u32, first_time: bool) -> u32 {
    delta = if first_time { delta / DAMP } else { delta / 2 };
    delta += delta / num_points;
    let mut k = 0;
    while delta > ((BASE - TMIN) * TMAX) / 2 {
        delta /= BASE - TMIN;
        k += BASE;
    }
    k + (((BASE - TMIN + 1) * delta) / (delta + SKEW))
}

/// A Bootstring digit (0..36) → its basic code point: 0..25 → 'a'..'z',
/// 26..35 → '0'..'9'.
fn digit_to_basic(d: u32) -> char {
    if d < 26 {
        (b'a' + d as u8) as char
    } else {
        (b'0' + (d - 26) as u8) as char
    }
}

/// A basic code point → its Bootstring digit value (case-insensitive letters).
fn basic_to_digit(c: char) -> Result<u32, String> {
    match c {
        'a'..='z' => Ok(c as u32 - 'a' as u32),
        'A'..='Z' => Ok(c as u32 - 'A' as u32),
        '0'..='9' => Ok(c as u32 - '0' as u32 + 26),
        _ => Err(format!("Invalid input: {c}")),
    }
}

/// Encode a slice of Unicode code points into a Punycode string (no `xn--`).
fn encode(input: &[u32]) -> Result<String, String> {
    let mut output = String::new();

    // 1. Copy all basic (ASCII) code points, in order, to the output.
    let mut b: u32 = 0;
    for &c in input {
        if c < 0x80 {
            output.push(c as u8 as char);
            b += 1;
        }
    }
    // 2. Delimiter after the basic run (only if there were basic code points).
    let mut h = b;
    if b > 0 {
        output.push('-');
    }

    let input_len = input.len() as u32;
    let mut n = INITIAL_N;
    let mut delta: u32 = 0;
    let mut bias = INITIAL_BIAS;

    while h < input_len {
        // 3. Smallest code point >= n not yet handled.
        let mut m = u32::MAX;
        for &c in input {
            if c >= n && c < m {
                m = c;
            }
        }
        // delta += (m - n) * (h + 1); guarded against overflow.
        delta = delta
            .checked_add((m - n).checked_mul(h + 1).ok_or("overflow")?)
            .ok_or("overflow")?;
        n = m;

        for &c in input {
            if c < n {
                delta = delta.checked_add(1).ok_or("overflow")?;
            }
            if c == n {
                // Represent delta as a generalized variable-length integer.
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = threshold(k, bias);
                    if q < t {
                        break;
                    }
                    let digit = t + ((q - t) % (BASE - t));
                    output.push(digit_to_basic(digit));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                output.push(digit_to_basic(q));
                bias = adapt(delta, h + 1, h == b);
                delta = 0;
                h += 1;
            }
        }
        delta += 1;
        n += 1;
    }
    Ok(output)
}

/// Decode a Punycode string (no `xn--`) into Unicode code points.
fn decode(input: &str) -> Result<Vec<u32>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut output: Vec<u32> = Vec::new();

    // 1. Consume basic code points before the last delimiter (if any).
    let mut idx = match input.rfind('-') {
        Some(pos) => {
            // `pos` is a byte index; it equals the char index here because every
            // char up to and including the delimiter is ASCII (1 byte).
            for &c in &chars[..pos] {
                if (c as u32) >= 0x80 {
                    return Err("Illegal basic code point".into());
                }
                output.push(c as u32);
            }
            pos + 1
        }
        None => 0,
    };

    let mut n = INITIAL_N;
    let mut i: u32 = 0;
    let mut bias = INITIAL_BIAS;
    let len = chars.len();

    while idx < len {
        let oldi = i;
        let mut w: u32 = 1;
        let mut k = BASE;
        loop {
            if idx >= len {
                return Err("Invalid input".into());
            }
            let digit = basic_to_digit(chars[idx])?;
            idx += 1;
            i = i
                .checked_add(digit.checked_mul(w).ok_or("overflow")?)
                .ok_or("overflow")?;
            let t = threshold(k, bias);
            if digit < t {
                break;
            }
            w = w.checked_mul(BASE - t).ok_or("overflow")?;
            k += BASE;
        }
        let out_len = output.len() as u32 + 1;
        bias = adapt(i - oldi, out_len, oldi == 0);
        n = n.checked_add(i / out_len).ok_or("overflow")?;
        i %= out_len;
        // Insert code point n at position i.
        output.insert(i as usize, n);
        i += 1;
    }
    Ok(output)
}

/// Per-position threshold `t(k)` (RFC 3492): clamped to `[tmin, tmax]` around the
/// current bias.
fn threshold(k: u32, bias: u32) -> u32 {
    if k <= bias {
        TMIN
    } else if k >= bias + TMAX {
        TMAX
    } else {
        k - bias
    }
}
