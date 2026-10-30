//! ICU's en SPECIFIC zone names — `Eastern Standard Time` / `EST`, the `zzzz`
//! and `z` pattern fields and the parenthetical of `Date.prototype.toString`.
//!
//! ICU names a zone through its METAZONE at an instant: `America/New_York`
//! belongs to `America_Eastern`, whose en names are `Eastern Standard Time`,
//! `Eastern Daylight Time`, `EST` and `EDT`. A zone may change metazone over
//! time (`Africa/Algiers` moved between `Europe_Western` and `Europe_Central`
//! four times), a metazone may lack a name of some width (`Europe_Central` has
//! no short names in en, so Paris is `GMT+1` under `timeZoneName: 'short'`),
//! and a zone may carry names of its own ahead of any metazone (`Etc/UTC`). A
//! missing name falls back to the localized GMT format, which the callers
//! render.
//!
//! The tables in `tznames_data.rs` transcribe ICU 79.1 / CLDR 49, the data node
//! v26.12.1 ships. Keying by the zone rather than by the abbreviation
//! `localtime` reports is what tells `Europe/London` (metazone `British`, no
//! standard-time name: `GMT+0`) from `Atlantic/Reykjavik` (metazone `GMT`:
//! `GMT`), which share the abbreviation `GMT`.

#[path = "tznames_data.rs"]
mod data;

/// ICU's canonical id for `id` (`Asia/Kolkata` → `Asia/Calcutta`,
/// `US/Eastern` → `America/New_York`): the key the metazone table uses.
fn canonical(id: &str) -> &str {
    match data::ALIASES.binary_search_by(|(alias, _)| alias.cmp(&id)) {
        Ok(i) => data::ALIASES[i].1,
        Err(_) => id,
    }
}

/// The metazone `id` belongs to at `secs` (UTC epoch seconds); a mapping
/// covers `[from, to)`.
fn metazone_at(id: &str, secs: i64) -> Option<&'static str> {
    let i = data::ZONE_METAZONES
        .binary_search_by(|(zone, _)| zone.cmp(&id))
        .ok()?;
    data::ZONE_METAZONES[i]
        .1
        .iter()
        .find(|(_, from, to)| *from <= secs && secs < *to)
        .map(|(mz, _, _)| *mz)
}

/// Pick one of a `[ls, ld, ss, sd]` row.
fn pick(row: &[Option<&'static str>; 4], dst: bool, long: bool) -> Option<&'static str> {
    row[usize::from(!long) * 2 + usize::from(dst)]
}

/// The en specific name of zone `id` at `secs` — long (`zzzz`) or short
/// (`z`), daylight or standard — or `None` where ICU falls back to the GMT
/// format.
pub fn specific_name(id: &str, secs: i64, dst: bool, long: bool) -> Option<&'static str> {
    let id = canonical(id);
    if let Ok(i) = data::ZONE_NAMES.binary_search_by(|(zone, _)| zone.cmp(&id)) {
        if let Some(name) = pick(&data::ZONE_NAMES[i].1, dst, long) {
            return Some(name);
        }
    }
    let mz = metazone_at(id, secs)?;
    let i = data::METAZONE_NAMES
        .binary_search_by(|(m, _)| m.cmp(&mz))
        .ok()?;
    pick(&data::METAZONE_NAMES[i].1, dst, long)
}

/// The IANA id of the process's local zone: `TZ` (a leading `:` dropped, a
/// path reduced to what follows `zoneinfo/`, empty meaning UTC), else the
/// target of `/etc/localtime`. Read on every call, as `localtime_r` re-reads
/// `TZ`. `None` when neither names a zone the system has (a POSIX rule string
/// such as `EST5EDT,M3.2.0,M11.1.0`).
pub fn local_zone_id() -> Option<String> {
    let from_path = |p: &str| p.rsplit_once("zoneinfo/").map(|(_, id)| id.to_string());
    let id = match std::env::var("TZ") {
        Ok(tz) => {
            let tz = tz.strip_prefix(':').unwrap_or(&tz);
            if tz.is_empty() {
                "UTC".to_string()
            } else if tz.starts_with('/') {
                from_path(tz)?
            } else {
                tz.to_string()
            }
        }
        Err(_) => from_path(&std::fs::read_link("/etc/localtime").ok()?.to_string_lossy())?,
    };
    crate::tzif::load(&id).map(|_| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_metazone_at_the_instant() {
        assert_eq!(
            specific_name("America/New_York", 0, false, true),
            Some("Eastern Standard Time")
        );
        assert_eq!(specific_name("US/Eastern", 0, true, false), Some("EDT"));
        assert_eq!(
            specific_name("Asia/Kolkata", 0, false, true),
            Some("India Standard Time")
        );
        assert_eq!(specific_name("Europe/Paris", 0, false, false), None);
        assert_eq!(
            specific_name("Europe/London", 1_700_000_000, false, true),
            None
        );
        assert_eq!(
            specific_name("Europe/London", 1_700_000_000, true, true),
            Some("British Summer Time")
        );
        assert_eq!(specific_name("UTC", 0, false, false), Some("UTC"));
    }
}
