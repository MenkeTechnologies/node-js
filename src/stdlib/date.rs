//! JavaScript `Date` (global constructor). A Date is a plain object tagged
//! `@@native = "Date"` whose time value (milliseconds since the Unix epoch, or
//! NaN for an invalid date) lives in a hidden `@@ms` field.
//!
//! The UTC-based surface is implemented in full: `getTime`/`valueOf`, the
//! `toISOString`/`toUTCString`/`toString`/`toDateString`/`toTimeString`/
//! `toLocale*` renderings, the field getters, the component SETTERS, the Annex-B
//! `getYear`/`setYear`, and the statics `Date.now`/`Date.parse`/`Date.UTC`.
//! Local-timezone getters and setters alias the UTC ones (node-js runs as if
//! `TZ=UTC`), which is the correct answer for the machine-readable date headers
//! express/send/fresh produce.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub const STATIC_METHODS: &[&str] = &["now", "parse", "UTC"];

/// Every method `instance_call` below answers. `valueOf` and `toString` are on
/// this list because `Object.prototype` has methods by those names too: without
/// the entry, `host::call_method` hands a Date to `object_builtin_method` and
/// `date.valueOf()` returns the Date itself instead of its time value (so `+d`
/// and `d - 0` were `NaN`).
pub const INSTANCE_METHODS: &[&str] = &[
    "getTime",
    "valueOf",
    "toISOString",
    "toJSON",
    "toUTCString",
    "toGMTString",
    "toString",
    "toDateString",
    "toLocaleString",
    "toLocaleDateString",
    "toLocaleTimeString",
    "getFullYear",
    "getUTCFullYear",
    "getMonth",
    "getUTCMonth",
    "getDate",
    "getUTCDate",
    "getDay",
    "getUTCDay",
    "getHours",
    "getUTCHours",
    "getMinutes",
    "getUTCMinutes",
    "getSeconds",
    "getUTCSeconds",
    "getMilliseconds",
    "getUTCMilliseconds",
    "getTimezoneOffset",
    "toTimeString",
    "setTime",
    "setFullYear",
    "setUTCFullYear",
    "setMonth",
    "setUTCMonth",
    "setDate",
    "setUTCDate",
    "setHours",
    "setUTCHours",
    "setMinutes",
    "setUTCMinutes",
    "setSeconds",
    "setUTCSeconds",
    "setMilliseconds",
    "setUTCMilliseconds",
    "getYear",
    "setYear",
    // 21.4.4.45. A Date is the one builtin whose DEFAULT hint is `"string"`,
    // which is why `date + 1` concatenates while `date - 1` subtracts. The
    // coercion path already knew that, but the method itself was not exposed,
    // so `date[Symbol.toPrimitive]` was not a function.
    "@@toPrimitive",
];

const MS_PER_DAY: f64 = 86_400_000.0;
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

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
                // 21.4.2.1 step 3.d is `ToNumber(v)` after `ToPrimitive`, and a
                // SYMBOL refuses it — `new Date(sym)` produced an Invalid Date
                // instead of throwing.
                if with_host(|h| matches!(h.get(a), Some(crate::host::JsObj::Symbol { .. }))) {
                    return Err(crate::host::type_error(
                        "Cannot convert a Symbol value to a number",
                    ));
                }
                with_host(|h| h.to_number(a))
            }
        }
        // (year, month[, day, hours, minutes, seconds, ms]) — LOCAL time
        // (21.4.2.1 step 5.k: `UTC(MakeDate(...))`). It was taken as UTC, so
        // under `TZ=America/New_York` `new Date(2024, 0, 31)` read back as
        // 19:00 on the 30th, and `new Date(99, 0).getFullYear()` as 1998.
        _ => {
            let n = |i: usize, dflt: f64| {
                args.get(i)
                    .map(|v| with_host(|h| h.to_number(v)))
                    .unwrap_or(dflt)
            };
            let mut year = n(0, f64::NAN);
            // Years 0..99 map to 1900..1999 per the spec.
            if (0.0..=99.0).contains(&year.trunc()) {
                year = year.trunc() + 1900.0;
            }
            utc_from_local(utc_from_fields(
                year,
                n(1, 0.0),
                n(2, 1.0),
                n(3, 0.0),
                n(4, 0.0),
                n(5, 0.0),
                n(6, 0.0),
            ))
        }
    };
    // TimeClip (21.4.1.31): a value beyond ±8.64e15 ms is not a representable
    // date and becomes NaN. `new Date(8.64e15 + 1)` used to keep the raw number
    // and print a real date where node prints `Invalid Date`.
    Ok(from_ms(time_clip(ms)))
}

