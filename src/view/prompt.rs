//! Prompts and the Cancel keyboard for the in-memory pending flows — the
//! time-only "send me the reminder text" completion and the event edit flow
//! (the flow state itself lives in [`crate::pending`]).

use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

/// Prompt shown after a time-only message, asking for the reminder text.
pub const ASK_TEXT: &str = "🕒 Got the time. Now send the reminder text";

/// Prompt shown when the user taps Edit, asking for the replacement input.
pub const EDIT_ASK_TEXT: &str = "✏️ Send the new time and message";

/// Prompt shown when the user picks "Edit text", asking for the replacement
/// message text (the schedule stays as is). Also the whitespace-only re-prompt.
pub const EDIT_TEXT_ASK: &str = "📝 Send the new message text";

/// Re-prompt when an edit reply carried a time but no reminder text.
pub const EDIT_NEED_TEXT: &str = "Please include the reminder text too";

/// Re-prompt when an edit reply couldn't be parsed into a time.
pub const EDIT_NEED_TIME: &str = "Couldn't read a time. Send the new time and message";

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
