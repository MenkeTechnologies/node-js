//! `Number.prototype.toLocaleString` / `BigInt.prototype.toLocaleString` with
//! their `options` argument, in the `en-US` shape.
//!
//! This is the ECMA-402 `Intl.NumberFormat` pipeline restricted to what needs no
//! locale data beyond en-US: the digit options (`minimumIntegerDigits`,
//! `minimum`/`maximumFractionDigits`, `minimum`/`maximumSignificantDigits`),
//! `useGrouping`, and the `decimal`/`percent`/`currency` styles. The `locales`
//! argument is still ignored (there is no ICU here, so every locale formats as
//! en-US — see BUGS.md). `signDisplay` and `currencyDisplay: "code"` are
//! honored; `notation`, the `name`/`narrowSymbol` currency displays and the
//! `unit` style's unit label are not modelled.
//!
//! Rounding is ICU's: the number's SHORTEST round-trip decimal form is rounded
//! half away from zero (`halfExpand`), so `(1.005).toLocaleString('en-US',
//! {maximumFractionDigits: 2})` is `1.01` even though the binary value is just
//! below 1.005 — which is where this differs from `toFixed`.

use crate::host::{self, with_host};
use fusevm::Value;

/// The currencies whose en-US rendering is not "the ISO code, a no-break space,
/// two fraction digits": `(code, symbol, fraction digits, NBSP after symbol)`.
///
/// GENERATED from node v26.10.0 (`Intl.supportedValuesOf('currency')`, each
/// formatted with `formatToParts`); any code absent here — including a
/// well-formed code ICU does not know, like `XYZ` — renders as `XYZ 1.00`.
const CURRENCIES: &[(&str, &str, usize, bool)] = &[
    ("AFN", "AFN", 0, true),
    ("ALL", "ALL", 0, true),
    ("AUD", "A$", 2, false),
    ("BHD", "BHD", 3, true),
    ("BIF", "BIF", 0, true),
    ("BRL", "R$", 2, false),
    ("CAD", "CA$", 2, false),
    ("CLP", "CLP", 0, true),
    ("CNY", "CN¥", 2, false),
    ("COP", "COP", 0, true),
    ("DJF", "DJF", 0, true),
    ("EUR", "€", 2, false),
    ("GBP", "£", 2, false),
    ("GNF", "GNF", 0, true),
    ("HKD", "HK$", 2, false),
    ("HUF", "HUF", 0, true),
    ("IDR", "IDR", 0, true),
    ("ILS", "₪", 2, false),
    ("INR", "₹", 2, false),
    ("IQD", "IQD", 0, true),
    ("IRR", "IRR", 0, true),
    ("ISK", "ISK", 0, true),
    ("JOD", "JOD", 3, true),
    ("JPY", "¥", 0, false),
    ("KMF", "KMF", 0, true),
    ("KPW", "KPW", 0, true),
    ("KRW", "₩", 0, false),
    ("KWD", "KWD", 3, true),
    ("LAK", "LAK", 0, true),
    ("LBP", "LBP", 0, true),
    ("LYD", "LYD", 3, true),
    ("MGA", "MGA", 0, true),
    ("MMK", "MMK", 0, true),
    ("MXN", "MX$", 2, false),
    ("NZD", "NZ$", 2, false),
    ("OMR", "OMR", 3, true),
    ("PHP", "₱", 2, false),
    ("PKR", "PKR", 0, true),
    ("PYG", "PYG", 0, true),
    ("RWF", "RWF", 0, true),
    ("SLL", "SLL", 0, true),
    ("SOS", "SOS", 0, true),
    ("SYP", "SYP", 0, true),
    ("TND", "TND", 3, true),
    ("TWD", "NT$", 2, false),
    ("UGX", "UGX", 0, true),
    ("USD", "$", 2, false),
    ("VND", "₫", 0, false),
    ("VUV", "VUV", 0, true),
    ("XAF", "FCFA", 0, true),
    ("XCD", "EC$", 2, false),
    ("XCG", "Cg.", 2, true),
    ("XOF", "F\u{202f}CFA", 0, true),
    ("XPF", "CFPF", 0, true),
    ("YER", "YER", 0, true),
];

/// The value being formatted. A BigInt is formatted from its exact digits.
pub enum Num<'a> {
    Float(f64),
    /// The magnitude's decimal digits and whether it is negative.
    BigInt(&'a str, bool),
}

enum Style {
    Decimal,
    Percent,
    /// The prefix as rendered (`$`, `CHF\u{a0}`).
    Currency(String),
}

#[derive(PartialEq)]
enum SignDisplay {
    Auto,
    Always,
    ExceptZero,
    Negative,
    Never,
}

