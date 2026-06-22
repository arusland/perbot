use crate::import::{self, PendingImport};
use crate::state::EventProvider;
use crate::telegram::{LIST_PAGE_SIZE, format_page_at, scheduled_message};
use crate::types::EventInfo;
use chrono::{Datelike, Duration, Local, NaiveDate};
use std::process;
use teloxide::{
    net::Download,
    prelude::*,
    types::{FileId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, ParseMode},
    utils::command::BotCommands,
};

/// The paginated list commands. Each variant knows how to fetch its events,
/// title its reply, and tag its inline-button callbacks (`<tag>:<page>`).
#[derive(Clone, Copy)]
enum ListKind {
    Events,
    Today,
    Tomorrow,
    Week,
    Month,
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
            _ => None,
        }
    }

    /// Bare heading (no markdown markers); `format_page_at` adds `*…:*`.
    fn title(self) -> &'static str {
        match self {
            ListKind::Events => "Upcoming events",
            ListKind::Today => "Today's events",
            ListKind::Tomorrow => "Tomorrow's events",
            ListKind::Week => "This week's events",
            ListKind::Month => "This month's events",
        }
    }

    /// Message shown when the list is empty.
    fn empty(self) -> &'static str {
        match self {
            ListKind::Events => "No upcoming events\\.",
            ListKind::Today => "No events today\\.",
            ListKind::Tomorrow => "No events tomorrow\\.",
            ListKind::Week => "No events this week\\.",
            ListKind::Month => "No events this month\\.",
        }
    }

    /// Fetches the events for this list. Date ranges are computed relative to
    /// "now", so paging recomputes them (a page turn across midnight reflects the
    /// then-current day/week/month).
    fn fetch(self, provider: &EventProvider, chat_id: i64) -> Vec<crate::types::EventInfo> {
        match self {
            ListKind::Events => provider.get_active_by_chat(chat_id),
            ListKind::Today => {
                let today = Local::now().naive_local().date();
                provider.get_active_by_chat_on_date(chat_id, today)
            }
            ListKind::Tomorrow => {
                let tomorrow = Local::now().naive_local().date() + Duration::days(1);
                provider.get_active_by_chat_on_date(chat_id, tomorrow)
            }
            ListKind::Week => {
                let today = Local::now().naive_local().date();
                let start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
                let end = start + Duration::days(7);
                provider.get_active_by_chat_in_range(chat_id, start, end)
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
                provider.get_active_by_chat_in_range(chat_id, start, end)
            }
        }
    }
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
pub enum Command {
    #[command(description = "show this help message")]
    Help,
    #[command(description = "list upcoming scheduled events")]
    Events,
    #[command(description = "list today's events")]
    Today,
    #[command(description = "list tomorrow's events")]
    Tomorrow,
    #[command(description = "list this week's events")]
    Week,
    #[command(description = "list this month's events")]
    Month,
    #[command(description = "import legacy alerts for a chat (admin only)", hide)]
    Import(i64),
    #[command(description = "shut the bot down (admin only)", hide)]
    Exit,
}

/// Shared dependencies passed to every command handler.
pub struct CmdContext<'a> {
    pub bot: &'a Bot,
    pub chat_id: ChatId,
    pub provider: &'a EventProvider,
    pub admin_id: ChatId,
    pub is_admin: bool,
    pub pending_import: &'a PendingImport,
}

impl Command {
    /// Dispatches a parsed command to its handler.
    pub async fn handle(self, ctx: CmdContext<'_>) -> ResponseResult<()> {
        match self {
            Command::Help => handle_help(&ctx).await,
            Command::Events => handle_list(&ctx, ListKind::Events).await,
            Command::Today => handle_list(&ctx, ListKind::Today).await,
            Command::Tomorrow => handle_list(&ctx, ListKind::Tomorrow).await,
            Command::Week => handle_list(&ctx, ListKind::Week).await,
            Command::Month => handle_list(&ctx, ListKind::Month).await,
            Command::Import(user_id) => handle_import(&ctx, user_id).await,
            Command::Exit => handle_exit(&ctx).await,
        }
    }
}

