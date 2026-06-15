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
    if events.is_empty() {
        return "No upcoming events\\.".to_string();
    }
    let mut out = String::from("*Upcoming events:*\n");
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
