//! Converts legacy MateBot `.alert` files into [`EventInfo`] records.
//!
//! Two physical shapes exist: plain text (just the user input) and JSON
//! `{"input": ..., "lastActivePeriodTime": <ms epoch>?}`. The file name
//! (`YYYYMMDD_HHMMSS_mmm.alert`) is the creation datetime. The old grammar is
//! `HH:MM [date|weekdays] [message]` with an optional `/N` period on any
//! component (see `OLD-SPEC.md`). This module is pure and unit-tested; the
//! Telegram/zip plumbing lives in [`crate::import`].

use std::collections::HashSet;

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};
use regex::Regex;

use crate::scheduler;
use crate::types::{EventInfo, Repetition, TimeUnit};

/// Outcome of converting a single legacy alert.
pub struct Converted {
    pub event: EventInfo,
    pub status: Status,
    /// Short human description of the extracted fields, for the report.
    pub summary: String,
    /// `true` when a stale `lastActivePeriodTime` was rolled forward using the
    /// input's period. Surfaced as a red note in the report.
    pub recalculated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Parsed and scheduled into the future (active).
    Scheduled,
    /// Parsed but out of date — stored inactive.
    Inactive,
    /// Could not be parsed as the old grammar — stored inactive, raw text kept.
    Unparsed,
}

impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Status::Scheduled => "scheduled",
            Status::Inactive => "inactive (out of date)",
            Status::Unparsed => "unparsed",
        }
    }
}

/// Parses `YYYYMMDD_HHMMSS_mmm.alert` (path or basename) into its creation datetime.
pub fn created_at_from_filename(name: &str) -> Option<NaiveDateTime> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let re = Regex::new(r"^(\d{4})(\d{2})(\d{2})_(\d{2})(\d{2})(\d{2})_(\d{3})").ok()?;
    let c = re.captures(base)?;
    let year: i32 = c[1].parse().ok()?;
    let month: u32 = c[2].parse().ok()?;
    let day: u32 = c[3].parse().ok()?;
    let hour: u32 = c[4].parse().ok()?;
    let min: u32 = c[5].parse().ok()?;
    let sec: u32 = c[6].parse().ok()?;
    let milli: u32 = c[7].parse().ok()?;
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let time = NaiveTime::from_hms_milli_opt(hour, min, sec, milli)?;
    Some(date.and_time(time))
}

/// Reads a file's contents into `(input, last_active_period_ms?)`.
///
/// Handles both plain-text files and JSON `{"input", "lastActivePeriodTime"?}`.
pub fn extract_input(contents: &str) -> (String, Option<i64>) {
    let trimmed = contents.trim();
    if trimmed.starts_with('{')
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(input) = value.get("input").and_then(|v| v.as_str())
    {
        let last = value.get("lastActivePeriodTime").and_then(|v| v.as_i64());
        return (input.to_string(), last);
    }
    (trimmed.to_string(), None)
}

/// Structured pieces extracted from a legacy input line.
struct LegacyParse {
    time: NaiveTime,
    days: Option<HashSet<Weekday>>,
    day: Option<u32>,
    month: Option<u32>,
    year: Option<i32>,
    repetition: Option<Repetition>,
    message: String,
}

fn iso_to_weekday(n: u32) -> Option<Weekday> {
    match n {
        1 => Some(Weekday::Mon),
        2 => Some(Weekday::Tue),
        3 => Some(Weekday::Wed),
        4 => Some(Weekday::Thu),
        5 => Some(Weekday::Fri),
        6 => Some(Weekday::Sat),
        7 => Some(Weekday::Sun),
        _ => None,
    }
}

/// Parses an ISO weekday set (`6`, `1-5`, `1,3,5`, `5-7`). Returns `None` when the
/// token contains anything other than digits/`-`/`,` or an invalid/descending range.
fn parse_iso_days(s: &str) -> Option<HashSet<Weekday>> {
    if s.is_empty()
        || !s
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == ',')
    {
        return None;
    }
    let mut set = HashSet::new();
    for token in s.split(',') {
        if let Some((a, b)) = token.split_once('-') {
            let sa: u32 = a.parse().ok()?;
            let sb: u32 = b.parse().ok()?;
            if sa > sb {
                return None;
            }
            for n in sa..=sb {
                set.insert(iso_to_weekday(n)?);
            }
        } else {
            set.insert(iso_to_weekday(token.parse().ok()?)?);
        }
    }
    if set.is_empty() { None } else { Some(set) }
}

