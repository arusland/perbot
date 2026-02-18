use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

use crate::parser::{MonthlyPattern, Ordinal, ParsedEvent, TimeUnit};
use crate::storage::StoredEvent;

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

/// Resolves a `ParsedEvent` to a concrete future `NaiveDateTime`.
pub fn resolve_datetime(event: &ParsedEvent) -> Option<NaiveDateTime> {
    let now = Local::now().naive_local();

    // Handle bare hour (e.g., "8 call Alex" -> next 08:00)
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

    // Handle relative offset (e.g., "8 min call her", "2 hours reminder")
    if let Some((value, unit)) = &event.in_offset {
        return match unit {
            TimeUnit::Minutes => Some(now + chrono::Duration::minutes(*value as i64)),
            TimeUnit::Hours => Some(now + chrono::Duration::hours(*value as i64)),
            TimeUnit::Days => Some(now + chrono::Duration::days(*value as i64)),
            TimeUnit::Weeks => Some(now + chrono::Duration::weeks(*value as i64)),
            TimeUnit::Months => {
                let new_date = now.date().checked_add_months(chrono::Months::new(*value))?;
                Some(new_date.and_time(now.time()))
            }
            TimeUnit::Years => {
                let new_date = now
                    .date()
                    .checked_add_months(chrono::Months::new(*value * 12))?;
                Some(new_date.and_time(now.time()))
            }
        };
    }

    // Handle monthly pattern (e.g., "first sunday", "last monday", "last day")
    if let Some(ref pattern) = event.monthly_pattern {
        let time = event
            .time
            .unwrap_or(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let today = now.date();
        let mut year = today.year();
        let mut month = today.month();

        for _ in 0..3 {
            let target_date = match pattern {
                MonthlyPattern::OrdinalWeekday(ordinal, weekday) => match ordinal {
                    Ordinal::First => nth_weekday_of_month(year, month, *weekday, 1),
                    Ordinal::Second => nth_weekday_of_month(year, month, *weekday, 2),
                    Ordinal::Third => nth_weekday_of_month(year, month, *weekday, 3),
                    Ordinal::Fourth => nth_weekday_of_month(year, month, *weekday, 4),
                    Ordinal::Last => last_weekday_of_month(year, month, *weekday),
                },
                MonthlyPattern::LastDay => last_day_of_month_date(year, month),
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
                None
            } else {
                // Short date in the past — try next year
                let next = NaiveDate::from_ymd_opt(d.year() + 1, d.month(), d.day())?;
                Some(next.and_time(t))
            }
        }
        (Some(t), None) => {
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
    if let Some(ref allowed_days) = event.days {
        let time = dt.time();
        let mut candidate = dt.date();
        for _ in 0..7 {
            if allowed_days.contains(&candidate.weekday()) && candidate.and_time(time) > now {
                return Some(candidate.and_time(time));
            }
            candidate = candidate.succ_opt()?;
        }
        // All 7 days checked — wrap to first allowed day next week
        None
    } else {
        Some(dt)
    }
}

/// Converts a `ParsedEvent` into a `StoredEvent` ready for persistence.
/// Returns `None` if the event cannot be resolved to a future datetime.
pub fn map(event: ParsedEvent, chat_id: i64) -> Option<StoredEvent> {
    let target_datetime = resolve_datetime(&event)?;
    let now = Local::now().naive_local();

    let days = event.days.as_ref().map(|days| {
        let mut day_strs: Vec<&str> = days
            .iter()
            .map(|d| match d {
                Weekday::Mon => "mon",
                Weekday::Tue => "tue",
                Weekday::Wed => "wed",
                Weekday::Thu => "thu",
                Weekday::Fri => "fri",
                Weekday::Sat => "sat",
                Weekday::Sun => "sun",
            })
            .collect();
        let order = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
        day_strs.sort_by_key(|d| order.iter().position(|o| o == d).unwrap_or(7));
        day_strs.join(",")
    });

    let (repeat_interval, repeat_unit) = match &event.repetition {
        Some(rep) => (
            Some(rep.interval),
            Some(
                match rep.unit {
                    TimeUnit::Minutes => "minutes",
                    TimeUnit::Hours => "hours",
                    TimeUnit::Days => "days",
                    TimeUnit::Weeks => "weeks",
                    TimeUnit::Months => "months",
                    TimeUnit::Years => "years",
                }
                .to_string(),
            ),
        ),
        None => (None, None),
    };

    let (in_offset, in_offset_unit) = match &event.in_offset {
        Some((value, unit)) => (
            Some(*value),
            Some(
                match unit {
                    TimeUnit::Minutes => "minutes",
                    TimeUnit::Hours => "hours",
                    TimeUnit::Days => "days",
                    TimeUnit::Weeks => "weeks",
                    TimeUnit::Months => "months",
                    TimeUnit::Years => "years",
                }
                .to_string(),
            ),
        ),
        None => (None, None),
    };

    let monthly_pattern = event.monthly_pattern.as_ref().map(|p| match p {
        MonthlyPattern::OrdinalWeekday(ord, wd) => {
            let ord_str = match ord {
                Ordinal::First => "first",
                Ordinal::Second => "second",
                Ordinal::Third => "third",
                Ordinal::Fourth => "fourth",
                Ordinal::Last => "last",
            };
            let wd_str = match wd {
                Weekday::Mon => "mon",
                Weekday::Tue => "tue",
                Weekday::Wed => "wed",
                Weekday::Thu => "thu",
                Weekday::Fri => "fri",
                Weekday::Sat => "sat",
                Weekday::Sun => "sun",
            };
            format!("{}_{}", ord_str, wd_str)
        }
        MonthlyPattern::LastDay => "last_day".to_string(),
    });

    Some(StoredEvent {
        id: 0,
        chat_id,
        date: event.date,
        time: event.time,
        year_explicit: event.year_explicit,
        days,
        message: event.message,
        target_datetime,
        created_at: now,
        fired: false,
        repeat_interval,
        repeat_unit,
        dismissed: false,
        in_offset,
        in_offset_unit,
        bare_hour: event.bare_hour,
        monthly_pattern,
        raw_msg: event.raw_msg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Local, NaiveTime, Weekday};
    use std::collections::HashSet;

    use crate::parser::{MonthlyPattern, Ordinal, ParsedEvent, TimeUnit};

    // --- resolve_datetime tests (moved from parser::tests) ---

    #[test]
    fn resolve_time_only_future_today() {
        let t = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
        let now = Local::now().naive_local();
        let event = ParsedEvent {
            date: None,
            time: Some(t),
            year_explicit: false,
            days: None,
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: None,
            message: "test".into(),
            raw_msg: String::new(),
        };
        let dt = resolve_datetime(&event).unwrap();
        assert!(dt > now || dt.date() == now.date().succ_opt().unwrap());
    }

    #[test]
    fn resolve_explicit_year_past_returns_none() {
        let event = ParsedEvent {
            date: NaiveDate::from_ymd_opt(2020, 1, 1),
            time: NaiveTime::from_hms_opt(12, 0, 0),
            year_explicit: true,
            days: None,
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: None,
            message: "old".into(),
            raw_msg: String::new(),
        };
        assert!(resolve_datetime(&event).is_none());
    }

    #[test]
    fn resolve_short_date_past_wraps_to_next_year() {
        let past = Local::now().naive_local() - chrono::Duration::days(2);
        let event = ParsedEvent {
            date: Some(past.date()),
            time: Some(past.time()),
            year_explicit: false,
            days: None,
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: None,
            message: "wrap".into(),
            raw_msg: String::new(),
        };
        let dt = resolve_datetime(&event).unwrap();
        assert_eq!(dt.date().year(), past.date().year() + 1);
    }

    #[test]
    fn resolve_skips_disallowed_day() {
        let t = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
        let now = Local::now().naive_local();
        let today = now.date();
        let disallowed_today = today.weekday();
        let target_day = disallowed_today.succ();
        let days: HashSet<Weekday> = [target_day].into_iter().collect();

        let event = ParsedEvent {
            date: None,
            time: Some(t),
            year_explicit: false,
            days: Some(days),
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: None,
            message: "skip".into(),
            raw_msg: String::new(),
        };
        let dt = resolve_datetime(&event).unwrap();
        assert_eq!(dt.date().weekday(), target_day);
        assert!(dt > now);
    }

    #[test]
    fn resolve_in_offset_minutes() {
        let now = Local::now().naive_local();
        let event = ParsedEvent {
            date: None,
            time: None,
            year_explicit: false,
            days: None,
            repetition: None,
            in_offset: Some((10, TimeUnit::Minutes)),
            bare_hour: None,
            monthly_pattern: None,
            message: "test".into(),
            raw_msg: String::new(),
        };
        let dt = resolve_datetime(&event).unwrap();
        let diff = dt.signed_duration_since(now).num_minutes();
        assert!(diff >= 9 && diff <= 11);
    }

    #[test]
    fn resolve_in_offset_hours() {
        let now = Local::now().naive_local();
        let event = ParsedEvent {
            date: None,
            time: None,
            year_explicit: false,
            days: None,
            repetition: None,
            in_offset: Some((2, TimeUnit::Hours)),
            bare_hour: None,
            monthly_pattern: None,
            message: "test".into(),
            raw_msg: String::new(),
        };
        let dt = resolve_datetime(&event).unwrap();
        let diff = dt.signed_duration_since(now).num_hours();
        assert!(diff >= 1 && diff <= 3);
    }

    #[test]
    fn resolve_monthly_first_sunday() {
        let now = Local::now().naive_local();
        let event = ParsedEvent {
            date: None,
            time: NaiveTime::from_hms_opt(10, 0, 0),
            year_explicit: false,
            days: None,
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Sun)),
            message: "test".into(),
            raw_msg: String::new(),
        };
        let dt = resolve_datetime(&event).unwrap();
        assert!(dt > now);
        assert_eq!(dt.date().weekday(), Weekday::Sun);
        assert!(dt.date().day() <= 7);
    }

    #[test]
    fn resolve_monthly_last_monday() {
        let now = Local::now().naive_local();
        let event = ParsedEvent {
            date: None,
            time: NaiveTime::from_hms_opt(9, 0, 0),
            year_explicit: false,
            days: None,
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: Some(MonthlyPattern::OrdinalWeekday(Ordinal::Last, Weekday::Mon)),
            message: "test".into(),
            raw_msg: String::new(),
        };
        let dt = resolve_datetime(&event).unwrap();
        assert!(dt > now);
        assert_eq!(dt.date().weekday(), Weekday::Mon);
        let next_week = dt.date() + chrono::Duration::days(7);
        assert_ne!(next_week.month(), dt.date().month());
    }

    #[test]
    fn resolve_monthly_last_day() {
        let now = Local::now().naive_local();
        let event = ParsedEvent {
            date: None,
            time: NaiveTime::from_hms_opt(18, 0, 0),
            year_explicit: false,
            days: None,
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: Some(MonthlyPattern::LastDay),
            message: "test".into(),
            raw_msg: String::new(),
        };
        let dt = resolve_datetime(&event).unwrap();
        assert!(dt > now);
        let next_day = dt.date().succ_opt().unwrap();
        assert_ne!(next_day.month(), dt.date().month());
    }

    #[test]
    fn resolve_monthly_no_time_uses_midnight() {
        let now = Local::now().naive_local();
        let event = ParsedEvent {
            date: None,
            time: None,
            year_explicit: false,
            days: None,
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: Some(MonthlyPattern::OrdinalWeekday(
                Ordinal::Second,
                Weekday::Wed,
            )),
            message: "test".into(),
            raw_msg: String::new(),
        };
        let dt = resolve_datetime(&event).unwrap();
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
