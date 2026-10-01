//! `Date.prototype.toLocaleString` / `toLocaleDateString` / `toLocaleTimeString`
//! with their `options` argument, in the `en-US` shape.
//!
//! Node formats these through ICU: ECMA-402's `CreateDateTimeFormat` turns the
//! options bag into a SKELETON (`"yMMMMEEEEd"`, `"hm"`, …), and ICU's
//! `DateTimePatternGenerator` turns the skeleton into a PATTERN
//! (`"EEEE, MMMM d, y"`) by a distance search over the locale's
//! `availableFormats`, adjusting field widths, appending fields no pattern
//! carries and gluing a date half to a time half. That is what this module is: a
//! port of `dtptngen.cpp`'s `getBestPattern` / `getBestRaw` /
//! `getBestAppending` / `adjustFieldTypes` over the en data ICU 77 ships, so
//! every option combination resolves to the pattern node's does — including the
//! odd ones, like `{day: '2-digit', hour: '2-digit', second: 'numeric'}` printing
//! `09, 9 (hour: 13)`.
//!
//! The `locales` argument is still ignored: every locale formats as en-US, as
//! `numfmt` does for numbers (see BUGS.md).

use crate::host::{self, with_host};
use fusevm::Value;

// ICU's `UDateTimePatternField` indices.
const ERA: usize = 0;
const YEAR: usize = 1;
const QUARTER: usize = 2;
const MONTH: usize = 3;
const WEEK_OF_YEAR: usize = 4;
const WEEK_OF_MONTH: usize = 5;
const WEEKDAY: usize = 6;
const DAY_OF_YEAR: usize = 7;
const DAY_OF_WEEK_IN_MONTH: usize = 8;
const DAY: usize = 9;
const DAYPERIOD: usize = 10;
const HOUR: usize = 11;
const MINUTE: usize = 12;
const SECOND: usize = 13;
const FRACTIONAL: usize = 14;
const ZONE: usize = 15;
const FIELD_COUNT: usize = 16;

const DATE_MASK: u32 = (1 << DAYPERIOD) - 1;
const TIME_MASK: u32 = ((1 << FIELD_COUNT) - 1) & !DATE_MASK;
const SECOND_AND_FRACTIONAL: u32 = (1 << SECOND) | (1 << FRACTIONAL);

// ICU's field "types": numeric types are positive (and the field LENGTH is added
// to them in a skeleton), text widths negative; the distance between two
// skeletons is the sum of the type differences.
const NUMERIC: i32 = 0x100;
const DELTA: i32 = 0x10;
const NARROW: i32 = -0x101;
const SHORTER: i32 = -0x102;
const SHORT: i32 = -0x103;
const LONG: i32 = -0x104;
const EXTRA_FIELD: i32 = 0x10000;
const MISSING_FIELD: i32 = 0x1000;

/// ICU's `dtTypes` table, in its order: `(pattern char, field, type, min length)`.
/// A run of `c` repeated `n` times is the LAST row for `c` whose min length is
/// at most `n`.
const TYPES: &[(char, usize, i32, usize)] = &[
    ('G', ERA, SHORT, 1),
    ('G', ERA, LONG, 4),
    ('G', ERA, NARROW, 5),
    ('y', YEAR, NUMERIC, 1),
    ('Q', QUARTER, NUMERIC, 1),
    ('Q', QUARTER, SHORT, 3),
    ('Q', QUARTER, LONG, 4),
    ('Q', QUARTER, NARROW, 5),
    ('M', MONTH, NUMERIC, 1),
    ('M', MONTH, SHORT, 3),
    ('M', MONTH, LONG, 4),
    ('M', MONTH, NARROW, 5),
    ('L', MONTH, NUMERIC + DELTA, 1),
    ('L', MONTH, SHORT - DELTA, 3),
    ('L', MONTH, LONG - DELTA, 4),
    ('L', MONTH, NARROW - DELTA, 5),
    ('w', WEEK_OF_YEAR, NUMERIC, 1),
    ('W', WEEK_OF_MONTH, NUMERIC, 1),
    ('E', WEEKDAY, SHORT, 1),
    ('E', WEEKDAY, LONG, 4),
    ('E', WEEKDAY, NARROW, 5),
    ('E', WEEKDAY, SHORTER, 6),
    ('c', WEEKDAY, NUMERIC + 2 * DELTA, 1),
    ('c', WEEKDAY, SHORT - 2 * DELTA, 3),
    ('c', WEEKDAY, LONG - 2 * DELTA, 4),
    ('c', WEEKDAY, NARROW - 2 * DELTA, 5),
    ('c', WEEKDAY, SHORTER - 2 * DELTA, 6),
    ('d', DAY, NUMERIC, 1),
    ('D', DAY_OF_YEAR, NUMERIC, 1),
    ('F', DAY_OF_WEEK_IN_MONTH, NUMERIC, 1),
    ('a', DAYPERIOD, SHORT, 1),
    ('a', DAYPERIOD, LONG, 4),
    ('a', DAYPERIOD, NARROW, 5),
    ('B', DAYPERIOD, SHORT - 3 * DELTA, 1),
    ('B', DAYPERIOD, LONG - 3 * DELTA, 4),
    ('B', DAYPERIOD, NARROW - 3 * DELTA, 5),
    ('H', HOUR, NUMERIC + 10 * DELTA, 1),
    ('k', HOUR, NUMERIC + 11 * DELTA, 1),
    ('h', HOUR, NUMERIC, 1),
    ('K', HOUR, NUMERIC + DELTA, 1),
    ('m', MINUTE, NUMERIC, 1),
    ('s', SECOND, NUMERIC, 1),
    ('S', FRACTIONAL, NUMERIC, 1),
    ('v', ZONE, SHORT - 2 * DELTA, 1),
    ('v', ZONE, LONG - 2 * DELTA, 4),
    ('z', ZONE, SHORT, 1),
    ('z', ZONE, LONG, 4),
    ('O', ZONE, SHORT - 3 * DELTA, 1),
    ('O', ZONE, LONG - 3 * DELTA, 4),
];