/// Splits a `value` or `value/period` spec into `(value, period?)`.
fn split_period(spec: &str) -> Option<(u32, Option<u32>)> {
    match spec.split_once('/') {
        Some((v, p)) => Some((v.parse().ok()?, Some(p.parse().ok()?))),
        None => Some((spec.parse().ok()?, None)),
    }
}

fn parse_legacy(input: &str) -> Option<LegacyParse> {
    let time_re = Regex::new(r"^\s*(\d{1,2})(?:/(\d+))?:(\d{1,2})(?:/(\d+))?").ok()?;
    let m = time_re.captures(input)?;
    let hour: u32 = m[1].parse().ok()?;
    let minute: u32 = m[3].parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    let time = NaiveTime::from_hms_opt(hour, minute, 0)?;
    let hour_period = m.get(2).and_then(|x| x.as_str().parse::<u32>().ok());
    let minute_period = m.get(4).and_then(|x| x.as_str().parse::<u32>().ok());
    let consumed = m.get(0).unwrap().end();

    let rest = input[consumed..].trim_start();
    let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let token = &rest[..token_end];

    let mut days = None;
    let mut day = None;
    let mut month = None;
    let mut year = None;
    let mut day_period = None;
    let mut month_period = None;
    let mut year_period = None;
    let mut message = rest;

    if let Some(set) = parse_iso_days(token) {
        days = Some(set);
        message = rest[token_end..].trim_start();
    } else if token.contains(':') {
        // Date spec: day[:month[:year]], each with an optional /period.
        let segs: Vec<&str> = token.split(':').collect();
        if let Some((d, p)) = segs.first().and_then(|s| split_period(s)) {
            day = Some(d);
            day_period = p;
            if let Some((mv, mp)) = segs
                .get(1)
                .filter(|s| !s.is_empty())
                .and_then(|s| split_period(s))
            {
                month = Some(mv);
                month_period = mp;
            }
            if let Some(yspec) = segs.get(2).filter(|s| !s.is_empty())
                && let Some((y, yp)) = split_period(yspec)
            {
                year = Some(y as i32);
                year_period = yp;
            }
            message = rest[token_end..].trim_start();
        }
    }

    // The old grammar allows only one period; take the first present, in
    // hour→minute→day→month→year order.
    let repetition = [
        (hour_period, TimeUnit::Hours),
        (minute_period, TimeUnit::Minutes),
        (day_period, TimeUnit::Days),
        (month_period, TimeUnit::Months),
        (year_period, TimeUnit::Years),
    ]
    .into_iter()
    .find_map(|(p, unit)| p.map(|interval| Repetition { interval, unit }));

    Some(LegacyParse {
        time,
        days,
        day,
        month,
        year,
        repetition,
        message: message.trim().to_string(),
    })
}

/// First date on or after `start` whose day-of-month equals `day`.
fn next_day_on_or_after(start: NaiveDate, day: u32) -> Option<NaiveDate> {
    let mut y = start.year();
    let mut m = start.month();
    for _ in 0..48 {
        if let Some(d) = NaiveDate::from_ymd_opt(y, m, day)
            && d >= start
        {
            return Some(d);
        }
        if m == 12 {
            y += 1;
            m = 1;
        } else {
            m += 1;
        }
    }
    None
}

fn ms_to_naive_local(ms: i64) -> Option<NaiveDateTime> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.naive_local())
}

fn base_event(chat_id: i64, created_at: NaiveDateTime, message: String) -> EventInfo {
    EventInfo {
        date: None,
        time: None,
        year_explicit: false,
        days: None,
        years: None,
        repetition: None,
        in_offset: None,
        bare_hour: None,
        monthly_pattern: None,
        message,
        id: 0,
        chat_id,
        active: false,
        next_datetime: None,
        created_at,
        msg_id: 0,
        legacy: true,
        snoozed: false,
    }
}

