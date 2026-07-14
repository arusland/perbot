//! The `/help` command.

use super::{CmdContext, Command};
use crate::view;
use teloxide::utils::command::BotCommands;

/// Replies with the list of commands followed by tap-to-copy example
/// reminders. Admins additionally see admin-only commands.
pub(super) async fn handle_help(ctx: &CmdContext<'_>) -> anyhow::Result<()> {
    let help = view::help_message(&Command::descriptions().to_string(), ctx.is_admin);
    ctx.bot.send_html(ctx.chat_id, help, None).await?;
    Ok(())
}