/// The `dtTypes` row a run of `c` × `len` belongs to (`getCanonicalIndex`).
fn type_row(c: char, len: usize) -> Option<(usize, i32, usize)> {
    TYPES
        .iter()
        .rev()
        .find(|r| r.0 == c && r.3 <= len)
        .or_else(|| TYPES.iter().find(|r| r.0 == c))
        .map(|r| (r.1, r.2, r.3))
}

/// One token of a pattern: a field (a run of one letter), or literal text kept
/// verbatim — a quoted run with its quotes, or punctuation and spaces.
#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Field(char, usize),
    Lit(String),
}

fn tokenize(pattern: &str) -> Vec<Tok> {
    let cs: Vec<char> = pattern.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_ascii_alphabetic() {
            let start = i;
            while i < cs.len() && cs[i] == c {
                i += 1;
            }
            out.push(Tok::Field(c, i - start));
        } else if c == '\'' {
            // A quoted literal runs to the next lone quote; `''` is a quote.
            let start = i;
            i += 1;
            while i < cs.len() {
                if cs[i] == '\'' {
                    if cs.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(Tok::Lit(cs[start..i].iter().collect()));
        } else {
            out.push(Tok::Lit(c.to_string()));
            i += 1;
        }
    }
    out
}

/// A skeleton (ICU's `PtnSkeleton`): per field, the run that names it, its
/// type, and the length of its BASE form (the row's minimum length).
#[derive(Clone, Debug, PartialEq, Default)]
struct Skeleton {
    orig: [Option<(char, usize)>; FIELD_COUNT],
    ty: [i32; FIELD_COUNT],
    base: [Option<(char, usize)>; FIELD_COUNT],
    /// The `a` ICU adds to a 12-hour skeleton that names no day period.
    added_day_period: bool,
}

impl Skeleton {
    /// `DateTimeMatcher::set`: the skeleton a pattern (or skeleton string)
    /// denotes. Literals are skipped.
    fn of(pattern: &str) -> Skeleton {
        let mut s = Skeleton::default();
        for t in tokenize(pattern) {
            let Tok::Field(c, len) = t else { continue };
            let Some((field, ty, min)) = type_row(c, len) else {
                continue;
            };
            s.orig[field] = Some((c, len));
            let base_char = TYPES
                .iter()
                .find(|r| r.1 == field && r.2 == ty)
                .map_or(c, |r| r.0);
            s.base[field] = Some((base_char, min));
            s.ty[field] = if ty > 0 { ty + len as i32 } else { ty };
        }
        // ICU #20739: minutes with fractional seconds but no seconds force a
        // seconds field, so the fraction has something to follow.
        if s.orig[MINUTE].is_some() && s.orig[FRACTIONAL].is_some() && s.orig[SECOND].is_none() {
            s.orig[SECOND] = Some(('s', 1));
            s.base[SECOND] = Some(('s', 1));
            s.ty[SECOND] = NUMERIC + 1;
        }
        // A 12-hour skeleton without a day period gets ICU's default `a`; a
        // 24-hour one drops any day period it names.
        if let Some((h, _)) = s.orig[HOUR] {
            if h == 'h' || h == 'K' {
                if s.orig[DAYPERIOD].is_none() {
                    s.orig[DAYPERIOD] = Some(('a', 1));
                    s.base[DAYPERIOD] = Some(('a', 1));
                    s.ty[DAYPERIOD] = SHORT;
                    s.added_day_period = true;
                }
            } else if s.orig[DAYPERIOD].is_some() {
                s.orig[DAYPERIOD] = None;
                s.base[DAYPERIOD] = None;
                s.ty[DAYPERIOD] = 0;
            }
        }
        s
    }

    fn mask(&self) -> u32 {
        (0..FIELD_COUNT)
            .filter(|&i| self.ty[i] != 0)
            .fold(0, |m, i| m | (1 << i))
    }

    /// The base pattern's text (`getBasePattern`), which keys ICU's buckets.
    fn base_string(&self) -> String {
        self.base
            .iter()
            .flatten()
            .map(|&(c, n)| c.to_string().repeat(n))
            .collect()
    }

    /// `DateTimeMatcher::getDistance`, restricted to the fields in `include`.
    fn distance(&self, other: &Skeleton, include: u32) -> (i32, u32, u32) {
        let (mut d, mut missing, mut extra) = (0, 0u32, 0u32);
        for i in 0..FIELD_COUNT {
            let mine = if include & (1 << i) == 0 {
                0
            } else {
                self.ty[i]
            };
            let theirs = other.ty[i];
            if mine == theirs {
                continue;
            }
            if mine == 0 {
                d += EXTRA_FIELD;
                extra |= 1 << i;
            } else if theirs == 0 {
                d += MISSING_FIELD;
                missing |= 1 << i;
            } else {
                d += (mine - theirs).abs();
            }
        }
        (d, missing, extra)
    }
}

/// One generator entry: the skeleton it is matched by, its pattern, and
/// whether the skeleton was given explicitly (an `availableFormats` key) or
/// derived from the pattern (a canonical item or a style pattern).
struct Entry {
    skel: Skeleton,
    pattern: &'static str,
    specified: bool,
}

/// en's `availableFormats` (ICU 77 / CLDR 47), in the resource bundle's key
/// order. The week-numbering items are left out: ECMA-402 never asks for a
/// week field, and they carry fields no request has, so they never win.
const AVAILABLE_FORMATS: &[(&str, &str)] = &[
    ("Bh", "h B"),
    ("Bhm", "h:mm B"),
    ("Bhms", "h:mm:ss B"),
    ("E", "ccc"),
    ("EBh", "E h B"),
    ("EBhm", "E h:mm B"),
    ("EBhms", "E h:mm:ss B"),
    ("EHm", "E HH:mm"),
    ("EHms", "E HH:mm:ss"),
    ("Ed", "d E"),
    ("Eh", "E h a"),
    ("Ehm", "E h:mm a"),
    ("Ehms", "E h:mm:ss a"),
    ("Gy", "y G"),
    ("GyM", "M/y G"),
    ("GyMEd", "E, M/d/y G"),
    ("GyMMM", "MMM y G"),
    ("GyMMMEd", "E, MMM d, y G"),
    ("GyMMMd", "MMM d, y G"),
    ("GyMd", "M/d/y G"),
    ("H", "HH"),
    ("Hm", "HH:mm"),
    ("Hms", "HH:mm:ss"),
    ("Hmsv", "HH:mm:ss v"),
    ("Hmv", "HH:mm v"),
    ("Hv", "HH v"),
    ("M", "L"),
    ("MEd", "E, M/d"),
    ("MMM", "LLL"),
    ("MMMEd", "E, MMM d"),
    ("MMMMd", "MMMM d"),
    ("MMMd", "MMM d"),
    ("Md", "M/d"),
    ("d", "d"),
    ("h", "h a"),
    ("hm", "h:mm a"),
    ("hms", "h:mm:ss a"),
    ("hmsv", "h:mm:ss a v"),
    ("hmv", "h:mm a v"),
    ("hv", "h a v"),
    ("ms", "mm:ss"),
    ("y", "y"),
    ("yM", "M/y"),
    ("yMEd", "E, M/d/y"),
    ("yMMM", "MMM y"),
    ("yMMMEd", "E, MMM d, y"),
    ("yMMMM", "MMMM y"),
    ("yMMMd", "MMM d, y"),
    ("yMd", "M/d/y"),
    ("yQQQ", "QQQ y"),
    ("yQQQQ", "QQQQ y"),
];

/// en's date styles (`full` … `short`) and time styles. ICU's own patterns put
/// U+202F before the day period; V8 swaps it for a space, so these do too.
const DATE_STYLES: [&str; 4] = ["EEEE, MMMM d, y", "MMMM d, y", "MMM d, y", "M/d/yy"];
const TIME_STYLES: [&str; 4] = ["h:mm:ss a zzzz", "h:mm:ss a z", "h:mm:ss a", "h:mm a"];

/// en's `dateTimeFormats` (the `atTime` set, which is what the generator and
/// the styles use), indexed like the styles.
const DATE_TIME_GLUE: [&str; 4] = ["{1} 'at' {0}", "{1} 'at' {0}", "{1}, {0}", "{1}, {0}"];

/// The generator's entries in ICU's ITERATION order, built as `initData` does:
/// canonical one-field items, then the style patterns, then the `mm:ss` hack,
/// all without override; then `availableFormats`, overriding. Iteration walks
/// ICU's buckets — keyed by the base pattern's first letter, `A`–`Z` then
/// `a`–`z` — each in insertion order, and that order breaks distance ties.
fn entries() -> &'static [Entry] {
    static ENTRIES: std::sync::OnceLock<Vec<Entry>> = std::sync::OnceLock::new();
    ENTRIES.get_or_init(|| {
        let mut list: Vec<Entry> = Vec::new();
        // `addPatternWithSkeleton` with `override = false` and no skeleton: a
        // BASE conflict or an identical skeleton keeps the earlier entry.
        let add_derived = |list: &mut Vec<Entry>, pattern: &'static str| {
            let skel = Skeleton::of(pattern);
            let base = skel.base_string();
            if list.iter().any(|e| e.skel.base_string() == base) {
                return;
            }
            list.push(Entry {
                skel,
                pattern,
                specified: false,
            });
        };
        const CANONICAL: &[&str] = &[
            "G", "y", "Q", "M", "w", "W", "E", "D", "F", "d", "a", "H", "m", "s", "S", "v",
        ];
        for p in CANONICAL {
            add_derived(&mut list, p);
        }
        for i in 0..4 {
            add_derived(&mut list, DATE_STYLES[i]);
            add_derived(&mut list, TIME_STYLES[i]);
        }
        // `hackTimes` on the medium time pattern.
        add_derived(&mut list, "mm:ss");
        for &(key, pattern) in AVAILABLE_FORMATS {
            let skel = Skeleton::of(key);
            match list.iter_mut().find(|e| e.skel == skel) {
                Some(e) => {
                    e.pattern = pattern;
                    e.specified = true;
                }
                None => list.push(Entry {
                    skel,
                    pattern,
                    specified: true,
                }),
            }
        }
        let bucket = |e: &Entry| {
            let c = e.skel.base_string().chars().next().unwrap_or('a');
            if c.is_ascii_uppercase() {
                c as u32 - 'A' as u32
            } else {
                26 + c as u32 - 'a' as u32
            }
        };
        list.sort_by_key(bucket);
        list
    })
}

