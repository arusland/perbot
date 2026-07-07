//! The snooze buttons under a fired reminder (`eid:<id>:sn:<minutes>`): each
//! press inserts a one-off `snoozed` copy of the event scheduled at
//! `now + <minutes>`, leaving the original untouched.

use super::event::parse_event_callback;
use crate::state::EventProvider;
use crate::tgbot::TgBot;
use crate::types::{EventInfo, NextSource};
use crate::view::{event_actions_keyboard, snoozed_message};
use chrono::{Duration, Local};
use teloxide::types::CallbackQuery;

/// Parses snooze callback data `eid:<id>:sn:<minutes>` into `(event_id, minutes)`.
/// Returns `None` for any malformed input or a non-snooze action.
fn parse_snooze_callback(data: &str) -> Option<(i64, i64)> {
    let (id, action) = parse_event_callback(data)?;
    let minutes = action.strip_prefix("sn:")?;
    Some((id, minutes.parse::<i64>().ok()?))
}

/// Builds the one-off event a snooze creates: an explicit-year reminder scheduled
/// exactly at `next`, already marked active. It is inserted via
/// `insert_prebuilt_event` (no scheduler run), and after it fires
/// `scheduler::calc_next_at` returns `None` (no repetition, year explicit), so it
/// goes inactive instead of repeating.
fn snoozed_event(
    chat_id: i64,
    msg_id: i64,
    title: String,
    next: chrono::NaiveDateTime,
) -> EventInfo {
    EventInfo {
        date: Some(next.date()),
        time: Some(next.time()),
        year_explicit: true,
        days: None,
        years: None,
        repetition: None,
        in_offset: None,
        bare_hour: None,
        monthly_pattern: None,
        message: title,
        id: 0,
        chat_id,
        active: true,
        next_datetime: Some(next),
        source: Some(NextSource::Date),
        last_next_datetime: Some(next),
        created_at: next,
        msg_id,
        legacy: false,
        snoozed: true,
    }
}

/// Handles a snooze-button press: creates a new one-off event with the same title
/// as the fired reminder, scheduled at `now + <minutes>`. The original event is
/// left untouched. Driven from `main`'s callback-query branch for `eid:`-prefixed
/// callback data.
///
/// The target event is identified by id from the callback data
/// (`eid:<id>:sn:<minutes>`) and loaded from storage. Because callback ids are
/// attacker-influenceable, the loaded event is only honored when it belongs to the
/// chat the button was pressed in.
pub async fn handle_snooze_callback(
    bot: &TgBot,
    provider: &EventProvider,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    let parsed = q.data.as_deref().and_then(parse_snooze_callback);
    let Some((event_id, minutes)) = parsed else {
        bot.answer_callback(q.id, None).await?;
        return Ok(());
    };

    let Some(message) = q.regular_message() else {
        bot.answer_callback(q.id, Some("Can't snooze this reminder.".to_owned()))
            .await?;
        return Ok(());
    };
    let chat_id = message.chat.id;

    // Load the event and verify it belongs to this chat before acting on it.
    // `event.message` is an HTML fragment, so the snoozed copy keeps the user's
    // formatting verbatim.
    let title = match provider.get_event(event_id)? {
        Some(event) if event.chat_id == chat_id.0 => event.message,
        _ => {
            bot.answer_callback(q.id, Some("Can't snooze this reminder.".to_owned()))
                .await?;
            return Ok(());
        }
    };

    let now = Local::now().naive_local();
    let next = now + Duration::minutes(minutes);
    let user_id = q.from.id.0 as i64;

    // Backing message row (events.msg_id is a NOT NULL FK to messages).
    let msg_id = match provider.insert_message(Some(user_id), chat_id.0, &title) {
        Ok(id) => id,
        Err(e) => {
            log::error!("Failed to save snooze message for chat {}: {e}", chat_id.0);
            bot.answer_callback(q.id, Some("Failed to snooze.".to_owned()))
                .await?;
            return Ok(());
        }
    };

    let mut event = snoozed_event(chat_id.0, msg_id, title, next);
    match provider.insert_prebuilt_event(&event) {
        Ok(id) => event.id = id,
        Err(e) => {
            log::error!("Failed to insert snoozed event for chat {}: {e}", chat_id.0);
            bot.answer_callback(q.id, Some("Failed to snooze.".to_owned()))
                .await?;
            return Ok(());
        }
    }

    bot.answer_callback(q.id, None).await?;
    let loc = crate::locale::for_chat(chat_id.0);
    let is_repetition = event.source == Some(NextSource::Repetition);
    bot.send_html(
        chat_id,
        snoozed_message(&event, now, loc),
        Some(event_actions_keyboard(
            event.id,
            event.active,
            is_repetition,
        )),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler;

    #[test]
    fn parse_snooze_callback_round_trips_and_rejects_malformed() {
        assert_eq!(parse_snooze_callback("eid:42:sn:30"), Some((42, 30)));
        assert_eq!(parse_snooze_callback("eid:-7:sn:1"), Some((-7, 1)));

        // Old format, non-numeric id/minutes, missing parts, and list callbacks.
        assert_eq!(parse_snooze_callback("sn:30"), None);
        assert_eq!(parse_snooze_callback("eid:x:sn:30"), None);
        assert_eq!(parse_snooze_callback("eid:42:sn:"), None);
        assert_eq!(parse_snooze_callback("eid:42:sn:abc"), None);
        assert_eq!(parse_snooze_callback("ev:1"), None);
    }

    #[test]
    fn snoozed_event_goes_inactive_after_firing() {
        // The snoozed event is scheduled at `next`; once "now" reaches it (firing),
        // calc_next_at must return inactive so it does not repeat.
        let next = Local::now().naive_local() + Duration::minutes(5);
        let event = snoozed_event(42, 7, "call mom".to_string(), next);
        assert!(event.active);
        assert!(event.snoozed);
        assert_eq!(event.next_datetime, Some(next));

        let fired = scheduler::calc_next_at(event, next);
        assert!(!fired.active);
        assert!(fired.next_datetime.is_none());
    }
}
