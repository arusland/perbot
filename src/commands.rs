use crate::import::{self, PendingImport};
use crate::pending::{self, PendingEdit, PendingMessage};
use crate::state::EventProvider;
use crate::telegram::{
    LIST_PAGE_SIZE, edit_prompt, event_detail, format_page_at, scheduled_message,
};
use crate::types::EventInfo;
use chrono::{Datelike, Duration, Local, NaiveDate};
use std::process;
use teloxide::{
    net::Download,
    prelude::*,
    types::{FileId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, ParseMode},
    utils::command::BotCommands,
};

/// Callback data for non-interactive buttons (e.g. the page indicator). It has
/// no `:`-prefixed envelope and matches no list tag, so `main`'s router hands it
/// to `handle_list_callback`, which answers the query and ignores it.
const NOOP_DATA: &str = "noop";

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

    /// Bare heading (no markup); `format_page_at` wraps it in `<b>…:</b>`.
    fn title(self) -> &'static str {
        match self {
            ListKind::Events => "Upcoming events",
            ListKind::Today => "Today's events",
            ListKind::Tomorrow => "Tomorrow's events",
            ListKind::Week => "This week's events",
            ListKind::Month => "This month's events",
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
    #[command(description = "download the database file (admin only)", hide)]
    Database,
    #[command(description = "download the current log file (admin only)", hide)]
    Logs,
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
            Command::Database => handle_database(&ctx).await,
            Command::Logs => handle_logs(&ctx).await,
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
             /database — download the database file\n\
             /logs — download the current log file\n\
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

/// Replies with the first page of a `kind` list, attaching navigation buttons
/// when the list spans more than one page.
async fn handle_list(ctx: &CmdContext<'_>, kind: ListKind) -> ResponseResult<()> {
    let events = kind.fetch(ctx.provider, ctx.chat_id.0);
    let loc = crate::locale::for_chat(ctx.chat_id.0);
    let (text, total_pages) = format_page_at(
        &events,
        Local::now().naive_local(),
        0,
        LIST_PAGE_SIZE,
        kind.title(),
        kind.empty(),
        matches!(kind, ListKind::Events),
        loc,
    );

    let mut req = ctx
        .bot
        .send_message(ctx.chat_id, &text)
        .parse_mode(ParseMode::Html);
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
    let loc = crate::locale::for_chat(chat_id.0);
    let (text, total_pages) = format_page_at(
        &events,
        Local::now().naive_local(),
        page,
        LIST_PAGE_SIZE,
        kind.title(),
        kind.empty(),
        matches!(kind, ListKind::Events),
        loc,
    );
    let page = page.min(total_pages.saturating_sub(1));

    let mut req = bot
        .edit_message_text(chat_id, message_id, &text)
        .parse_mode(ParseMode::Html);
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

/// Parses a `/event<id>` (or `/event<id>@<bot_username>`) command into the event id.
///
/// `/event<id>` has no space between the name and its argument, so teloxide's
/// `BotCommands` derive can't parse it; it is matched manually here. Returns `None`
/// for anything else (including the bare `/events` list command, `/event` with no id,
/// a non-numeric id, or a mismatched `@bot` suffix).
pub fn parse_event_command(text: &str, bot_username: &str) -> Option<i64> {
    let token = text.trim().split_whitespace().next()?;
    let rest = token.strip_prefix("/event")?;
    // Strip an optional `@bot_username` suffix; reject if it names another bot.
    let digits = match rest.split_once('@') {
        Some((digits, bot)) if bot.eq_ignore_ascii_case(bot_username) => digits,
        Some(_) => return None,
        None => rest,
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// Sends the single-event detail view for `/event<id>`: the bold datetime/recurrence
/// line, the full rich-text message, and the upcoming-launches preview. The event is
/// loaded by id and shown only when it belongs to the requesting chat (ids are
/// user-influenceable), otherwise the chat is told the event was not found.
pub async fn handle_event_view(
    bot: &Bot,
    provider: &EventProvider,
    chat_id: ChatId,
    id: i64,
) -> ResponseResult<()> {
    match provider.get_event(id) {
        Some(event) if event.chat_id == chat_id.0 => {
            let now = Local::now().naive_local();
            let loc = crate::locale::for_chat(chat_id.0);
            bot.send_message(chat_id, event_detail(&event, now, loc))
                .parse_mode(ParseMode::Html)
                .reply_markup(event_actions_keyboard(id))
                .await?;
        }
        _ => {
            bot.send_message(chat_id, "Event not found.").await?;
        }
    }
    Ok(())
}

/// The action buttons shown under the `/event<id>` detail view: `✏️ Edit`
/// (callback `eid:<id>:ed`, starts the edit flow) and `🗑 Delete` (callback
/// `eid:<id>:del`, swaps in the [`delete_confirm_keyboard`] row).
fn event_actions_keyboard(event_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("✏️ Edit", format!("eid:{event_id}:ed")),
        InlineKeyboardButton::callback("🗑 Delete", format!("eid:{event_id}:del")),
    ]])
}

/// The single Cancel button shown while the chat is editing an event (callback
/// `eid:<id>:edno`, drops the pending edit). Public so `main`'s edit-completion
/// re-prompts can reuse it.
pub fn edit_cancel_keyboard(event_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "Cancel",
        format!("eid:{event_id}:edno"),
    )]])
}

