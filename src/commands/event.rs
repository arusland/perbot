//! The single-event detail view (`/event<id>`) and its `eid:<id>:<action>`
//! callbacks: dismiss / dismiss-repetition, the delete confirmation flow, and
//! the edit flow. Snooze (`sn:<minutes>`) is dispatched from here to
//! [`super::snooze`].

use crate::pending::PendingEdit;
use crate::state::{DismissOutcome, EventProvider};
use crate::tgbot::TgBot;
use crate::types::{EventInfo, NextSource};
use crate::view::{
    EDIT_ASK_TEXT, delete_confirm_keyboard, edit_cancel_keyboard, edit_prompt,
    event_actions_keyboard, event_detail, notification_keyboard,
};
use chrono::Utc;
use teloxide::types::{CallbackQuery, ChatId, InlineKeyboardMarkup};

/// Parses a `/event<id>` (or `/event<id>@<bot_username>`) command into the event id.
///
/// `/event<id>` has no space between the name and its argument, so teloxide's
/// `BotCommands` derive can't parse it; it is matched manually here. Returns `None`
/// for anything else (including the bare `/events` list command, `/event` with no id,
/// a non-numeric id, or a mismatched `@bot` suffix).
pub fn parse_event_command(text: &str, bot_username: &str) -> Option<i64> {
    let token = text.split_whitespace().next()?;
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
    bot: &TgBot,
    provider: &EventProvider,
    chat_id: ChatId,
    id: i64,
) -> anyhow::Result<()> {
    match provider.get_event(id)? {
        Some(event) if event.chat_id == chat_id.0 => {
            let now = Utc::now().naive_utc();
            let loc = crate::locale::for_chat(chat_id.0);
            let tz = provider.tz_or_utc(chat_id.0);
            let is_repetition = event.source == Some(NextSource::Repetition);
            bot.send_html(
                chat_id,
                event_detail(&event, now, tz, loc),
                Some(event_actions_keyboard(id, event.active, is_repetition)),
            )
            .await?;
        }
        _ => {
            bot.send_text(chat_id, "Event not found.", None).await?;
        }
    }
    Ok(())
}

/// Decodes the event-specific callback envelope `eid:<id>:<action>` into the
/// event id and the action remainder (e.g. `sn:30`, `del`, `delyes`). Returns
/// `None` for anything not shaped like the envelope.
pub(super) fn parse_event_callback(data: &str) -> Option<(i64, &str)> {
    let rest = data.strip_prefix("eid:")?;
    let (id, action) = rest.split_once(':')?;
    Some((id.parse::<i64>().ok()?, action))
}

/// Dispatches an event-specific callback (`eid:<id>:<action>`) to the matching
/// handler: dismiss (`dis` → advance past the current occurrence), dismiss
/// repetition (`disr` → skip the interval fills to the next anchor), snooze
/// (`sn:<minutes>`; `snx` expands the collapsed notification keyboard to the
/// full snooze rows), the delete flow (`del` → confirm prompt, `delyes` → delete,
/// `delno` → restore the action buttons), or the edit flow (`ed` → start
/// editing, `edno` → cancel editing). The `:n`-suffixed dismiss/delete variants
/// are the notification-keyboard flavor that preserves the fired message.
/// Unknown actions are acknowledged and ignored. Routed from `main`'s
/// `eid:`-prefixed callback branch.
pub async fn handle_event_callback(
    bot: &TgBot,
    provider: &EventProvider,
    pending_edit: &PendingEdit,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    match q.data.as_deref().and_then(parse_event_callback) {
        Some((id, "dis")) => handle_dismiss(bot, provider, id, false, q).await,
        Some((id, "dis:n")) => handle_dismiss(bot, provider, id, true, q).await,
        Some((id, "disr")) => handle_dismiss_repetition(bot, provider, id, false, q).await,
        Some((id, "disr:n")) => handle_dismiss_repetition(bot, provider, id, true, q).await,
        Some((id, "del")) => handle_delete_prompt(bot, id, false, q).await,
        Some((id, "del:n")) => handle_delete_prompt(bot, id, true, q).await,
        Some((id, "delyes")) => handle_delete_confirm(bot, provider, id, false, q).await,
        Some((id, "delyes:n")) => handle_delete_confirm(bot, provider, id, true, q).await,
        Some((id, "delno")) => handle_delete_cancel(bot, provider, id, false, q).await,
        Some((id, "delno:n")) => handle_delete_cancel(bot, provider, id, true, q).await,
        Some((id, "ed")) => handle_edit_prompt(bot, provider, pending_edit, id, q).await,
        Some((_, "edno")) => handle_edit_cancel(bot, pending_edit, q).await,
        Some((id, "snx")) => super::snooze::handle_snooze_expand(bot, provider, id, q).await,
        Some((_, action)) if action.starts_with("sn:") => {
            super::snooze::handle_snooze_callback(bot, provider, q).await
        }
        _ => {
            bot.answer_callback(q.id, None).await?;
            Ok(())
        }
    }
}

