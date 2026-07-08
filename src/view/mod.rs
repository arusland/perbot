//! The presentation layer: every user-facing chat message — HTML text and
//! inline keyboards — is composed here and nowhere else. Producers (`state`,
//! `commands`, `main`) decide *which* builder to call; the builders own the
//! wording, markup, and button layout. Every time-bearing helper takes an
//! explicit `&dyn LocaleProvider`.
//!
//! Submodules are private; every public item is re-exported flat
//! (`view::event_detail`), mirroring the `commands::X` convention.

mod event;
mod list;
mod message;
mod notification;
mod prompt;

pub use event::{
    delete_confirm_keyboard, edit_cancel_keyboard, edit_prompt, event_actions_keyboard,
    event_detail, event_source_input, next_launches_preview, scheduled_message, snoozed_message,
};
pub use list::{
    LIST_PAGE_SIZE, ListKind, RowStyle, format_missed_page, format_page_at, list_keyboard,
    total_pages,
};
pub use message::{
    MESSAGE_MAX_LEN, MESSAGE_TRUNCATED, TELEGRAM_MAX_LEN, clamp_message, format_when,
    inactive_event_reply, rendered_len, unparsable_message,
};
pub use notification::{fired_message, notification_keyboard};
pub use prompt::{
    ASK_TEXT, CANCEL_DATA, EDIT_ASK_TEXT, EDIT_NEED_TEXT, EDIT_NEED_TIME, cancel_keyboard,
};

#[cfg(test)]
pub(crate) mod test_support {
    use crate::types::EventInfo;
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

    /// Minimal one-off event for rendering tests: carries `message` and, when
    /// `next` is given, is active with that upcoming datetime.
    pub fn sample_event(message: &str, next: Option<NaiveDateTime>) -> EventInfo {
        EventInfo {
            id: 0,
            chat_id: 0,
            date: None,
            time: None,
            year_explicit: false,
            days: None,
            years: None,
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: None,
            message: message.to_string(),
            active: next.is_some(),
            next_datetime: next,
            source: next.map(|_| crate::types::NextSource::Date),
            last_next_datetime: next,
            created_at: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            msg_id: 0,
            legacy: false,
            parent: None,
        }
    }
}
