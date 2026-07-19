//! The paginated list commands (`/events`, `/today`, `/tomorrow`, `/week`,
//! `/month`) plus the startup-only Missed list: [`handle_list`] replies with
//! page 0 and [`handle_list_callback`] serves the `<tag>:<page>` page-turn
//! buttons. The presentation half of each list ([`ListKind`]'s titles, tags,
//! row styles, and the page/keyboard rendering) lives in `crate::view`.

use super::CmdContext;
use crate::state::EventProvider;
use crate::tgbot::TgBot;
use crate::types::{PageRequest, PageResponse};
use crate::view::{LIST_PAGE_SIZE, ListKind, format_page_at, list_keyboard, total_pages};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use teloxide::types::CallbackQuery;

/// Converts a chat-local calendar-day window `[start_day, start_day + days)`
/// into UTC instant bounds for the storage range queries.
fn day_window(
    start_day: NaiveDate,
    days: i64,
    tz: Tz,
) -> (chrono::NaiveDateTime, chrono::NaiveDateTime) {
    let start = start_day.and_hms_opt(0, 0, 0).unwrap();
    let end = start + Duration::days(days);
    (crate::tz::to_utc(start, tz), crate::tz::to_utc(end, tz))
}

/// Fetches one page of a `kind` list's events plus the list's total size (the
/// storage layer pages in SQL, so large lists never load whole). Day/week/month
/// windows are the chat's local calendar days (converted to UTC bounds) and are
/// computed relative to "now", so paging recomputes them (a page turn across
/// midnight reflects the then-current day/week/month). `Missed` reads the
/// startup snapshot instead.
fn fetch_page(
    kind: ListKind,
    provider: &EventProvider,
    chat_id: i64,
    tz: Tz,
    page: usize,
) -> crate::error::Result<PageResponse> {
    let page = PageRequest::new(page, LIST_PAGE_SIZE);
    let today = || crate::tz::to_local(Utc::now().naive_utc(), tz).date();
    match kind {
        ListKind::Events => provider.get_active_by_chat(chat_id, page),
        ListKind::Missed => provider.get_missed_snapshot_events(chat_id, page),
        ListKind::Today => {
            let (start, end) = day_window(today(), 1, tz);
            provider.get_active_by_chat_in_range(chat_id, start, end, page)
        }
        ListKind::Tomorrow => {
            let (start, end) = day_window(today() + Duration::days(1), 1, tz);
            provider.get_active_by_chat_in_range(chat_id, start, end, page)
        }
        ListKind::Week => {
            let today = today();
            let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
            let (start, end) = day_window(monday, 7, tz);
            provider.get_active_by_chat_in_range(chat_id, start, end, page)
        }
        ListKind::Month => {
            let today = today();
            let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
            let (next_year, next_month) = if today.month() == 12 {
                (today.year() + 1, 1)
            } else {
                (today.year(), today.month() + 1)
            };
            let next_first = NaiveDate::from_ymd_opt(next_year, next_month, 1).unwrap_or(first);
            let (start, _) = day_window(first, 0, tz);
            let (end, _) = day_window(next_first, 0, tz);
            provider.get_active_by_chat_in_range(chat_id, start, end, page)
        }
    }
}

/// Replies with the first page of a `kind` list, attaching navigation buttons
/// when the list spans more than one page.
pub(super) async fn handle_list(ctx: &CmdContext<'_>, kind: ListKind) -> anyhow::Result<()> {
    let tz = ctx.tz;
    let PageResponse { events, total } = fetch_page(kind, ctx.provider, ctx.chat_id.0, tz, 0)?;
    let (text, total_pages) = format_page_at(
        &events,
        total,
        Utc::now().naive_utc(),
        tz,
        LIST_PAGE_SIZE,
        kind.title(),
        kind.empty(),
        kind.row_style(),
        ctx.loc,
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

    let tz = provider.tz_or_utc(chat_id.0);
    let PageResponse { events, total } = fetch_page(kind, provider, chat_id.0, tz, page)?;
    // The requested page can fall past the end when events were removed since
    // the keyboard was rendered; clamp to the (then-current) last page and
    // refetch it.
    let pages = total_pages(total, LIST_PAGE_SIZE);
    let (events, page) = if page >= pages {
        let page = pages - 1;
        (
            fetch_page(kind, provider, chat_id.0, tz, page)?.events,
            page,
        )
    } else {
        (events, page)
    };
    let loc = crate::locale::for_chat(chat_id.0);
    let (text, total_pages) = format_page_at(
        &events,
        total,
        Utc::now().naive_utc(),
        tz,
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
