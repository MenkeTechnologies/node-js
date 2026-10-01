//! Named IANA time zones, read from the system's compiled zoneinfo
//! (`/usr/share/zoneinfo`, RFC 8536 TZif files) — what `Intl` and
//! `toLocaleString` need for a `timeZone: "America/New_York"` option.
//!
//! A TZif file lists every transition up to some year and then, in its footer,
//! the POSIX `TZ` rule that continues past the last one (`EST5EDT,M3.2.0,
//! M11.1.0`). macOS ships "slim" files whose transitions stop early and lean on
//! that rule, so both halves are read.

use std::collections::HashMap;
use std::rc::Rc;

/// One local-time type: offset east of UTC in seconds, and its abbreviation.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalType {
    pub offset: i64,
    pub abbrev: String,
}

/// A parsed zone.
#[derive(Debug)]
pub struct Tz {
    /// The canonical name as found on disk.
    pub name: String,
    transitions: Vec<i64>,
    type_of: Vec<usize>,
    types: Vec<LocalType>,
    rule: Option<PosixRule>,
}

/// The POSIX `TZ` rule of the footer: standard time, and optionally daylight
/// time between two yearly transition dates.
#[derive(Debug)]
struct PosixRule {
    std: LocalType,
    dst: Option<(LocalType, RuleDate, RuleDate)>,
}

/// `Mm.w.d/time` — day `d` (0 = Sunday) of week `w` (5 = last) of month `m`,
/// at `time` seconds of LOCAL time. `Jn` and plain-`n` dates are read as
/// day-of-year forms.
#[derive(Debug, Clone, Copy)]
enum RuleDate {
    MonthWeekDay {
        month: i64,
        week: i64,
        day: i64,
        time: i64,
    },
    /// `Jn`: day 1..=365, February 29 never counted.
    Julian1 { day: i64, time: i64 },
    /// `n`: day 0..=365, February 29 counted.
    Julian0 { day: i64, time: i64 },
}

impl Tz {
    /// The local type in force at `t` (seconds since the epoch, UTC).
    pub fn at(&self, t: i64) -> LocalType {
        match self.transitions.binary_search(&t) {
            Ok(i) => self.types[self.type_of[i]].clone(),
            // Past the last transition (or with none at all), the footer's
            // rule continues the zone.
            Err(i) if i == self.transitions.len() && self.rule.is_some() => {
                self.rule.as_ref().map(|r| r.at(t)).expect("checked")
            }
            // Before the first transition: type 0 (RFC 8536 3.2).
            Err(0) => self.types[0].clone(),
            Err(i) => self.types[self.type_of[i - 1]].clone(),
        }
    }
}

thread_local! {
    static CACHE: std::cell::RefCell<HashMap<String, Option<Rc<Tz>>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// The zone named `name`, if the system has it. Names are matched as the
/// filesystem matches them; `..` and absolute paths are refused.
pub fn load(name: &str) -> Option<Rc<Tz>> {
    if name.is_empty()
        || name.starts_with('/')
        || name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
    {
        return None;
    }
    if let Some(hit) = CACHE.with(|c| c.borrow().get(name).cloned()) {
        return hit;
    }
    let tz = [
        "/usr/share/zoneinfo",
        "/usr/lib/zoneinfo",
        "/usr/share/lib/zoneinfo",
    ]
    .iter()
    .find_map(|dir| std::fs::read(format!("{dir}/{name}")).ok())
    .and_then(|bytes| parse(&bytes, name))
    .map(Rc::new);
    CACHE.with(|c| c.borrow_mut().insert(name.to_string(), tz.clone()));
    tz
}

fn be32(b: &[u8], at: usize) -> Option<i64> {
    Some(i32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?) as i64)
}

