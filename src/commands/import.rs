//! The admin `/import <user_id>` command: records the pending target, then
//! [`handle_import_zip`] processes the zip of legacy `.alert` files the admin
//! sends next (the conversion itself lives in `crate::import`).

use super::CmdContext;
use crate::import;
use crate::state::EventProvider;
use crate::tgbot::TgBot;
use teloxide::types::{ChatId, FileId};

/// Begins a legacy import for `user_id`. Admin-only; records the pending target
/// and asks the admin to send the zip of `.alert` files next.
pub(super) async fn handle_import(ctx: &CmdContext<'_>, user_id: i64) -> anyhow::Result<()> {
    if !ctx.is_admin {
        ctx.bot
            .send_text(ctx.chat_id, "Not authorized.", None)
            .await?;
        return Ok(());
    }
    *ctx.pending_import.lock().unwrap() = Some(user_id);
    ctx.bot
        .send_text(
            ctx.chat_id,
            format!("Send the .zip of legacy alerts now to import them for chat {user_id}."),
            None,
        )
        .await?;
    Ok(())
}

/// Downloads the admin's zip, imports the legacy alerts for `target`, and replies
/// with a summary plus the HTML report as a document. Driven from `main` when the
/// admin sends the zip after `/import <user_id>`.
pub async fn handle_import_zip(
    bot: &TgBot,
    provider: &EventProvider,
    chat_id: ChatId,
    target: i64,
    file_id: FileId,
) -> anyhow::Result<()> {
    let file = bot.get_file(file_id).await?;
    let mut buf: Vec<u8> = Vec::new();
    if let Err(e) = bot.download_file(&file.path, &mut buf).await {
        bot.send_text(chat_id, format!("Failed to download the zip: {e}"), None)
            .await?;
        return Ok(());
    }

    bot.send_text(chat_id, "Importing events from file...", None)
        .await?;

    match import::import_zip(provider, target, &buf) {
        Ok(outcome) => {
            let report_path = std::env::temp_dir().join("perbot-legacy-report.html");
            bot.send_text(chat_id, outcome.summary(), None).await?;
            match std::fs::write(&report_path, &outcome.html) {
                Ok(()) => {
                    bot.send_document(chat_id, &report_path, None).await?;
                }
                Err(e) => {
                    bot.send_text(chat_id, format!("Failed to write report: {e}"), None)
                        .await?;
                }
            }
        }
        Err(e) => {
            bot.send_text(chat_id, format!("Import failed: {e}"), None)
                .await?;
        }
    }
    Ok(())
}