/// `Date.now()` / `Date.parse(str)` / `Date.UTC(...)`.
pub fn static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "now" => Ok(Value::Float(now_ms())),
        "parse" => Ok(Value::Float(parse_str(&super::arg_str(args, 0)))),
        "UTC" => {
            let n = |i: usize, dflt: f64| {
                args.get(i)
                    .map(|v| with_host(|h| h.to_number(v)))
                    .unwrap_or(dflt)
            };
            let mut year = n(0, f64::NAN);
            if (0.0..=99.0).contains(&year.trunc()) {
                year = year.trunc() + 1900.0;
            }
            Ok(Value::Float(utc_from_fields(
                year,
                n(1, 0.0),
                n(2, 1.0),
                n(3, 0.0),
                n(4, 0.0),
                n(5, 0.0),
                n(6, 0.0),
            )))
        }
        _ => return None,
    })
}

/// Date instance methods (all treated as UTC — see the module note).
pub fn instance_call(recv: &Value, method: &str, _args: &[Value]) -> Result<Value, String> {
    // Every `set*` argument is `ToNumber`d (21.4.4.x), which runs a user
    // `valueOf` and can throw from it. The reads below are infallible and do no
    // `ToPrimitive`, so `d.setFullYear({valueOf: () => 2020})` produced an
    // Invalid Date.
    let coerced: Vec<Value>;
    let _args: &[Value] = if method.starts_with("set") {
        let mut out = Vec::with_capacity(_args.len());
        for a in _args {
            let p = crate::host::to_primitive(a, "number")?;
            out.push(Value::Float(with_host(|h| h.to_number(&p))));
        }
        coerced = out;
        &coerced
    } else {
        _args
    };
    let ms = ms_of(recv);
    let f = |ms: f64| ms; // readability alias for numeric returns
    Ok(match method {
        "getTime" | "valueOf" => Value::Float(f(ms)),
        // Only an explicit `"number"` hint yields the timestamp; `"string"` and
        // `"default"` both render the date, which is the rule that makes the
        // default hint behave as `"string"`.
        "@@toPrimitive" => {
            let hint = with_host(|h| h.str_of(&_args.first().cloned().unwrap_or(Value::Undef)));
            if hint == "number" {
                Value::Float(f(ms))
            } else {
                return instance_call(recv, "toString", _args);
            }
        }
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
        // `toString` (21.4.4.41) is NOT the RFC-7231 header form — that is
        // `toUTCString`. It is `ToDateString`: the `toDateString` half, a space,
        // then the `toTimeString` half. This used to answer `toUTCString`, so
        // `String(date)` and `` `${date}` `` printed
        // `Thu, 01 Jan 1970 00:00:00 GMT` where node prints
        // `Thu Jan 01 1970 00:00:00 GMT+0000 (Coordinated Universal Time)`.
        "toString" => with_host(|h| {
            h.new_str(if ms.is_nan() {
                "Invalid Date".into()
            } else {
                format!("{} {}", date_string(local_ms(ms)), time_string(ms))
            })
        }),
        "toDateString" => with_host(|h| h.new_str(date_string(local_ms(ms)))),
        // The three `toLocale*` forms threw `is not a function` — absent
        // entirely, so `new Date(0).toLocaleString()` failed where node prints
        // `1/1/1970, 12:00:00 AM`. Rendered in node's default en-US shape
        // (`M/D/YYYY` and 12-hour `h:mm:ss AM/PM`) at UTC, consistent with the
        // rest of this module running as if `TZ=UTC`. The `locales`/`options`
        // arguments are accepted and ignored: without ICU there is nothing to
        // vary, and answering the default form beats throwing.
        "toLocaleString" => with_host(|h| {
            h.new_str(if ms.is_nan() {
                "Invalid Date".into()
            } else {
                format!(
                    "{}, {}",
                    locale_date(local_ms(ms)),
                    locale_time(local_ms(ms))
                )
            })
        }),
        "toLocaleDateString" => with_host(|h| {
            h.new_str(if ms.is_nan() {
                "Invalid Date".into()
            } else {
                locale_date(local_ms(ms))
            })
        }),
        "toLocaleTimeString" => with_host(|h| {
            h.new_str(if ms.is_nan() {
                "Invalid Date".into()
            } else {
                locale_time(local_ms(ms))
            })
        }),
        "getFullYear" => Value::Float(field(local_ms(ms), Field::Year)),
        "getUTCFullYear" => Value::Float(field(ms, Field::Year)),
        "getMonth" => Value::Float(field(local_ms(ms), Field::Month)),
        "getUTCMonth" => Value::Float(field(ms, Field::Month)),
        "getDate" => Value::Float(field(local_ms(ms), Field::Day)),
        "getUTCDate" => Value::Float(field(ms, Field::Day)),
        "getDay" => Value::Float(field(local_ms(ms), Field::Weekday)),
        "getUTCDay" => Value::Float(field(ms, Field::Weekday)),
        "getHours" => Value::Float(field(local_ms(ms), Field::Hours)),
        "getUTCHours" => Value::Float(field(ms, Field::Hours)),
        "getMinutes" => Value::Float(field(local_ms(ms), Field::Minutes)),
        "getUTCMinutes" => Value::Float(field(ms, Field::Minutes)),
        "getSeconds" => Value::Float(field(local_ms(ms), Field::Seconds)),
        "getUTCSeconds" => Value::Float(field(ms, Field::Seconds)),
        "getMilliseconds" => Value::Float(field(local_ms(ms), Field::Millis)),
        "getUTCMilliseconds" => Value::Float(field(ms, Field::Millis)),
        // 21.4.4.7: minutes WEST of UTC, so the sign is the opposite of the
        // offset itself — `TZ=America/Detroit` reports 300, not -300.
        // Computed as `(t - LocalTime(t)) / msPerMinute`, as written, so a zero
        // offset is +0: negating it printed `-0` under `TZ=UTC`.
        "getTimezoneOffset" => Value::Float(if ms.is_nan() {
            f64::NAN
        } else {
            (ms - local_ms(ms)) / 60_000.0
        }),
        "toTimeString" => with_host(|h| h.new_str(time_string(ms))),
        "setTime" => Value::Float(store_ms(recv, time_clip(super::arg_num(_args, 0)))),
        // The component setters (21.4.4.20-21.4.4.28). Each takes its own field
        // plus every LOWER-order one it can reach, defaulting the rest from the
        // current time value, then rebuilds and TimeClips. `setUTCFullYear` and
        // friends were absent entirely, so `d.setUTCFullYear(2000)` threw
        // `is not a function` — a Date could be read but never modified except
        // wholesale through `setTime`.
        "setFullYear" => Value::Float(set_fields_local(recv, ms, 0, _args, false)),
        "setUTCFullYear" => Value::Float(set_fields(recv, ms, 0, _args, false)),
        "setMonth" => Value::Float(set_fields_local(recv, ms, 1, _args, false)),
        "setUTCMonth" => Value::Float(set_fields(recv, ms, 1, _args, false)),
        "setDate" => Value::Float(set_fields_local(recv, ms, 2, _args, false)),
        "setUTCDate" => Value::Float(set_fields(recv, ms, 2, _args, false)),
        "setHours" => Value::Float(set_fields_local(recv, ms, 3, _args, false)),
        "setUTCHours" => Value::Float(set_fields(recv, ms, 3, _args, false)),
        "setMinutes" => Value::Float(set_fields_local(recv, ms, 4, _args, false)),
        "setUTCMinutes" => Value::Float(set_fields(recv, ms, 4, _args, false)),
        "setSeconds" => Value::Float(set_fields_local(recv, ms, 5, _args, false)),
        "setUTCSeconds" => Value::Float(set_fields(recv, ms, 5, _args, false)),
        "setMilliseconds" => Value::Float(set_fields_local(recv, ms, 6, _args, false)),
        "setUTCMilliseconds" => Value::Float(set_fields(recv, ms, 6, _args, false)),
        // Annex B B.2.3.3 / B.2.3.4 — offset-from-1900 year accessors kept for
        // legacy code. `setYear` maps 0..99 onto 1900..1999, which is the only
        // way it differs from `setFullYear`.
        // Annex B's pair is LOCAL, like `getFullYear`/`setFullYear`.
        "getYear" => Value::Float(if ms.is_nan() {
            f64::NAN
        } else {
            field(local_ms(ms), Field::Year) - 1900.0
        }),
        "setYear" => Value::Float(set_fields_local(recv, ms, 0, _args, true)),
        _ => {
            return Err(crate::host::type_error(&format!(
                "date.{method} is not a function"
            )))
        }
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
    // MakeDay / MakeTime (21.4.1.28-29): a non-finite field is NaN, and every
    // field is `ToIntegerOrInfinity`d first — so `new Date(2024, 0, 1, 1.5)` is
    // 01:00, not 01:30, and `Date.UTC(1970, 0, 1, 0, 0, 0, 0.9)` is 0.
    if [y, mo, d, h, mi, s, ms].iter().any(|v| !v.is_finite()) {
        return f64::NAN;
    }
    let [y, mo, d, h, mi, s, ms] = [y, mo, d, h, mi, s, ms].map(f64::trunc);
    // Normalize month into 0..11, carrying into the year.
    let total_months = y as i64 * 12 + mo as i64;
    let year = total_months.div_euclid(12);
    let month = total_months.rem_euclid(12);
    let days = days_from_civil(year, month, d as i64);
    days as f64 * MS_PER_DAY + h * 3_600_000.0 + mi * 60_000.0 + s * 1000.0 + ms
}

/// `Wed, 21 Oct 2015 07:28:00 GMT` — the RFC-7231 IMF-fixdate HTTP header form.
/// The zone offset in MILLISECONDS east of UTC that applies at `ms`, from the
/// C library's `localtime_r` — which reads `TZ` exactly as node does and is
/// DST-aware per timestamp rather than per zone.
///
/// Everything local used to be UTC: `getTimezoneOffset()` answered 0, each
/// local getter shared its arm with the `getUTC*` one, and `toString` rendered
/// the UTC wall clock. Under `TZ=America/Detroit` that made
/// `new Date(0).getMonth()` 0 where node says 11. The parity harness pins
/// `TZ=UTC` for both sides, which is why no record ever caught it.
#[cfg(unix)]
fn zone_offset_ms(ms: f64) -> f64 {
    if !ms.is_finite() {
        return 0.0;
    }
    // `localtime_r` takes SECONDS; flooring keeps a pre-epoch timestamp in the
    // right second rather than rounding it toward zero.
    let secs = (ms / 1000.0).floor() as i64;
    let t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `localtime_r` writes into the caller's `tm` and reads only `t`.
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    if !ok {
        return 0.0;
    }
    tm.tm_gmtoff as f64 * 1000.0
}

#[cfg(not(unix))]
fn zone_offset_ms(_ms: f64) -> f64 {
    0.0
}

/// The local wall-clock time value for `ms` — what every local getter reads its
/// fields out of.
fn local_ms(ms: f64) -> f64 {
    ms + zone_offset_ms(ms)
}

/// The inverse: a local wall-clock time value back to a timestamp.
///
/// The offset depends on the instant, so one lookup is not enough near a DST
/// transition. Two candidates are built — using the offset at the naive guess
/// and at the corrected one — and the one that reads BACK as the requested
/// local time wins. A spring-forward GAP has no such candidate, because the
/// wall clock never showed that time; 21.4.1.26 leaves the choice to the
/// implementation and V8 takes the offset from BEFORE the transition, which
/// pushes the result past it: local 02:30 on a spring-forward day is 03:30.
fn utc_from_local(local: f64) -> f64 {
    if !local.is_finite() {
        return local;
    }
    let off_naive = zone_offset_ms(local);
    let cand_a = local - off_naive;
    let off_corrected = zone_offset_ms(cand_a);
    if off_corrected == off_naive {
        return cand_a;
    }
    let cand_b = local - off_corrected;
    if local_ms(cand_b) == local {
        return cand_b;
    }
    if local_ms(cand_a) == local {
        return cand_a;
    }
    local - off_naive.min(off_corrected)
}

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

/// `00:00:00 GMT+0000 (Coordinated Universal Time)` — the `toTimeString` form
/// (21.4.4.42 TimeString + TimeZoneString). The clock is LOCAL and the offset
/// is the zone's real one; both were fixed at UTC before.
///
/// The parenthetical is the zone's LONG name, which node takes from ICU. There
/// is none here, so only UTC — the one name that is not data — is spelled out
/// and every other zone reports the abbreviation `localtime_r` supplies
/// (`GMT-0500 (EST)` where node writes `(Eastern Standard Time)`). Recorded in
/// BUGS.md.
fn time_string(ms: f64) -> String {
    if ms.is_nan() {
        return "Invalid Date".into();
    }
    let local = local_ms(ms);
    let off_min = (zone_offset_ms(ms) / 60_000.0) as i64;
    let sign = if off_min < 0 { '-' } else { '+' };
    let abs = off_min.abs();
    format!(
        "{:02}:{:02}:{:02} GMT{}{:02}{:02} ({})",
        field(local, Field::Hours) as i64,
        field(local, Field::Minutes) as i64,
        field(local, Field::Seconds) as i64,
        sign,
        abs / 60,
        abs % 60,
        zone_name(ms),
    )
}

/// The zone's display name for `toString`. UTC is spelled out the way node
/// does; anything else falls back to the abbreviation.
#[cfg(unix)]
fn zone_name(ms: f64) -> String {
    if zone_offset_ms(ms) == 0.0 {
        return "Coordinated Universal Time".into();
    }
    let secs = (ms / 1000.0).floor() as i64;
    let t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: as in `zone_offset_ms`.
    if unsafe { libc::localtime_r(&t, &mut tm).is_null() } || tm.tm_zone.is_null() {
        return "Coordinated Universal Time".into();
    }
    // SAFETY: `tm_zone` points at a static zone-name string owned by libc.
    let z = unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) };
    z.to_string_lossy().into_owned()
}

