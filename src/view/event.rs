//! Single-event rendering: the scheduling confirmation, the upcoming-launches
//! preview, the `/event<id>` detail view, the re-parseable input reconstruction
//! and edit prompt, plus the event action keyboards.

use super::message::{BULLET, format_when};
use crate::locale::LocaleProvider;
use crate::scheduler;
use crate::types::{EventInfo, MonthlyPattern};
use chrono::NaiveDateTime;
use chrono_tz::Tz;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use teloxide::utils::html;

/// Maximum upcoming launches previewed for a reminder. A further `▪ ...` bullet
/// is shown when more launches follow.
const MAX_NEXT_PREVIEW: usize = 3;

/// Preview block of upcoming launches for a reminder, computed with
/// `scheduler::calc_next_at`. Lists up to MAX_NEXT_PREVIEW launches as bullets,
/// plus a trailing `▪ ...` when more remain. Returns "" for one-off events
/// (no future occurrence). `after` is the search baseline (the launch being
/// confirmed or fired), so the listed launches are strictly after it; `now` is
/// the relative-time origin, so each bullet's relative offset (`(1d)`) is
/// measured from the current moment rather than from `after`. Output is an HTML
/// fragment: the `<b>Next launches:</b>` header is bold, the bullets and
/// datetimes are plain (no HTML specials), so callers embed it verbatim into
/// their HTML output.
pub fn next_launches_preview(
    event: &EventInfo,
    now: NaiveDateTime,
    after: NaiveDateTime,
    tz: Tz,
    loc: &dyn LocaleProvider,
) -> String {
    let mut launches: Vec<NaiveDateTime> = Vec::new();
    let mut current = event.clone();
    let mut cursor = after;
    // Probe one beyond the limit so we know whether to show the "..." bullet.
    while launches.len() <= MAX_NEXT_PREVIEW {
        current = scheduler::calc_next_at(current, cursor, tz);
        match current.next_datetime {
            Some(next) => {
                launches.push(next);
                cursor = next;
            }
            None => break,
        }
    }
    if launches.is_empty() {
        return String::new();
    }
    let mut out = format!("\n\n<b>{}</b>", loc.next_launches_header());
    for dt in launches.iter().take(MAX_NEXT_PREVIEW) {
        out.push_str(&format!("\n{BULLET} {}", format_when(now, *dt, tz, loc)));
    }
    if launches.len() > MAX_NEXT_PREVIEW {
        out.push_str(&format!("\n{BULLET} ..."));
    }
    out
}

/// Confirmation sent when a reminder is scheduled (new parse): the
/// single-event detail view ([`event_detail`]) with a bold `Event created`
/// caption prepended, so the confirmation and `/event<id>` render identically.
/// Callers attach [`event_actions_keyboard`] themselves, exactly like the
/// detail view.
pub fn scheduled_message(
    event: &EventInfo,
    now: NaiveDateTime,
    tz: Tz,
    loc: &dyn LocaleProvider,
) -> String {
    detail_body(Some("✅ Event created"), event, now, tz, loc)
}

/// Confirmation sent when a snooze button creates its one-off copy: the same
/// captioned detail view as [`scheduled_message`], but titled `Event snoozed`.
pub fn snoozed_message(
    event: &EventInfo,
    now: NaiveDateTime,
    tz: Tz,
    loc: &dyn LocaleProvider,
) -> String {
    detail_body(Some("💤 Event snoozed"), event, now, tz, loc)
}

/// Builds the HTML edit prompt: the `lead` line followed by the event's current
/// input — its canonical time expression ([`EventInfo::normalize_time`]) and the
/// original message with its Telegram formatting intact (the stored HTML
/// fragment, inserted verbatim), so the user can select, copy and paste it as a
/// starting point. Pasting the formatted text round-trips: ingestion re-renders
/// the entities back to the same HTML.
pub fn edit_prompt(lead: &str, event: &EventInfo, loc: &dyn LocaleProvider) -> String {
    let time = html::escape(&event.normalize_time(loc));
    if event.message.is_empty() {
        format!("{}\n\n{}", html::escape(lead), time)
    } else {
        format!("{}\n\n{} {}", html::escape(lead), time, event.message)
    }
}

