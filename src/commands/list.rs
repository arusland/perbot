//! The paginated list commands (`/events`, `/today`, `/tomorrow`, `/week`,
//! `/month`) plus the startup-only Missed list: [`ListKind`] describes each
//! list, [`handle_list`] replies with page 0, and [`handle_list_callback`]
//! serves the `<tag>:<page>` page-turn buttons.

use super::CmdContext;
use crate::locale::LocaleProvider;
use crate::state::EventProvider;
use crate::telegram::RowStyle;
use crate::telegram::{LIST_PAGE_SIZE, format_page_at};
use crate::tgbot::TgBot;
use crate::types::{EventInfo, PageRequest, PageResponse};
use chrono::{Datelike, Duration, Local, NaiveDate};
use teloxide::types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup};

/// Callback data for non-interactive buttons (e.g. the page indicator). It has
/// no `:`-prefixed envelope and matches no list tag, so `main`'s router hands it
/// to `handle_list_callback`, which answers the query and ignores it.
const NOOP_DATA: &str = "noop";

/// The paginated list commands. Each variant knows how to fetch its events,
/// title its reply, and tag its inline-button callbacks (`<tag>:<page>`).
///
/// `Missed` is not a user-typed command: it is reached only by the startup
/// missed-events send and its `ms:<page>` page-turn callbacks. Its events come
/// from the `missed_events` snapshot table populated at startup (see
/// [`EventProvider::get_missed_snapshot_events`]).
#[derive(Clone, Copy)]
pub(super) enum ListKind {
    Events,
    Today,
    Tomorrow,
    Week,
    Month,
    Missed,
}

impl ListKind {
    /// Short tag used as the callback-data prefix (`<tag>:<page>`).
    fn tag(self) -> &'static str {
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
    fn from_tag(tag: &str) -> Option<Self> {
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
    fn title(self) -> &'static str {
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
    fn empty(self) -> &'static str {
        match self {
            ListKind::Events => "No upcoming events.",
            ListKind::Today => "No events today.",
            ListKind::Tomorrow => "No events tomorrow.",
            ListKind::Week => "No events this week.",
            ListKind::Month => "No events this month.",
            ListKind::Missed => "No missed events.",
        }
    }

    /// Per-row layout for this list. `/events` uses the two-line row; the missed
    /// list shows only a plain preview + `/event<id>` link; the rest use the
    /// single-line row.
    fn row_style(self) -> RowStyle {
        match self {
            ListKind::Events => RowStyle::TwoLine,
            ListKind::Missed => RowStyle::PreviewLink,
            ListKind::Today | ListKind::Tomorrow | ListKind::Week | ListKind::Month => {
                RowStyle::SingleLine
            }
        }
    }

    /// Fetches one page of this list's events plus the list's total size (the
    /// storage layer pages in SQL, so large lists never load whole). Date ranges
    /// are computed relative to "now", so paging recomputes them (a page turn
    /// across midnight reflects the then-current day/week/month). `Missed` reads
    /// the startup snapshot instead.
    fn fetch(
        self,
        provider: &EventProvider,
        chat_id: i64,
        page: usize,
    ) -> crate::error::Result<PageResponse> {
        let page = PageRequest::new(page, LIST_PAGE_SIZE);
        match self {
            ListKind::Events => provider.get_active_by_chat(chat_id, page),
            ListKind::Missed => provider.get_missed_snapshot_events(chat_id, page),
            ListKind::Today => {
                let today = Local::now().naive_local().date();
                provider.get_active_by_chat_on_date(chat_id, today, page)
            }
            ListKind::Tomorrow => {
                let tomorrow = Local::now().naive_local().date() + Duration::days(1);
                provider.get_active_by_chat_on_date(chat_id, tomorrow, page)
            }
            ListKind::Week => {
                let today = Local::now().naive_local().date();
                let start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
                let end = start + Duration::days(7);
                provider.get_active_by_chat_in_range(chat_id, start, end, page)
            }
            ListKind::Month => {
                let today = Local::now().naive_local().date();
                let start =
                    NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
                let (next_year, next_month) = if today.month() == 12 {
                    (today.year() + 1, 1)
                } else {
                    (today.year(), today.month() + 1)
                };
                let end = NaiveDate::from_ymd_opt(next_year, next_month, 1).unwrap_or(start);
                provider.get_active_by_chat_in_range(chat_id, start, end, page)
            }
        }
    }
}

