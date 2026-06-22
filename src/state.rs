use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{Local, NaiveDate, NaiveDateTime};

use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use teloxide::utils::html;

use crate::error::Result;
use crate::scheduler;
use crate::storage::EventStorage;
use crate::types::{ChatInfo, EventInfo, MessageInfo, MessageSender, TgMessage};

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

/// Inline keyboard attached to a fired reminder, offering to re-send it after a
/// fixed delay. Each button carries `eid:<id>:sn:<minutes>` callback data, where
/// `<id>` is the fired event's DB id (used to load the event when pressed).
fn snooze_keyboard(event_id: i64) -> InlineKeyboardMarkup {
    // Four buttons on the first row, the rest on the second, to fit narrow screens.
    let rows: Vec<Vec<InlineKeyboardButton>> = SNOOZE_OPTIONS
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
    InlineKeyboardMarkup::new(rows)
}

struct EventProviderState {
    storage: EventStorage,
    /// Next event to be processed. Stored in memory for efficiency.
    next_event: Option<EventInfo>,
}

/// Cloneable handle around shared storage plus the cached nearest event.
///
/// All methods take `&self` and lock the inner mutex internally, so the handle
/// can be cloned freely across the async message handler and the background
/// polling thread. The lock is only ever held for the duration of a synchronous
/// storage call — never across an `.await`.
#[derive(Clone)]
pub struct EventProvider {
    inner: Arc<Mutex<EventProviderState>>,
}

