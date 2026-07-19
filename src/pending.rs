//! Interactive pending flows: the "send me the reminder text" completion for
//! time-only messages (e.g. a bare `13:30`) and the edit flow started from the
//! `/event<id>` view. Holds both the per-chat in-memory state and the
//! text-message logic that starts and completes the flows; `main` routes each
//! incoming text through the completion handlers before trying to parse a new
//! event. State is in-memory only; a restart simply drops pending requests.
//! The prompt strings and the Cancel keyboard the flows show live in
//! `crate::view`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::commands::CmdContext;
use crate::parser;
use crate::richtext;
use crate::send_schedule_confirmation;
use crate::types::EventInfo;
use crate::view::{self, clamp_message, edit_prompt};

/// Per-chat events awaiting a reminder body, keyed by chat id. The stored
/// [`EventInfo`] has its time/recurrence fields set and an empty `message`;
/// `chat_id`/`msg_id`/`message` are filled in when the body arrives.
pub type PendingMessage = Arc<Mutex<HashMap<i64, EventInfo>>>;

pub fn new_pending() -> PendingMessage {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Per-chat events being edited, keyed by chat id; the value is the id of the
/// event whose time and message the next message will replace. Set when the user
/// taps Edit on the `/event<id>` view and cleared when the edit completes or is
/// cancelled. In-memory only, like [`PendingMessage`].
pub type PendingEdit = Arc<Mutex<HashMap<i64, i64>>>;

pub fn new_pending_edit() -> PendingEdit {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Drops both pending flows for a chat. Used by `main`'s timezone gate: a chat
/// without a timezone can't schedule, so any held flow state is stale.
pub fn clear_chat(pending_msg: &PendingMessage, pending_edit: &PendingEdit, chat_id: i64) {
    pending_msg.lock().unwrap().remove(&chat_id);
    pending_edit.lock().unwrap().remove(&chat_id);
}

/// Completes a pending edit (the chat tapped Edit on an `/event<id>` view):
/// the message replaces the event's time and message. A time-only or
/// unparsable reply re-prompts instead of applying. Returns `true` when the
/// chat had a pending edit and the message was consumed by it.
pub async fn handle_edit_completion(ctx: &CmdContext<'_>, msg_id: i64) -> anyhow::Result<bool> {
    let editing = ctx
        .pending_edit
        .lock()
        .unwrap()
        .get(&ctx.chat_id.0)
        .copied();
    let Some(event_id) = editing else {
        return Ok(false);
    };

    // Re-load the event once and verify it still belongs to this chat; a
    // pending edit can outlive the event (deleted meanwhile).
    let Some(old) = ctx
        .provider
        .get_event(event_id)?
        .filter(|e| e.chat_id == ctx.chat_id.0)
    else {
        ctx.pending_edit.lock().unwrap().remove(&ctx.chat_id.0);
        ctx.bot
            .send_text(ctx.chat_id, "Event not found.", None)
            .await?;
        return Ok(true);
    };

    if let Some((mut event, spans)) = parser::parse_full(ctx.text, ctx.loc, ctx.tz) {
        let entities = ctx.msg.parse_entities().unwrap_or_default();
        event.id = old.id;
        event.chat_id = old.chat_id;
        event.created_at = old.created_at;
        event.msg_id = msg_id;
        event.legacy = old.legacy;
        event.parent = old.parent;
        let rendered = richtext::render_html(ctx.text, &spans, &entities);
        let (clamped, truncated) = clamp_message(&rendered);
        event.message = clamped;

        let stored = ctx.provider.update_event_and_get(event)?;
        ctx.pending_edit.lock().unwrap().remove(&ctx.chat_id.0);
        if truncated {
            ctx.bot
                .send_text(ctx.chat_id, view::MESSAGE_TRUNCATED, None)
                .await?;
        }
        send_schedule_confirmation(ctx, &stored, true).await?;
    } else {
        // A time-only or unparsable reply: re-prompt (keeping the pending
        // edit) with the current input still attached.
        let lead = if parser::parse_time_only(ctx.text, ctx.loc, ctx.tz).is_some() {
            view::EDIT_NEED_TEXT
        } else {
            view::EDIT_NEED_TIME
        };
        ctx.bot
            .send_html(
                ctx.chat_id,
                edit_prompt(lead, &old, ctx.loc),
                Some(view::edit_cancel_keyboard(event_id)),
            )
            .await?;
    }
    Ok(true)
}

/// Begins the time-only flow: holds the parsed body-less event for the chat
/// and asks for the reminder text, offering a Cancel button.
pub async fn request_body(ctx: &CmdContext<'_>, event: EventInfo) -> anyhow::Result<()> {
    ctx.pending_msg.lock().unwrap().insert(ctx.chat_id.0, event);
    ctx.bot
        .send_text(ctx.chat_id, view::ASK_TEXT, Some(view::cancel_keyboard()))
        .await?;
    Ok(())
}

/// Completes a pending "send me the reminder text": once a chat is waiting
/// for a body, the next non-command text is used verbatim as that body.
/// Returns `true` when the chat was waiting and the message was consumed.
pub async fn handle_body_completion(ctx: &CmdContext<'_>, msg_id: i64) -> anyhow::Result<bool> {
    let pending_event = ctx.pending_msg.lock().unwrap().remove(&ctx.chat_id.0);
    let Some(mut event) = pending_event else {
        return Ok(false);
    };

    let entities = ctx.msg.parse_entities().unwrap_or_default();
    // The whole reply text is the body, so a single span covers all of it.
    let span = 0..ctx.text.len();
    let body = richtext::render_html(ctx.text, std::slice::from_ref(&span), &entities);
    if body.is_empty() {
        // Whitespace-only reply carries no usable body: keep waiting and
        // re-prompt with the Cancel button.
        request_body(ctx, event).await?;
        return Ok(true);
    }
    event.chat_id = ctx.chat_id.0;
    event.msg_id = msg_id;
    let (clamped, truncated) = clamp_message(&body);
    event.message = clamped;
    let stored = ctx.provider.insert_event_and_get(event)?;
    if truncated {
        ctx.bot
            .send_text(ctx.chat_id, view::MESSAGE_TRUNCATED, None)
            .await?;
    }
    send_schedule_confirmation(ctx, &stored, false).await?;
    Ok(true)
}
