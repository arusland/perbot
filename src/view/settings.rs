//! The Settings menu, reached from the `⚙ Settings` button under the /events
//! list and the /help reply. Today it holds one setting: the **morning
//! digest** — a daily message with today's events (the `/today` list), off by
//! default, sent at a chat-local time the user picks (whole hours, default
//! [`DEFAULT_DIGEST_HOUR`]).
//!
//! Callback envelope `st:`: [`SETTINGS_OPEN_DATA`] (`st:o`) opens the menu as
//! a new message; `st:s` re-renders it in place (the picker's Back button);
//! `st:on` / `st:off` toggle the digest; `st:t` opens the hour picker;
//! `st:h:<H>` picks the digest hour. The setting applies to the chat the
//! pressed message lives in — the same authority model as list pagination.

use chrono::NaiveTime;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

/// Digest hour applied when the morning digest is switched on (08:00).
pub const DEFAULT_DIGEST_HOUR: u32 = 8;

/// Callback data of the `⚙ Settings` entry button: opens the menu as a new
/// message, keeping the list/help message it was pressed under intact.
pub const SETTINGS_OPEN_DATA: &str = "st:o";

/// Prompt shown above the hour picker opened by the digest-time button.
pub const DIGEST_TIME_ASK: &str = "🕗 Choose the morning digest time:";

/// Hour buttons per picker row (24 hours → 4 rows).
const DIGEST_HOURS_PER_ROW: usize = 6;

/// The Settings menu text (HTML): the morning digest's state — its chat-local
/// time when on — and what the digest does.
pub fn settings_message(digest: Option<NaiveTime>) -> String {
    let state = match digest {
        Some(t) => format!("<b>on, {}</b>", t.format("%H:%M")),
        None => "<b>off</b>".to_string(),
    };
    format!(
        "⚙️ <b>Settings</b>\n\n\
         🌅 Morning digest: {state}\n\
         A daily message with today's events."
    )
}

/// The Settings menu keyboard: a digest on/off toggle, plus the time button
/// (opening the hour picker) while the digest is on.
pub fn settings_keyboard(digest: Option<NaiveTime>) -> InlineKeyboardMarkup {
    let rows = match digest {
        Some(t) => vec![
            vec![InlineKeyboardButton::callback(
                "🌅 Turn morning digest off",
                "st:off",
            )],
            vec![InlineKeyboardButton::callback(
                format!("🕗 Digest time: {}", t.format("%H:%M")),
                "st:t",
            )],
        ],
        None => vec![vec![InlineKeyboardButton::callback(
            "🌅 Turn morning digest on",
            "st:on",
        )]],
    };
    InlineKeyboardMarkup::new(rows)
}

/// The digest-time picker: all 24 whole hours (`st:h:<H>`) plus a Back row to
/// the Settings menu (`st:s`).
pub fn digest_time_keyboard() -> InlineKeyboardMarkup {
    let hours: Vec<u32> = (0..24).collect();
    let mut rows: Vec<Vec<InlineKeyboardButton>> = hours
        .chunks(DIGEST_HOURS_PER_ROW)
        .map(|chunk| {
            chunk
                .iter()
                .map(|h| InlineKeyboardButton::callback(format!("{h:02}:00"), format!("st:h:{h}")))
                .collect()
        })
        .collect();
    rows.push(vec![InlineKeyboardButton::callback("« Back", "st:s")]);
    InlineKeyboardMarkup::new(rows)
}

/// The `⚙ Settings` entry row shared by the /events list keyboard and the
/// /help reply.
pub(super) fn settings_entry_row() -> Vec<InlineKeyboardButton> {
    vec![InlineKeyboardButton::callback(
        "⚙ Settings",
        SETTINGS_OPEN_DATA,
    )]
}

/// A keyboard holding only the `⚙ Settings` entry row (attached to /help).
pub fn settings_entry_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![settings_entry_row()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use teloxide::types::InlineKeyboardButtonKind::CallbackData;

    fn datas(kb: &InlineKeyboardMarkup) -> Vec<String> {
        kb.inline_keyboard
            .iter()
            .flatten()
            .map(|b| match &b.kind {
                CallbackData(d) => d.clone(),
                _ => panic!("expected callback data"),
            })
            .collect()
    }

    #[test]
    fn settings_message_shows_digest_state() {
        let off = settings_message(None);
        assert!(off.starts_with("⚙️ <b>Settings</b>"));
        assert!(off.contains("Morning digest: <b>off</b>"));

        let on = settings_message(NaiveTime::from_hms_opt(8, 0, 0));
        assert!(on.contains("Morning digest: <b>on, 08:00</b>"));
        assert!(on.contains("/today"));
    }

    #[test]
    fn settings_keyboard_toggles_and_offers_time_when_on() {
        assert_eq!(datas(&settings_keyboard(None)), vec!["st:on"]);

        let on = settings_keyboard(NaiveTime::from_hms_opt(9, 0, 0));
        assert_eq!(datas(&on), vec!["st:off", "st:t"]);
        // The time button carries the current time in its label.
        let time_label = &on.inline_keyboard[1][0].text;
        assert!(time_label.contains("09:00"), "{time_label}");
    }

    #[test]
    fn digest_time_keyboard_offers_every_hour_and_back() {
        let kb = digest_time_keyboard();
        let all = datas(&kb);
        for h in 0..24 {
            assert!(all.contains(&format!("st:h:{h}")), "hour {h} missing");
        }
        assert_eq!(all.last().unwrap(), "st:s");
        // 4 hour rows of 6 + the Back row; every payload fits Telegram's
        // 64-byte callback-data limit.
        assert_eq!(kb.inline_keyboard.len(), 5);
        assert!(all.iter().all(|d| d.len() <= 64));
    }

    #[test]
    fn entry_keyboard_opens_settings() {
        assert_eq!(datas(&settings_entry_keyboard()), vec![SETTINGS_OPEN_DATA]);
    }
}