/// Builds an [`EventInfo`] from a single legacy alert.
///
/// `last_active_ms` (from `lastActivePeriodTime`) is authoritative for periodic
/// alerts: when present it is used directly as `next_datetime` and the scheduler
/// is *not* run; an event whose stored activation is already past is inactive.
/// Otherwise the next occurrence is computed with [`scheduler::calc_next_at`].
pub fn convert(
    input: &str,
    created_at: NaiveDateTime,
    last_active_ms: Option<i64>,
    chat_id: i64,
    now: NaiveDateTime,
) -> Converted {
    let parse = parse_legacy(input);
    let message = parse
        .as_ref()
        .map(|p| p.message.clone())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| input.trim().to_string());

    let mut event = base_event(chat_id, created_at, message);

    if let Some(p) = &parse {
        event.time = Some(p.time);
        event.repetition = p.repetition.clone();

        if let Some(days) = &p.days {
            event.days = Some(days.clone());
        } else if let (Some(d), Some(mo), Some(y)) = (p.day, p.month, p.year) {
            // Fully-qualified one-off date.
            event.date = NaiveDate::from_ymd_opt(y, mo, d);
            event.year_explicit = true;
        } else if let (Some(d), Some(mo)) = (p.day, p.month) {
            // Day + month: recurs yearly. Stamp the current year so the scheduler
            // (which only wraps a short date forward by one year) lands on the next
            // occurrence rather than the creation year.
            event.date = NaiveDate::from_ymd_opt(now.year(), mo, d);
        } else if let Some(d) = p.day {
            // Day-of-month only: one-shot on its next occurrence from creation.
            event.date = next_day_on_or_after(created_at.date(), d);
            event.year_explicit = true;
        }
    }

    // Schedule. Periodic alerts carry a stored next activation: use it directly
    // when it is still in the future. When it is stale, roll it forward using the
    // input's period (treating the stale activation as the previous fire), and
    // flag the recalculation for the report.
    let mut recalculated = false;
    if let Some(ms) = last_active_ms {
        match ms_to_naive_local(ms) {
            Some(dt) if dt > now => {
                event.next_datetime = Some(dt);
                event.active = true;
            }
            Some(stale) => {
                event.next_datetime = Some(stale);
                let scheduled = scheduler::calc_next_at(event.clone(), now);
                event.next_datetime = scheduled.next_datetime;
                event.active = scheduled.active;
                recalculated = event.active;
            }
            None => event.active = false,
        }
    } else if parse.is_some() {
        let scheduled = scheduler::calc_next_at(event.clone(), now);
        event.next_datetime = scheduled.next_datetime;
        event.active = scheduled.active;
    }

    let status = if parse.is_none() {
        Status::Unparsed
    } else if event.active {
        Status::Scheduled
    } else {
        Status::Inactive
    };

    let summary = summarize(&event);
    Converted {
        event,
        status,
        summary,
        recalculated,
    }
}

