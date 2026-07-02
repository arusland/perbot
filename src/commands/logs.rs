//! The admin `/logs` command.

use super::CmdContext;

/// Sends the current log file back as a document. Admin-only; non-admins get a
/// rejection reply. The log file is append-only text, so it is sent directly
/// (no snapshot needed).
pub(super) async fn handle_logs(ctx: &CmdContext<'_>) -> anyhow::Result<()> {
    if !ctx.is_admin {
        ctx.bot
            .send_text(ctx.chat_id, "Not authorized.", None)
            .await?;
        return Ok(());
    }

    let path = crate::logger::current_log_path();
    log::info!("Sending log file: {:?}", path);
    if !path.exists() {
        ctx.bot
            .send_text(ctx.chat_id, "No log file found.", None)
            .await?;
        return Ok(());
    }

    if let Err(e) = ctx
        .bot
        .send_document(ctx.chat_id, &path, Some("perbot.log"))
        .await
    {
        log::error!("Failed to send logs to chat {}: {e}", ctx.chat_id.0);
    }
    Ok(())
}