/// Handles the `⏭ Dismiss` press (`eid:<id>:dis` / `eid:<id>:dis:n`): delegates
/// to [`EventProvider::dismiss`] (which advances the event past its current
/// occurrence and access-checks the chat), then re-renders the detail view in
/// place — or, for the notification flavor (`from_notification`), keeps the
/// fired text and refreshes only the keyboard. Replies with a toast for a
/// missing/foreign id or an already-inactive event.
async fn handle_dismiss(
    bot: &TgBot,
    provider: &EventProvider,
    id: i64,
    from_notification: bool,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    let Some(message) = q.regular_message() else {
        bot.answer_callback(q.id, None).await?;
        return Ok(());
    };
    let chat_id = message.chat.id;
    let message_id = message.id;

    match provider.dismiss(id, chat_id.0)? {
        DismissOutcome::NotFound => {
            bot.answer_callback(q.id, Some("Event not found.".to_owned()))
                .await?;
        }
        DismissOutcome::Inactive => {
            bot.answer_callback(q.id, Some("Nothing to dismiss.".to_owned()))
                .await?;
        }
        DismissOutcome::Dismissed(updated) => {
            bot.answer_callback(q.id, Some("⏭ Dismissed.".to_owned()))
                .await?;
            let tz = provider.tz_or_utc(chat_id.0);
            if let Err(e) = refresh_dismissed_view(
                bot,
                provider,
                chat_id,
                message_id,
                &updated,
                tz,
                from_notification,
            )
            .await
            {
                log::warn!("Failed to refresh dismissed event {id}: {e}");
            }
        }
    }
    Ok(())
}

/// Handles the `⏩ Dismiss repetition` press (`eid:<id>:disr` / `eid:<id>:disr:n`):
/// delegates to [`EventProvider::dismiss_repetition`] (which skips the event's
/// repetition fills to the next anchor and access-checks the chat), then
/// re-renders the detail view in place — or, for the notification flavor
/// (`from_notification`), keeps the fired text and refreshes only the keyboard.
/// Replies with a toast for a missing/foreign id or an already-inactive event.
async fn handle_dismiss_repetition(
    bot: &TgBot,
    provider: &EventProvider,
    id: i64,
    from_notification: bool,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    let Some(message) = q.regular_message() else {
        bot.answer_callback(q.id, None).await?;
        return Ok(());
    };
    let chat_id = message.chat.id;
    let message_id = message.id;

    match provider.dismiss_repetition(id, chat_id.0)? {
        DismissOutcome::NotFound => {
            bot.answer_callback(q.id, Some("Event not found.".to_owned()))
                .await?;
        }
        DismissOutcome::Inactive => {
            bot.answer_callback(q.id, Some("Nothing to dismiss.".to_owned()))
                .await?;
        }
        DismissOutcome::Dismissed(updated) => {
            bot.answer_callback(q.id, Some("⏩ Repetition dismissed.".to_owned()))
                .await?;
            let tz = provider.tz_or_utc(chat_id.0);
            if let Err(e) = refresh_dismissed_view(
                bot,
                provider,
                chat_id,
                message_id,
                &updated,
                tz,
                from_notification,
            )
            .await
            {
                log::warn!("Failed to refresh dismissed-repetition event {id}: {e}");
            }
        }
    }
    Ok(())
}