/// Renders the extracted fields of an event into a compact, report-friendly string.
fn summarize(event: &EventInfo) -> String {
    let mut parts = Vec::new();
    if let Some(t) = event.time {
        parts.push(format!("time={}", t.format("%H:%M")));
    }
    if let Some(d) = event.date {
        parts.push(format!(
            "date={}{}",
            d.format("%Y-%m-%d"),
            if event.year_explicit {
                " (exact)"
            } else {
                " (yearly)"
            }
        ));
    }
    if let Some(days) = &event.days {
        let order = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ];
        let names: Vec<&str> = order
            .iter()
            .filter(|d| days.contains(d))
            .map(|d| crate::types::day_to_str(*d))
            .collect();
        parts.push(format!("days={}", names.join(",")));
    }
    if let Some(rep) = &event.repetition {
        parts.push(format!("repeat=every {} {:?}", rep.interval, rep.unit));
    }
    if parts.is_empty() {
        "—".to_string()
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> NaiveDateTime {
        // Fixed "today" matching the project's reference date.
        NaiveDate::from_ymd_opt(2026, 6, 16)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
    }

    #[test]
    fn filename_parses_to_created_at() {
        let dt = created_at_from_filename("alerts/20170925_124839_126.alert").unwrap();
        assert_eq!(dt.date(), NaiveDate::from_ymd_opt(2017, 9, 25).unwrap());
        assert_eq!(dt.time().format("%H:%M:%S").to_string(), "12:48:39");
        // Basename without directory works too.
        assert!(created_at_from_filename("20180131_143625_667.alert").is_some());
        assert!(created_at_from_filename("not-an-alert.txt").is_none());
    }

    #[test]
    fn extract_input_plain_and_json() {
        assert_eq!(
            extract_input("10:00 13:03 Pi day"),
            ("10:00 13:03 Pi day".to_string(), None)
        );
        assert_eq!(
            extract_input("{\n  \"input\": \"10:00 28:01: Birthday\"\n}"),
            ("10:00 28:01: Birthday".to_string(), None)
        );
        let (input, last) = extract_input(
            "{\"input\": \"22:15 28/1: rent\", \"lastActivePeriodTime\": 1782764100000}",
        );
        assert_eq!(input, "22:15 28/1: rent");
        assert_eq!(last, Some(1782764100000));
    }

    #[test]
    fn birthday_short_date_recurs_yearly_and_is_active() {
        // Created 2017, day+month only -> next 2026-09-26.
        let created = created_at_from_filename("20170925_124839_126.alert").unwrap();
        let c = convert("10:00 26:09 birthday", created, None, 42, now());
        assert_eq!(c.status, Status::Scheduled);
        let next = c.event.next_datetime.unwrap();
        assert_eq!(next.date(), NaiveDate::from_ymd_opt(2026, 9, 26).unwrap());
        assert_eq!(next.time(), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
        assert!(!c.event.year_explicit);
        assert!(c.event.legacy);
    }

    #[test]
    fn full_date_in_past_is_inactive() {
        let created = created_at_from_filename("20260407_131907_140.alert").unwrap();
        let c = convert("08:22 09:04:2026 take laptop", created, None, 42, now());
        assert_eq!(c.status, Status::Inactive);
        assert!(!c.event.active);
        assert!(c.event.year_explicit);
        assert_eq!(c.event.date, NaiveDate::from_ymd_opt(2026, 4, 9));
    }

    #[test]
    fn full_date_in_future_is_active() {
        let created = created_at_from_filename("20260612_143809_036.alert").unwrap();
        let c = convert("18:50 20:06:2026 match", created, None, 42, now());
        assert_eq!(c.status, Status::Scheduled);
        assert_eq!(
            c.event.next_datetime.unwrap().date(),
            NaiveDate::from_ymd_opt(2026, 6, 20).unwrap()
        );
    }

    #[test]
    fn weekdays_recur_weekly() {
        let created = created_at_from_filename("20230620_140142_480.alert").unwrap();
        let c = convert("17:00 1-5 snack", created, None, 42, now());
        assert_eq!(c.status, Status::Scheduled);
        let days = c.event.days.unwrap();
        assert_eq!(days.len(), 5);
        assert!(days.contains(&Weekday::Mon) && days.contains(&Weekday::Fri));
        assert!(!days.contains(&Weekday::Sat));
        assert!(c.event.date.is_none());
    }

    #[test]
    fn day_only_is_one_shot_concrete_date() {
        // Created 2024-10-12; day 27 -> 2024-10-27, in the past now -> inactive.
        let created = created_at_from_filename("20241012_105344_580.alert").unwrap();
        let c = convert("16:33 27: concert", created, None, 42, now());
        assert_eq!(c.event.date, NaiveDate::from_ymd_opt(2024, 10, 27));
        assert!(c.event.year_explicit);
        assert_eq!(c.status, Status::Inactive);
    }

    #[test]
    fn period_maps_to_repetition() {
        // "28/1:" -> day 28, every 1 day.
        let p = parse_legacy("22:15 28/1: rent").unwrap();
        assert_eq!(p.day, Some(28));
        assert_eq!(
            p.repetition,
            Some(Repetition {
                interval: 1,
                unit: TimeUnit::Days
            })
        );
        // "05/2:11:" -> day 5 every 2 days, month 11.
        let p = parse_legacy("11:07 05/2:11: bday").unwrap();
        assert_eq!((p.day, p.month), (Some(5), Some(11)));
        assert_eq!(p.repetition.unwrap().unit, TimeUnit::Days);
        // Minute period "11:36/90 4:" -> every 90 minutes.
        let p = parse_legacy("11:36/90 4: pay").unwrap();
        assert_eq!(p.repetition.unwrap().unit, TimeUnit::Minutes);
        assert_eq!(p.day, Some(4));
    }

    #[test]
    fn periodic_uses_last_active_time() {
        let created = created_at_from_filename("20180131_143625_667.alert").unwrap();
        // 1782764100000 ms = 2026-07-... (future relative to test now).
        let future_ms = Local
            .from_local_datetime(&(now() + chrono::Duration::days(10)))
            .single()
            .unwrap()
            .timestamp_millis();
        let c = convert("22:15 28/1: rent", created, Some(future_ms), 42, now());
        assert_eq!(c.status, Status::Scheduled);
        assert!(c.event.next_datetime.is_some());

        // Stale activation is rolled forward using the input's period (every 1 day).
        let past_ms = Local
            .from_local_datetime(&(now() - chrono::Duration::days(10)))
            .single()
            .unwrap()
            .timestamp_millis();
        let c = convert("22:15 28/1: rent", created, Some(past_ms), 42, now());
        assert_eq!(c.status, Status::Scheduled);
        assert!(c.event.active);
        assert!(c.recalculated);
        assert!(c.event.next_datetime.unwrap() > now());
    }

    #[test]
    fn unparsable_keeps_raw_text_inactive() {
        let created = now();
        let c = convert("no time here at all", created, None, 42, now());
        assert_eq!(c.status, Status::Unparsed);
        assert_eq!(c.event.message, "no time here at all");
        assert!(c.event.time.is_none());
        assert!(!c.event.active);
        assert!(c.event.legacy);
    }
}
