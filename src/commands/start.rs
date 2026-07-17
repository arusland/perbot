//! The `/start` command.

use super::CmdContext;
use crate::view;

/// Replies with the welcome message: what the bot does, an example reminder,
/// and a pointer to /help.
pub(super) async fn handle_start(ctx: &CmdContext<'_>) -> anyhow::Result<()> {
    ctx.bot
        .send_html(ctx.chat_id, view::welcome_message(), None)
        .await?;
    Ok(())
}
