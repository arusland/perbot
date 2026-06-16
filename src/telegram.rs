use crate::types::{ChatInfo, ChatType, EventInfo};
use chrono::{Local, NaiveDateTime};
use std::fmt::Write as _;

pub fn escape_markdown(text: &str) -> String {
    let special_chars = [
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];
    let mut result = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if special_chars.contains(&c) {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

/// Short relative time until `dt` from `now`, e.g. `13 mins`, `1h`, `2d`, `1w`.
fn format_relative(now: NaiveDateTime, dt: NaiveDateTime) -> String {
    let secs = (dt - now).num_seconds();
    if secs <= 0 {
        return "now".to_string();
    }
    let mins = secs / 60;
    if mins < 1 {
        return "now".to_string();
    }
    if mins < 60 {
        return format!("{} min{}", mins, if mins == 1 { "" } else { "s" });
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{}h", hours);
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{}d", days);
    }
    format!("{}w", days / 7)
}

/// Builds a MarkdownV2 reply listing upcoming events ordered by next datetime.
pub fn format_events_list(events: &[EventInfo]) -> String {
    format_events_list_at(events, Local::now().naive_local())
}

/// Like [`format_events_list`] but with an explicit `now` for relative-time tests.
pub fn format_events_list_at(events: &[EventInfo], now: NaiveDateTime) -> String {
    format_list(events, now, "*Upcoming events:*", "No upcoming events\\.")
}

/// Builds a MarkdownV2 reply listing today's events ordered by next datetime.
pub fn format_today_list(events: &[EventInfo]) -> String {
    format_today_list_at(events, Local::now().naive_local())
}

/// Like [`format_today_list`] but with an explicit `now` for relative-time tests.
pub fn format_today_list_at(events: &[EventInfo], now: NaiveDateTime) -> String {
    format_list(events, now, "*Today's events:*", "No events today\\.")
}

/// Builds a MarkdownV2 reply listing tomorrow's events ordered by next datetime.
pub fn format_tomorrow_list(events: &[EventInfo]) -> String {
    format_tomorrow_list_at(events, Local::now().naive_local())
}

/// Like [`format_tomorrow_list`] but with an explicit `now` for relative-time tests.
pub fn format_tomorrow_list_at(events: &[EventInfo], now: NaiveDateTime) -> String {
    format_list(events, now, "*Tomorrow's events:*", "No events tomorrow\\.")
}

/// Builds a MarkdownV2 reply listing this month's events ordered by next datetime.
pub fn format_month_list(events: &[EventInfo]) -> String {
    format_month_list_at(events, Local::now().naive_local())
}

/// Like [`format_month_list`] but with an explicit `now` for relative-time tests.
pub fn format_month_list_at(events: &[EventInfo], now: NaiveDateTime) -> String {
    format_list(
        events,
        now,
        "*This month's events:*",
        "No events this month\\.",
    )
}

/// Builds a MarkdownV2 reply listing this week's events ordered by next datetime.
pub fn format_week_list(events: &[EventInfo]) -> String {
    format_week_list_at(events, Local::now().naive_local())
}

/// Like [`format_week_list`] but with an explicit `now` for relative-time tests.
pub fn format_week_list_at(events: &[EventInfo], now: NaiveDateTime) -> String {
    format_list(
        events,
        now,
        "*This week's events:*",
        "No events this week\\.",
    )
}

/// Renders an event list under `title`, or `empty` when there are no events.
fn format_list(events: &[EventInfo], now: NaiveDateTime, title: &str, empty: &str) -> String {
    if events.is_empty() {
        return empty.to_string();
    }
    let mut out = format!("{}\n", title);
    for e in events {
        let when = match e.next_datetime {
            Some(dt) => format!(
                "{} \\({}\\)",
                dt.format("%H:%M %d\\.%m\\.%Y"),
                escape_markdown(&format_relative(now, dt))
            ),
            None => "—".to_string(),
        };
        let _ = writeln!(out, "• {} — {}", when, escape_markdown(&e.message));
    }
    out
}

pub fn extract_chat_info(chat: &teloxide::types::Chat) -> ChatInfo {
    use teloxide::types::{ChatKind, PublicChatChannel, PublicChatKind, PublicChatSupergroup};

    let (chat_type, title, username, first_name, last_name) = match &chat.kind {
        ChatKind::Private(private) => (
            ChatType::Private,
            None,
            private.username.clone(),
            private.first_name.clone(),
            private.last_name.clone(),
        ),
        ChatKind::Public(public) => {
            let (chat_type, username) = match &public.kind {
                PublicChatKind::Group => (ChatType::Group, None),
                PublicChatKind::Supergroup(PublicChatSupergroup { username, .. }) => {
                    (ChatType::Supergroup, username.clone())
                }
                PublicChatKind::Channel(PublicChatChannel { username, .. }) => {
                    (ChatType::Channel, username.clone())
                }
            };
            (chat_type, public.title.clone(), username, None, None)
        }
    };

    ChatInfo {
        id: chat.id.0,
        chat_type,
        title,
        username,
        first_name,
        last_name,
        updated_at: None,
        created_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EventInfo;
    use chrono::{Duration, NaiveDate, NaiveTime};

    fn at(now: NaiveDateTime, d: Duration) -> String {
        format_relative(now, now + d)
    }

    fn sample_event(message: &str, next: Option<NaiveDateTime>) -> EventInfo {
        EventInfo {
            id: 0,
            chat_id: 0,
            date: None,
            time: None,
            year_explicit: false,
            days: None,
            years: None,
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: None,
            message: message.to_string(),
            active: next.is_some(),
            next_datetime: next,
            created_at: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            msg_id: 0,
            legacy: false,
        }
    }

    #[test]
    fn format_events_list_empty() {
        assert_eq!(
            format_events_list_at(&[], Local::now().naive_local()),
            "No upcoming events\\."
        );
    }

    #[test]
    fn format_events_list_rows() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events = vec![
            sample_event("call mom", Some(now + Duration::hours(2))),
            // Markdown special chars in the message must be escaped.
            sample_event("pay rent (urgent)", Some(now + Duration::days(3))),
        ];
        let out = format_events_list_at(&events, now);
        assert!(out.starts_with("*Upcoming events:*\n"));
        assert!(out.contains("14:00 15\\.06\\.2026 \\(2h\\)"));
        assert!(out.contains("pay rent \\(urgent\\)"));
        assert!(out.contains("\\(3d\\)"));
    }

    #[test]
    fn format_today_list_empty() {
        assert_eq!(
            format_today_list_at(&[], Local::now().naive_local()),
            "No events today\\."
        );
    }

    #[test]
    fn format_today_list_rows() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-16 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events = vec![sample_event("standup", Some(now + Duration::hours(1)))];
        let out = format_today_list_at(&events, now);
        assert!(out.starts_with("*Today's events:*\n"));
        assert!(out.contains("10:00 16\\.06\\.2026 \\(1h\\)"));
        assert!(out.contains("standup"));
    }

    #[test]
    fn format_tomorrow_list_empty() {
        assert_eq!(
            format_tomorrow_list_at(&[], Local::now().naive_local()),
            "No events tomorrow\\."
        );
    }

    #[test]
    fn format_tomorrow_list_rows() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-16 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events = vec![sample_event("dentist", Some(now + Duration::days(1)))];
        let out = format_tomorrow_list_at(&events, now);
        assert!(out.starts_with("*Tomorrow's events:*\n"));
        assert!(out.contains("09:00 17\\.06\\.2026 \\(1d\\)"));
        assert!(out.contains("dentist"));
    }

    #[test]
    fn format_month_list_empty() {
        assert_eq!(
            format_month_list_at(&[], Local::now().naive_local()),
            "No events this month\\."
        );
    }

    #[test]
    fn format_month_list_rows() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-16 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events = vec![sample_event("pay rent", Some(now + Duration::days(5)))];
        let out = format_month_list_at(&events, now);
        assert!(out.starts_with("*This month's events:*\n"));
        assert!(out.contains("09:00 21\\.06\\.2026"));
        assert!(out.contains("pay rent"));
    }

    #[test]
    fn format_week_list_empty() {
        assert_eq!(
            format_week_list_at(&[], Local::now().naive_local()),
            "No events this week\\."
        );
    }

    #[test]
    fn format_week_list_rows() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-16 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events = vec![sample_event("gym", Some(now + Duration::days(2)))];
        let out = format_week_list_at(&events, now);
        assert!(out.starts_with("*This week's events:*\n"));
        assert!(out.contains("09:00 18\\.06\\.2026 \\(2d\\)"));
        assert!(out.contains("gym"));
    }

    #[test]
    fn relative_time_units() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(at(now, Duration::seconds(30)), "now");
        assert_eq!(at(now, Duration::seconds(-5)), "now");
        assert_eq!(at(now, Duration::minutes(1)), "1 min");
        assert_eq!(at(now, Duration::minutes(13)), "13 mins");
        assert_eq!(at(now, Duration::hours(1)), "1h");
        assert_eq!(at(now, Duration::hours(23)), "23h");
        assert_eq!(at(now, Duration::days(2)), "2d");
        assert_eq!(at(now, Duration::days(7)), "1w");
        assert_eq!(at(now, Duration::days(21)), "3w");
    }
}
