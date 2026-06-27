use crate::locale::LocaleProvider;
use crate::scheduler;
use crate::types::{ChatInfo, ChatType, EventInfo, MonthlyPattern};
use chrono::{Local, NaiveDateTime, Weekday};
use std::fmt::Write as _;
use teloxide::utils::html;

/// Maximum upcoming launches previewed for a reminder. A further `• ...` bullet
/// is shown when more launches follow.
const MAX_NEXT_PREVIEW: usize = 3;

/// Preview block of upcoming launches for a reminder, computed with
/// `scheduler::calc_next_at`. Lists up to MAX_NEXT_PREVIEW launches as bullets,
/// plus a trailing `• ...` when more remain. Returns "" for one-off events
/// (no future occurrence). `after` is the baseline (the launch being confirmed
/// or fired), used as both the search baseline and the relative-time origin, so
/// the listed launches are strictly after it. Output is an HTML fragment: the
/// `<b>Next launches:</b>` header is bold, the bullets and datetimes are plain
/// (no HTML specials), so callers embed it verbatim into their HTML output.
pub fn next_launches_preview(
    event: &EventInfo,
    after: NaiveDateTime,
    loc: &dyn LocaleProvider,
) -> String {
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
    let mut out = format!("\n\n<b>{}</b>", loc.next_launches_header());
    for dt in launches.iter().take(MAX_NEXT_PREVIEW) {
        out.push_str(&format!("\n• {}", format_when(after, *dt, loc)));
    }
    if launches.len() > MAX_NEXT_PREVIEW {
        out.push_str("\n• ...");
    }
    out
}

/// Confirmation sent when a reminder is scheduled (new parse or snooze).
/// HTML: the bolded title shows the absolute datetime plus the relative time
/// from `now` (e.g. `13:30 22.06.2026 (1d)`), followed by a
/// `Message: <event.message>` line. `event.message` is already an HTML fragment
/// (the user's formatting preserved), so it is embedded verbatim. For recurring
/// events a "Next launches" preview is appended; one-off events (empty preview)
/// render as just the title plus the message line.
pub fn scheduled_message(
    now: NaiveDateTime,
    dt: NaiveDateTime,
    event: &EventInfo,
    loc: &dyn LocaleProvider,
) -> String {
    let preview = next_launches_preview(event, dt, loc);
    format!(
        "Scheduled message for <b>{}</b>\nMessage: {}{}",
        html::escape(&format_when(now, dt, loc)),
        event.message,
        preview
    )
}

/// Short relative time until `dt` from `now`, delegated to the locale (e.g.
/// `13 mins`, `1h`, `2d`, `1.4y`).
fn format_relative(now: NaiveDateTime, dt: NaiveDateTime, loc: &dyn LocaleProvider) -> String {
    loc.format_relative((dt - now).num_seconds())
}

/// Plain-text "HH:MM dd.mm.yyyy (relative)" for a single datetime, e.g.
/// `14:00 23.06.2026 (1d)`. Unescaped — for the fired-reminder preview. List
/// replies use `write_event_row` (HTML) instead.
pub fn format_when(now: NaiveDateTime, dt: NaiveDateTime, loc: &dyn LocaleProvider) -> String {
    format!(
        "{} ({})",
        loc.format_datetime(dt),
        format_relative(now, dt, loc)
    )
}

/// Appends a single HTML event row (`• datetime (relative) — message`). The
/// datetime/relative parts contain no HTML specials; `e.message` is already an
/// HTML fragment, so it is embedded verbatim.
fn write_event_row(out: &mut String, e: &EventInfo, now: NaiveDateTime, loc: &dyn LocaleProvider) {
    let when = match e.next_datetime {
        Some(dt) => html::escape(&format_when(now, dt, loc)),
        None => "—".to_string(),
    };
    let _ = writeln!(out, "• {} — {}", when, e.message);
}

