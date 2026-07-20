//! Fired-reminder presentation: the notification message ([`fired_message`])
//! and its snooze/dismiss/edit keyboard ([`notification_keyboard`]).

use super::event::next_launches_preview;
use crate::locale::LocaleProvider;
use crate::types::{EventInfo, TgMessage};
use chrono::NaiveDateTime;
use chrono_tz::Tz;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

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

/// Snooze duration the collapsed keyboard offers when a chat has never snoozed
/// (no [`crate::storage::LAST_SNOOZE_SETTING`] stored) or the stored value is
/// unparsable.
pub const DEFAULT_SNOOZE_MINUTES: i64 = 5;

/// The button label for a snooze duration: the canonical [`SNOOZE_OPTIONS`]
/// label when the minutes match an offered option, otherwise a plain
/// `<minutes> mins` fallback (only reachable through a hand-edited setting).
fn snooze_label(minutes: i64) -> String {
    SNOOZE_OPTIONS
        .iter()
        .find(|(_, m)| *m == minutes)
        .map(|(label, _)| (*label).to_owned())
        .unwrap_or_else(|| format!("{minutes} mins"))
}

/// The action rows shared by both notification keyboards: a dismiss row when
/// the event stays `active` after the fire (`eid:<id>:dis:n` skips the upcoming
/// occurrence; `eid:<id>:disr:n` is added when `is_repetition` — the upcoming
/// `source` is `Repetition` — and skips the interval fills to the next anchor),
/// plus an Edit/Delete row (`eid:<id>:ed` starts the edit flow;
/// `eid:<id>:del:n` starts the notification-aware delete flow, whose Cancel
/// restores the collapsed keyboard).
fn action_rows(event_id: i64, active: bool, is_repetition: bool) -> Vec<Vec<InlineKeyboardButton>> {
    let mut rows = Vec::new();
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
    rows
}

/// Inline keyboard attached to a fired reminder, in its default **collapsed**
/// form: one snooze row — `Snooze for ...` (`eid:<id>:snx`, swaps in the
/// [`expanded_notification_keyboard`] in place) next to a last-used shortcut
/// `Snooze <label>` (`eid:<id>:sn:<last_snooze>`, where `<id>` is the fired
/// event's DB id, used to load the event when pressed) — followed by the shared
/// dismiss and Edit/Delete rows ([`action_rows`]). `last_snooze` is the chat's
/// stored last snooze duration in minutes ([`EventProvider::last_snooze`]).
/// The `:n` suffix marks the notification flavor: the handlers refresh the
/// fired message in place instead of swapping in the detail view. Public so
/// the dismiss and delete-cancel handlers in `commands::event` can rebuild it.
///
/// [`EventProvider::last_snooze`]: crate::state::EventProvider::last_snooze
pub fn notification_keyboard(
    event_id: i64,
    active: bool,
    is_repetition: bool,
    last_snooze: i64,
) -> InlineKeyboardMarkup {
    let mut rows = vec![vec![
        InlineKeyboardButton::callback("Snooze for ...", format!("eid:{event_id}:snx")),
        InlineKeyboardButton::callback(
            format!("Snooze {}", snooze_label(last_snooze)),
            format!("eid:{event_id}:sn:{last_snooze}"),
        ),
    ]];
    rows.extend(action_rows(event_id, active, is_repetition));
    InlineKeyboardMarkup::new(rows)
}

/// The expanded flavor of [`notification_keyboard`]: the full [`SNOOZE_OPTIONS`]
/// rows (each button carries `eid:<id>:sn:<minutes>`) instead of the collapsed
/// two-button row, followed by the same shared action rows. Swapped in by the
/// `Snooze for ...` (`eid:<id>:snx`) handler in `commands::snooze`.
pub fn expanded_notification_keyboard(
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
    rows.extend(action_rows(event_id, active, is_repetition));
    InlineKeyboardMarkup::new(rows)
}