/// `getBestRaw`: the entry nearest `req` over the fields in `include`, with
/// the missing and extra field masks of that match.
fn best_raw(req: &Skeleton, include: u32) -> (&'static Entry, u32, u32) {
    let mut best: Option<(&Entry, i32, u32, u32)> = None;
    for e in entries() {
        let (d, missing, extra) = req.distance(&e.skel, include);
        let better = match best {
            None => true,
            Some((_, bd, bmissing, _)) => d < bd || (d == bd && bmissing < missing),
        };
        if better {
            best = Some((e, d, missing, extra));
            if d == 0 {
                break;
            }
        }
    }
    let (e, _, missing, extra) = best.expect("the generator has entries");
    (e, missing, extra)
}

/// `adjustFieldTypes` with `UDATPG_MATCH_HOUR_FIELD_LENGTH` (what V8 passes):
/// each field the request names takes the request's width, except where ICU
/// keeps the pattern's own.
fn adjust(
    pattern: &str,
    entry_skel: Option<&Skeleton>,
    req: &Skeleton,
    fix_fraction: bool,
) -> String {
    let mut out = String::new();
    for t in tokenize(pattern) {
        let (c, len) = match t {
            Tok::Lit(s) => {
                out.push_str(&s);
                continue;
            }
            Tok::Field(c, len) => (c, len),
        };
        let Some((field, row_ty, _)) = type_row(c, len) else {
            out.push_str(&c.to_string().repeat(len));
            continue;
        };
        if fix_fraction && field == SECOND {
            out.push_str(&c.to_string().repeat(len));
            out.push('.');
            if let Some((fc, flen)) = req.orig[FRACTIONAL] {
                out.push_str(&fc.to_string().repeat(flen));
            }
            continue;
        }
        if req.ty[field] == 0 {
            out.push_str(&c.to_string().repeat(len));
            continue;
        }
        let (req_char, mut req_len) = req.orig[field].unwrap_or((c, len));
        if req_char == 'E' && req_len < 3 {
            req_len = 3;
        }
        let mut adj_len = req_len;
        if field == MINUTE || field == SECOND {
            adj_len = len;
        } else if let Some(spec) = entry_skel {
            let skel_len = spec.orig[field].map_or(0, |(_, n)| n);
            let pat_numeric = row_ty > 0;
            let skel_numeric = spec.ty[field] > 0;
            if skel_len == req_len || pat_numeric != skel_numeric {
                adj_len = len;
            }
        }
        let keep_pattern_char = matches!(field, HOUR | MONTH | WEEKDAY | YEAR);
        let mut ch = if keep_pattern_char { c } else { req_char };
        if field == HOUR {
            // en's default hour character is `h`.
            ch = match req_char {
                'h' => 'h',
                'K' => 'h',
                _ => ch,
            };
        }
        out.push_str(&ch.to_string().repeat(adj_len));
    }
    out
}

