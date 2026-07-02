//! The `/help` command.

use super::{CmdContext, Command};
use teloxide::utils::command::BotCommands;

/// Replies with the list of commands. Admins additionally see admin-only commands.
pub(super) async fn handle_help(ctx: &CmdContext<'_>) -> anyhow::Result<()> {
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
    ctx.bot.send_text(ctx.chat_id, help, None).await?;
    Ok(())
}
