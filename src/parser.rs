use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Years,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repetition {
    pub interval: u32,
    pub unit: TimeUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ordinal {
    First,
    Second,
    Third,
    Fourth,
    Fifth,
    Last,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthlyPattern {
    OrdinalWeekday(Ordinal, Weekday),
    LastDay,
}

#[derive(Debug, Clone)]
pub struct EventInfo {
    /// "26.11", "31.12.2027"
    pub date: Option<NaiveDate>,
    /// "13:23", "5:24 PM"
    pub time: Option<NaiveTime>,
    /// "31.12.2027" — true when year is given explicitly
    pub year_explicit: bool,
    /// "13:45 mon-fri", "13:25 thu-fri,sun"
    pub days: Option<HashSet<Weekday>>,
    /// "13:25 2027 fri,sun", "11:13 2027,2028"
    pub years: Option<HashSet<i32>>,
    /// "every 2 weeks", "every hour"
    pub repetition: Option<Repetition>,
    /// "8 min call her", "2 hours reminder"
    pub in_offset: Option<(u32, TimeUnit)>,
    /// "8 call Alex" → 8, "0 call Sacha" → 0, "24 call Poly" → 24
    pub bare_hour: Option<u32>,
    /// "first sunday", "last monday", "last day of the month"
    pub monthly_pattern: Option<MonthlyPattern>,
    /// remainder after extracting all time/date components
    pub message: String,
    pub id: i64,
    pub chat_id: i64,
    pub active: bool,
    pub next_datetime: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub msg_id: i64,
}

static RE_TIME_12H: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\d{1,2}):(\d{2})\s*(AM|PM)").unwrap());

static RE_TIME_24H: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{1,2}):(\d{2})").unwrap());

static RE_DATE_FULL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2})\.(\d{1,2})\.(\d{4})").unwrap());

static RE_DATE_SHORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2})\.(\d{1,2})(?:[^\.\d]|$)").unwrap());

static RE_EVERY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bevery\s+(?:(\d+)\s+)?(minutes?|hours?|days?|weeks?|months?|years?)\b")
        .unwrap()
});

static RE_IN_OFFSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(\d+)\s+(min(?:ute)?s?|hours?|days?|weeks?|months?|years?)\b").unwrap()
});

static RE_BARE_HOUR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,2})\s").unwrap());

static RE_DAYS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b((?:mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?)(?:\s*[-,]\s*(?:mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?))*)\b").unwrap()
});

static RE_MONTHLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(first|1st|second|2nd|third|3rd|fourth|4th|fifth|5th|last)\s+(mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?|day)(?:\s+of\s+the\s+month)?\b").unwrap()
});

static RE_YEARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{4}(?:\s*,\s*\d{4})*)\b").unwrap());

fn ordinal_from_str(s: &str) -> Option<Ordinal> {
    match s.to_ascii_lowercase().as_str() {
        "first" | "1st" => Some(Ordinal::First),
        "second" | "2nd" => Some(Ordinal::Second),
        "third" | "3rd" => Some(Ordinal::Third),
        "fourth" | "4th" => Some(Ordinal::Fourth),
        "fifth" | "5th" => Some(Ordinal::Fifth),
        "last" => Some(Ordinal::Last),
        _ => None,
    }
}

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
    if set.is_empty() { None } else { Some(set) }
}

pub fn unit_from_str(s: &str) -> Option<TimeUnit> {
    match s.to_ascii_lowercase().as_str() {
        "min" | "mins" | "minute" | "minutes" => Some(TimeUnit::Minutes),
        "hour" | "hours" => Some(TimeUnit::Hours),
        "day" | "days" => Some(TimeUnit::Days),
        "week" | "weeks" => Some(TimeUnit::Weeks),
        "month" | "months" => Some(TimeUnit::Months),
        "year" | "years" => Some(TimeUnit::Years),
        _ => None,
    }
}

