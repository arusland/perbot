use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use rusqlite::{Connection, Result, params};
use std::collections::HashSet;
use std::path::Path;

use crate::parser::{
    EventInfo, MonthlyPattern, Ordinal, Repetition, TimeUnit, parse_days, unit_from_str,
};

/// User message information. Used both for inserting and for reading from the database.
/// `id` and `created_at` are `None` when constructing a value to insert
/// and `Some` when reading back from the database.
#[derive(Debug, Clone)]
pub struct MessageInfo {
    pub id: i64,
    pub user_id: Option<i64>,
    pub chat_id: i64,
    pub created_at: Option<NaiveDateTime>,
    pub message: String,
}

/// Chat type enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatType {
    Private,
    Group,
    Supergroup,
    Channel,
}

impl ChatType {
    fn as_str(&self) -> &'static str {
        match self {
            ChatType::Private => "private",
            ChatType::Group => "group",
            ChatType::Supergroup => "supergroup",
            ChatType::Channel => "channel",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "private" => Some(ChatType::Private),
            "group" => Some(ChatType::Group),
            "supergroup" => Some(ChatType::Supergroup),
            "channel" => Some(ChatType::Channel),
            _ => None,
        }
    }
}

/// Chat information. Used both for upserting and for reading from the database.
/// `updated_at` and `created_at` are `None` when constructing a value to upsert
/// and `Some` when reading back from the database.
#[derive(Debug, Clone)]
pub struct ChatInfo {
    pub id: i64,
    pub chat_type: ChatType,
    pub title: Option<String>,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub updated_at: Option<NaiveDateTime>,
    pub created_at: Option<NaiveDateTime>,
}

// --- Private serialization helpers ---

fn serialize_days(days: &HashSet<Weekday>) -> String {
    let order = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let weekday_to_str = |d: &Weekday| match d {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    };
    let mut day_strs: Vec<&str> = days.iter().map(weekday_to_str).collect();
    day_strs.sort_by_key(|d| order.iter().position(|o| o == d).unwrap_or(7));
    day_strs.join(",")
}

fn serialize_time_unit(unit: TimeUnit) -> &'static str {
    match unit {
        TimeUnit::Minutes => "minutes",
        TimeUnit::Hours => "hours",
        TimeUnit::Days => "days",
        TimeUnit::Weeks => "weeks",
        TimeUnit::Months => "months",
        TimeUnit::Years => "years",
    }
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
            let wd_str = match wd {
                Weekday::Mon => "mon",
                Weekday::Tue => "tue",
                Weekday::Wed => "wed",
                Weekday::Thu => "thu",
                Weekday::Fri => "fri",
                Weekday::Sat => "sat",
                Weekday::Sun => "sun",
            };
            format!("{}_{}", ord_str, wd_str)
        }
        MonthlyPattern::LastDay => "last_day".to_string(),
    }
}

// --- Private deserialization helpers ---

