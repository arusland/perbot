use crate::scheduler;
use crate::types::{ChatInfo, ChatType, EventInfo};
use chrono::{Local, NaiveDateTime};
use std::fmt::Write as _;

/// Maximum upcoming launches previewed for a reminder. A further `• ...` bullet
/// is shown when more launches follow.
const MAX_NEXT_PREVIEW: usize = 3;

/// Preview block of upcoming launches for a reminder, computed with
/// `scheduler::calc_next_at`. Lists up to MAX_NEXT_PREVIEW launches as bullets,
/// plus a trailing `• ...` when more remain. Returns "" for one-off events
/// (no future occurrence). `after` is the baseline (the launch being confirmed
/// or fired), used as both the search baseline and the relative-time origin, so
/// the listed launches are strictly after it. Output is plain text; callers
/// targeting MarkdownV2 escape it with `escape_markdown`.
pub fn next_launches_preview(event: &EventInfo, after: NaiveDateTime) -> String {
    let mut launches: Vec<NaiveDateTime> = Vec::new();
    let mut current = event.clone();
    let mut cursor = after;
    // Probe one beyond the limit so we know whether to show the "..." bullet.
    while launches.len() <= MAX_NEXT_PREVIEW {
        current = scheduler::calc_next_at(current, cursor);
        match current.next_datetime {
            Some(next) => {
                launches.push(next);
                cursor = next;
            }
            None => break,
        }
    }
    if launches.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nNext launches:");
    for dt in launches.iter().take(MAX_NEXT_PREVIEW) {
        out.push_str(&format!("\n• {}", format_when(after, *dt)));
    }
    if launches.len() > MAX_NEXT_PREVIEW {
        out.push_str("\n• ...");
    }
    out
}

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

/// Confirmation sent when a reminder is scheduled (new parse or snooze).
/// MarkdownV2: the bolded title shows the absolute datetime plus the relative
/// time from `now` (e.g. `13:30 22.06.2026 (1d)`), escaped. For recurring events
/// a "Next launches" preview is appended (escaped); one-off events (empty
/// preview) render as just the title.
pub fn scheduled_message(now: NaiveDateTime, dt: NaiveDateTime, event: &EventInfo) -> String {
    let preview = next_launches_preview(event, dt);
    format!(
        "Scheduled message for *{}*{}",
        escape_markdown(&format_when(now, dt)),
        escape_markdown(&preview)
    )
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

/// Plain-text "HH:MM dd.mm.yyyy (relative)" for a single datetime, e.g.
/// `14:00 23.06.2026 (1d)`. Unescaped — for plain-text messages such as fired
/// reminders. List replies use `write_event_row` (MarkdownV2) instead.
pub fn format_when(now: NaiveDateTime, dt: NaiveDateTime) -> String {
    format!(
        "{} ({})",
        dt.format("%H:%M %d.%m.%Y"),
        format_relative(now, dt)
    )
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
            snoozed: false,
        }
    }

    #[test]
    fn scheduled_message_formats_datetime() {
        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 30).unwrap(),
            NaiveTime::from_hms_opt(13, 5, 0).unwrap(),
        );
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(13, 5, 0).unwrap(),
        );
        // A one-off event has no upcoming launches, so only the title is shown,
        // with the relative time (1d) appended and escaped.
        let event = sample_event("ring in the new year", Some(dt));
        assert_eq!(
            scheduled_message(now, dt, &event),
            "Scheduled message for *13:05 31\\.12\\.2027 \\(1d\\)*"
        );
    }

    #[test]
    fn scheduled_message_appends_escaped_preview_for_recurring() {
        use crate::types::{Repetition, TimeUnit};
        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        );
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let mut event = sample_event("standup", Some(dt));
        event.time = NaiveTime::from_hms_opt(10, 0, 0);
        event.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Days,
        });

        let text = scheduled_message(now, dt, &event);
        assert!(text.starts_with("Scheduled message for *10:00 22\\.06\\.2026 \\(1h\\)*"));
        // Preview lists launches strictly after the confirmed datetime, escaped.
        assert!(text.contains("Next launches:"));
        assert!(text.contains("• 10:00 23\\.06\\.2026"));
        assert!(text.contains("• \\.\\.\\."));
    }

    #[test]
    fn next_launches_preview_one_off_is_empty() {
        let fire = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let mut event = sample_event("call mom", Some(fire));
        event.time = NaiveTime::from_hms_opt(10, 0, 0);
        assert_eq!(next_launches_preview(&event, fire), "");
    }

    #[test]
    fn next_launches_preview_recurring_shows_three_plus_ellipsis() {
        use crate::types::{Repetition, TimeUnit};
        let fire = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let mut event = sample_event("standup", Some(fire));
        event.time = NaiveTime::from_hms_opt(10, 0, 0);
        event.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Days,
        });

        let preview = next_launches_preview(&event, fire);
        assert!(preview.starts_with("\n\nNext launches:"));
        // Three consecutive days after the firing day, then the overflow bullet.
        assert!(preview.contains("• 10:00 23.06.2026"));
        assert!(preview.contains("• 10:00 24.06.2026"));
        assert!(preview.contains("• 10:00 25.06.2026"));
        assert!(preview.contains("• ..."));
        assert_eq!(preview.matches('•').count(), 4);
    }

    #[test]
    fn next_launches_preview_fewer_than_three_has_no_ellipsis() {
        use std::collections::HashSet;
        // Year-restricted to 2027; firing on its second-to-last day leaves a single
        // future launch (2027-12-31 23:00) before the schedule is exhausted.
        let fire = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 30).unwrap(),
            NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
        );
        let mut event = sample_event("year end", Some(fire));
        event.time = NaiveTime::from_hms_opt(23, 0, 0);
        event.years = Some(HashSet::from([2027]));

        let preview = next_launches_preview(&event, fire);
        assert!(preview.starts_with("\n\nNext launches:"));
        assert!(preview.contains("• 23:00 31.12.2027"));
        assert!(!preview.contains("• ..."));
        assert_eq!(preview.matches('•').count(), 1);
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