pub fn parse(input: &str) -> Option<EventInfo> {
    let mut remaining = input.to_string();
    let mut time: Option<NaiveTime> = None;
    let mut date: Option<NaiveDate> = None;
    let mut year_explicit = false;
    let mut in_offset: Option<(u32, TimeUnit)> = None;
    let mut bare_hour: Option<u32> = None;
    let mut days: Option<HashSet<Weekday>> = None;
    let mut years: Option<HashSet<i32>> = None;
    let mut monthly_pattern: Option<MonthlyPattern> = None;

    // Relative offset: "N unit" e.g. "8 min call her", "2 hours reminder" (checked first)
    if let Some(caps) = RE_IN_OFFSET.captures(&remaining) {
        if let Ok(n) = caps[1].parse::<u32>() {
            if let Some(unit) = unit_from_str(&caps[2]) {
                in_offset = Some((n, unit));
                remaining = remaining[caps.get(0).unwrap().end()..].to_string();
            }
        }
    }

    if in_offset.is_none() {
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

        // Bare hour: "8 call Alex" -> bare_hour=8, "0 call Sacha" -> bare_hour=0
        if time.is_none() {
            if let Some(caps) = RE_BARE_HOUR.captures(&remaining) {
                if let Ok(n) = caps[1].parse::<u32>() {
                    if n <= 24 {
                        bare_hour = Some(n);
                        remaining = remaining[caps.get(0).unwrap().end()..].to_string();
                    }
                }
            }
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

        // Years: "2027", "2027,2028" — only when no full date was already parsed
        if date.is_none() {
            if let Some(m) = RE_YEARS.find(&remaining) {
                let year_set: HashSet<i32> = m
                    .as_str()
                    .split(',')
                    .filter_map(|s| s.trim().parse::<i32>().ok())
                    .filter(|&y| (2000..=2100).contains(&y))
                    .collect();
                if !year_set.is_empty() {
                    years = Some(year_set);
                    remaining = remaining[..m.start()].to_string() + &remaining[m.end()..];
                }
            }
        }

        // Monthly pattern: "first sunday", "last monday", "last day of the month"
        if let Some(caps) = RE_MONTHLY.captures(&remaining) {
            if let Some(ord) = ordinal_from_str(&caps[1]) {
                let target = caps[2].to_ascii_lowercase();
                let pattern = if target == "day" {
                    if ord == Ordinal::Last {
                        Some(MonthlyPattern::LastDay)
                    } else {
                        None
                    }
                } else {
                    day_from_str(&target).map(|wd| MonthlyPattern::OrdinalWeekday(ord, wd))
                };

                if pattern.is_some() {
                    monthly_pattern = pattern;
                    remaining = remaining[..caps.get(0).unwrap().start()].to_string()
                        + &remaining[caps.get(0).unwrap().end()..];
                }
            }
        }

        // Days of week (skip if monthly pattern already matched)
        if monthly_pattern.is_none() {
            if let Some(caps) = RE_DAYS.captures(&remaining) {
                if let Some(parsed) = parse_days(&caps[1]) {
                    days = Some(parsed);
                    remaining = remaining[..caps.get(0).unwrap().start()].to_string()
                        + &remaining[caps.get(0).unwrap().end()..];
                }
            }
        }
    }

    // Repetition: "every N unit" or "every unit" (checked for both offset and time modes)
    let mut repetition: Option<Repetition> = None;
    if let Some(caps) = RE_EVERY.captures(&remaining) {
        let interval: u32 = caps
            .get(1)
            .map(|m| m.as_str().parse().unwrap_or(1))
            .unwrap_or(1);
        if let Some(unit) = unit_from_str(&caps[2]) {
            repetition = Some(Repetition { interval, unit });
            remaining = remaining[..caps.get(0).unwrap().start()].to_string()
                + &remaining[caps.get(0).unwrap().end()..];
        }
    }

    let message = remaining.split_whitespace().collect::<Vec<_>>().join(" ");

    if time.is_none()
        && date.is_none()
        && in_offset.is_none()
        && bare_hour.is_none()
        && monthly_pattern.is_none()
    {
        return None;
    }
    if message.is_empty() {
        return None;
    }

    Some(EventInfo {
        date,
        time,
        year_explicit,
        days,
        years,
        repetition,
        in_offset,
        bare_hour,
        monthly_pattern,
        message,
        id: 0,
        chat_id: 0,
        active: false,
        next_datetime: None,
        created_at: Local::now().naive_local(),
        msg_id: 0,
    })
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
        let expected: HashSet<Weekday> = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Fri]
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
    fn parse_single_day() {
        let e = parse("13:00 wed meeting").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(13, 0, 0));
        let expected_days: HashSet<Weekday> = [Weekday::Wed].into_iter().collect();
        assert_eq!(e.days, Some(expected_days));
        assert_eq!(e.message, "meeting");
    }

    // --- Repetition tests ---

    #[test]
    fn parse_every_n_weeks() {
        let e = parse("14:55 20.05 every 2 weeks call office").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(14, 55, 0));
        assert!(e.date.is_some());
        let d = e.date.unwrap();
        assert_eq!(d.day(), 20);
        assert_eq!(d.month(), 5);
        let rep = e.repetition.unwrap();
        assert_eq!(rep.interval, 2);
        assert_eq!(rep.unit, TimeUnit::Weeks);
        assert_eq!(e.message, "call office");
    }

    #[test]
    fn parse_every_day() {
        let e = parse("9:00 every day standup").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(9, 0, 0));
        let rep = e.repetition.unwrap();
        assert_eq!(rep.interval, 1);
        assert_eq!(rep.unit, TimeUnit::Days);
        assert_eq!(e.message, "standup");
    }

    #[test]
    fn parse_every_month() {
        let e = parse("10:00 01.01 every 1 month pay rent").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(10, 0, 0));
        let rep = e.repetition.unwrap();
        assert_eq!(rep.interval, 1);
        assert_eq!(rep.unit, TimeUnit::Months);
        assert_eq!(e.message, "pay rent");
    }

    #[test]
    fn parse_every_without_number() {
        let e = parse("8:00 every hour check logs").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(8, 0, 0));
        let rep = e.repetition.unwrap();
        assert_eq!(rep.interval, 1);
        assert_eq!(rep.unit, TimeUnit::Hours);
        assert_eq!(e.message, "check logs");
    }

    #[test]
    fn parse_every_year() {
        let e = parse("12:00 01.01 every year happy new year").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(12, 0, 0));
        let rep = e.repetition.unwrap();
        assert_eq!(rep.interval, 1);
        assert_eq!(rep.unit, TimeUnit::Years);
        assert_eq!(e.message, "happy new year");
    }

    // --- Bare hour tests ---

    #[test]
    fn parse_bare_hour() {
        let e = parse("8 call Alex").unwrap();
        assert_eq!(e.bare_hour, Some(8));
        assert!(e.time.is_none());
        assert!(e.date.is_none());
        assert!(e.in_offset.is_none());
        assert_eq!(e.message, "call Alex");
    }

    #[test]
    fn parse_bare_hour_24() {
        let e = parse("24 call Poly").unwrap();
        assert_eq!(e.bare_hour, Some(24));
        assert_eq!(e.message, "call Poly");
    }

    #[test]
    fn parse_bare_hour_25_returns_none() {
        assert!(parse("25 call Alex").is_none());
    }

    #[test]
    fn parse_bare_hour_0() {
        let e = parse("0 call Alex").unwrap();
        assert_eq!(e.bare_hour, Some(0));
        assert_eq!(e.message, "call Alex");
    }

    #[test]
    fn parse_bare_hour_does_not_match_date() {
        let e = parse("8.11 birthday").unwrap();
        assert!(e.time.is_none());
        assert!(e.bare_hour.is_none());
        assert!(e.date.is_some());
        assert_eq!(e.date.unwrap().day(), 8);
        assert_eq!(e.date.unwrap().month(), 11);
    }

    #[test]
    fn parse_bare_hour_with_date() {
        let e = parse("8 26.11 birthday").unwrap();
        assert_eq!(e.bare_hour, Some(8));
        assert!(e.time.is_none());
        assert!(e.date.is_some());
        assert_eq!(e.date.unwrap().day(), 26);
        assert_eq!(e.date.unwrap().month(), 11);
        assert_eq!(e.message, "birthday");
    }

    // --- Relative offset tests ---

    #[test]
    fn parse_minutes_offset() {
        let e = parse("8 min call her").unwrap();
        assert_eq!(e.in_offset, Some((8, TimeUnit::Minutes)));
        assert!(e.time.is_none());
        assert!(e.date.is_none());
        assert_eq!(e.message, "call her");
    }

    #[test]
    fn parse_minutes_offset_with_repetition() {
        let e = parse("8 min every hour check server").unwrap();
        assert_eq!(e.in_offset, Some((8, TimeUnit::Minutes)));
        let rep = e.repetition.unwrap();
        assert_eq!(rep.interval, 1);
        assert_eq!(rep.unit, TimeUnit::Hours);
        assert_eq!(e.message, "check server");
    }

    #[test]
    fn parse_minutes_long_form() {
        let e = parse("15 minutes stretch").unwrap();
        assert_eq!(e.in_offset, Some((15, TimeUnit::Minutes)));
        assert_eq!(e.message, "stretch");
    }

    #[test]
    fn parse_minutes_singular() {
        let e = parse("1 minute reminder").unwrap();
        assert_eq!(e.in_offset, Some((1, TimeUnit::Minutes)));
        assert_eq!(e.message, "reminder");
    }

    #[test]
    fn parse_minutes_mins_form() {
        let e = parse("30 mins break").unwrap();
        assert_eq!(e.in_offset, Some((30, TimeUnit::Minutes)));
        assert_eq!(e.message, "break");
    }

    #[test]
    fn parse_hours_offset() {
        let e = parse("2 hours call her").unwrap();
        assert_eq!(e.in_offset, Some((2, TimeUnit::Hours)));
        assert_eq!(e.message, "call her");
    }

    #[test]
    fn parse_days_offset() {
        let e = parse("3 days check report").unwrap();
        assert_eq!(e.in_offset, Some((3, TimeUnit::Days)));
        assert_eq!(e.message, "check report");
    }

    // --- Monthly pattern tests ---

    #[test]
    fn parse_first_sunday() {
        let e = parse("10:00 first sunday call mom").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(10, 0, 0));
        assert_eq!(
            e.monthly_pattern,
            Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Sun))
        );
        assert!(e.days.is_none());
        assert_eq!(e.message, "call mom");
    }

    #[test]
    fn parse_last_monday() {
        let e = parse("9:30 last monday team sync").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(9, 30, 0));
        assert_eq!(
            e.monthly_pattern,
            Some(MonthlyPattern::OrdinalWeekday(Ordinal::Last, Weekday::Mon))
        );
        assert_eq!(e.message, "team sync");
    }

    #[test]
    fn parse_second_thursday() {
        let e = parse("14:00 second thursday board meeting").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(14, 0, 0));
        assert_eq!(
            e.monthly_pattern,
            Some(MonthlyPattern::OrdinalWeekday(
                Ordinal::Second,
                Weekday::Thu
            ))
        );
        assert_eq!(e.message, "board meeting");
    }

    #[test]
    fn parse_third_friday() {
        let e = parse("17:00 3rd friday happy hour").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(17, 0, 0));
        assert_eq!(
            e.monthly_pattern,
            Some(MonthlyPattern::OrdinalWeekday(Ordinal::Third, Weekday::Fri))
        );
        assert_eq!(e.message, "happy hour");
    }

    #[test]
    fn parse_fourth_wednesday() {
        let e = parse("11:00 4th wednesday review").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(11, 0, 0));
        assert_eq!(
            e.monthly_pattern,
            Some(MonthlyPattern::OrdinalWeekday(
                Ordinal::Fourth,
                Weekday::Wed
            ))
        );
        assert_eq!(e.message, "review");
    }

    #[test]
    fn parse_last_day_of_the_month() {
        let e = parse("18:00 last day of the month pay rent").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(18, 0, 0));
        assert_eq!(e.monthly_pattern, Some(MonthlyPattern::LastDay));
        assert_eq!(e.message, "pay rent");
    }

    #[test]
    fn parse_last_day_short() {
        let e = parse("18:00 last day pay bills").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(18, 0, 0));
        assert_eq!(e.monthly_pattern, Some(MonthlyPattern::LastDay));
        assert_eq!(e.message, "pay bills");
    }

    #[test]
    fn parse_monthly_pattern_case_insensitive() {
        let e = parse("8:00 First Saturday chores").unwrap();
        assert_eq!(
            e.monthly_pattern,
            Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Sat))
        );
        assert_eq!(e.message, "chores");
    }

    #[test]
    fn parse_monthly_pattern_no_time_defaults_valid() {
        let e = parse("first tuesday standup").unwrap();
        assert!(e.time.is_none());
        assert_eq!(
            e.monthly_pattern,
            Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Tue))
        );
        assert_eq!(e.message, "standup");
    }

    #[test]
    fn parse_first_day_ignored() {
        // "first day" is not supported (only "last day")
        // "first" won't match as monthly pattern for "day", so it falls through
        assert!(parse("first day something").is_none());
    }

    #[test]
    fn parse_monthly_with_repetition() {
        let e = parse("10:00 first sunday every month call mom").unwrap();
        assert_eq!(
            e.monthly_pattern,
            Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Sun))
        );
        let rep = e.repetition.unwrap();
        assert_eq!(rep.interval, 1);
        assert_eq!(rep.unit, TimeUnit::Months);
        assert_eq!(e.message, "call mom");
    }
}
