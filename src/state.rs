use chrono::Local;

use crate::parser::EventInfo;
use crate::scheduler;
use crate::storage::{ChatInfo, EventStorage, MessageInfo};

pub struct EventProvider {
    storage: EventStorage,
    events: Vec<EventInfo>,
    abort_handle: Option<tokio::task::AbortHandle>,
}

impl EventProvider {
    pub fn new(storage: EventStorage) -> Self {
        Self {
            storage,
            events: Vec::new(),
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

    /// Loads the top nearest active events from DB into the in-memory list.
    pub fn reload(&mut self) {
        let now = Local::now().naive_local();
        match self.storage.get_top_events(now) {
            Ok(events) => {
                log::info!("Loaded {} top events from storage", events.len());
                self.events = events;
            }
            Err(e) => log::error!("Failed to load top events: {}", e),
        }
    }

    /// Returns the first (nearest) active event, if any.
    pub fn get_next(&self) -> Option<EventInfo> {
        self.events.first().cloned()
    }

    /// Inserts a new event: calculates next datetime, persists to DB,
    /// reloads the in-memory list, and returns the first active event.
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

    /// Recalculates the event's next occurrence, saves to DB,
    /// updates the in-memory list, and returns the first active event.
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

        // Remove the old entry
        self.events.retain(|e| e.id != event_id);

        // If still active, insert back in sorted position
        if next.active {
            let pos = self
                .events
                .iter()
                .position(|e| e.next_datetime > next.next_datetime)
                .unwrap_or(self.events.len());
            self.events.insert(pos, next);
        }

        // If list is empty, reload from DB
        if self.events.is_empty() {
            self.reload();
        }
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
