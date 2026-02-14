use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rusqlite::{params, Connection, Result};
use std::path::Path;

use crate::parser::ParsedEvent;

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

/// Stored chat information.
#[derive(Debug, Clone)]
pub struct StoredChat {
    pub id: i64,
    pub chat_type: ChatType,
    pub title: Option<String>,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub updated_at: NaiveDateTime,
}

/// Chat info for upserting (without updated_at).
#[derive(Debug, Clone)]
pub struct ChatInfo {
    pub id: i64,
    pub chat_type: ChatType,
    pub title: Option<String>,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

/// A stored event with database metadata.
#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub id: i64,
    pub chat_id: i64,
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub year_explicit: bool,
    pub days: Option<String>,
    pub message: String,
    pub target_datetime: NaiveDateTime,
    pub created_at: NaiveDateTime,
    pub fired: bool,
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
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS events (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id         INTEGER NOT NULL,
                date            TEXT,
                time            TEXT,
                year_explicit   INTEGER NOT NULL DEFAULT 0,
                message         TEXT NOT NULL,
                target_datetime TEXT NOT NULL,
                created_at      TEXT NOT NULL DEFAULT (datetime('now')),
                fired           INTEGER NOT NULL DEFAULT 0,
                days            TEXT
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_chat_id ON events(chat_id)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_fired ON events(fired)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_target_datetime ON events(target_datetime)",
            [],
        )?;

