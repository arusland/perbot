//! English [`LocaleProvider`] — the only locale implemented today, and the
//! worked example for adding more (see the [`super`] module docs).
//!
//! It supplies a [`GrammarVocab`] (the shared builder assembles the regexes),
//! maps English words to the shared enums, and provides the output vocabulary and
//! format patterns. Storage-canonical vocabulary (`unit_from_str`,
//! `day_from_str`, `parse_days`, `TimeUnit::label`) is delegated to
//! [`crate::types`] so the DB serialization and the English UI stay
//! byte-identical (and the DB stays locale-independent).

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use std::collections::HashSet;
use std::sync::LazyLock;

use super::{GrammarVocab, LocaleProvider, TimeGrammar};
use crate::types::{self, Ordinal, TimeUnit};

/// English vocabulary fed to the shared regex builder. NOTE: the time regexes are
/// intentionally not anchored to the start of the message — a clock time is
/// matched wherever it appears, while the offset/bare-hour/short-date forms must
/// lead (enforced by the builder's anchors, not this data). An optional leading
/// `in`/`every`/`each` and a trailing `yearly` are absorbed so the loose
/// spellings collapse to the canonical form emitted by `normalize_time`.
const VOCAB: GrammarVocab = GrammarVocab {
    units: r"min(?:ute)?s?|hours?|days?|weeks?|months?|years?",
    weekdays: r"mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?",
    ordinals: r"first|1st|second|2nd|third|3rd|fourth|4th|fifth|5th|last",
    ordinal_suffix: r"(?:st|nd|rd|th)",
    every: "every",
    in_word: "in",
    yearly: "yearly",
    each: "each",
    day_word: "day",
    of_the_month: r"of\s+(?:the\s+)?month",
    month_suffix_strict: r"of\s+the\s+month",
    am: "AM",
    pm: "PM",
};

static GRAMMAR: LazyLock<TimeGrammar> = LazyLock::new(|| TimeGrammar::build(&VOCAB));

/// The English locale.
pub struct English;

impl LocaleProvider for English {
    fn grammar(&self) -> &TimeGrammar {
        &GRAMMAR
    }

    fn unit_from_str(&self, s: &str) -> Option<TimeUnit> {
        types::unit_from_str(s)
    }
    fn day_from_str(&self, s: &str) -> Option<Weekday> {
        types::day_from_str(s)
    }
    fn parse_days(&self, s: &str) -> Option<HashSet<Weekday>> {
        types::parse_days(s)
    }
    fn ordinal_from_str(&self, s: &str) -> Option<Ordinal> {
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

    fn unit_label(&self, unit: TimeUnit, plural: bool) -> &'static str {
        unit.label(plural)
    }