#[cfg(not(unix))]
fn zone_name(_ms: f64) -> String {
    "Coordinated Universal Time".into()
}

/// TimeClip (21.4.1.31): a time value more than 8.64e15 ms from the epoch is not
/// representable and becomes NaN; anything else truncates toward zero.
///
/// Without this a `new Date(8.64e15 + 1)` kept the out-of-range value and
/// printed a real date (`Sat, 13 Sep 275760 …`) where node prints
/// `Invalid Date`, so the boundary every date-range check relies on was absent.
fn time_clip(ms: f64) -> f64 {
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return f64::NAN;
    }
    ms.trunc()
}

/// Write a time value into the receiver's hidden `@@ms` slot, returning it (as
/// every mutator does).
fn store_ms(recv: &Value, ms: f64) -> f64 {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert("@@ms".into(), Value::Float(ms));
        }
    });
    ms
}

/// The shared body of every component setter.
///
/// `start` indexes the field the setter names within
/// `[year, month, date, hours, minutes, seconds, ms]`; the setter consumes that
/// field and every LOWER-order one in its own group (date fields 0..2, time
/// fields 3..6), defaulting anything not supplied from the current time value.
///
/// `legacy_year` applies Annex B `setYear`'s 0..99 → 1900..1999 mapping.
///
/// NaN handling follows the spec's split: `setFullYear` on an invalid date
/// treats the time value as +0 and so can REVIVE it (21.4.4.21 step 2), while
/// every other setter leaves an invalid date invalid.
/// `set_fields` on the LOCAL wall clock: read the current fields in local time,
/// replace the ones given, then convert the result back to a timestamp.
fn set_fields_local(recv: &Value, ms: f64, start: usize, args: &[Value], legacy_year: bool) -> f64 {
    if ms.is_nan() && start != 0 {
        return store_ms(recv, f64::NAN);
    }
    let local = set_fields_value(local_ms(ms), start, args, legacy_year);
    store_ms(recv, time_clip(utc_from_local(local)))
}

