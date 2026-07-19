//! The admin `/user<id>` view: the stored chat-info card and its Ban/Unban
//! toggle keyboard (`us:` callback envelope). Admin-facing flow wording, not
//! time vocabulary — timestamps render as stored (UTC), no `LocaleProvider`
//! methods.

use crate::types::ChatInfo;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use teloxide::utils::html;

/// Callback data of the Ban button: `us:b:<chat_id>`.
pub fn user_ban_data(chat_id: i64) -> String {
    format!("us:b:{chat_id}")
}

/// Callback data of the Unban button: `us:ub:<chat_id>`.
pub fn user_unban_data(chat_id: i64) -> String {
    format!("us:ub:{chat_id}")
}

/// The `/user<id>` card: everything the `chats` table knows about the chat —
/// id and type, the identity fields Telegram provided (HTML-escaped), the
/// active-event count, the stored timestamps (UTC) — plus a banned notice.
pub fn user_detail_message(chat: &ChatInfo, active_events: usize) -> String {
    let mut out = if chat.banned {
        "🚫 <b>User info (banned)</b>".to_owned()
    } else {
        "👤 <b>User info</b>".to_owned()
    };
    out.push_str(&format!(
        "\nChat: <code>{}</code> ({})",
        chat.id,
        chat.chat_type.as_str()
    ));
    let name = [chat.first_name.as_deref(), chat.last_name.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    if !name.is_empty() {
        out.push_str(&format!("\nName: {}", html::escape(&name)));
    }
    if let Some(username) = &chat.username {
        out.push_str(&format!("\nUsername: @{}", html::escape(username)));
    }
    if let Some(title) = &chat.title {
        out.push_str(&format!("\nTitle: {}", html::escape(title)));
    }
    out.push_str(&format!("\nActive events: {active_events}"));
    if let Some(created) = chat.created_at {
        out.push_str(&format!(
            "\nCreated: {} UTC",
            created.format("%Y-%m-%d %H:%M:%S")
        ));
    }
    if let Some(updated) = chat.updated_at {
        out.push_str(&format!(
            "\nUpdated: {} UTC",
            updated.format("%Y-%m-%d %H:%M:%S")
        ));
    }
    out
}

/// The single-button toggle under the `/user<id>` card: Ban when the chat is
/// not banned, Unban when it is.
pub fn user_ban_keyboard(chat_id: i64, banned: bool) -> InlineKeyboardMarkup {
    let button = if banned {
        InlineKeyboardButton::callback("✅ Unban user", user_unban_data(chat_id))
    } else {
        InlineKeyboardButton::callback("🚫 Ban user", user_ban_data(chat_id))
    };
    InlineKeyboardMarkup::new(vec![vec![button]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatType;

    fn sample_chat(banned: bool) -> ChatInfo {
        ChatInfo {
            id: 42,
            chat_type: ChatType::Private,
            title: None,
            username: Some("j<doe>".into()),
            first_name: Some("John".into()),
            last_name: Some("Doe & Co".into()),
            banned,
            updated_at: chrono::NaiveDateTime::parse_from_str(
                "2026-07-19 10:00:00",
                "%Y-%m-%d %H:%M:%S",
            )
            .ok(),
            created_at: chrono::NaiveDateTime::parse_from_str(
                "2026-01-01 08:30:00",
                "%Y-%m-%d %H:%M:%S",
            )
            .ok(),
        }
    }

    #[test]
    fn user_detail_message_lists_fields_escaped() {
        let out = user_detail_message(&sample_chat(false), 3);
        assert!(out.starts_with("👤 <b>User info</b>"));
        assert!(out.contains("<code>42</code> (private)"));
        assert!(out.contains("Name: John Doe &amp; Co"));
        assert!(out.contains("Username: @j&lt;doe&gt;"));
        assert!(!out.contains("Title:"));
        assert!(out.contains("Active events: 3"));
        assert!(out.contains("Created: 2026-01-01 08:30:00 UTC"));
        assert!(out.contains("Updated: 2026-07-19 10:00:00 UTC"));
    }

    #[test]
    fn user_detail_message_marks_banned() {
        let out = user_detail_message(&sample_chat(true), 0);
        assert!(out.starts_with("🚫 <b>User info (banned)</b>"));
    }

    #[test]
    fn user_ban_keyboard_toggles_and_fits_callback_limit() {
        let ban = user_ban_keyboard(42, false);
        let unban = user_ban_keyboard(42, true);
        let data = |kb: &InlineKeyboardMarkup| match &kb.inline_keyboard[0][0].kind {
            teloxide::types::InlineKeyboardButtonKind::CallbackData(d) => d.clone(),
            other => panic!("unexpected button kind: {other:?}"),
        };
        assert_eq!(data(&ban), "us:b:42");
        assert_eq!(data(&unban), "us:ub:42");

        // Worst-case chat id stays well under Telegram's 64-byte limit.
        assert!(user_unban_data(i64::MIN).len() <= 64);
    }
}