/// Human-readable recurrence period for an event, e.g. `"every 2 days"`,
/// `"every Friday"`, `"every first Sunday"`, `"last day of the month"`. Weekday
/// sets collapse contiguous runs of ≥3 days into full-name ranges
/// ([`crate::types::weekday_runs`]): `"every Monday-Sunday"`,
/// `"every Monday-Wednesday, Friday"`. Returns `None` for one-off events (no
/// recurrence). The recurrence-bearing fields are mutually exclusive, checked in
/// priority order. Output is plain text with no HTML specials.
fn describe_recurrence(e: &EventInfo, loc: &dyn LocaleProvider) -> Option<String> {
    let every = loc.every_word();
    if let Some(rep) = &e.repetition {
        let unit = loc.unit_label(rep.unit, rep.interval != 1);
        return Some(if rep.interval == 1 {
            format!("{every} {unit}")
        } else {
            format!("{every} {} {unit}", rep.interval)
        });
    }
    if let Some(days) = &e.days {
        let names = crate::types::weekday_runs(days)
            .into_iter()
            .map(|(first, last)| {
                let name = loc.weekday_full(first);
                if first == last {
                    name.to_string()
                } else {
                    format!("{name}-{}", loc.weekday_full(last))
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Some(format!("{every} {names}"));
    }
    if let Some(pattern) = &e.monthly_pattern {
        return Some(match pattern {
            MonthlyPattern::OrdinalWeekday(ord, wd) => {
                format!(
                    "{every} {} {}",
                    loc.ordinal_word(*ord),
                    loc.weekday_full(*wd)
                )
            }
            MonthlyPattern::LastDay => loc.last_day_phrase().to_string(),
            MonthlyPattern::DayOfMonth(d) => loc.day_of_month_recurrence(&loc.ordinal_suffix(*d)),
        });
    }
    None
}

/// The escaped datetime/relative/recurrence text shared by the when-lines:
/// `HH:MM dd.mm.yyyy (in <rel>[, <recurrence>])` (`(soon[, …])` when under a
/// minute away — the locale owns the preposition). `, <recurrence>` is appended
/// inside the parentheses, next to the relative time, when the event repeats.
/// Contains no HTML specials. Returns `—` for an event with no upcoming launch.
fn when_text(e: &EventInfo, now: NaiveDateTime, tz: Tz, loc: &dyn LocaleProvider) -> String {
    let recurrence = describe_recurrence(e, loc)
        .map(|r| format!(", {r}"))
        .unwrap_or_default();
    match e.next_datetime {
        Some(dt) => html::escape(&format!(
            "{} ({}{})",
            loc.format_datetime(crate::tz::to_local(dt, tz)),
            loc.format_relative_in((dt - now).num_seconds()),
            recurrence
        )),
        None => "—".to_string(),
    }
}

/// The bold datetime/relative bullet line of the `/events` two-line row:
/// `▪ <b><when_text></b>` (see [`when_text`]).
pub(super) fn event_when_line(
    e: &EventInfo,
    now: NaiveDateTime,
    tz: Tz,
    loc: &dyn LocaleProvider,
) -> String {
    format!("{BULLET} <b>{}</b>", when_text(e, now, tz, loc))
}

/// The shared single-event detail body: an optional bold caption first line,
/// the bold `Time: <when_text>` line, a blank line, the full HTML message
/// fragment (formatting preserved, not truncated), and the upcoming-launches preview
/// ([`next_launches_preview`], identical to a fired reminder; empty for
/// one-off events). An **inactive** (out-of-date) event instead renders a
/// single bold notice above the message body — "Event is out of date. Last
/// fired at <last_next_datetime>", or "Event was dismissed." when
/// `last_next_datetime` is still in the future (dismissed before it fired).
fn detail_body(
    caption: Option<&str>,
    event: &EventInfo,
    now: NaiveDateTime,
    tz: Tz,
    loc: &dyn LocaleProvider,
) -> String {
    let caption = caption
        .map(|c| format!("<b>{}</b>\n", html::escape(c)))
        .unwrap_or_default();
    if !event.active {
        let notice = match event.last_next_datetime {
            // A future last_next_datetime means the event never fired — it was
            // dismissed past its final occurrence.
            Some(dt) if dt > now => "⌛ Event was dismissed.".to_string(),
            Some(dt) => html::escape(&format!(
                "⌛ Event is out of date. Last fired at {}",
                loc.format_datetime(crate::tz::to_local(dt, tz))
            )),
            None => "⌛ Event is out of date.".to_string(),
        };
        return format!("{caption}<b>{notice}</b>\n\n{}", event.message);
    }
    let preview = match event.next_datetime {
        Some(dt) => next_launches_preview(event, now, dt, tz, loc),
        None => String::new(),
    };
    format!(
        "{caption}<b>{}: {}</b>\n\n{}{}",
        html::escape(loc.time_label()),
        when_text(event, now, tz, loc),
        event.message,
        preview
    )
}

/// Detailed single-event view for `/event<id>`: the caption-less
/// [`detail_body`] — bold `Time:` when-line, full message, launches preview.
pub fn event_detail(
    event: &EventInfo,
    now: NaiveDateTime,
    tz: Tz,
    loc: &dyn LocaleProvider,
) -> String {
    detail_body(None, event, now, tz, loc)
}

/// The action buttons shown under the `/event<id>` detail view: an optional
/// `⏭ Dismiss` (callback `eid:<id>:dis`, advances past the current occurrence —
/// only shown when the event is `active`), an optional `⏩ Dismiss repetition`
/// (callback `eid:<id>:disr`, skips the interval fills to the next anchor — only
/// shown when the event is `active` and its current source is `Repetition`),
/// `✏️ Edit` (callback `eid:<id>:ed`, starts the edit flow) and `🗑 Delete`
/// (callback `eid:<id>:del`, swaps in the [`delete_confirm_keyboard`] row).
pub fn event_actions_keyboard(
    event_id: i64,
    active: bool,
    is_repetition: bool,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    // Dismiss actions get their own first row (only for active events).
    if active {
        let mut dismiss_row = vec![InlineKeyboardButton::callback(
            "⏭ Dismiss next",
            format!("eid:{event_id}:dis"),
        )];
        if is_repetition {
            dismiss_row.push(InlineKeyboardButton::callback(
                "⏩ Dismiss repetition",
                format!("eid:{event_id}:disr"),
            ));
        }
        rows.push(dismiss_row);
    }
    // Edit / Delete are always present, on their own row.
    rows.push(vec![
        InlineKeyboardButton::callback("✏️ Edit", format!("eid:{event_id}:ed")),
        InlineKeyboardButton::callback("🗑 Delete", format!("eid:{event_id}:del")),
    ]);
    InlineKeyboardMarkup::new(rows)
}

/// The single Cancel button shown while the chat is editing an event (callback
/// `eid:<id>:edno`, drops the pending edit). Public so `main`'s edit-completion
/// re-prompts can reuse it.
pub fn edit_cancel_keyboard(event_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "Cancel",
        format!("eid:{event_id}:edno"),
    )]])
}

