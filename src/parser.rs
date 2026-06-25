use chrono::{Datelike, Local, NaiveDate, NaiveTime, Weekday};
use regex::Regex;
use std::collections::HashSet;
use std::ops::Range;
use std::sync::LazyLock;

use crate::types::{
    EventInfo, MonthlyPattern, Ordinal, Repetition, TimeUnit, day_from_str, parse_days,
    unit_from_str,
};

// NOTE: the time regexes are intentionally *not* anchored to the start of the
// message. A clock time is matched wherever it appears (e.g. "call office at
// 5:30" extracts 5:30), unlike the relative offset, bare hour, and short date
// which must lead the message. 12h is tried before 24h so "5:24 PM" is not
// partially consumed as "5:24". Minutes accept 1-2 digits so "10:6" means
// "10:06" ("9:5 PM" -> 21:05).
static RE_TIME_12H: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\d{1,2}):(\d{1,2})\s*(AM|PM)").unwrap());

static RE_TIME_24H: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{1,2}):(\d{1,2})").unwrap());

static RE_DATE_FULL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2})\.(\d{1,2})\.(\d{4})").unwrap());

static RE_DATE_SHORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2})\.(\d{1,2})(?:[^\.\d]|$)").unwrap());

static RE_EVERY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bevery\s+(?:(\d+)\s+)?(min(?:ute)?s?|hours?|days?|weeks?|months?|years?)\b")
        .unwrap()
});

// Standalone "yearly" token, absorbed on short dates (the canonical suffix
// emitted by `EventInfo::normalize_time` for a yearly short-date event).
static RE_YEARLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\byearly\b").unwrap());

// An optional leading "in" is absorbed so "in 8 min" is identical to "8 min"
// (and matches the canonical form emitted by `EventInfo::normalize_time`).
static RE_IN_OFFSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:in\s+)?(\d+)\s+(min(?:ute)?s?|hours?|days?|weeks?|months?|years?)\b")
        .unwrap()
});

static RE_BARE_HOUR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,2})\s").unwrap());

// An optional leading "every" is absorbed so "every fri" is treated exactly
// like "fri" (a weekly recurrence on that weekday set), consuming the word so it
// does not leak into the message or get misread by RE_EVERY.
static RE_DAYS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:every\s+)?((?:mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?)(?:\s*[-,]\s*(?:mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?))*)\b").unwrap()
});

// Fixed day of the month: "28 of the month", "28th of the month",
// "28th day of the month", "every 28 of the month", "each 5 of the month". The
// literal "of [the] month" is required so it never collides with the bare-hour
// format (extraction also runs before bare hour). An optional leading
// "every"/"each" is absorbed; an optional ordinal suffix and an optional literal
// "day" before "of" are accepted (the canonical form is "each <N><ord> day of
// the month").
static RE_DAY_OF_MONTH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:every\s+|each\s+)?(\d{1,2})(?:st|nd|rd|th)?\s+(?:day\s+)?of\s+(?:the\s+)?month\b",
    )
    .unwrap()
});

static RE_MONTHLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(first|1st|second|2nd|third|3rd|fourth|4th|fifth|5th|last)\s+(mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?|day)(?:\s+of\s+the\s+month)?\b").unwrap()
});

// Matches any standalone 4-digit token (or comma list) anywhere in the message;
// only values in 2000..=2100 are kept (see `parse`). This is greedy by design —
// "buy 2025 tickets" is treated as a year restriction.
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

/// The working message text plus the byte ranges of the *original* input that
/// survive token extraction, kept in lockstep so callers can map message
/// entities (whose offsets reference the original text) onto the leftover body.
///
/// `spans` are original-input byte ranges in order; their concatenation always
/// equals `text`. Every parser deletion is a single contiguous removal, applied
/// via [`Remaining::delete`].
struct Remaining {
    text: String,
    spans: Vec<Range<usize>>,
}

impl Remaining {
    fn new(input: &str) -> Self {
        let spans = if input.is_empty() {
            Vec::new()
        } else {
            // One span covering the whole input (not a Vec of its indices).
            std::iter::once(0..input.len()).collect()
        };
        Self {
            text: input.to_string(),
            spans,
        }
    }

