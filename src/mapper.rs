use chrono::{Local, Weekday};

use crate::parser::{MonthlyPattern, Ordinal, ParsedEvent, TimeUnit};
use crate::storage::StoredEvent;

/// Converts a `ParsedEvent` into a `StoredEvent` ready for persistence.
/// Does not calculate datetimes — call `storage::play` on the result to
/// compute `next_datetime` and set `active`.
pub fn map(event: ParsedEvent, chat_id: i64, message_id: i64) -> StoredEvent {
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

    StoredEvent {
        id: 0,
        chat_id,
        date: event.date,
        time: event.time,
        year_explicit: event.year_explicit,
        days,
        message: event.message,
        active: false,
        next_datetime: None,
        created_at: now,
        repeat_interval,
        repeat_unit,
        in_offset,
        in_offset_unit,
        bare_hour: event.bare_hour,
        monthly_pattern,
        msg_id: message_id,
    }
}
