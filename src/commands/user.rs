//! The admin `/user<id>` chat-info view and its `us:` Ban/Unban callbacks.
//! The card and toggle keyboard live in `view/user.rs`; a ban drops the
//! chat's incoming messages and button presses at `main`'s ingress gates and
//! excludes its events from firing (see `storage::FIREABLE_ONLY`).

use crate::state::EventProvider;
use crate::tgbot::TgBot;
use crate::view::{user_ban_keyboard, user_detail_message};
use teloxide::types::{CallbackQuery, ChatId};

/// Parses a `/user<id>` (or `/user<id>@<bot_username>`) command into the chat id.
///
/// Like `/event<id>`, the argument follows the name with no space, so it is
/// matched manually rather than via the `BotCommands` derive. Group chat ids
/// are negative, so an optional leading `-` is accepted. Returns `None` for
/// anything else (no id, a non-numeric id, or a mismatched `@bot` suffix).
pub fn parse_user_command(text: &str, bot_username: &str) -> Option<i64> {
    let token = text.split_whitespace().next()?;
    let rest = token.strip_prefix("/user")?;
    // Strip an optional `@bot_username` suffix; reject if it names another bot.
    let id = match rest.split_once('@') {
        Some((id, bot)) if bot.eq_ignore_ascii_case(bot_username) => id,
        Some(_) => return None,
        None => rest,
    };
    let digits = id.strip_prefix('-').unwrap_or(id);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    id.parse::<i64>().ok()
}

/// The decoded `us:` callback actions: ban or unban the target chat.
enum UserAction {
    Ban(i64),
    Unban(i64),
}

/// Decodes a `us:` callback payload (`us:b:<id>` / `us:ub:<id>`). `None` for
/// anything malformed.
fn parse_user_callback(data: &str) -> Option<UserAction> {
    let rest = data.strip_prefix("us:")?;
    if let Some(id) = rest.strip_prefix("b:") {
        Some(UserAction::Ban(id.parse().ok()?))
    } else if let Some(id) = rest.strip_prefix("ub:") {
        Some(UserAction::Unban(id.parse().ok()?))
    } else {
        None
    }
}

/// Renders the `/user<id>` card (info + active-event count + Ban/Unban toggle)
/// or `None` when the chat is unknown.
fn render_user_card(
    provider: &EventProvider,
    id: i64,
) -> anyhow::Result<Option<(String, teloxide::types::InlineKeyboardMarkup)>> {
    let Some(chat) = provider.get_chat(id)? else {
        return Ok(None);
    };
    let active_events = provider.count_active_events(id)?;
    Ok(Some((
        user_detail_message(&chat, active_events),
        user_ban_keyboard(id, chat.banned),
    )))
}

/// Sends the chat-info card for `/user<id>`. Admin-only; non-admins get a
/// rejection reply, unknown chat ids a not-found reply.
pub async fn handle_user_view(ctx: &super::CmdContext<'_>, id: i64) -> anyhow::Result<()> {
    if !ctx.is_admin {
        ctx.bot
            .send_text(ctx.chat_id, "Not authorized.", None)
            .await?;
        return Ok(());
    }

    match render_user_card(ctx.provider, id)? {
        Some((text, keyboard)) => {
            ctx.bot.send_html(ctx.chat_id, text, Some(keyboard)).await?;
        }
        None => {
            ctx.bot
                .send_text(ctx.chat_id, "Chat not found.", None)
                .await?;
        }
    }
    Ok(())
}

/// Handles a `us:`-prefixed callback: toggles the target chat's `banned` flag
/// and re-renders the card in place. The pressed message must live in the
/// admin chat (callback ids are user-influenceable — the `/user<id>` card only
/// ever exists there); anything else is acknowledged and ignored.
pub async fn handle_user_callback(
    bot: &TgBot,
    provider: &EventProvider,
    admin_id: ChatId,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    let Some(message) = q.regular_message() else {
        // Message is too old/inaccessible to act on.
        bot.answer_callback(q.id, None).await?;
        return Ok(());
    };
    if message.chat.id != admin_id {
        bot.answer_callback(q.id, None).await?;
        return Ok(());
    }
    let message_id = message.id;

    let (target, banned, toast) = match q.data.as_deref().and_then(parse_user_callback) {
        Some(UserAction::Ban(id)) => (id, true, "🚫 User banned."),
        Some(UserAction::Unban(id)) => (id, false, "User unbanned."),
        None => {
            bot.answer_callback(q.id, None).await?;
            return Ok(());
        }
    };

    provider.set_banned(target, banned)?;
    log::info!("Admin set banned={banned} for chat {target}");
    bot.answer_callback(q.id, Some(toast.to_owned())).await?;

    if let Some((text, keyboard)) = render_user_card(provider, target)?
        && let Err(e) = bot
            .edit_html(admin_id, message_id, text, Some(keyboard))
            .await
    {
        // "message is not modified" (e.g. double-tap) is benign; just log.
        log::warn!("Failed to re-render user card for chat {target}: {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_user_command_round_trips_and_rejects() {
        assert_eq!(parse_user_command("/user42", "perbot"), Some(42));
        assert_eq!(parse_user_command("  /user7  ", "perbot"), Some(7));
        assert_eq!(
            parse_user_command("/user-1001234567890", "perbot"),
            Some(-1001234567890)
        );
        assert_eq!(parse_user_command("/user42@perbot", "perbot"), Some(42));
        assert_eq!(parse_user_command("/user42@PerBot", "perbot"), Some(42));

        assert_eq!(parse_user_command("/user", "perbot"), None);
        assert_eq!(parse_user_command("/user-", "perbot"), None);
        assert_eq!(parse_user_command("/userabc", "perbot"), None);
        assert_eq!(parse_user_command("/user42@otherbot", "perbot"), None);
        assert_eq!(parse_user_command("/users", "perbot"), None);
        assert_eq!(parse_user_command("not a command", "perbot"), None);
    }

    #[test]
    fn parse_user_callback_decodes_envelope() {
        assert!(matches!(
            parse_user_callback("us:b:42"),
            Some(UserAction::Ban(42))
        ));
        assert!(matches!(
            parse_user_callback("us:ub:-100123"),
            Some(UserAction::Unban(-100123))
        ));

        assert!(parse_user_callback("us:b:").is_none());
        assert!(parse_user_callback("us:x:42").is_none());
        assert!(parse_user_callback("us:").is_none());
        assert!(parse_user_callback("eid:1:dis").is_none());
    }
}
