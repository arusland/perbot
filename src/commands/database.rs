//! The admin `/database` command.

use super::CmdContext;

/// Sends a consistent snapshot of the SQLite database back as a document.
/// Admin-only; non-admins get a rejection reply. The bot holds an open connection,
/// so we snapshot via `VACUUM INTO` (a temp file) rather than copying the live file,
/// then clean the snapshot up.
pub(super) async fn handle_database(ctx: &CmdContext<'_>) -> anyhow::Result<()> {
    if !ctx.is_admin {
        ctx.bot
            .send_text(ctx.chat_id, "Not authorized.", None)
            .await?;
        return Ok(());
    }

    let snapshot = std::env::temp_dir().join("perbot-db-snapshot.sqlite");
    // VACUUM INTO requires the destination not to exist.
    let _ = std::fs::remove_file(&snapshot);
    if let Err(e) = ctx.provider.backup_database(&snapshot) {
        log::error!("Failed to snapshot database: {e}");
        ctx.bot
            .send_text(
                ctx.chat_id,
                format!("Failed to snapshot database: {e}"),
                None,
            )
            .await?;
        return Ok(());
    }

    if let Err(e) = ctx
        .bot
        .send_document(ctx.chat_id, &snapshot, Some("perbot.db"))
        .await
    {
        log::error!("Failed to send database to chat {}: {e}", ctx.chat_id.0);
    }
    let _ = std::fs::remove_file(&snapshot);
    Ok(())
}