fn be64(b: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// The six counts of a TZif header: isutcnt, isstdcnt, leapcnt, timecnt,
/// typecnt, charcnt.
fn counts(b: &[u8], at: usize) -> Option<[usize; 6]> {
    let mut c = [0usize; 6];
    for (i, slot) in c.iter_mut().enumerate() {
        *slot = be32(b, at + 20 + i * 4)? as usize;
    }
    Some(c)
}

fn parse(b: &[u8], name: &str) -> Option<Tz> {
    if b.get(..4)? != b"TZif" {
        return None;
    }
    let version = *b.get(4)?;
    let c1 = counts(b, 0)?;
    let v1_len = c1[3] * 5 + c1[4] * 6 + c1[5] + c1[2] * 8 + c1[1] + c1[0];
    // Version 2+ repeats the data with 64-bit times after the v1 block.
    let (base, c, time_size) = if version >= b'2' {
        let at = 44 + v1_len;
        (at, counts(b, at)?, 8)
    } else {
        (0, c1, 4)
    };
    let [isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt] = c;
    let mut p = base + 44;
    let mut transitions = Vec::with_capacity(timecnt);
    for i in 0..timecnt {
        transitions.push(if time_size == 8 {
            be64(b, p + i * 8)?
        } else {
            be32(b, p + i * 4)?
        });
    }
    p += timecnt * time_size;
    let type_of: Vec<usize> = b.get(p..p + timecnt)?.iter().map(|&x| x as usize).collect();
    p += timecnt;
    let mut raw_types = Vec::with_capacity(typecnt);
    for i in 0..typecnt {
        let at = p + i * 6;
        raw_types.push((be32(b, at)?, *b.get(at + 5)? as usize));
    }
    p += typecnt * 6;
    let chars = b.get(p..p + charcnt)?;
    let types: Vec<LocalType> = raw_types
        .into_iter()
        .map(|(offset, idx)| {
            let end = chars[idx.min(chars.len())..]
                .iter()
                .position(|&ch| ch == 0)
                .map_or(chars.len(), |e| idx + e);
            LocalType {
                offset,
                abbrev: String::from_utf8_lossy(&chars[idx.min(end)..end]).into_owned(),
            }
        })
        .collect();
    if types.is_empty() || type_of.iter().any(|&t| t >= types.len()) {
        return None;
    }
    p += charcnt + leapcnt * (time_size + 4) + isstdcnt + isutcnt;
    let rule = if version >= b'2' {
        b.get(p..).and_then(|rest| {
            let text = std::str::from_utf8(rest).ok()?;
            let line = text.strip_prefix('\n')?.split('\n').next()?;
            PosixRule::parse(line)
        })
    } else {
        None
    };
    Some(Tz {
        name: name.to_string(),
        transitions,
        type_of,
        types,
        rule,
    })
}

impl PosixRule {
    fn parse(s: &str) -> Option<PosixRule> {
        let mut cur = s;
        let std_name = take_name(&mut cur)?;
        // POSIX offsets are WEST of Greenwich.
        let std_off = -take_offset(&mut cur)?;
        let std = LocalType {
            offset: std_off,
            abbrev: std_name,
        };
        if cur.is_empty() {
            return Some(PosixRule { std, dst: None });
        }
        let dst_name = take_name(&mut cur)?;
        let dst_off = if cur.starts_with(',') {
            std_off + 3600
        } else {
            -take_offset(&mut cur)?
        };
        let rest = cur.strip_prefix(',')?;
        let (start, end) = rest.split_once(',')?;
        Some(PosixRule {
            std,
            dst: Some((
                LocalType {
                    offset: dst_off,
                    abbrev: dst_name,
                },
                RuleDate::parse(start)?,
                RuleDate::parse(end)?,
            )),
        })
    }

    fn at(&self, t: i64) -> LocalType {
        let Some((dst, start, end)) = &self.dst else {
            return self.std.clone();
        };
        // The transitions of the year `t` falls in (in standard time), as UTC
        // instants: the start is read in standard time, the end in daylight.
        let year = crate::stdlib::date::civil_from_days((t + self.std.offset).div_euclid(86_400)).0;
        let begin = start.local_secs(year) - self.std.offset;
        let finish = end.local_secs(year) - dst.offset;
        let in_dst = if begin < finish {
            t >= begin && t < finish
        } else {
            // Southern hemisphere: daylight time spans the new year.
            !(t >= finish && t < begin)
        };
        if in_dst {
            dst.clone()
        } else {
            self.std.clone()
        }
    }
}

/// A zone abbreviation: letters, or anything between `<` and `>`.
fn take_name(cur: &mut &str) -> Option<String> {
    if let Some(rest) = cur.strip_prefix('<') {
        let end = rest.find('>')?;
        let name = rest[..end].to_string();
        *cur = &rest[end + 1..];
        return Some(name);
    }
    let end = cur
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(cur.len());
    if end < 3 {
        return None;
    }
    let name = cur[..end].to_string();
    *cur = &cur[end..];
    Some(name)
}

/// `[+-]hh[:mm[:ss]]` in seconds.
fn take_offset(cur: &mut &str) -> Option<i64> {
    let (sign, rest) = match cur.as_bytes().first()? {
        b'-' => (-1, &cur[1..]),
        b'+' => (1, &cur[1..]),
        _ => (1, *cur),
    };
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == ':'))
        .unwrap_or(rest.len());
    let secs = hms(&rest[..end])?;
    *cur = &rest[end..];
    Some(sign * secs)
}

