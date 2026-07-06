//! Fired-reminder presentation: the notification message ([`fired_message`])
//! and its snooze/dismiss/edit keyboard ([`notification_keyboard`]).

use super::event::next_launches_preview;
use crate::locale::LocaleProvider;
use crate::types::{EventInfo, TgMessage};
use chrono::NaiveDateTime;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use teloxide::utils::html;

/// Snooze durations offered on a fired reminder: `(label, minutes)`. The minutes
/// value is embedded in the callback data (`eid:<id>:sn:<minutes>`).
const SNOOZE_OPTIONS: &[(&str, i64)] = &[
    ("1 min", 1),
    ("5 mins", 5),
    ("10 mins", 10),
    ("30 mins", 30),
    ("1 hour", 60),
    ("2 hours", 120),
    ("8 hours", 480),
    ("1 day", 1440),
];

/// Hint appended below a fired reminder, explaining the snooze buttons. Purely
/// informational — the snooze title is loaded from the stored event, not from
/// the message text.
const SNOOZE_HINT: &str = "💤 Snooze this reminder:";

/// Inline keyboard attached to a fired reminder: snooze rows (each button
/// carries `eid:<id>:sn:<minutes>` callback data, where `<id>` is the fired
/// event's DB id, used to load the event when pressed), a dismiss row when the
/// event stays `active` after the fire (`eid:<id>:dis:n` skips the upcoming
/// occurrence; `eid:<id>:disr:n` is added when `is_repetition` — the upcoming
/// `source` is `Repetition` — and skips the interval fills to the next anchor),
/// plus an Edit/Delete row (`eid:<id>:ed` starts the edit flow;
/// `eid:<id>:del:n` starts the notification-aware delete flow, whose Cancel
/// restores this keyboard). The `:n` suffix marks the notification flavor: the
/// handlers keep the fired text and refresh only the keyboard. Public so the
/// dismiss and delete-cancel handlers in `commands::event` can rebuild it.
pub fn notification_keyboard(
    event_id: i64,
    active: bool,
    is_repetition: bool,
) -> InlineKeyboardMarkup {
    // Four buttons on the first row, the rest on the second, to fit narrow screens.
    let mut rows: Vec<Vec<InlineKeyboardButton>> = SNOOZE_OPTIONS
        .chunks(4)
        .map(|chunk| {
            chunk
                .iter()
                .map(|(label, minutes)| {
                    InlineKeyboardButton::callback(*label, format!("eid:{event_id}:sn:{minutes}"))
                })
                .collect()
        })
        .collect();
    // Dismiss actions get their own row (only while a future occurrence remains).
    if active {
        let mut dismiss_row = vec![InlineKeyboardButton::callback(
            "⏭ Dismiss next",
            format!("eid:{event_id}:dis:n"),
        )];
        if is_repetition {
            dismiss_row.push(InlineKeyboardButton::callback(
                "⏩ Dismiss repetition",
                format!("eid:{event_id}:disr:n"),
            ));
        }
        rows.push(dismiss_row);
    }
    rows.push(vec![
        InlineKeyboardButton::callback("✏️ Edit", format!("eid:{event_id}:ed")),
        InlineKeyboardButton::callback("🗑 Delete", format!("eid:{event_id}:del:n")),
    ]);
    InlineKeyboardMarkup::new(rows)
}