/// Replies with the list of commands. Admins additionally see admin-only commands.
async fn handle_help(ctx: &CmdContext<'_>) -> ResponseResult<()> {
    let mut help = Command::descriptions().to_string();
    if ctx.is_admin {
        help.push_str(
            "\n\nAdmin commands:\n\
             /import <user_id> — import legacy alerts for a chat\n\
             /exit — shut the bot down",
        );
    }
    ctx.bot.send_message(ctx.chat_id, help).await?;
    Ok(())
}

/// Builds the inline navigation keyboard for a page of a `kind` list.
///
/// Returns `None` when everything fits on a single page (no buttons needed).
/// Otherwise a single row holds `◀` / `▶` buttons (each present only when there
/// is a page to move to), carrying `<tag>:<target-page>` callback data.
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
    if page + 1 < total_pages {
        row.push(InlineKeyboardButton::callback(
            "Next ▶",
            format!("{tag}:{}", page + 1),
        ));
    }
    Some(InlineKeyboardMarkup::new(vec![row]))
}

/// Replies with the first page of a `kind` list, attaching navigation buttons
/// when the list spans more than one page.
async fn handle_list(ctx: &CmdContext<'_>, kind: ListKind) -> ResponseResult<()> {
    let events = kind.fetch(ctx.provider, ctx.chat_id.0);
    let (text, total_pages) = format_page_at(
        &events,
        Local::now().naive_local(),
        0,
        LIST_PAGE_SIZE,
        kind.title(),
        kind.empty(),
    );

    let mut req = ctx
        .bot
        .send_message(ctx.chat_id, &text)
        .parse_mode(ParseMode::MarkdownV2);
    if let Some(kb) = list_keyboard(kind, 0, total_pages) {
        req = req.reply_markup(kb);
    }
    if let Err(e) = req.await {
        // A single page shouldn't exceed Telegram's 4096-char limit, but keep the
        // safety net: log with context and warn the admin instead of bubbling up.
        log::error!(
            "Failed to send /{} reply to chat {}: {e} ({} events, {} chars).",
            kind.tag(),
            ctx.chat_id.0,
            events.len(),
            text.chars().count(),
        );
        let warning = format!(
            "Failed to send /{} reply to chat {}: {e} ({} events, {} chars).",
            kind.tag(),
            ctx.chat_id.0,
            events.len(),
            text.chars().count(),
        );
        if let Err(warn_err) = ctx.bot.send_message(ctx.admin_id, warning).await {
            log::error!("Failed to warn admin about send failure: {warn_err}");
        }
    }
    Ok(())
}

/// Handles an inline-button press from any paginated list message: decodes the
/// `<tag>:<page>` callback data, re-queries that list's events, renders the
/// requested page, and edits the message in place.
pub async fn handle_list_callback(
    bot: &Bot,
    provider: &EventProvider,
    q: CallbackQuery,
) -> ResponseResult<()> {
    // Always answer to clear the client's loading spinner.
    bot.answer_callback_query(q.id.clone()).await?;

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

    let events = kind.fetch(provider, chat_id.0);
    let (text, total_pages) = format_page_at(
        &events,
        Local::now().naive_local(),
        page,
        LIST_PAGE_SIZE,
        kind.title(),
        kind.empty(),
    );
    let page = page.min(total_pages.saturating_sub(1));

    let mut req = bot
        .edit_message_text(chat_id, message_id, &text)
        .parse_mode(ParseMode::MarkdownV2);
    if let Some(kb) = list_keyboard(kind, page, total_pages) {
        req = req.reply_markup(kb);
    }
    if let Err(e) = req.await {
        // "message is not modified" (e.g. double-tap) is benign; just log others.
        log::warn!(
            "Failed to edit /{} page for chat {}: {e}",
            kind.tag(),
            chat_id.0
        );
    }
    Ok(())
}

