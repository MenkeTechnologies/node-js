//! JavaScript `Date` (global constructor). A Date is a plain object tagged
//! `@@native = "Date"` whose time value (milliseconds since the Unix epoch, or
//! NaN for an invalid date) lives in a hidden `@@ms` field.
//!
//! Only the UTC-based surface HTTP libraries actually exercise is implemented —
//! `getTime`/`valueOf`, `toISOString`/`toUTCString`/`toString`, the UTC field
//! getters, plus the statics `Date.now`/`Date.parse`/`Date.UTC`. Local-timezone
//! getters alias the UTC ones (node-js runs as if TZ=UTC), which is the correct
//! answer for the machine-readable date headers express/send/fresh produce.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub const STATIC_METHODS: &[&str] = &["now", "parse", "UTC"];

const MS_PER_DAY: f64 = 86_400_000.0;
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Milliseconds since the Unix epoch, right now.
fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Build a Date value carrying `ms` (NaN → an "Invalid Date").
fn from_ms(ms: f64) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Date"));
        m.insert("@@ms".into(), Value::Float(ms));
        h.new_object(m)
    })
}

/// The stored time value of a Date instance (NaN if not a Date).
fn ms_of(recv: &Value) -> f64 {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@ms").map(|v| h.to_number(v)).unwrap_or(f64::NAN),
        _ => f64::NAN,
    })
}

/// `new Date(...)`.
pub fn construct(args: &[Value]) -> Result<Value, String> {
    let ms = match args.len() {
        0 => now_ms(),
        1 => {
            let a = &args[0];
            // A string argument is parsed; anything else is coerced to a number
            // (milliseconds). Another Date coerces via its time value.
            if let Value::Str(_) = a {
                parse_str(&with_host(|h| h.str_of(a)))
            } else if with_host(|h| matches!(h.get(a), Some(JsObj::Str(_)))) {
                parse_str(&with_host(|h| h.str_of(a)))
            } else if super::native_tag(a).as_deref() == Some("Date") {
                ms_of(a)
            } else {
                with_host(|h| h.to_number(a))
            }
        }
        // (year, month[, day, hours, minutes, seconds, ms]) — interpreted as UTC.
        _ => {
            let n = |i: usize, dflt: f64| {
                args.get(i).map(|v| with_host(|h| h.to_number(v))).unwrap_or(dflt)
            };
            let mut year = n(0, f64::NAN);
            // Years 0..99 map to 1900..1999 per the spec.
            if (0.0..=99.0).contains(&year) {
                year += 1900.0;
            }
            utc_from_fields(year, n(1, 0.0), n(2, 1.0), n(3, 0.0), n(4, 0.0), n(5, 0.0), n(6, 0.0))
        }
    };
    Ok(from_ms(ms))
}

/// `Date.now()` / `Date.parse(str)` / `Date.UTC(...)`.
pub fn static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "now" => Ok(Value::Float(now_ms())),
        "parse" => Ok(Value::Float(parse_str(&super::arg_str(args, 0)))),
        "UTC" => {
            let n = |i: usize, dflt: f64| {
                args.get(i).map(|v| with_host(|h| h.to_number(v))).unwrap_or(dflt)
            };
            let mut year = n(0, f64::NAN);
            if (0.0..=99.0).contains(&year) {
                year += 1900.0;
            }
            Ok(Value::Float(utc_from_fields(
                year, n(1, 0.0), n(2, 1.0), n(3, 0.0), n(4, 0.0), n(5, 0.0), n(6, 0.0),
            )))
        }
        _ => return None,
    })
}