/// Builds the inline navigation keyboard for a page of a `kind` list.
///
/// Returns `None` when everything fits on a single page (no buttons needed).
/// Otherwise a single row holds `◀` / `▶` buttons (each present only when there
/// is a page to move to), carrying `<tag>:<target-page>` callback data, with a
/// non-interactive `<page>/<total>` indicator button between them (callback
/// `noop`, no handler — see `NOOP_DATA`).
fn list_keyboard(kind: ListKind, page: usize, total_pages: usize) -> Option<InlineKeyboardMarkup> {
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
/// missed-events message; page turns reuse [`handle_list_callback`] via the
/// `ms` tag.
pub fn format_missed_page(
    page_events: &[EventInfo],
    total: usize,
    now: chrono::NaiveDateTime,
    page: usize,
    loc: &dyn LocaleProvider,
) -> (String, Option<InlineKeyboardMarkup>) {
    let kind = ListKind::Missed;
    let (text, total_pages) = format_page_at(
        page_events,
        total,
        now,
        LIST_PAGE_SIZE,
        kind.title(),
        kind.empty(),
        kind.row_style(),
        loc,
    );
    (text, list_keyboard(kind, page, total_pages))
}

/// Replies with the first page of a `kind` list, attaching navigation buttons
/// when the list spans more than one page.
pub(super) async fn handle_list(ctx: &CmdContext<'_>, kind: ListKind) -> anyhow::Result<()> {
    let PageResponse { events, total } = kind.fetch(ctx.provider, ctx.chat_id.0, 0)?;
    let loc = crate::locale::for_chat(ctx.chat_id.0);
    let (text, total_pages) = format_page_at(
        &events,
        total,
        Local::now().naive_local(),
        LIST_PAGE_SIZE,
        kind.title(),
        kind.empty(),
        kind.row_style(),
        loc,
    );

    if let Err(e) = ctx
        .bot
        .send_html(
            ctx.chat_id,
            text.as_str(),
            list_keyboard(kind, 0, total_pages),
        )
        .await
    {
        // A single page shouldn't exceed Telegram's 4096-char limit, but keep the
        // safety net: log with context and warn the admin instead of bubbling up.
        log::error!(
            "Failed to send /{} reply to chat {}: {e} ({} events, {} chars).",
            kind.tag(),
            ctx.chat_id.0,
            total,
            text.chars().count(),
        );
        let warning = format!(
            "Failed to send /{} reply to chat {}: {e} ({} events, {} chars).",
            kind.tag(),
            ctx.chat_id.0,
            total,
            text.chars().count(),
        );
        if let Err(warn_err) = ctx.bot.send_text(ctx.admin_id, warning, None).await {
            log::error!("Failed to warn admin about send failure: {warn_err}");
        }
    }
    Ok(())
}

/// Handles an inline-button press from any paginated list message: decodes the
/// `<tag>:<page>` callback data, re-queries that list's events, renders the
/// requested page, and edits the message in place.
pub async fn handle_list_callback(
    bot: &TgBot,
    provider: &EventProvider,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    // Always answer to clear the client's loading spinner.
    bot.answer_callback(q.id.clone(), None).await?;

    let Some((kind, page)) = q.data.as_deref().and_then(|d| {
        let (tag, page) = d.split_once(':')?;
        Some((ListKind::from_tag(tag)?, page.parse::<usize>().ok()?))
    }) else {
        return Ok(());
    };

    let Some(message) = q.regular_message() else {
        // Message is too old/inaccessible to edit.
        return Ok(());
    };
    let chat_id = message.chat.id;
    let message_id = message.id;

    let PageResponse { events, total } = kind.fetch(provider, chat_id.0, page)?;
    // The requested page can fall past the end when events were removed since
    // the keyboard was rendered; clamp to the (then-current) last page and
    // refetch it.
    let pages = crate::telegram::total_pages(total, LIST_PAGE_SIZE);
    let (events, page) = if page >= pages {
        let page = pages - 1;
        (kind.fetch(provider, chat_id.0, page)?.events, page)
    } else {
        (events, page)
    };
    let loc = crate::locale::for_chat(chat_id.0);
    let (text, total_pages) = format_page_at(
        &events,
        total,
        Local::now().naive_local(),
        LIST_PAGE_SIZE,
        kind.title(),
        kind.empty(),
        kind.row_style(),
        loc,
    );

    if let Err(e) = bot
        .edit_html(
            chat_id,
            message_id,
            text.as_str(),
            list_keyboard(kind, page, total_pages),
        )
        .await
    {
        // "message is not modified" (e.g. double-tap) is benign; just log others.
        log::warn!(
            "Failed to edit /{} page for chat {}: {e}",
            kind.tag(),
            chat_id.0
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler;

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
        use crate::locale::EN;
        use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        );
        let mut event = scheduler::calc_next_at(
            {
                let mut e = sample_missed_event("call the office");
                e.time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
                e
            },
            now,
        );
        event.id = 7;

        let (text, keyboard) = format_missed_page(&[event], 1, now, 0, &EN);
        assert!(text.starts_with("<b>Missed events:</b>\n"));
        assert!(text.contains("/event7"));
        // One event → single page → no navigation keyboard.
        assert!(keyboard.is_none());
    }

    /// Minimal one-off event carrying just a message, for list-rendering tests.
    fn sample_missed_event(message: &str) -> EventInfo {
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
            message: message.to_string(),
            id: 0,
            chat_id: 0,
            active: false,
            next_datetime: None,
            source: None,
            last_next_datetime: None,
            created_at: Local::now().naive_local(),
            msg_id: 0,
            legacy: false,
            snoozed: false,
        }
    }
}