/// Max characters of message shown in the two-line `/events` row before it is
/// truncated with a trailing `...`.
const MESSAGE_PREVIEW_MAX: usize = 50;

/// Plain-text, newline-free rendering of an HTML message fragment: strips HTML
/// tags, unescapes the three specials `teloxide::utils::html::escape` emits
/// (`&amp; &lt; &gt;`), and collapses all whitespace (incl. newlines) to single
/// spaces. The result is plain text; callers targeting HTML must escape it.
fn html_to_plain(html_fragment: &str) -> String {
    // Strip tags: drop everything between '<' and the next '>'.
    let mut stripped = String::with_capacity(html_fragment.len());
    let mut in_tag = false;
    for c in html_fragment.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => stripped.push(c),
            _ => {}
        }
    }
    // Unescape: do `&lt;`/`&gt;` before `&amp;` so an escaped `&` is not turned
    // into the start of another entity.
    let unescaped = stripped
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    // Collapse all whitespace (incl. newlines) to single spaces; trim ends.
    unescaped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Plain-text, newline-free preview of an HTML message fragment, truncated to
/// `max` characters (chars, not bytes) with a trailing `...` when longer.
/// The result is plain text; callers targeting HTML must escape it.
fn message_preview(html_fragment: &str, max: usize) -> String {
    let collapsed = html_to_plain(html_fragment);
    // Truncate by char count for UTF-8 safety.
    if collapsed.chars().count() > max {
        let head: String = collapsed.chars().take(max).collect();
        format!("{head}...")
    } else {
        collapsed
    }
}

/// Telegram's hard message limit, in UTF-16 code units, measured on the rendered
/// text (after entities parsing — HTML tags don't count toward it).
pub const TELEGRAM_MAX_LEN: usize = 4096;

/// Headroom reserved below [`TELEGRAM_MAX_LEN`] for the bits the bot appends to a
/// user's body when it fires a reminder: the `Next launches:` preview (up to a
/// few launch lines + header) and the snooze hint, plus the one-off
/// confirmation's labels. The realistic worst case is ~150 units; 300 is a safe
/// margin so a clamped body never makes an outbound message exceed the limit.
const FIRED_EXTRAS_RESERVE: usize = 300;

/// Maximum rendered length (UTF-16 code units) of a user-supplied reminder body,
/// leaving [`FIRED_EXTRAS_RESERVE`] for the appended preview/hint.
pub const MESSAGE_MAX_LEN: usize = TELEGRAM_MAX_LEN - FIRED_EXTRAS_RESERVE;

/// Rendered length Telegram counts for an HTML fragment: its plain text (tags
/// stripped, specials unescaped) measured in UTF-16 code units — the same unit
/// the Bot API uses for the 4096-char limit.
pub fn rendered_len(html_fragment: &str) -> usize {
    html_to_plain(html_fragment).encode_utf16().count()
}

/// Clamps an HTML message fragment to [`MESSAGE_MAX_LEN`] rendered UTF-16 units.
/// Returns `(fragment, false)` unchanged when it already fits; otherwise returns
/// `(escaped_truncated_plain_text, true)`. Over-limit truncation falls back to
/// the plain (un-formatted) text re-escaped, which is always valid HTML — losing
/// formatting only in this rare case, where the caller warns the user anyway.
pub fn clamp_message(html_fragment: &str) -> (String, bool) {
    if rendered_len(html_fragment) <= MESSAGE_MAX_LEN {
        return (html_fragment.to_owned(), false);
    }
    let plain = html_to_plain(html_fragment);
    // Truncate by whole chars, accumulating UTF-16 widths so a surrogate pair is
    // never split and the head never exceeds the cap.
    let mut head = String::with_capacity(plain.len());
    let mut units = 0usize;
    for c in plain.chars() {
        let w = c.len_utf16();
        if units + w > MESSAGE_MAX_LEN {
            break;
        }
        head.push(c);
        units += w;
    }
    (html::escape(&head), true)
}

