use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

/// Placeholder for future period support.
pub struct Period;

/// Placeholder for future repetition support.
pub struct Repetition;

#[allow(dead_code)]
pub struct ParsedEvent {
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub year_explicit: bool,
    pub days: Option<HashSet<Weekday>>,
    pub period: Option<Period>,
    pub repetition: Option<Repetition>,
    pub message: String,
}

static RE_TIME_12H: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\d{1,2}):(\d{2})\s*(AM|PM)").unwrap());

static RE_TIME_24H: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{1,2}):(\d{2})").unwrap());

static RE_DATE_FULL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2})\.(\d{1,2})\.(\d{4})").unwrap());

static RE_DATE_SHORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2})\.(\d{1,2})(?:[^\.\d]|$)").unwrap());

static RE_DAYS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b((?:mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?)(?:\s*[-,]\s*(?:mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?))*)\b").unwrap()
});

fn day_from_str(s: &str) -> Option<Weekday> {
    match s.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Some(Weekday::Mon),
        "tue" | "tuesday" => Some(Weekday::Tue),
        "wed" | "wednesday" => Some(Weekday::Wed),
        "thu" | "thursday" => Some(Weekday::Thu),
        "fri" | "friday" => Some(Weekday::Fri),
        "sat" | "saturday" => Some(Weekday::Sat),
        "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

fn day_index(d: Weekday) -> u8 {
    match d {
        Weekday::Mon => 0,
        Weekday::Tue => 1,
        Weekday::Wed => 2,
        Weekday::Thu => 3,
        Weekday::Fri => 4,
        Weekday::Sat => 5,
        Weekday::Sun => 6,
    }
}

fn day_from_index(i: u8) -> Weekday {
    match i % 7 {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        _ => Weekday::Sun,
    }
}

pub fn parse_days(s: &str) -> Option<HashSet<Weekday>> {
    let mut set = HashSet::new();
    for token in s.split(',') {
        let token = token.trim();
        if let Some(dash_pos) = token.find('-') {
            let left = token[..dash_pos].trim();
            let right = token[dash_pos + 1..].trim();
            let start = day_from_str(left)?;
            let end = day_from_str(right)?;
            let mut i = day_index(start);
            let end_i = day_index(end);
            loop {
                set.insert(day_from_index(i));
                if i == end_i {
                    break;
                }
                i = (i + 1) % 7;
            }
        } else {
            set.insert(day_from_str(token)?);
        }
    }
    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}

pub fn parse(input: &str) -> Option<ParsedEvent> {
    let mut remaining = input.to_string();
    let mut time: Option<NaiveTime> = None;
    let mut date: Option<NaiveDate> = None;
    let mut year_explicit = false;

    // 12h time (must be checked before 24h to avoid partial match)
    if let Some(caps) = RE_TIME_12H.captures(&remaining) {
        let mut hour: u32 = caps[1].parse().ok()?;
        let minute: u32 = caps[2].parse().ok()?;
        let ampm = caps[3].to_ascii_uppercase();

        if hour > 12 || minute >= 60 || hour == 0 {
            return None;
        }

        if ampm == "PM" && hour != 12 {
            hour += 12;
        } else if ampm == "AM" && hour == 12 {
            hour = 0;
        }

        time = NaiveTime::from_hms_opt(hour, minute, 0);
        if time.is_none() {
            return None;
        }
        remaining = remaining[..caps.get(0).unwrap().start()].to_string()
            + &remaining[caps.get(0).unwrap().end()..];
    } else if let Some(caps) = RE_TIME_24H.captures(&remaining) {
        let hour: u32 = caps[1].parse().ok()?;
        let minute: u32 = caps[2].parse().ok()?;

        time = NaiveTime::from_hms_opt(hour, minute, 0);
        if time.is_none() {
            return None;
        }
        remaining = remaining[..caps.get(0).unwrap().start()].to_string()
            + &remaining[caps.get(0).unwrap().end()..];
    }

    // Full date (must be checked before short date)
    if let Some(caps) = RE_DATE_FULL.captures(&remaining) {
        let day: u32 = caps[1].parse().ok()?;
        let month: u32 = caps[2].parse().ok()?;
        let year: i32 = caps[3].parse().ok()?;

        date = NaiveDate::from_ymd_opt(year, month, day);
        if date.is_none() {
            return None;
        }
        year_explicit = true;
        remaining = remaining[..caps.get(0).unwrap().start()].to_string()
            + &remaining[caps.get(0).unwrap().end()..];
    } else if let Some(caps) = RE_DATE_SHORT.captures(&remaining) {
        let day: u32 = caps[1].parse().ok()?;
        let month: u32 = caps[2].parse().ok()?;
        let year = Local::now().year();

        date = NaiveDate::from_ymd_opt(year, month, day);
        if date.is_none() {
            return None;
        }
        remaining = remaining[..caps.get(0).unwrap().start()].to_string()
            + &remaining[caps.get(0).unwrap().end()..];
    }

    // Days of week
    let mut days: Option<HashSet<Weekday>> = None;
    if let Some(caps) = RE_DAYS.captures(&remaining) {
        if let Some(parsed) = parse_days(&caps[1]) {
            days = Some(parsed);
            remaining = remaining[..caps.get(0).unwrap().start()].to_string()
                + &remaining[caps.get(0).unwrap().end()..];
        }
    }

    let message = remaining.split_whitespace().collect::<Vec<_>>().join(" ");

    if time.is_none() && date.is_none() {
        return None;
    }
    if message.is_empty() {
        return None;
    }

    Some(ParsedEvent {
        date,
        time,
        year_explicit,
        days,
        period: None,
        repetition: None,
        message,
    })
}

pub fn resolve_datetime(event: &ParsedEvent) -> Option<NaiveDateTime> {
    let now = Local::now().naive_local();

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn parse_24h_time_with_message() {
        let e = parse("13:23 lunch meeting").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(13, 23, 0));
        assert!(e.date.is_none());
        assert_eq!(e.message, "lunch meeting");
    }

    #[test]
    fn parse_12h_time_am() {
        let e = parse("5:24 AM wake up").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(5, 24, 0));
        assert_eq!(e.message, "wake up");
    }

    #[test]
    fn parse_12h_time_pm() {
        let e = parse("5:24 PM evening walk").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(17, 24, 0));
        assert_eq!(e.message, "evening walk");
    }

    #[test]
    fn parse_12h_12pm_is_noon() {
        let e = parse("12:00 PM noon bell").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(12, 0, 0));
    }

    #[test]
    fn parse_12h_12am_is_midnight() {
        let e = parse("12:00 AM midnight snack").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(0, 0, 0));
    }

    #[test]
    fn parse_12h_case_insensitive() {
        let e = parse("3:30 pm tea time").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(15, 30, 0));
    }

    #[test]
    fn parse_time_and_short_date() {
        let e = parse("1:23 26.11 birthday reminder").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(1, 23, 0));
        let d = e.date.unwrap();
        assert_eq!(d.day(), 26);
        assert_eq!(d.month(), 11);
        assert!(!e.year_explicit);
        assert_eq!(e.message, "birthday reminder");
    }

    #[test]
    fn parse_full_date_only() {
        let e = parse("31.12.2027 new years eve").unwrap();
        assert!(e.time.is_none());
        let d = e.date.unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2027, 12, 31).unwrap());
        assert!(e.year_explicit);
        assert_eq!(e.message, "new years eve");
    }

    #[test]
    fn parse_full_date_with_time() {
        let e = parse("23:59 31.12.2027 fireworks").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(23, 59, 0));
        assert_eq!(
            e.date.unwrap(),
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap()
        );
        assert!(e.year_explicit);
        assert_eq!(e.message, "fireworks");
    }

    #[test]
    fn parse_no_time_no_date_returns_none() {
        assert!(parse("just a normal message").is_none());
    }

    #[test]
    fn parse_time_but_no_message_returns_none() {
        assert!(parse("13:00").is_none());
        assert!(parse("13:00   ").is_none());
    }

    #[test]
    fn parse_invalid_hour_returns_none() {
        assert!(parse("25:00 bad time").is_none());
    }

    #[test]
    fn parse_invalid_minute_returns_none() {
        assert!(parse("12:61 bad minute").is_none());
    }

    #[test]
    fn parse_invalid_12h_hour_returns_none() {
        assert!(parse("0:30 AM invalid").is_none());
        assert!(parse("13:00 PM invalid").is_none());
    }

    #[test]
    fn parse_invalid_date_returns_none() {
        assert!(parse("32.13.2025 bad date").is_none());
    }

    #[test]
    fn resolve_time_only_future_today() {
        let t = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
        let now = Local::now().naive_local();
        // If 23:59 hasn't passed yet, it should be today; otherwise tomorrow.
        let event = ParsedEvent {
            date: None,
            time: Some(t),
            year_explicit: false,
            days: None,
            period: None,
            repetition: None,
            message: "test".into(),
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
            period: None,
            repetition: None,
            message: "old".into(),
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
            period: None,
            repetition: None,
            message: "wrap".into(),
        };
        let dt = resolve_datetime(&event).unwrap();
        assert_eq!(dt.date().year(), past.date().year() + 1);
    }

    // --- Days-of-week tests ---

    #[test]
    fn parse_days_range() {
        let days = parse_days("mon-fri").unwrap();
        let expected: HashSet<Weekday> = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ]
        .into_iter()
        .collect();
        assert_eq!(days, expected);
    }

    #[test]
    fn parse_days_comma() {
        let days = parse_days("sunday,Thu").unwrap();
        let expected: HashSet<Weekday> = [Weekday::Sun, Weekday::Thu].into_iter().collect();
        assert_eq!(days, expected);
    }

    #[test]
    fn parse_days_mixed() {
        let days = parse_days("mon-wed,fri").unwrap();
        let expected: HashSet<Weekday> = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Fri,
        ]
        .into_iter()
        .collect();
        assert_eq!(days, expected);
    }

    #[test]
    fn parse_days_case_insensitive() {
        let days = parse_days("MONDAY,tue").unwrap();
        let expected: HashSet<Weekday> = [Weekday::Mon, Weekday::Tue].into_iter().collect();
        assert_eq!(days, expected);
    }

    #[test]
    fn parse_days_full_names() {
        let days = parse_days("wednesday").unwrap();
        let expected: HashSet<Weekday> = [Weekday::Wed].into_iter().collect();
        assert_eq!(days, expected);
    }

    #[test]
    fn parse_with_time_and_days() {
        let e = parse("13:30 mon-fri call office").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(13, 30, 0));
        let expected_days: HashSet<Weekday> = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ]
        .into_iter()
        .collect();
        assert_eq!(e.days, Some(expected_days));
        assert_eq!(e.message, "call office");
    }

    #[test]
    fn parse_with_time_date_and_days() {
        let e = parse("9:00 AM 15.03 sun,sat weekend task").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(9, 0, 0));
        assert!(e.date.is_some());
        let d = e.date.unwrap();
        assert_eq!(d.day(), 15);
        assert_eq!(d.month(), 3);
        let expected_days: HashSet<Weekday> = [Weekday::Sun, Weekday::Sat].into_iter().collect();
        assert_eq!(e.days, Some(expected_days));
        assert_eq!(e.message, "weekend task");
    }

    #[test]
    fn resolve_skips_disallowed_day() {
        // Pick a time far in the future so it's definitely not past
        let t = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
        let now = Local::now().naive_local();
        let today = now.date();
        // Restrict to a single day that is NOT today
        let disallowed_today = today.weekday();
        let target_day = disallowed_today.succ();
        let days: HashSet<Weekday> = [target_day].into_iter().collect();

        let event = ParsedEvent {
            date: None,
            time: Some(t),
            year_explicit: false,
            days: Some(days),
            period: None,
            repetition: None,
            message: "skip".into(),
        };
        let dt = resolve_datetime(&event).unwrap();
        assert_eq!(dt.date().weekday(), target_day);
        assert!(dt > now);
    }

    #[test]
    fn parse_single_day() {
        let e = parse("13:00 wed meeting").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(13, 0, 0));
        let expected_days: HashSet<Weekday> = [Weekday::Wed].into_iter().collect();
        assert_eq!(e.days, Some(expected_days));
        assert_eq!(e.message, "meeting");
    }
}