/// Refreshes the message a dismiss was pressed on so its buttons match the
/// advanced schedule: on the detail view the whole message is re-rendered with
/// fresh action buttons; on a fired notification (`from_notification`) the
/// fired text is kept and only the keyboard is rebuilt (in its collapsed form).
async fn refresh_dismissed_view(
    bot: &TgBot,
    provider: &EventProvider,
    chat_id: ChatId,
    message_id: teloxide::types::MessageId,
    updated: &EventInfo,
    tz: chrono_tz::Tz,
    from_notification: bool,
) -> teloxide::prelude::ResponseResult<()> {
    let is_repetition = updated.source == Some(NextSource::Repetition);
    if from_notification {
        bot.edit_markup(
            chat_id,
            message_id,
            notification_keyboard(
                updated.id,
                updated.active,
                is_repetition,
                provider.last_snooze(chat_id.0),
            ),
        )
        .await
    } else {
        let loc = crate::locale::for_chat(chat_id.0);
        let text = event_detail(updated, Utc::now().naive_utc(), tz, loc);
        bot.edit_html(
            chat_id,
            message_id,
            text.as_str(),
            Some(event_actions_keyboard(
                updated.id,
                updated.active,
                is_repetition,
            )),
        )
        .await
    }
}

/// Resolves which event an Edit press on `id` actually targets: the event
/// itself, or its parent when `id` is a snoozed child (a snooze owns only its
/// time — its text lives on the parent, so edits go there). Both the pressed
/// event and the resolved parent are access-checked against `chat_id`; a
/// missing or foreign event (or dangling parent) resolves to `None`.
fn resolve_edit_target(
    provider: &EventProvider,
    id: i64,
    chat_id: i64,
) -> crate::error::Result<Option<EventInfo>> {
    let Some(event) = provider.get_event(id)?.filter(|e| e.chat_id == chat_id) else {
        return Ok(None);
    };
    match event.parent {
        Some(pid) => Ok(provider.get_event(pid)?.filter(|p| p.chat_id == chat_id)),
        None => Ok(Some(event)),
    }
}

/// Handles the `✏️ Edit` press (`eid:<id>:ed`): access-checks the event against
/// the chat the button was pressed in (callback ids are user-influenceable),
/// records the chat as editing that event, and prompts for the replacement input
/// with the event's current input as a copyable `<code>` block ([`edit_prompt`])
/// and a Cancel button. An Edit press on a snoozed event starts the flow for its
/// parent instead ([`resolve_edit_target`]). Replies "Event not found." for a
/// missing or foreign id.
async fn handle_edit_prompt(
    bot: &TgBot,
    provider: &EventProvider,
    pending_edit: &PendingEdit,
    id: i64,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    let Some(message) = q.regular_message() else {
        bot.answer_callback(q.id, None).await?;
        return Ok(());
    };
    let chat_id = message.chat.id;

    // Editing re-parses a time expression, which needs a configured timezone;
    // a pre-existing-DB chat that never picked one gets the picker instead.
    if provider.get_timezone(chat_id.0)?.is_none() {
        bot.answer_callback(q.id, None).await?;
        bot.send_html(
            chat_id,
            crate::view::TZ_REQUIRED,
            Some(crate::view::timezone_regions_keyboard()),
        )
        .await?;
        return Ok(());
    }

    let event = resolve_edit_target(provider, id, chat_id.0)?;
    bot.answer_callback(q.id, None).await?;
    if let Some(event) = event {
        pending_edit.lock().unwrap().insert(chat_id.0, event.id);
        let loc = crate::locale::for_chat(chat_id.0);
        bot.send_html(
            chat_id,
            edit_prompt(EDIT_ASK_TEXT, &event, loc),
            Some(edit_cancel_keyboard(event.id)),
        )
        .await?;
    } else {
        bot.send_text(chat_id, "Event not found.", None).await?;
    }
    Ok(())
}

