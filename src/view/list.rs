//! Paginated list rendering: per-row layouts ([`RowStyle`]), page assembly
//! ([`format_page_at`]), the navigation keyboard ([`list_keyboard`]), and
//! [`ListKind`] — each list's tag, title, empty text, and row style.

use super::event::event_when_line;
use super::message::{BULLET, MESSAGE_PREVIEW_MAX, message_preview};
use crate::locale::LocaleProvider;
use crate::types::EventInfo;
use chrono::NaiveDateTime;
use chrono_tz::Tz;
use std::fmt::Write as _;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use teloxide::utils::html;

/// Callback data for non-interactive buttons (e.g. the page indicator). It has
/// no `:`-prefixed envelope and matches no list tag, so `main`'s router hands it
/// to `handle_list_callback`, which answers the query and ignores it.
pub(super) const NOOP_DATA: &str = "noop";

/// The paginated list commands. Each variant knows how to title its reply, tag
/// its inline-button callbacks (`<tag>:<page>`), and lay out its rows; fetching
/// the events stays with `commands::list` (`fetch_page`).
///
/// `Missed` is not a user-typed command: it is reached only by the startup
/// missed-events send and its `ms:<page>` page-turn callbacks. Its events come
/// from the `missed_events` snapshot table populated at startup (see
/// `EventProvider::get_missed_snapshot_events`).
#[derive(Clone, Copy)]
pub enum ListKind {
    Events,
    Today,
    Tomorrow,
    Week,
    Month,
    Missed,
}

impl ListKind {
    /// Short tag used as the callback-data prefix (`<tag>:<page>`).
    pub fn tag(self) -> &'static str {
        match self {
            ListKind::Events => "ev",
            ListKind::Today => "td",
            ListKind::Tomorrow => "tm",
            ListKind::Week => "wk",
            ListKind::Month => "mo",
            ListKind::Missed => "ms",
        }
    }

    /// Parses a callback-data tag back into its kind.
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "ev" => Some(ListKind::Events),
            "td" => Some(ListKind::Today),
            "tm" => Some(ListKind::Tomorrow),
            "wk" => Some(ListKind::Week),
            "mo" => Some(ListKind::Month),
            "ms" => Some(ListKind::Missed),
            _ => None,
        }
    }

    /// Bare heading (no markup); `format_page_at` wraps it in `<b>…:</b>`.
    pub fn title(self) -> &'static str {
        match self {
            ListKind::Events => "Upcoming events",
            ListKind::Today => "Today's events",
            ListKind::Tomorrow => "Tomorrow's events",
            ListKind::Week => "This week's events",
            ListKind::Month => "This month's events",
            ListKind::Missed => "Missed events",
        }
    }

    /// Message shown when the list is empty (plain text, HTML-safe).
    pub fn empty(self) -> &'static str {
        match self {
            ListKind::Events => "No upcoming events.",
            ListKind::Today => "No events today.",
            ListKind::Tomorrow => "No events tomorrow.",
            ListKind::Week => "No events this week.",
            ListKind::Month => "No events this month.",
            ListKind::Missed => "No missed events.",
        }
    }

    /// Per-row layout for this list. Every user-typed list uses the two-line
    /// row; the missed list shows the missed datetime + plain preview +
    /// `/event<id>` link.
    pub fn row_style(self) -> RowStyle {
        match self {
            ListKind::Missed => RowStyle::PreviewLink,
            ListKind::Events
            | ListKind::Today
            | ListKind::Tomorrow
            | ListKind::Week
            | ListKind::Month => RowStyle::TwoLine,
        }
    }
}

/// How each event renders in a paginated list row.
#[derive(Clone, Copy)]
pub enum RowStyle {
    /// Bold datetime/recurrence line + `/event<id>` link, then an indented plain
    /// preview underneath (used by every user-typed list command).
    TwoLine,
    /// `▪ datetime — <plain preview> /event<id>` — the datetime the event should
    /// have fired at, absolute only (no relative part: it lies in the past), then
    /// the preview and the tappable link (used by the missed events list, whose
    /// snapshot rows carry the missed moment in `next_datetime`).
    PreviewLink,
}

