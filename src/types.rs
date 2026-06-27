use crate::locale::LocaleProvider;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use std::collections::HashSet;
use teloxide::types::InlineKeyboardMarkup;
use tokio::sync::mpsc;

/// A parsed reminder event plus the fields used to track it in the database.
///
/// The first group of fields is populated by [`crate::parser::parse`]; the
/// trailing group (`id`, `chat_id`, `active`, `next_datetime`,
/// `last_next_datetime`, `created_at`, `msg_id`) is filled in by
/// storage/scheduling and defaults to zero/false/None
/// on a freshly parsed value.
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
    /// The reminder body as an **HTML fragment**, ready to embed in
    /// `ParseMode::Html` output. It is the text left after extracting all
    /// time/date components, with the user's Telegram formatting (bold, italic,
    /// links, …) preserved as HTML tags. Plain messages are escaped HTML (so
    /// `<`/`>`/`&` are safe). The parser fills this with the *plain* extracted
    /// text as a transient value; `main` replaces it with the HTML rendering
    /// (`crate::richtext::render_html`) before persisting. Every consumer
    /// (confirmation, lists, fired reminder, snooze) reads the HTML form.
    pub message: String,
    pub id: i64,
    pub chat_id: i64,
    pub active: bool,
    pub next_datetime: Option<NaiveDateTime>,
    /// The most recent non-null `next_datetime` this event ever had. Tracks when
    /// the event last fired so an inactive (out-of-date) event can still report
    /// it. Set by [`crate::scheduler::calc_next_at`]; never cleared once set.
    pub last_next_datetime: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub msg_id: i64,
    /// `true` for events imported from the legacy MateBot `.alert` files.
    pub legacy: bool,
    /// `true` for one-off events created by the snooze flow.
    pub snoozed: bool,
}