/// `getBestAppending`: the pattern for the fields in `fields`, appending (with
/// en's `appendItems`) whatever the nearest pattern lacks.
fn best_appending(req: &Skeleton, fields: u32) -> String {
    if fields == 0 {
        return String::new();
    }
    let (e, mut missing, extra) = best_raw(req, fields);
    let spec = |e: &Entry| e.specified.then_some(&e.skel).cloned();
    let mut spec_skel = spec(e);
    let mut result = adjust(e.pattern, spec_skel.as_ref(), req, false);
    if missing == 0 && extra == 0 {
        return result;
    }
    let mut last_missing = 0;
    while missing != 0 {
        if last_missing == missing {
            break;
        }
        if missing & SECOND_AND_FRACTIONAL == 1 << FRACTIONAL
            && fields & SECOND_AND_FRACTIONAL == SECOND_AND_FRACTIONAL
        {
            result = adjust(&result, spec_skel.as_ref(), req, true);
            missing &= !(1 << FRACTIONAL);
            continue;
        }
        let starting = missing;
        let (e, m, _) = best_raw(req, missing);
        missing = m;
        spec_skel = spec(e);
        let temp = adjust(e.pattern, spec_skel.as_ref(), req, false);
        let found = starting & !missing;
        let top = 31 - found.leading_zeros() as usize;
        result = append_item(top, &result, &temp);
        last_missing = missing;
    }
    result
}

/// en's `appendItems` (from root), with each field's wide display name.
fn append_item(field: usize, result: &str, temp: &str) -> String {
    let name = match field {
        ERA => "era",
        YEAR => "year",
        QUARTER => "quarter",
        MONTH => "month",
        WEEK_OF_YEAR => "week",
        WEEK_OF_MONTH => "week of month",
        WEEKDAY => "day of the week",
        DAY_OF_YEAR => "day of year",
        DAY_OF_WEEK_IN_MONTH => "weekday of the month",
        DAY => "day",
        DAYPERIOD => "AM/PM",
        HOUR => "hour",
        MINUTE => "minute",
        SECOND => "second",
        FRACTIONAL => "F14",
        _ => "time zone",
    };
    match field {
        ERA | YEAR | WEEKDAY | ZONE => format!("{result} {temp}"),
        QUARTER | MONTH | WEEK_OF_YEAR | DAY | HOUR | MINUTE | SECOND => {
            format!("{result} ('{name}': {temp})")
        }
        _ => format!("{result} \u{251C}'{name}': {temp}\u{2524}"),
    }
}

/// `getBestPattern(skeleton, UDATPG_MATCH_HOUR_FIELD_LENGTH)`.
fn best_pattern(skeleton: &str) -> String {
    let req = Skeleton::of(skeleton);
    let (e, missing, extra) = best_raw(&req, u32::MAX);
    if missing == 0 && extra == 0 {
        return adjust(e.pattern, e.specified.then_some(&e.skel), &req, false);
    }
    let needed = req.mask();
    let date = best_appending(&req, needed & DATE_MASK);
    let time = best_appending(&req, needed & TIME_MASK);
    if date.is_empty() {
        return time;
    }
    if time.is_empty() {
        return date;
    }
    let month_len = req.base[MONTH].map_or(0, |(_, n)| n);
    let style = match month_len {
        4 if req.base[WEEKDAY].is_some() => 0,
        4 => 1,
        3 => 2,
        _ => 3,
    };
    glue(style, &date, &time)
}

fn glue(style: usize, date: &str, time: &str) -> String {
    DATE_TIME_GLUE[style]
        .replace("{1}", date)
        .replace("{0}", time)
}

// ── options ──────────────────────────────────────────────────────────────────

/// Which `toLocale*String` is asking: what it requires and defaults to
/// (ECMA-402 `ToDateTimeOptions`).
#[derive(Clone, Copy, PartialEq)]
pub enum Required {
    Any,
    Date,
    Time,
}

/// The hour cycle (`hourCycle` / `hour12`).
#[derive(Clone, Copy, PartialEq)]
enum Hc {
    H11,
    H12,
    H23,
    H24,
}

/// The zone a format renders in.
#[derive(Clone)]
enum Zone {
    /// The process's zone (`TZ`).
    Local,
    Utc,
    /// A fixed offset east of UTC, in minutes (`"+05:30"`, `Etc/GMT-5`).
    Fixed(i64),
    /// A named IANA zone from the system zoneinfo.
    Named(std::rc::Rc<crate::tzif::Tz>),
}