fn set_fields(recv: &Value, ms: f64, start: usize, args: &[Value], legacy_year: bool) -> f64 {
    if ms.is_nan() && start != 0 {
        return store_ms(recv, f64::NAN);
    }
    store_ms(
        recv,
        time_clip(set_fields_value(ms, start, args, legacy_year)),
    )
}

/// The field replacement itself, on whatever time value it is handed — the
/// local wall clock for a `setHours`, the timestamp for a `setUTCHours`. Split
/// out so the two differ only in what they pass in and what they do with the
/// result.
fn set_fields_value(ms: f64, start: usize, args: &[Value], legacy_year: bool) -> f64 {
    let base = if ms.is_nan() {
        0.0 // setFullYear/setYear on an Invalid Date starts from the epoch.
    } else {
        ms
    };
    let mut f = [
        field(base, Field::Year),
        field(base, Field::Month),
        field(base, Field::Day),
        field(base, Field::Hours),
        field(base, Field::Minutes),
        field(base, Field::Seconds),
        field(base, Field::Millis),
    ];
    // A date setter reaches at most field 2; a time setter at most field 6.
    let end = if start < 3 { 3 } else { 7 };
    for (i, slot) in f.iter_mut().enumerate().take(end).skip(start) {
        match args.get(i - start) {
            Some(v) => *slot = with_host(|h| h.to_number(v)).trunc(),
            None => break,
        }
    }
    if legacy_year && (0.0..=99.0).contains(&f[0]) {
        f[0] += 1900.0;
    }
    utc_from_fields(f[0], f[1], f[2], f[3], f[4], f[5], f[6])
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

/// `1/2/2020` — the `toLocaleDateString` default (en-US `M/D/YYYY`, no padding).
fn locale_date(ms: f64) -> String {
    let (day, _) = split_day(ms);
    let (y, mo, d) = civil_from_days(day);
    format!("{}/{}/{:04}", mo + 1, d, y)
}

/// `3:04:05 PM` — the `toLocaleTimeString` default (en-US 12-hour). Hour 0 and
/// hour 12 both render as `12`, which is why this is not `h % 12`.
fn locale_time(ms: f64) -> String {
    let h24 = field(ms, Field::Hours) as i64;
    let (h12, meridiem) = match h24 {
        0 => (12, "AM"),
        1..=11 => (h24, "AM"),
        12 => (12, "PM"),
        _ => (h24 - 12, "PM"),
    };
    format!(
        "{}:{:02}:{:02} {}",
        h12,
        field(ms, Field::Minutes) as i64,
        field(ms, Field::Seconds) as i64,
        meridiem
    )
}

/// The year field of an ISO-8601 date (21.4.4.36 `Date.prototype.toISOString`).
///
/// Years 0..=9999 are four digits; anything outside that range uses the EXPANDED
/// form — an explicit sign and exactly six digits, `+275760` / `-000001`. A bare
/// `{:04}` gets both wrong, since Rust counts the sign inside the width (`-1`
/// formats as `-001`) and never emits `+`.
fn iso_year(y: i64) -> String {
    if (0..=9999).contains(&y) {
        return format!("{y:04}");
    }
    let sign = if y < 0 { '-' } else { '+' };
    format!("{sign}{:06}", y.abs())
}

/// `2015-10-21T07:28:00.000Z` — the ISO-8601 / `toISOString` form.
fn iso_string(ms: f64) -> String {
    let (day, _) = split_day(ms);
    let (y, mo, d) = civil_from_days(day);
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        iso_year(y),
        mo + 1,
        d,
        field(ms, Field::Hours) as i64,
        field(ms, Field::Minutes) as i64,
        field(ms, Field::Seconds) as i64,
        field(ms, Field::Millis) as i64,
    )
}