        // Chats table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS chats (
                id          INTEGER PRIMARY KEY,
                chat_type   TEXT NOT NULL,
                title       TEXT,
                username    TEXT,
                first_name  TEXT,
                last_name   TEXT,
                updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
            )",
            [],
        )?;

        Ok(())
    }

    /// Inserts a new event into the database.
    pub fn insert(
        &self,
        chat_id: i64,
        event: &ParsedEvent,
        target_datetime: NaiveDateTime,
    ) -> Result<i64> {
        let date_str = event.date.map(|d| d.format("%Y-%m-%d").to_string());
        let time_str = event.time.map(|t| t.format("%H:%M:%S").to_string());
        let target_str = target_datetime.format("%Y-%m-%d %H:%M:%S").to_string();
        let days_str = event.days.as_ref().map(|days| {
            let mut day_strs: Vec<&str> = days
                .iter()
                .map(|d| match d {
                    chrono::Weekday::Mon => "mon",
                    chrono::Weekday::Tue => "tue",
                    chrono::Weekday::Wed => "wed",
                    chrono::Weekday::Thu => "thu",
                    chrono::Weekday::Fri => "fri",
                    chrono::Weekday::Sat => "sat",
                    chrono::Weekday::Sun => "sun",
                })
                .collect();
            let order = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
            day_strs.sort_by_key(|d| order.iter().position(|o| o == d).unwrap_or(7));
            day_strs.join(",")
        });

        self.conn.execute(
            "INSERT INTO events (chat_id, date, time, year_explicit, message, target_datetime, days)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                chat_id,
                date_str,
                time_str,
                event.year_explicit as i32,
                event.message,
                target_str,
                days_str,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Retrieves an event by its ID.
    pub fn get(&self, id: i64) -> Result<Option<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, target_datetime, created_at, fired, days
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
    pub fn get_by_chat(&self, chat_id: i64) -> Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, target_datetime, created_at, fired, days
             FROM events WHERE chat_id = ?1 ORDER BY target_datetime ASC",
        )?;

        let rows = stmt.query_map(params![chat_id], Self::row_to_event)?;

        rows.collect()
    }

    /// Retrieves all pending (not yet fired) events.
    pub fn get_pending(&self) -> Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, target_datetime, created_at, fired, days
             FROM events WHERE fired = 0 ORDER BY target_datetime ASC",
        )?;

        let rows = stmt.query_map([], Self::row_to_event)?;

        rows.collect()
    }

    /// Retrieves pending events for a specific chat.
    pub fn get_pending_by_chat(&self, chat_id: i64) -> Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, target_datetime, created_at, fired, days
             FROM events WHERE chat_id = ?1 AND fired = 0 ORDER BY target_datetime ASC",
        )?;

        let rows = stmt.query_map(params![chat_id], Self::row_to_event)?;

        rows.collect()
    }

    /// Marks an event as fired.
    pub fn mark_fired(&self, id: i64) -> Result<bool> {
        let rows_affected = self
            .conn
            .execute("UPDATE events SET fired = 1 WHERE id = ?1", params![id])?;

        Ok(rows_affected > 0)
    }

    /// Deletes an event by its ID.
    pub fn delete(&self, id: i64) -> Result<bool> {
        let rows_affected = self
            .conn
            .execute("DELETE FROM events WHERE id = ?1", params![id])?;

        Ok(rows_affected > 0)
    }

    /// Deletes all fired events.
    pub fn delete_fired(&self) -> Result<usize> {
        let rows_affected = self
            .conn
            .execute("DELETE FROM events WHERE fired = 1", [])?;

        Ok(rows_affected)
    }

    /// Inserts or updates chat information.
    pub fn upsert_chat(&self, chat: &ChatInfo) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chats (id, chat_type, title, username, first_name, last_name, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
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

    /// Retrieves chat information by ID.
    pub fn get_chat(&self, id: i64) -> Result<Option<StoredChat>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_type, title, username, first_name, last_name, updated_at
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
    pub fn get_all_chats(&self) -> Result<Vec<StoredChat>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_type, title, username, first_name, last_name, updated_at
             FROM chats ORDER BY updated_at DESC",
        )?;

        let rows = stmt.query_map([], Self::row_to_chat)?;

        rows.collect()
    }

    /// Converts a database row to a StoredChat.
    fn row_to_chat(row: &rusqlite::Row) -> Result<StoredChat> {
        let chat_type_str: String = row.get(1)?;
        let updated_str: String = row.get(6)?;

        let chat_type = ChatType::from_str(&chat_type_str).unwrap_or(ChatType::Private);
        let updated_at = NaiveDateTime::parse_from_str(&updated_str, "%Y-%m-%d %H:%M:%S").unwrap();

        Ok(StoredChat {
            id: row.get(0)?,
            chat_type,
            title: row.get(2)?,
            username: row.get(3)?,
            first_name: row.get(4)?,
            last_name: row.get(5)?,
            updated_at,
        })
    }

    /// Converts a database row to a StoredEvent.
    fn row_to_event(row: &rusqlite::Row) -> Result<StoredEvent> {
        let date_str: Option<String> = row.get(2)?;
        let time_str: Option<String> = row.get(3)?;
        let target_str: String = row.get(6)?;
        let created_str: String = row.get(7)?;
        let days: Option<String> = row.get(9)?;

        let date = date_str.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok());
        let time = time_str.and_then(|s| NaiveTime::parse_from_str(&s, "%H:%M:%S").ok());
        let target_datetime =
            NaiveDateTime::parse_from_str(&target_str, "%Y-%m-%d %H:%M:%S").unwrap();
        let created_at = NaiveDateTime::parse_from_str(&created_str, "%Y-%m-%d %H:%M:%S").unwrap();

        Ok(StoredEvent {
            id: row.get(0)?,
            chat_id: row.get(1)?,
            date,
            time,
            year_explicit: row.get::<_, i32>(4)? != 0,
            days,
            message: row.get(5)?,
            target_datetime,
            created_at,
            fired: row.get::<_, i32>(8)? != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_event(message: &str) -> ParsedEvent {
        ParsedEvent {
            date: Some(NaiveDate::from_ymd_opt(2027, 12, 31).unwrap()),
            time: Some(NaiveTime::from_hms_opt(23, 59, 0).unwrap()),
            year_explicit: true,
            days: None,
            period: None,
            repetition: None,
            message: message.to_string(),
        }
    }

    #[test]
    fn test_insert_and_get() {
        let storage = EventStorage::open_in_memory().unwrap();
        let event = create_test_event("test message");
        let target = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        );

        let id = storage.insert(12345, &event, target).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.id, id);
        assert_eq!(stored.chat_id, 12345);
        assert_eq!(stored.message, "test message");
        assert_eq!(stored.date, event.date);
        assert_eq!(stored.time, event.time);
        assert!(stored.year_explicit);
        assert!(!stored.fired);
    }

    #[test]
    fn test_get_by_chat() {
        let storage = EventStorage::open_in_memory().unwrap();
        let event1 = create_test_event("event 1");
        let event2 = create_test_event("event 2");
        let target = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        );

        storage.insert(111, &event1, target).unwrap();
        storage.insert(111, &event2, target).unwrap();
        storage.insert(222, &event1, target).unwrap();

        let events = storage.get_by_chat(111).unwrap();
        assert_eq!(events.len(), 2);

        let events = storage.get_by_chat(222).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_mark_fired() {
        let storage = EventStorage::open_in_memory().unwrap();
        let event = create_test_event("fire me");
        let target = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        );

        let id = storage.insert(123, &event, target).unwrap();

        let stored = storage.get(id).unwrap().unwrap();
        assert!(!stored.fired);

        storage.mark_fired(id).unwrap();

        let stored = storage.get(id).unwrap().unwrap();
        assert!(stored.fired);
    }

    #[test]
    fn test_get_pending() {
        let storage = EventStorage::open_in_memory().unwrap();
        let event = create_test_event("pending");
        let target = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        );

        let id1 = storage.insert(123, &event, target).unwrap();
        let id2 = storage.insert(123, &event, target).unwrap();

        storage.mark_fired(id1).unwrap();

        let pending = storage.get_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id2);
    }

    #[test]
    fn test_delete() {
        let storage = EventStorage::open_in_memory().unwrap();
        let event = create_test_event("delete me");
        let target = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        );

        let id = storage.insert(123, &event, target).unwrap();
        assert!(storage.get(id).unwrap().is_some());

        storage.delete(id).unwrap();
        assert!(storage.get(id).unwrap().is_none());
    }

    #[test]
    fn test_delete_fired() {
        let storage = EventStorage::open_in_memory().unwrap();
        let event = create_test_event("test");
        let target = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        );

        let id1 = storage.insert(123, &event, target).unwrap();
        let id2 = storage.insert(123, &event, target).unwrap();
        let id3 = storage.insert(123, &event, target).unwrap();

        storage.mark_fired(id1).unwrap();
        storage.mark_fired(id2).unwrap();

        let deleted = storage.delete_fired().unwrap();
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
        };

        storage.upsert_chat(&chat1).unwrap();

        let chat2 = ChatInfo {
            id: 12345,
            chat_type: ChatType::Private,
            title: None,
            username: Some("newuser".to_string()),
            first_name: Some("New".to_string()),
            last_name: Some("Name".to_string()),
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
        };

        let chat2 = ChatInfo {
            id: 222,
            chat_type: ChatType::Group,
            title: Some("Test Group".to_string()),
            username: None,
            first_name: None,
            last_name: None,
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
        use chrono::Weekday;
        use std::collections::HashSet;

        let storage = EventStorage::open_in_memory().unwrap();
        let days: HashSet<Weekday> = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ]
        .into_iter()
        .collect();
        let event = ParsedEvent {
            date: Some(NaiveDate::from_ymd_opt(2027, 12, 31).unwrap()),
            time: Some(NaiveTime::from_hms_opt(13, 30, 0).unwrap()),
            year_explicit: true,
            days: Some(days),
            period: None,
            repetition: None,
            message: "weekday meeting".to_string(),
        };
        let target = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(13, 30, 0).unwrap(),
        );

        let id = storage.insert(999, &event, target).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.days, Some("mon,tue,wed,thu,fri".to_string()));
        assert_eq!(stored.message, "weekday meeting");
    }

    #[test]
    fn test_days_none_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        let event = create_test_event("no days");
        let target = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        );

        let id = storage.insert(999, &event, target).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.days, None);
    }
}