/// Reconstructs the re-parseable plain-text input for an event: its canonical time
/// expression ([`EventInfo::normalize_time`]) followed by the plain-text message.
/// Parsing the result yields the same event, so it can be offered as a copyable
/// starting point for editing. Plain text; callers targeting HTML must escape it.
pub fn event_source_input(event: &EventInfo, loc: &dyn LocaleProvider) -> String {
    let time = event.normalize_time(loc);
    let message = html_to_plain(&event.message);
    if message.is_empty() {
        time
    } else {
        format!("{time} {message}")
    }
}

/// Builds the HTML edit prompt: the `lead` line followed by the event's current
/// input wrapped in a `<code>` block, which Telegram clients copy on tap. Both
/// parts are HTML-escaped.
pub fn edit_prompt(lead: &str, event: &EventInfo, loc: &dyn LocaleProvider) -> String {
    format!(
        "{}\n\n<code>{}</code>",
        html::escape(lead),
        html::escape(&event_source_input(event, loc))
    )
}

/// Human-readable recurrence period for an event, e.g. `"every 2 days"`,
/// `"every friday"`, `"every first sunday"`, `"last day of the month"`. Returns
/// `None` for one-off events (no recurrence). The recurrence-bearing fields are
/// mutually exclusive, checked in priority order. Output is plain text with no
/// HTML specials.
fn describe_recurrence(e: &EventInfo, loc: &dyn LocaleProvider) -> Option<String> {
    let every = loc.every_word();
    if let Some(rep) = &e.repetition {
        let unit = loc.unit_label(rep.unit, rep.interval != 1);
        return Some(if rep.interval == 1 {
            format!("{every} {unit}")
        } else {
            format!("{every} {} {unit}", rep.interval)
        });
    }
    if let Some(days) = &e.days {
        let mut list: Vec<Weekday> = days.iter().copied().collect();
        list.sort_by_key(|d| d.num_days_from_monday());
        let names = list
            .iter()
            .map(|d| loc.weekday_full(*d))
            .collect::<Vec<_>>()
            .join(", ");
        return Some(format!("{every} {names}"));
    }
    if let Some(pattern) = &e.monthly_pattern {
        return Some(match pattern {
            MonthlyPattern::OrdinalWeekday(ord, wd) => {
                format!(
                    "{every} {} {}",
                    loc.ordinal_word(*ord),
                    loc.weekday_full(*wd)
                )
            }
            MonthlyPattern::LastDay => loc.last_day_phrase().to_string(),
            MonthlyPattern::DayOfMonth(d) => loc.day_of_month_recurrence(&loc.ordinal_suffix(*d)),
        });
    }
    None
}

/// The bold datetime/relative line shared by the `/events` two-line row and the
/// single-event detail view: `• <b>HH:MM dd.mm.yyyy (in <rel>[, <recurrence>])</b>`.
/// `, <recurrence>` is appended inside the parentheses, next to the relative time,
/// when the event repeats. Datetime/relative/recurrence parts contain no HTML
/// specials. Returns `• <b>—</b>` for an event with no upcoming launch.
fn event_when_line(e: &EventInfo, now: NaiveDateTime, loc: &dyn LocaleProvider) -> String {
    let recurrence = describe_recurrence(e, loc)
        .map(|r| format!(", {r}"))
        .unwrap_or_default();
    let when = match e.next_datetime {
        Some(dt) => html::escape(&format!(
            "{} (in {}{})",
            loc.format_datetime(dt),
            format_relative(now, dt, loc),
            recurrence
        )),
        None => "—".to_string(),
    };
    format!("• <b>{when}</b>")
}

