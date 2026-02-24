use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

use crate::parser::{EventInfo, MonthlyPattern, Ordinal, TimeUnit};

/// Calculates the next occurrence datetime for an event and returns the
/// updated event. Sets `active = true` and `next_datetime = Some(dt)` when a
/// future datetime can be determined, otherwise `active = false` and
/// `next_datetime = None`.
pub fn calc_next(event: EventInfo) -> EventInfo {
    calc_next_at(event, Local::now().naive_local())
}

pub fn calc_next_at(event: EventInfo, now: NaiveDateTime) -> EventInfo {
    let next_datetime = calculate_next_datetime(&event, now);
    EventInfo {
        active: next_datetime.is_some(),
        next_datetime,
        ..event
    }
}

fn calculate_next_datetime(event: &EventInfo, now: NaiveDateTime) -> Option<NaiveDateTime> {
    // Handle bare hour (e.g., bare_hour=8 -> next 08:00)
    if let Some(h) = event.bare_hour {
        // One-shot: already scheduled and no repetition means it has fired
        if event.next_datetime.is_some() && event.repetition.is_none() {
            return None;
        }
        let hour = if h == 24 { 0 } else { h };
        let t = NaiveTime::from_hms_opt(hour, 0, 0)?;
        // Repeating: advance from the previously scheduled datetime by the interval
        if let (Some(base), Some(rep)) = (event.next_datetime, event.repetition.as_ref()) {
            let mut next = base;
            while next <= now {
                next = advance_by(next, rep.interval, rep.unit)?;
            }
            return Some(next);
        }
        let today = now.date();
        let dt = today.and_time(t);
        return if dt > now {
            Some(dt)
        } else {
            let tomorrow = today.succ_opt()?;
            Some(tomorrow.and_time(t))
        };
    }

    // Handle relative offset (e.g., in_offset=Some((8, TimeUnit::Minutes)))
    if let Some((value, unit)) = event.in_offset {
        // One-shot: already scheduled and no repetition means it has fired
        if event.next_datetime.is_some() && event.repetition.is_none() {
            return None;
        }
        // Repeating: advance from the previously scheduled datetime by the interval
        if let (Some(base), Some(rep)) = (event.next_datetime, event.repetition.as_ref()) {
            let mut next = base;
            while next <= now {
                next = advance_by(next, rep.interval, rep.unit)?;
            }
            return Some(next);
        }
        // First scheduling: fire at now + offset
        return match unit {
            TimeUnit::Minutes => Some(now + chrono::Duration::minutes(value as i64)),
            TimeUnit::Hours => Some(now + chrono::Duration::hours(value as i64)),
            TimeUnit::Days => Some(now + chrono::Duration::days(value as i64)),
            TimeUnit::Weeks => Some(now + chrono::Duration::weeks(value as i64)),
            TimeUnit::Months => {
                let new_date = now.date().checked_add_months(chrono::Months::new(value))?;
                Some(new_date.and_time(now.time()))
            }
            TimeUnit::Years => {
                let new_date = now
                    .date()
                    .checked_add_months(chrono::Months::new(value * 12))?;
                Some(new_date.and_time(now.time()))
            }
        };
    }

    // Handle years restriction: fire at the given time within the specified years only,
    // optionally filtered by weekdays. "11:13 2027,2028" fires every day; "13:25 2027 fri,sun"
    // fires only on Fridays and Sundays.
    if let Some(ref year_set) = event.years {
        let time = event
            .time
            .unwrap_or(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let mut sorted_years: Vec<i32> = year_set.iter().copied().collect();
        sorted_years.sort();
        for &year in &sorted_years {
            if now.date().year() > year {
                continue; // entirely in the past
            }
            let start_date = if now.date().year() < year {
                NaiveDate::from_ymd_opt(year, 1, 1)?
            } else {
                now.date()
            };
            let mut candidate = start_date;
            loop {
                if candidate.year() > year {
                    break; // no more days in this year — try next year
                }
                let candidate_dt = candidate.and_time(time);
                let day_ok = event
                    .days
                    .as_ref()
                    .map_or(true, |days| days.contains(&candidate.weekday()));
                if day_ok && candidate_dt > now {
                    return Some(candidate_dt);
                }
                candidate = candidate.succ_opt()?;
            }
        }
        return None;
    }

    // Handle monthly pattern (e.g., OrdinalWeekday(First, Sun), LastDay)
    if let Some(ref pattern) = event.monthly_pattern {
        let time = event
            .time
            .unwrap_or(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let today = now.date();
        let mut year = today.year();
        let mut month = today.month();

        for _ in 0..3 {
            let target_date = match pattern {
                MonthlyPattern::LastDay => last_day_of_month_date(year, month),
                MonthlyPattern::OrdinalWeekday(ord, wd) => {
                    let n = match ord {
                        Ordinal::First => 1,
                        Ordinal::Second => 2,
                        Ordinal::Third => 3,
                        Ordinal::Fourth => 4,
                        Ordinal::Last => 0,
                    };
                    if n == 0 {
                        last_weekday_of_month(year, month, *wd)
                    } else {
                        nth_weekday_of_month(year, month, *wd, n)
                    }
                }
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
                if let (Some(base), Some(rep)) = (event.next_datetime, event.repetition.as_ref()) {
                    let mut next = base;
                    while next <= now {
                        next = advance_by(next, rep.interval, rep.unit)?;
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
            if event.next_datetime.is_some() && event.repetition.is_none() && event.days.is_none() {
                return None;
            }
            // Repeating event: advance from the previously scheduled datetime by
            // the repeat interval until a future datetime is found.
            if let (Some(base), Some(rep)) = (event.next_datetime, event.repetition.as_ref()) {
                let mut next = base;
                while next <= now {
                    next = advance_by(next, rep.interval, rep.unit)?;
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
    if let Some(ref days) = event.days {
        let time = dt.time();
        let mut candidate = dt.date();
        for _ in 0..7 {
            if days.contains(&candidate.weekday()) && candidate.and_time(time) > now {
                return Some(candidate.and_time(time));
            }
            candidate = candidate.succ_opt()?;
        }
        None
    } else {
        Some(dt)
    }
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

fn advance_by(dt: NaiveDateTime, interval: u32, unit: TimeUnit) -> Option<NaiveDateTime> {
    match unit {
        TimeUnit::Minutes => Some(dt + chrono::Duration::minutes(interval as i64)),
        TimeUnit::Hours => Some(dt + chrono::Duration::hours(interval as i64)),
        TimeUnit::Days => Some(dt + chrono::Duration::days(interval as i64)),
        TimeUnit::Weeks => Some(dt + chrono::Duration::weeks(interval as i64)),
        TimeUnit::Months => {
            let new_date = dt
                .date()
                .checked_add_months(chrono::Months::new(interval))?;
            Some(new_date.and_time(dt.time()))
        }
        TimeUnit::Years => {
            let new_date = dt
                .date()
                .checked_add_months(chrono::Months::new(interval * 12))?;
            Some(new_date.and_time(dt.time()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{EventInfo, MonthlyPattern, Ordinal, TimeUnit};
    use std::collections::HashSet;

    fn make_play_event() -> EventInfo {
        EventInfo {
            id: 0,
            chat_id: 0,
            date: None,
            time: None,
            year_explicit: false,
            days: None,
            years: None,
            message: String::new(),
            active: false,
            next_datetime: None,
            created_at: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            repetition: None,
            in_offset: None,
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

        let mut event = make_play_event();
        event.time = Some(t);
        event.days = Some(HashSet::from([target_day]));
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
        event.in_offset = Some((10, TimeUnit::Minutes));
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
        event.in_offset = Some((2, TimeUnit::Hours));
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
        event.monthly_pattern = Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Sun));
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
        event.monthly_pattern = Some(MonthlyPattern::OrdinalWeekday(Ordinal::Last, Weekday::Mon));
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
        event.monthly_pattern = Some(MonthlyPattern::LastDay);
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
        event.monthly_pattern = Some(MonthlyPattern::OrdinalWeekday(
            Ordinal::Second,
            Weekday::Wed,
        ));
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
