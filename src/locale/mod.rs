//! Localization seam for all time-format **input parsing** and **time-bearing
//! output**.
//!
//! Every natural-language time word the bot accepts (`every`, `in`, weekday
//! names, `first sunday`, AM/PM, …) and every time word it emits (the
//! re-parseable [`crate::types::EventInfo::normalize_time`] canonical form, the
//! `/events` recurrence description, the relative time `13 mins`/`2d`, the date
//! pattern, the `Next launches:` header) is routed through a [`LocaleProvider`].
//! The active locale is threaded as an explicit `&dyn LocaleProvider` argument so
//! per-chat locale selection is possible without a global.
//!
//! # Adding a new locale
//!
//! A locale supplies **data**, not regex structure:
//!
//! 1. Fill in a [`GrammarVocab`] with the language's word alternations and
//!    keywords, then build its regexes once via [`TimeGrammar::build`]. The
//!    intricate regex shapes (weekday ranges, ordinal monthly patterns, the
//!    "of the month" tail, the offset/`every` interval forms) live in the shared
//!    builder — a locale never writes them.
//! 2. Implement [`LocaleProvider`]: return the prebuilt [`TimeGrammar`], map the
//!    language's words to the shared enums ([`TimeUnit`]/[`Weekday`]/[`Ordinal`]),
//!    and supply the output vocabulary, `chrono` format patterns and the
//!    relative-time formatter.
//!
//! See [`english`] as the worked example.
//!
//! **Boundary:** this is *not* the database serialization format. `storage.rs`
//! and `converter.rs` persist weekday/unit strings via the fixed canonical free
//! functions in [`crate::types`] (`day_to_str`, `parse_days`, `unit_from_str`,
//! `TimeUnit::label`) so the SQLite columns never depend on the UI locale. The
//! English provider *reuses* those functions for its English vocabulary.

pub mod english;

use crate::types::{Ordinal, TimeUnit};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use regex::Regex;
use std::collections::HashSet;

/// The single English locale instance.
pub static EN: english::English = english::English;

/// Resolves the locale for a chat. The single seam where future per-chat locale
/// selection will hook in; returns English for every chat today.
pub fn for_chat(_chat_id: i64) -> &'static dyn LocaleProvider {
    &EN
}

/// The per-locale vocabulary the shared regex builder ([`TimeGrammar::build`])
/// needs. Every field is a **regex fragment** (so callers can supply optional
/// suffixes, alternations, etc.); a locale fills these instead of writing the
/// full time-expression regexes by hand.
pub struct GrammarVocab {
    /// Unit alternation, e.g. `"min(?:ute)?s?|hours?|days?|weeks?|months?|years?"`.
    pub units: &'static str,
    /// Weekday alternation (no surrounding group), e.g.
    /// `"mon(?:day)?|tue(?:sday)?|…|sun(?:day)?"`.
    pub weekdays: &'static str,
    /// Ordinal alternation, e.g. `"first|1st|second|2nd|…|last"`.
    pub ordinals: &'static str,
    /// Optional ordinal-suffix fragment after a number, e.g. `"(?:st|nd|rd|th)"`.
    pub ordinal_suffix: &'static str,
    /// Repetition lead word, e.g. `"every"`.
    pub every: &'static str,
    /// Relative-offset lead word, e.g. `"in"`.
    pub in_word: &'static str,
    /// Standalone yearly marker, e.g. `"yearly"`.
    pub yearly: &'static str,
    /// Alternative day-of-month lead word, e.g. `"each"`.
    pub each: &'static str,
    /// Literal `"day"` word accepted in patterns.
    pub day_word: &'static str,
    /// Day-of-month tail, optional `the`, e.g. `r"of\s+(?:the\s+)?month"`.
    pub of_the_month: &'static str,
    /// Strict monthly-pattern tail (mandatory `the`), e.g. `r"of\s+the\s+month"`.
    pub month_suffix_strict: &'static str,
    /// 12-hour ante-meridiem marker, e.g. `"AM"`.
    pub am: &'static str,
    /// 12-hour post-meridiem marker, e.g. `"PM"`.
    pub pm: &'static str,
}

/// The compiled time-expression regexes for one locale, assembled from a
/// [`GrammarVocab`] by [`TimeGrammar::build`]. The purely numeric forms
/// (24h clock, dotted dates, bare hour, year list) are locale-independent and
/// shared verbatim.
pub struct TimeGrammar {
    pub time_12h: Regex,
    pub time_24h: Regex,
    pub date_full: Regex,
    pub date_short: Regex,
    pub every: Regex,
    pub yearly: Regex,
    pub in_offset: Regex,
    pub bare_hour: Regex,
    pub days: Regex,
    pub day_of_month: Regex,
    pub monthly: Regex,
    pub years: Regex,
}

