//! The snooze buttons under a fired reminder (`eid:<id>:sn:<minutes>`): each
//! press inserts a one-off child event scheduled at `now + <minutes>` whose
//! `parent` points at the original (root) event, leaving the original
//! untouched. The child owns only its time — its message is resolved from the
//! parent by storage, and deleting the parent cascade-deletes it.

use super::event::parse_event_callback;
use crate::state::EventProvider;
use crate::tgbot::TgBot;
use crate::types::{EventInfo, NextSource};
use crate::view::{event_actions_keyboard, snoozed_message};
use chrono::{Duration, Utc};
use chrono_tz::Tz;
use teloxide::types::CallbackQuery;

/// Parses snooze callback data `eid:<id>:sn:<minutes>` into `(event_id, minutes)`.
/// Returns `None` for any malformed input or a non-snooze action.
fn parse_snooze_callback(data: &str) -> Option<(i64, i64)> {
    let (id, action) = parse_event_callback(data)?;
    let minutes = action.strip_prefix("sn:")?;
    Some((id, minutes.parse::<i64>().ok()?))
}

/// Builds the one-off child event a snooze creates: an explicit-year reminder
/// scheduled exactly at `next` (UTC), already marked active, whose `parent`
/// points at `source`'s root (`source.parent` when `source` is itself a snooze —
/// snoozes never chain). The wall-clock `date`/`time` fields carry the chat-local
/// reading of `next`, so the edit prompt and a timezone-change reschedule see
/// consistent values. The child stores an empty message and reuses the parent's
/// `msg_id`; the effective text always comes from the parent. It is inserted via
/// `insert_prebuilt_event` (no scheduler run), and after it fires
/// `scheduler::calc_next_at` returns `None` (no repetition, year explicit), so it
/// goes inactive instead of repeating.
fn snoozed_event(source: &EventInfo, next: chrono::NaiveDateTime, tz: Tz) -> EventInfo {
    let local = crate::tz::to_local(next, tz);
    EventInfo {
        date: Some(local.date()),
        time: Some(local.time()),
        year_explicit: true,
        days: None,
        years: None,
        repetition: None,
        in_offset: None,
        bare_hour: None,
        monthly_pattern: None,
        message: String::new(),
        id: 0,
        chat_id: source.chat_id,
        active: true,
        next_datetime: Some(next),
        source: Some(NextSource::Date),
        last_next_datetime: Some(next),
        created_at: next,
        msg_id: source.msg_id,
        legacy: false,
        parent: Some(source.parent.unwrap_or(source.id)),
    }
}

/// Handles a snooze-button press: creates a new one-off child event referencing
/// the fired reminder (its root parent), scheduled at `now + <minutes>`. The
/// original event is left untouched. Driven from `main`'s callback-query branch
/// for `eid:`-prefixed callback data.
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
    let source = match provider.get_event(event_id)? {
        Some(event) if event.chat_id == chat_id.0 => event,
        _ => {
            bot.answer_callback(q.id, Some("Can't snooze this reminder.".to_owned()))
                .await?;
            return Ok(());
        }
    };

    let now = Utc::now().naive_utc();
    let next = now + Duration::minutes(minutes);

    // Snooze is deliberately not gated on an unset timezone: it is pure-instant
    // ("in N minutes"), so the UTC fallback only affects the wall-clock fields.
    let tz = provider.tz_or_utc(chat_id.0);
    let mut event = snoozed_event(&source, next, tz);
    match provider.insert_prebuilt_event(&event) {
        Ok(id) => event.id = id,
        Err(e) => {
            log::error!("Failed to insert snoozed event for chat {}: {e}", chat_id.0);
            bot.answer_callback(q.id, Some("Failed to snooze.".to_owned()))
                .await?;
            return Ok(());
        }
    }
    // The local struct stores an empty message; reload so the confirmation
    // carries the parent-resolved text.
    let event = provider.get_event(event.id)?.unwrap_or(event);

    bot.answer_callback(q.id, None).await?;
    let loc = crate::locale::for_chat(chat_id.0);
    let is_repetition = event.source == Some(NextSource::Repetition);
    bot.send_html(
        chat_id,
        snoozed_message(&event, now, tz, loc),
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
        let next = Utc::now().naive_utc() + Duration::minutes(5);
        let mut source = crate::view::test_support::sample_event("call mom", Some(next));
        source.id = 42;
        source.msg_id = 7;
        let event = snoozed_event(&source, next, Tz::UTC);
        assert!(event.active);
        assert!(event.is_snoozed());
        assert_eq!(event.parent, Some(42));
        assert_eq!(event.msg_id, 7);
        assert!(event.message.is_empty());
        assert_eq!(event.next_datetime, Some(next));

        let fired = scheduler::calc_next_at(event, next, Tz::UTC);
        assert!(!fired.active);
        assert!(fired.next_datetime.is_none());
    }

    #[test]
    fn snoozing_a_snoozed_event_reparents_to_root() {
        // Snoozes never chain: a snooze of a snooze points at the original root.
        let next = Utc::now().naive_utc() + Duration::minutes(5);
        let mut source = crate::view::test_support::sample_event("call mom", Some(next));
        source.id = 42;
        source.parent = Some(7);
        let event = snoozed_event(&source, next, Tz::UTC);
        assert_eq!(event.parent, Some(7));
    }
}