/// Handles the Cancel press while editing (`eid:<id>:edno`): drops the chat's
/// pending edit and edits the prompt to "❌ Cancelled." (clearing the keyboard).
async fn handle_edit_cancel(
    bot: &TgBot,
    pending_edit: &PendingEdit,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    bot.answer_callback(q.id.clone(), None).await?;

    let Some(message) = q.regular_message() else {
        return Ok(());
    };
    let chat_id = message.chat.id;
    pending_edit.lock().unwrap().remove(&chat_id.0);

    if let Err(e) = bot.edit_text(chat_id, message.id, "❌ Cancelled.").await {
        log::warn!(
            "Failed to edit cancelled edit prompt for chat {}: {e}",
            chat_id.0
        );
    }
    Ok(())
}

/// Handles the `🗑 Delete` press (`eid:<id>:del` / `eid:<id>:del:n`): swaps the
/// keyboard in place for the confirm/cancel row, leaving the message text
/// untouched. `from_notification` is carried into the confirm keyboard so the
/// follow-up presses know which flavor of the flow they belong to.
async fn handle_delete_prompt(
    bot: &TgBot,
    id: i64,
    from_notification: bool,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    if let Some(message) = q.regular_message()
        && let Err(e) = bot
            .edit_markup(
                message.chat.id,
                message.id,
                delete_confirm_keyboard(id, from_notification),
            )
            .await
    {
        log::warn!("Failed to show delete confirmation for event {id}: {e}");
    }
    bot.answer_callback(q.id, None).await?;
    Ok(())
}

/// Handles the `❌ Cancel` press (`eid:<id>:delno` / `eid:<id>:delno:n`): restores
/// the keyboard the flow started from — the collapsed notification keyboard for
/// the `:n` variant, otherwise the Dismiss/Edit/Delete action buttons — leaving the
/// message text untouched. For the detail view the event is reloaded so the
/// restored keyboard reflects its current active state (whether the Dismiss
/// button belongs there).
async fn handle_delete_cancel(
    bot: &TgBot,
    provider: &EventProvider,
    id: i64,
    from_notification: bool,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    if let Some(message) = q.regular_message() {
        let event = provider.get_event(id)?;
        let active = event.as_ref().is_some_and(|e| e.active);
        let is_repetition = event
            .as_ref()
            .is_some_and(|e| e.source == Some(NextSource::Repetition));
        let markup = if from_notification {
            notification_keyboard(
                id,
                active,
                is_repetition,
                provider.last_snooze(message.chat.id.0),
            )
        } else {
            event_actions_keyboard(id, active, is_repetition)
        };
        if let Err(e) = bot.edit_markup(message.chat.id, message.id, markup).await {
            log::warn!("Failed to restore delete button for event {id}: {e}");
        }
    }
    bot.answer_callback(q.id, None).await?;
    Ok(())
}