/// The confirmation row shown after the Delete button is tapped: a confirm
/// (`eid:<id>:delyes`) and a cancel (`eid:<id>:delno`) button.
fn delete_confirm_keyboard(event_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("✅ Yes, delete", format!("eid:{event_id}:delyes")),
        InlineKeyboardButton::callback("❌ Cancel", format!("eid:{event_id}:delno")),
    ]])
}

/// Decodes the event-specific callback envelope `eid:<id>:<action>` into the
/// event id and the action remainder (e.g. `sn:30`, `del`, `delyes`). Returns
/// `None` for anything not shaped like the envelope.
fn parse_event_callback(data: &str) -> Option<(i64, &str)> {
    let rest = data.strip_prefix("eid:")?;
    let (id, action) = rest.split_once(':')?;
    Some((id.parse::<i64>().ok()?, action))
}

/// Parses snooze callback data `eid:<id>:sn:<minutes>` into `(event_id, minutes)`.
/// Returns `None` for any malformed input or a non-snooze action.
fn parse_snooze_callback(data: &str) -> Option<(i64, i64)> {
    let (id, action) = parse_event_callback(data)?;
    let minutes = action.strip_prefix("sn:")?;
    Some((id, minutes.parse::<i64>().ok()?))
}

/// Dispatches an event-specific callback (`eid:<id>:<action>`) to the matching
/// handler: snooze (`sn:<minutes>`), the delete flow (`del` → confirm prompt,
/// `delyes` → delete, `delno` → restore the action buttons), or the edit flow
/// (`ed` → start editing, `edno` → cancel editing). Unknown actions are
/// acknowledged and ignored. Routed from `main`'s `eid:`-prefixed callback branch.
pub async fn handle_event_callback(
    bot: &Bot,
    provider: &EventProvider,
    pending_edit: &PendingEdit,
    q: CallbackQuery,
) -> ResponseResult<()> {
    match q.data.as_deref().and_then(parse_event_callback) {
        Some((id, "del")) => handle_delete_prompt(bot, id, q).await,
        Some((id, "delyes")) => handle_delete_confirm(bot, provider, id, q).await,
        Some((id, "delno")) => handle_delete_cancel(bot, id, q).await,
        Some((id, "ed")) => handle_edit_prompt(bot, provider, pending_edit, id, q).await,
        Some((_, "edno")) => handle_edit_cancel(bot, pending_edit, q).await,
        Some((_, action)) if action.starts_with("sn:") => {
            handle_snooze_callback(bot, provider, q).await
        }
        _ => {
            bot.answer_callback_query(q.id).await?;
            Ok(())
        }
    }
}

/// Handles the `✏️ Edit` press (`eid:<id>:ed`): access-checks the event against
/// the chat the button was pressed in (callback ids are user-influenceable),
/// records the chat as editing that event, and prompts for the replacement input
/// with the event's current input as a copyable `<code>` block ([`edit_prompt`])
/// and a Cancel button. Replies "Event not found." for a missing or foreign id.
async fn handle_edit_prompt(
    bot: &Bot,
    provider: &EventProvider,
    pending_edit: &PendingEdit,
    id: i64,
    q: CallbackQuery,
) -> ResponseResult<()> {
    let Some(message) = q.regular_message() else {
        bot.answer_callback_query(q.id).await?;
        return Ok(());
    };
    let chat_id = message.chat.id;

    let event = provider.get_event(id).filter(|e| e.chat_id == chat_id.0);
    bot.answer_callback_query(q.id).await?;
    if let Some(event) = event {
        pending_edit.lock().unwrap().insert(chat_id.0, id);
        let loc = crate::locale::for_chat(chat_id.0);
        bot.send_message(chat_id, edit_prompt(pending::EDIT_ASK_TEXT, &event, loc))
            .parse_mode(ParseMode::Html)
            .reply_markup(edit_cancel_keyboard(id))
            .await?;
    } else {
        bot.send_message(chat_id, "Event not found.").await?;
    }
    Ok(())
}