fn out_of_range(method: &str, value: &str, name: &str) -> String {
    host::range_error(&format!(
        "Value {value} out of range for {method} options property {name}"
    ))
}

fn string_option(
    opts: &Value,
    method: &str,
    name: &str,
    allowed: &[&str],
) -> Result<Option<String>, String> {
    let v = crate::builtins::get_property(opts, name)?;
    if matches!(v, Value::Undef) {
        return Ok(None);
    }
    let s = host::to_string_value(&v)?;
    let s = with_host(|h| h.str_of(&s));
    if !allowed.is_empty() && !allowed.contains(&s.as_str()) {
        return Err(out_of_range(method, &s, name));
    }
    Ok(Some(s))
}

/// A resolved format: the ICU pattern and the zone it renders in.
pub struct Format {
    pattern: String,
    zone: Zone,
}

const TEXT_WIDTHS: &[&str] = &["narrow", "short", "long"];
const NUMERIC_WIDTHS: &[&str] = &["numeric", "2-digit"];

/// `CreateDateTimeFormat` for the `toLocale*String` family: read and validate
/// the options bag in the order V8 does, apply `ToDateTimeOptions`, and resolve
/// the pattern.
pub fn resolve(opts: &Value, required: Required, method: &str) -> Result<Format, String> {
    if with_host(|h| h.is_null(opts)) {
        return Err(host::type_error(&format!(
            "{method} called on null or undefined"
        )));
    }
    let is_object = matches!(opts, Value::Obj(_)) && !with_host(|h| host::is_primitive(h, opts));
    let undef = Value::Undef;
    let o = if is_object { opts } else { &undef };
    let get = |name: &str, allowed: &[&str]| -> Result<Option<String>, String> {
        if is_object {
            string_option(o, method, name, allowed)
        } else {
            Ok(None)
        }
    };
    let hour12 = if is_object {
        let v = crate::builtins::get_property(o, "hour12")?;
        (!matches!(v, Value::Undef)).then(|| with_host(|h| h.truthy(&v)))
    } else {
        None
    };
    let hour_cycle = get("hourCycle", &["h11", "h12", "h23", "h24"])?;
    let time_zone = if is_object {
        let v = crate::builtins::get_property(o, "timeZone")?;
        if matches!(v, Value::Undef) {
            None
        } else {
            let s = host::to_string_value(&v)?;
            Some(with_host(|h| h.str_of(&s)))
        }
    } else {
        None
    };
    let weekday = get("weekday", TEXT_WIDTHS)?;
    let era = get("era", TEXT_WIDTHS)?;
    let mut year = get("year", NUMERIC_WIDTHS)?;
    let mut month = get("month", &["numeric", "2-digit", "narrow", "short", "long"])?;
    let mut day = get("day", NUMERIC_WIDTHS)?;
    let day_period = get("dayPeriod", TEXT_WIDTHS)?;
    let mut hour = get("hour", NUMERIC_WIDTHS)?;
    let mut minute = get("minute", NUMERIC_WIDTHS)?;
    let mut second = get("second", NUMERIC_WIDTHS)?;
    let fsd = if is_object {
        let v = crate::builtins::get_property(o, "fractionalSecondDigits")?;
        if matches!(v, Value::Undef) {
            None
        } else {
            let n = host::to_number_value(&v)?;
            if n.is_nan() || !(1.0..=3.0).contains(&n) {
                return Err(host::range_error(
                    "fractionalSecondDigits value is out of range.",
                ));
            }
            Some(n.floor() as usize)
        }
    } else {
        None
    };
    let tz_name = get(
        "timeZoneName",
        &[
            "short",
            "long",
            "shortOffset",
            "longOffset",
            "shortGeneric",
            "longGeneric",
        ],
    )?;
    let style_names = &["full", "long", "medium", "short"];
    let date_style = get("dateStyle", style_names)?;
    let time_style = get("timeStyle", style_names)?;

    let zone = match time_zone {
        None => Zone::Local,
        Some(z) => parse_zone(&z)?,
    };

    let mut hc = match (hour12, hour_cycle.as_deref()) {
        (Some(true), _) => Some(Hc::H12),
        (Some(false), _) => Some(Hc::H23),
        (None, Some("h11")) => Some(Hc::H11),
        (None, Some("h12")) => Some(Hc::H12),
        (None, Some("h23")) => Some(Hc::H23),
        (None, Some("h24")) => Some(Hc::H24),
        _ => None,
    };

    if date_style.is_some() || time_style.is_some() {
        let explicit = weekday.is_some()
            || era.is_some()
            || year.is_some()
            || month.is_some()
            || day.is_some()
            || day_period.is_some()
            || hour.is_some()
            || minute.is_some()
            || second.is_some()
            || fsd.is_some()
            || tz_name.is_some();
        if explicit {
            return Err(host::type_error("Invalid option : option"));
        }
        if required == Required::Date && time_style.is_some() {
            return Err(host::type_error("Invalid option : timeStyle"));
        }
        if required == Required::Time && date_style.is_some() {
            return Err(host::type_error("Invalid option : dateStyle"));
        }
        let index = |s: &Option<String>| {
            s.as_deref()
                .and_then(|s| style_names.iter().position(|n| *n == s))
        };
        let (ds, ts) = (index(&date_style), index(&time_style));
        let time = ts.map(|t| {
            let p = TIME_STYLES[t];
            match hc {
                // The style's own `h` stands for the locale default; another
                // cycle is the style's skeleton regenerated with that hour.
                None | Some(Hc::H12) => p.to_string(),
                Some(c) => {
                    let skel: String = Skeleton::of(p)
                        .orig
                        .iter()
                        .flatten()
                        .filter(|(ch, _)| *ch != 'a')
                        .map(|&(ch, n)| {
                            let ch = if ch == 'h' { hour_char(c) } else { ch };
                            ch.to_string().repeat(n)
                        })
                        .collect();
                    best_pattern(&skel)
                }
            }
        });
        let pattern = match (ds, time) {
            (Some(d), Some(t)) => glue(d, DATE_STYLES[d], &t),
            (Some(d), None) => DATE_STYLES[d].to_string(),
            (None, Some(t)) => t,
            (None, None) => unreachable!("a style was given"),
        };
        return Ok(Format {
            pattern: replace_hour_cycle(&pattern, hc),
            zone,
        });
    }

    // ToDateTimeOptions.
    let mut need_defaults = true;
    if matches!(required, Required::Date | Required::Any)
        && (weekday.is_some() || year.is_some() || month.is_some() || day.is_some())
    {
        need_defaults = false;
    }
    if matches!(required, Required::Time | Required::Any)
        && (day_period.is_some()
            || hour.is_some()
            || minute.is_some()
            || second.is_some()
            || fsd.is_some())
    {
        need_defaults = false;
    }
    if need_defaults && matches!(required, Required::Date | Required::Any) {
        year = Some("numeric".into());
        month = Some("numeric".into());
        day = Some("numeric".into());
    }
    if need_defaults && matches!(required, Required::Time | Required::Any) {
        hour = Some("numeric".into());
        minute = Some("numeric".into());
        second = Some("numeric".into());
        // An hour the caller did not ask for follows only the 12/24 split of
        // the cycle: node prints `12:00:00 AM` under `h11` and `00:00:00` under
        // `h24` for a defaulted hour, and `0`/`24` only for an explicit one.
        hc = hc.map(|c| match c {
            Hc::H11 | Hc::H12 => Hc::H12,
            Hc::H23 | Hc::H24 => Hc::H23,
        });
    }

    let mut skel = String::new();
    let mut push = |c: char, n: usize| skel.push_str(&c.to_string().repeat(n));
    let text_len = |w: &str| match w {
        "narrow" => 5,
        "long" => 4,
        _ => 3,
    };
    if let Some(w) = &era {
        push('G', text_len(w));
    }
    if let Some(w) = &year {
        push('y', if w == "2-digit" { 2 } else { 1 });
    }
    if let Some(w) = &month {
        push(
            'M',
            match w.as_str() {
                "numeric" => 1,
                "2-digit" => 2,
                w => text_len(w),
            },
        );
    }
    if let Some(w) = &weekday {
        push('E', text_len(w));
    }
    if let Some(w) = &day {
        push('d', if w == "2-digit" { 2 } else { 1 });
    }
    if let Some(w) = &day_period {
        push('B', text_len(w));
    }
    if let Some(w) = &hour {
        push(
            hour_char(hc.unwrap_or(Hc::H12)),
            if w == "2-digit" { 2 } else { 1 },
        );
    }
    if let Some(w) = &minute {
        push('m', if w == "2-digit" { 2 } else { 1 });
    }
    if let Some(w) = &second {
        push('s', if w == "2-digit" { 2 } else { 1 });
    }
    if let Some(n) = fsd {
        push('S', n);
    }
    if let Some(w) = &tz_name {
        let (c, n) = match w.as_str() {
            "short" => ('z', 1),
            "long" => ('z', 4),
            "shortOffset" => ('O', 1),
            "longOffset" => ('O', 4),
            "shortGeneric" => ('v', 1),
            _ => ('v', 4),
        };
        push(c, n);
    }
    let pattern = best_pattern(&skel);
    Ok(Format {
        pattern: replace_hour_cycle(&pattern, if hour.is_some() { hc } else { None }),
        zone,
    })
}