/// Parses snooze callback data `eid:<id>:sn:<minutes>` into `(event_id, minutes)`.
/// Returns `None` for any malformed input or a non-snooze action.
fn parse_snooze_callback(data: &str) -> Option<(i64, i64)> {
    let rest = data.strip_prefix("eid:")?;
    let (id, action) = rest.split_once(':')?;
    let minutes = action.strip_prefix("sn:")?;
    Some((id.parse::<i64>().ok()?, minutes.parse::<i64>().ok()?))
}

/// Builds the one-off event a snooze creates: an explicit-year reminder scheduled
/// exactly at `next`, already marked active. It is inserted via
/// `insert_prebuilt_event` (no scheduler run), and after it fires
/// `scheduler::calc_next_at` returns `None` (no repetition, year explicit), so it
/// goes inactive instead of repeating.
fn snoozed_event(
    chat_id: i64,
    msg_id: i64,
    title: String,
    next: chrono::NaiveDateTime,
) -> EventInfo {
    EventInfo {
        date: Some(next.date()),
        time: Some(next.time()),
        year_explicit: true,
        days: None,
        years: None,
        repetition: None,
        in_offset: None,
        bare_hour: None,
        monthly_pattern: None,
        message: title,
        id: 0,
        chat_id,
        active: true,
        next_datetime: Some(next),
        created_at: next,
        msg_id,
        legacy: false,
        snoozed: true,
    }
}

/// Handles a snooze-button press: creates a new one-off event with the same title
/// as the fired reminder, scheduled at `now + <minutes>`. The original event is
/// left untouched. Driven from `main`'s callback-query branch for `eid:`-prefixed
/// callback data.
///
/// The target event is identified by id from the callback data
/// (`eid:<id>:sn:<minutes>`) and loaded from storage. Because callback ids are
/// attacker-influenceable, the loaded event is only honored when it belongs to the
/// chat the button was pressed in.
pub async fn handle_snooze_callback(
    bot: &Bot,
    provider: &EventProvider,
    q: CallbackQuery,
) -> ResponseResult<()> {
    let parsed = q.data.as_deref().and_then(parse_snooze_callback);
    let Some((event_id, minutes)) = parsed else {
        bot.answer_callback_query(q.id).await?;
        return Ok(());
    };

    let Some(message) = q.regular_message() else {
        bot.answer_callback_query(q.id)
            .text("Can't snooze this reminder.")
            .await?;
        return Ok(());
    };
    let chat_id = message.chat.id;

    // Load the event and verify it belongs to this chat before acting on it.
    let title = match provider.get_event(event_id) {
        Some(event) if event.chat_id == chat_id.0 => event.message,
        _ => {
            bot.answer_callback_query(q.id)
                .text("Can't snooze this reminder.")
                .await?;
            return Ok(());
        }
    };

    let now = Local::now().naive_local();
    let next = now + Duration::minutes(minutes);
    let user_id = q.from.id.0 as i64;

    // Backing message row (events.msg_id is a NOT NULL FK to messages).
    let msg_id = match provider.insert_message(Some(user_id), chat_id.0, &title) {
        Ok(id) => id,
        Err(e) => {
            log::error!("Failed to save snooze message for chat {}: {e}", chat_id.0);
            bot.answer_callback_query(q.id)
                .text("Failed to snooze.")
                .await?;
            return Ok(());
        }
    };

    let event = snoozed_event(chat_id.0, msg_id, title, next);
    if let Err(e) = provider.insert_prebuilt_event(&event) {
        log::error!("Failed to insert snoozed event for chat {}: {e}", chat_id.0);
        bot.answer_callback_query(q.id)
            .text("Failed to snooze.")
            .await?;
        return Ok(());
    }

    bot.answer_callback_query(q.id).await?;
    bot.send_message(chat_id, scheduled_message(next))
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
    Ok(())
}

