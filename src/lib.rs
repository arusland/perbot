pub mod commands;

use commands::CmdContext;
use types::{EventInfo, NextSource};

/// Confirms a freshly created event: the `Event created` detail view
/// ([`send_event_detail_confirmation`]) when a launch is scheduled, or the
/// inactive-event notice (echoing the user's input) with no keyboard. Shared by
/// the time-only completion flow in `pending` and `main`'s fresh-parse path.
pub async fn send_schedule_confirmation(
    ctx: &CmdContext<'_>,
    stored: &EventInfo,
) -> anyhow::Result<()> {
    if stored.next_datetime.is_some() {
        send_event_detail_confirmation(ctx, stored, false).await
    } else {
        ctx.bot
            .send_html(ctx.chat_id, view::inactive_event_reply(ctx.text), None)
            .await?;
        Ok(())
    }
}

/// Sends the captioned detail view of a stored event (`view::scheduled_message`,
/// or `view::updated_message` when `updated`) with the same action keyboard as
/// `/event<id>` — inactive events included (the detail renders the
/// out-of-date / dismissed notice). Both edit completions (full and text-only)
/// confirm through this directly, so an edited event always gets the full
/// detail, never the bare inactive-input echo. While the chat has all events disabled (the
/// Settings toggle), an active event gets the `view::EVENTS_DISABLED_NOTE`
/// footer — it is stored but will not fire.
pub async fn send_event_detail_confirmation(
    ctx: &CmdContext<'_>,
    stored: &EventInfo,
    updated: bool,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().naive_utc();
    let is_repetition = stored.source == Some(NextSource::Repetition);
    let mut text = if updated {
        view::updated_message(stored, now, ctx.tz, ctx.loc)
    } else {
        view::scheduled_message(stored, now, ctx.tz, ctx.loc)
    };
    // Display-only footer: a read failure falls open (no note) rather than
    // failing the confirmation.
    if stored.active
        && !ctx
            .provider
            .is_activated(ctx.chat_id.0)
            .unwrap_or_else(|e| {
                log::warn!("Failed to read activation for chat {}: {e}", ctx.chat_id.0);
                true
            })
    {
        text.push_str("\n\n");
        text.push_str(view::EVENTS_DISABLED_NOTE);
    }
    ctx.bot
        .send_html(
            ctx.chat_id,
            text,
            Some(view::event_actions_keyboard(
                stored.id,
                stored.active,
                is_repetition,
            )),
        )
        .await?;
    Ok(())
}
pub mod converter;
pub mod error;
pub mod locale;
pub mod logger;
pub mod parser;
pub mod pending;
pub mod richtext;
pub mod scheduler;
pub mod state;
pub mod storage;
pub mod tgbot;
pub mod types;
pub mod tz;
pub mod view;