/// Builds the message sent when a reminder fires: the stored HTML body, the
/// upcoming-launches preview, and the snooze hint, with the
/// [`notification_keyboard`] attached. `due` is the occurrence being fired (the
/// preview baseline); `now` is the relative-time origin. `post_fire_active` /
/// `post_fire_is_repetition` describe the event *after* the reschedule the
/// caller is about to persist, so the dismiss row matches the state the buttons
/// will act on.
pub fn fired_message(
    event: &EventInfo,
    now: NaiveDateTime,
    due: NaiveDateTime,
    post_fire_active: bool,
    post_fire_is_repetition: bool,
    loc: &dyn LocaleProvider,
) -> TgMessage {
    // `event.message` and the preview are HTML fragments; the hint is plain
    // text, so escape only the hint for HTML.
    let preview = next_launches_preview(event, now, due, loc);
    TgMessage {
        chat_id: event.chat_id,
        text: format!(
            "{}{}\n\n{}",
            event.message,
            preview,
            html::escape(SNOOZE_HINT)
        ),
        reply_markup: Some(notification_keyboard(
            event.id,
            post_fire_active,
            post_fire_is_repetition,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::EN;
    use crate::view::test_support::sample_event;

    #[test]
    fn notification_keyboard_has_snooze_buttons_plus_action_rows() {
        // Inactive after the fire: snooze rows + Edit/Delete only.
        let kb = notification_keyboard(42, false, false);
        let count: usize = kb.inline_keyboard.iter().map(|row| row.len()).sum();
        assert_eq!(count, SNOOZE_OPTIONS.len() + 2);
        assert_eq!(kb.inline_keyboard.last().unwrap().len(), 2);

        // Still active: a dismiss row appears between snooze and Edit/Delete.
        let kb = notification_keyboard(42, true, false);
        let count: usize = kb.inline_keyboard.iter().map(|row| row.len()).sum();
        assert_eq!(count, SNOOZE_OPTIONS.len() + 3);

        // Active with a Repetition source: the dismiss row gains a second button.
        let kb = notification_keyboard(42, true, true);
        let count: usize = kb.inline_keyboard.iter().map(|row| row.len()).sum();
        assert_eq!(count, SNOOZE_OPTIONS.len() + 4);
        let dismiss_row = &kb.inline_keyboard[kb.inline_keyboard.len() - 2];
        assert_eq!(dismiss_row.len(), 2);
        assert_eq!(kb.inline_keyboard.last().unwrap().len(), 2);
    }

    #[test]
    fn notification_keyboard_embeds_event_id_in_callback_data() {
        use teloxide::types::InlineKeyboardButtonKind;

        let datas = |kb: InlineKeyboardMarkup| -> Vec<String> {
            kb.inline_keyboard
                .into_iter()
                .flatten()
                .map(|button| {
                    let InlineKeyboardButtonKind::CallbackData(data) = button.kind else {
                        panic!("expected callback-data button");
                    };
                    data
                })
                .collect()
        };
        let snoozes = || {
            SNOOZE_OPTIONS
                .iter()
                .map(|(_, minutes)| format!("eid:42:sn:{minutes}"))
        };

        let expected: Vec<String> = snoozes()
            .chain(["eid:42:ed".to_owned(), "eid:42:del:n".to_owned()])
            .collect();
        assert_eq!(datas(notification_keyboard(42, false, false)), expected);

        let expected: Vec<String> = snoozes()
            .chain([
                "eid:42:dis:n".to_owned(),
                "eid:42:disr:n".to_owned(),
                "eid:42:ed".to_owned(),
                "eid:42:del:n".to_owned(),
            ])
            .collect();
        assert_eq!(datas(notification_keyboard(42, true, true)), expected);

        let expected: Vec<String> = snoozes()
            .chain([
                "eid:42:dis:n".to_owned(),
                "eid:42:ed".to_owned(),
                "eid:42:del:n".to_owned(),
            ])
            .collect();
        assert_eq!(datas(notification_keyboard(42, true, false)), expected);
    }

    #[test]
    fn fired_message_composes_body_preview_and_hint() {
        use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

        let due = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let mut event = sample_event("<b>call</b> the office", Some(due));
        event.id = 42;
        event.chat_id = 7;

        // One-off event: no launches preview between the body and the hint.
        let msg = fired_message(&event, due, due, true, false, &EN);
        assert_eq!(msg.chat_id, 7);
        assert_eq!(
            msg.text,
            "<b>call</b> the office\n\n💤 Snooze this reminder:"
        );
        // The keyboard reflects the *post-fire* flags handed in: active → the
        // dismiss row is present (snooze rows + dismiss + Edit/Delete).
        let kb = msg.reply_markup.expect("notification keyboard");
        let count: usize = kb.inline_keyboard.iter().map(|row| row.len()).sum();
        assert_eq!(count, SNOOZE_OPTIONS.len() + 3);

        // Inactive post-fire state drops the dismiss row.
        let kb = fired_message(&event, due, due, false, false, &EN)
            .reply_markup
            .expect("notification keyboard");
        let count: usize = kb.inline_keyboard.iter().map(|row| row.len()).sum();
        assert_eq!(count, SNOOZE_OPTIONS.len() + 2);
    }
}
