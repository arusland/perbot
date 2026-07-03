use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{Duration, Local, Months, NaiveDate, NaiveDateTime};

use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use teloxide::utils::html;

use crate::error::Result;
use crate::scheduler;
use crate::storage::EventStorage;
use crate::types::{
    ChatInfo, EventInfo, MessageInfo, MessageSender, NextSource, PageRequest, PageResponse,
    TgMessage,
};

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
    /// Per-chat snapshot of the event ids that were missed at startup, in display
    /// order. Captured in `start()` before the missed events are rescheduled (which
    /// would otherwise make them un-queryable), so the missed-events list can be
    /// paged after the fact. In-memory only; empty on a fresh restart.
    missed_snapshot: HashMap<i64, Vec<i64>>,
}

/// Result of an [`EventProvider::dismiss`] request.
pub enum DismissOutcome {
    /// The event was advanced past its current occurrence; carries the updated
    /// event as stored (inactive when nothing followed).
    Dismissed(Box<EventInfo>),
    /// The event was already inactive — there was no `next_datetime` to advance
    /// past, so nothing changed.
    Inactive,
    /// No event with that id exists in the given chat (missing or foreign id).
    NotFound,
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
                missed_snapshot: HashMap::new(),
            })),
        }
    }

    pub fn upsert_chat(&self, chat: &ChatInfo) -> Result<()> {
        let inner = self.inner.lock().unwrap();
        inner.storage.upsert_chat(chat)
    }

    /// Writes a consistent snapshot of the database to `dest` (see
    /// `EventStorage::backup_to`). Used by the admin `/database` command.
    pub fn backup_database<P: AsRef<std::path::Path>>(&self, dest: P) -> Result<()> {
        let inner = self.inner.lock().unwrap();
        inner.storage.backup_to(dest)
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
    pub fn get_missed_events(&self) -> Result<Vec<EventInfo>> {
        let inner = self.inner.lock().unwrap();
        let now = Local::now().naive_local();
        inner.storage.get_missed_events(now)
    }

    /// Returns one page of the events recorded in the startup missed snapshot for
    /// `chat_id`, reloaded from storage (so they reflect their current,
    /// post-reschedule state), plus the snapshot's total size (for page-count
    /// math). Only the page's ids are loaded; ids deleted since startup are
    /// skipped (which can leave a page short — the snapshot is not re-counted).
    /// Empty when the chat had no missed events. Backs the missed-events list's
    /// page-turn callbacks.
    pub fn get_missed_snapshot_events(
        &self,
        chat_id: i64,
        page: PageRequest,
    ) -> Result<PageResponse> {
        let inner = self.inner.lock().unwrap();
        let Some(ids) = inner.missed_snapshot.get(&chat_id) else {
            return Ok(PageResponse {
                events: Vec::new(),
                total: 0,
            });
        };
        let total = ids.len();
        let start = page.offset().min(total);
        let end = (start + page.size).min(total);
        // Direct storage calls (not `self.get_event`, which would re-lock the
        // non-reentrant mutex); ids removed since the snapshot read back as `None`
        // and are skipped, while a real query failure propagates.
        let mut events = Vec::new();
        for id in &ids[start..end] {
            if let Some(event) = inner.storage.get_event(*id)? {
                events.push(event);
            }
        }
        Ok(PageResponse { events, total })
    }

    /// Returns the nearest active event, if any.
    pub fn get_next_event(&self) -> Option<EventInfo> {
        let inner = self.inner.lock().unwrap();
        inner.next_event.clone()
    }

    /// Returns an event by ID.
    pub fn get_event(&self, id: i64) -> Result<Option<EventInfo>> {
        let inner = self.inner.lock().unwrap();
        inner.storage.get_event(id)
    }

    /// Returns one page of the active events for a chat (ordered by next
    /// datetime) plus the total number of active events. Paging happens in SQL,
    /// so only `page.size` rows are ever loaded; the count and the page read
    /// the same locked storage, so they are consistent with each other.
    pub fn get_active_by_chat(&self, chat_id: i64, page: PageRequest) -> Result<PageResponse> {
        let inner = self.inner.lock().unwrap();
        let total = inner.storage.count_active_by_chat(chat_id)?;
        let events = inner
            .storage
            .get_active_by_chat(chat_id, page.size, page.offset())?;
        Ok(PageResponse { events, total })
    }

    /// Returns one page of the active events for a chat scheduled on the given
    /// date plus the day's total (see [`get_active_by_chat`](Self::get_active_by_chat)).
    pub fn get_active_by_chat_on_date(
        &self,
        chat_id: i64,
        date: NaiveDate,
        page: PageRequest,
    ) -> Result<PageResponse> {
        let inner = self.inner.lock().unwrap();
        let total = inner.storage.count_active_by_chat_on_date(chat_id, date)?;
        let events =
            inner
                .storage
                .get_active_by_chat_on_date(chat_id, date, page.size, page.offset())?;
        Ok(PageResponse { events, total })
    }

    /// Returns one page of the active events for a chat scheduled within
    /// `[start, end)` plus the range's total (see
    /// [`get_active_by_chat`](Self::get_active_by_chat)).
    pub fn get_active_by_chat_in_range(
        &self,
        chat_id: i64,
        start: NaiveDate,
        end: NaiveDate,
        page: PageRequest,
    ) -> Result<PageResponse> {
        let inner = self.inner.lock().unwrap();
        let total = inner
            .storage
            .count_active_by_chat_in_range(chat_id, start, end)?;
        let events = inner.storage.get_active_by_chat_in_range(
            chat_id,
            start,
            end,
            page.size,
            page.offset(),
        )?;
        Ok(PageResponse { events, total })
    }

    /// Returns all active events scheduled at the given datetime.
    fn get_events_at(&self, dt: NaiveDateTime) -> Result<Vec<EventInfo>> {
        let inner = self.inner.lock().unwrap();
        inner.storage.get_events_at(dt)
    }

    /// Inserts a new event: calculates next datetime, persists to DB,
    /// reloads the next event, and returns the event as stored in DB.
    pub fn insert_event_and_get(&self, event: EventInfo) -> Result<EventInfo> {
        self.insert_event_and_get_at(event, Local::now().naive_local())
    }

    /// Inserts a new event: calculates next datetime at the given time,
    /// persists to DB, reloads the next event, and returns the event as stored in DB.
    pub fn insert_event_and_get_at(
        &self,
        event: EventInfo,
        now: NaiveDateTime,
    ) -> Result<EventInfo> {
        let mut inner = self.inner.lock().unwrap();
        let calculated = scheduler::calc_next_at(event, now);
        let id = inner.storage.insert_event(&calculated)?;

        // Reload to update the next event cache
        Self::load_next_event(&mut inner)?;

        match inner.storage.get_event(id)? {
            Some(event) => Ok(event),
            None => {
                log::error!("Event {} not found after insert", id);
                Ok(calculated)
            }
        }
    }

    /// Replaces an existing event's time/recurrence + message: recalculates the
    /// schedule, persists the full row via `update_event`, reloads the next-event
    /// cache (the edited event may be or have been the cached next), and returns the
    /// event as stored. Used by the `/event<id>` edit flow.
    pub fn update_event_and_get(&self, event: EventInfo) -> Result<EventInfo> {
        self.update_event_and_get_at(event, Local::now().naive_local())
    }

    /// Like [`update_event_and_get`] but with an explicit `now` (for tests).
    pub fn update_event_and_get_at(
        &self,
        event: EventInfo,
        now: NaiveDateTime,
    ) -> Result<EventInfo> {
        let mut inner = self.inner.lock().unwrap();
        let id = event.id;
        let calculated = scheduler::calc_next_at(event, now);
        inner.storage.update_event(&calculated)?;

        // Reload to update the next event cache
        Self::load_next_event(&mut inner)?;

        match inner.storage.get_event(id)? {
            Some(event) => Ok(event),
            None => {
                log::error!("Event {} not found after update", id);
                Ok(calculated)
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
        Self::load_next_event(&mut inner)?;
        Ok(id)
    }

    /// Deletes an event by id and reloads the cached next event (the deleted
    /// event may have been the cached `next_event`). Returns `true` when a row
    /// was removed.
    pub fn delete(&self, id: i64) -> Result<bool> {
        let mut inner = self.inner.lock().unwrap();
        let deleted = inner.storage.delete(id)?;
        Self::load_next_event(&mut inner)?;
        Ok(deleted)
    }

    /// Recalculates all given events and reloads the next event from DB.
    fn update_and_reload(&self, events: Vec<EventInfo>) -> Result<()> {
        self.update_at_and_reload(events, Local::now().naive_local())
    }

    /// Recalculates all given events and reloads the next event from DB.
    pub fn update_at_and_reload(&self, events: Vec<EventInfo>, now: NaiveDateTime) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        for event in events {
            let event_id = event.id;
            let next = scheduler::calc_next_at(event, now);
            inner.storage.update_schedule(
                event_id,
                next.active,
                next.next_datetime,
                next.last_next_datetime,
                next.source,
            )?;
        }
        Self::load_next_event(&mut inner)?;
        Ok(())
    }

    /// Dismisses an event: advances it past its current occurrence by rescheduling
    /// as if "now" were one second after the current `next_datetime` — the next
    /// occurrence for a recurring event, or inactive for a one-off with nothing
    /// further. Access-checks `chat_id` against the stored event (callback ids are
    /// user-influenceable). Returns [`DismissOutcome::NotFound`] for a missing or
    /// foreign id and [`DismissOutcome::Inactive`] when the event has no
    /// `next_datetime` to advance past; otherwise persists the new schedule,
    /// reloads the next-event cache, and returns the updated event.
    pub fn dismiss(&self, id: i64, chat_id: i64) -> Result<DismissOutcome> {
        let mut inner = self.inner.lock().unwrap();
        let event = match inner.storage.get_event(id)? {
            Some(event) if event.chat_id == chat_id => event,
            _ => return Ok(DismissOutcome::NotFound),
        };
        let Some(next_dt) = event.next_datetime else {
            return Ok(DismissOutcome::Inactive);
        };

        let calculated = scheduler::calc_next_at(event, next_dt + Duration::seconds(1));
        inner.storage.update_schedule(
            id,
            calculated.active,
            calculated.next_datetime,
            calculated.last_next_datetime,
            calculated.source,
        )?;
        Self::load_next_event(&mut inner)?;

        let updated = inner.storage.get_event(id)?.unwrap_or(calculated);
        Ok(DismissOutcome::Dismissed(Box::new(updated)))
    }

    /// Dismisses the *repetition fills* of an event: advances it past every
    /// consecutive `NextSource::Repetition` occurrence to the next occurrence whose
    /// source is something else — the next anchor (a yearly short-date `Date` or a
    /// `MonthlyPattern`). Used by the `/event<id>` "Dismiss repetition" action, which
    /// is only offered when the current `source` is `Repetition`.
    ///
    /// Access-checks `chat_id` like [`dismiss`](Self::dismiss); returns
    /// [`DismissOutcome::NotFound`]/[`DismissOutcome::Inactive`] the same way.
    ///
    /// Only a short date (`date` set, year not explicit) or a `monthly_pattern` can
    /// ever produce a non-`Repetition` source once repeating; an event with neither
    /// anchor stays `Repetition` forever, so for those we skip the search and fall
    /// straight back to the ordinary single-step dismiss. When an anchored event's
    /// anchor somehow stays out of reach for 100 years, we likewise fall back to the
    /// single step. Advancing one interval at a time via `calc_next_at(_, prev + 1s)`
    /// mirrors [`dismiss`], so the fallback is exactly one ordinary dismiss.
    pub fn dismiss_repetition(&self, id: i64, chat_id: i64) -> Result<DismissOutcome> {
        let mut inner = self.inner.lock().unwrap();
        let event = match inner.storage.get_event(id)? {
            Some(event) if event.chat_id == chat_id => event,
            _ => return Ok(DismissOutcome::NotFound),
        };
        let Some(next_dt) = event.next_datetime else {
            return Ok(DismissOutcome::Inactive);
        };

        let has_anchor =
            (event.date.is_some() && !event.year_explicit) || event.monthly_pattern.is_some();

        // The single-step advance — identical to `dismiss`. Also the fallback when no
        // non-repetition occurrence is reachable.
        let fallback = scheduler::calc_next_at(event.clone(), next_dt + Duration::seconds(1));

        let chosen = if !has_anchor {
            fallback
        } else {
            let horizon = next_dt
                .checked_add_months(Months::new(1200))
                .unwrap_or(NaiveDateTime::MAX);
            let mut current = fallback.clone();
            loop {
                match current.source {
                    // Reached the next anchor (or the event went inactive): done.
                    Some(NextSource::Repetition) => {}
                    _ => break current,
                }
                let Some(cur_dt) = current.next_datetime else {
                    break current;
                };
                if cur_dt > horizon {
                    break fallback;
                }
                current = scheduler::calc_next_at(current, cur_dt + Duration::seconds(1));
            }
        };

        inner.storage.update_schedule(
            id,
            chosen.active,
            chosen.next_datetime,
            chosen.last_next_datetime,
            chosen.source,
        )?;
        Self::load_next_event(&mut inner)?;

        let updated = inner.storage.get_event(id)?.unwrap_or(chosen);
        Ok(DismissOutcome::Dismissed(Box::new(updated)))
    }

    /// Starts the background polling thread. Reloads events from DB, sends missed events,
    /// then loops every second checking if the nearest event is due.
    ///
    /// Returns `Err` if the startup reload (missed-events fetch + reschedule) hits a DB
    /// error, *before* the polling thread is spawned — the caller treats this as fatal
    /// (notify the admin, don't start the bot). Once the polling thread is running, its
    /// per-tick errors are logged in place (it has no caller to return them to).
    pub fn start(&self, msg_tx: MessageSender) -> Result<()> {
        // Initial reload and send missed events
        {
            let missed = self.get_missed_events()?;
            if !missed.is_empty() {
                log::info!("Sending {} missed event(s)", missed.len());

                // Group the missed events per chat (already ordered by
                // next_datetime, so per-chat order is preserved).
                let mut events_by_chat: HashMap<i64, Vec<EventInfo>> = HashMap::new();
                for event in &missed {
                    events_by_chat
                        .entry(event.chat_id)
                        .or_default()
                        .push(event.clone());
                }

                // Snapshot the missed ids per chat so the list can be paged later,
                // after these events are rescheduled below (which makes them
                // un-queryable via get_missed_events).
                {
                    let mut inner = self.inner.lock().unwrap();
                    inner.missed_snapshot = events_by_chat
                        .iter()
                        .map(|(chat_id, events)| (*chat_id, events.iter().map(|e| e.id).collect()))
                        .collect();
                }

                let now = Local::now().naive_local();
                let messages: Vec<TgMessage> = events_by_chat
                    .into_iter()
                    .map(|(chat_id, events)| {
                        let loc = crate::locale::for_chat(chat_id);
                        // Page 0 only; page turns re-fetch via the snapshot.
                        let first_page =
                            &events[..events.len().min(crate::telegram::LIST_PAGE_SIZE)];
                        let (text, reply_markup) = crate::commands::format_missed_page(
                            first_page,
                            events.len(),
                            now,
                            0,
                            loc,
                        );
                        TgMessage {
                            chat_id,
                            text,
                            reply_markup,
                        }
                    })
                    .collect();

                if let Err(e) = msg_tx.send(messages) {
                    log::error!("Failed to queue missed messages: {}", e);
                }
            }
            self.update_and_reload(missed)?;
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
                    let events = match provider.get_events_at(dt) {
                        Ok(events) => events,
                        Err(e) => {
                            log::error!("Failed to get events at {:?}: {}", dt, e);
                            continue;
                        }
                    };
                    let messages: Vec<TgMessage> = events
                        .iter()
                        .map(|e| {
                            // `e.message` and the preview are HTML fragments; the
                            // hint is plain text, so escape only the hint for HTML.
                            let loc = crate::locale::for_chat(e.chat_id);
                            let preview = crate::telegram::next_launches_preview(e, now, dt, loc);
                            TgMessage {
                                chat_id: e.chat_id,
                                text: format!(
                                    "{}{}\n\n{}",
                                    e.message,
                                    preview,
                                    html::escape(SNOOZE_HINT)
                                ),
                                reply_markup: Some(snooze_keyboard(e.id)),
                            }
                        })
                        .collect();

                    if let Err(e) = msg_tx.send(messages) {
                        log::error!("Failed to queue messages: {}", e);
                    }
                    if let Err(e) = provider.update_and_reload(events) {
                        log::error!("Failed to reschedule fired events: {}", e);
                    }
                } else if next_date.is_none() || next_date.unwrap() != dt {
                    next_date = Some(dt);
                    log::info!("Next event: {}", dt);
                }
            }
        });

        Ok(())
    }

    /// Internal reload that operates on an already-locked inner.
    fn load_next_event(inner: &mut EventProviderState) -> Result<()> {
        inner.next_event = inner.storage.get_next_event()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an in-memory provider with a single private chat and a backing
    /// message row so events (whose `msg_id`/`chat_id` are FKs) can be inserted.
    fn test_provider(chat_id: i64) -> (EventProvider, i64) {
        use crate::types::{ChatInfo, ChatType};
        let storage = EventStorage::open_in_memory().unwrap();
        let provider = EventProvider::new(storage);
        provider
            .upsert_chat(&ChatInfo {
                id: chat_id,
                chat_type: ChatType::Private,
                title: None,
                username: None,
                first_name: None,
                last_name: None,
                updated_at: None,
                created_at: None,
            })
            .unwrap();
        let msg_id = provider
            .insert_message(None, chat_id, "call the office")
            .unwrap();
        (provider, msg_id)
    }

    fn ndt(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        use chrono::NaiveTime;
        NaiveDateTime::new(
            NaiveDate::from_ymd_opt(y, m, d).unwrap(),
            NaiveTime::from_hms_opt(h, mi, s).unwrap(),
        )
    }

    /// A minimal event carrying just chat/message wiring; time fields are set by
    /// each test.
    fn base_event(chat_id: i64, msg_id: i64) -> EventInfo {
        EventInfo {
            id: 0,
            chat_id,
            date: None,
            time: None,
            year_explicit: false,
            days: None,
            years: None,
            message: "call the office".to_string(),
            active: false,
            next_datetime: None,
            source: None,
            last_next_datetime: None,
            created_at: ndt(2099, 1, 1, 0, 0, 0),
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: None,
            msg_id,
            legacy: false,
            snoozed: false,
        }
    }

    #[test]
    fn dismiss_repetition_jumps_to_next_date_anchor() {
        use crate::types::{Repetition, TimeUnit};
        use chrono::NaiveTime;
        let chat_id = 555;
        let (provider, msg_id) = test_provider(chat_id);

        // Short date (no explicit year) + every 2 days: the yearly Nov 5 date is the
        // anchor, the interval fills between. Mirrors the scheduler's
        // `play_short_date_with_repetition` data. Far-future `now` keeps the stored
        // short-date year safely in the past.
        let mut event = base_event(chat_id, msg_id);
        event.time = NaiveTime::from_hms_opt(11, 7, 0);
        event.date = NaiveDate::from_ymd_opt(2026, 11, 5);
        event.repetition = Some(Repetition {
            interval: 2,
            unit: TimeUnit::Days,
        });

        // First schedule → the Nov 5 anchor (source Date).
        let event = provider
            .insert_event_and_get_at(event, ndt(2099, 10, 1, 9, 0, 0))
            .unwrap();
        assert_eq!(event.next_datetime, Some(ndt(2099, 11, 5, 11, 7, 0)));
        assert_eq!(event.source, Some(NextSource::Date));

        // Advance once past the anchor → now on a repetition fill (source Repetition).
        provider
            .update_at_and_reload(vec![event.clone()], ndt(2099, 11, 5, 11, 7, 1))
            .unwrap();
        let stepped = provider.get_event(event.id).unwrap().unwrap();
        assert_eq!(stepped.next_datetime, Some(ndt(2099, 11, 7, 11, 7, 0)));
        assert_eq!(stepped.source, Some(NextSource::Repetition));

        // Dismiss repetition → skip the interval fills to the next yearly anchor.
        match provider.dismiss_repetition(event.id, chat_id).unwrap() {
            DismissOutcome::Dismissed(updated) => {
                assert_eq!(updated.next_datetime, Some(ndt(2100, 11, 5, 11, 7, 0)));
                assert_eq!(updated.source, Some(NextSource::Date));
                assert!(updated.active);
            }
            _ => panic!("expected Dismissed"),
        }
    }

    #[test]
    fn dismiss_repetition_without_anchor_advances_one_step() {
        use crate::types::{Repetition, TimeUnit};
        use chrono::NaiveTime;
        let chat_id = 556;
        let (provider, msg_id) = test_provider(chat_id);

        // Time-only + every 3 days: no anchor, so `source` is Repetition forever.
        // Dismiss repetition falls back to a single ordinary step.
        let mut event = base_event(chat_id, msg_id);
        event.time = NaiveTime::from_hms_opt(15, 30, 0);
        event.repetition = Some(Repetition {
            interval: 3,
            unit: TimeUnit::Days,
        });
        let event = provider
            .insert_event_and_get_at(event, ndt(2099, 10, 1, 9, 0, 0))
            .unwrap();
        let first = event.next_datetime.unwrap();

        match provider.dismiss_repetition(event.id, chat_id).unwrap() {
            DismissOutcome::Dismissed(updated) => {
                assert_eq!(updated.next_datetime, Some(first + Duration::days(3)));
                assert_eq!(updated.source, Some(NextSource::Repetition));
            }
            _ => panic!("expected Dismissed"),
        }
    }

    #[test]
    fn get_active_by_chat_pages_in_storage() {
        use chrono::NaiveTime;
        let chat_id = 557;
        let (provider, msg_id) = test_provider(chat_id);

        // Twelve one-off events, one hour apart.
        for hour in 0..12 {
            let mut event = base_event(chat_id, msg_id);
            event.date = NaiveDate::from_ymd_opt(2099, 5, 20);
            event.year_explicit = true;
            event.time = NaiveTime::from_hms_opt(hour, 0, 0);
            provider
                .insert_event_and_get_at(event, ndt(2099, 1, 1, 0, 0, 0))
                .unwrap();
        }

        let page0 = provider
            .get_active_by_chat(chat_id, PageRequest::new(0, 10))
            .unwrap();
        assert_eq!(page0.total, 12);
        assert_eq!(page0.events.len(), 10);
        assert_eq!(
            page0.events[0].next_datetime,
            Some(ndt(2099, 5, 20, 0, 0, 0))
        );

        let page1 = provider
            .get_active_by_chat(chat_id, PageRequest::new(1, 10))
            .unwrap();
        assert_eq!(page1.total, 12);
        assert_eq!(page1.events.len(), 2);
        assert_eq!(
            page1.events[1].next_datetime,
            Some(ndt(2099, 5, 20, 11, 0, 0))
        );

        let page2 = provider
            .get_active_by_chat(chat_id, PageRequest::new(2, 10))
            .unwrap();
        assert!(page2.events.is_empty());
    }

    #[test]
    fn dismiss_repetition_rejects_foreign_and_missing() {
        let (provider, _) = test_provider(1);
        assert!(matches!(
            provider.dismiss_repetition(9999, 1).unwrap(),
            DismissOutcome::NotFound
        ));
    }

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