impl EventInfo {
    /// Canonical, **re-parseable** rendering of this event's time/recurrence
    /// expression (the message body is *not* included). Re-parsing the returned
    /// string and calling `normalize_time` again yields the byte-identical string
    /// (idempotent round-trip), so it collapses the many loose spellings the
    /// parser accepts into one form:
    ///
    /// - clock time → zero-padded 24h `HH:MM` (`12:2` → `12:02`, `5:24 PM` → `17:24`);
    /// - bare hour → the equivalent clock time `HH:00` (`8` → `08:00`, `24` → `00:00`);
    /// - relative offset → `in <n> <unit>` (`8 min` → `in 8 minutes`, `1 minute` → `in 1 minute`);
    /// - dates → `dd.mm` or `dd.mm.yyyy` (when the year was explicit);
    /// - year restrictions → ascending comma list (`2027,2028`);
    /// - weekday sets → capitalized 3-letter days, Monday-first, contiguous runs of
    ///   ≥3 collapsed to `First-Last` (`mon-Friday,Sat` → `Mon-Sat`);
    /// - monthly patterns → `first sunday` / `last day of the month` / `each 28th day of the month`;
    /// - repetition → `every <unit>` / `every <n> <units>`.
    ///
    /// Parts are emitted in the order [`crate::parser`] re-extracts them and joined
    /// with single spaces. Returns `""` for a value with no time component (which
    /// the parser never produces).
    pub fn normalize_time(&self, loc: &dyn LocaleProvider) -> String {
        let mut parts: Vec<String> = Vec::new();

        // Anchor: offset / clock time / bare hour (mutually exclusive).
        if let Some((n, unit)) = self.in_offset {
            parts.push(format!(
                "{} {n} {}",
                loc.offset_prefix(),
                loc.unit_label(unit, n != 1)
            ));
        } else if let Some(t) = self.time {
            parts.push(loc.format_time(t));
        } else if let Some(h) = self.bare_hour {
            parts.push(format!("{:02}:00", h % 24));
        }

        // Date (never co-occurs with an offset). A short date (no explicit year)
        // is a yearly event; the `yearly` marker is emitted last (after any
        // repetition, see below) so it always trails the canonical string.
        if let Some(d) = self.date {
            if self.year_explicit {
                parts.push(loc.format_date_year(d));
            } else {
                parts.push(loc.format_date(d));
            }
        }

        // Year restrictions (only present when there is no explicit date).
        if let Some(years) = &self.years {
            let mut ys: Vec<i32> = years.iter().copied().collect();
            ys.sort_unstable();
            parts.push(
                ys.iter()
                    .map(|y| y.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }

        // Day recurrence: weekday set or monthly pattern (mutually exclusive).
        if let Some(days) = &self.days {
            parts.push(format_weekday_set(days, loc));
        } else if let Some(pattern) = &self.monthly_pattern {
            parts.push(match pattern {
                MonthlyPattern::OrdinalWeekday(ord, wd) => {
                    format!("{} {}", loc.ordinal_word(*ord), loc.weekday_full(*wd))
                }
                MonthlyPattern::LastDay => loc.last_day_phrase().to_string(),
                MonthlyPattern::DayOfMonth(d) => {
                    loc.day_of_month_canonical(&loc.ordinal_suffix(*d))
                }
            });
        }

        // Repetition interval.
        if let Some(rep) = &self.repetition {
            parts.push(if rep.interval == 1 {
                format!("{} {}", loc.every_word(), loc.unit_label(rep.unit, false))
            } else {
                format!(
                    "{} {} {}",
                    loc.every_word(),
                    rep.interval,
                    loc.unit_label(rep.unit, true)
                )
            });
        }

        // A short date (no explicit year) is yearly by default. Emit the marker
        // last, so it trails any repetition (`every 2 days yearly`). This is a
        // distinct token from the `every year` a `Years` repetition produces.
        if self.date.is_some() && !self.year_explicit {
            parts.push(loc.yearly_marker().to_string());
        }

        parts.join(" ")
    }
}

/// Renders a weekday set as a canonical, re-parseable string: capitalized
/// 3-letter day names, Monday-first, with contiguous runs of length ≥3 collapsed
/// into `First-Last` ranges and shorter runs / isolated days listed individually,
/// all joined with `,`. E.g. `{Mon..Sat}` → `"Mon-Sat"`, `{Mon,Tue,Wed,Fri}` →
/// `"Mon-Wed,Fri"`, `{Thu,Sun}` → `"Thu,Sun"`.
fn format_weekday_set(days: &HashSet<Weekday>, loc: &dyn LocaleProvider) -> String {
    let mut idx: Vec<u32> = days.iter().map(|d| d.num_days_from_monday()).collect();
    idx.sort_unstable();

    let mut groups: Vec<String> = Vec::new();
    let mut i = 0;
    while i < idx.len() {
        // Extend the run while indices stay consecutive.
        let mut j = i;
        while j + 1 < idx.len() && idx[j + 1] == idx[j] + 1 {
            j += 1;
        }
        let run_len = j - i + 1;
        let first = loc.weekday_abbrev_cap(weekday_from_monday(idx[i]));
        if run_len >= 3 {
            let last = loc.weekday_abbrev_cap(weekday_from_monday(idx[j]));
            groups.push(format!("{first}-{last}"));
        } else {
            for k in i..=j {
                groups.push(
                    loc.weekday_abbrev_cap(weekday_from_monday(idx[k]))
                        .to_string(),
                );
            }
        }
        i = j + 1;
    }
    groups.join(",")
}

fn weekday_from_monday(i: u32) -> Weekday {
    day_from_index((i % 7) as u8)
}

/// User message information. Used both for inserting and for reading from the database.
/// `id` and `created_at` are `None` when constructing a value to insert
/// and `Some` when reading back from the database.
#[derive(Debug, Clone)]
pub struct MessageInfo {
    pub id: i64,
    pub user_id: Option<i64>,
    pub chat_id: i64,
    pub created_at: Option<NaiveDateTime>,
    pub message: String,
}

/// Chat type enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatType {
    Private,
    Group,
    Supergroup,
    Channel,
}

impl ChatType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChatType::Private => "private",
            ChatType::Group => "group",
            ChatType::Supergroup => "supergroup",
            ChatType::Channel => "channel",
        }
    }
}

impl std::str::FromStr for ChatType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "private" => Ok(ChatType::Private),
            "group" => Ok(ChatType::Group),
            "supergroup" => Ok(ChatType::Supergroup),
            "channel" => Ok(ChatType::Channel),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Years,
}

impl TimeUnit {
    /// Lower-case unit word: plural (`"days"`, the form persisted in storage) or
    /// singular (`"day"`) when `plural` is false.
    pub fn label(self, plural: bool) -> &'static str {
        match (self, plural) {
            (TimeUnit::Minutes, true) => "minutes",
            (TimeUnit::Minutes, false) => "minute",
            (TimeUnit::Hours, true) => "hours",
            (TimeUnit::Hours, false) => "hour",
            (TimeUnit::Days, true) => "days",
            (TimeUnit::Days, false) => "day",
            (TimeUnit::Weeks, true) => "weeks",
            (TimeUnit::Weeks, false) => "week",
            (TimeUnit::Months, true) => "months",
            (TimeUnit::Months, false) => "month",
            (TimeUnit::Years, true) => "years",
            (TimeUnit::Years, false) => "year",
        }
    }
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
    /// Fixed calendar day of the month (`28` → the 28th). Months that lack that
    /// day (e.g. the 31st in February) are skipped.
    DayOfMonth(u32),
}

