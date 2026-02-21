use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

use crate::storage::StoredEvent;

/// Calculates the next occurrence datetime for a stored event and returns the
/// updated event. Sets `active = true` and `next_datetime = Some(dt)` when a
/// future datetime can be determined, otherwise `active = false` and
/// `next_datetime = None`.
pub fn calc_next(event: StoredEvent) -> StoredEvent {
    calc_next_at(event, Local::now().naive_local())
}

pub fn calc_next_at(event: StoredEvent, now: NaiveDateTime) -> StoredEvent {
    let next_datetime = calculate_next_datetime(&event, now);
    StoredEvent {
        active: next_datetime.is_some(),
        next_datetime,
        ..event
    }
}

fn calculate_next_datetime(event: &StoredEvent, now: NaiveDateTime) -> Option<NaiveDateTime> {
    // Handle bare hour (e.g., bare_hour=8 -> next 08:00)
    if let Some(h) = event.bare_hour {
        let hour = if h == 24 { 0 } else { h };
        let t = NaiveTime::from_hms_opt(hour, 0, 0)?;
        let today = now.date();
        let dt = today.and_time(t);
        return if dt > now {
            Some(dt)
        } else {
            let tomorrow = today.succ_opt()?;
            Some(tomorrow.and_time(t))
        };
    }

    // Handle relative offset (e.g., in_offset=8, in_offset_unit="minutes")
    if let (Some(value), Some(unit_str)) = (event.in_offset, event.in_offset_unit.as_deref()) {
        return match unit_str {
            "minutes" => Some(now + chrono::Duration::minutes(value as i64)),
            "hours" => Some(now + chrono::Duration::hours(value as i64)),
            "days" => Some(now + chrono::Duration::days(value as i64)),
            "weeks" => Some(now + chrono::Duration::weeks(value as i64)),
            "months" => {
                let new_date = now.date().checked_add_months(chrono::Months::new(value))?;
                Some(new_date.and_time(now.time()))
            }
            "years" => {
                let new_date = now
                    .date()
                    .checked_add_months(chrono::Months::new(value * 12))?;
                Some(new_date.and_time(now.time()))
            }
            _ => None,
        };
    }

    // Handle monthly pattern (e.g., "first_sun", "last_mon", "last_day")
    if let Some(ref pattern_str) = event.monthly_pattern {
        let time = event
            .time
            .unwrap_or(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let today = now.date();
        let mut year = today.year();
        let mut month = today.month();

        for _ in 0..3 {
            let target_date = if pattern_str == "last_day" {
                last_day_of_month_date(year, month)
            } else if let Some((n, weekday)) = parse_ordinal_weekday(pattern_str) {
                if n == 0 {
                    last_weekday_of_month(year, month, weekday)
                } else {
                    nth_weekday_of_month(year, month, weekday, n)
                }
            } else {
                None
            };

            if let Some(d) = target_date {
                let dt = d.and_time(time);
                if dt > now {
                    return Some(dt);
                }
            }

            let (ny, nm) = next_month(year, month);
            year = ny;
            month = nm;
        }

        return None;
    }

    let dt = match (event.time, event.date) {
        (Some(t), Some(d)) => {
            let dt = d.and_time(t);
            if dt > now {
                Some(dt)
            } else if event.year_explicit {
                // Explicit year: one-shot unless a repeat interval is set
                if let (Some(base), Some(interval), Some(unit_str)) = (
                    event.next_datetime,
                    event.repeat_interval,
                    event.repeat_unit.as_deref(),
                ) {
                    let mut next = base;
                    while next <= now {
                        next = advance_by(next, interval, unit_str)?;
                    }
                    Some(next)
                } else {
                    None
                }
            } else {
                // Short date in the past — try next year
                let next = NaiveDate::from_ymd_opt(d.year() + 1, d.month(), d.day())?;
                Some(next.and_time(t))
            }
        }
        (Some(t), None) => {
            // One-shot event (no repetition): if already scheduled and the time
            // has now passed, it has fired and should not repeat.
            if event.next_datetime.is_some()
                && event.repeat_interval.is_none()
                && event.days.is_none()
            {
                return None;
            }
            // Repeating event: advance from the previously scheduled datetime by
            // the repeat interval until a future datetime is found.
            if let (Some(base), Some(interval), Some(unit_str)) = (
                event.next_datetime,
                event.repeat_interval,
                event.repeat_unit.as_deref(),
            ) {
                let mut next = base;
                while next <= now {
                    next = advance_by(next, interval, unit_str)?;
                }
                return Some(next);
            }
            let today = now.date();
            let dt = today.and_time(t);
            if dt > now {
                Some(dt)
            } else {
                // Time already passed today — tomorrow
                let tomorrow = today.succ_opt()?;
                Some(tomorrow.and_time(t))
            }
        }
        (None, Some(d)) => {
            // Date only — use midnight
            let dt = d.and_hms_opt(0, 0, 0)?;
            if dt > now {
                Some(dt)
            } else if event.year_explicit {
                None
            } else {
                let next = NaiveDate::from_ymd_opt(d.year() + 1, d.month(), d.day())?;
                Some(next.and_hms_opt(0, 0, 0)?)
            }
        }
        (None, None) => None,
    }?;

    // If days-of-week restriction is set, advance to the next allowed day
    if let Some(ref days_str) = event.days {
        let allowed: Vec<Weekday> = days_str.split(',').filter_map(str_to_weekday).collect();
        let time = dt.time();
        let mut candidate = dt.date();
        for _ in 0..7 {
            if allowed.contains(&candidate.weekday()) && candidate.and_time(time) > now {
                return Some(candidate.and_time(time));
            }
            candidate = candidate.succ_opt()?;
        }
        None
    } else {
        Some(dt)
    }
}

fn str_to_weekday(s: &str) -> Option<Weekday> {
    match s {
        "mon" => Some(Weekday::Mon),
        "tue" => Some(Weekday::Tue),
        "wed" => Some(Weekday::Wed),
        "thu" => Some(Weekday::Thu),
        "fri" => Some(Weekday::Fri),
        "sat" => Some(Weekday::Sat),
        "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

/// Parses a stored ordinal-weekday pattern like "first_sun" or "last_mon".
/// Returns `(n, weekday)` where `n = 0` means "last occurrence".
fn parse_ordinal_weekday(s: &str) -> Option<(u32, Weekday)> {
    let (ord_str, wd_str) = s.split_once('_')?;
    let n = match ord_str {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "last" => 0,
        _ => return None,
    };
    Some((n, str_to_weekday(wd_str)?))
}

fn nth_weekday_of_month(year: i32, month: u32, weekday: Weekday, n: u32) -> Option<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let offset = (weekday.num_days_from_monday() as i64
        - first.weekday().num_days_from_monday() as i64)
        .rem_euclid(7) as u32;
    let day = 1 + offset + (n - 1) * 7;
    let d = NaiveDate::from_ymd_opt(year, month, day)?;
    if d.month() == month {
        Some(d)
    } else {
        None
    }
}

fn last_weekday_of_month(year: i32, month: u32, weekday: Weekday) -> Option<NaiveDate> {
    let last = last_day_of_month_date(year, month)?;
    let offset = (last.weekday().num_days_from_monday() as i64
        - weekday.num_days_from_monday() as i64)
        .rem_euclid(7) as u32;
    NaiveDate::from_ymd_opt(year, month, last.day() - offset)
}

fn last_day_of_month_date(year: i32, month: u32) -> Option<NaiveDate> {
    if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?.pred_opt()
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?.pred_opt()
    }
}

fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

fn advance_by(dt: NaiveDateTime, interval: u32, unit: &str) -> Option<NaiveDateTime> {
    match unit {
        "minutes" => Some(dt + chrono::Duration::minutes(interval as i64)),
        "hours" => Some(dt + chrono::Duration::hours(interval as i64)),
        "days" => Some(dt + chrono::Duration::days(interval as i64)),
        "weeks" => Some(dt + chrono::Duration::weeks(interval as i64)),
        "months" => {
            let new_date = dt
                .date()
                .checked_add_months(chrono::Months::new(interval))?;
            Some(new_date.and_time(dt.time()))
        }
        "years" => {
            let new_date = dt
                .date()
                .checked_add_months(chrono::Months::new(interval * 12))?;
            Some(new_date.and_time(dt.time()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_play_event() -> StoredEvent {
        StoredEvent {
            id: 0,
            chat_id: 0,
            date: None,
            time: None,
            year_explicit: false,
            days: None,
            message: String::new(),
            active: false,
            next_datetime: None,
            created_at: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            repeat_interval: None,
            repeat_unit: None,
            in_offset: None,
            in_offset_unit: None,
            bare_hour: None,
            monthly_pattern: None,
            msg_id: 0,
        }
    }

    #[test]
    fn play_time_only_future_today() {
        let t = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.time = Some(t);
        let result = calc_next(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert!(dt > now || dt.date() == now.date().succ_opt().unwrap());
    }

    #[test]
    fn play_explicit_year_past_returns_inactive() {
        let mut event = make_play_event();
        event.date = NaiveDate::from_ymd_opt(2020, 1, 1);
        event.time = NaiveTime::from_hms_opt(12, 0, 0);
        event.year_explicit = true;
        let result = calc_next(event);
        assert!(!result.active);
        assert!(result.next_datetime.is_none());
    }

    #[test]
    fn play_short_date_past_wraps_to_next_year() {
        let past = Local::now().naive_local() - chrono::Duration::days(2);
        let mut event = make_play_event();
        event.date = Some(past.date());
        event.time = Some(past.time());
        event.year_explicit = false;
        let result = calc_next(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert_eq!(dt.date().year(), past.date().year() + 1);
    }

    #[test]
    fn play_skips_disallowed_day() {
        let t = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
        let now = Local::now().naive_local();
        let today = now.date();
        let target_day = today.weekday().succ();
        let day_str = match target_day {
            Weekday::Mon => "mon",
            Weekday::Tue => "tue",
            Weekday::Wed => "wed",
            Weekday::Thu => "thu",
            Weekday::Fri => "fri",
            Weekday::Sat => "sat",
            Weekday::Sun => "sun",
        };

        let mut event = make_play_event();
        event.time = Some(t);
        event.days = Some(day_str.to_string());
        let result = calc_next(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert_eq!(dt.date().weekday(), target_day);
        assert!(dt > now);
    }

    #[test]
    fn play_in_offset_minutes() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.in_offset = Some(10);
        event.in_offset_unit = Some("minutes".to_string());
        let result = calc_next(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        let diff = dt.signed_duration_since(now).num_minutes();
        assert!(diff >= 9 && diff <= 11);
    }

    #[test]
    fn play_in_offset_hours() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.in_offset = Some(2);
        event.in_offset_unit = Some("hours".to_string());
        let result = calc_next(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        let diff = dt.signed_duration_since(now).num_hours();
        assert!(diff >= 1 && diff <= 3);
    }

    #[test]
    fn play_monthly_first_sunday() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.time = Some(NaiveTime::from_hms_opt(10, 0, 0).unwrap());
        event.monthly_pattern = Some("first_sun".to_string());
        let result = calc_next(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert!(dt > now);
        assert_eq!(dt.date().weekday(), Weekday::Sun);
        assert!(dt.date().day() <= 7);
    }

    #[test]
    fn play_monthly_last_monday() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        event.monthly_pattern = Some("last_mon".to_string());
        let result = calc_next(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert!(dt > now);
        assert_eq!(dt.date().weekday(), Weekday::Mon);
        let next_week = dt.date() + chrono::Duration::days(7);
        assert_ne!(next_week.month(), dt.date().month());
    }

    #[test]
    fn play_monthly_last_day() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.time = Some(NaiveTime::from_hms_opt(18, 0, 0).unwrap());
        event.monthly_pattern = Some("last_day".to_string());
        let result = calc_next(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert!(dt > now);
        let next_day = dt.date().succ_opt().unwrap();
        assert_ne!(next_day.month(), dt.date().month());
    }

    #[test]
    fn play_monthly_no_time_uses_midnight() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.monthly_pattern = Some("second_wed".to_string());
        let result = calc_next(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert!(dt > now);
        assert_eq!(dt.time(), NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        assert_eq!(dt.date().weekday(), Weekday::Wed);
    }

    // --- Helper function unit tests ---

    #[test]
    fn test_nth_weekday_of_month() {
        // March 2026: first day is Sunday
        // First Monday = March 2
        assert_eq!(
            nth_weekday_of_month(2026, 3, Weekday::Mon, 1),
            NaiveDate::from_ymd_opt(2026, 3, 2)
        );
        // Second Monday = March 9
        assert_eq!(
            nth_weekday_of_month(2026, 3, Weekday::Mon, 2),
            NaiveDate::from_ymd_opt(2026, 3, 9)
        );
        // First Sunday = March 1
        assert_eq!(
            nth_weekday_of_month(2026, 3, Weekday::Sun, 1),
            NaiveDate::from_ymd_opt(2026, 3, 1)
        );
        // Fourth Friday = March 27
        assert_eq!(
            nth_weekday_of_month(2026, 3, Weekday::Fri, 4),
            NaiveDate::from_ymd_opt(2026, 3, 27)
        );
    }

    #[test]
    fn test_last_weekday_of_month() {
        // March 2026: last day is Tuesday March 31
        assert_eq!(
            last_weekday_of_month(2026, 3, Weekday::Tue),
            NaiveDate::from_ymd_opt(2026, 3, 31)
        );
        // Last Sunday in March 2026 = March 29
        assert_eq!(
            last_weekday_of_month(2026, 3, Weekday::Sun),
            NaiveDate::from_ymd_opt(2026, 3, 29)
        );
    }

    #[test]
    fn test_last_day_of_month_date() {
        assert_eq!(
            last_day_of_month_date(2026, 2),
            NaiveDate::from_ymd_opt(2026, 2, 28)
        );
        assert_eq!(
            last_day_of_month_date(2024, 2),
            NaiveDate::from_ymd_opt(2024, 2, 29)
        );
        assert_eq!(
            last_day_of_month_date(2026, 12),
            NaiveDate::from_ymd_opt(2026, 12, 31)
        );
    }
}
