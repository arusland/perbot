use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use std::collections::HashSet;
use tokio::sync::mpsc;

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

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "private" => Some(ChatType::Private),
            "group" => Some(ChatType::Group),
            "supergroup" => Some(ChatType::Supergroup),
            "channel" => Some(ChatType::Channel),
            _ => None,
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

pub struct TgMessage {
    pub chat_id: i64,
    pub text: String,
}

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