enum Rounding {
    Fraction { min: usize, max: usize },
    Significant { min: usize, max: usize },
}

#[derive(PartialEq)]
enum Grouping {
    Always,
    Min2,
    Off,
}

struct Format {
    style: Style,
    sign: SignDisplay,
    min_int: usize,
    rounding: Rounding,
    grouping: Grouping,
}

/// A non-negative decimal `0.d1d2…dn × 10^point`: `digits` has no leading
/// zero (empty for zero), and `point` is how many of them precede the decimal
/// point (negative or past the end for very small / very large values).
struct Decimal {
    digits: Vec<u8>,
    point: i64,
}

impl Decimal {
    /// The shortest round-trip decimal of a finite, non-negative `x`.
    fn from_f64(x: f64) -> Decimal {
        if x == 0.0 {
            return Decimal {
                digits: Vec::new(),
                point: 0,
            };
        }
        // `{:e}` is Rust's shortest round-trip form: `1.005e0`, `1e21`.
        let s = format!("{x:e}");
        let (mant, exp) = s.split_once('e').expect("{:e} has an exponent");
        let exp: i64 = exp.parse().expect("integer exponent");
        let digits: Vec<u8> = mant
            .bytes()
            .filter(u8::is_ascii_digit)
            .map(|b| b - b'0')
            .collect();
        let mut d = Decimal {
            digits,
            point: exp + 1,
        };
        d.trim();
        d
    }

    fn from_digits(s: &str) -> Decimal {
        let digits: Vec<u8> = s.bytes().map(|b| b - b'0').collect();
        let mut d = Decimal {
            point: digits.len() as i64,
            digits,
        };
        d.trim();
        d
    }

    /// Drop leading and trailing zero digits, keeping the value.
    fn trim(&mut self) {
        let lead = self.digits.iter().take_while(|&&b| b == 0).count();
        self.digits.drain(..lead);
        self.point -= lead as i64;
        while self.digits.last() == Some(&0) {
            self.digits.pop();
        }
        if self.digits.is_empty() {
            self.point = 0;
        }
    }

    /// Keep the first `keep` digits, rounding half away from zero on the rest.
    fn round_to(&mut self, keep: i64) {
        let n = self.digits.len() as i64;
        if keep >= n {
            return;
        }
        if keep < 0 {
            self.digits.clear();
            self.point = 0;
            return;
        }
        let up = self.digits[keep as usize] >= 5;
        self.digits.truncate(keep as usize);
        if up {
            let mut i = self.digits.len();
            loop {
                if i == 0 {
                    self.digits.insert(0, 1);
                    self.point += 1;
                    break;
                }
                i -= 1;
                if self.digits[i] == 9 {
                    self.digits[i] = 0;
                } else {
                    self.digits[i] += 1;
                    break;
                }
            }
        }
        self.trim();
    }

    /// Multiply by 100 (the percent style), exactly.
    fn times_100(&mut self) {
        if !self.digits.is_empty() {
            self.point += 2;
        }
    }

    /// `(integer digits, fraction digits)` with no padding.
    fn split(&self) -> (String, String) {
        let digit = |i: i64| -> char {
            if i >= 0 && (i as usize) < self.digits.len() {
                (b'0' + self.digits[i as usize]) as char
            } else {
                '0'
            }
        };
        let int: String = if self.point > 0 {
            (0..self.point).map(digit).collect()
        } else {
            String::new()
        };
        let frac_len = self.digits.len() as i64 - self.point;
        let frac: String = (self.point..self.point + frac_len.max(0))
            .map(digit)
            .collect();
        (int, frac)
    }
}

/// V8's RangeError for an option value outside its allowed set.
fn out_of_range(value: &str, name: &str) -> String {
    host::range_error(&format!(
        "Value {value} out of range for Number.prototype.toLocaleString options property {name}"
    ))
}

/// `GetOption` for a string option restricted to `allowed`.
fn enum_option(opts: &Value, name: &str, allowed: &[&str]) -> Result<Option<String>, String> {
    match string_option(opts, name)? {
        Some(s) if !allowed.contains(&s.as_str()) => Err(out_of_range(&s, name)),
        other => Ok(other),
    }
}

/// `GetNumberOption`: `undefined` → `None`; otherwise `ToNumber`, a RangeError
/// outside `[lo, hi]` (or NaN), then `floor`.
fn digit_option(opts: &Value, name: &str, lo: f64, hi: f64) -> Result<Option<usize>, String> {
    let v = crate::builtins::get_property(opts, name)?;
    if matches!(v, Value::Undef) {
        return Ok(None);
    }
    let n = host::to_number_value(&v)?;
    if n.is_nan() || n < lo || n > hi {
        return Err(host::range_error(&format!("{name} value is out of range.")));
    }
    Ok(Some(n.floor() as usize))
}

