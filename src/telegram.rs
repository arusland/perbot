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

/// Appends a single MarkdownV2 event row (`• datetime (relative) — message`).
fn write_event_row(out: &mut String, e: &EventInfo, now: NaiveDateTime) {
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

/// Number of events shown per page in a paginated list reply.
pub const LIST_PAGE_SIZE: usize = 10;

/// Total number of pages for `len` events at `per_page` events per page.
/// Always at least 1 so an empty list still renders one (empty) page.
pub fn total_pages(len: usize, per_page: usize) -> usize {
    len.div_ceil(per_page).max(1)
}

/// Builds the MarkdownV2 reply for a single page of an event list.
///
/// `title` is the bare heading (e.g. `"Upcoming events"`); a `(page x/y)` suffix
/// is appended only when there is more than one page. `empty` is the full message
/// shown when there are no events. Returns the rendered text and the total number
/// of pages, so the caller can decide whether to attach navigation buttons.
/// `page` is clamped to the valid range.
pub fn format_page(
    events: &[EventInfo],
    page: usize,
    per_page: usize,
    title: &str,
    empty: &str,
) -> (String, usize) {
    format_page_at(
        events,
        Local::now().naive_local(),
        page,
        per_page,
        title,
        empty,
    )
}

/// Like [`format_page`] but with an explicit `now` for relative-time tests.
pub fn format_page_at(
    events: &[EventInfo],
    now: NaiveDateTime,
    page: usize,
    per_page: usize,
    title: &str,
    empty: &str,
) -> (String, usize) {
    let pages = total_pages(events.len(), per_page);
    if events.is_empty() {
        return (empty.to_string(), pages);
    }
    let page = page.min(pages - 1);
    let start = page * per_page;
    let slice = &events[start..(start + per_page).min(events.len())];

    let mut out = if pages > 1 {
        format!("*{} \\(page {}/{}\\):*\n", title, page + 1, pages)
    } else {
        format!("*{}:*\n", title)
    };
    for e in slice {
        write_event_row(&mut out, e, now);
    }
    (out, pages)
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
    fn total_pages_counts() {
        assert_eq!(total_pages(0, 10), 1);
        assert_eq!(total_pages(10, 10), 1);
        assert_eq!(total_pages(11, 10), 2);
        assert_eq!(total_pages(25, 10), 3);
    }

    #[test]
    fn format_page_empty() {
        let (text, pages) = format_page_at(
            &[],
            Local::now().naive_local(),
            0,
            10,
            "Upcoming events",
            "No upcoming events\\.",
        );
        assert_eq!(text, "No upcoming events\\.");
        assert_eq!(pages, 1);
    }

    #[test]
    fn format_page_single_page_has_no_page_suffix() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events = vec![
            sample_event("call mom", Some(now + Duration::hours(2))),
            // Markdown special chars in the message must be escaped.
            sample_event("pay rent (urgent)", Some(now + Duration::days(3))),
        ];
        let (text, pages) = format_page_at(&events, now, 0, 10, "Upcoming events", "none");
        assert_eq!(pages, 1);
        assert!(text.starts_with("*Upcoming events:*\n"));
        assert!(text.contains("14:00 15\\.06\\.2026 \\(2h\\)"));
        assert!(text.contains("pay rent \\(urgent\\)"));
        assert!(text.contains("\\(3d\\)"));
    }

    #[test]
    fn format_page_uses_given_title() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-16 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events = vec![sample_event("standup", Some(now + Duration::hours(1)))];
        let (text, _) = format_page_at(&events, now, 0, 10, "Today's events", "none");
        assert!(text.starts_with("*Today's events:*\n"));
        assert!(text.contains("10:00 16\\.06\\.2026 \\(1h\\)"));
        assert!(text.contains("standup"));
    }

    #[test]
    fn format_page_slices_and_labels() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events: Vec<EventInfo> = (0..25)
            .map(|i| sample_event(&format!("event {i}"), Some(now + Duration::hours(i + 1))))
            .collect();

        // First page: 10 rows, labelled 1/3.
        let (p0, pages) = format_page_at(&events, now, 0, 10, "Upcoming events", "none");
        assert_eq!(pages, 3);
        assert!(p0.starts_with("*Upcoming events \\(page 1/3\\):*\n"));
        assert!(p0.contains("event 0"));
        assert!(p0.contains("event 9"));
        assert!(!p0.contains("event 10"));

        // Last page: only 5 rows, labelled 3/3. Out-of-range page clamps to last.
        let (p_last, _) = format_page_at(&events, now, 9, 10, "Upcoming events", "none");
        assert!(p_last.starts_with("*Upcoming events \\(page 3/3\\):*\n"));
        assert!(p_last.contains("event 20"));
        assert!(p_last.contains("event 24"));
        assert_eq!(p_last.lines().count(), 1 + 5);
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