fn hour_char(hc: Hc) -> char {
    match hc {
        Hc::H11 => 'K',
        Hc::H12 => 'h',
        Hc::H23 => 'H',
        Hc::H24 => 'k',
    }
}

/// V8's `ReplaceHourCycleInPattern`: every hour field outside quotes takes the
/// requested cycle's letter.
fn replace_hour_cycle(pattern: &str, hc: Option<Hc>) -> String {
    let Some(hc) = hc else {
        return pattern.to_string();
    };
    let want = hour_char(hc);
    let mut quoted = false;
    pattern
        .chars()
        .map(|c| match c {
            '\'' => {
                quoted = !quoted;
                c
            }
            'H' | 'h' | 'K' | 'k' if !quoted => want,
            _ => c,
        })
        .collect()
}

/// The `timeZone` option: UTC under any of its names, or a fixed offset.
/// A named IANA zone is not modelled and is reported as invalid.
fn parse_zone(z: &str) -> Result<Zone, String> {
    let upper = z.to_ascii_uppercase();
    const UTC_NAMES: &[&str] = &[
        "UTC",
        "ETC/UTC",
        "GMT",
        "ETC/GMT",
        "UCT",
        "ETC/UCT",
        "ETC/UNIVERSAL",
        "UNIVERSAL",
        "ETC/ZULU",
        "ZULU",
        "ETC/GREENWICH",
        "GREENWICH",
        "GMT0",
        "ETC/GMT0",
        "GMT+0",
        "GMT-0",
        "ETC/GMT+0",
        "ETC/GMT-0",
    ];
    if UTC_NAMES.contains(&upper.as_str()) {
        return Ok(Zone::Utc);
    }
    // `Etc/GMT+5` is five hours WEST of Greenwich.
    if let Some(rest) = upper.strip_prefix("ETC/GMT") {
        let (sign, digits) = rest.split_at(1.min(rest.len()));
        if let (Some(s), Ok(h)) = (
            match sign {
                "+" => Some(-1),
                "-" => Some(1),
                _ => None,
            },
            digits.parse::<i64>(),
        ) {
            if !digits.is_empty() && digits.len() <= 2 && h <= 14 && !digits.starts_with('0') {
                return Ok(Zone::Fixed(s * h * 60));
            }
        }
    }
    // `±HH:MM`, `±HHMM`, `±HH`.
    let b = z.as_bytes();
    if matches!(b.first(), Some(b'+') | Some(b'-')) {
        let sign = if b[0] == b'-' { -1 } else { 1 };
        let rest = &z[1..];
        let digits: String = rest.chars().filter(|c| *c != ':').collect();
        let well_formed = match rest.len() {
            2 => rest.bytes().all(|c| c.is_ascii_digit()),
            4 => rest.bytes().all(|c| c.is_ascii_digit()),
            5 => rest.as_bytes()[2] == b':' && digits.bytes().all(|c| c.is_ascii_digit()),
            _ => false,
        };
        if well_formed {
            let h: i64 = digits[..2].parse().unwrap_or(99);
            let m: i64 = digits
                .get(2..)
                .filter(|s| !s.is_empty())
                .map_or(Ok(0), str::parse)
                .unwrap_or(99);
            if h <= 23 && m <= 59 {
                return Ok(Zone::Fixed(sign * (h * 60 + m)));
            }
        }
    }
    match crate::tzif::load(z) {
        Some(tz) => Ok(Zone::Named(tz)),
        None => Err(host::range_error(&format!(
            "Invalid time zone specified: {z}"
        ))),
    }
}