/// Date instance methods (all treated as UTC — see the module note).
pub fn instance_call(recv: &Value, method: &str, _args: &[Value]) -> Result<Value, String> {
    let ms = ms_of(recv);
    let f = |ms: f64| ms; // readability alias for numeric returns
    Ok(match method {
        "getTime" | "valueOf" => Value::Float(f(ms)),
        "toISOString" | "toJSON" => {
            if ms.is_nan() {
                if method == "toJSON" {
                    with_host(|h| h.null())
                } else {
                    return Err(crate::host::range_error("Invalid time value"));
                }
            } else {
                with_host(|h| h.new_str(iso_string(ms)))
            }
        }
        "toUTCString" | "toGMTString" => with_host(|h| h.new_str(utc_string(ms))),
        "toString" => with_host(|h| h.new_str(if ms.is_nan() { "Invalid Date".into() } else { utc_string(ms) })),
        "toDateString" => with_host(|h| h.new_str(date_string(ms))),
        "getFullYear" | "getUTCFullYear" => Value::Float(field(ms, Field::Year)),
        "getMonth" | "getUTCMonth" => Value::Float(field(ms, Field::Month)),
        "getDate" | "getUTCDate" => Value::Float(field(ms, Field::Day)),
        "getDay" | "getUTCDay" => Value::Float(field(ms, Field::Weekday)),
        "getHours" | "getUTCHours" => Value::Float(field(ms, Field::Hours)),
        "getMinutes" | "getUTCMinutes" => Value::Float(field(ms, Field::Minutes)),
        "getSeconds" | "getUTCSeconds" => Value::Float(field(ms, Field::Seconds)),
        "getMilliseconds" | "getUTCMilliseconds" => Value::Float(field(ms, Field::Millis)),
        "getTimezoneOffset" => Value::Float(0.0), // node-js runs as UTC
        "setTime" => {
            let new_ms = super::arg_num(_args, 0);
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get_mut(recv) {
                    p.insert("@@ms".into(), Value::Float(new_ms));
                }
            });
            Value::Float(new_ms)
        }
        _ => return Err(crate::host::type_error(&format!("date.{method} is not a function"))),
    })
}

// ── civil-calendar conversions (days-from-epoch ⇄ Y/M/D), UTC only ────────────

enum Field {
    Year,
    Month,
    Day,
    Weekday,
    Hours,
    Minutes,
    Seconds,
    Millis,
}

/// Split a time value into (days-from-epoch, ms-within-day), flooring toward -∞
/// so negative (pre-1970) times decompose correctly.
fn split_day(ms: f64) -> (i64, i64) {
    let day = (ms / MS_PER_DAY).floor();
    let rem = ms - day * MS_PER_DAY;
    (day as i64, rem as i64)
}

/// Convert a days-from-epoch count to (year, month 0-11, day 1-31) using
/// Howard Hinnant's civil_from_days algorithm.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m - 1, d)
}

/// Inverse: (year, month 0-11, day) → days from epoch.
fn days_from_civil(y: i64, m0: i64, d: i64) -> i64 {
    let m = m0 + 1;
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn field(ms: f64, which: Field) -> f64 {
    if ms.is_nan() {
        return f64::NAN;
    }
    let (day, rem) = split_day(ms);
    let (y, mo, d) = civil_from_days(day);
    match which {
        Field::Year => y as f64,
        Field::Month => mo as f64,
        Field::Day => d as f64,
        // Weekday: 1970-01-01 (day 0) was a Thursday (4).
        Field::Weekday => (((day % 7) + 4 + 7) % 7) as f64,
        Field::Hours => (rem / 3_600_000) as f64,
        Field::Minutes => (rem / 60_000 % 60) as f64,
        Field::Seconds => (rem / 1000 % 60) as f64,
        Field::Millis => (rem % 1000) as f64,
    }
}

/// Assemble a UTC time value from broken-down fields (with month/day overflow
/// normalized the way JS does, e.g. month 12 rolls into the next year).
fn utc_from_fields(y: f64, mo: f64, d: f64, h: f64, mi: f64, s: f64, ms: f64) -> f64 {
    if [y, mo, d, h, mi, s, ms].iter().any(|v| v.is_nan()) {
        return f64::NAN;
    }
    // Normalize month into 0..11, carrying into the year.
    let total_months = y as i64 * 12 + mo as i64;
    let year = total_months.div_euclid(12);
    let month = total_months.rem_euclid(12);
    let days = days_from_civil(year, month, d as i64);
    days as f64 * MS_PER_DAY + h * 3_600_000.0 + mi * 60_000.0 + s * 1000.0 + ms
}

/// `Wed, 21 Oct 2015 07:28:00 GMT` — the RFC-7231 IMF-fixdate HTTP header form.
fn utc_string(ms: f64) -> String {
    if ms.is_nan() {
        return "Invalid Date".into();
    }
    let (day, _) = split_day(ms);
    let (y, mo, d) = civil_from_days(day);
    let wd = (((day % 7) + 4 + 7) % 7) as usize;
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        DAYS[wd],
        d,
        MONTHS[mo as usize],
        y,
        field(ms, Field::Hours) as i64,
        field(ms, Field::Minutes) as i64,
        field(ms, Field::Seconds) as i64,
    )
}

