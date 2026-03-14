use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{Local, NaiveDateTime};

use crate::scheduler;
use crate::storage::EventStorage;
use crate::types::{ChatInfo, EventInfo, MessageInfo, MessageSender, TgMessage};

struct EventProviderState {
    storage: EventStorage,
    next_event: Option<EventInfo>,
    missed_events: Vec<EventInfo>,
}

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
                missed_events: Vec::new(),
            })),
        }
    }

    pub fn upsert_chat(&self, chat: &ChatInfo) -> rusqlite::Result<()> {
        let inner = self.inner.lock().unwrap();
        inner.storage.upsert_chat(chat)
    }

    pub fn insert_message(
        &self,
        user_id: Option<i64>,
        chat_id: i64,
        message: &str,
    ) -> rusqlite::Result<i64> {
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

    /// Loads the nearest active event and any missed events from DB into memory.
    pub fn reload(&self) {
        let mut inner = self.inner.lock().unwrap();
        let now = Local::now().naive_local();
        match inner.storage.get_next_event(now) {
            Ok(event) => {
                log::info!(
                    "Loaded next event from storage: {}",
                    event
                        .as_ref()
                        .map(|e| format!("id={}", e.id))
                        .unwrap_or_else(|| "none".to_string())
                );
                inner.next_event = event;
            }
            Err(e) => log::error!("Failed to load next event: {}", e),
        }
        match inner.storage.get_missed_events(now) {
            Ok(events) => {
                log::info!("Loaded {} missed event(s) from storage", events.len());
                inner.missed_events = events;
            }
            Err(e) => log::error!("Failed to load missed events: {}", e),
        }
    }

    /// Returns missed events (active events whose datetime is in the past).
    pub fn get_missed_events(&self) -> Vec<EventInfo> {
        let inner = self.inner.lock().unwrap();
        inner.missed_events.clone()
    }

    /// Returns the nearest active event, if any.
    pub fn get_next(&self) -> Option<EventInfo> {
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

    /// Returns all active events scheduled at the given datetime.
    pub fn get_events_at(&self, dt: NaiveDateTime) -> Vec<EventInfo> {
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
    pub fn insert_and_get(&self, event: EventInfo) -> EventInfo {
        self.insert_and_get_at(event, Local::now().naive_local())
    }

    /// Inserts a new event: calculates next datetime at the given time,
    /// persists to DB, reloads the next event, and returns the event as stored in DB.
    pub fn insert_and_get_at(&self, event: EventInfo, now: NaiveDateTime) -> EventInfo {
        let mut inner = self.inner.lock().unwrap();
        let calculated = scheduler::calc_next_at(event, now);
        let id = match inner.storage.insert_event(&calculated) {
            Ok(id) => {
                log::info!("Saved event with id: {}", id);
                id
            }
            Err(e) => {
                log::error!("Failed to save event: {}", e);
                return calculated;
            }
        };

        // Reload to update the next event cache
        Self::reload_inner(&mut inner);

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

    /// Recalculates the event's next occurrence and saves to DB.
    pub fn update(&self, event: EventInfo) {
        let now = Local::now().naive_local();
        self.update_at(event, now);
    }

    /// Recalculates the event's next occurrence at the given datetime and saves to DB.
    pub fn update_at(&self, event: EventInfo, now: NaiveDateTime) {
        let inner = self.inner.lock().unwrap();
        let event_id = event.id;
        let next = scheduler::calc_next_at(event, now);

        if let Err(e) = inner
            .storage
            .update_schedule(event_id, next.active, next.next_datetime)
        {
            log::error!("Failed to update schedule for event {}: {}", event_id, e);
        }
    }

    /// Recalculates all given events and reloads the next event from DB.
    pub fn update_and_reload(&self, events: Vec<EventInfo>) {
        let mut inner = self.inner.lock().unwrap();
        let now = Local::now().naive_local();
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
        Self::reload_inner(&mut inner);
    }

    /// Starts the background polling thread. Reloads events from DB, sends missed events,
    /// then loops every second checking if the nearest event is due.
    pub fn start(&self, msg_tx: MessageSender) {
        // Initial reload and send missed events
        {
            self.reload();

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
                        }
                    })
                    .collect();

                if let Err(e) = msg_tx.send(messages) {
                    log::error!("Failed to queue missed messages: {}", e);
                }
                self.update_and_reload(missed);
            }
        }

        // Polling loop
        let provider = self.clone();
        std::thread::spawn(move || {
            let mut next_date: Option<NaiveDateTime> = None;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));

                let Some(event) = provider.get_next() else {
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
                        .map(|e| TgMessage {
                            chat_id: e.chat_id,
                            text: e.message.clone(),
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
    fn reload_inner(inner: &mut EventProviderState) {
        let now = Local::now().naive_local();
        match inner.storage.get_next_event(now) {
            Ok(event) => {
                log::info!(
                    "Loaded next event from storage: {}",
                    event
                        .as_ref()
                        .map(|e| format!("id={}", e.id))
                        .unwrap_or_else(|| "none".to_string())
                );
                inner.next_event = event;
            }
            Err(e) => log::error!("Failed to load next event: {}", e),
        }
        match inner.storage.get_missed_events(now) {
            Ok(events) => {
                if !events.is_empty() {
                    log::info!("Loaded {} missed event(s) from storage", events.len());
                }
                inner.missed_events = events;
            }
            Err(e) => log::error!("Failed to load missed events: {}", e),
        }
    }
}