impl EventProvider {
    pub fn new(storage: EventStorage) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventProviderState {
                storage,
                next_event: None,
            })),
        }
    }

    pub fn upsert_chat(&self, chat: &ChatInfo) -> Result<()> {
        let inner = self.inner.lock().unwrap();
        inner.storage.upsert_chat(chat)
    }

    pub fn insert_message(&self, user_id: Option<i64>, chat_id: i64, message: &str) -> Result<i64> {
        let inner = self.inner.lock().unwrap();
        let msg = MessageInfo {
            id: 0,
            user_id,
            chat_id,
            created_at: None,
            message: message.to_string(),
        };
        inner.storage.insert_message(&msg)
    }

    /// Returns missed events (active events whose datetime is in the past).
    pub fn get_missed_events(&self) -> Vec<EventInfo> {
        let inner = self.inner.lock().unwrap();
        let now = Local::now().naive_local();
        match inner.storage.get_missed_events(now) {
            Ok(events) => events,
            Err(e) => {
                log::error!("Failed to get missed events: {}", e);
                Vec::new()
            }
        }
    }

    /// Returns the nearest active event, if any.
    pub fn get_next_event(&self) -> Option<EventInfo> {
        let inner = self.inner.lock().unwrap();
        inner.next_event.clone()
    }

    /// Returns an event by ID.
    pub fn get_event(&self, id: i64) -> Option<EventInfo> {
        let inner = self.inner.lock().unwrap();
        match inner.storage.get_event(id) {
            Ok(event) => event,
            Err(e) => {
                log::error!("Failed to get event {}: {}", id, e);
                None
            }
        }
    }

    /// Returns active events for a chat, ordered by next datetime.
    pub fn get_active_by_chat(&self, chat_id: i64) -> Vec<EventInfo> {
        let inner = self.inner.lock().unwrap();
        match inner.storage.get_active_by_chat(chat_id) {
            Ok(events) => events,
            Err(e) => {
                log::error!("Failed to get active events for chat {}: {}", chat_id, e);
                Vec::new()
            }
        }
    }

    /// Returns active events for a chat scheduled on the given date, ordered by next datetime.
    pub fn get_active_by_chat_on_date(&self, chat_id: i64, date: NaiveDate) -> Vec<EventInfo> {
        let inner = self.inner.lock().unwrap();
        match inner.storage.get_active_by_chat_on_date(chat_id, date) {
            Ok(events) => events,
            Err(e) => {
                log::error!(
                    "Failed to get events for chat {} on {}: {}",
                    chat_id,
                    date,
                    e
                );
                Vec::new()
            }
        }
    }

    /// Returns active events for a chat scheduled within `[start, end)`, ordered by next datetime.
    pub fn get_active_by_chat_in_range(
        &self,
        chat_id: i64,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Vec<EventInfo> {
        let inner = self.inner.lock().unwrap();
        match inner
            .storage
            .get_active_by_chat_in_range(chat_id, start, end)
        {
            Ok(events) => events,
            Err(e) => {
                log::error!(
                    "Failed to get events for chat {} in [{}, {}): {}",
                    chat_id,
                    start,
                    end,
                    e
                );
                Vec::new()
            }
        }
    }

    /// Returns all active events scheduled at the given datetime.
    fn get_events_at(&self, dt: NaiveDateTime) -> Vec<EventInfo> {
        let inner = self.inner.lock().unwrap();
        match inner.storage.get_events_at(dt) {
            Ok(events) => events,
            Err(e) => {
                log::error!("Failed to get events at {:?}: {}", dt, e);
                Vec::new()
            }
        }
    }

    /// Inserts a new event: calculates next datetime, persists to DB,
    /// reloads the next event, and returns the event as stored in DB.
    pub fn insert_event_and_get(&self, event: EventInfo) -> EventInfo {
        self.insert_event_and_get_at(event, Local::now().naive_local())
    }

    /// Inserts a new event: calculates next datetime at the given time,
    /// persists to DB, reloads the next event, and returns the event as stored in DB.
    pub fn insert_event_and_get_at(&self, event: EventInfo, now: NaiveDateTime) -> EventInfo {
        let mut inner = self.inner.lock().unwrap();
        let calculated = scheduler::calc_next_at(event, now);
        let id = match inner.storage.insert_event(&calculated) {
            Ok(id) => id,
            Err(e) => {
                log::error!("Failed to save event: {}", e);
                return calculated;
            }
        };

        // Reload to update the next event cache
        Self::load_next_event(&mut inner);

        match inner.storage.get_event(id) {
            Ok(Some(event)) => event,
            Ok(None) => {
                log::error!("Event {} not found after insert", id);
                calculated
            }
            Err(e) => {
                log::error!("Failed to get event {}: {}", id, e);
                calculated
            }
        }
    }

    /// Inserts an event exactly as supplied, without running the scheduler.
    ///
    /// Used by the legacy importer, where `next_datetime`/`active` are already
    /// computed (and periodic events must keep their stored next activation
    /// rather than being recalculated). Returns the new event id.
    pub fn insert_prebuilt_event(&self, event: &EventInfo) -> Result<i64> {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.storage.insert_event(event)?;
        Self::load_next_event(&mut inner);
        Ok(id)
    }

    /// Recalculates all given events and reloads the next event from DB.
    fn update_and_reload(&self, events: Vec<EventInfo>) {
        self.update_at_and_reload(events, Local::now().naive_local());
    }

    /// Recalculates all given events and reloads the next event from DB.
    pub fn update_at_and_reload(&self, events: Vec<EventInfo>, now: NaiveDateTime) {
        let mut inner = self.inner.lock().unwrap();
        for event in events {
            let event_id = event.id;
            let next = scheduler::calc_next_at(event, now);
            if let Err(e) = inner
                .storage
                .update_schedule(event_id, next.active, next.next_datetime)
            {
                log::error!("Failed to update schedule for event {}: {}", event_id, e);
            }
        }
        Self::load_next_event(&mut inner);
    }

    /// Starts the background polling thread. Reloads events from DB, sends missed events,
    /// then loops every second checking if the nearest event is due.
    pub fn start(&self, msg_tx: MessageSender) {
        // Initial reload and send missed events
        {
            let missed = self.get_missed_events();
            if !missed.is_empty() {
                log::info!("Sending {} missed event(s)", missed.len());

                let mut by_chat: HashMap<i64, Vec<&str>> = HashMap::new();
                for event in &missed {
                    by_chat
                        .entry(event.chat_id)
                        .or_default()
                        .push(&event.message);
                }

                let messages: Vec<TgMessage> = by_chat
                    .into_iter()
                    .map(|(chat_id, msgs)| {
                        let combined = msgs.join("\n");
                        TgMessage {
                            chat_id,
                            text: format!("Missed:\n{}", combined),
                            reply_markup: None,
                        }
                    })
                    .collect();

                if let Err(e) = msg_tx.send(messages) {
                    log::error!("Failed to queue missed messages: {}", e);
                }
            }
            self.update_and_reload(missed);
        }

        // Polling loop
        let provider = self.clone();
        std::thread::spawn(move || {
            let mut next_date: Option<NaiveDateTime> = None;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));

                let Some(event) = provider.get_next_event() else {
                    continue;
                };
                let Some(dt) = event.next_datetime else {
                    continue;
                };

                let now = Local::now().naive_local();
                if now >= dt {
                    let events = provider.get_events_at(dt);
                    let messages: Vec<TgMessage> = events
                        .iter()
                        .map(|e| {
                            // `e.message` is already an HTML fragment; the preview
                            // and hint are plain text, so escape them for HTML.
                            let preview = crate::telegram::next_launches_preview(e, dt);
                            TgMessage {
                                chat_id: e.chat_id,
                                text: format!(
                                    "{}{}\n\n{}",
                                    e.message,
                                    html::escape(&preview),
                                    html::escape(SNOOZE_HINT)
                                ),
                                reply_markup: Some(snooze_keyboard(e.id)),
                            }
                        })
                        .collect();

                    if let Err(e) = msg_tx.send(messages) {
                        log::error!("Failed to queue messages: {}", e);
                    }
                    provider.update_and_reload(events);
                } else if next_date.is_none() || next_date.unwrap() != dt {
                    next_date = Some(dt);
                    log::info!("Next event: {}", dt);
                }
            }
        });
    }

    /// Internal reload that operates on an already-locked inner.
    fn load_next_event(inner: &mut EventProviderState) {
        match inner.storage.get_next_event() {
            Ok(event) => {
                inner.next_event = event;
            }
            Err(e) => log::error!("Failed to load next event: {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snooze_keyboard_has_a_button_per_option() {
        let kb = snooze_keyboard(42);
        let count: usize = kb.inline_keyboard.iter().map(|row| row.len()).sum();
        assert_eq!(count, SNOOZE_OPTIONS.len());
    }

    #[test]
    fn snooze_keyboard_embeds_event_id_in_callback_data() {
        use teloxide::types::InlineKeyboardButtonKind;

        let kb = snooze_keyboard(42);
        for (button, (_, minutes)) in kb
            .inline_keyboard
            .iter()
            .flatten()
            .zip(SNOOZE_OPTIONS.iter())
        {
            let InlineKeyboardButtonKind::CallbackData(data) = &button.kind else {
                panic!("expected callback-data button");
            };
            assert_eq!(data, &format!("eid:42:sn:{minutes}"));
        }
    }
}
