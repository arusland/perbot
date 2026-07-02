//! The admin `/exit` command.

use super::CmdContext;
use std::process;

/// Shuts the bot down. Admin-only; non-admins get a rejection reply.
pub(super) async fn handle_exit(ctx: &CmdContext<'_>, arg: &str) -> anyhow::Result<()> {
    if !ctx.is_admin {
        ctx.bot
            .send_markdown(ctx.chat_id, "Not authorized\\.")
            .await?;
        return Ok(());
    }
    if arg.trim() != "yes" {
        ctx.bot
            .send_text(
                ctx.chat_id,
                "Admin command /exit must be run as \"/exit yes\"",
                None,
            )
            .await?;
        return Ok(());
    }
    log::info!("Received /exit command. Shutting down...");
    let _ = ctx
        .bot
        .send_text(ctx.admin_id, "Shutting down...", None)
        .await;
    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        process::exit(0);
    });
    Ok(())
}