    fn weekday_abbrev_cap(&self, wd: Weekday) -> &'static str {
        match wd {
            Weekday::Mon => "Mon",
            Weekday::Tue => "Tue",
            Weekday::Wed => "Wed",
            Weekday::Thu => "Thu",
            Weekday::Fri => "Fri",
            Weekday::Sat => "Sat",
            Weekday::Sun => "Sun",
        }
    }

    fn weekday_full(&self, wd: Weekday) -> &'static str {
        match wd {
            Weekday::Mon => "Monday",
            Weekday::Tue => "Tuesday",
            Weekday::Wed => "Wednesday",
            Weekday::Thu => "Thursday",
            Weekday::Fri => "Friday",
            Weekday::Sat => "Saturday",
            Weekday::Sun => "Sunday",
        }
    }

    fn ordinal_word(&self, ord: Ordinal) -> &'static str {
        match ord {
            Ordinal::First => "first",
            Ordinal::Second => "second",
            Ordinal::Third => "third",
            Ordinal::Fourth => "fourth",
            Ordinal::Fifth => "fifth",
            Ordinal::Last => "last",
        }
    }

    fn ordinal_suffix(&self, n: u32) -> String {
        let suffix = match (n % 10, n % 100) {
            (1, 11) | (2, 12) | (3, 13) => "th",
            (1, _) => "st",
            (2, _) => "nd",
            (3, _) => "rd",
            _ => "th",
        };
        format!("{n}{suffix}")
    }

    fn offset_prefix(&self) -> &'static str {
        "in"
    }
    fn every_word(&self) -> &'static str {
        "every"
    }
    fn yearly_marker(&self) -> &'static str {
        "yearly"
    }
    fn day_of_month_canonical(&self, ordinal_suffix: &str) -> String {
        format!("each {ordinal_suffix} day of the month")
    }
    fn last_day_phrase(&self) -> &'static str {
        "last day of the month"
    }
    fn format_time(&self, t: NaiveTime) -> String {
        t.format("%H:%M").to_string()
    }
    fn format_date(&self, d: NaiveDate) -> String {
        d.format("%d.%m").to_string()
    }
    fn format_date_year(&self, d: NaiveDate) -> String {
        d.format("%d.%m.%Y").to_string()
    }

    fn day_of_month_recurrence(&self, ordinal_suffix: &str) -> String {
        format!("{ordinal_suffix} day of the month")
    }

    fn format_relative(&self, secs: i64) -> String {
        if secs <= 0 {
            return "soon".to_string();
        }
        let mins = secs / 60;
        if mins < 1 {
            return "soon".to_string();
        }
        if mins < 60 {
            return format!("{} min{}", mins, if mins == 1 { "" } else { "s" });
        }
        let hours = mins / 60;
        if hours < 24 {
            return format!("{hours}h");
        }
        let days = hours / 24;
        if days < 7 {
            return format!("{days}d");
        }
        let weeks = days / 7;
        if weeks < 52 {
            return format!("{weeks}w");
        }
        // >= ~1 year: show years to one decimal, dropping a trailing ".0".
        // tenths-of-a-year via integer rounding (+182 ≈ half of 365).
        let tenths = (days * 10 + 182) / 365;
        if tenths % 10 == 0 {
            format!("{}y", tenths / 10)
        } else {
            format!("{}.{}y", tenths / 10, tenths % 10)
        }
    }

    fn format_datetime(&self, dt: NaiveDateTime) -> String {
        dt.format("%H:%M %d.%m.%Y").to_string()
    }
    fn next_launches_header(&self) -> &'static str {
        "Next launches:"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared builder must reproduce the historical hand-written English
    /// regexes byte-for-byte, so `test-cases.md` keeps passing unchanged.
    #[test]
    fn english_grammar_matches_legacy_regex_strings() {
        let g = TimeGrammar::build(&VOCAB);
        assert_eq!(g.time_12h.as_str(), r"(?i)(\d{1,2}):(\d{1,2})\s*(AM|PM)");
        assert_eq!(g.time_24h.as_str(), r"(\d{1,2}):(\d{1,2})");
        assert_eq!(g.date_full.as_str(), r"(\d{1,2})\.(\d{1,2})\.(\d{4})");
        assert_eq!(g.date_short.as_str(), r"(\d{1,2})\.(\d{1,2})(?:[^\.\d]|$)");
        assert_eq!(
            g.every.as_str(),
            r"(?i)\bevery\s+(?:(\d+)\s+)?(min(?:ute)?s?|hours?|days?|weeks?|months?|years?)\b"
        );
        assert_eq!(g.yearly.as_str(), r"(?i)\byearly\b");
        assert_eq!(
            g.in_offset.as_str(),
            r"(?i)^(?:in\s+)?(\d+)\s+(min(?:ute)?s?|hours?|days?|weeks?|months?|years?)\b"
        );
        assert_eq!(g.bare_hour.as_str(), r"^(\d{1,2})\s");
        assert_eq!(
            g.days.as_str(),
            r"(?i)\b(?:every\s+)?((?:mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?)(?:\s*[-,]\s*(?:mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?))*)\b"
        );
        assert_eq!(
            g.day_of_month.as_str(),
            r"(?i)\b(?:every\s+|each\s+)?(\d{1,2})(?:st|nd|rd|th)?\s+(?:day\s+)?of\s+(?:the\s+)?month\b"
        );
        assert_eq!(
            g.monthly.as_str(),
            r"(?i)\b(first|1st|second|2nd|third|3rd|fourth|4th|fifth|5th|last)\s+(mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?|day)(?:\s+of\s+the\s+month)?\b"
        );
        assert_eq!(g.years.as_str(), r"\b(\d{4}(?:\s*,\s*\d{4})*)\b");
    }
}