// ── rendering ────────────────────────────────────────────────────────────────

const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// The broken-down wall clock a pattern reads its fields from.
struct Fields {
    year: i64,
    month: usize,
    day: i64,
    weekday: usize,
    hour: i64,
    minute: i64,
    second: i64,
    millis: i64,
    /// Offset east of UTC, in seconds (a pre-1900 local mean time is not a
    /// whole number of minutes).
    offset_sec: i64,
    /// The zone's abbreviation at this instant (`EST`), from 1970 on; `None`
    /// for UTC, a fixed offset, or an earlier instant.
    abbrev: Option<String>,
    /// Whether the zone is UTC itself, which has names of its own.
    utc: bool,
}

impl Format {
    /// Render the time value `ms` (finite) with this format.
    pub fn format(&self, ms: f64) -> String {
        let (offset_ms, abbrev) = match &self.zone {
            Zone::Local => (
                crate::stdlib::date::zone_offset_ms(ms),
                crate::stdlib::date::zone_abbrev(ms),
            ),
            Zone::Utc => (0.0, None),
            Zone::Fixed(m) => (*m as f64 * 60_000.0, None),
            Zone::Named(tz) => {
                let ty = tz.at((ms / 1000.0).floor() as i64);
                (ty.offset as f64 * 1000.0, Some(ty.abbrev))
            }
        };
        let local = ms + offset_ms;
        let (day_num, rem) = crate::stdlib::date::split_day(local);
        let (year, month, day) = crate::stdlib::date::civil_from_days(day_num);
        let f = Fields {
            year,
            month: month as usize,
            day,
            weekday: (((day_num % 7) + 4 + 7) % 7) as usize,
            hour: rem / 3_600_000,
            minute: rem / 60_000 % 60,
            second: rem / 1000 % 60,
            millis: rem % 1000,
            offset_sec: (offset_ms / 1000.0) as i64,
            // ICU maps a zone to its named metazone only from 1970 on;
            // before that every zone is named by its offset.
            utc: match &self.zone {
                Zone::Utc => true,
                Zone::Fixed(_) => false,
                Zone::Local => offset_ms == 0.0 && abbrev.as_deref().map_or(true, |a| a == "UTC"),
                Zone::Named(_) => offset_ms == 0.0 && abbrev.as_deref() == Some("UTC"),
            },
            abbrev: abbrev.filter(|_| ms >= 0.0),
        };
        let pattern_has_minute = self.pattern.contains('m');
        let pattern_has_second = self.pattern.contains('s');
        let mut out = String::new();
        for t in tokenize(&self.pattern) {
            match t {
                Tok::Lit(s) => {
                    if s.len() >= 2 && s.starts_with('\'') {
                        let inner = &s[1..s.len() - 1];
                        if inner.is_empty() {
                            out.push('\'');
                        } else {
                            out.push_str(&inner.replace("''", "'"));
                        }
                    } else {
                        out.push_str(&s);
                    }
                }
                Tok::Field(c, n) => {
                    out.push_str(&self.field(c, n, &f, pattern_has_minute, pattern_has_second))
                }
            }
        }
        out
    }

    fn field(&self, c: char, n: usize, f: &Fields, has_min: bool, has_sec: bool) -> String {
        let num = |v: i64, n: usize| format!("{:0width$}", v, width = n);
        let bc = f.year <= 0;
        match c {
            'G' => match (n, bc) {
                (1..=3, false) => "AD".into(),
                (1..=3, true) => "BC".into(),
                (4, false) => "Anno Domini".into(),
                (4, true) => "Before Christ".into(),
                (_, false) => "A".into(),
                (_, true) => "B".into(),
            },
            'y' => {
                let y = if bc { 1 - f.year } else { f.year };
                if n == 2 {
                    num(y % 100, 2)
                } else {
                    num(y, n)
                }
            }
            'M' | 'L' => match n {
                1 | 2 => num(f.month as i64 + 1, n),
                3 => MONTHS_LONG[f.month][..3].to_string(),
                4 => MONTHS_LONG[f.month].to_string(),
                _ => MONTHS_LONG[f.month][..1].to_string(),
            },
            'E' | 'c' => match n {
                4 => DAYS_LONG[f.weekday].to_string(),
                5 => DAYS_LONG[f.weekday][..1].to_string(),
                6 => DAYS_LONG[f.weekday][..2].to_string(),
                _ => DAYS_LONG[f.weekday][..3].to_string(),
            },
            'd' => num(f.day, n),
            'a' => match (n, f.hour < 12) {
                (5, true) => "a".into(),
                (5, false) => "p".into(),
                (_, true) => "AM".into(),
                (_, false) => "PM".into(),
            },
            'B' => {
                // en's flexible day periods. ICU never says "midnight"; it says
                // "noon" only when nothing finer than the hour is shown or
                // what is shown is zero.
                let noon =
                    f.hour == 12 && (!has_min || f.minute == 0) && (!has_sec || f.second == 0);
                if noon {
                    return if n == 5 { "n".into() } else { "noon".into() };
                }
                match f.hour {
                    0..=11 => "in the morning",
                    12..=17 => "in the afternoon",
                    18..=20 => "in the evening",
                    _ => "at night",
                }
                .into()
            }
            'h' => num(if f.hour % 12 == 0 { 12 } else { f.hour % 12 }, n),
            'K' => num(f.hour % 12, n),
            'H' => num(f.hour, n),
            'k' => num(if f.hour == 0 { 24 } else { f.hour }, n),
            'm' => num(f.minute, n),
            's' => num(f.second, n),
            'S' => {
                let digits = format!("{:03}", f.millis);
                let mut s: String = digits.chars().take(n).collect();
                while s.len() < n {
                    s.push('0');
                }
                s
            }
            'z' | 'O' | 'v' => self.zone_name(c, n, f),
            _ => c.to_string().repeat(n),
        }
    }