/// Chat information. Used both for upserting and for reading from the database.
/// `updated_at` and `created_at` are `None` when constructing a value to upsert
/// and `Some` when reading back from the database.
#[derive(Debug, Clone)]
pub struct ChatInfo {
    pub id: i64,
    pub chat_type: ChatType,
    pub title: Option<String>,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub updated_at: Option<NaiveDateTime>,
    pub created_at: Option<NaiveDateTime>,
}

/// An outbound Telegram message: the destination chat, its final (already
/// formatted, hint already appended) text body, and the inline keyboard the
/// producer chose for it. `reply_markup` is `Some(..)` when the message should
/// carry buttons (e.g. the snooze keyboard on a fired reminder) and `None`
/// otherwise (e.g. the missed-events batch). The sender task forwards both
/// verbatim — it does not decide which buttons a message gets. `text` is always
/// an HTML fragment; the sender always sends it with `ParseMode::Html`.
pub struct TgMessage {
    pub chat_id: i64,
    pub text: String,
    pub reply_markup: Option<InlineKeyboardMarkup>,
}

/// Channel sender used by the scheduler to hand batches of due/missed messages
/// to the Telegram-sending task.
pub type MessageSender = mpsc::UnboundedSender<Vec<TgMessage>>;

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

/// Parses a weekday name (`"mon"`/`"monday"`, case-insensitive) into a [`Weekday`].
pub fn day_from_str(s: &str) -> Option<Weekday> {
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

/// The canonical three-letter abbreviation used to persist a [`Weekday`].
pub fn day_to_str(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::EN;

    /// A blank `EventInfo` with every time/recurrence field cleared; tests set
    /// only the fields they exercise.
    fn blank() -> EventInfo {
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
            message: String::new(),
            id: 0,
            chat_id: 0,
            active: false,
            next_datetime: None,
            last_next_datetime: None,
            created_at: NaiveDate::from_ymd_opt(2024, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            msg_id: 0,
            legacy: false,
            snoozed: false,
        }
    }

    fn day_set(days: &[Weekday]) -> HashSet<Weekday> {
        days.iter().copied().collect()
    }

    #[test]
    fn normalize_clock_time_zero_pads() {
        let mut e = blank();
        e.time = NaiveTime::from_hms_opt(12, 2, 0);
        assert_eq!(e.normalize_time(&EN), "12:02");
        e.time = NaiveTime::from_hms_opt(1, 25, 0);
        assert_eq!(e.normalize_time(&EN), "01:25");
        // 5:24 PM is stored as 17:24, rendered 24h.
        e.time = NaiveTime::from_hms_opt(17, 24, 0);
        assert_eq!(e.normalize_time(&EN), "17:24");
    }

    #[test]
    fn normalize_bare_hour_as_clock_time() {
        let mut e = blank();
        e.bare_hour = Some(8);
        assert_eq!(e.normalize_time(&EN), "08:00");
        e.bare_hour = Some(24);
        assert_eq!(e.normalize_time(&EN), "00:00");
        e.bare_hour = Some(0);
        assert_eq!(e.normalize_time(&EN), "00:00");
        e.bare_hour = Some(21);
        e.repetition = Some(Repetition {
            interval: 2,
            unit: TimeUnit::Days,
        });
        assert_eq!(e.normalize_time(&EN), "21:00 every 2 days");
    }

    #[test]
    fn normalize_offset_plural_and_singular() {
        let mut e = blank();
        e.in_offset = Some((8, TimeUnit::Minutes));
        assert_eq!(e.normalize_time(&EN), "in 8 minutes");
        e.in_offset = Some((1, TimeUnit::Minutes));
        assert_eq!(e.normalize_time(&EN), "in 1 minute");
        e.in_offset = Some((3, TimeUnit::Hours));
        assert_eq!(e.normalize_time(&EN), "in 3 hours");
        // The user's example: "8 min every 2 hour test" → "in 8 minutes every 2 hours".
        e.in_offset = Some((8, TimeUnit::Minutes));
        e.repetition = Some(Repetition {
            interval: 2,
            unit: TimeUnit::Hours,
        });
        assert_eq!(e.normalize_time(&EN), "in 8 minutes every 2 hours");
        e.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Hours,
        });
        assert_eq!(e.normalize_time(&EN), "in 8 minutes every hour");
    }

    #[test]
    fn normalize_dates() {
        let mut e = blank();
        e.time = NaiveTime::from_hms_opt(11, 26, 0);
        e.date = NaiveDate::from_ymd_opt(2026, 10, 12);
        e.year_explicit = true;
        assert_eq!(e.normalize_time(&EN), "11:26 12.10.2026");
        // Short date drops the year.
        e.date = NaiveDate::from_ymd_opt(2026, 11, 26);
        e.year_explicit = false;
        e.time = NaiveTime::from_hms_opt(1, 23, 0);
        assert_eq!(e.normalize_time(&EN), "01:23 26.11 yearly");
    }

    #[test]
    fn normalize_years_and_days() {
        let mut e = blank();
        e.time = NaiveTime::from_hms_opt(13, 25, 0);
        e.years = Some([2028, 2027].into_iter().collect());
        e.days = Some(day_set(&[Weekday::Sun, Weekday::Fri]));
        assert_eq!(e.normalize_time(&EN), "13:25 2027,2028 Fri,Sun");
    }

    #[test]
    fn normalize_weekday_set_collapsing() {
        let mut e = blank();
        e.time = NaiveTime::from_hms_opt(1, 25, 0);
        // mon-Friday,Sat == {Mon..Sat} collapses to one range.
        e.days = Some(day_set(&[
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
        ]));
        assert_eq!(e.normalize_time(&EN), "01:25 Mon-Sat");
        // A non-contiguous mix: a ≥3 run plus an isolated day.
        e.days = Some(day_set(&[
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Fri,
        ]));
        assert_eq!(e.normalize_time(&EN), "01:25 Mon-Wed,Fri");
        // A 2-day run stays a comma list (range needs ≥3).
        e.days = Some(day_set(&[Weekday::Thu, Weekday::Sun]));
        assert_eq!(e.normalize_time(&EN), "01:25 Thu,Sun");
        // Single day.
        e.days = Some(day_set(&[Weekday::Fri]));
        assert_eq!(e.normalize_time(&EN), "01:25 Fri");
    }

    #[test]
    fn normalize_monthly_patterns() {
        let mut e = blank();
        e.time = NaiveTime::from_hms_opt(10, 0, 0);
        e.monthly_pattern = Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Sun));
        assert_eq!(e.normalize_time(&EN), "10:00 first Sunday");
        e.time = NaiveTime::from_hms_opt(17, 0, 0);
        e.monthly_pattern = Some(MonthlyPattern::OrdinalWeekday(Ordinal::Third, Weekday::Fri));
        assert_eq!(e.normalize_time(&EN), "17:00 third Friday");
        e.time = NaiveTime::from_hms_opt(18, 0, 0);
        e.monthly_pattern = Some(MonthlyPattern::LastDay);
        assert_eq!(e.normalize_time(&EN), "18:00 last day of the month");
    }

    #[test]
    fn normalize_day_of_month() {
        let mut e = blank();
        e.time = NaiveTime::from_hms_opt(22, 15, 0);
        e.monthly_pattern = Some(MonthlyPattern::DayOfMonth(28));
        assert_eq!(e.normalize_time(&EN), "22:15 each 28th day of the month");
        // Combined with a repeat interval (day-of-month rendered before repetition).
        e.repetition = Some(Repetition {
            interval: 2,
            unit: TimeUnit::Days,
        });
        assert_eq!(
            e.normalize_time(&EN),
            "22:15 each 28th day of the month every 2 days"
        );
        // Ordinal suffixes.
        e.repetition = None;
        e.monthly_pattern = Some(MonthlyPattern::DayOfMonth(1));
        assert_eq!(e.normalize_time(&EN), "22:15 each 1st day of the month");
        e.monthly_pattern = Some(MonthlyPattern::DayOfMonth(2));
        assert_eq!(e.normalize_time(&EN), "22:15 each 2nd day of the month");
        e.monthly_pattern = Some(MonthlyPattern::DayOfMonth(3));
        assert_eq!(e.normalize_time(&EN), "22:15 each 3rd day of the month");
        e.monthly_pattern = Some(MonthlyPattern::DayOfMonth(11));
        assert_eq!(e.normalize_time(&EN), "22:15 each 11th day of the month");
        e.monthly_pattern = Some(MonthlyPattern::DayOfMonth(21));
        assert_eq!(e.normalize_time(&EN), "22:15 each 21st day of the month");
    }

    #[test]
    fn normalize_repetition() {
        let mut e = blank();
        e.time = NaiveTime::from_hms_opt(15, 30, 0);
        e.repetition = Some(Repetition {
            interval: 3,
            unit: TimeUnit::Days,
        });
        assert_eq!(e.normalize_time(&EN), "15:30 every 3 days");
        e.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Years,
        });
        e.time = NaiveTime::from_hms_opt(1, 34, 0);
        assert_eq!(e.normalize_time(&EN), "01:34 every year");
    }
}