    /// Removes `cur` (a byte range in the *current* `text`) from both `text` and
    /// the surviving-span map.
    fn delete(&mut self, cur: Range<usize>) {
        self.text = format!("{}{}", &self.text[..cur.start], &self.text[cur.end..]);

        let mut new_spans: Vec<Range<usize>> = Vec::new();
        let mut pos = 0usize; // current-coord offset at the start of each span
        for span in &self.spans {
            let len = span.end - span.start;
            let span_start = pos; // this span's range in current coords
            let span_end = pos + len;

            // Keep the portion before the deletion.
            if span_start < cur.start {
                let keep_end = cur.start.min(span_end);
                new_spans.push(span.start..span.start + (keep_end - span_start));
            }
            // Keep the portion after the deletion.
            if span_end > cur.end {
                let keep_start = cur.end.max(span_start);
                new_spans.push(span.start + (keep_start - span_start)..span.end);
            }
            pos = span_end;
        }
        new_spans.retain(|s| s.start < s.end);
        self.spans = new_spans;
    }
}

/// Parses a natural-language reminder from `input`.
///
/// Extracts the time/date components (see module regexes) and returns an
/// [`EventInfo`] whose `message` is the leftover text. Returns `None` when no
/// time component is found or nothing is left for the message body. The
/// DB-tracking fields are left at their defaults (zero/false/None).
pub fn parse(input: &str) -> Option<EventInfo> {
    parse_full(input).map(|(event, _)| event)
}

/// Like [`parse`] but also returns the byte ranges of `input` that compose the
/// message body (before whitespace normalization). Their concatenation equals
/// the pre-normalized leftover text; `crate::richtext` uses them to re-map the
/// message's formatting entities onto the surviving body.
///
/// Returns `None` when no time component is found or when the message body is
/// empty (a time was given but no reminder text). To detect the latter case on
/// its own, use [`parse_time_only`].
pub fn parse_full(input: &str) -> Option<(EventInfo, Vec<Range<usize>>)> {
    let (event, spans) = parse_components(input)?;
    if event.message.is_empty() {
        return None;
    }
    Some((event, spans))
}

/// Returns the parsed event when `input` carries a recognizable time component
/// but **no** message body (e.g. `13:30`, `in 8 min`, `mon-fri 9:00`). Returns
/// `None` when there is no time component or when a body was supplied (in which
/// case [`parse_full`] handles it). Used by the interactive "send me the reminder
/// text" flow.
pub fn parse_time_only(input: &str) -> Option<EventInfo> {
    let (event, _) = parse_components(input)?;
    if event.message.is_empty() {
        Some(event)
    } else {
        None
    }
}