impl TimeGrammar {
    /// Assembles all twelve regexes from `v`. The regex *shapes* are fixed here;
    /// only the vocabulary varies per locale.
    pub fn build(v: &GrammarVocab) -> Self {
        let wd = v.weekdays;
        TimeGrammar {
            // 12h before 24h so "5:24 PM" is not partially consumed as "5:24".
            time_12h: Regex::new(&format!(
                r"(?i)(\d{{1,2}}):(\d{{1,2}})\s*({}|{})",
                v.am, v.pm
            ))
            .unwrap(),
            time_24h: Regex::new(r"(\d{1,2}):(\d{1,2})").unwrap(),
            date_full: Regex::new(r"(\d{1,2})\.(\d{1,2})\.(\d{4})").unwrap(),
            date_short: Regex::new(r"(\d{1,2})\.(\d{1,2})(?:[^\.\d]|$)").unwrap(),
            every: Regex::new(&format!(
                r"(?i)\b{}\s+(?:(\d+)\s+)?({})\b",
                v.every, v.units
            ))
            .unwrap(),
            yearly: Regex::new(&format!(r"(?i)\b{}\b", v.yearly)).unwrap(),
            in_offset: Regex::new(&format!(
                r"(?i)^(?:{}\s+)?(\d+)\s+({})\b",
                v.in_word, v.units
            ))
            .unwrap(),
            bare_hour: Regex::new(r"^(\d{1,2})\s").unwrap(),
            days: Regex::new(&format!(
                r"(?i)\b(?:{}\s+)?((?:{wd})(?:\s*[-,]\s*(?:{wd}))*)\b",
                v.every
            ))
            .unwrap(),
            day_of_month: Regex::new(&format!(
                r"(?i)\b(?:{}\s+|{}\s+)?(\d{{1,2}}){}?\s+(?:{}\s+)?{}\b",
                v.every, v.each, v.ordinal_suffix, v.day_word, v.of_the_month
            ))
            .unwrap(),
            monthly: Regex::new(&format!(
                r"(?i)\b({})\s+({}|{})(?:\s+{})?\b",
                v.ordinals, wd, v.day_word, v.month_suffix_strict
            ))
            .unwrap(),
            years: Regex::new(r"\b(\d{4}(?:\s*,\s*\d{4})*)\b").unwrap(),
        }
    }
}

/// Supplies all locale-specific vocabulary, regexes and format patterns used to
/// parse time expressions and render time-bearing output. See the module docs for
/// how to add a locale and the storage-vs-locale boundary.
pub trait LocaleProvider: Sync {
    /// The compiled time-expression regexes (built once via [`TimeGrammar::build`]).
    fn grammar(&self) -> &TimeGrammar;

    // --- Input word maps ---
    fn unit_from_str(&self, s: &str) -> Option<TimeUnit>;
    fn day_from_str(&self, s: &str) -> Option<Weekday>;
    fn parse_days(&self, s: &str) -> Option<HashSet<Weekday>>;
    fn ordinal_from_str(&self, s: &str) -> Option<Ordinal>;

    // --- Output vocabulary ---
    /// Unit word: plural (`"days"`) or singular (`"day"`).
    fn unit_label(&self, unit: TimeUnit, plural: bool) -> &'static str;
    /// Capitalized 3-letter weekday (`"Mon"`), used in canonical weekday sets.
    fn weekday_abbrev_cap(&self, wd: Weekday) -> &'static str;
    /// Full weekday name (`"Monday"`).
    fn weekday_full(&self, wd: Weekday) -> &'static str;
    /// Ordinal word for a monthly pattern (`"first"`…`"last"`).
    fn ordinal_word(&self, ord: Ordinal) -> &'static str;
    /// Ordinal numeral (`28` → `"28th"`).
    fn ordinal_suffix(&self, n: u32) -> String;

    // --- Canonical (re-parseable) joiner literals for `normalize_time` ---
    /// Leading word of a relative offset, e.g. `"in"` in `in 8 minutes`.
    fn offset_prefix(&self) -> &'static str;
    /// Repetition lead word, e.g. `"every"` in `every 2 days`.
    fn every_word(&self) -> &'static str;
    /// Trailing marker emitted for an implicitly-yearly short date.
    fn yearly_marker(&self) -> &'static str;
    /// Canonical day-of-month phrase, e.g. `"each 28th day of the month"`.
    fn day_of_month_canonical(&self, ordinal_suffix: &str) -> String;
    /// `"last day of the month"` (identical in canonical and recurrence forms).
    fn last_day_phrase(&self) -> &'static str;
    /// A bare clock time, e.g. `"13:05"`.
    fn format_time(&self, t: NaiveTime) -> String;
    /// A short date without year, e.g. `"26.11"`.
    fn format_date(&self, d: NaiveDate) -> String;
    /// A full date with year, e.g. `"26.11.2027"`.
    fn format_date_year(&self, d: NaiveDate) -> String;

    // --- Display-only output (`describe_recurrence`, list/detail rendering) ---
    /// Recurrence phrase for a fixed calendar day, e.g. `"28th day of the month"`.
    fn day_of_month_recurrence(&self, ordinal_suffix: &str) -> String;
    /// Short relative time for a positive/negative second delta until an event,
    /// e.g. `"soon"`, `"13 mins"`, `"2d"`, `"1.4y"`.
    fn format_relative(&self, secs: i64) -> String;
    /// An absolute datetime line, e.g. `"13:05 31.12.2027"`.
    fn format_datetime(&self, dt: NaiveDateTime) -> String;
    /// Bold header above the upcoming-launches preview (`"Next launches:"`).
    fn next_launches_header(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    #[test]
    fn for_chat_returns_english_and_round_trips() {
        let loc = for_chat(42);
        // A parse → normalize round-trip through the trait object yields the
        // canonical English form.
        let event = parser::parse("8 call Alex", loc).unwrap();
        assert_eq!(event.normalize_time(loc), "08:00");
        let event = parser::parse("13:30 mon-fri standup", loc).unwrap();
        assert_eq!(event.normalize_time(loc), "13:30 Mon-Fri");
    }
}