/// Appends a missed-list HTML event row: `▪ datetime — <plain preview>
/// /event<id>` — the datetime the event should have fired at (absolute only;
/// a relative part would render "soon" for past moments), then the truncated,
/// tag-stripped, HTML-escaped message preview and the tappable `/event<id>`
/// link (see [`RowStyle::PreviewLink`]).
fn write_event_row_preview_only(out: &mut String, e: &EventInfo, tz: Tz, loc: &dyn LocaleProvider) {
    let message = html::escape(&message_preview(&e.message, MESSAGE_PREVIEW_MAX));
    match e.next_datetime {
        Some(dt) => {
            let when = html::escape(&loc.format_datetime(crate::tz::to_local(dt, tz)));
            let _ = writeln!(out, "{BULLET} {when} — {message} /event{}", e.id);
        }
        None => {
            let _ = writeln!(out, "{BULLET} {message} /event{}", e.id);
        }
    }
}

/// Appends a two-line HTML event row used by `/events`: the bold datetime/relative
/// line ([`event_when_line`]) ending with a tappable `/event<id>` link that opens
/// the single-event detail view, then an indented plain-text message preview (tags
/// stripped, truncated; HTML-escaped).
fn write_event_row_two_line(
    out: &mut String,
    e: &EventInfo,
    now: NaiveDateTime,
    tz: Tz,
    loc: &dyn LocaleProvider,
) {
    let message = html::escape(&message_preview(&e.message, MESSAGE_PREVIEW_MAX));
    let _ = writeln!(
        out,
        "{} /event{}\n  {message}",
        event_when_line(e, now, tz, loc),
        e.id
    );
}

/// Number of events shown per page in a paginated list reply.
pub const LIST_PAGE_SIZE: usize = 10;

/// Total number of pages for `len` events at `per_page` events per page.
/// Always at least 1 so an empty list still renders one (empty) page.
pub fn total_pages(len: usize, per_page: usize) -> usize {
    len.div_ceil(per_page).max(1)
}

/// Builds the HTML reply for one already-fetched page of an event list.
///
/// `page_events` holds only the rows of the page being rendered — the caller
/// pages at the storage layer (SQL `LIMIT`/`OFFSET`), so a large list never
/// loads whole. `total` is the list's full length, used to derive the page
/// count. `title` is the bare heading (e.g. `"Upcoming events"`), rendered
/// as-is; the page position is surfaced by the navigation keyboard's indicator
/// button, not in the title. `empty` is the full message shown when the page
/// has no events. Returns the rendered text and the total number of pages, so
/// the caller can decide whether to attach navigation buttons. `style` selects
/// the per-row layout (see [`RowStyle`]); rows are separated by a blank line.
#[allow(clippy::too_many_arguments)]
pub fn format_page_at(
    page_events: &[EventInfo],
    total: usize,
    now: NaiveDateTime,
    tz: Tz,
    per_page: usize,
    title: &str,
    empty: &str,
    style: RowStyle,
    loc: &dyn LocaleProvider,
) -> (String, usize) {
    let pages = total_pages(total, per_page);
    if page_events.is_empty() {
        return (empty.to_string(), pages);
    }

    let mut out = format!("<b>{}:</b>\n", html::escape(title));
    for (i, e) in page_events.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match style {
            RowStyle::TwoLine => write_event_row_two_line(&mut out, e, now, tz, loc),
            RowStyle::PreviewLink => write_event_row_preview_only(&mut out, e, tz, loc),
        }
    }
    (out, pages)
}

/// Builds the inline navigation keyboard for a page of a `kind` list.
///
/// Returns `None` when everything fits on a single page (no buttons needed).
/// Otherwise a single row holds `◀` / `▶` buttons (each present only when there
/// is a page to move to), carrying `<tag>:<target-page>` callback data, with a
/// non-interactive `<page>/<total>` indicator button between them (callback
/// `noop`, no handler — see `NOOP_DATA`).
pub fn list_keyboard(
    kind: ListKind,
    page: usize,
    total_pages: usize,
) -> Option<InlineKeyboardMarkup> {
    if total_pages <= 1 {
        return None;
    }
    let tag = kind.tag();
    let mut row = Vec::new();
    if page > 0 {
        row.push(InlineKeyboardButton::callback(
            "◀ Prev",
            format!("{tag}:{}", page - 1),
        ));
    }
    row.push(InlineKeyboardButton::callback(
        format!("{}/{total_pages}", page + 1),
        NOOP_DATA,
    ));
    if page + 1 < total_pages {
        row.push(InlineKeyboardButton::callback(
            "Next ▶",
            format!("{tag}:{}", page + 1),
        ));
    }
    Some(InlineKeyboardMarkup::new(vec![row]))
}

