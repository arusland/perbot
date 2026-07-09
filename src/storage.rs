use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use rusqlite::{Connection, params};
use std::collections::HashSet;
use std::path::Path;

use crate::error::Result;
use crate::types::{
    ChatInfo, ChatType, EventInfo, MessageInfo, MonthlyPattern, NextSource, Ordinal, Repetition,
    day_to_str, format_time_left, parse_days, unit_from_str,
};

// --- Private serialization helpers ---

fn serialize_days(days: &HashSet<Weekday>) -> String {
    let order = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let mut day_strs: Vec<&str> = days.iter().copied().map(day_to_str).collect();
    day_strs.sort_by_key(|d| order.iter().position(|o| o == d).unwrap_or(7));
    day_strs.join(",")
}

fn serialize_years(years: &HashSet<i32>) -> String {
    let mut sorted: Vec<i32> = years.iter().copied().collect();
    sorted.sort();
    sorted
        .iter()
        .map(|y| y.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn deserialize_years(s: &str) -> Option<HashSet<i32>> {
    let set: HashSet<i32> = s.split(',').filter_map(|y| y.trim().parse().ok()).collect();
    if set.is_empty() { None } else { Some(set) }
}

fn serialize_monthly_pattern(p: &MonthlyPattern) -> String {
    match p {
        MonthlyPattern::OrdinalWeekday(ord, wd) => {
            let ord_str = match ord {
                Ordinal::First => "first",
                Ordinal::Second => "second",
                Ordinal::Third => "third",
                Ordinal::Fourth => "fourth",
                Ordinal::Fifth => "fifth",
                Ordinal::Last => "last",
            };
            format!("{}_{}", ord_str, day_to_str(*wd))
        }
        MonthlyPattern::LastDay => "last_day".to_string(),
        MonthlyPattern::DayOfMonth(d) => format!("day_{d}"),
    }
}

// --- Private deserialization helpers ---

fn deserialize_monthly_pattern(s: &str) -> Option<MonthlyPattern> {
    if s == "last_day" {
        return Some(MonthlyPattern::LastDay);
    }
    if let Some(rest) = s.strip_prefix("day_") {
        return rest.parse().ok().map(MonthlyPattern::DayOfMonth);
    }
    let (ord_str, wd_str) = s.split_once('_')?;
    let ord = match ord_str {
        "first" => Ordinal::First,
        "second" => Ordinal::Second,
        "third" => Ordinal::Third,
        "fourth" => Ordinal::Fourth,
        "fifth" => Ordinal::Fifth,
        "last" => Ordinal::Last,
        _ => return None,
    };
    let wd = parse_days(wd_str)?.into_iter().next()?;
    Some(MonthlyPattern::OrdinalWeekday(ord, wd))
}

// --- Shared event-query fragments ---

/// The event column list in the exact positional order [`EventStorage::row_to_event`]
/// reads. `next_expr` fills the `next_datetime` slot (position 7): `e.next_datetime`
/// normally, `m.missed_at` for the missed snapshot. `message` resolves through the
/// parent (see [`EVENT_FROM`]) so snoozed events always carry their parent's text;
/// the `COALESCE` degrades to the child's own (empty) message if the parent row is
/// somehow gone.
fn event_cols(next_expr: &str) -> String {
    format!(
        "e.id, e.chat_id, e.date, e.time, e.year_explicit, \
         COALESCE(p.message, e.message) AS message, e.active, {next_expr}, \
         e.created_at, e.days, e.repeat_interval, e.repeat_unit, e.in_offset, \
         e.in_offset_unit, e.bare_hour, e.monthly_pattern, e.msg_id, e.years, \
         e.legacy, e.parent, e.last_next_datetime, e.source"
    )
}

/// The FROM clause every event SELECT uses: the self-join `p` supplies the parent's
/// message for snoozed events. WHERE/ORDER BY columns must be `e.`-qualified —
/// bare names are ambiguous against the join.
const EVENT_FROM: &str = "FROM events e LEFT JOIN events p ON e.parent = p.id";

/// SQLite-based storage for parsed events.
pub struct EventStorage {
    conn: Connection,
}

impl EventStorage {
    /// Opens or creates a database at the given path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    /// Creates an in-memory database (useful for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    /// Writes a consistent snapshot of the database to `dest` using `VACUUM INTO`.
    /// `dest` must not already exist (SQLite requirement); the caller removes it
    /// first. Yields a self-contained copy regardless of journal mode.
    pub fn backup_to<P: AsRef<Path>>(&self, dest: P) -> Result<()> {
        let path = dest.as_ref().to_string_lossy();
        self.conn.execute("VACUUM INTO ?1", [path.as_ref()])?;
        Ok(())
    }

    /// Initializes the database schema.
    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA foreign_keys = ON")?;

        // Chats table (must be created before events due to foreign key)
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS chats (
                id          INTEGER PRIMARY KEY,
                chat_type   TEXT NOT NULL,
                title       TEXT,
                username    TEXT,
                first_name  TEXT,
                last_name   TEXT,
                updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
                created_at  TEXT NOT NULL DEFAULT (datetime('now'))
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id     INTEGER,
                chat_id     INTEGER NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                message     TEXT NOT NULL
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS events (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id         INTEGER NOT NULL REFERENCES chats(id),
                date            TEXT,
                time            TEXT,
                year_explicit   INTEGER NOT NULL DEFAULT 0,
                message         TEXT NOT NULL,
                active          INTEGER NOT NULL DEFAULT 1,
                next_datetime   TEXT,
                last_next_datetime TEXT,
                created_at      TEXT NOT NULL DEFAULT (datetime('now')),
                days            TEXT,
                repeat_interval INTEGER,
                repeat_unit     TEXT,
                in_offset       INTEGER,
                in_offset_unit  TEXT,
                bare_hour       INTEGER,
                monthly_pattern TEXT,
                msg_id          INTEGER NOT NULL REFERENCES messages(id),
                years           TEXT,
                legacy          INTEGER NOT NULL DEFAULT 0,
                parent          INTEGER REFERENCES events(id) ON DELETE CASCADE,
                source          TEXT
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_chat_id ON events(chat_id)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_active ON events(active)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_next_datetime ON events(next_datetime)",
            [],
        )?;

        // Without this, deleting a parent scans the whole table for cascading
        // children.
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_parent ON events(parent)",
            [],
        )?;

        // Helper table recording the events that were missed at the last
        // startup, in missed (next_datetime) order — rescheduling wipes their
        // old next_datetime, so `missed_at` preserves the datetime each event
        // should have fired at and the autoincrement id keeps the list
        // pageable. Cleared at every startup before being repopulated.
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS missed_events (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id  INTEGER NOT NULL UNIQUE REFERENCES events(id) ON DELETE CASCADE,
                missed_at TEXT NOT NULL
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_missed_events_event_id ON missed_events(event_id)",
            [],
        )?;

        Ok(())
    }

    /// Inserts a new event into the database from a `EventInfo`.
    pub fn insert_event(&self, event: &EventInfo) -> Result<i64> {
        let date_str = event.date.map(|d| d.format("%Y-%m-%d").to_string());
        let time_str = event.time.map(|t| t.format("%H:%M:%S").to_string());
        let next_str = event
            .next_datetime
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        let last_next_str = event
            .last_next_datetime
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        let days_str = event.days.as_ref().map(serialize_days);
        let (repeat_interval, repeat_unit) = match &event.repetition {
            Some(rep) => (Some(rep.interval), Some(rep.unit.label(true).to_string())),
            None => (None, None),
        };
        let (in_offset_val, in_offset_unit) = match event.in_offset {
            Some((v, u)) => (Some(v), Some(u.label(true).to_string())),
            None => (None, None),
        };
        let monthly_str = event
            .monthly_pattern
            .as_ref()
            .map(serialize_monthly_pattern);
        let years_str = event.years.as_ref().map(serialize_years);
        let source_str = event.source.map(|s| s.as_str());

        self.conn.execute(
            "INSERT INTO events (chat_id, date, time, year_explicit, message, active, next_datetime, last_next_datetime, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id, years, legacy, parent, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                event.chat_id,
                date_str,
                time_str,
                event.year_explicit as i32,
                event.message,
                event.active as i32,
                next_str,
                last_next_str,
                days_str,
                repeat_interval,
                repeat_unit,
                in_offset_val,
                in_offset_unit,
                event.bare_hour,
                monthly_str,
                event.msg_id,
                years_str,
                event.legacy as i32,
                event.parent,
                source_str,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Retrieves all events for a given chat.
    pub fn get_by_chat(&self, chat_id: i64) -> Result<Vec<EventInfo>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} {EVENT_FROM} WHERE e.chat_id = ?1 ORDER BY e.next_datetime ASC",
            event_cols("e.next_datetime")
        ))?;

        let rows = stmt.query_map(params![chat_id], Self::row_to_event)?;

        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Retrieves all active events.
    pub fn get_active_events(&self) -> Result<Vec<EventInfo>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} {EVENT_FROM} WHERE e.active = 1 ORDER BY e.next_datetime ASC",
            event_cols("e.next_datetime")
        ))?;

        let rows = stmt.query_map([], Self::row_to_event)?;

        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Retrieves one page of the active events for a chat, ordered by next
    /// datetime: at most `limit` rows starting `offset` rows in. Paging happens
    /// in SQL so large lists never load whole.
    pub fn get_active_by_chat(
        &self,
        chat_id: i64,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<EventInfo>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} {EVENT_FROM}
             WHERE e.chat_id = ?1 AND e.active = 1 ORDER BY e.next_datetime ASC
             LIMIT ?2 OFFSET ?3",
            event_cols("e.next_datetime")
        ))?;

        let rows = stmt.query_map(
            params![chat_id, limit as i64, offset as i64],
            Self::row_to_event,
        )?;

        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Counts the active events for a chat (the total behind
    /// [`get_active_by_chat`](Self::get_active_by_chat) pages).
    pub fn count_active_by_chat(&self, chat_id: i64) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE chat_id = ?1 AND active = 1",
            params![chat_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Retrieves one page of the active events for a chat scheduled on the given
    /// calendar date (see [`get_active_by_chat`](Self::get_active_by_chat) for
    /// the `limit`/`offset` semantics).
    pub fn get_active_by_chat_on_date(
        &self,
        chat_id: i64,
        date: NaiveDate,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<EventInfo>> {
        let next_day = date.succ_opt().unwrap_or(date);
        self.get_active_by_chat_in_range(chat_id, date, next_day, limit, offset)
    }

    /// Counts the active events for a chat scheduled on the given calendar date.
    pub fn count_active_by_chat_on_date(&self, chat_id: i64, date: NaiveDate) -> Result<usize> {
        let next_day = date.succ_opt().unwrap_or(date);
        self.count_active_by_chat_in_range(chat_id, date, next_day)
    }

    /// Retrieves one page of the active events for a chat scheduled within
    /// `[start, end)` (end exclusive; see
    /// [`get_active_by_chat`](Self::get_active_by_chat) for the `limit`/`offset`
    /// semantics).
    pub fn get_active_by_chat_in_range(
        &self,
        chat_id: i64,
        start: NaiveDate,
        end: NaiveDate,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<EventInfo>> {
        let start_str = start
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let end_str = end
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} {EVENT_FROM}
             WHERE e.chat_id = ?1 AND e.active = 1 AND e.next_datetime >= ?2 AND e.next_datetime < ?3
             ORDER BY e.next_datetime ASC LIMIT ?4 OFFSET ?5",
            event_cols("e.next_datetime")
        ))?;

        let rows = stmt.query_map(
            params![chat_id, start_str, end_str, limit as i64, offset as i64],
            Self::row_to_event,
        )?;

        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Counts the active events for a chat scheduled within `[start, end)`.
    pub fn count_active_by_chat_in_range(
        &self,
        chat_id: i64,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<usize> {
        let start_str = start
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let end_str = end
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE chat_id = ?1 AND active = 1 AND next_datetime >= ?2 AND next_datetime < ?3",
            params![chat_id, start_str, end_str],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Replaces every parsed field of an event (time/recurrence + message) plus the
    /// recomputed `active`/`next_datetime`. Identity columns (`chat_id`, `msg_id`,
    /// `created_at`, `legacy`, `parent`) are left untouched. Used by the `/event<id>`
    /// edit flow. Returns `true` when a row was updated.
    pub fn update_event(&self, event: &EventInfo) -> Result<bool> {
        let date_str = event.date.map(|d| d.format("%Y-%m-%d").to_string());
        let time_str = event.time.map(|t| t.format("%H:%M:%S").to_string());
        let next_str = event
            .next_datetime
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        let last_next_str = event
            .last_next_datetime
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        let days_str = event.days.as_ref().map(serialize_days);
        let (repeat_interval, repeat_unit) = match &event.repetition {
            Some(rep) => (Some(rep.interval), Some(rep.unit.label(true).to_string())),
            None => (None, None),
        };
        let (in_offset_val, in_offset_unit) = match event.in_offset {
            Some((v, u)) => (Some(v), Some(u.label(true).to_string())),
            None => (None, None),
        };
        let monthly_str = event
            .monthly_pattern
            .as_ref()
            .map(serialize_monthly_pattern);
        let years_str = event.years.as_ref().map(serialize_years);
        let source_str = event.source.map(|s| s.as_str());

        let rows_affected = self.conn.execute(
            "UPDATE events SET date = ?1, time = ?2, year_explicit = ?3, message = ?4, active = ?5, next_datetime = ?6, last_next_datetime = ?7, days = ?8, repeat_interval = ?9, repeat_unit = ?10, in_offset = ?11, in_offset_unit = ?12, bare_hour = ?13, monthly_pattern = ?14, years = ?15, source = ?16
             WHERE id = ?17",
            params![
                date_str,
                time_str,
                event.year_explicit as i32,
                event.message,
                event.active as i32,
                next_str,
                last_next_str,
                days_str,
                repeat_interval,
                repeat_unit,
                in_offset_val,
                in_offset_unit,
                event.bare_hour,
                monthly_str,
                years_str,
                source_str,
                event.id,
            ],
        )?;

        Ok(rows_affected > 0)
    }

    /// Updates `active` and `next_datetime` for an event after `calc_next` is called.
    pub fn update_schedule(
        &self,
        id: i64,
        active: bool,
        next_datetime: Option<NaiveDateTime>,
        last_next_datetime: Option<NaiveDateTime>,
        source: Option<NextSource>,
    ) -> Result<()> {
        let next_str = next_datetime.map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        let last_next_str = last_next_datetime.map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        let source_str = source.map(|s| s.as_str());
        self.conn.execute(
            "UPDATE events SET active = ?1, next_datetime = ?2, last_next_datetime = ?3, source = ?4 WHERE id = ?5",
            params![active as i32, next_str, last_next_str, source_str, id],
        )?;
        let time_left = next_datetime
            .map(|dt| {
                format!(
                    " (in {})",
                    format_time_left(dt - Local::now().naive_local())
                )
            })
            .unwrap_or_default();
        log::info!(
            "Updated event {id}: active={active}, next_datetime={next_datetime:?}{time_left}, source={source:?}"
        );
        Ok(())
    }

    /// Marks an event as inactive.
    pub fn mark_inactive(&self, id: i64) -> Result<bool> {
        let rows_affected = self
            .conn
            .execute("UPDATE events SET active = 0 WHERE id = ?1", params![id])?;

        Ok(rows_affected > 0)
    }

    /// Deletes an event by its ID. Deleting a parent cascade-deletes its
    /// snoozed children.
    pub fn delete(&self, id: i64) -> Result<bool> {
        let rows_affected = self
            .conn
            .execute("DELETE FROM events WHERE id = ?1", params![id])?;

        log::info!("Event {id} deleted: {}", rows_affected > 0);
        Ok(rows_affected > 0)
    }

    /// Returns an event by its ID.
    pub fn get_event(&self, id: i64) -> Result<Option<EventInfo>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} {EVENT_FROM} WHERE e.id = ?1",
            event_cols("e.next_datetime")
        ))?;

        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_event(row)?))
        } else {
            Ok(None)
        }
    }

    /// Returns the single nearest active event from `now`.
    pub fn get_next_event(&self) -> Result<Option<EventInfo>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} {EVENT_FROM} WHERE e.active = 1
             ORDER BY e.next_datetime ASC LIMIT 1",
            event_cols("e.next_datetime")
        ))?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_event(row)?))
        } else {
            Ok(None)
        }
    }

    /// Returns up to `limit` active events whose `next_datetime` is before `now`
    /// (missed events), earliest first. Callers batch through the backlog by
    /// rescheduling each batch (which removes it from this predicate) and
    /// fetching again — no offset needed.
    pub fn get_missed_events(&self, now: NaiveDateTime, limit: usize) -> Result<Vec<EventInfo>> {
        let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();

        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} {EVENT_FROM} WHERE e.active = 1 AND e.next_datetime < ?1
             ORDER BY e.next_datetime ASC LIMIT ?2",
            event_cols("e.next_datetime")
        ))?;

        let rows = stmt.query_map(params![now_str, limit as i64], Self::row_to_event)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Empties the `missed_events` helper table. Called once at startup before
    /// the missed backlog is re-recorded.
    pub fn clear_missed_events(&self) -> Result<()> {
        self.conn.execute("DELETE FROM missed_events", [])?;
        Ok(())
    }

    /// Records a batch of missed events in the `missed_events` table as
    /// `(event id, missed datetime)` pairs — the datetime the event should
    /// have fired at, captured before rescheduling wipes it. Insertion order
    /// is preserved by the autoincrement `id`, so callers must pass entries
    /// in display (missed) order.
    pub fn insert_missed_events(&self, entries: &[(i64, NaiveDateTime)]) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare("INSERT INTO missed_events (event_id, missed_at) VALUES (?1, ?2)")?;
        for (id, missed_at) in entries {
            let missed_str = missed_at.format("%Y-%m-%d %H:%M:%S").to_string();
            stmt.execute(params![id, missed_str])?;
        }
        Ok(())
    }

    /// Returns the distinct chat ids that have events recorded in
    /// `missed_events`.
    pub fn get_missed_chat_ids(&self) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT e.chat_id FROM missed_events m
             JOIN events e ON e.id = m.event_id
             ORDER BY e.chat_id",
        )?;

        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Counts the recorded missed events for a chat (events deleted since the
    /// snapshot drop out via the join/cascade).
    pub fn count_missed_snapshot_by_chat(&self, chat_id: i64) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM missed_events m
             JOIN events e ON e.id = m.event_id
             WHERE e.chat_id = ?1",
            params![chat_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Returns one page of a chat's recorded missed events in the order they
    /// were missed (insertion order of `missed_events`). Each returned event
    /// carries the snapshot's `missed_at` — the datetime it should have fired
    /// at — in `next_datetime` instead of its post-reschedule value: the
    /// snapshot is display-only and the missed list shows the missed moment.
    pub fn get_missed_snapshot_by_chat(
        &self,
        chat_id: i64,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<EventInfo>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {}
             FROM missed_events m
             JOIN events e ON e.id = m.event_id
             LEFT JOIN events p ON e.parent = p.id
             WHERE e.chat_id = ?1
             ORDER BY m.id ASC LIMIT ?2 OFFSET ?3",
            event_cols("m.missed_at")
        ))?;

        let rows = stmt.query_map(
            params![chat_id, limit as i64, offset as i64],
            Self::row_to_event,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Returns all active events with the exact given `next_datetime`.
    pub fn get_events_at(&self, dt: NaiveDateTime) -> Result<Vec<EventInfo>> {
        let dt_str = dt.format("%Y-%m-%d %H:%M:%S").to_string();

        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} {EVENT_FROM} WHERE e.active = 1 AND e.next_datetime = ?1
             ORDER BY e.id ASC",
            event_cols("e.next_datetime")
        ))?;

        let rows = stmt.query_map(params![dt_str], Self::row_to_event)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Deletes all inactive events. Deleting an inactive parent cascades to its
    /// snoozed children, including still-active ones.
    pub fn delete_inactive(&self) -> Result<usize> {
        let rows_affected = self
            .conn
            .execute("DELETE FROM events WHERE active = 0", [])?;

        Ok(rows_affected)
    }

    /// Inserts or updates chat information.
    pub fn upsert_chat(&self, chat: &ChatInfo) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chats (id, chat_type, title, username, first_name, last_name, updated_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))
             ON CONFLICT(id) DO UPDATE SET
                chat_type = excluded.chat_type,
                title = excluded.title,
                username = excluded.username,
                first_name = excluded.first_name,
                last_name = excluded.last_name,
                updated_at = datetime('now')",
            params![
                chat.id,
                chat.chat_type.as_str(),
                chat.title,
                chat.username,
                chat.first_name,
                chat.last_name,
            ],
        )?;

        log::debug!("Chat information upserted: {:?}", chat);

        Ok(())
    }

    /// Inserts a user message and returns its ID.
    pub fn insert_message(&self, msg: &MessageInfo) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO messages (user_id, chat_id, message) VALUES (?1, ?2, ?3)",
            params![msg.user_id, msg.chat_id, msg.message],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Retrieves chat information by ID.
    pub fn get_chat(&self, id: i64) -> Result<Option<ChatInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_type, title, username, first_name, last_name, updated_at, created_at
             FROM chats WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![id])?;

        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_chat(row)?))
        } else {
            Ok(None)
        }
    }

    /// Retrieves all stored chats.
    pub fn get_all_chats(&self) -> Result<Vec<ChatInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_type, title, username, first_name, last_name, updated_at, created_at
             FROM chats ORDER BY updated_at DESC",
        )?;

        let rows = stmt.query_map([], Self::row_to_chat)?;

        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Converts a database row to a ChatInfo.
    fn row_to_chat(row: &rusqlite::Row) -> rusqlite::Result<ChatInfo> {
        let chat_type_str: String = row.get(1)?;
        let updated_str: String = row.get(6)?;
        let created_str: String = row.get(7)?;

        let chat_type = chat_type_str.parse().unwrap_or(ChatType::Private);
        let updated_at = NaiveDateTime::parse_from_str(&updated_str, "%Y-%m-%d %H:%M:%S").ok();
        let created_at = NaiveDateTime::parse_from_str(&created_str, "%Y-%m-%d %H:%M:%S").ok();

        Ok(ChatInfo {
            id: row.get(0)?,
            chat_type,
            title: row.get(2)?,
            username: row.get(3)?,
            first_name: row.get(4)?,
            last_name: row.get(5)?,
            updated_at,
            created_at,
        })
    }

    /// Converts a database row to a EventInfo, deserializing all fields.
    fn row_to_event(row: &rusqlite::Row) -> rusqlite::Result<EventInfo> {
        let date_str: Option<String> = row.get(2)?;
        let time_str: Option<String> = row.get(3)?;
        let next_str: Option<String> = row.get(7)?;
        let created_str: String = row.get(8)?;
        let last_next_str: Option<String> = row.get(20)?;

        let date = date_str.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok());
        let time = time_str.and_then(|s| NaiveTime::parse_from_str(&s, "%H:%M:%S").ok());
        let next_datetime =
            next_str.and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok());
        let last_next_datetime =
            last_next_str.and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok());
        let created_at =
            NaiveDateTime::parse_from_str(&created_str, "%Y-%m-%d %H:%M:%S").map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

        let days_str: Option<String> = row.get(9)?;
        let days = days_str.and_then(|s| parse_days(&s));

        let repeat_interval: Option<u32> = row.get(10)?;
        let repeat_unit_str: Option<String> = row.get(11)?;
        let repetition = match (repeat_interval, repeat_unit_str) {
            (Some(interval), Some(unit_str)) => {
                unit_from_str(&unit_str).map(|unit| Repetition { interval, unit })
            }
            _ => None,
        };

        let in_offset_val: Option<u32> = row.get(12)?;
        let in_offset_unit_str: Option<String> = row.get(13)?;
        let in_offset = match (in_offset_val, in_offset_unit_str) {
            (Some(v), Some(u)) => unit_from_str(&u).map(|unit| (v, unit)),
            _ => None,
        };

        let bare_hour: Option<u32> = row.get(14)?;
        let monthly_str: Option<String> = row.get(15)?;
        let monthly_pattern = monthly_str.and_then(|s| deserialize_monthly_pattern(&s));
        let years_str: Option<String> = row.get(17)?;
        let years = years_str.and_then(|s| deserialize_years(&s));
        let source_str: Option<String> = row.get(21)?;
        let source = source_str.and_then(|s| NextSource::from_str(&s));

        Ok(EventInfo {
            id: row.get(0)?,
            chat_id: row.get(1)?,
            date,
            time,
            year_explicit: row.get::<_, i32>(4)? != 0,
            message: row.get(5)?,
            active: row.get::<_, i32>(6)? != 0,
            next_datetime,
            source,
            last_next_datetime,
            created_at,
            days,
            years,
            repetition,
            in_offset,
            bare_hour,
            monthly_pattern,
            msg_id: row.get(16)?,
            legacy: row.get::<_, i32>(18)? != 0,
            parent: row.get(19)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ChatInfo, ChatType, EventInfo, MessageInfo, MonthlyPattern, NextSource, Ordinal,
        Repetition, TimeUnit,
    };

    fn ensure_chat(storage: &EventStorage, chat_id: i64) {
        storage
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
    }

    fn ensure_message(storage: &EventStorage, chat_id: i64) -> i64 {
        storage
            .insert_message(&MessageInfo {
                id: 0,
                user_id: None,
                chat_id,
                created_at: None,
                message: "test".to_string(),
            })
            .unwrap()
    }

    fn make_event(message: &str) -> EventInfo {
        EventInfo {
            id: 0,
            chat_id: 0,
            date: Some(NaiveDate::from_ymd_opt(2027, 12, 31).unwrap()),
            time: Some(NaiveTime::from_hms_opt(23, 59, 0).unwrap()),
            year_explicit: true,
            days: None,
            years: None,
            message: message.to_string(),
            active: true,
            next_datetime: Some(NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
                NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
            )),
            source: Some(NextSource::Date),
            last_next_datetime: None,
            created_at: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: None,
            msg_id: 0,
            legacy: false,
            parent: None,
        }
    }

    #[test]
    fn test_insert_and_get() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 12345);
        let mut event = make_event("test message");
        event.chat_id = 12345;
        event.msg_id = ensure_message(&storage, 12345);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(stored.id, id);
        assert_eq!(stored.chat_id, 12345);
        assert_eq!(stored.message, "test message");
        assert_eq!(stored.date, event.date);
        assert_eq!(stored.time, event.time);
        assert!(stored.year_explicit);
        assert!(stored.active);
    }

    #[test]
    fn test_last_next_datetime_round_trips_and_updates() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 555);
        let mut event = make_event("tracked");
        event.chat_id = 555;
        event.msg_id = ensure_message(&storage, 555);
        let fired = dt(2027, 12, 31, 23, 59);
        event.last_next_datetime = Some(fired);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();
        assert_eq!(stored.last_next_datetime, Some(fired));

        // update_schedule persists a new last_next_datetime even as the event
        // goes inactive.
        let later = dt(2028, 1, 1, 8, 0);
        storage
            .update_schedule(id, false, None, Some(later), None)
            .unwrap();
        let reloaded = storage.get_event(id).unwrap().unwrap();
        assert!(!reloaded.active);
        assert!(reloaded.next_datetime.is_none());
        assert_eq!(reloaded.last_next_datetime, Some(later));
        assert!(reloaded.source.is_none());
    }

    #[test]
    fn test_source_round_trips_and_updates() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 777);
        let mut event = make_event("sourced");
        event.chat_id = 777;
        event.msg_id = ensure_message(&storage, 777);
        event.source = Some(NextSource::Repetition);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();
        assert_eq!(stored.source, Some(NextSource::Repetition));

        // update_schedule persists a new source alongside the reschedule.
        storage
            .update_schedule(
                id,
                true,
                Some(dt(2028, 1, 1, 8, 0)),
                Some(dt(2028, 1, 1, 8, 0)),
                Some(NextSource::MonthlyPattern),
            )
            .unwrap();
        let reloaded = storage.get_event(id).unwrap().unwrap();
        assert_eq!(reloaded.source, Some(NextSource::MonthlyPattern));
    }

    #[test]
    fn test_get_by_chat() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 111);
        ensure_chat(&storage, 222);
        let msg_id_111 = ensure_message(&storage, 111);
        let msg_id_222 = ensure_message(&storage, 222);
        let mut event1 = make_event("event 1");
        event1.chat_id = 111;
        event1.msg_id = msg_id_111;
        let mut event2 = make_event("event 2");
        event2.chat_id = 111;
        event2.msg_id = msg_id_111;
        let mut event3 = make_event("event 1");
        event3.chat_id = 222;
        event3.msg_id = msg_id_222;

        storage.insert_event(&event1).unwrap();
        storage.insert_event(&event2).unwrap();
        storage.insert_event(&event3).unwrap();

        let events = storage.get_by_chat(111).unwrap();
        assert_eq!(events.len(), 2);

        let events = storage.get_by_chat(222).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_mark_inactive() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("deactivate me");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id = storage.insert_event(&event).unwrap();

        let stored = storage.get_event(id).unwrap().unwrap();
        assert!(stored.active);

        storage.mark_inactive(id).unwrap();

        let stored = storage.get_event(id).unwrap().unwrap();
        assert!(!stored.active);
    }

    #[test]
    fn test_get_active_events() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("active");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id1 = storage.insert_event(&event).unwrap();
        let id2 = storage.insert_event(&event).unwrap();

        storage.mark_inactive(id1).unwrap();

        let active = storage.get_active_events().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, id2);
    }

    #[test]
    fn test_delete() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("delete me");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id = storage.insert_event(&event).unwrap();
        assert!(storage.get_event(id).unwrap().is_some());

        storage.delete(id).unwrap();
        assert!(storage.get_event(id).unwrap().is_none());
    }

    #[test]
    fn test_update_event_replaces_fields_keeps_identity() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("original message");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id = storage.insert_event(&event).unwrap();
        let original = storage.get_event(id).unwrap().unwrap();

        // Build the replacement: a new clock time + weekday recurrence and a new
        // message, with the identity fields carried over (as the edit flow does).
        let mut updated = original.clone();
        updated.message = "updated message".to_string();
        updated.date = None;
        updated.year_explicit = false;
        updated.time = Some(NaiveTime::from_hms_opt(9, 30, 0).unwrap());
        updated.days = Some(HashSet::from([Weekday::Mon, Weekday::Fri]));
        updated.next_datetime = Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2030, 1, 4).unwrap(),
            NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
        ));

        assert!(storage.update_event(&updated).unwrap());

        let stored = storage.get_event(id).unwrap().unwrap();
        assert_eq!(stored.message, "updated message");
        assert_eq!(stored.time, updated.time);
        assert_eq!(stored.days, updated.days);
        assert_eq!(stored.date, None);
        assert!(!stored.year_explicit);
        assert_eq!(stored.next_datetime, updated.next_datetime);
        // Identity fields are untouched by update_event.
        assert_eq!(stored.id, id);
        assert_eq!(stored.chat_id, original.chat_id);
        assert_eq!(stored.msg_id, original.msg_id);
        assert_eq!(stored.created_at, original.created_at);
        assert_eq!(stored.parent, original.parent);
    }

    /// Inserts a parent + snoozed child pair for the parent-resolution tests:
    /// the child stores an empty message and points at the parent.
    fn insert_parent_and_child(storage: &EventStorage, chat_id: i64) -> (i64, i64) {
        ensure_chat(storage, chat_id);
        let mut parent = make_event("call mom");
        parent.chat_id = chat_id;
        parent.msg_id = ensure_message(storage, chat_id);
        let parent_id = storage.insert_event(&parent).unwrap();

        let mut child = make_event("");
        child.chat_id = chat_id;
        child.msg_id = parent.msg_id;
        child.parent = Some(parent_id);
        let child_id = storage.insert_event(&child).unwrap();
        (parent_id, child_id)
    }

    #[test]
    fn test_parent_round_trips_and_resolves_message() {
        let storage = EventStorage::open_in_memory().unwrap();
        let (parent_id, child_id) = insert_parent_and_child(&storage, 900);

        let parent = storage.get_event(parent_id).unwrap().unwrap();
        assert_eq!(parent.parent, None);
        assert!(!parent.is_snoozed());
        assert_eq!(parent.message, "call mom");

        // The child's stored message is empty; every read resolves the parent's.
        let child = storage.get_event(child_id).unwrap().unwrap();
        assert_eq!(child.parent, Some(parent_id));
        assert!(child.is_snoozed());
        assert_eq!(child.message, "call mom");

        // The list-shaped queries resolve through the same join.
        let listed = storage.get_by_chat(900).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|e| e.message == "call mom"));
    }

    #[test]
    fn test_editing_parent_message_propagates_to_child() {
        let storage = EventStorage::open_in_memory().unwrap();
        let (parent_id, child_id) = insert_parent_and_child(&storage, 901);

        let mut parent = storage.get_event(parent_id).unwrap().unwrap();
        parent.message = "call dad".to_string();
        assert!(storage.update_event(&parent).unwrap());

        let child = storage.get_event(child_id).unwrap().unwrap();
        assert_eq!(child.message, "call dad");
    }

    #[test]
    fn test_delete_parent_cascades_to_children() {
        let storage = EventStorage::open_in_memory().unwrap();
        let (parent_id, child_id) = insert_parent_and_child(&storage, 902);

        // Deleting a child leaves the parent intact.
        assert!(storage.delete(child_id).unwrap());
        assert!(storage.get_event(parent_id).unwrap().is_some());

        // Deleting the parent takes its remaining children with it.
        let (parent_id, child_id) = insert_parent_and_child(&storage, 902);
        assert!(storage.delete(parent_id).unwrap());
        assert!(storage.get_event(child_id).unwrap().is_none());
    }

    #[test]
    fn test_missed_snapshot_resolves_parent_message() {
        let storage = EventStorage::open_in_memory().unwrap();
        let (_, child_id) = insert_parent_and_child(&storage, 903);
        let missed_at = dt(2027, 6, 1, 10, 0);
        storage
            .insert_missed_events(&[(child_id, missed_at)])
            .unwrap();

        let snapshot = storage.get_missed_snapshot_by_chat(903, 10, 0).unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, child_id);
        assert_eq!(snapshot[0].message, "call mom");
        assert_eq!(snapshot[0].next_datetime, Some(missed_at));
    }

    #[test]
    fn test_delete_inactive() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("test");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id1 = storage.insert_event(&event).unwrap();
        let id2 = storage.insert_event(&event).unwrap();
        let id3 = storage.insert_event(&event).unwrap();

        storage.mark_inactive(id1).unwrap();
        storage.mark_inactive(id2).unwrap();

        let deleted = storage.delete_inactive().unwrap();
        assert_eq!(deleted, 2);

        assert!(storage.get_event(id1).unwrap().is_none());
        assert!(storage.get_event(id2).unwrap().is_none());
        assert!(storage.get_event(id3).unwrap().is_some());
    }

    #[test]
    fn test_upsert_and_get_chat() {
        let storage = EventStorage::open_in_memory().unwrap();

        let chat = ChatInfo {
            id: 12345,
            chat_type: ChatType::Private,
            title: None,
            username: Some("testuser".to_string()),
            first_name: Some("John".to_string()),
            last_name: Some("Doe".to_string()),
            updated_at: None,
            created_at: None,
        };

        storage.upsert_chat(&chat).unwrap();

        let stored = storage.get_chat(12345).unwrap().unwrap();
        assert_eq!(stored.id, 12345);
        assert_eq!(stored.chat_type, ChatType::Private);
        assert_eq!(stored.username, Some("testuser".to_string()));
        assert_eq!(stored.first_name, Some("John".to_string()));
        assert_eq!(stored.last_name, Some("Doe".to_string()));
    }

    #[test]
    fn test_upsert_chat_updates_existing() {
        let storage = EventStorage::open_in_memory().unwrap();

        let chat1 = ChatInfo {
            id: 12345,
            chat_type: ChatType::Private,
            title: None,
            username: Some("olduser".to_string()),
            first_name: Some("Old".to_string()),
            last_name: Some("Name".to_string()),
            updated_at: None,
            created_at: None,
        };

        storage.upsert_chat(&chat1).unwrap();

        let chat2 = ChatInfo {
            id: 12345,
            chat_type: ChatType::Private,
            title: None,
            username: Some("newuser".to_string()),
            first_name: Some("New".to_string()),
            last_name: Some("Name".to_string()),
            updated_at: None,
            created_at: None,
        };

        storage.upsert_chat(&chat2).unwrap();

        let stored = storage.get_chat(12345).unwrap().unwrap();
        assert_eq!(stored.username, Some("newuser".to_string()));
        assert_eq!(stored.first_name, Some("New".to_string()));
    }

    #[test]
    fn test_get_all_chats() {
        let storage = EventStorage::open_in_memory().unwrap();

        let chat1 = ChatInfo {
            id: 111,
            chat_type: ChatType::Private,
            title: None,
            username: Some("user1".to_string()),
            first_name: Some("User".to_string()),
            last_name: Some("One".to_string()),
            updated_at: None,
            created_at: None,
        };

        let chat2 = ChatInfo {
            id: 222,
            chat_type: ChatType::Group,
            title: Some("Test Group".to_string()),
            username: None,
            first_name: None,
            last_name: None,
            updated_at: None,
            created_at: None,
        };

        storage.upsert_chat(&chat1).unwrap();
        storage.upsert_chat(&chat2).unwrap();

        let chats = storage.get_all_chats().unwrap();
        assert_eq!(chats.len(), 2);
    }

    #[test]
    fn test_chat_type_conversion() {
        assert_eq!(ChatType::Private.as_str(), "private");
        assert_eq!(ChatType::Group.as_str(), "group");
        assert_eq!(ChatType::Supergroup.as_str(), "supergroup");
        assert_eq!(ChatType::Channel.as_str(), "channel");

        assert_eq!("private".parse(), Ok(ChatType::Private));
        assert_eq!("group".parse(), Ok(ChatType::Group));
        assert_eq!("supergroup".parse(), Ok(ChatType::Supergroup));
        assert_eq!("channel".parse(), Ok(ChatType::Channel));
        assert_eq!("unknown".parse::<ChatType>(), Err(()));
    }

    #[test]
    fn test_days_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 999);
        let mut event = make_event("weekday meeting");
        event.chat_id = 999;
        event.msg_id = ensure_message(&storage, 999);
        event.days = Some(HashSet::from([
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ]));
        event.time = Some(NaiveTime::from_hms_opt(13, 30, 0).unwrap());
        event.next_datetime = Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(13, 30, 0).unwrap(),
        ));

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(
            stored.days,
            Some(HashSet::from([
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
            ]))
        );
        assert_eq!(stored.message, "weekday meeting");
    }

    #[test]
    fn test_days_none_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 999);
        let mut event = make_event("no days");
        event.chat_id = 999;
        event.msg_id = ensure_message(&storage, 999);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(stored.days, None);
    }

    #[test]
    fn test_repetition_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("call office");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);
        event.date = Some(NaiveDate::from_ymd_opt(2027, 5, 20).unwrap());
        event.time = Some(NaiveTime::from_hms_opt(14, 55, 0).unwrap());
        event.year_explicit = false;
        event.repetition = Some(Repetition {
            interval: 2,
            unit: TimeUnit::Weeks,
        });
        event.next_datetime = Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 5, 20).unwrap(),
            NaiveTime::from_hms_opt(14, 55, 0).unwrap(),
        ));

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(
            stored.repetition,
            Some(Repetition {
                interval: 2,
                unit: TimeUnit::Weeks
            })
        );
        assert!(stored.active);
    }

    #[test]
    fn test_repetition_none_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("no repeat");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(stored.repetition, None);
        assert!(stored.active);
    }

    #[test]
    fn test_inactive_excluded_from_active() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("active test");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id1 = storage.insert_event(&event).unwrap();
        let id2 = storage.insert_event(&event).unwrap();

        storage.mark_inactive(id1).unwrap();

        let active = storage.get_active_events().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, id2);
    }

    #[test]
    fn test_monthly_pattern_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("call mom");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);
        event.date = None;
        event.time = Some(NaiveTime::from_hms_opt(10, 0, 0).unwrap());
        event.year_explicit = false;
        event.monthly_pattern = Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Sun));
        event.next_datetime = Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 3, 7).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        ));

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(
            stored.monthly_pattern,
            Some(MonthlyPattern::OrdinalWeekday(Ordinal::First, Weekday::Sun))
        );
        assert_eq!(stored.message, "call mom");
    }

    #[test]
    fn test_monthly_pattern_last_day_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("pay rent");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);
        event.date = None;
        event.time = Some(NaiveTime::from_hms_opt(18, 0, 0).unwrap());
        event.year_explicit = false;
        event.monthly_pattern = Some(MonthlyPattern::LastDay);
        event.next_datetime = Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 2, 28).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        ));

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(stored.monthly_pattern, Some(MonthlyPattern::LastDay));
    }

    #[test]
    fn test_monthly_pattern_day_of_month_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("call Mal");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);
        event.date = None;
        event.time = Some(NaiveTime::from_hms_opt(22, 15, 0).unwrap());
        event.year_explicit = false;
        event.monthly_pattern = Some(MonthlyPattern::DayOfMonth(28));
        event.next_datetime = Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 6, 28).unwrap(),
            NaiveTime::from_hms_opt(22, 15, 0).unwrap(),
        ));

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(stored.monthly_pattern, Some(MonthlyPattern::DayOfMonth(28)));
        assert_eq!(stored.message, "call Mal");
    }

    #[test]
    fn test_monthly_pattern_none_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("no pattern");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(stored.monthly_pattern, None);
    }

    #[test]
    fn test_years_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("yearly reminder");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);
        event.date = None;
        event.year_explicit = false;
        event.years = Some(HashSet::from([2027, 2028]));
        event.time = Some(NaiveTime::from_hms_opt(11, 13, 0).unwrap());
        event.next_datetime = None;
        event.active = false;

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(stored.years, Some(HashSet::from([2027, 2028])));
        assert_eq!(stored.message, "yearly reminder");
    }

    #[test]
    fn test_years_none_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("no years");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(stored.years, None);
    }

    #[test]
    fn test_big_chat_id_exceeding_i32() {
        let storage = EventStorage::open_in_memory().unwrap();
        let big_chat_id: i64 = i32::MAX as i64 + 1; // 2_147_483_648
        ensure_chat(&storage, big_chat_id);

        let mut event = make_event("big id test");
        event.chat_id = big_chat_id;
        event.msg_id = ensure_message(&storage, big_chat_id);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get_event(id).unwrap().unwrap();

        assert_eq!(stored.chat_id, big_chat_id);
        assert_eq!(stored.message, "big id test");
    }

    /// Builds an active event for `chat_id` scheduled at `dt`.
    fn event_at(chat_id: i64, msg_id: i64, dt: NaiveDateTime) -> EventInfo {
        let mut event = make_event("scheduled");
        event.chat_id = chat_id;
        event.msg_id = msg_id;
        event.date = Some(dt.date());
        event.time = Some(dt.time());
        event.next_datetime = Some(dt);
        event
    }

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDateTime::new(
            NaiveDate::from_ymd_opt(y, m, d).unwrap(),
            NaiveTime::from_hms_opt(h, min, 0).unwrap(),
        )
    }

    #[test]
    fn test_get_next_event_returns_nearest() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 1);
        let msg = ensure_message(&storage, 1);

        storage
            .insert_event(&event_at(1, msg, dt(2027, 6, 1, 12, 0)))
            .unwrap();
        let near_id = storage
            .insert_event(&event_at(1, msg, dt(2027, 1, 1, 8, 0)))
            .unwrap();
        storage
            .insert_event(&event_at(1, msg, dt(2027, 12, 1, 9, 0)))
            .unwrap();

        let next = storage.get_next_event().unwrap().unwrap();
        assert_eq!(next.id, near_id);
        assert_eq!(next.next_datetime, Some(dt(2027, 1, 1, 8, 0)));
    }

    #[test]
    fn test_get_next_event_skips_inactive() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 1);
        let msg = ensure_message(&storage, 1);

        let early = storage
            .insert_event(&event_at(1, msg, dt(2027, 1, 1, 8, 0)))
            .unwrap();
        let later = storage
            .insert_event(&event_at(1, msg, dt(2027, 2, 1, 8, 0)))
            .unwrap();
        storage.mark_inactive(early).unwrap();

        let next = storage.get_next_event().unwrap().unwrap();
        assert_eq!(next.id, later);
    }

    #[test]
    fn test_get_next_event_none_when_empty() {
        let storage = EventStorage::open_in_memory().unwrap();
        assert!(storage.get_next_event().unwrap().is_none());
    }

    #[test]
    fn test_get_missed_events_returns_only_past_ordered() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 1);
        let msg = ensure_message(&storage, 1);

        let past_late = storage
            .insert_event(&event_at(1, msg, dt(2026, 6, 1, 10, 0)))
            .unwrap();
        let past_early = storage
            .insert_event(&event_at(1, msg, dt(2026, 1, 1, 10, 0)))
            .unwrap();
        storage
            .insert_event(&event_at(1, msg, dt(2030, 1, 1, 10, 0)))
            .unwrap();

        let now = dt(2027, 1, 1, 0, 0);
        let missed = storage.get_missed_events(now, 10).unwrap();
        let ids: Vec<i64> = missed.iter().map(|e| e.id).collect();
        // Only the two past events, ordered by next_datetime ascending.
        assert_eq!(ids, vec![past_early, past_late]);

        // The limit caps the batch, keeping the earliest.
        let capped = storage.get_missed_events(now, 1).unwrap();
        let ids: Vec<i64> = capped.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![past_early]);
    }

    #[test]
    fn test_get_missed_events_excludes_inactive() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 1);
        let msg = ensure_message(&storage, 1);

        let id = storage
            .insert_event(&event_at(1, msg, dt(2026, 1, 1, 10, 0)))
            .unwrap();
        storage.mark_inactive(id).unwrap();

        let missed = storage.get_missed_events(dt(2027, 1, 1, 0, 0), 10).unwrap();
        assert!(missed.is_empty());
    }

    #[test]
    fn test_missed_events_table_pages_per_chat_in_insertion_order() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 1);
        ensure_chat(&storage, 2);
        let msg1 = ensure_message(&storage, 1);
        let msg2 = ensure_message(&storage, 2);

        let a = storage
            .insert_event(&event_at(1, msg1, dt(2026, 1, 1, 10, 0)))
            .unwrap();
        let b = storage
            .insert_event(&event_at(1, msg1, dt(2026, 2, 1, 10, 0)))
            .unwrap();
        let c = storage
            .insert_event(&event_at(1, msg1, dt(2026, 3, 1, 10, 0)))
            .unwrap();
        let other = storage
            .insert_event(&event_at(2, msg2, dt(2026, 1, 15, 10, 0)))
            .unwrap();

        storage
            .insert_missed_events(&[(a, dt(2026, 1, 1, 10, 0)), (other, dt(2026, 1, 15, 10, 0))])
            .unwrap();
        storage
            .insert_missed_events(&[(b, dt(2026, 2, 1, 10, 0)), (c, dt(2026, 3, 1, 10, 0))])
            .unwrap();

        assert_eq!(storage.get_missed_chat_ids().unwrap(), vec![1, 2]);
        assert_eq!(storage.count_missed_snapshot_by_chat(1).unwrap(), 3);
        assert_eq!(storage.count_missed_snapshot_by_chat(2).unwrap(), 1);

        // Pages keep insertion order and never mix chats.
        let page0 = storage.get_missed_snapshot_by_chat(1, 2, 0).unwrap();
        let page1 = storage.get_missed_snapshot_by_chat(1, 2, 2).unwrap();
        let ids0: Vec<i64> = page0.iter().map(|e| e.id).collect();
        let ids1: Vec<i64> = page1.iter().map(|e| e.id).collect();
        assert_eq!(ids0, vec![a, b]);
        assert_eq!(ids1, vec![c]);

        // Snapshot rows carry the recorded missed_at in next_datetime.
        assert_eq!(page0[0].next_datetime, Some(dt(2026, 1, 1, 10, 0)));
        assert_eq!(page0[1].next_datetime, Some(dt(2026, 2, 1, 10, 0)));
    }

    #[test]
    fn test_missed_events_table_clear_and_cascade_delete() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 1);
        let msg = ensure_message(&storage, 1);

        let a = storage
            .insert_event(&event_at(1, msg, dt(2026, 1, 1, 10, 0)))
            .unwrap();
        let b = storage
            .insert_event(&event_at(1, msg, dt(2026, 2, 1, 10, 0)))
            .unwrap();
        storage
            .insert_missed_events(&[(a, dt(2026, 1, 1, 10, 0)), (b, dt(2026, 2, 1, 10, 0))])
            .unwrap();

        // Deleting the event removes it from the missed list (cascade), so the
        // count and the page shrink together.
        assert!(storage.delete(a).unwrap());
        assert_eq!(storage.count_missed_snapshot_by_chat(1).unwrap(), 1);
        let ids: Vec<i64> = storage
            .get_missed_snapshot_by_chat(1, 10, 0)
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec![b]);

        storage.clear_missed_events().unwrap();
        assert_eq!(storage.count_missed_snapshot_by_chat(1).unwrap(), 0);
        assert!(storage.get_missed_chat_ids().unwrap().is_empty());
    }

    #[test]
    fn test_get_active_by_chat_on_date() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 1);
        ensure_chat(&storage, 2);
        let msg1 = ensure_message(&storage, 1);
        let msg2 = ensure_message(&storage, 2);

        // Two events for chat 1 today, at the day's boundaries.
        let morning = storage
            .insert_event(&event_at(1, msg1, dt(2026, 6, 16, 0, 0)))
            .unwrap();
        let night = storage
            .insert_event(&event_at(1, msg1, dt(2026, 6, 16, 23, 59)))
            .unwrap();
        // Next day for chat 1 must be excluded.
        storage
            .insert_event(&event_at(1, msg1, dt(2026, 6, 17, 0, 0)))
            .unwrap();
        // Same day but a different chat must be excluded.
        storage
            .insert_event(&event_at(2, msg2, dt(2026, 6, 16, 12, 0)))
            .unwrap();
        // Inactive same-day event must be excluded.
        let inactive = storage
            .insert_event(&event_at(1, msg1, dt(2026, 6, 16, 8, 0)))
            .unwrap();
        storage.mark_inactive(inactive).unwrap();

        let today = NaiveDate::from_ymd_opt(2026, 6, 16).unwrap();
        let events = storage
            .get_active_by_chat_on_date(1, today, 100, 0)
            .unwrap();
        let ids: Vec<i64> = events.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![morning, night]);
        assert_eq!(storage.count_active_by_chat_on_date(1, today).unwrap(), 2);
    }

    #[test]
    fn test_get_active_by_chat_in_range() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 1);
        ensure_chat(&storage, 2);
        let msg1 = ensure_message(&storage, 1);
        let msg2 = ensure_message(&storage, 2);

        // Events for chat 1 inside June 2026, at the month's boundaries.
        let first = storage
            .insert_event(&event_at(1, msg1, dt(2026, 6, 1, 0, 0)))
            .unwrap();
        let last = storage
            .insert_event(&event_at(1, msg1, dt(2026, 6, 30, 23, 59)))
            .unwrap();
        // Start of next month must be excluded (end is exclusive).
        storage
            .insert_event(&event_at(1, msg1, dt(2026, 7, 1, 0, 0)))
            .unwrap();
        // Previous month must be excluded.
        storage
            .insert_event(&event_at(1, msg1, dt(2026, 5, 31, 23, 59)))
            .unwrap();
        // Same month but a different chat must be excluded.
        storage
            .insert_event(&event_at(2, msg2, dt(2026, 6, 15, 12, 0)))
            .unwrap();
        // Inactive in-range event must be excluded.
        let inactive = storage
            .insert_event(&event_at(1, msg1, dt(2026, 6, 10, 8, 0)))
            .unwrap();
        storage.mark_inactive(inactive).unwrap();

        let start = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let events = storage
            .get_active_by_chat_in_range(1, start, end, 100, 0)
            .unwrap();
        let ids: Vec<i64> = events.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![first, last]);
        assert_eq!(
            storage
                .count_active_by_chat_in_range(1, start, end)
                .unwrap(),
            2
        );

        // Paging: one row per page, ordered by next_datetime.
        let page0 = storage
            .get_active_by_chat_in_range(1, start, end, 1, 0)
            .unwrap();
        let page1 = storage
            .get_active_by_chat_in_range(1, start, end, 1, 1)
            .unwrap();
        let page2 = storage
            .get_active_by_chat_in_range(1, start, end, 1, 2)
            .unwrap();
        assert_eq!(page0.iter().map(|e| e.id).collect::<Vec<_>>(), vec![first]);
        assert_eq!(page1.iter().map(|e| e.id).collect::<Vec<_>>(), vec![last]);
        assert!(page2.is_empty());
    }

    #[test]
    fn test_get_active_by_chat_pages_and_counts() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 1);
        let msg = ensure_message(&storage, 1);

        // Five active events, inserted out of order; pages follow next_datetime.
        let mut ids: Vec<i64> = Vec::new();
        for day in [3, 1, 5, 2, 4] {
            ids.push(
                storage
                    .insert_event(&event_at(1, msg, dt(2027, 1, day, 10, 0)))
                    .unwrap(),
            );
        }
        // An inactive event is excluded from pages and count alike.
        let inactive = storage
            .insert_event(&event_at(1, msg, dt(2027, 1, 6, 10, 0)))
            .unwrap();
        storage.mark_inactive(inactive).unwrap();

        assert_eq!(storage.count_active_by_chat(1).unwrap(), 5);

        let days = |events: Vec<EventInfo>| -> Vec<u32> {
            use chrono::Datelike;
            events
                .iter()
                .map(|e| e.next_datetime.unwrap().day())
                .collect()
        };
        assert_eq!(days(storage.get_active_by_chat(1, 2, 0).unwrap()), [1, 2]);
        assert_eq!(days(storage.get_active_by_chat(1, 2, 2).unwrap()), [3, 4]);
        assert_eq!(days(storage.get_active_by_chat(1, 2, 4).unwrap()), [5]);
        assert!(storage.get_active_by_chat(1, 2, 6).unwrap().is_empty());
    }

    #[test]
    fn test_get_events_at_exact_match_only() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 1);
        let msg = ensure_message(&storage, 1);

        let target = dt(2027, 5, 20, 14, 55);
        let a = storage.insert_event(&event_at(1, msg, target)).unwrap();
        let b = storage.insert_event(&event_at(1, msg, target)).unwrap();
        storage
            .insert_event(&event_at(1, msg, dt(2027, 5, 20, 14, 56)))
            .unwrap();

        let at = storage.get_events_at(target).unwrap();
        let mut ids: Vec<i64> = at.iter().map(|e| e.id).collect();
        ids.sort();
        assert_eq!(ids, vec![a, b]);
    }
}