/// Appends a two-line HTML event row used by `/events`: the bold datetime/relative
/// line ([`event_when_line`]) ending with a tappable `/event<id>` link that opens
/// the single-event detail view, then an indented plain-text message preview (tags
/// stripped, truncated; HTML-escaped).
fn write_event_row_two_line(
    out: &mut String,
    e: &EventInfo,
    now: NaiveDateTime,
    loc: &dyn LocaleProvider,
) {
    let message = html::escape(&message_preview(&e.message, MESSAGE_PREVIEW_MAX));
    let _ = writeln!(
        out,
        "{} /event{}\n  {message}",
        event_when_line(e, now, loc),
        e.id
    );
}

/// Detailed single-event view for `/event<id>`: the same bold datetime/recurrence
/// line as the `/events` two-line row ([`event_when_line`]), the full HTML message
/// fragment (formatting preserved, not truncated), and the upcoming-launches preview
/// ([`next_launches_preview`], identical to a fired reminder). The preview is empty
/// for one-off events. An **inactive** (out-of-date) event instead renders a single
/// bold notice — "Event is out of date. Last fired at <last_next_datetime>" — above
/// the message body.
pub fn event_detail(event: &EventInfo, now: NaiveDateTime, loc: &dyn LocaleProvider) -> String {
    if !event.active {
        let notice = match event.last_next_datetime {
            Some(dt) => html::escape(&format!(
                "Event is out of date. Last fired at {}",
                loc.format_datetime(dt)
            )),
            None => "Event is out of date.".to_string(),
        };
        return format!("• <b>{notice}</b>\n{}", event.message);
    }
    let preview = match event.next_datetime {
        Some(dt) => next_launches_preview(event, dt, loc),
        None => String::new(),
    };
    format!(
        "{}\n{}{}",
        event_when_line(event, now, loc),
        event.message,
        preview
    )
}

/// Number of events shown per page in a paginated list reply.
pub const LIST_PAGE_SIZE: usize = 10;

/// Total number of pages for `len` events at `per_page` events per page.
/// Always at least 1 so an empty list still renders one (empty) page.
pub fn total_pages(len: usize, per_page: usize) -> usize {
    len.div_ceil(per_page).max(1)
}

/// Builds the HTML reply for a single page of an event list.
///
/// `title` is the bare heading (e.g. `"Upcoming events"`), rendered as-is; the
/// page position is surfaced by the navigation keyboard's indicator button, not in
/// the title. `empty` is the full message shown when there are no events. Returns
/// the rendered text and the total number of pages, so the caller can decide
/// whether to attach navigation buttons.
/// `page` is clamped to the valid range. When `two_line` is true (used by
/// `/events`), each event renders as a datetime line plus an indented plain-text
/// message preview; otherwise as the single-line HTML row.
pub fn format_page(
    events: &[EventInfo],
    page: usize,
    per_page: usize,
    title: &str,
    empty: &str,
    two_line: bool,
    loc: &dyn LocaleProvider,
) -> (String, usize) {
    format_page_at(
        events,
        Local::now().naive_local(),
        page,
        per_page,
        title,
        empty,
        two_line,
        loc,
    )
}

