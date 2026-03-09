use chrono::{Local, NaiveDateTime};

use crate::parser::EventInfo;
use crate::scheduler;
use crate::storage::{ChatInfo, EventStorage, MessageInfo};

pub struct EventProvider {
    storage: EventStorage,
    next_event: Option<EventInfo>,
    abort_handle: Option<tokio::task::AbortHandle>,
}

impl EventProvider {
    pub fn new(storage: EventStorage) -> Self {
        Self {
            storage,
            next_event: None,
            abort_handle: None,
        }
    }

    pub fn upsert_chat(&self, chat: &ChatInfo) -> rusqlite::Result<()> {
        self.storage.upsert_chat(chat)
    }

    pub fn insert_message(
        &self,
        user_id: Option<i64>,
        chat_id: i64,
        message: &str,
    ) -> rusqlite::Result<i64> {
        let msg = MessageInfo {
            id: 0,
            user_id,
            chat_id,
            created_at: None,
            message: message.to_string(),
        };
        self.storage.insert_message(&msg)
    }

    /// Loads the nearest active event from DB into memory.
    pub fn reload(&mut self) {
        let now = Local::now().naive_local();
        match self.storage.get_next_event(now) {
            Ok(event) => {
                log::info!(
                    "Loaded next event from storage: {}",
                    event
                        .as_ref()
                        .map(|e| format!("id={}", e.id))
                        .unwrap_or_else(|| "none".to_string())
                );
                self.next_event = event;
            }
            Err(e) => log::error!("Failed to load next event: {}", e),
        }
    }

    /// Returns the nearest active event, if any.
    pub fn get_next(&self) -> Option<EventInfo> {
        self.next_event.clone()
    }

    /// Returns all active events scheduled at the given datetime.
    pub fn get_events_at(&self, dt: NaiveDateTime) -> Vec<EventInfo> {
        match self.storage.get_events_at(dt) {
            Ok(events) => events,
            Err(e) => {
                log::error!("Failed to get events at {:?}: {}", dt, e);
                Vec::new()
            }
        }
    }

    /// Inserts a new event: calculates next datetime, persists to DB,
    /// reloads the next event, and returns the stored event plus the next schedulable event.
    pub fn insert(&mut self, event: EventInfo) -> (EventInfo, Option<EventInfo>) {
        let stored = scheduler::calc_next(event);

        match self.storage.insert_event(&stored) {
            Ok(id) => log::info!("Saved event with id: {}", id),
            Err(e) => {
                log::error!("Failed to save event: {}", e);
                return (stored, self.get_next());
            }
        }

        self.reload();
        (stored, self.get_next())
    }

    /// Recalculates the event's next occurrence and saves to DB.
    pub fn update(&mut self, event: EventInfo) {
        let now = Local::now().naive_local();
        let event_id = event.id;
        let next = scheduler::calc_next_at(event, now);

        if let Err(e) = self
            .storage
            .update_schedule(event_id, next.active, next.next_datetime)
        {
            log::error!("Failed to update schedule for event {}: {}", event_id, e);
        }
    }

    /// Recalculates all given events and reloads the next event from DB.
    pub fn update_and_reload(&mut self, events: Vec<EventInfo>) {
        for event in events {
            self.update(event);
        }
        self.reload();
    }

    /// Stores the abort handle for the currently scheduled task.
    pub fn set_abort_handle(&mut self, handle: tokio::task::AbortHandle) {
        self.abort_handle = Some(handle);
    }

    /// Aborts the currently scheduled task, if any.
    pub fn abort_current(&mut self) {
        if let Some(handle) = self.abort_handle.take() {
            log::info!("Aborting previously scheduled task");
            handle.abort();
        }
    }
}