/// The confirmation row shown after the Delete button is tapped: a confirm
/// (`eid:<id>:delyes`) and a cancel (`eid:<id>:delno`) button. When the flow
/// started from a fired notification (`from_notification`), both callbacks
/// carry a trailing `:n` so the follow-up handlers keep the notification text
/// and restore the notification keyboard instead of the detail-view one.
pub fn delete_confirm_keyboard(event_id: i64, from_notification: bool) -> InlineKeyboardMarkup {
    let suffix = if from_notification { ":n" } else { "" };
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("✅ Yes, delete", format!("eid:{event_id}:delyes{suffix}")),
        InlineKeyboardButton::callback("❌ Cancel", format!("eid:{event_id}:delno{suffix}")),
    ]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::EN;
    use crate::view::test_support::sample_event;
    use chrono::{Duration, NaiveDate, NaiveTime, Weekday};

    #[test]
    fn scheduled_message_formats_datetime() {
        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 30).unwrap(),
            NaiveTime::from_hms_opt(13, 5, 0).unwrap(),
        );
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(13, 5, 0).unwrap(),
        );
        // A one-off event has no upcoming launches: caption, Time line with the
        // relative offset, then the message body.
        let event = sample_event("ring in the new year", Some(dt));
        assert_eq!(
            scheduled_message(&event, now, Tz::UTC, &EN),
            "<b>✅ Event created</b>\n<b>Time: 13:05 31.12.2027, Fri (in 1d)</b>\n\nring in the new year"
        );
    }

    #[test]
    fn scheduled_message_embeds_html_message_verbatim() {
        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        );
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        // `message` is already an HTML fragment; it is embedded as-is.
        let event = sample_event("<b>call</b> the office", Some(dt));
        assert_eq!(
            scheduled_message(&event, now, Tz::UTC, &EN),
            "<b>✅ Event created</b>\n<b>Time: 10:00 22.06.2026, Mon (in 1h)</b>\n\n<b>call</b> the office"
        );
    }

    #[test]
    fn scheduled_message_appends_preview_for_recurring() {
        use crate::types::{Repetition, TimeUnit};
        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        );
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let mut event = sample_event("standup", Some(dt));
        event.time = NaiveTime::from_hms_opt(10, 0, 0);
        event.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Days,
        });

        let text = scheduled_message(&event, now, Tz::UTC, &EN);
        // The recurrence rides inside the when-line parentheses, like /event<id>.
        assert!(text.starts_with(
            "<b>✅ Event created</b>\n<b>Time: 10:00 22.06.2026, Mon (in 1h, every day)</b>"
        ));
        assert!(text.contains("</b>\n\nstandup"));
        // Preview lists launches strictly after the confirmed datetime.
        assert!(text.contains("<b>Next launches:</b>"));
        assert!(text.contains("▪ 10:00 23.06.2026, Tue"));
        assert!(text.contains("▪ ..."));
    }

    #[test]
    fn snoozed_message_uses_snoozed_caption() {
        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        );
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
        );
        let event = sample_event("call the office", Some(dt));
        assert_eq!(
            snoozed_message(&event, now, Tz::UTC, &EN),
            "<b>💤 Event snoozed</b>\n<b>Time: 09:30 22.06.2026, Mon (in 30 mins)</b>\n\ncall the office"
        );
    }

    #[test]
    fn next_launches_preview_one_off_is_empty() {
        let fire = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let mut event = sample_event("call mom", Some(fire));
        event.time = NaiveTime::from_hms_opt(10, 0, 0);
        assert_eq!(next_launches_preview(&event, fire, fire, Tz::UTC, &EN), "");
    }

    #[test]
    fn next_launches_preview_recurring_shows_three_plus_ellipsis() {
        use crate::types::{Repetition, TimeUnit};
        let fire = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let mut event = sample_event("standup", Some(fire));
        event.time = NaiveTime::from_hms_opt(10, 0, 0);
        event.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Days,
        });

        let preview = next_launches_preview(&event, fire, fire, Tz::UTC, &EN);
        assert!(preview.starts_with("\n\n<b>Next launches:</b>"));
        // Three consecutive days after the firing day, then the overflow bullet.
        assert!(preview.contains("▪ 10:00 23.06.2026, Tue"));
        assert!(preview.contains("▪ 10:00 24.06.2026, Wed"));
        assert!(preview.contains("▪ 10:00 25.06.2026, Thu"));
        assert!(preview.contains("▪ ..."));
        assert_eq!(preview.matches('▪').count(), 4);
    }

    #[test]
    fn next_launches_preview_relative_measured_from_now_not_after() {
        use crate::types::{Repetition, TimeUnit};
        let fire = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let mut event = sample_event("standup", Some(fire));
        event.time = NaiveTime::from_hms_opt(10, 0, 0);
        event.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Days,
        });

        // `now` one day before the firing occurrence. The first upcoming launch
        // is 2026-06-23 10:00 — two days after `now`, so the relative offset must
        // read `(2d)` (measured from `now`), not `(1d)` (from `after`/`fire`).
        let now = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 21).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let preview = next_launches_preview(&event, now, fire, Tz::UTC, &EN);
        assert!(preview.contains("▪ 10:00 23.06.2026, Tue (2d)"));
    }

    #[test]
    fn next_launches_preview_fewer_than_three_has_no_ellipsis() {
        use std::collections::HashSet;
        // Year-restricted to 2027; firing on its second-to-last day leaves a single
        // future launch (2027-12-31 23:00) before the schedule is exhausted.
        let fire = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 30).unwrap(),
            NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
        );
        let mut event = sample_event("year end", Some(fire));
        event.time = NaiveTime::from_hms_opt(23, 0, 0);
        event.years = Some(HashSet::from([2027]));

        let preview = next_launches_preview(&event, fire, fire, Tz::UTC, &EN);
        assert!(preview.starts_with("\n\n<b>Next launches:</b>"));
        assert!(preview.contains("▪ 23:00 31.12.2027, Fri"));
        assert!(!preview.contains("▪ ..."));
        assert_eq!(preview.matches('▪').count(), 1);
    }

    #[test]
    fn edit_prompt_shows_time_and_formatted_message() {
        // The message keeps its Telegram formatting (HTML fragment verbatim).
        let mut e = sample_event("<b>call</b> her", None);
        e.time = Some(NaiveTime::from_hms_opt(8, 0, 0).unwrap());
        assert_eq!(
            edit_prompt("Edit:", &e, &EN),
            "Edit:\n\n08:00 <b>call</b> her"
        );

        // Recurrence (weekday set) is included via normalize_time.
        let mut r = sample_event("standup", None);
        r.time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        r.days = Some(std::collections::HashSet::from([
            chrono::Weekday::Mon,
            chrono::Weekday::Tue,
            chrono::Weekday::Wed,
            chrono::Weekday::Thu,
            chrono::Weekday::Fri,
        ]));
        assert_eq!(
            edit_prompt("Edit:", &r, &EN),
            "Edit:\n\n09:00 Mon-Fri standup"
        );

        // An already-escaped message body passes through unchanged; the lead is
        // escaped for HTML output.
        let mut s = sample_event("a &amp; b &lt;c&gt;", None);
        s.time = Some(NaiveTime::from_hms_opt(10, 0, 0).unwrap());
        let prompt = edit_prompt("Edit <now>:", &s, &EN);
        assert_eq!(prompt, "Edit &lt;now&gt;:\n\n10:00 a &amp; b &lt;c&gt;");

        // A body-less event (snoozed child) renders just the time expression.
        let mut t = sample_event("", None);
        t.time = Some(NaiveTime::from_hms_opt(11, 0, 0).unwrap());
        assert_eq!(edit_prompt("Edit:", &t, &EN), "Edit:\n\n11:00");
    }

    #[test]
    fn describe_recurrence_variants() {
        use crate::types::{Ordinal, Repetition, TimeUnit};
        use std::collections::HashSet;

        let mut e = sample_event("x", None);
        // One-off → no recurrence.
        assert_eq!(describe_recurrence(&e, &EN), None);

        // Interval repetition: plural and singular (n == 1).
        e.repetition = Some(Repetition {
            interval: 2,
            unit: TimeUnit::Days,
        });
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("every 2 days")
        );
        e.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Hours,
        });
        assert_eq!(describe_recurrence(&e, &EN).as_deref(), Some("every hour"));
        e.repetition = None;

        // Single weekday, then a sorted multi-day set (Mon before Fri).
        e.days = Some(HashSet::from([Weekday::Fri]));
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("every Friday")
        );
        e.days = Some(HashSet::from([Weekday::Fri, Weekday::Mon]));
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("every Monday, Friday")
        );

        // Contiguous runs of ≥3 days collapse into full-name ranges.
        e.days = Some(HashSet::from([
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ]));
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("every Monday-Sunday")
        );
        e.days = Some(HashSet::from([
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Fri,
        ]));
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("every Monday-Wednesday, Friday")
        );
        e.days = None;

        // Monthly patterns.
        e.monthly_pattern = Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Sun));
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("every first Sunday")
        );
        e.monthly_pattern = Some(MonthlyPattern::LastDay);
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("last day of the month")
        );
        e.monthly_pattern = Some(MonthlyPattern::DayOfMonth(28));
        assert_eq!(
            describe_recurrence(&e, &EN).as_deref(),
            Some("28th day of the month")
        );
    }

    #[test]
    fn event_detail_one_off_has_no_launches_block() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let e = sample_event(
            "<b>call</b> the office and bring the documents",
            Some(now + Duration::hours(2)),
        );
        let text = event_detail(&e, now, Tz::UTC, &EN);
        // Bold Time line, then the full untruncated HTML message verbatim.
        assert!(text.starts_with("<b>Time: 14:00 15.06.2026, Mon (in 2h)</b>\n\n"));
        assert!(text.contains("<b>call</b> the office and bring the documents"));
        // One-off: no upcoming-launches block.
        assert!(!text.contains("Next launches:"));
    }

    #[test]
    fn event_when_line_imminent_says_soon_without_in() {
        use crate::types::{Repetition, TimeUnit};
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let dt = now + Duration::seconds(30);
        let mut e = sample_event("standup", Some(dt));
        e.time = Some(dt.time());
        e.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Hours,
        });
        let text = event_detail(&e, now, Tz::UTC, &EN);
        // Under a minute away: bare "soon", never "in soon".
        assert!(text.starts_with("<b>Time: 12:00 15.06.2026, Mon (soon, every hour)</b>\n\n"));
        assert!(!text.contains("in soon"));
    }

    #[test]
    fn event_detail_recurring_shows_launches_block() {
        use crate::types::{Repetition, TimeUnit};
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let dt = now + Duration::hours(2);
        let mut e = sample_event("standup", Some(dt));
        e.time = Some(dt.time());
        e.repetition = Some(Repetition {
            interval: 1,
            unit: TimeUnit::Days,
        });
        let text = event_detail(&e, now, Tz::UTC, &EN);
        assert!(text.starts_with("<b>Time: 14:00 15.06.2026, Mon (in 2h, every day)</b>\n\n"));
        assert!(text.contains("standup"));
        // Recurring: launches block present, listing dates after the upcoming one.
        assert!(text.contains("<b>Next launches:</b>"));
        assert!(text.contains("▪ 14:00 16.06.2026, Tue"));
    }

    #[test]
    fn event_detail_inactive_shows_last_fired_notice() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let mut e = sample_event("expired reminder", None);
        e.last_next_datetime = Some(
            NaiveDateTime::parse_from_str("2026-06-10 09:30:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        );
        let text = event_detail(&e, now, Tz::UTC, &EN);
        assert!(text.starts_with(
            "<b>⌛ Event is out of date. Last fired at 09:30 10.06.2026, Wed</b>\n\n"
        ));
        assert!(text.contains("expired reminder"));
        // Inactive: no when-line relative time, no launches block.
        assert!(!text.contains("Next launches:"));
    }

    #[test]
    fn event_detail_dismissed_before_firing_shows_dismissed_notice() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let mut e = sample_event("dismissed reminder", None);
        // Dismissed one-off: last_next_datetime keeps the (future) fire time.
        e.last_next_datetime = Some(
            NaiveDateTime::parse_from_str("2026-06-20 09:30:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        );
        let text = event_detail(&e, now, Tz::UTC, &EN);
        assert!(text.starts_with("<b>⌛ Event was dismissed.</b>\n\n"));
        assert!(text.contains("dismissed reminder"));
        assert!(!text.contains("Last fired at"));
    }

    #[test]
    fn event_keyboards_embed_event_id_and_actions() {
        use teloxide::types::InlineKeyboardButtonKind::CallbackData;

        let datas = |kb: InlineKeyboardMarkup| -> Vec<String> {
            kb.inline_keyboard
                .concat()
                .iter()
                .map(|b| match &b.kind {
                    CallbackData(d) => d.clone(),
                    _ => panic!("expected callback data"),
                })
                .collect()
        };

        assert_eq!(
            datas(event_actions_keyboard(42, true, false)),
            ["eid:42:dis", "eid:42:ed", "eid:42:del"]
        );
        // Active + repetition source → the extra Dismiss-repetition button appears.
        assert_eq!(
            datas(event_actions_keyboard(42, true, true)),
            ["eid:42:dis", "eid:42:disr", "eid:42:ed", "eid:42:del"]
        );
        assert_eq!(
            datas(event_actions_keyboard(42, false, false)),
            ["eid:42:ed", "eid:42:del"]
        );
        // Inactive → no dismiss buttons even when the last source was repetition.
        assert_eq!(
            datas(event_actions_keyboard(42, false, true)),
            ["eid:42:ed", "eid:42:del"]
        );
        assert_eq!(
            datas(delete_confirm_keyboard(42, false)),
            ["eid:42:delyes", "eid:42:delno"]
        );
        // Started from a notification → the `:n` suffix rides along.
        assert_eq!(
            datas(delete_confirm_keyboard(42, true)),
            ["eid:42:delyes:n", "eid:42:delno:n"]
        );
        assert_eq!(datas(edit_cancel_keyboard(42)), ["eid:42:edno"]);

        // Row layout: dismiss actions sit on their own first row, Edit/Delete
        // on the second. Inactive events drop the dismiss row entirely.
        let rows = |kb: InlineKeyboardMarkup| -> Vec<Vec<String>> {
            kb.inline_keyboard
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|b| match &b.kind {
                            CallbackData(d) => d.clone(),
                            _ => panic!("expected callback data"),
                        })
                        .collect()
                })
                .collect()
        };
        assert_eq!(
            rows(event_actions_keyboard(42, true, true)),
            vec![
                vec!["eid:42:dis", "eid:42:disr"],
                vec!["eid:42:ed", "eid:42:del"],
            ]
        );
        assert_eq!(
            rows(event_actions_keyboard(42, true, false)),
            vec![vec!["eid:42:dis"], vec!["eid:42:ed", "eid:42:del"]]
        );
        assert_eq!(
            rows(event_actions_keyboard(42, false, false)),
            vec![vec!["eid:42:ed", "eid:42:del"]]
        );
    }
}
