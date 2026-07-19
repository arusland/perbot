//! The Settings menu, reached from the `⚙ Settings` button under the /events
//! list and the /help reply. It holds the **morning digest** — a daily message
//! with today's events (the `/today` list), off by default, sent at a
//! chat-local time the user picks (whole hours, default
//! [`DEFAULT_DIGEST_HOUR`]) — and shows the chat's **timezone** with a button
//! into the region picker.
//!
//! Callback envelope `st:`: [`SETTINGS_OPEN_DATA`] (`st:o`) opens the menu as
//! a new message; `st:s` re-renders it in place (the picker's Back button);
//! `st:on` / `st:off` toggle the digest; `st:t` opens the hour picker;
//! `st:h:<H>` picks the digest hour; `st:tz` swaps in the timezone region
//! picker (the `tz:` flow takes over from there). The setting applies to the
//! chat the pressed message lives in — the same authority model as list
//! pagination.

use chrono::NaiveTime;
use chrono_tz::Tz;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use teloxide::utils::html;

/// Digest hour applied when the morning digest is switched on (08:00).
pub const DEFAULT_DIGEST_HOUR: u32 = 8;

/// Callback data of the `⚙ Settings` entry button: opens the menu as a new
/// message, keeping the list/help message it was pressed under intact.
pub const SETTINGS_OPEN_DATA: &str = "st:o";

/// Prompt shown above the hour picker opened by the digest-time button.
pub const DIGEST_TIME_ASK: &str = "🕗 Choose the morning digest time:";

/// Footer appended to every morning-digest message, pointing at /settings.
pub const DIGEST_NOTE: &str = "<i>This message was sent because the morning digest is on. You can turn it off in /settings.</i>";

/// Hour buttons per picker row (24 hours → 4 rows).
const DIGEST_HOURS_PER_ROW: usize = 6;

/// The Settings menu text (HTML): the morning digest's state — its chat-local
/// time when on — what the digest does, and the chat's timezone.
pub fn settings_message(digest: Option<NaiveTime>, tz: Option<Tz>) -> String {
    let state = match digest {
        Some(t) => format!("<b>on, {}</b>", t.format("%H:%M")),
        None => "<b>off</b>".to_string(),
    };
    let tz_state = match tz {
        Some(tz) => format!("<b>{}</b>", html::escape(tz.name())),
        None => "<b>not set</b>".to_string(),
    };
    format!(
        "⚙️ <b>Settings</b>\n\n\
         🌅 Morning digest: {state}\n\
         A daily message with today's events.\n\n\
         🌍 Timezone: {tz_state}"
    )
}

/// The Settings menu keyboard: a digest on/off toggle, the time button
/// (opening the hour picker) while the digest is on, and the timezone button
/// (opening the region picker).
pub fn settings_keyboard(digest: Option<NaiveTime>, tz: Option<Tz>) -> InlineKeyboardMarkup {
    let mut rows = match digest {
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
    let tz_label = match tz {
        Some(tz) => format!("🌍 Timezone: {}", tz.name()),
        None => "🌍 Set timezone".to_string(),
    };
    rows.push(vec![InlineKeyboardButton::callback(tz_label, "st:tz")]);
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
        let off = settings_message(None, None);
        assert!(off.starts_with("⚙️ <b>Settings</b>"));
        assert!(off.contains("Morning digest: <b>off</b>"));
        assert!(off.contains("Timezone: <b>not set</b>"));

        let on = settings_message(NaiveTime::from_hms_opt(8, 0, 0), Some(Tz::Europe__Berlin));
        assert!(on.contains("Morning digest: <b>on, 08:00</b>"));
        assert!(on.contains("today's events"));
        assert!(on.contains("Timezone: <b>Europe/Berlin</b>"));
    }

    #[test]
    fn digest_note_points_at_settings_command() {
        assert!(DIGEST_NOTE.contains("/settings"));
    }

    #[test]
    fn settings_keyboard_toggles_and_offers_time_when_on() {
        assert_eq!(
            datas(&settings_keyboard(None, None)),
            vec!["st:on", "st:tz"]
        );

        let on = settings_keyboard(NaiveTime::from_hms_opt(9, 0, 0), Some(Tz::Europe__Berlin));
        assert_eq!(datas(&on), vec!["st:off", "st:t", "st:tz"]);
        // The time button carries the current time in its label.
        let time_label = &on.inline_keyboard[1][0].text;
        assert!(time_label.contains("09:00"), "{time_label}");
        // The timezone button carries the current zone in its label.
        let tz_label = &on.inline_keyboard[2][0].text;
        assert!(tz_label.contains("Europe/Berlin"), "{tz_label}");
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