/// `Wed Oct 21 2015` — the `toDateString` form.
fn date_string(ms: f64) -> String {
    if ms.is_nan() {
        return "Invalid Date".into();
    }
    let (day, _) = split_day(ms);
    let (y, mo, d) = civil_from_days(day);
    let wd = (((day % 7) + 4 + 7) % 7) as usize;
    format!("{} {} {:02} {:04}", DAYS[wd], MONTHS[mo as usize], d, y)
}

/// `2015-10-21T07:28:00.000Z` — the ISO-8601 / `toISOString` form.
fn iso_string(ms: f64) -> String {
    let (day, _) = split_day(ms);
    let (y, mo, d) = civil_from_days(day);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        mo + 1,
        d,
        field(ms, Field::Hours) as i64,
        field(ms, Field::Minutes) as i64,
        field(ms, Field::Seconds) as i64,
        field(ms, Field::Millis) as i64,
    )
}

/// Parse a date string. Supports the two forms HTTP code produces: ISO-8601
/// (`2015-10-21T07:28:00.000Z` / date-only `2015-10-21`) and the RFC-1123 /
/// IMF-fixdate header form (`Wed, 21 Oct 2015 07:28:00 GMT`). Returns NaN on any
/// input that does not match — the JS "Invalid Date" contract.
fn parse_str(s: &str) -> f64 {
    let s = s.trim();
    if let Some(ms) = parse_iso(s) {
        return ms;
    }
    if let Some(ms) = parse_rfc1123(s) {
        return ms;
    }
    f64::NAN
}

/// ISO-8601: `YYYY-MM-DD[THH:MM:SS[.sss]][Z]` (a bare date is treated as UTC
/// midnight, matching modern V8).
fn parse_iso(s: &str) -> Option<f64> {
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let dp: Vec<&str> = date.split('-').collect();
    if dp.len() != 3 {
        return None;
    }
    let y: i64 = dp[0].parse().ok()?;
    let mo: i64 = dp[1].parse().ok()?;
    let d: i64 = dp[2].parse().ok()?;
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let (mut h, mut mi, mut sec, mut milli) = (0i64, 0i64, 0i64, 0i64);
    if let Some(t) = time {
        let t = t.trim_end_matches('Z');
        let (hms, frac) = match t.split_once('.') {
            Some((a, b)) => (a, Some(b)),
            None => (t, None),
        };
        let tp: Vec<&str> = hms.split(':').collect();
        if tp.is_empty() {
            return None;
        }
        h = tp[0].parse().ok()?;
        mi = tp.get(1).map(|v| v.parse().ok()).unwrap_or(Some(0))?;
        sec = tp.get(2).map(|v| v.parse().ok()).unwrap_or(Some(0))?;
        if let Some(fr) = frac {
            let fr: String = fr.chars().take(3).collect();
            let padded = format!("{fr:0<3}");
            milli = padded.parse().ok()?;
        }
    }
    let days = days_from_civil(y, mo - 1, d);
    Some(days as f64 * MS_PER_DAY + h as f64 * 3_600_000.0 + mi as f64 * 60_000.0 + sec as f64 * 1000.0 + milli as f64)
}

/// RFC-1123 / IMF-fixdate: `Wed, 21 Oct 2015 07:28:00 GMT`.
fn parse_rfc1123(s: &str) -> Option<f64> {
    // Drop an optional leading weekday token (`Wed,`).
    let s = match s.split_once(", ") {
        Some((_, rest)) => rest,
        None => s,
    };
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }
    let d: i64 = parts[0].parse().ok()?;
    let mo = MONTHS.iter().position(|m| *m == parts[1])? as i64;
    let y: i64 = parts[2].parse().ok()?;
    let tp: Vec<&str> = parts[3].split(':').collect();
    if tp.len() < 2 {
        return None;
    }
    let h: i64 = tp[0].parse().ok()?;
    let mi: i64 = tp[1].parse().ok()?;
    let sec: i64 = tp.get(2).map(|v| v.parse().ok()).unwrap_or(Some(0))?;
    let days = days_from_civil(y, mo, d);
    Some(days as f64 * MS_PER_DAY + h as f64 * 3_600_000.0 + mi as f64 * 60_000.0 + sec as f64 * 1000.0)
}