/// Handles the `✅ Yes, delete` press (`eid:<id>:delyes` / `eid:<id>:delyes:n`):
/// access-checks the event against the chat the button was pressed in (callback
/// ids are user-influenceable) and deletes it. On the detail view the message is
/// edited to a confirmation (clearing the keyboard); from a notification the
/// fired reminder text is kept — only the keyboard is removed and the outcome is
/// delivered as a toast. Replies "Event not found." for a missing or foreign id.
async fn handle_delete_confirm(
    bot: &TgBot,
    provider: &EventProvider,
    id: i64,
    from_notification: bool,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    let Some(message) = q.regular_message() else {
        bot.answer_callback(q.id, None).await?;
        return Ok(());
    };
    let chat_id = message.chat.id;
    let message_id = message.id;

    let owned = matches!(provider.get_event(id)?, Some(event) if event.chat_id == chat_id.0);
    let text = if owned && provider.delete(id)? {
        "🗑 Event deleted."
    } else {
        "Event not found."
    };

    if from_notification {
        bot.answer_callback(q.id, Some(text.to_owned())).await?;
        if let Err(e) = bot
            .edit_markup(chat_id, message_id, InlineKeyboardMarkup::default())
            .await
        {
            log::warn!("Failed to clear deleted-event notification keyboard for event {id}: {e}");
        }
    } else {
        bot.answer_callback(q.id, None).await?;
        if let Err(e) = bot.edit_text(chat_id, message_id, text).await {
            log::warn!("Failed to edit deleted-event message for event {id}: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an in-memory provider with one chat, a backing message row, and a
    /// parent event, returning the provider plus the parent's id.
    fn provider_with_parent(chat_id: i64) -> (EventProvider, i64) {
        use crate::storage::EventStorage;
        use crate::types::{ChatInfo, ChatType};
        let provider = EventProvider::new(EventStorage::open_in_memory().unwrap());
        provider
            .upsert_chat(&ChatInfo {
                id: chat_id,
                chat_type: ChatType::Private,
                title: None,
                username: None,
                first_name: None,
                last_name: None,
                updated_at: None,
                created_at: None,
            })
            .unwrap();
        let msg_id = provider.insert_message(None, chat_id, "call mom").unwrap();
        let mut parent = crate::view::test_support::sample_event("call mom", None);
        parent.chat_id = chat_id;
        parent.msg_id = msg_id;
        let parent_id = provider.insert_prebuilt_event(&parent).unwrap();
        (provider, parent_id)
    }

    #[test]
    fn resolve_edit_target_follows_parent() {
        let chat_id = 42;
        let (provider, parent_id) = provider_with_parent(chat_id);
        let mut child = crate::view::test_support::sample_event("", None);
        child.chat_id = chat_id;
        child.msg_id = provider.get_event(parent_id).unwrap().unwrap().msg_id;
        child.parent = Some(parent_id);
        let child_id = provider.insert_prebuilt_event(&child).unwrap();

        // A root event resolves to itself; a snoozed child resolves to its parent.
        let target = resolve_edit_target(&provider, parent_id, chat_id)
            .unwrap()
            .unwrap();
        assert_eq!(target.id, parent_id);
        let target = resolve_edit_target(&provider, child_id, chat_id)
            .unwrap()
            .unwrap();
        assert_eq!(target.id, parent_id);

        // Foreign chats never resolve.
        assert!(
            resolve_edit_target(&provider, child_id, chat_id + 1)
                .unwrap()
                .is_none()
        );
        // A missing id resolves to None.
        assert!(
            resolve_edit_target(&provider, 9999, chat_id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn parse_event_callback_splits_id_and_action() {
        assert_eq!(parse_event_callback("eid:42:sn:30"), Some((42, "sn:30")));
        assert_eq!(parse_event_callback("eid:42:snx"), Some((42, "snx")));
        assert_eq!(parse_event_callback("eid:-7:del"), Some((-7, "del")));
        assert_eq!(parse_event_callback("eid:5:delyes"), Some((5, "delyes")));
        assert_eq!(parse_event_callback("eid:5:del:n"), Some((5, "del:n")));
        assert_eq!(
            parse_event_callback("eid:5:delyes:n"),
            Some((5, "delyes:n"))
        );
        assert_eq!(parse_event_callback("eid:5:delno:n"), Some((5, "delno:n")));
        assert_eq!(parse_event_callback("eid:42:dis"), Some((42, "dis")));
        assert_eq!(parse_event_callback("eid:42:disr"), Some((42, "disr")));
        assert_eq!(parse_event_callback("eid:42:dis:n"), Some((42, "dis:n")));
        assert_eq!(parse_event_callback("eid:42:disr:n"), Some((42, "disr:n")));

        // Missing prefix, non-numeric id, no action separator.
        assert_eq!(parse_event_callback("ev:1:del"), None);
        assert_eq!(parse_event_callback("eid:x:del"), None);
        assert_eq!(parse_event_callback("eid:42"), None);
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
}
