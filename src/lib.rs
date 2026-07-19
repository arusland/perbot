pub mod commands;

use commands::CmdContext;
use types::{EventInfo, NextSource};

/// Confirms a freshly stored event: the captioned detail view
/// (`view::scheduled_message`, or `view::updated_message` when `updated` —
/// the edit-completion flow) with the same action keyboard as `/event<id>`
/// when a launch is scheduled, or the inactive-event notice (echoing the
/// user's input) with no keyboard. Shared by both completion flows in
/// `pending` and `main`'s fresh-parse path — every scheduling path confirms
/// through it.
pub async fn send_schedule_confirmation(
    ctx: &CmdContext<'_>,
    stored: &EventInfo,
    updated: bool,
) -> anyhow::Result<()> {
    if stored.next_datetime.is_some() {
        let now = chrono::Utc::now().naive_utc();
        let is_repetition = stored.source == Some(NextSource::Repetition);
        let text = if updated {
            view::updated_message(stored, now, ctx.tz, ctx.loc)
        } else {
            view::scheduled_message(stored, now, ctx.tz, ctx.loc)
        };
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
    } else {
        ctx.bot
            .send_html(ctx.chat_id, view::inactive_event_reply(ctx.text), None)
            .await?;
    }
    Ok(())
}
pub mod converter;
pub mod error;
pub mod import;
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