/// `GetOption` for a string-valued option, `None` when absent.
fn string_option(opts: &Value, name: &str) -> Result<Option<String>, String> {
    let v = crate::builtins::get_property(opts, name)?;
    if matches!(v, Value::Undef) {
        return Ok(None);
    }
    Ok(Some(to_str(&v)?))
}

fn read_format(opts: &Value) -> Result<Format, String> {
    if with_host(|h| h.is_null(opts)) {
        return Err(host::type_error(
            "Number.prototype.toLocaleString called on null or undefined",
        ));
    }
    // A primitive `options` is `ToObject`ed in the spec, and a wrapper owns none
    // of these keys, so it formats exactly as `undefined` does.
    let is_object = matches!(opts, Value::Obj(_)) && !with_host(|h| host::is_primitive(h, opts));
    if !is_object {
        return Ok(Format {
            style: Style::Decimal,
            sign: SignDisplay::Auto,
            min_int: 1,
            rounding: Rounding::Fraction { min: 0, max: 3 },
            grouping: Grouping::Always,
        });
    }
    // Read in `InitializeNumberFormat` order, so the first invalid option is
    // the one reported: style, currency, currencyDisplay, the digit options,
    // useGrouping, signDisplay.
    let style_name = enum_option(opts, "style", &["decimal", "percent", "currency", "unit"])?
        .unwrap_or_else(|| "decimal".into());
    // A malformed code is refused whatever the style (`IsWellFormedCurrencyCode`).
    let currency = string_option(opts, "currency")?;
    if let Some(code) = &currency {
        if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err(host::range_error(&format!(
                "Invalid currency code : {code}"
            )));
        }
    }
    let display = enum_option(
        opts,
        "currencyDisplay",
        &["code", "symbol", "narrowSymbol", "name"],
    )?;
    let (style, currency_digits) = match style_name.as_str() {
        "percent" => (Style::Percent, None),
        "currency" => {
            let Some(code) = currency else {
                return Err(host::type_error(
                    "Currency code is required with currency style.",
                ));
            };
            let code = code.to_ascii_uppercase();
            let known = CURRENCIES.iter().find(|(c, ..)| *c == code);
            let digits = known.map(|(_, _, d, _)| *d).unwrap_or(2);
            // `code` always spells the ISO code; `symbol` (and, not modelled,
            // `narrowSymbol`/`name`) uses the en-US symbol table.
            let prefix = match (display.as_deref(), known) {
                (Some("code"), _) | (_, None) => format!("{code}\u{a0}"),
                (_, Some((_, sym, _, spaced))) => {
                    let gap = if *spaced { "\u{a0}" } else { "" };
                    format!("{sym}{gap}")
                }
            };
            (Style::Currency(prefix), Some(digits))
        }
        // `unit` needs per-unit display data; its number formats as decimal.
        _ => (Style::Decimal, None),
    };
    let min_int = digit_option(opts, "minimumIntegerDigits", 1.0, 21.0)?.unwrap_or(1);
    let min_frac = digit_option(opts, "minimumFractionDigits", 0.0, 100.0)?;
    let max_frac = digit_option(opts, "maximumFractionDigits", 0.0, 100.0)?;
    // `SetNumberFormatDigitOptions`: significant digits win when either is set.
    let min_sig = digit_option(opts, "minimumSignificantDigits", 1.0, 21.0)?;
    let max_sig = digit_option(opts, "maximumSignificantDigits", 1.0, 21.0)?;
    let rounding = if min_sig.is_some() || max_sig.is_some() {
        let min = min_sig.unwrap_or(1);
        let max = max_sig.unwrap_or(21);
        if min > max {
            return Err(host::range_error(
                "maximumSignificantDigits value is out of range.",
            ));
        }
        Rounding::Significant { min, max }
    } else {
        let (min_default, max_default) = match (&style, currency_digits) {
            (_, Some(d)) => (d, d),
            (Style::Percent, _) => (0, 0),
            _ => (0, 3),
        };
        let (min, max) = match (min_frac, max_frac) {
            (None, None) => (min_default, max_default),
            (Some(mn), None) => (mn, mn.max(max_default)),
            (None, Some(mx)) => (min_default.min(mx), mx),
            (Some(mn), Some(mx)) => {
                if mn > mx {
                    return Err(host::range_error(
                        "maximumFractionDigits value is out of range.",
                    ));
                }
                (mn, mx)
            }
        };
        Rounding::Fraction { min, max }
    };
    // `GetBooleanOrStringNumberFormatOption`: `true` is "always", any other
    // falsy value turns grouping off, and the strings "true"/"false" select
    // the default ("auto", which groups in en-US).
    let grouping_v = crate::builtins::get_property(opts, "useGrouping")?;
    let grouping = match grouping_v {
        Value::Undef | Value::Bool(true) => Grouping::Always,
        ref v if !with_host(|h| h.truthy(v)) => Grouping::Off,
        ref v => match to_str(v)?.as_str() {
            "min2" => Grouping::Min2,
            "always" | "auto" | "true" | "false" => Grouping::Always,
            other => return Err(out_of_range(other, "useGrouping")),
        },
    };
    let sign = match enum_option(
        opts,
        "signDisplay",
        &["auto", "never", "always", "exceptZero", "negative"],
    )?
    .as_deref()
    {
        Some("never") => SignDisplay::Never,
        Some("always") => SignDisplay::Always,
        Some("exceptZero") => SignDisplay::ExceptZero,
        Some("negative") => SignDisplay::Negative,
        _ => SignDisplay::Auto,
    };
    Ok(Format {
        style,
        sign,
        min_int,
        rounding,
        grouping,
    })
}