/// Extracts the time/date components and leftover message body. Returns `None`
/// only when no time component is found; the returned `message` may be empty (the
/// callers [`parse_full`] / [`parse_time_only`] decide how to treat that).
fn parse_components(input: &str) -> Option<(EventInfo, Vec<Range<usize>>)> {
    let mut rem = Remaining::new(input);
    let mut time: Option<NaiveTime> = None;
    let mut date: Option<NaiveDate> = None;
    let mut year_explicit = false;
    let mut in_offset: Option<(u32, TimeUnit)> = None;
    let mut bare_hour: Option<u32> = None;
    let mut days: Option<HashSet<Weekday>> = None;
    let mut years: Option<HashSet<i32>> = None;
    let mut monthly_pattern: Option<MonthlyPattern> = None;

    // Relative offset: "N unit" e.g. "8 min call her", "2 hours reminder" (checked first)
    if let Some(caps) = RE_IN_OFFSET.captures(&rem.text)
        && let Ok(n) = caps[1].parse::<u32>()
        && let Some(unit) = unit_from_str(&caps[2])
    {
        let end = caps.get(0).unwrap().end();
        in_offset = Some((n, unit));
        rem.delete(0..end);
    }

    if in_offset.is_none() {
        // 12h time (must be checked before 24h to avoid partial match)
        if let Some(caps) = RE_TIME_12H.captures(&rem.text) {
            let mut hour: u32 = caps[1].parse().ok()?;
            let minute: u32 = caps[2].parse().ok()?;
            let ampm = caps[3].to_ascii_uppercase();
            let m = caps.get(0).unwrap();
            let (start, end) = (m.start(), m.end());

            if hour > 12 || minute >= 60 || hour == 0 {
                return None;
            }

            if ampm == "PM" && hour != 12 {
                hour += 12;
            } else if ampm == "AM" && hour == 12 {
                hour = 0;
            }

            time = Some(NaiveTime::from_hms_opt(hour, minute, 0)?);
            rem.delete(start..end);
        } else if let Some(caps) = RE_TIME_24H.captures(&rem.text) {
            let hour: u32 = caps[1].parse().ok()?;
            let minute: u32 = caps[2].parse().ok()?;
            let m = caps.get(0).unwrap();
            let (start, end) = (m.start(), m.end());

            time = Some(NaiveTime::from_hms_opt(hour, minute, 0)?);
            rem.delete(start..end);
        }

        // Day of the month: "every 28 of the month", "28th of the month".
        // Checked before the bare hour so "<N> of the month" always wins the
        // overlap (e.g. "5 of the month" is day-5, not hour-5).
        if let Some(caps) = RE_DAY_OF_MONTH.captures(&rem.text)
            && let Ok(day) = caps[1].parse::<u32>()
            && (1..=31).contains(&day)
        {
            let m = caps.get(0).unwrap();
            let (start, end) = (m.start(), m.end());
            monthly_pattern = Some(MonthlyPattern::DayOfMonth(day));
            rem.delete(start..end);
        }

        // Bare hour: "8 call Alex" -> bare_hour=8, "0 call Sacha" -> bare_hour=0
        if time.is_none()
            && monthly_pattern.is_none()
            && let Some(caps) = RE_BARE_HOUR.captures(&rem.text)
            && let Ok(n) = caps[1].parse::<u32>()
            && n <= 24
        {
            let end = caps.get(0).unwrap().end();
            bare_hour = Some(n);
            rem.delete(0..end);
        }

        // Full date (must be checked before short date)
        if let Some(caps) = RE_DATE_FULL.captures(&rem.text) {
            let day: u32 = caps[1].parse().ok()?;
            let month: u32 = caps[2].parse().ok()?;
            let year: i32 = caps[3].parse().ok()?;
            let m = caps.get(0).unwrap();
            let (start, end) = (m.start(), m.end());

            date = Some(NaiveDate::from_ymd_opt(year, month, day)?);
            year_explicit = true;
            rem.delete(start..end);
        } else if let Some(caps) = RE_DATE_SHORT.captures(&rem.text) {
            let day: u32 = caps[1].parse().ok()?;
            let month: u32 = caps[2].parse().ok()?;
            let year = Local::now().year();
            let m = caps.get(0).unwrap();
            let (start, end) = (m.start(), m.end());

            date = Some(NaiveDate::from_ymd_opt(year, month, day)?);
            rem.delete(start..end);
        }

        // Years: "2027", "2027,2028" — only when no full date was already parsed
        if date.is_none()
            && let Some(m) = RE_YEARS.find(&rem.text)
        {
            let (mstart, mend) = (m.start(), m.end());
            let year_set: HashSet<i32> = m
                .as_str()
                .split(',')
                .filter_map(|s| s.trim().parse::<i32>().ok())
                .filter(|&y| (2000..=2100).contains(&y))
                .collect();
            if !year_set.is_empty() {
                years = Some(year_set);
                rem.delete(mstart..mend);
            }
        }

        // Monthly pattern: "first sunday", "last monday", "last day of the month"
        if monthly_pattern.is_none()
            && let Some(caps) = RE_MONTHLY.captures(&rem.text)
            && let Some(ord) = ordinal_from_str(&caps[1])
        {
            let target = caps[2].to_ascii_lowercase();
            let m = caps.get(0).unwrap();
            let (start, end) = (m.start(), m.end());
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
                rem.delete(start..end);
            }
        }

        // Days of week (skip if monthly pattern already matched)
        if monthly_pattern.is_none()
            && let Some(caps) = RE_DAYS.captures(&rem.text)
            && let Some(parsed) = parse_days(&caps[1])
        {
            let m = caps.get(0).unwrap();
            let (start, end) = (m.start(), m.end());
            days = Some(parsed);
            rem.delete(start..end);
        }
    }

    // Repetition: "every N unit" or "every unit" (checked for both offset and time modes)
    let mut repetition: Option<Repetition> = None;
    if let Some(caps) = RE_EVERY.captures(&rem.text) {
        let interval: u32 = caps
            .get(1)
            .map(|m| m.as_str().parse().unwrap_or(1))
            .unwrap_or(1);
        if let Some(unit) = unit_from_str(&caps[2]) {
            let m = caps.get(0).unwrap();
            let (start, end) = (m.start(), m.end());
            repetition = Some(Repetition { interval, unit });
            rem.delete(start..end);
        }
    }

    // A short date (day.month, no year) is inherently a yearly event. The date
    // itself drives the yearly wrap in `scheduler`, so absorb a redundant
    // explicit "every year"/"yearly" and drop any Years repetition. An explicit
    // year keeps its repetition.
    if date.is_some() && !year_explicit {
        if matches!(&repetition, Some(r) if r.unit == TimeUnit::Years) {
            repetition = None;
        }
        if let Some(m) = RE_YEARLY.find(&rem.text) {
            rem.delete(m.start()..m.end());
        }
    }

    // Derive the plain message from the same normalization `richtext` uses for
    // the HTML fragment (single source of truth): horizontal whitespace within a
    // line collapses to single spaces, line breaks are preserved verbatim. The
    // surviving spans concatenate to `rem.text`, so this processes the same
    // characters.
    let (message, _) = crate::richtext::normalize(input, &rem.spans);

    if time.is_none()
        && date.is_none()
        && in_offset.is_none()
        && bare_hour.is_none()
        && monthly_pattern.is_none()
    {
        return None;
    }

    let event = EventInfo {
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
        legacy: false,
        snoozed: false,
    };
    Some((event, rem.spans))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    /// Concatenates the surviving spans back out of the original input.
    fn concat_spans(input: &str, spans: &[Range<usize>]) -> String {
        spans.iter().map(|s| &input[s.clone()]).collect()
    }

    #[test]
    fn parse_time_only_detects_body_less_inputs() {
        // A time with no body: parse_full rejects it, parse_time_only accepts it.
        for input in ["13:30", "5:24 PM", "8 min", "9:00 mon-fri"] {
            assert!(
                parse_full(input).is_none(),
                "parse_full should reject body-less {input:?}"
            );
            let event = parse_time_only(input)
                .unwrap_or_else(|| panic!("parse_time_only should accept {input:?}"));
            assert!(event.message.is_empty());
        }
    }

    #[test]
    fn parse_time_only_rejects_bodies_and_unparsable() {
        // A body was supplied -> parse_full's job, not parse_time_only's.
        assert!(parse_time_only("13:30 lunch").is_none());
        // No time component at all.
        assert!(parse_time_only("hello").is_none());
    }

    #[test]
    fn parse_full_spans_concatenate_to_leftover() {
        // Leading time prefix removed.
        let input = "13:23 lunch meeting";
        let (e, spans) = parse_full(input).unwrap();
        let leftover = concat_spans(input, &spans);
        assert_eq!(leftover, " lunch meeting");
        assert_eq!(
            leftover.split_whitespace().collect::<Vec<_>>().join(" "),
            e.message
        );
    }

    #[test]
    fn parse_full_spans_handle_mid_text_time_removal() {
        // A clock time is matched anywhere, so the removed range is in the
        // middle/end of the body; the surviving spans skip exactly that range.
        let input = "call office at 17:00";
        let (e, spans) = parse_full(input).unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(17, 0, 0));
        let leftover = concat_spans(input, &spans);
        assert_eq!(leftover, "call office at ");
        assert_eq!(
            leftover.split_whitespace().collect::<Vec<_>>().join(" "),
            e.message
        );
        assert_eq!(e.message, "call office at");
    }

    #[test]
    fn parse_full_spans_handle_multiple_removals() {
        // Time + short date + weekday all stripped; spans still reconstruct the
        // exact leftover and normalize to the message.
        let input = "9:00 AM 15.03 sun,sat weekend task";
        let (e, spans) = parse_full(input).unwrap();
        let leftover = concat_spans(input, &spans);
        assert_eq!(
            leftover.split_whitespace().collect::<Vec<_>>().join(" "),
            e.message
        );
        assert_eq!(e.message, "weekend task");
    }

    #[test]
    fn parse_24h_time_with_message() {
        let e = parse("13:23 lunch meeting").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(13, 23, 0));
        assert!(e.date.is_none());
        assert_eq!(e.message, "lunch meeting");
    }

    #[test]
    fn parse_24h_single_digit_minute() {
        // "10:6" is shorthand for "10:06"
        let e = parse("10:6 standup").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(10, 6, 0));
        assert_eq!(e.message, "standup");
    }

    #[test]
    fn parse_12h_single_digit_minute() {
        let e = parse("9:5 PM call back").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(21, 5, 0));
        assert_eq!(e.message, "call back");
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
    fn parse_preserves_newline_in_message() {
        let e = parse("13:30 line one\nline two").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(13, 30, 0));
        assert_eq!(e.message, "line one\nline two");
    }

    #[test]
    fn parse_preserves_blank_line_in_message() {
        // Horizontal whitespace within each line collapses, but the blank line
        // (two newlines) survives verbatim.
        let e = parse("9:00 buy   milk\n\ncall  mom").unwrap();
        assert_eq!(e.message, "buy milk\n\ncall mom");
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

    #[test]
    fn parse_every_weekday_same_as_weekday() {
        // "every fri" is treated exactly like "fri" — a weekly recurrence on
        // that weekday, with "every" consumed (not left in the message).
        let e = parse("10:30 every fri release day").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(10, 30, 0));
        let expected_days: HashSet<Weekday> = [Weekday::Fri].into_iter().collect();
        assert_eq!(e.days, Some(expected_days));
        assert!(e.repetition.is_none());
        assert_eq!(e.message, "release day");
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
    fn parse_every_n_min_abbreviated() {
        let e = parse("19:30 every 7 min call Peter").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(19, 30, 0));
        let rep = e.repetition.unwrap();
        assert_eq!(rep.interval, 7);
        assert_eq!(rep.unit, TimeUnit::Minutes);
        assert_eq!(e.message, "call Peter");
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
        // "every year" repetition without a date (e.g. "1:06 every year ...").
        let e = parse("1:06 every year happy new year").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(1, 6, 0));
        assert!(e.date.is_none());
        let rep = e.repetition.as_ref().unwrap();
        assert_eq!(rep.interval, 1);
        assert_eq!(rep.unit, TimeUnit::Years);
        assert_eq!(e.message, "happy new year");
        assert_eq!(e.normalize_time(), "01:06 every year");
    }

    #[test]
    fn parse_short_date_absorbs_redundant_every_year() {
        // A short date is inherently yearly, so an explicit "every year"/"yearly"
        // is redundant: the repetition is dropped and the word absorbed.
        for input in [
            "12:00 01.01 every year happy new year",
            "12:00 01.01 yearly happy new year",
            "12:00 01.01 happy new year",
        ] {
            let e = parse(input).unwrap();
            assert_eq!(e.time, NaiveTime::from_hms_opt(12, 0, 0));
            assert_eq!(e.date, NaiveDate::from_ymd_opt(Local::now().year(), 1, 1));
            assert!(!e.year_explicit);
            assert!(e.repetition.is_none(), "{input}");
            assert_eq!(e.message, "happy new year");
            assert_eq!(e.normalize_time(), "12:00 01.01 yearly");
        }
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
    fn parse_minutes_offset_with_leading_in() {
        // An optional leading "in" is absorbed (matches the normalize_time form).
        let e = parse("in 8 min call her").unwrap();
        assert_eq!(e.in_offset, Some((8, TimeUnit::Minutes)));
        assert_eq!(e.message, "call her");
        let e = parse("in 8 min every 2 hours test").unwrap();
        assert_eq!(e.in_offset, Some((8, TimeUnit::Minutes)));
        let rep = e.repetition.unwrap();
        assert_eq!(rep.interval, 2);
        assert_eq!(rep.unit, TimeUnit::Hours);
        assert_eq!(e.message, "test");
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
    fn parse_day_of_month_every_prefix() {
        let e = parse("12:05 every 28 of the month pay rent").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(12, 5, 0));
        assert_eq!(e.monthly_pattern, Some(MonthlyPattern::DayOfMonth(28)));
        assert!(e.days.is_none());
        assert!(e.bare_hour.is_none());
        assert!(e.repetition.is_none());
        assert_eq!(e.message, "pay rent");
    }

    #[test]
    fn parse_day_of_month_ordinal_suffix() {
        let e = parse("9:00 28th of the month check").unwrap();
        assert_eq!(e.monthly_pattern, Some(MonthlyPattern::DayOfMonth(28)));
        assert_eq!(e.message, "check");
    }

    #[test]
    fn parse_day_of_month_day_word() {
        // The literal "day" before "of the month" is accepted.
        let e = parse("9:00 each 28th day of the month check").unwrap();
        assert_eq!(e.monthly_pattern, Some(MonthlyPattern::DayOfMonth(28)));
        assert_eq!(e.message, "check");
    }

    #[test]
    fn parse_day_of_month_each_prefix() {
        let e = parse("8:00 each 5 of the month water plants").unwrap();
        assert_eq!(e.monthly_pattern, Some(MonthlyPattern::DayOfMonth(5)));
        assert_eq!(e.message, "water plants");
    }

    #[test]
    fn parse_day_of_month_beats_bare_hour() {
        // "5 of the month" is day-5, not bare-hour-5.
        let e = parse("5 of the month rent").unwrap();
        assert_eq!(e.monthly_pattern, Some(MonthlyPattern::DayOfMonth(5)));
        assert!(e.bare_hour.is_none());
        assert!(e.time.is_none());
        assert_eq!(e.message, "rent");
    }

    #[test]
    fn parse_day_of_month_with_repetition() {
        // The user's example: day-of-month anchor combined with a repeat interval.
        let e = parse("22:15 every 28 of the month every 2 day call Mal").unwrap();
        assert_eq!(e.time, NaiveTime::from_hms_opt(22, 15, 0));
        assert_eq!(e.monthly_pattern, Some(MonthlyPattern::DayOfMonth(28)));
        let rep = e.repetition.unwrap();
        assert_eq!(rep.interval, 2);
        assert_eq!(rep.unit, TimeUnit::Days);
        assert_eq!(e.message, "call Mal");
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

    // --- normalize_time round-trip tests ---

    #[test]
    fn normalize_time_round_trips() {
        // Each canonical string, when re-parsed (with a trailing message), must
        // produce an event whose `normalize_time()` is byte-identical — the
        // idempotency guarantee of the canonical form.
        for s in [
            "12:02",
            "01:25",
            "17:24",
            "08:00",
            "00:00",
            "in 8 minutes",
            "in 1 minute",
            "in 3 hours",
            "in 8 minutes every hour",
            "in 20 hours every 2 weeks",
            "11:26 12.10.2026",
            "11:26 12.10.2026 every 2 weeks",
            "01:23 26.11 yearly",
            "10:03 15.12.2027 every year",
            "10:25 Mon-Fri",
            "01:25 Mon-Sat",
            "13:25 Mon-Wed,Fri",
            "13:25 2027 Fri,Sun",
            "13:25 2027,2028 Fri,Sun",
            "10:00 first Sunday",
            "17:00 third Friday",
            "18:00 last day of the month",
            "22:15 each 28th day of the month",
            "22:15 each 28th day of the month every 2 days",
            "10:00 first Friday every 10 days",
            "15:30 every 3 days",
            "01:34 every year",
            "21:00 every 2 days",
        ] {
            let event = parse(&format!("{s} reminder"))
                .unwrap_or_else(|| panic!("canonical string {s:?} should parse"));
            assert_eq!(
                event.normalize_time(),
                s,
                "normalize_time should round-trip {s:?}"
            );
        }
    }
}