/// Like [`format_page`] but with an explicit `now` for relative-time tests.
#[allow(clippy::too_many_arguments)]
pub fn format_page_at(
    events: &[EventInfo],
    now: NaiveDateTime,
    page: usize,
    per_page: usize,
    title: &str,
    empty: &str,
    two_line: bool,
    loc: &dyn LocaleProvider,
) -> (String, usize) {
    let pages = total_pages(events.len(), per_page);
    if events.is_empty() {
        return (empty.to_string(), pages);
    }
    let page = page.min(pages - 1);
    let start = page * per_page;
    let slice = &events[start..(start + per_page).min(events.len())];

    let mut out = format!("<b>{}:</b>\n", html::escape(title));
    for e in slice {
        if two_line {
            write_event_row_two_line(&mut out, e, now, loc);
        } else {
            write_event_row(&mut out, e, now, loc);
        }
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
    use crate::locale::EN;
    use crate::types::EventInfo;
    use chrono::{Duration, NaiveDate, NaiveTime};

    fn at(now: NaiveDateTime, d: Duration) -> String {
        format_relative(now, now + d, &EN)
    }

    #[test]
    fn rendered_len_ignores_tags_and_counts_utf16() {
        // Tags don't count; only the two rendered letters do.
        assert_eq!(rendered_len("<b>hi</b>"), 2);
        // An astral-plane emoji is one char but two UTF-16 code units.
        assert_eq!(rendered_len("😀"), 2);
        // Escaped specials render back to a single visible char.
        assert_eq!(rendered_len("a &amp; b"), 5);
    }

    #[test]
    fn clamp_message_leaves_short_fragment_unchanged() {
        let (out, truncated) = clamp_message("<b>hi</b> there");
        assert_eq!(out, "<b>hi</b> there");
        assert!(!truncated);
    }

    #[test]
    fn clamp_message_truncates_over_limit_to_valid_html() {
        let long = "a".repeat(MESSAGE_MAX_LEN + 50);
        let (out, truncated) = clamp_message(&long);
        assert!(truncated);
        assert_eq!(rendered_len(&out), MESSAGE_MAX_LEN);
        // Escaped plain text carries no dangling tag.
        assert!(!out.contains('<'));
    }

    #[test]
    fn clamp_message_never_splits_a_surrogate_pair() {
        // A run of astral-plane emoji (2 UTF-16 units each) overruns the cap;
        // the head must stop on a whole-emoji boundary and never exceed it.
        let long = "😀".repeat(MESSAGE_MAX_LEN);
        let (out, truncated) = clamp_message(&long);
        assert!(truncated);
        assert!(rendered_len(&out) <= MESSAGE_MAX_LEN);
        // MESSAGE_MAX_LEN is even, so an exact fill of 2-unit chars lands on it.
        assert_eq!(rendered_len(&out), MESSAGE_MAX_LEN);
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
            last_next_datetime: next,
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
        // with the relative time (1d) appended.
        let event = sample_event("ring in the new year", Some(dt));
        assert_eq!(
            scheduled_message(now, dt, &event, &EN),
            "Scheduled message for <b>13:05 31.12.2027 (1d)</b>\nMessage: ring in the new year"
        );
    }

    #[test]
    fn scheduled_message_embeds_html_message_verbatim() {
        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        );
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        // `message` is already an HTML fragment; it is embedded as-is.
        let event = sample_event("<b>call</b> the office", Some(dt));
        assert_eq!(
            scheduled_message(now, dt, &event, &EN),
            "Scheduled message for <b>10:00 22.06.2026 (1h)</b>\nMessage: <b>call</b> the office"
        );
    }

    #[test]
    fn scheduled_message_appends_preview_for_recurring() {
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

        let text = scheduled_message(now, dt, &event, &EN);
        assert!(text.starts_with("Scheduled message for <b>10:00 22.06.2026 (1h)</b>"));
        assert!(text.contains("Message: standup"));
        // Preview lists launches strictly after the confirmed datetime.
        assert!(text.contains("<b>Next launches:</b>"));
        assert!(text.contains("• 10:00 23.06.2026"));
        assert!(text.contains("• ..."));
    }

    #[test]
    fn next_launches_preview_one_off_is_empty() {
        let fire = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let mut event = sample_event("call mom", Some(fire));
        event.time = NaiveTime::from_hms_opt(10, 0, 0);
        assert_eq!(next_launches_preview(&event, fire, &EN), "");
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

        let preview = next_launches_preview(&event, fire, &EN);
        assert!(preview.starts_with("\n\n<b>Next launches:</b>"));
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

        let preview = next_launches_preview(&event, fire, &EN);
        assert!(preview.starts_with("\n\n<b>Next launches:</b>"));
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
            "No upcoming events.",
            false,
            &EN,
        );
        assert_eq!(text, "No upcoming events.");
        assert_eq!(pages, 1);
    }

    #[test]
    fn format_page_single_page_has_no_page_suffix() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events = vec![
            sample_event("call mom", Some(now + Duration::hours(2))),
            // `message` is an HTML fragment, embedded verbatim (parens are not
            // HTML specials).
            sample_event("pay rent (urgent)", Some(now + Duration::days(3))),
        ];
        let (text, pages) =
            format_page_at(&events, now, 0, 10, "Upcoming events", "none", false, &EN);
        assert_eq!(pages, 1);
        assert!(text.starts_with("<b>Upcoming events:</b>\n"));
        assert!(text.contains("14:00 15.06.2026 (2h)"));
        assert!(text.contains("pay rent (urgent)"));
        assert!(text.contains("(3d)"));
    }

    #[test]
    fn format_page_uses_given_title() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-16 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events = vec![sample_event("standup", Some(now + Duration::hours(1)))];
        let (text, _) = format_page_at(&events, now, 0, 10, "Today's events", "none", false, &EN);
        assert!(text.starts_with("<b>Today's events:</b>\n"));
        assert!(text.contains("10:00 16.06.2026 (1h)"));
        assert!(text.contains("standup"));
    }

    #[test]
    fn format_page_slices_and_labels() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events: Vec<EventInfo> = (0..25)
            .map(|i| sample_event(&format!("event {i}"), Some(now + Duration::hours(i + 1))))
            .collect();

        // First page: 10 rows. Page position lives on the keyboard, not the title.
        let (p0, pages) =
            format_page_at(&events, now, 0, 10, "Upcoming events", "none", false, &EN);
        assert_eq!(pages, 3);
        assert!(p0.starts_with("<b>Upcoming events:</b>\n"));
        assert!(p0.contains("event 0"));
        assert!(p0.contains("event 9"));
        assert!(!p0.contains("event 10"));

        // Last page: only 5 rows. Out-of-range page clamps to last.
        let (p_last, _) =
            format_page_at(&events, now, 9, 10, "Upcoming events", "none", false, &EN);
        assert!(p_last.starts_with("<b>Upcoming events:</b>\n"));
        assert!(p_last.contains("event 20"));
        assert!(p_last.contains("event 24"));
        assert_eq!(p_last.lines().count(), 1 + 5);
    }

    #[test]
    fn message_preview_strips_tags_and_unescapes() {
        assert_eq!(
            message_preview("<b>call</b> the office", 50),
            "call the office"
        );
        assert_eq!(message_preview("<a href=\"x\">site</a>", 50), "site");
        assert_eq!(message_preview("a &amp; b", 50), "a & b");
        assert_eq!(message_preview("&lt;tag&gt;", 50), "<tag>");
    }

    #[test]
    fn message_preview_removes_newlines() {
        assert_eq!(message_preview("line1\nline2", 50), "line1 line2");
        assert_eq!(message_preview("a\n\n  b\tc", 50), "a b c");
    }

    #[test]
    fn message_preview_truncates_by_chars() {
        // 30 chars -> first 20 + "...".
        let msg = "abcdefghijklmnopqrstuvwxyz1234";
        assert_eq!(message_preview(msg, 20), "abcdefghijklmnopqrst...");
        // Short message left intact.
        assert_eq!(message_preview("short", 20), "short");
        // Exactly 20 chars: no ellipsis.
        assert_eq!(
            message_preview("01234567890123456789", 20),
            "01234567890123456789"
        );
    }

    #[test]
    fn message_preview_truncation_is_utf8_safe() {
        // 21 multi-byte chars; truncating by bytes would panic, by chars is fine.
        let msg = "ñññññññññññññññññññññ";
        let out = message_preview(msg, 20);
        assert_eq!(out.chars().count(), 23); // 20 + "..."
        assert!(out.ends_with("..."));
    }

    #[test]
    fn event_source_input_reconstructs_reparseable_text() {
        use crate::parser;

        // Clock time + message.
        let mut e = sample_event("call the office", None);
        e.time = Some(NaiveTime::from_hms_opt(13, 30, 0).unwrap());
        assert_eq!(event_source_input(&e, &EN), "13:30 call the office");
        // Round-trips: parsing yields the same time/message.
        let parsed = parser::parse(&event_source_input(&e, &EN), &EN).unwrap();
        assert_eq!(parsed.time, e.time);
        assert_eq!(parsed.message, "call the office");

        // Recurrence (weekday set) is included via normalize_time.
        let mut r = sample_event("standup", None);
        r.time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        r.days = Some(std::collections::HashSet::from([
            chrono::Weekday::Mon,
            chrono::Weekday::Tue,
            chrono::Weekday::Wed,
            chrono::Weekday::Thu,
            chrono::Weekday::Fri,
        ]));
        assert_eq!(event_source_input(&r, &EN), "09:00 Mon-Fri standup");

        // HTML formatting in the message is reduced to plain text.
        let mut b = sample_event("<b>call</b> her", None);
        b.time = Some(NaiveTime::from_hms_opt(8, 0, 0).unwrap());
        assert_eq!(event_source_input(&b, &EN), "08:00 call her");
    }

    #[test]
    fn edit_prompt_wraps_input_in_code_and_escapes() {
        let mut e = sample_event("a &amp; b &lt;c&gt;", None);
        e.time = Some(NaiveTime::from_hms_opt(10, 0, 0).unwrap());
        let prompt = edit_prompt("Edit:", &e, &EN);
        // The copyable block is a <code> span...
        assert!(prompt.contains("<code>"));
        assert!(prompt.contains("</code>"));
        // ...holding the plain input with HTML specials re-escaped for HTML output.
        assert!(prompt.contains("<code>10:00 a &amp; b &lt;c&gt;</code>"));
        assert!(prompt.starts_with("Edit:\n\n"));
    }

    #[test]
    fn format_page_two_line_layout() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        // Message longer than MESSAGE_PREVIEW_MAX (50) to exercise truncation.
        let events = vec![sample_event(
            "<b>call</b> the office right now please and bring the documents",
            Some(now + Duration::hours(2)),
        )];
        let (text, _) = format_page_at(&events, now, 0, 10, "Upcoming events", "none", true, &EN);
        assert!(text.starts_with("<b>Upcoming events:</b>\n"));
        // Bold datetime line and message live on separate lines; no `—` separator.
        assert!(!text.contains(" — "));
        // Plain text, tag-free, truncated to MESSAGE_PREVIEW_MAX chars + "...".
        assert!(text.contains("  call the office right now please and bring the doc..."));
        // One-off event: no recurrence suffix on the datetime line.
        assert!(!text.contains(", every"));
        // The /event<id> link ends the bold datetime line (id 0 for sample events).
        assert!(text.contains("• <b>14:00 15.06.2026 (in 2h)</b> /event0\n"));
    }

    #[test]
    fn describe_recurrence_variants() {
        use crate::types::{Ordinal, Repetition, TimeUnit};
        use std::collections::HashSet;

        let mut e = sample_event("x", None);
        // One-off → no recurrence.
        assert_eq!(describe_recurrence(&e, &EN), None);

        // Interval repetition: plural and singular (n == 1).
        e.repetition = Some(Repetition {
            interval: 2,
            unit: TimeUnit::Days,
        });
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("every 2 days")
        );
        e.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Hours,
        });
        assert_eq!(describe_recurrence(&e, &EN).as_deref(), Some("every hour"));
        e.repetition = None;

        // Single weekday, then a sorted multi-day set (Mon before Fri).
        e.days = Some(HashSet::from([Weekday::Fri]));
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("every Friday")
        );
        e.days = Some(HashSet::from([Weekday::Fri, Weekday::Mon]));
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("every Monday, Friday")
        );
        e.days = None;

        // Monthly patterns.
        e.monthly_pattern = Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Sun));
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("every first Sunday")
        );
        e.monthly_pattern = Some(MonthlyPattern::LastDay);
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("last day of the month")
        );
        e.monthly_pattern = Some(MonthlyPattern::DayOfMonth(28));
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("28th day of the month")
        );
    }

    #[test]
    fn format_page_two_line_appends_recurrence() {
        use crate::types::{Repetition, TimeUnit};
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let mut e = sample_event("standup", Some(now + Duration::hours(2)));
        e.repetition = Some(Repetition {
            interval: 2,
            unit: TimeUnit::Days,
        });
        let (text, _) = format_page_at(&[e], now, 0, 10, "Upcoming events", "none", true, &EN);
        // Recurrence sits inside the parentheses, next to the relative time; the
        // /event<id> link ends the line.
        assert!(text.contains("• <b>14:00 15.06.2026 (in 2h, every 2 days)</b> /event0\n"));
    }

    #[test]
    fn event_detail_one_off_has_no_launches_block() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let e = sample_event(
            "<b>call</b> the office and bring the documents",
            Some(now + Duration::hours(2)),
        );
        let text = event_detail(&e, now, &EN);
        // Bold when-line, then the full untruncated HTML message verbatim.
        assert!(text.starts_with("• <b>14:00 15.06.2026 (in 2h)</b>\n"));
        assert!(text.contains("<b>call</b> the office and bring the documents"));
        // One-off: no upcoming-launches block.
        assert!(!text.contains("Next launches:"));
    }

    #[test]
    fn event_detail_recurring_shows_launches_block() {
        use crate::types::{Repetition, TimeUnit};
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let dt = now + Duration::hours(2);
        let mut e = sample_event("standup", Some(dt));
        e.time = Some(dt.time());
        e.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Days,
        });
        let text = event_detail(&e, now, &EN);
        assert!(text.starts_with("• <b>14:00 15.06.2026 (in 2h, every day)</b>\n"));
        assert!(text.contains("standup"));
        // Recurring: launches block present, listing dates after the upcoming one.
        assert!(text.contains("<b>Next launches:</b>"));
        assert!(text.contains("• 14:00 16.06.2026"));
    }

    #[test]
    fn event_detail_inactive_shows_last_fired_notice() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let mut e = sample_event("expired reminder", None);
        e.last_next_datetime = Some(
            NaiveDateTime::parse_from_str("2026-06-10 09:30:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        );
        let text = event_detail(&e, now, &EN);
        assert!(
            text.starts_with("• <b>Event is out of date. Last fired at 09:30 10.06.2026</b>\n")
        );
        assert!(text.contains("expired reminder"));
        // Inactive: no when-line relative time, no launches block.
        assert!(!text.contains("Next launches:"));
    }

    #[test]
    fn relative_time_units() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(at(now, Duration::seconds(30)), "soon");
        assert_eq!(at(now, Duration::seconds(-5)), "soon");
        assert_eq!(at(now, Duration::minutes(1)), "1 min");
        assert_eq!(at(now, Duration::minutes(13)), "13 mins");
        assert_eq!(at(now, Duration::hours(1)), "1h");
        assert_eq!(at(now, Duration::hours(23)), "23h");
        assert_eq!(at(now, Duration::days(2)), "2d");
        assert_eq!(at(now, Duration::days(7)), "1w");
        assert_eq!(at(now, Duration::days(21)), "3w");
        assert_eq!(at(now, Duration::days(51 * 7)), "51w"); // just under a year
        assert_eq!(at(now, Duration::days(52 * 7)), "1y"); // 364 days
        assert_eq!(at(now, Duration::days(511)), "1.4y");
        assert_eq!(at(now, Duration::days(693)), "1.9y");
        assert_eq!(at(now, Duration::days(104 * 7)), "2y"); // 728 days
    }
}
