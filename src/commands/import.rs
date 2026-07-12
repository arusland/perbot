//! The admin `/import <user_id> <timezone>` command: records the pending
//! target and the timezone the legacy wall-clock data is read in, then
//! [`handle_import_zip`] processes the zip of legacy `.alert` files the admin
//! sends next (the conversion itself lives in `crate::import`).

use super::CmdContext;
use crate::import;
use crate::state::EventProvider;
use crate::tgbot::TgBot;
use chrono_tz::Tz;
use teloxide::types::{ChatId, FileId};

const IMPORT_USAGE: &str = "Usage: /import <user_id> <timezone>, e.g. /import 12345 Europe/Berlin";

/// Parses the `/import` arguments: exactly a chat id and an IANA timezone.
fn parse_import_args(args: &str) -> Option<(i64, Tz)> {
    let mut words = args.split_whitespace();
    let user_id = words.next()?.parse().ok()?;
    let tz = crate::tz::parse_tz(words.next()?)?;
    if words.next().is_some() {
        return None;
    }
    Some((user_id, tz))
}

/// Begins a legacy import. Admin-only; records the pending target chat and
/// timezone and asks the admin to send the zip of `.alert` files next.
pub(super) async fn handle_import(ctx: &CmdContext<'_>, args: &str) -> anyhow::Result<()> {
    if !ctx.is_admin {
        ctx.bot
            .send_text(ctx.chat_id, "Not authorized.", None)
            .await?;
        return Ok(());
    }
    let Some((user_id, tz)) = parse_import_args(args) else {
        ctx.bot.send_text(ctx.chat_id, IMPORT_USAGE, None).await?;
        return Ok(());
    };
    *ctx.pending_import.lock().unwrap() = Some((user_id, tz));
    ctx.bot
        .send_text(
            ctx.chat_id,
            format!("Send the .zip of legacy alerts now to import them for chat {user_id} ({tz})."),
            None,
        )
        .await?;
    Ok(())
}

/// Downloads the admin's zip, imports the legacy alerts for `target` reading
/// their wall-clock data in `tz`, and replies with a summary plus the HTML
/// report as a document. Driven from `main` when the admin sends the zip after
/// `/import <user_id> <timezone>`.
pub async fn handle_import_zip(
    bot: &TgBot,
    provider: &EventProvider,
    chat_id: ChatId,
    target: i64,
    tz: Tz,
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

    match import::import_zip(provider, target, &buf, tz) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_import_args_accepts_id_and_timezone() {
        assert_eq!(
            parse_import_args("123 Europe/Berlin"),
            Some((123, Tz::Europe__Berlin))
        );
        assert_eq!(parse_import_args("  -42   UTC  "), Some((-42, Tz::UTC)));
    }

    #[test]
    fn parse_import_args_rejects_bad_input() {
        assert_eq!(parse_import_args(""), None);
        assert_eq!(parse_import_args("123"), None);
        assert_eq!(parse_import_args("123 Nowhere/Nothing"), None);
        assert_eq!(parse_import_args("abc Europe/Berlin"), None);
        assert_eq!(parse_import_args("123 Europe/Berlin extra"), None);
    }
}
