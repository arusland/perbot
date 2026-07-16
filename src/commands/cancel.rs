//! The Cancel button of the time-only flow's "send me the reminder text"
//! prompt (`pm:`-prefixed callback data).

use crate::pending::PendingMessage;
use crate::tgbot::TgBot;
use teloxide::types::CallbackQuery;

/// Handles a Cancel-button press from the "send me the reminder text" prompt:
/// drops the pending request for the chat and edits the prompt to "❌ Cancelled."
/// (clearing the keyboard). Routed from `main`'s callback branch for the `pm:`
/// prefix.
pub async fn handle_cancel_pending(
    bot: &TgBot,
    pending_msg: &PendingMessage,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    bot.answer_callback(q.id.clone(), None).await?;

    let Some(message) = q.regular_message() else {
        return Ok(());
    };
    let chat_id = message.chat.id;
    pending_msg.lock().unwrap().remove(&chat_id.0);

    if let Err(e) = bot.edit_text(chat_id, message.id, "❌ Cancelled.").await {
        log::warn!(
            "Failed to edit cancelled prompt for chat {}: {e}",
            chat_id.0
        );
    }
    Ok(())
}