fn group(int: &str, grouping: &Grouping) -> String {
    if *grouping == Grouping::Off || (*grouping == Grouping::Min2 && int.len() < 5) {
        return int.to_string();
    }
    let mut out = String::with_capacity(int.len() + int.len() / 3);
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Format `x` the way `x.toLocaleString(locales, opts)` does under en-US.
pub fn to_locale_string(x: Num, opts: &Value) -> Result<String, String> {
    let fmt = read_format(opts)?;
    let (neg, mut dec, special) = match x {
        Num::Float(f) if f.is_nan() => (false, Decimal::from_digits(""), Some("NaN")),
        Num::Float(f) if f.is_infinite() => (f < 0.0, Decimal::from_digits(""), Some("∞")),
        Num::Float(f) => (f.is_sign_negative(), Decimal::from_f64(f.abs()), None),
        Num::BigInt(digits, neg) => (neg, Decimal::from_digits(digits), None),
    };
    let body = if let Some(text) = special {
        text.to_string()
    } else {
        if matches!(fmt.style, Style::Percent) {
            dec.times_100();
        }
        let (min_frac_shown, min_sig) = match fmt.rounding {
            Rounding::Fraction { min, max } => {
                dec.round_to(dec.point + max as i64);
                (min, None)
            }
            Rounding::Significant { min, max } => {
                dec.round_to(max as i64);
                (0, Some(min))
            }
        };
        let (mut int, mut frac) = dec.split();
        if int.len() < fmt.min_int {
            int = format!("{}{int}", "0".repeat(fmt.min_int - int.len()));
        }
        let mut min_frac = min_frac_shown;
        if let Some(min_sig) = min_sig {
            // Pad to `minimumSignificantDigits`, counted from the first
            // non-zero digit; a zero counts its units digit as the one it has.
            let sig_int = int.trim_start_matches('0').len();
            let lead_frac_zeros = if sig_int == 0 {
                frac.bytes().take_while(|&b| b == b'0').count()
            } else {
                0
            };
            let have = (sig_int + frac.len() - lead_frac_zeros).max(1);
            min_frac = frac.len() + min_sig.saturating_sub(have);
        }
        while frac.len() < min_frac {
            frac.push('0');
        }
        let grouped = group(&int, &fmt.grouping);
        if frac.is_empty() {
            grouped
        } else {
            format!("{grouped}.{frac}")
        }
    };
    // The sign is decided on the ROUNDED value: `-0.0001` rounds to zero, and
    // only `auto`/`always` still show its minus.
    let zero = special.is_none() && dec.digits.is_empty();
    let nan = special == Some("NaN");
    let sign = match fmt.sign {
        SignDisplay::Never => "",
        SignDisplay::ExceptZero if zero || nan => "",
        SignDisplay::Negative if zero => "",
        _ if neg => "-",
        SignDisplay::Always | SignDisplay::ExceptZero => "+",
        _ => "",
    };
    Ok(match fmt.style {
        Style::Decimal => format!("{sign}{body}"),
        Style::Percent => format!("{sign}{body}%"),
        Style::Currency(prefix) => format!("{sign}{prefix}{body}"),
    })
}

/// `ToString` of an option value, running a user `toString`.
fn to_str(v: &Value) -> Result<String, String> {
    let s = host::to_string_value(v)?;
    Ok(with_host(|h| h.str_of(&s)))
}