/// Builds the message sent when a reminder fires: the stored HTML body, the
/// upcoming-launches preview, and the snooze hint, with the
/// [`notification_keyboard`] attached. `due` is the occurrence being fired (the
/// preview baseline); `now` is the relative-time origin. `post_fire_active` /
/// `post_fire_is_repetition` describe the event *after* the reschedule the
/// caller is about to persist, so the dismiss row matches the state the buttons
/// will act on. `last_snooze` labels the collapsed keyboard's last-used snooze
/// shortcut (minutes).
#[allow(clippy::too_many_arguments)]
pub fn fired_message(
    event: &EventInfo,
    now: NaiveDateTime,
    due: NaiveDateTime,
    post_fire_active: bool,
    post_fire_is_repetition: bool,
    last_snooze: i64,
    tz: Tz,
    loc: &dyn LocaleProvider,
) -> TgMessage {
    log::info!(
        "Firing event {} due {} (source={:?})",
        event.id,
        due,
        event.source
    );
    // `event.message` and the preview are HTML fragments; the hint is plain
    // text, so escape only the hint for HTML.
    let preview = next_launches_preview(event, now, due, tz, loc);
    TgMessage {
        chat_id: event.chat_id,
        text: format!("🔔 {}{}", event.message, preview),
        reply_markup: Some(notification_keyboard(
            event.id,
            post_fire_active,
            post_fire_is_repetition,
            last_snooze,
        )),
    }
}