/// Handles the Cancel press while editing (`eid:<id>:edno`): drops the chat's
/// pending edit and edits the prompt to "Cancelled." (clearing the keyboard).
async fn handle_edit_cancel(
    bot: &Bot,
    pending_edit: &PendingEdit,
    q: CallbackQuery,
) -> ResponseResult<()> {
    bot.answer_callback_query(q.id.clone()).await?;

    let Some(message) = q.regular_message() else {
        return Ok(());
    };
    let chat_id = message.chat.id;
    pending_edit.lock().unwrap().remove(&chat_id.0);

    if let Err(e) = bot
        .edit_message_text(chat_id, message.id, "Cancelled.")
        .await
    {
        log::warn!(
            "Failed to edit cancelled edit prompt for chat {}: {e}",
            chat_id.0
        );
    }
    Ok(())
}

/// Handles the `🗑 Delete` press (`eid:<id>:del`): swaps the keyboard in place for
/// the confirm/cancel row, leaving the detail text untouched.
async fn handle_delete_prompt(bot: &Bot, id: i64, q: CallbackQuery) -> ResponseResult<()> {
    if let Some(message) = q.regular_message() {
        if let Err(e) = bot
            .edit_message_reply_markup(message.chat.id, message.id)
            .reply_markup(delete_confirm_keyboard(id))
            .await
        {
            log::warn!("Failed to show delete confirmation for event {id}: {e}");
        }
    }
    bot.answer_callback_query(q.id).await?;
    Ok(())
}

/// Handles the `❌ Cancel` press (`eid:<id>:delno`): restores the original
/// Edit/Delete action buttons, leaving the detail text untouched.
async fn handle_delete_cancel(bot: &Bot, id: i64, q: CallbackQuery) -> ResponseResult<()> {
    if let Some(message) = q.regular_message() {
        if let Err(e) = bot
            .edit_message_reply_markup(message.chat.id, message.id)
            .reply_markup(event_actions_keyboard(id))
            .await
        {
            log::warn!("Failed to restore delete button for event {id}: {e}");
        }
    }
    bot.answer_callback_query(q.id).await?;
    Ok(())
}