fn hms(s: &str) -> Option<i64> {
    let mut total = 0;
    let mut scale = 3600;
    for part in s.split(':') {
        if part.is_empty() || scale == 0 {
            return None;
        }
        total += part.parse::<i64>().ok()? * scale;
        scale /= 60;
    }
    Some(total)
}

impl RuleDate {
    fn parse(s: &str) -> Option<RuleDate> {
        let (date, time) = match s.split_once('/') {
            Some((d, t)) => {
                let (neg, t) = match t.strip_prefix('-') {
                    Some(t) => (true, t),
                    None => (false, t.strip_prefix('+').unwrap_or(t)),
                };
                let secs = hms(t)?;
                (d, if neg { -secs } else { secs })
            }
            None => (s, 7200),
        };
        if let Some(m) = date.strip_prefix('M') {
            let mut it = m.split('.').map(|x| x.parse::<i64>().ok());
            let (month, week, day) = (it.next()??, it.next()??, it.next()??);
            return Some(RuleDate::MonthWeekDay {
                month,
                week,
                day,
                time,
            });
        }
        if let Some(n) = date.strip_prefix('J') {
            return Some(RuleDate::Julian1 {
                day: n.parse().ok()?,
                time,
            });
        }
        Some(RuleDate::Julian0 {
            day: date.parse().ok()?,
            time,
        })
    }

    /// Seconds since the epoch, in LOCAL time, of this date in `year`.
    fn local_secs(&self, year: i64) -> i64 {
        use crate::stdlib::date::days_from_civil;
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let (days, time) = match *self {
            RuleDate::MonthWeekDay {
                month,
                week,
                day,
                time,
            } => {
                let first = days_from_civil(year, month - 1, 1);
                // 1970-01-01 was a Thursday (4).
                let first_wd = (first + 4).rem_euclid(7);
                let mut d = first + (day - first_wd).rem_euclid(7) + (week - 1) * 7;
                let next_month = if month == 12 {
                    days_from_civil(year + 1, 0, 1)
                } else {
                    days_from_civil(year, month, 1)
                };
                while d >= next_month {
                    d -= 7;
                }
                (d, time)
            }
            RuleDate::Julian1 { day, time } => {
                let skip = if leap && day >= 60 { 1 } else { 0 };
                (days_from_civil(year, 0, 1) + day - 1 + skip, time)
            }
            RuleDate::Julian0 { day, time } => (days_from_civil(year, 0, 1) + day, time),
        };
        days * 86_400 + time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_rule_new_york() {
        let r = PosixRule::parse("EST5EDT,M3.2.0,M11.1.0").unwrap();
        // 2030-07-01T12:00Z is daylight time; 2030-01-01T12:00Z is not.
        assert_eq!(r.at(1_909_137_600).abbrev, "EDT");
        assert_eq!(r.at(1_893_499_200).abbrev, "EST");
        let r = PosixRule::parse("<+0530>-5:30").unwrap();
        assert_eq!(r.at(0).offset, 19_800);
        assert_eq!(r.at(0).abbrev, "+0530");
    }
}