/// Rebuilds a fired reminder's text after a dismiss advanced its schedule: the
/// stored HTML body plus a fresh upcoming-launches preview led by the new
/// `next_datetime`. The fired text has no `Time:` line, so the preview baseline
/// sits one second before the new occurrence (mirroring the `+ 1s` step the
/// dismiss itself used) to keep that occurrence in the list. An event with
/// nothing further to fire renders the bare body, like a one-off fire.
pub fn dismissed_notification_text(
    event: &EventInfo,
    now: NaiveDateTime,
    tz: Tz,
    loc: &dyn LocaleProvider,
) -> String {
    let preview = match event.next_datetime {
        Some(next) => {
            next_launches_preview(event, now, next - chrono::Duration::seconds(1), tz, loc)
        }
        None => String::new(),
    };
    format!("🔔 {}{}", event.message, preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::EN;
    use crate::view::test_support::sample_event;

    /// Flattens a keyboard into its callback-data strings, in row order.
    fn datas(kb: InlineKeyboardMarkup) -> Vec<String> {
        use teloxide::types::InlineKeyboardButtonKind;
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
    }

    #[test]
    fn snooze_label_prefers_canonical_option_labels() {
        assert_eq!(snooze_label(5), "5 mins");
        assert_eq!(snooze_label(60), "1 hour");
        assert_eq!(snooze_label(1440), "1 day");
        // Not an offered option (hand-edited setting): plain-minutes fallback.
        assert_eq!(snooze_label(42), "42 mins");
    }

    #[test]
    fn notification_keyboard_collapses_snooze_into_one_row() {
        // Inactive after the fire: collapsed snooze row + Edit/Delete only.
        let kb = notification_keyboard(42, false, false, 5);
        assert_eq!(kb.inline_keyboard.len(), 2);
        assert_eq!(kb.inline_keyboard[0].len(), 2);
        assert_eq!(kb.inline_keyboard[0][0].text, "Snooze for ...");
        assert_eq!(kb.inline_keyboard[0][1].text, "Snooze 5 mins");
        assert_eq!(kb.inline_keyboard.last().unwrap().len(), 2);

        // Still active: a dismiss row appears between snooze and Edit/Delete.
        let kb = notification_keyboard(42, true, false, 30);
        assert_eq!(kb.inline_keyboard.len(), 3);
        assert_eq!(kb.inline_keyboard[0][1].text, "Snooze 30 mins");

        // Active with a Repetition source: the dismiss row gains a second button.
        let kb = notification_keyboard(42, true, true, 5);
        assert_eq!(kb.inline_keyboard.len(), 3);
        assert_eq!(kb.inline_keyboard[1].len(), 2);
    }

    #[test]
    fn notification_keyboard_embeds_event_id_in_callback_data() {
        let collapsed = || ["eid:42:snx".to_owned(), "eid:42:sn:30".to_owned()].into_iter();

        let expected: Vec<String> = collapsed()
            .chain(["eid:42:ed".to_owned(), "eid:42:del:n".to_owned()])
            .collect();
        assert_eq!(datas(notification_keyboard(42, false, false, 30)), expected);

        let expected: Vec<String> = collapsed()
            .chain([
                "eid:42:dis:n".to_owned(),
                "eid:42:disr:n".to_owned(),
                "eid:42:ed".to_owned(),
                "eid:42:del:n".to_owned(),
            ])
            .collect();
        assert_eq!(datas(notification_keyboard(42, true, true, 30)), expected);

        let expected: Vec<String> = collapsed()
            .chain([
                "eid:42:dis:n".to_owned(),
                "eid:42:ed".to_owned(),
                "eid:42:del:n".to_owned(),
            ])
            .collect();
        assert_eq!(datas(notification_keyboard(42, true, false, 30)), expected);
    }

    #[test]
    fn expanded_notification_keyboard_has_full_snooze_rows() {
        let snoozes = || {
            SNOOZE_OPTIONS
                .iter()
                .map(|(_, minutes)| format!("eid:42:sn:{minutes}"))
        };

        let expected: Vec<String> = snoozes()
            .chain(["eid:42:ed".to_owned(), "eid:42:del:n".to_owned()])
            .collect();
        assert_eq!(
            datas(expanded_notification_keyboard(42, false, false)),
            expected
        );

        let expected: Vec<String> = snoozes()
            .chain([
                "eid:42:dis:n".to_owned(),
                "eid:42:disr:n".to_owned(),
                "eid:42:ed".to_owned(),
                "eid:42:del:n".to_owned(),
            ])
            .collect();
        assert_eq!(
            datas(expanded_notification_keyboard(42, true, true)),
            expected
        );

        // Snooze buttons split four per row.
        let kb = expanded_notification_keyboard(42, true, false);
        assert_eq!(kb.inline_keyboard[0].len(), 4);
        assert_eq!(kb.inline_keyboard[1].len(), 4);
    }

    #[test]
    fn dismissed_notification_text_previews_from_new_next_datetime() {
        use crate::types::{NextSource, Repetition, TimeUnit};
        use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        // Daily repetition, already dismissed past tomorrow: next is 2026-06-24.
        let next = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 24).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        // A `10:00 every 1 day` event: time + repetition, like the parser builds.
        let mut event = sample_event("<b>call</b> the office", Some(next));
        event.time = NaiveTime::from_hms_opt(10, 0, 0);
        event.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Days,
        });
        event.source = Some(NextSource::Repetition);

        // The preview leads with the post-dismiss `next_datetime` itself, not
        // the occurrence the dismiss skipped.
        let text = dismissed_notification_text(&event, now, Tz::UTC, &EN);
        assert!(text.starts_with("🔔 <b>call</b> the office\n\n"));
        assert!(text.contains("Next launches:"));
        let first_bullet = text
            .lines()
            .find(|l| l.starts_with('▪'))
            .expect("preview bullet");
        assert!(
            first_bullet.contains("24.06.2026"),
            "first preview entry should be the new next_datetime: {first_bullet}"
        );
        assert!(!text.contains("23.06.2026"));

        // Nothing further to fire: the bare body, like a one-off fire.
        let inactive = sample_event("done", None);
        assert_eq!(
            dismissed_notification_text(&inactive, now, Tz::UTC, &EN),
            "🔔 done"
        );
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
        let msg = fired_message(&event, due, due, true, false, 5, Tz::UTC, &EN);
        assert_eq!(msg.chat_id, 7);
        assert_eq!(msg.text, "🔔 <b>call</b> the office");
        // The keyboard reflects the *post-fire* flags handed in: active → the
        // dismiss row is present (collapsed snooze row + dismiss + Edit/Delete).
        let kb = msg.reply_markup.expect("notification keyboard");
        assert_eq!(kb.inline_keyboard.len(), 3);
        assert_eq!(kb.inline_keyboard[0][1].text, "Snooze 5 mins");

        // Inactive post-fire state drops the dismiss row.
        let kb = fired_message(&event, due, due, false, false, 5, Tz::UTC, &EN)
            .reply_markup
            .expect("notification keyboard");
        assert_eq!(kb.inline_keyboard.len(), 2);
    }
}