/// Begins a legacy import for `user_id`. Admin-only; records the pending target
/// and asks the admin to send the zip of `.alert` files next.
async fn handle_import(ctx: &CmdContext<'_>, user_id: i64) -> ResponseResult<()> {
    if !ctx.is_admin {
        ctx.bot.send_message(ctx.chat_id, "Not authorized.").await?;
        return Ok(());
    }
    *ctx.pending_import.lock().unwrap() = Some(user_id);
    ctx.bot
        .send_message(
            ctx.chat_id,
            format!("Send the .zip of legacy alerts now to import them for chat {user_id}."),
        )
        .await?;
    Ok(())
}

/// Downloads the admin's zip, imports the legacy alerts for `target`, and replies
/// with a summary plus the HTML report as a document. Driven from `main` when the
/// admin sends the zip after `/import <user_id>`.
pub async fn handle_import_zip(
    bot: &Bot,
    provider: &EventProvider,
    chat_id: ChatId,
    target: i64,
    file_id: FileId,
) -> ResponseResult<()> {
    let file = bot.get_file(file_id).await?;
    let mut buf: Vec<u8> = Vec::new();
    if let Err(e) = bot.download_file(&file.path, &mut buf).await {
        bot.send_message(chat_id, format!("Failed to download the zip: {e}"))
            .await?;
        return Ok(());
    }

    bot.send_message(chat_id, "Importing events from file...")
        .await?;

    match import::import_zip(provider, target, &buf) {
        Ok(outcome) => {
            let report_path = std::env::temp_dir().join("perbot-legacy-report.html");
            bot.send_message(chat_id, outcome.summary()).await?;
            match std::fs::write(&report_path, &outcome.html) {
                Ok(()) => {
                    bot.send_document(chat_id, InputFile::file(&report_path))
                        .await?;
                }
                Err(e) => {
                    bot.send_message(chat_id, format!("Failed to write report: {e}"))
                        .await?;
                }
            }
        }
        Err(e) => {
            bot.send_message(chat_id, format!("Import failed: {e}"))
                .await?;
        }
    }
    Ok(())
}

/// Shuts the bot down. Admin-only; non-admins get a rejection reply.
async fn handle_exit(ctx: &CmdContext<'_>) -> ResponseResult<()> {
    if !ctx.is_admin {
        ctx.bot
            .send_message(ctx.chat_id, "Not authorized\\.")
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        return Ok(());
    }
    log::info!("Received /exit command. Shutting down...");
    let _ = ctx.bot.send_message(ctx.admin_id, "Shutting down...").await;
    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        process::exit(0);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler;

    #[test]
    fn parse_snooze_callback_round_trips_and_rejects_malformed() {
        assert_eq!(parse_snooze_callback("eid:42:sn:30"), Some((42, 30)));
        assert_eq!(parse_snooze_callback("eid:-7:sn:1"), Some((-7, 1)));

        // Old format, non-numeric id/minutes, missing parts, and list callbacks.
        assert_eq!(parse_snooze_callback("sn:30"), None);
        assert_eq!(parse_snooze_callback("eid:x:sn:30"), None);
        assert_eq!(parse_snooze_callback("eid:42:sn:"), None);
        assert_eq!(parse_snooze_callback("eid:42:sn:abc"), None);
        assert_eq!(parse_snooze_callback("ev:1"), None);
    }

    #[test]
    fn snoozed_event_goes_inactive_after_firing() {
        // The snoozed event is scheduled at `next`; once "now" reaches it (firing),
        // calc_next_at must return inactive so it does not repeat.
        let next = Local::now().naive_local() + Duration::minutes(5);
        let event = snoozed_event(42, 7, "call mom".to_string(), next);
        assert!(event.active);
        assert!(event.snoozed);
        assert_eq!(event.next_datetime, Some(next));

        let fired = scheduler::calc_next_at(event, next);
        assert!(!fired.active);
        assert!(fired.next_datetime.is_none());
    }
}