/// Parse a date string: the Date Time String Format first ([`parse_iso`]),
/// then the free-form fallback every engine keeps ([`parse_legacy`]). NaN on
/// anything neither accepts — the "Invalid Date" contract.
///
/// The string is not trimmed first: V8 hands `" 2024-01-01 "` to the fallback,
/// which reads it as LOCAL midnight rather than the format's UTC one.
fn parse_str(s: &str) -> f64 {
    time_clip(parse_iso(s).or_else(|| parse_legacy(s)).unwrap_or(f64::NAN))
}

/// The Date Time String Format (21.4.1.32): `YYYY[-MM[-DD]]`, optionally
/// followed by `THH:mm[:ss[.sss]]`, optionally followed by a zone — `Z` or
/// `±HH:mm` (V8 also takes `±HHmm`). The year may be the expanded `±YYYYYY`.
///
/// A date-only form is UTC; a date-time form WITHOUT a zone is LOCAL time
/// (21.4.3.2 via the format's own note), which is why `"2024-01-01T10:20"` is
/// 15:20Z under `TZ=America/New_York`. The offset used to be dropped entirely:
/// `"…30.123+01:00"` read the fraction as `123` and ignored the rest, and
/// `"…00+05:30"` failed to parse. Fields out of range (`T25:00`, `T10:60`, a
/// month of 13) are NaN, as in V8; a day up to 31 rolls over as V8's does
/// (`2023-02-29` is March 1).
fn parse_iso(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    // `n` ASCII digits at the cursor, as a number.
    let digits = |i: &mut usize, n: usize| -> Option<i64> {
        let end = *i + n;
        let part = b.get(*i..end)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        *i = end;
        std::str::from_utf8(part).ok()?.parse().ok()
    };
    let year = match b.first()? {
        sign @ (b'+' | b'-') => {
            i = 1;
            let y = digits(&mut i, 6)?;
            // `-000000` is the one spelling the format forbids.
            if *sign == b'-' && y == 0 {
                return None;
            }
            if *sign == b'-' {
                -y
            } else {
                y
            }
        }
        _ => digits(&mut i, 4)?,
    };
    let (mut month, mut day) = (1, 1);
    if b.get(i) == Some(&b'-') {
        i += 1;
        month = digits(&mut i, 2)?;
        if b.get(i) == Some(&b'-') {
            i += 1;
            day = digits(&mut i, 2)?;
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let date = days_from_civil(year, month - 1, day) as f64 * MS_PER_DAY;
    // A date-only form, bare or with `Z`, is UTC.
    match b.get(i) {
        None => return Some(date),
        Some(b'Z' | b'z') if i + 1 == b.len() => return Some(date),
        Some(b'T' | b't' | b' ') => i += 1,
        Some(_) => return None,
    }
    let h = digits(&mut i, 2)?;
    if b.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    let mi = digits(&mut i, 2)?;
    let (mut sec, mut milli) = (0, 0);
    if b.get(i) == Some(&b':') {
        i += 1;
        sec = digits(&mut i, 2)?;
        if b.get(i) == Some(&b'.') {
            i += 1;
            let start = i;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if i == start {
                return None;
            }
            // Only the milliseconds are kept; further digits are truncated.
            let frac = &s[start..i.min(start + 3)];
            milli = format!("{frac:0<3}").parse().ok()?;
        }
    }
    // `24:00` is the end of the day and nothing past it is.
    if h > 24 || mi > 59 || sec > 59 || (h == 24 && (mi, sec, milli) != (0, 0, 0)) {
        return None;
    }
    let wall = date
        + h as f64 * 3_600_000.0
        + mi as f64 * 60_000.0
        + sec as f64 * 1000.0
        + milli as f64;
    let offset = match b.get(i) {
        None => return Some(utc_from_local(wall)),
        Some(b'Z' | b'z') if i + 1 == b.len() => 0.0,
        Some(sign @ (b'+' | b'-')) => {
            let sign = if *sign == b'-' { -1.0 } else { 1.0 };
            i += 1;
            let oh = digits(&mut i, 2)?;
            if b.get(i) == Some(&b':') {
                i += 1;
            }
            let om = digits(&mut i, 2)?;
            if i != b.len() || oh > 23 || om > 59 {
                return None;
            }
            sign * (oh as f64 * 3_600_000.0 + om as f64 * 60_000.0)
        }
        Some(_) => return None,
    };
    Some(wall - offset)
}

/// One token of a free-form date string, as V8's `DateStringTokenizer` splits
/// it: a run of digits (with its length, which decides how an offset reads), a
/// run of letters, one other character, or white space. A parenthesized
/// comment — `(Eastern Standard Time)` — is skipped whole.
#[derive(Clone, Copy, PartialEq)]
enum Tok<'a> {
    Num(i64, usize),
    Word(&'a str),
    Sym(u8),
    Space,
    End,
}

struct Toks<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Toks<'a> {
    fn next(&mut self) -> Tok<'a> {
        let b = self.s.as_bytes();
        let Some(&c) = b.get(self.i) else {
            return Tok::End;
        };
        let start = self.i;
        let run = |i: &mut usize, f: fn(&u8) -> bool| {
            while b.get(*i).is_some_and(f) {
                *i += 1;
            }
        };
        if c.is_ascii_digit() {
            run(&mut self.i, u8::is_ascii_digit);
            let text = &self.s[start..self.i];
            // A number too long for any field is still one token; its value
            // only has to be large enough to fail every range check.
            return Tok::Num(text.parse().unwrap_or(i64::MAX), text.len());
        }
        if c.is_ascii_alphabetic() {
            run(&mut self.i, u8::is_ascii_alphabetic);
            return Tok::Word(&self.s[start..self.i]);
        }
        if c.is_ascii_whitespace() {
            run(&mut self.i, u8::is_ascii_whitespace);
            return Tok::Space;
        }
        if c == b'(' {
            let mut depth = 0;
            while let Some(&c) = b.get(self.i) {
                self.i += 1;
                match c {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            return Tok::Space;
        }
        // One character, whole: a non-ASCII one is ignored like any symbol.
        let len = self.s[start..].chars().next().map_or(1, char::len_utf8);
        self.i += len;
        Tok::Sym(if len == 1 { c } else { 0 })
    }

    fn peek(&self) -> Tok<'a> {
        Toks { s: self.s, i: self.i }.next()
    }

    fn skip(&mut self, sym: u8) -> bool {
        let taken = self.peek() == Tok::Sym(sym);
        if taken {
            self.next();
        }
        taken
    }
}

/// What a word means to the fallback parser: V8's `KeywordTable`, matched on
/// the first three letters (so `January` and `Janu` are both January, and `Ja`
/// is nothing), with the short entries matched whole.
enum Keyword {
    Month(i64),
    AmPm(i64),
    /// A zone: its offset from UTC in hours.
    Zone(i64),
    Other,
}

fn keyword(word: &str) -> Keyword {
    let w = word.to_ascii_lowercase();
    let prefix = &w[..w.len().min(3)];
    if w.len() >= 3 {
        const MONTHS: [&str; 12] = [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ];
        if let Some(m) = MONTHS.iter().position(|m| *m == prefix) {
            return Keyword::Month(m as i64 + 1);
        }
    }
    match prefix {
        "am" if w.len() == 2 => Keyword::AmPm(0),
        "pm" if w.len() == 2 => Keyword::AmPm(12),
        "ut" if w.len() == 2 => Keyword::Zone(0),
        "z" => Keyword::Zone(0),
        "utc" | "gmt" if w.len() >= 3 => Keyword::Zone(0),
        "cdt" => Keyword::Zone(-5),
        "cst" => Keyword::Zone(-6),
        "edt" => Keyword::Zone(-4),
        "est" => Keyword::Zone(-5),
        "mdt" => Keyword::Zone(-6),
        "mst" => Keyword::Zone(-7),
        "pdt" => Keyword::Zone(-7),
        "pst" => Keyword::Zone(-8),
        _ => Keyword::Other,
    }
}

/// The free-form fallback — V8's legacy `DateParser` loop, which is what makes
/// `new Date("March 7, 2024 10:00")`, `"1/5/2024"`, `"Oct 21, 2015 7:28 PM"`
/// and the `toString` form `"Thu Mar 07 2024 10:00:00 GMT-0500 (…)"` dates.
/// Only the IMF-fixdate header form was read before, so every one of those was
/// an Invalid Date.
///
/// Numbers fill the date (`DayComposer`), a number followed by `:` starts the
/// time (`TimeComposer`), and a zone word or a sign after the time sets the
/// offset (`TimeZoneComposer`). With no zone the result is LOCAL time.
fn parse_legacy(s: &str) -> Option<f64> {
    const NONE: i64 = i64::MIN;
    let mut t = Toks { s, i: 0 };
    let (mut day, mut named_month) = (Vec::with_capacity(3), NONE);
    let (mut time, mut hour_offset) = (Vec::with_capacity(4), NONE);
    let (mut sign, mut tz_hour, mut tz_min) = (0i64, NONE, NONE);
    let mut read_number = false;
    // `TimeComposer::IsExpecting`: the next field the time can take.
    let expecting = |time: &Vec<i64>, n: i64| match time.len() {
        1 | 2 => (0..60).contains(&n),
        3 => (0..1000).contains(&n),
        _ => false,
    };
    loop {
        match t.next() {
            Tok::End => break,
            Tok::Num(n, _) => {
                read_number = true;
                if t.skip(b':') {
                    if t.skip(b':') {
                        if !time.is_empty() {
                            return None;
                        }
                        time.extend([n, 0]);
                    } else {
                        if time.len() >= 4 {
                            return None;
                        }
                        time.push(n);
                        t.skip(b'.');
                    }
                } else if t.peek() == Tok::Sym(b'.') && expecting(&time, n) {
                    t.next();
                    time.push(n);
                    let Tok::Num(ms, len) = t.next() else {
                        return None;
                    };
                    // Milliseconds are the first three digits, scaled.
                    let ms = match len {
                        1 => ms * 100,
                        2 => ms * 10,
                        3 => ms,
                        _ => t.s[t.i - len..t.i - len + 3].parse().ok()?,
                    };
                    time.push(ms);
                    time.resize(4, 0);
                } else if tz_hour != NONE && tz_min == NONE && (0..60).contains(&n) {
                    tz_min = n;
                } else if expecting(&time, n) {
                    time.push(n);
                    time.resize(4, 0);
                    // The time must end at the end, white space, `Z` or a sign.
                    match t.peek() {
                        Tok::End | Tok::Space | Tok::Sym(b'+' | b'-') => {}
                        Tok::Word(w) if w.eq_ignore_ascii_case("z") => {}
                        _ => return None,
                    }
                } else {
                    if day.len() >= 3 {
                        return None;
                    }
                    day.push(n);
                    t.skip(b'-');
                }
            }
            Tok::Word(w) => match keyword(w) {
                Keyword::AmPm(off) if !time.is_empty() => hour_offset = off,
                Keyword::Month(m) => {
                    named_month = m;
                    t.skip(b'-');
                }
                Keyword::Zone(h) if read_number => {
                    sign = if h < 0 { -1 } else { 1 };
                    tz_hour = h.abs();
                    tz_min = 0;
                }
                _ => {
                    // A stray word is refused once a number has been read,
                    // and must be kept apart from the first number.
                    if read_number || matches!(t.peek(), Tok::Num(..)) {
                        return None;
                    }
                }
            },
            Tok::Sym(c @ (b'+' | b'-')) if (sign != 0 && tz_hour == 0 && tz_min == 0) || !time.is_empty() => {
                // An offset, only after a UTC word or a time: `+05`, `+0530`,
                // `+05:30`.
                sign = if c == b'-' { -1 } else { 1 };
                let (n, len) = match t.peek() {
                    Tok::Num(n, len) => {
                        t.next();
                        (n, len)
                    }
                    _ => (0, 0),
                };
                read_number = true;
                if t.peek() == Tok::Sym(b':') {
                    tz_hour = n;
                    tz_min = NONE;
                } else if len <= 2 {
                    tz_hour = n;
                    tz_min = 0;
                } else if len <= 4 {
                    tz_hour = n / 100;
                    tz_min = n % 100;
                } else {
                    return None;
                }
            }
            Tok::Sym(b'+' | b'-' | b')') if read_number => return None,
            Tok::Sym(_) | Tok::Space => {}
        }
    }

    // `DayComposer::Write`: the missing components are 1, and which one is the
    // year is decided by whether the first can be a day at all.
    if day.is_empty() {
        return None;
    }
    day.resize(3, 1);
    let is_day = |n: i64| (1..=31).contains(&n);
    let (mut year, month, dd) = if named_month == NONE {
        if is_day(day[0]) {
            (day[2], day[0], day[1])
        } else {
            (day[0], day[1], day[2])
        }
    } else if !is_day(day[0]) {
        (day[0], named_month, day[1])
    } else {
        (day[1], named_month, day[0])
    };
    if (0..=49).contains(&year) {
        year += 2000;
    } else if (50..=99).contains(&year) {
        year += 1900;
    }
    if !(1..=12).contains(&month) || !is_day(dd) {
        return None;
    }

    // `TimeComposer::Write`: missing fields are 0; AM/PM needs an hour of
    // 0-12; hour 24 only as the very end of a day.
    time.resize(4, 0);
    let (mut h, mi, sec, ms) = (time[0], time[1], time[2], time[3]);
    if hour_offset != NONE {
        if !(0..=12).contains(&h) {
            return None;
        }
        h = h % 12 + hour_offset;
    }
    let in_range = (0..24).contains(&h) && (0..60).contains(&mi) && (0..60).contains(&sec) && (0..1000).contains(&ms);
    if !in_range && (h, mi, sec, ms) != (24, 0, 0, 0) {
        return None;
    }

    let wall = utc_from_fields(
        year as f64,
        (month - 1) as f64,
        dd as f64,
        h as f64,
        mi as f64,
        sec as f64,
        ms as f64,
    );
    if sign == 0 {
        return Some(utc_from_local(wall));
    }
    let tz_min = if tz_min == NONE { 0 } else { tz_min };
    let tz_hour = if tz_hour == NONE { 0 } else { tz_hour };
    Some(wall - sign as f64 * (tz_hour * 3_600_000 + tz_min * 60_000) as f64)
}

/// A Date's `util.inspect` rendering, resolved against an ALREADY-BORROWED host.
///
/// Node prints a Date as its ISO-8601 form — `console.log(new Date(1))` is
/// `1970-01-01T00:00:00.001Z`, not an object literal — and prints the string
/// `Invalid Date` for a NaN time value. Without this the inspect walk reached a
/// Date through the generic object branch, found its time value in the internal
/// `@@ms` slot rather than in an enumerable property, and rendered every Date
/// ever logged as `{}`.
///
/// Takes `&JsHost` rather than calling `with_host` because the inspect walk is
/// already inside that borrow; borrowing again aborts the process.
pub(crate) fn inspect_with_host(h: &crate::host::JsHost, v: &Value) -> String {
    let ms = match h.get(v) {
        Some(JsObj::Object(p)) => p.get("@@ms").map(|x| h.to_number(x)).unwrap_or(f64::NAN),
        _ => f64::NAN,
    };
    if ms.is_nan() {
        "Invalid Date".into()
    } else {
        iso_string(ms)
    }
}