/// Handles the `✅ Yes, delete` press (`eid:<id>:delyes`): access-checks the event
/// against the chat the button was pressed in (callback ids are
/// user-influenceable), deletes it, and edits the message to a confirmation
/// (clearing the keyboard). Replies "Event not found." for a missing or foreign id.
async fn handle_delete_confirm(
    bot: &Bot,
    provider: &EventProvider,
    id: i64,
    q: CallbackQuery,
) -> ResponseResult<()> {
    let Some(message) = q.regular_message() else {
        bot.answer_callback_query(q.id).await?;
        return Ok(());
    };
    let chat_id = message.chat.id;
    let message_id = message.id;

    let owned = matches!(provider.get_event(id), Some(event) if event.chat_id == chat_id.0);
    let text = if owned && provider.delete(id) {
        "Event deleted."
    } else {
        "Event not found."
    };

    bot.answer_callback_query(q.id).await?;
    if let Err(e) = bot.edit_message_text(chat_id, message_id, text).await {
        log::warn!("Failed to edit deleted-event message for event {id}: {e}");
    }
    Ok(())
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
    // `event.message` is an HTML fragment, so the snoozed copy keeps the user's
    // formatting verbatim.
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
    let loc = crate::locale::for_chat(chat_id.0);
    bot.send_message(chat_id, scheduled_message(now, next, &event, loc))
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

/// Handles a Cancel-button press from the "send me the reminder text" prompt:
/// drops the pending request for the chat and edits the prompt to "Cancelled."
/// (clearing the keyboard). Routed from `main`'s callback branch for the `pm:`
/// prefix.
pub async fn handle_cancel_pending(
    bot: &Bot,
    pending_msg: &PendingMessage,
    q: CallbackQuery,
) -> ResponseResult<()> {
    bot.answer_callback_query(q.id.clone()).await?;

    let Some(message) = q.regular_message() else {
        return Ok(());
    };
    let chat_id = message.chat.id;
    pending_msg.lock().unwrap().remove(&chat_id.0);

    if let Err(e) = bot
        .edit_message_text(chat_id, message.id, "Cancelled.")
        .await
    {
        log::warn!(
            "Failed to edit cancelled prompt for chat {}: {e}",
            chat_id.0
        );
    }
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

/// Sends a consistent snapshot of the SQLite database back as a document.
/// Admin-only; non-admins get a rejection reply. The bot holds an open connection,
/// so we snapshot via `VACUUM INTO` (a temp file) rather than copying the live file,
/// then clean the snapshot up.
async fn handle_database(ctx: &CmdContext<'_>) -> ResponseResult<()> {
    if !ctx.is_admin {
        ctx.bot.send_message(ctx.chat_id, "Not authorized.").await?;
        return Ok(());
    }

    let snapshot = std::env::temp_dir().join("perbot-db-snapshot.sqlite");
    // VACUUM INTO requires the destination not to exist.
    let _ = std::fs::remove_file(&snapshot);
    if let Err(e) = ctx.provider.backup_database(&snapshot) {
        log::error!("Failed to snapshot database: {e}");
        ctx.bot
            .send_message(ctx.chat_id, format!("Failed to snapshot database: {e}"))
            .await?;
        return Ok(());
    }

    let doc = InputFile::file(&snapshot).file_name("perbot.db");
    if let Err(e) = ctx.bot.send_document(ctx.chat_id, doc).await {
        log::error!("Failed to send database to chat {}: {e}", ctx.chat_id.0);
    }
    let _ = std::fs::remove_file(&snapshot);
    Ok(())
}

/// Sends the current log file back as a document. Admin-only; non-admins get a
/// rejection reply. The log file is append-only text, so it is sent directly
/// (no snapshot needed).
async fn handle_logs(ctx: &CmdContext<'_>) -> ResponseResult<()> {
    if !ctx.is_admin {
        ctx.bot.send_message(ctx.chat_id, "Not authorized.").await?;
        return Ok(());
    }

    let path = crate::logger::current_log_path();
    log::info!("Sending log file: {:?}", path);
    if !path.exists() {
        ctx.bot
            .send_message(ctx.chat_id, "No log file found.")
            .await?;
        return Ok(());
    }

    let doc = InputFile::file(&path).file_name("perbot.log");
    if let Err(e) = ctx.bot.send_document(ctx.chat_id, doc).await {
        log::error!("Failed to send logs to chat {}: {e}", ctx.chat_id.0);
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
    fn parse_event_callback_splits_id_and_action() {
        assert_eq!(parse_event_callback("eid:42:sn:30"), Some((42, "sn:30")));
        assert_eq!(parse_event_callback("eid:-7:del"), Some((-7, "del")));
        assert_eq!(parse_event_callback("eid:5:delyes"), Some((5, "delyes")));

        // Missing prefix, non-numeric id, no action separator.
        assert_eq!(parse_event_callback("ev:1:del"), None);
        assert_eq!(parse_event_callback("eid:x:del"), None);
        assert_eq!(parse_event_callback("eid:42"), None);
    }

    #[test]
    fn event_keyboards_embed_event_id_and_actions() {
        use teloxide::types::InlineKeyboardButtonKind::CallbackData;

        let datas = |kb: InlineKeyboardMarkup| -> Vec<String> {
            kb.inline_keyboard
                .concat()
                .iter()
                .map(|b| match &b.kind {
                    CallbackData(d) => d.clone(),
                    _ => panic!("expected callback data"),
                })
                .collect()
        };

        assert_eq!(
            datas(event_actions_keyboard(42)),
            ["eid:42:ed", "eid:42:del"]
        );
        assert_eq!(
            datas(delete_confirm_keyboard(42)),
            ["eid:42:delyes", "eid:42:delno"]
        );
        assert_eq!(datas(edit_cancel_keyboard(42)), ["eid:42:edno"]);
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
    fn parse_event_command_round_trips_and_rejects() {
        assert_eq!(parse_event_command("/event42", "perbot"), Some(42));
        assert_eq!(parse_event_command("  /event7  ", "perbot"), Some(7));
        assert_eq!(parse_event_command("/event42@perbot", "perbot"), Some(42));
        assert_eq!(parse_event_command("/event42@PerBot", "perbot"), Some(42));

        // The list command, missing/empty/non-numeric ids, and a foreign @bot.
        assert_eq!(parse_event_command("/events", "perbot"), None);
        assert_eq!(parse_event_command("/event", "perbot"), None);
        assert_eq!(parse_event_command("/eventabc", "perbot"), None);
        assert_eq!(parse_event_command("/event42@otherbot", "perbot"), None);
        assert_eq!(parse_event_command("not a command", "perbot"), None);
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
