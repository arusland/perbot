//! Interactive "send me the reminder text" flow for time-only messages.
//!
//! When a user sends only a time expression (e.g. `13:30`) with no reminder body,
//! the bot asks for the text and shows a Cancel button. The parsed (body-less)
//! event is held per-chat until the next text message supplies the body, mirroring
//! the in-memory [`crate::import::PendingImport`] pattern. State is in-memory only;
//! a restart simply drops pending requests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::types::EventInfo;

/// Per-chat events awaiting a reminder body, keyed by chat id. The stored
/// [`EventInfo`] has its time/recurrence fields set and an empty `message`;
/// `chat_id`/`msg_id`/`message` are filled in when the body arrives.
pub type PendingMessage = Arc<Mutex<HashMap<i64, EventInfo>>>;

pub fn new_pending() -> PendingMessage {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Per-chat events being edited, keyed by chat id; the value is the id of the
/// event whose time and message the next message will replace. Set when the user
/// taps Edit on the `/event<id>` view and cleared when the edit completes or is
/// cancelled. In-memory only, like [`PendingMessage`].
pub type PendingEdit = Arc<Mutex<HashMap<i64, i64>>>;

pub fn new_pending_edit() -> PendingEdit {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Prompt shown after a time-only message, asking for the reminder text.
pub const ASK_TEXT: &str = "🕒 Got the time. Now send the reminder text:";

/// Prompt shown when the user taps Edit, asking for the replacement input.
pub const EDIT_ASK_TEXT: &str = "✏️ Send the new time and message:";

/// Re-prompt when an edit reply carried a time but no reminder text.
pub const EDIT_NEED_TEXT: &str = "Please include the reminder text too:";

/// Re-prompt when an edit reply couldn't be parsed into a time.
pub const EDIT_NEED_TIME: &str = "Couldn't read a time. Send the new time and message:";

/// Callback data carried by the Cancel button (routed by the `pm:` prefix).
pub const CANCEL_DATA: &str = "pm:cancel";

/// Single-button keyboard offering to cancel the pending request.
pub fn cancel_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "Cancel",
        CANCEL_DATA,
    )]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use teloxide::types::InlineKeyboardButtonKind;

    #[test]
    fn cancel_keyboard_carries_cancel_data() {
        let kb = cancel_keyboard();
        let button = kb
            .inline_keyboard
            .iter()
            .flatten()
            .next()
            .expect("one button");
        let InlineKeyboardButtonKind::CallbackData(data) = &button.kind else {
            panic!("expected callback-data button");
        };
        assert_eq!(data, CANCEL_DATA);
    }
}