/// Renders one page of the missed-events list: the preview-only HTML body
/// ([`RowStyle::PreviewLink`]) plus the navigation keyboard (`None` when it fits
/// on a single page). `page_events` holds only the rows of `page`; `total` is
/// the whole list's length. Used by `state.rs` to build the startup
/// missed-events message; page turns reuse `commands::handle_list_callback` via
/// the `ms` tag.
pub fn format_missed_page(
    page_events: &[EventInfo],
    total: usize,
    now: chrono::NaiveDateTime,
    tz: Tz,
    page: usize,
    loc: &dyn LocaleProvider,
) -> (String, Option<InlineKeyboardMarkup>) {
    let kind = ListKind::Missed;
    let (text, total_pages) = format_page_at(
        page_events,
        total,
        now,
        tz,
        LIST_PAGE_SIZE,
        kind.title(),
        kind.empty(),
        kind.row_style(),
        loc,
    );
    (text, list_keyboard(kind, page, total_pages))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::EN;
    use crate::scheduler;
    use crate::view::test_support::sample_event;
    use chrono::{Duration, Utc};

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
            0,
            Utc::now().naive_utc(),
            Tz::UTC,
            10,
            "Upcoming events",
            "No upcoming events.",
            RowStyle::TwoLine,
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
            sample_event("pay rent (urgent)", Some(now + Duration::days(3))),
        ];
        let (text, pages) = format_page_at(
            &events,
            events.len(),
            now,
            Tz::UTC,
            10,
            "Upcoming events",
            "none",
            RowStyle::TwoLine,
            &EN,
        );
        assert_eq!(pages, 1);
        assert!(text.starts_with("<b>Upcoming events:</b>\n"));
        assert!(text.contains("▪ <b>14:00 15.06.2026 (in 2h)</b> /event0\n  call mom"));
        assert!(text.contains("(in 3d)"));
        assert!(text.contains("  pay rent (urgent)"));
        // Events are separated by a blank line.
        assert!(text.contains("  call mom\n\n▪ <b>"));
    }

    #[test]
    fn format_page_uses_given_title() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-16 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events = vec![sample_event("standup", Some(now + Duration::hours(1)))];
        let (text, _) = format_page_at(
            &events,
            events.len(),
            now,
            Tz::UTC,
            10,
            "Today's events",
            "none",
            RowStyle::TwoLine,
            &EN,
        );
        assert!(text.starts_with("<b>Today's events:</b>\n"));
        assert!(text.contains("▪ <b>10:00 16.06.2026 (in 1h)</b> /event0\n  standup"));
    }

    #[test]
    fn format_page_renders_given_slice_and_counts_pages_from_total() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let events: Vec<EventInfo> = (0..25)
            .map(|i| sample_event(&format!("event {i}"), Some(now + Duration::hours(i + 1))))
            .collect();

        // First page slice: 10 rows. The page count comes from `total`, not the
        // slice; page position lives on the keyboard, not the title.
        let (p0, pages) = format_page_at(
            &events[..10],
            25,
            now,
            Tz::UTC,
            10,
            "Upcoming events",
            "none",
            RowStyle::TwoLine,
            &EN,
        );
        assert_eq!(pages, 3);
        assert!(p0.starts_with("<b>Upcoming events:</b>\n"));
        assert!(p0.contains("event 0"));
        assert!(p0.contains("event 9"));
        assert!(!p0.contains("event 10"));

        // Last page slice: only 5 rows.
        let (p_last, pages) = format_page_at(
            &events[20..],
            25,
            now,
            Tz::UTC,
            10,
            "Upcoming events",
            "none",
            RowStyle::TwoLine,
            &EN,
        );
        assert_eq!(pages, 3);
        assert!(p_last.starts_with("<b>Upcoming events:</b>\n"));
        assert!(p_last.contains("event 20"));
        assert!(p_last.contains("event 24"));
        // Title line + two lines per row + a blank separator between rows.
        assert_eq!(p_last.lines().count(), 1 + 5 * 2 + 4);
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
        let (text, _) = format_page_at(
            &events,
            events.len(),
            now,
            Tz::UTC,
            10,
            "Upcoming events",
            "none",
            RowStyle::TwoLine,
            &EN,
        );
        assert!(text.starts_with("<b>Upcoming events:</b>\n"));
        // Bold datetime line and message live on separate lines; no `—` separator.
        assert!(!text.contains(" — "));
        // Plain text, tag-free, truncated to MESSAGE_PREVIEW_MAX chars + "...".
        assert!(text.contains("  call the office right now please and bring the doc..."));
        // One-off event: no recurrence suffix on the datetime line.
        assert!(!text.contains(", every"));
        // The /event<id> link ends the bold datetime line (id 0 for sample events).
        assert!(text.contains("▪ <b>14:00 15.06.2026 (in 2h)</b> /event0\n"));
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
        let (text, _) = format_page_at(
            &[e],
            1,
            now,
            Tz::UTC,
            10,
            "Upcoming events",
            "none",
            RowStyle::TwoLine,
            &EN,
        );
        // Recurrence sits inside the parentheses, next to the relative time; the
        // /event<id> link ends the line.
        assert!(text.contains("▪ <b>14:00 15.06.2026 (in 2h, every 2 days)</b> /event0\n"));
    }

    #[test]
    fn format_page_preview_link_layout() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let mut e = sample_event(
            "<b>call</b> the office right now please and bring the documents",
            Some(now + Duration::hours(2)),
        );
        e.id = 42;
        let (text, _) = format_page_at(
            &[e],
            1,
            now,
            Tz::UTC,
            10,
            "Missed events",
            "No missed events.",
            RowStyle::PreviewLink,
            &EN,
        );
        assert!(text.starts_with("<b>Missed events:</b>\n"));
        // Missed datetime (absolute, no bold), then the plain preview (tags
        // stripped, truncated) + /event<id>. No relative part — the moment is
        // in the past and would render "soon".
        assert!(text.contains(
            "▪ 14:00 15.06.2026 — call the office right now please and bring the doc... /event42\n"
        ));
        assert!(!text.contains("<b>14:"));
        assert!(!text.contains("(in "));
        assert!(!text.contains("(soon)"));
    }

    #[test]
    fn list_keyboard_layout_and_indicator() {
        use teloxide::types::InlineKeyboardButtonKind::CallbackData;

        // (label, callback-data) pairs of the single keyboard row.
        let buttons = |kb: InlineKeyboardMarkup| -> Vec<(String, String)> {
            kb.inline_keyboard
                .concat()
                .iter()
                .map(|b| match &b.kind {
                    CallbackData(d) => (b.text.clone(), d.clone()),
                    _ => panic!("expected callback data"),
                })
                .collect()
        };

        // Single page → no keyboard.
        assert!(list_keyboard(ListKind::Events, 0, 1).is_none());

        // Middle page: Prev, the indicator, then Next.
        assert_eq!(
            buttons(list_keyboard(ListKind::Events, 1, 3).unwrap()),
            [
                ("◀ Prev".to_string(), "ev:0".to_string()),
                ("2/3".to_string(), NOOP_DATA.to_string()),
                ("Next ▶".to_string(), "ev:2".to_string()),
            ]
        );

        // First page: no Prev.
        assert_eq!(
            buttons(list_keyboard(ListKind::Events, 0, 3).unwrap()),
            [
                ("1/3".to_string(), NOOP_DATA.to_string()),
                ("Next ▶".to_string(), "ev:1".to_string()),
            ]
        );

        // Last page: no Next.
        assert_eq!(
            buttons(list_keyboard(ListKind::Events, 2, 3).unwrap()),
            [
                ("◀ Prev".to_string(), "ev:1".to_string()),
                ("3/3".to_string(), NOOP_DATA.to_string()),
            ]
        );
    }

    #[test]
    fn missed_list_kind_tag_round_trips() {
        assert_eq!(ListKind::Missed.tag(), "ms");
        assert!(matches!(ListKind::from_tag("ms"), Some(ListKind::Missed)));
    }

    #[test]
    fn format_missed_page_renders_preview_link_rows() {
        use chrono::{NaiveDate, NaiveTime};

        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        );
        let mut event = scheduler::calc_next_at(
            {
                let mut e = sample_event("call the office", None);
                e.time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
                e
            },
            now,
            Tz::UTC,
        );
        event.id = 7;

        let (text, keyboard) = format_missed_page(&[event], 1, now, Tz::UTC, 0, &EN);
        assert!(text.starts_with("<b>Missed events:</b>\n"));
        // The row leads with the event's datetime (the snapshot puts the
        // missed moment in `next_datetime`).
        assert!(text.contains("▪ 09:00 16.06.2026 — call the office /event7"));
        // One event → single page → no navigation keyboard.
        assert!(keyboard.is_none());
    }
}