fn deserialize_monthly_pattern(s: &str) -> Option<MonthlyPattern> {
    if s == "last_day" {
        return Some(MonthlyPattern::LastDay);
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
                created_at      TEXT NOT NULL DEFAULT (datetime('now')),
                days            TEXT,
                repeat_interval INTEGER,
                repeat_unit     TEXT,
                in_offset       INTEGER,
                in_offset_unit  TEXT,
                bare_hour       INTEGER,
                monthly_pattern TEXT,
                msg_id          INTEGER NOT NULL REFERENCES messages(id),
                years           TEXT
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

        Ok(())
    }

    /// Inserts a new event into the database from a `EventInfo`.
    pub fn insert_event(&self, event: &EventInfo) -> Result<i64> {
        let date_str = event.date.map(|d| d.format("%Y-%m-%d").to_string());
        let time_str = event.time.map(|t| t.format("%H:%M:%S").to_string());
        let next_str = event
            .next_datetime
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        let days_str = event.days.as_ref().map(|d| serialize_days(d));
        let (repeat_interval, repeat_unit) = match &event.repetition {
            Some(rep) => (
                Some(rep.interval),
                Some(serialize_time_unit(rep.unit).to_string()),
            ),
            None => (None, None),
        };
        let (in_offset_val, in_offset_unit) = match event.in_offset {
            Some((v, u)) => (Some(v), Some(serialize_time_unit(u).to_string())),
            None => (None, None),
        };
        let monthly_str = event
            .monthly_pattern
            .as_ref()
            .map(serialize_monthly_pattern);
        let years_str = event.years.as_ref().map(|y| serialize_years(y));

        self.conn.execute(
            "INSERT INTO events (chat_id, date, time, year_explicit, message, active, next_datetime, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id, years)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                event.chat_id,
                date_str,
                time_str,
                event.year_explicit as i32,
                event.message,
                event.active as i32,
                next_str,
                days_str,
                repeat_interval,
                repeat_unit,
                in_offset_val,
                in_offset_unit,
                event.bare_hour,
                monthly_str,
                event.msg_id,
                years_str,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Retrieves an event by its ID.
    pub fn get(&self, id: i64) -> Result<Option<EventInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id, years
             FROM events WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![id])?;

        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_event(row)?))
        } else {
            Ok(None)
        }
    }

    /// Retrieves all events for a given chat.
    pub fn get_by_chat(&self, chat_id: i64) -> Result<Vec<EventInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id, years
             FROM events WHERE chat_id = ?1 ORDER BY next_datetime ASC",
        )?;

        let rows = stmt.query_map(params![chat_id], Self::row_to_event)?;

        rows.collect()
    }

    /// Retrieves all active events.
    pub fn get_active_events(&self) -> Result<Vec<EventInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id, years
             FROM events WHERE active = 1 ORDER BY next_datetime ASC",
        )?;

        let rows = stmt.query_map([], Self::row_to_event)?;

        rows.collect()
    }

    /// Retrieves active events for a specific chat.
    pub fn get_active_by_chat(&self, chat_id: i64) -> Result<Vec<EventInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id, years
             FROM events WHERE chat_id = ?1 AND active = 1 ORDER BY next_datetime ASC",
        )?;

        let rows = stmt.query_map(params![chat_id], Self::row_to_event)?;

        rows.collect()
    }

    /// Updates `active` and `next_datetime` for an event after `calc_next` is called.
    pub fn update_schedule(
        &self,
        id: i64,
        active: bool,
        next_datetime: Option<NaiveDateTime>,
    ) -> Result<()> {
        let next_str = next_datetime.map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        self.conn.execute(
            "UPDATE events SET active = ?1, next_datetime = ?2 WHERE id = ?3",
            params![active as i32, next_str, id],
        )?;
        Ok(())
    }

    /// Marks an event as inactive.
    pub fn mark_inactive(&self, id: i64) -> Result<bool> {
        let rows_affected = self
            .conn
            .execute("UPDATE events SET active = 0 WHERE id = ?1", params![id])?;

        Ok(rows_affected > 0)
    }

    /// Deletes an event by its ID.
    pub fn delete(&self, id: i64) -> Result<bool> {
        let rows_affected = self
            .conn
            .execute("DELETE FROM events WHERE id = ?1", params![id])?;

        Ok(rows_affected > 0)
    }

    /// Returns the single nearest active event from `now`.
    pub fn get_next_event(&self, now: NaiveDateTime) -> Result<Option<EventInfo>> {
        let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();

        // TODO: support getting missed events
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id, years
             FROM events WHERE active = 1 AND next_datetime >= ?1
             ORDER BY next_datetime ASC LIMIT 1",
        )?;

        let mut rows = stmt.query(params![now_str])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_event(row)?))
        } else {
            Ok(None)
        }
    }

    /// Returns all active events with the exact given `next_datetime`.
    pub fn get_events_at(&self, dt: NaiveDateTime) -> Result<Vec<EventInfo>> {
        let dt_str = dt.format("%Y-%m-%d %H:%M:%S").to_string();

        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id, years
             FROM events WHERE active = 1 AND next_datetime = ?1
             ORDER BY id ASC",
        )?;

        let rows = stmt.query_map(params![dt_str], Self::row_to_event)?;
        rows.collect()
    }

    /// Deletes all inactive events.
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

        rows.collect()
    }

    /// Converts a database row to a ChatInfo.
    fn row_to_chat(row: &rusqlite::Row) -> Result<ChatInfo> {
        let chat_type_str: String = row.get(1)?;
        let updated_str: String = row.get(6)?;
        let created_str: String = row.get(7)?;

        let chat_type = ChatType::from_str(&chat_type_str).unwrap_or(ChatType::Private);
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
    fn row_to_event(row: &rusqlite::Row) -> Result<EventInfo> {
        let date_str: Option<String> = row.get(2)?;
        let time_str: Option<String> = row.get(3)?;
        let next_str: Option<String> = row.get(7)?;
        let created_str: String = row.get(8)?;

        let date = date_str.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok());
        let time = time_str.and_then(|s| NaiveTime::parse_from_str(&s, "%H:%M:%S").ok());
        let next_datetime =
            next_str.and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok());
        let created_at = NaiveDateTime::parse_from_str(&created_str, "%Y-%m-%d %H:%M:%S").unwrap();

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

        Ok(EventInfo {
            id: row.get(0)?,
            chat_id: row.get(1)?,
            date,
            time,
            year_explicit: row.get::<_, i32>(4)? != 0,
            message: row.get(5)?,
            active: row.get::<_, i32>(6)? != 0,
            next_datetime,
            created_at,
            days,
            years,
            repetition,
            in_offset,
            bare_hour,
            monthly_pattern,
            msg_id: row.get(16)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{EventInfo, MonthlyPattern, Ordinal, Repetition, TimeUnit};

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
            created_at: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            repetition: None,
            in_offset: None,
            bare_hour: None,
            monthly_pattern: None,
            msg_id: 0,
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
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.id, id);
        assert_eq!(stored.chat_id, 12345);
        assert_eq!(stored.message, "test message");
        assert_eq!(stored.date, event.date);
        assert_eq!(stored.time, event.time);
        assert!(stored.year_explicit);
        assert!(stored.active);
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

        let stored = storage.get(id).unwrap().unwrap();
        assert!(stored.active);

        storage.mark_inactive(id).unwrap();

        let stored = storage.get(id).unwrap().unwrap();
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
        assert!(storage.get(id).unwrap().is_some());

        storage.delete(id).unwrap();
        assert!(storage.get(id).unwrap().is_none());
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

        assert!(storage.get(id1).unwrap().is_none());
        assert!(storage.get(id2).unwrap().is_none());
        assert!(storage.get(id3).unwrap().is_some());
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

        assert_eq!(ChatType::from_str("private"), Some(ChatType::Private));
        assert_eq!(ChatType::from_str("group"), Some(ChatType::Group));
        assert_eq!(ChatType::from_str("supergroup"), Some(ChatType::Supergroup));
        assert_eq!(ChatType::from_str("channel"), Some(ChatType::Channel));
        assert_eq!(ChatType::from_str("unknown"), None);
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
        let stored = storage.get(id).unwrap().unwrap();

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
        let stored = storage.get(id).unwrap().unwrap();

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
        let stored = storage.get(id).unwrap().unwrap();

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
        let stored = storage.get(id).unwrap().unwrap();

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
        let stored = storage.get(id).unwrap().unwrap();

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
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.monthly_pattern, Some(MonthlyPattern::LastDay));
    }

    #[test]
    fn test_monthly_pattern_none_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_event("no pattern");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

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
        let stored = storage.get(id).unwrap().unwrap();

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
        let stored = storage.get(id).unwrap().unwrap();

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
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.chat_id, big_chat_id);
        assert_eq!(stored.message, "big id test");
    }
}
