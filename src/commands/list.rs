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
use chrono::{Datelike, Duration, Local, NaiveDate};
use teloxide::types::CallbackQuery;

/// Fetches one page of a `kind` list's events plus the list's total size (the
/// storage layer pages in SQL, so large lists never load whole). Date ranges
/// are computed relative to "now", so paging recomputes them (a page turn
/// across midnight reflects the then-current day/week/month). `Missed` reads
/// the startup snapshot instead.
fn fetch_page(
    kind: ListKind,
    provider: &EventProvider,
    chat_id: i64,
    page: usize,
) -> crate::error::Result<PageResponse> {
    let page = PageRequest::new(page, LIST_PAGE_SIZE);
    match kind {
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
            let start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
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

/// Replies with the first page of a `kind` list, attaching navigation buttons
/// when the list spans more than one page.
pub(super) async fn handle_list(ctx: &CmdContext<'_>, kind: ListKind) -> anyhow::Result<()> {
    let PageResponse { events, total } = fetch_page(kind, ctx.provider, ctx.chat_id.0, 0)?;
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

    let PageResponse { events, total } = fetch_page(kind, provider, chat_id.0, page)?;
    // The requested page can fall past the end when events were removed since
    // the keyboard was rendered; clamp to the (then-current) last page and
    // refetch it.
    let pages = total_pages(total, LIST_PAGE_SIZE);
    let (events, page) = if page >= pages {
        let page = pages - 1;
        (fetch_page(kind, provider, chat_id.0, page)?.events, page)
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
