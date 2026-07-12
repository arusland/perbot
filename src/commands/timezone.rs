//! The `/timezone` command and its `tz:` picker callbacks: `tz:r` re-opens the
//! region list, `tz:g:<Region>:<page>` shows a city page, `tz:p:<Zone>` stores
//! the pick via [`EventProvider::set_timezone`] (which re-anchors the chat's
//! active events to the new zone's wall clock). The setting applies to the chat
//! the pressed picker message lives in — the same authority model as list
//! pagination — so no id-based access check is needed.

use super::CmdContext;
use crate::state::EventProvider;
use crate::tgbot::TgBot;
use crate::view::{
    timezone_cities_keyboard, timezone_current_message, timezone_regions_keyboard,
    timezone_set_message,
};
use teloxide::types::CallbackQuery;

/// Handles `/timezone`: shows the current setting and the region picker.
pub(super) async fn handle_timezone(ctx: &CmdContext<'_>) -> anyhow::Result<()> {
    let current = ctx.provider.get_timezone(ctx.chat_id.0)?;
    ctx.bot
        .send_html(
            ctx.chat_id,
            timezone_current_message(current),
            Some(timezone_regions_keyboard()),
        )
        .await?;
    Ok(())
}

/// The decoded `tz:` callback actions.
enum TzAction<'a> {
    Regions,
    Cities { region: &'a str, page: usize },
    Pick(&'a str),
}

/// Decodes `tz:r` / `tz:g:<Region>:<page>` / `tz:p:<Zone>`. `None` for
/// anything else.
fn parse_tz_callback(data: &str) -> Option<TzAction<'_>> {
    let rest = data.strip_prefix("tz:")?;
    if rest == "r" {
        return Some(TzAction::Regions);
    }
    if let Some(zone) = rest.strip_prefix("p:") {
        return Some(TzAction::Pick(zone));
    }
    let spec = rest.strip_prefix("g:")?;
    let (region, page) = spec.rsplit_once(':')?;
    Some(TzAction::Cities {
        region,
        page: page.parse().ok()?,
    })
}

/// Handles a `tz:`-prefixed callback: navigation edits only the picker
/// message's keyboard in place; a pick stores the zone, reschedules the chat's
/// events, and replaces the message with the confirmation. Malformed or
/// unknown payloads are acknowledged and ignored.
pub async fn handle_timezone_callback(
    bot: &TgBot,
    provider: &EventProvider,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    let Some(message) = q.regular_message() else {
        bot.answer_callback(q.id, None).await?;
        return Ok(());
    };
    let chat_id = message.chat.id;
    let message_id = message.id;

    match q.data.as_deref().and_then(parse_tz_callback) {
        Some(TzAction::Regions) => {
            bot.answer_callback(q.id, None).await?;
            if let Err(e) = bot
                .edit_markup(chat_id, message_id, timezone_regions_keyboard())
                .await
            {
                log::warn!(
                    "Failed to show timezone regions for chat {}: {e}",
                    chat_id.0
                );
            }
        }
        Some(TzAction::Cities { region, page }) => {
            bot.answer_callback(q.id, None).await?;
            let Some(keyboard) = timezone_cities_keyboard(region, page) else {
                return Ok(());
            };
            if let Err(e) = bot.edit_markup(chat_id, message_id, keyboard).await {
                log::warn!(
                    "Failed to show timezone cities {region}:{page} for chat {}: {e}",
                    chat_id.0
                );
            }
        }
        Some(TzAction::Pick(zone)) => {
            let Some(tz) = crate::tz::parse_tz(zone) else {
                bot.answer_callback(q.id, Some("Unknown timezone.".to_owned()))
                    .await?;
                return Ok(());
            };
            let rescheduled = provider.set_timezone(chat_id.0, tz)?;
            bot.answer_callback(q.id, Some(format!("Timezone set to {}.", tz.name())))
                .await?;
            if let Err(e) = bot
                .edit_html(
                    chat_id,
                    message_id,
                    timezone_set_message(tz, rescheduled),
                    None,
                )
                .await
            {
                log::warn!(
                    "Failed to confirm timezone pick for chat {}: {e}",
                    chat_id.0
                );
            }
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
    fn parse_tz_callback_decodes_envelope() {
        assert!(matches!(parse_tz_callback("tz:r"), Some(TzAction::Regions)));
        assert!(matches!(
            parse_tz_callback("tz:g:America:3"),
            Some(TzAction::Cities {
                region: "America",
                page: 3
            })
        ));
        assert!(matches!(
            parse_tz_callback("tz:p:America/Argentina/Buenos_Aires"),
            Some(TzAction::Pick("America/Argentina/Buenos_Aires"))
        ));
        assert!(matches!(
            parse_tz_callback("tz:p:UTC"),
            Some(TzAction::Pick("UTC"))
        ));

        // Malformed: wrong prefix, missing page, non-numeric page.
        assert!(parse_tz_callback("eid:1:dis").is_none());
        assert!(parse_tz_callback("tz:g:America").is_none());
        assert!(parse_tz_callback("tz:g:America:x").is_none());
        assert!(parse_tz_callback("tz:").is_none());
    }
}
