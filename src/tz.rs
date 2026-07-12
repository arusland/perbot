//! Timezone seam: the per-chat `timezone` setting key and the only
//! local↔UTC conversion helpers in the crate. Stored instants
//! (`next_datetime`, `last_next_datetime`, `created_at`, `missed_at`) are
//! naive **UTC**; wall-clock fields (`time`/`date`/…) are chat-local.
//! Conversion happens only in `scheduler::calc_next_at` and at the `view/`
//! formatting boundary — both through this module, which owns the DST policy.

use chrono::{Duration, NaiveDateTime, TimeZone};
use chrono_tz::Tz;

/// Settings-table key holding a chat's IANA timezone name (e.g. `Europe/Berlin`).
pub const TIMEZONE_SETTING: &str = "timezone";

/// Parses a stored IANA timezone name. Any name chrono-tz knows is accepted,
/// including deprecated aliases the picker doesn't offer.
pub fn parse_tz(s: &str) -> Option<Tz> {
    s.parse().ok()
}

/// Converts a UTC instant to the chat's local wall-clock time.
pub fn to_local(utc: NaiveDateTime, tz: Tz) -> NaiveDateTime {
    tz.from_utc_datetime(&utc).naive_local()
}

/// Converts a chat-local wall-clock time to UTC. DST policy: a time inside a
/// spring-forward gap resolves forward to the gap end; an ambiguous fall-back
/// time takes the earliest offset.
pub fn to_utc(local: NaiveDateTime, tz: Tz) -> NaiveDateTime {
    let mut candidate = local;
    // A gap is normally ≤ 2h, but calendar reforms skipped whole days
    // (Pacific/Apia 2011); probe far enough to cross any of them.
    for _ in 0..(3 * 24 * 60) {
        match tz.from_local_datetime(&candidate) {
            chrono::LocalResult::Single(dt) => return dt.naive_utc(),
            chrono::LocalResult::Ambiguous(earliest, _) => return earliest.naive_utc(),
            chrono::LocalResult::None => candidate += Duration::minutes(1),
        }
    }
    // Unreachable for real tzdata; fall back to interpreting the input as UTC.
    local
}

/// Re-anchors a UTC instant so its wall-clock reading moves from `old` to
/// `new` unchanged (09:00 Amsterdam → 09:00 Tokyo): the timezone-change
/// reschedule for everything except pure-instant offsets.
pub fn shift_wallclock(utc: NaiveDateTime, old: Tz, new: Tz) -> NaiveDateTime {
    to_utc(to_local(utc, old), new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap()
    }

    #[test]
    fn utc_round_trip_is_identity() {
        let t = dt(2026, 7, 12, 9, 30);
        assert_eq!(to_local(t, Tz::UTC), t);
        assert_eq!(to_utc(t, Tz::UTC), t);
    }

    #[test]
    fn to_local_and_back() {
        let tz = Tz::Europe__Amsterdam;
        let utc = dt(2026, 7, 12, 7, 0);
        let local = to_local(utc, tz);
        assert_eq!(local, dt(2026, 7, 12, 9, 0)); // CEST = UTC+2
        assert_eq!(to_utc(local, tz), utc);
    }

    #[test]
    fn gap_resolves_forward_to_gap_end() {
        // Europe/Amsterdam springs 02:00 → 03:00 on 2026-03-29; 02:30 doesn't
        // exist and resolves to 03:00 local = 01:00 UTC.
        let tz = Tz::Europe__Amsterdam;
        assert_eq!(to_utc(dt(2026, 3, 29, 2, 30), tz), dt(2026, 3, 29, 1, 0));
    }

    #[test]
    fn fold_takes_earliest_offset() {
        // Europe/Amsterdam falls back 03:00 → 02:00 on 2026-10-25; 02:30
        // happens twice, earliest is the CEST (+2) reading = 00:30 UTC.
        let tz = Tz::Europe__Amsterdam;
        assert_eq!(to_utc(dt(2026, 10, 25, 2, 30), tz), dt(2026, 10, 25, 0, 30));
    }

    #[test]
    fn shift_wallclock_keeps_reading() {
        // 09:00 Amsterdam (07:00 UTC) becomes 09:00 Tokyo (00:00 UTC).
        let utc = dt(2026, 7, 12, 7, 0);
        let shifted = shift_wallclock(utc, Tz::Europe__Amsterdam, Tz::Asia__Tokyo);
        assert_eq!(shifted, dt(2026, 7, 12, 0, 0));
        assert_eq!(
            to_local(shifted, Tz::Asia__Tokyo),
            to_local(utc, Tz::Europe__Amsterdam)
        );
    }

    #[test]
    fn parse_tz_accepts_iana_and_rejects_junk() {
        assert_eq!(parse_tz("Europe/Berlin"), Some(Tz::Europe__Berlin));
        assert_eq!(parse_tz("UTC"), Some(Tz::UTC));
        assert!(parse_tz("Nowhere/Nothing").is_none());
    }
}
