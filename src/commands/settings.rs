//! The `/settings` command and the `st:` Settings-menu callbacks (the menu
//! itself lives in `view/settings.rs`): the command and `st:o` open the menu
//! as a new message (keeping the /events list or /help reply it was pressed
//! under), `st:s` re-renders it in place (the picker's Back button),
//! `st:on`/`st:off` toggle the morning digest, `st:t` opens the hour picker,
//! `st:h:<H>` sets the digest hour (which also turns the digest on). Every
//! change applies to the chat the pressed message lives in — the same
//! authority model as list pagination.

use super::CmdContext;
use crate::state::EventProvider;
use crate::tgbot::TgBot;
use crate::view::{
    DEFAULT_DIGEST_HOUR, DIGEST_TIME_ASK, digest_time_keyboard, settings_keyboard, settings_message,
};
use chrono::NaiveTime;
use teloxide::types::{CallbackQuery, ChatId, MessageId};

/// The decoded `st:` callback actions.
enum SettingsAction {
    /// `st:o` — open the menu as a new message.
    Open,
    /// `st:s` — re-render the menu in place (Back from the hour picker).
    Show,
    /// `st:on` — switch the morning digest on at the default hour.
    DigestOn,
    /// `st:off` — switch the morning digest off.
    DigestOff,
    /// `st:t` — swap the menu for the digest hour picker.
    PickTime,
    /// `st:h:<H>` — set the digest hour (0..=23).
    SetHour(u32),
}

/// Decodes an `st:` callback payload. `None` for anything malformed.
fn parse_settings_callback(data: &str) -> Option<SettingsAction> {
    match data.strip_prefix("st:")? {
        "o" => Some(SettingsAction::Open),
        "s" => Some(SettingsAction::Show),
        "on" => Some(SettingsAction::DigestOn),
        "off" => Some(SettingsAction::DigestOff),
        "t" => Some(SettingsAction::PickTime),
        rest => {
            let hour = rest.strip_prefix("h:")?.parse::<u32>().ok()?;
            (hour < 24).then_some(SettingsAction::SetHour(hour))
        }
    }
}

/// Sends the current Settings menu as a new message.
async fn send_settings_menu(
    bot: &TgBot,
    provider: &EventProvider,
    chat_id: ChatId,
) -> anyhow::Result<()> {
    let digest = provider.digest_time(chat_id.0)?;
    bot.send_html(
        chat_id,
        settings_message(digest),
        Some(settings_keyboard(digest)),
    )
    .await?;
    Ok(())
}

/// Handles `/settings`: opens the Settings menu.
pub async fn handle_settings(ctx: &CmdContext<'_>) -> anyhow::Result<()> {
    send_settings_menu(ctx.bot, ctx.provider, ctx.chat_id).await
}

/// Edits the pressed message into the current Settings menu.
async fn render_settings_in_place(
    bot: &TgBot,
    provider: &EventProvider,
    chat_id: ChatId,
    message_id: MessageId,
) -> anyhow::Result<()> {
    let digest = provider.digest_time(chat_id.0)?;
    if let Err(e) = bot
        .edit_html(
            chat_id,
            message_id,
            settings_message(digest),
            Some(settings_keyboard(digest)),
        )
        .await
    {
        // "message is not modified" (e.g. double-tap) is benign; just log.
        log::warn!("Failed to render settings for chat {}: {e}", chat_id.0);
    }
    Ok(())
}

/// Handles an `st:`-prefixed callback: opening sends a fresh menu message;
/// every other action edits the menu message in place. Malformed payloads are
/// acknowledged and ignored.
pub async fn handle_settings_callback(
    bot: &TgBot,
    provider: &EventProvider,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    let Some(message) = q.regular_message() else {
        // Message is too old/inaccessible to act on.
        bot.answer_callback(q.id, None).await?;
        return Ok(());
    };
    let chat_id = message.chat.id;
    let message_id = message.id;

    match q.data.as_deref().and_then(parse_settings_callback) {
        Some(SettingsAction::Open) => {
            bot.answer_callback(q.id, None).await?;
            send_settings_menu(bot, provider, chat_id).await?;
        }
        Some(SettingsAction::Show) => {
            bot.answer_callback(q.id, None).await?;
            render_settings_in_place(bot, provider, chat_id, message_id).await?;
        }
        Some(SettingsAction::DigestOn) => {
            let time = NaiveTime::from_hms_opt(DEFAULT_DIGEST_HOUR, 0, 0).unwrap();
            provider.set_digest_time(chat_id.0, Some(time))?;
            bot.answer_callback(q.id, Some("🌅 Morning digest is on.".to_owned()))
                .await?;
            render_settings_in_place(bot, provider, chat_id, message_id).await?;
        }
        Some(SettingsAction::DigestOff) => {
            provider.set_digest_time(chat_id.0, None)?;
            bot.answer_callback(q.id, Some("Morning digest is off.".to_owned()))
                .await?;
            render_settings_in_place(bot, provider, chat_id, message_id).await?;
        }
        Some(SettingsAction::PickTime) => {
            bot.answer_callback(q.id, None).await?;
            if let Err(e) = bot
                .edit_html(
                    chat_id,
                    message_id,
                    DIGEST_TIME_ASK,
                    Some(digest_time_keyboard()),
                )
                .await
            {
                log::warn!(
                    "Failed to show digest time picker for chat {}: {e}",
                    chat_id.0
                );
            }
        }
        Some(SettingsAction::SetHour(hour)) => {
            let time = NaiveTime::from_hms_opt(hour, 0, 0).unwrap();
            provider.set_digest_time(chat_id.0, Some(time))?;
            bot.answer_callback(q.id, Some(format!("🕗 Digest time set to {hour:02}:00.")))
                .await?;
            render_settings_in_place(bot, provider, chat_id, message_id).await?;
        }
        None => {
            bot.answer_callback(q.id, None).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_settings_callback_decodes_envelope() {
        assert!(matches!(
            parse_settings_callback("st:o"),
            Some(SettingsAction::Open)
        ));
        assert!(matches!(
            parse_settings_callback("st:s"),
            Some(SettingsAction::Show)
        ));
        assert!(matches!(
            parse_settings_callback("st:on"),
            Some(SettingsAction::DigestOn)
        ));
        assert!(matches!(
            parse_settings_callback("st:off"),
            Some(SettingsAction::DigestOff)
        ));
        assert!(matches!(
            parse_settings_callback("st:t"),
            Some(SettingsAction::PickTime)
        ));
        assert!(matches!(
            parse_settings_callback("st:h:0"),
            Some(SettingsAction::SetHour(0))
        ));
        assert!(matches!(
            parse_settings_callback("st:h:23"),
            Some(SettingsAction::SetHour(23))
        ));

        // Malformed: out-of-range hour, junk, foreign envelopes.
        assert!(parse_settings_callback("st:h:24").is_none());
        assert!(parse_settings_callback("st:h:x").is_none());
        assert!(parse_settings_callback("st:").is_none());
        assert!(parse_settings_callback("st:x").is_none());
        assert!(parse_settings_callback("tz:r").is_none());
    }
}