    /// The `timeZoneName` text. UTC has names of its own, and so do the zones
    /// in [`named_zone`]; any other zone is named by its offset
    /// (node knows the long names of every zone from ICU; this does not).
    fn zone_name(&self, c: char, n: usize, f: &Fields) -> String {
        let long = n >= 4;
        let utc = f.utc;
        let named = f.abbrev.as_deref().and_then(named_zone);
        match (c, utc, named) {
            ('z', true, _) if long => "Coordinated Universal Time".into(),
            ('z', true, _) => "UTC".into(),
            ('z', false, Some((_, long_name, _, _))) if long => long_name.into(),
            ('z', false, Some((Some(short), _, _, _))) => short.into(),
            ('v', false, Some((_, _, _, generic_long))) if long => generic_long.into(),
            ('v', false, Some((_, _, generic, _))) => generic.into(),
            _ => gmt_offset(f.offset_sec, long),
        }
    }
}

/// `GMT-5`, `GMT+5:30` (short) or `GMT-05:00` (long); `GMT+0` / `GMT+00:00` at
/// zero, and a seconds part when the offset has one (`GMT-4:56:02`).
fn gmt_offset(secs: i64, long: bool) -> String {
    let sign = if secs < 0 { '-' } else { '+' };
    let a = secs.abs();
    let (h, m, s) = (a / 3600, a / 60 % 60, a % 60);
    let tail = if s != 0 {
        format!(":{s:02}")
    } else {
        String::new()
    };
    if long {
        format!("GMT{sign}{h:02}:{m:02}{tail}")
    } else if m == 0 && s == 0 {
        format!("GMT{sign}{h}")
    } else {
        format!("GMT{sign}{h}:{m:02}{tail}")
    }
}

/// The zones en names by the abbreviation `localtime_r` reports:
/// `(short, long, short generic, long generic)`, `None` where en writes the
/// GMT offset instead.
type ZoneNames = (
    Option<&'static str>,
    &'static str,
    &'static str,
    &'static str,
);

fn named_zone(abbrev: &str) -> Option<ZoneNames> {
    let us = |short, long, region: &'static str| -> ZoneNames {
        let generic: &'static str = match region {
            "Eastern" => "ET",
            "Central" => "CT",
            "Mountain" => "MT",
            "Pacific" => "PT",
            _ => "AKT",
        };
        let generic_long: &'static str = match region {
            "Eastern" => "Eastern Time",
            "Central" => "Central Time",
            "Mountain" => "Mountain Time",
            "Pacific" => "Pacific Time",
            _ => "Alaska Time",
        };
        (Some(short), long, generic, generic_long)
    };
    Some(match abbrev {
        "EST" => us("EST", "Eastern Standard Time", "Eastern"),
        "EDT" => us("EDT", "Eastern Daylight Time", "Eastern"),
        "CST" => us("CST", "Central Standard Time", "Central"),
        "CDT" => us("CDT", "Central Daylight Time", "Central"),
        "MST" => us("MST", "Mountain Standard Time", "Mountain"),
        "MDT" => us("MDT", "Mountain Daylight Time", "Mountain"),
        "PST" => us("PST", "Pacific Standard Time", "Pacific"),
        "PDT" => us("PDT", "Pacific Daylight Time", "Pacific"),
        "AKST" => us("AKST", "Alaska Standard Time", "Alaska"),
        "AKDT" => us("AKDT", "Alaska Daylight Time", "Alaska"),
        // Hawaii keeps no daylight time, so its generic names are its
        // standard ones.
        "HST" => (
            Some("HST"),
            "Hawaii-Aleutian Standard Time",
            "HST",
            "Hawaii-Aleutian Standard Time",
        ),
        "GMT" => (
            Some("GMT"),
            "Greenwich Mean Time",
            "GMT",
            "Greenwich Mean Time",
        ),
        _ => return None,
    })
}

/// `Date.prototype.toLocale{,Date,Time}String(locales, options)`.
pub fn date_to_locale(
    ms: f64,
    opts: &Value,
    required: Required,
    method: &str,
) -> Result<String, String> {
    if ms.is_nan() {
        return Ok("Invalid Date".into());
    }
    Ok(resolve(opts, required, method)?.format(ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_icu() {
        assert_eq!(best_pattern("yMd"), "M/d/y");
        assert_eq!(best_pattern("yMMMMEEEEd"), "EEEE, MMMM d, y");
        assert_eq!(best_pattern("hms"), "h:mm:ss a");
        assert_eq!(best_pattern("yMdhms"), "M/d/y, h:mm:ss a");
        assert_eq!(best_pattern("Hms"), "HH:mm:ss");
        assert_eq!(best_pattern("MMMMMh"), "LLLLL, h a");
        assert_eq!(best_pattern("ss"), "s");
        assert_eq!(best_pattern("ddHHs"), "dd, s ('hour': HH)");
    }
}
