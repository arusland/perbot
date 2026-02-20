use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use rusqlite::{params, Connection, Result};
use std::path::Path;

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
    pub active: bool,
    pub next_datetime: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub repeat_interval: Option<u32>,
    pub repeat_unit: Option<String>,
    pub in_offset: Option<u32>,
    pub in_offset_unit: Option<String>,
    pub bare_hour: Option<u32>,
    pub monthly_pattern: Option<String>,
    pub msg_id: i64,
}

/// A stored user message.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: i64,
    pub user_id: Option<i64>,
    pub chat_id: i64,
    pub created_at: NaiveDateTime,
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

fn str_to_weekday(s: &str) -> Option<Weekday> {
    match s {
        "mon" => Some(Weekday::Mon),
        "tue" => Some(Weekday::Tue),
        "wed" => Some(Weekday::Wed),
        "thu" => Some(Weekday::Thu),
        "fri" => Some(Weekday::Fri),
        "sat" => Some(Weekday::Sat),
        "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

/// Parses a stored ordinal-weekday pattern like "first_sun" or "last_mon".
/// Returns `(n, weekday)` where `n = 0` means "last occurrence".
fn parse_ordinal_weekday(s: &str) -> Option<(u32, Weekday)> {
    let (ord_str, wd_str) = s.split_once('_')?;
    let n = match ord_str {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "last" => 0,
        _ => return None,
    };
    Some((n, str_to_weekday(wd_str)?))
}

fn nth_weekday_of_month(year: i32, month: u32, weekday: Weekday, n: u32) -> Option<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let offset = (weekday.num_days_from_monday() as i64
        - first.weekday().num_days_from_monday() as i64)
        .rem_euclid(7) as u32;
    let day = 1 + offset + (n - 1) * 7;
    let d = NaiveDate::from_ymd_opt(year, month, day)?;
    if d.month() == month {
        Some(d)
    } else {
        None
    }
}

fn last_weekday_of_month(year: i32, month: u32, weekday: Weekday) -> Option<NaiveDate> {
    let last = last_day_of_month_date(year, month)?;
    let offset = (last.weekday().num_days_from_monday() as i64
        - weekday.num_days_from_monday() as i64)
        .rem_euclid(7) as u32;
    NaiveDate::from_ymd_opt(year, month, last.day() - offset)
}

fn last_day_of_month_date(year: i32, month: u32) -> Option<NaiveDate> {
    if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?.pred_opt()
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?.pred_opt()
    }
}

fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

fn calculate_next_datetime(event: &StoredEvent) -> Option<NaiveDateTime> {
    let now = Local::now().naive_local();

    // Handle bare hour (e.g., bare_hour=8 -> next 08:00)
    if let Some(h) = event.bare_hour {
        let hour = if h == 24 { 0 } else { h };
        let t = NaiveTime::from_hms_opt(hour, 0, 0)?;
        let today = now.date();
        let dt = today.and_time(t);
        return if dt > now {
            Some(dt)
        } else {
            let tomorrow = today.succ_opt()?;
            Some(tomorrow.and_time(t))
        };
    }

    // Handle relative offset (e.g., in_offset=8, in_offset_unit="minutes")
    if let (Some(value), Some(unit_str)) = (event.in_offset, event.in_offset_unit.as_deref()) {
        return match unit_str {
            "minutes" => Some(now + chrono::Duration::minutes(value as i64)),
            "hours" => Some(now + chrono::Duration::hours(value as i64)),
            "days" => Some(now + chrono::Duration::days(value as i64)),
            "weeks" => Some(now + chrono::Duration::weeks(value as i64)),
            "months" => {
                let new_date = now.date().checked_add_months(chrono::Months::new(value))?;
                Some(new_date.and_time(now.time()))
            }
            "years" => {
                let new_date = now
                    .date()
                    .checked_add_months(chrono::Months::new(value * 12))?;
                Some(new_date.and_time(now.time()))
            }
            _ => None,
        };
    }

    // Handle monthly pattern (e.g., "first_sun", "last_mon", "last_day")
    if let Some(ref pattern_str) = event.monthly_pattern {
        let time = event
            .time
            .unwrap_or(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let today = now.date();
        let mut year = today.year();
        let mut month = today.month();

        for _ in 0..3 {
            let target_date = if pattern_str == "last_day" {
                last_day_of_month_date(year, month)
            } else if let Some((n, weekday)) = parse_ordinal_weekday(pattern_str) {
                if n == 0 {
                    last_weekday_of_month(year, month, weekday)
                } else {
                    nth_weekday_of_month(year, month, weekday, n)
                }
            } else {
                None
            };

            if let Some(d) = target_date {
                let dt = d.and_time(time);
                if dt > now {
                    return Some(dt);
                }
            }

            let (ny, nm) = next_month(year, month);
            year = ny;
            month = nm;
        }

        return None;
    }

    let dt = match (event.time, event.date) {
        (Some(t), Some(d)) => {
            let dt = d.and_time(t);
            if dt > now {
                Some(dt)
            } else if event.year_explicit {
                None
            } else {
                // Short date in the past — try next year
                let next = NaiveDate::from_ymd_opt(d.year() + 1, d.month(), d.day())?;
                Some(next.and_time(t))
            }
        }
        (Some(t), None) => {
            let today = now.date();
            let dt = today.and_time(t);
            if dt > now {
                Some(dt)
            } else {
                // Time already passed today — tomorrow
                let tomorrow = today.succ_opt()?;
                Some(tomorrow.and_time(t))
            }
        }
        (None, Some(d)) => {
            // Date only — use midnight
            let dt = d.and_hms_opt(0, 0, 0)?;
            if dt > now {
                Some(dt)
            } else if event.year_explicit {
                None
            } else {
                let next = NaiveDate::from_ymd_opt(d.year() + 1, d.month(), d.day())?;
                Some(next.and_hms_opt(0, 0, 0)?)
            }
        }
        (None, None) => None,
    }?;

    // If days-of-week restriction is set, advance to the next allowed day
    if let Some(ref days_str) = event.days {
        let allowed: Vec<Weekday> = days_str.split(',').filter_map(str_to_weekday).collect();
        let time = dt.time();
        let mut candidate = dt.date();
        for _ in 0..7 {
            if allowed.contains(&candidate.weekday()) && candidate.and_time(time) > now {
                return Some(candidate.and_time(time));
            }
            candidate = candidate.succ_opt()?;
        }
        None
    } else {
        Some(dt)
    }
}

/// Calculates the next occurrence datetime for a stored event and returns the
/// updated event. Sets `active = true` and `next_datetime = Some(dt)` when a
/// future datetime can be determined, otherwise `active = false` and
/// `next_datetime = None`.
pub fn play(event: StoredEvent) -> StoredEvent {
    let next_datetime = calculate_next_datetime(&event);
    StoredEvent {
        active: next_datetime.is_some(),
        next_datetime,
        ..event
    }
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
                updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
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
                msg_id          INTEGER NOT NULL REFERENCES messages(id)
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

    /// Inserts a new event into the database from a mapped `StoredEvent`.
    pub fn insert_event(&self, event: &StoredEvent) -> Result<i64> {
        let date_str = event.date.map(|d| d.format("%Y-%m-%d").to_string());
        let time_str = event.time.map(|t| t.format("%H:%M:%S").to_string());
        let next_str = event
            .next_datetime
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());

        self.conn.execute(
            "INSERT INTO events (chat_id, date, time, year_explicit, message, active, next_datetime, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                event.chat_id,
                date_str,
                time_str,
                event.year_explicit as i32,
                event.message,
                event.active as i32,
                next_str,
                event.days,
                event.repeat_interval,
                event.repeat_unit,
                event.in_offset,
                event.in_offset_unit,
                event.bare_hour,
                event.monthly_pattern,
                event.msg_id,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Retrieves an event by its ID.
    pub fn get(&self, id: i64) -> Result<Option<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id
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
            "SELECT id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id
             FROM events WHERE chat_id = ?1 ORDER BY next_datetime ASC",
        )?;

        let rows = stmt.query_map(params![chat_id], Self::row_to_event)?;

        rows.collect()
    }

    /// Retrieves all active (pending) events.
    pub fn get_pending(&self) -> Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id
             FROM events WHERE active = 1 ORDER BY next_datetime ASC",
        )?;

        let rows = stmt.query_map([], Self::row_to_event)?;

        rows.collect()
    }

    /// Retrieves active events for a specific chat.
    pub fn get_pending_by_chat(&self, chat_id: i64) -> Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id
             FROM events WHERE chat_id = ?1 AND active = 1 ORDER BY next_datetime ASC",
        )?;

        let rows = stmt.query_map(params![chat_id], Self::row_to_event)?;

        rows.collect()
    }

    /// Updates `active` and `next_datetime` for an event after `play` is called.
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

    /// Inserts a user message and returns its ID.
    pub fn insert_message(&self, user_id: Option<i64>, chat_id: i64, message: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO messages (user_id, chat_id, message) VALUES (?1, ?2, ?3)",
            params![user_id, chat_id, message],
        )?;
        Ok(self.conn.last_insert_rowid())
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
        let next_str: Option<String> = row.get(7)?;
        let created_str: String = row.get(8)?;
        let days: Option<String> = row.get(9)?;
        let repeat_interval: Option<u32> = row.get(10)?;
        let repeat_unit: Option<String> = row.get(11)?;

        let date = date_str.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok());
        let time = time_str.and_then(|s| NaiveTime::parse_from_str(&s, "%H:%M:%S").ok());
        let next_datetime =
            next_str.and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok());
        let created_at = NaiveDateTime::parse_from_str(&created_str, "%Y-%m-%d %H:%M:%S").unwrap();

        let in_offset: Option<u32> = row.get(12)?;
        let in_offset_unit: Option<String> = row.get(13)?;
        let bare_hour: Option<u32> = row.get(14)?;
        let monthly_pattern: Option<String> = row.get(15)?;
        let msg_id: i64 = row.get(16)?;

        Ok(StoredEvent {
            id: row.get(0)?,
            chat_id: row.get(1)?,
            date,
            time,
            year_explicit: row.get::<_, i32>(4)? != 0,
            days,
            message: row.get(5)?,
            active: row.get::<_, i32>(6)? != 0,
            next_datetime,
            created_at,
            repeat_interval,
            repeat_unit,
            in_offset,
            in_offset_unit,
            bare_hour,
            monthly_pattern,
            msg_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ensure_chat(storage: &EventStorage, chat_id: i64) {
        storage
            .upsert_chat(&ChatInfo {
                id: chat_id,
                chat_type: ChatType::Private,
                title: None,
                username: None,
                first_name: None,
                last_name: None,
            })
            .unwrap();
    }

    fn ensure_message(storage: &EventStorage, chat_id: i64) -> i64 {
        storage.insert_message(None, chat_id, "test").unwrap()
    }

    fn make_stored_event(message: &str) -> StoredEvent {
        StoredEvent {
            id: 0,
            chat_id: 0,
            date: Some(NaiveDate::from_ymd_opt(2027, 12, 31).unwrap()),
            time: Some(NaiveTime::from_hms_opt(23, 59, 0).unwrap()),
            year_explicit: true,
            days: None,
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
            repeat_interval: None,
            repeat_unit: None,
            in_offset: None,
            in_offset_unit: None,
            bare_hour: None,
            monthly_pattern: None,
            msg_id: 0,
        }
    }

    /// Minimal event for testing `play`, with no datetime info set.
    fn make_play_event() -> StoredEvent {
        StoredEvent {
            id: 0,
            chat_id: 0,
            date: None,
            time: None,
            year_explicit: false,
            days: None,
            message: String::new(),
            active: false,
            next_datetime: None,
            created_at: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            repeat_interval: None,
            repeat_unit: None,
            in_offset: None,
            in_offset_unit: None,
            bare_hour: None,
            monthly_pattern: None,
            msg_id: 0,
        }
    }

    #[test]
    fn test_insert_and_get() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 12345);
        let mut event = make_stored_event("test message");
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
        let mut event1 = make_stored_event("event 1");
        event1.chat_id = 111;
        event1.msg_id = msg_id_111;
        let mut event2 = make_stored_event("event 2");
        event2.chat_id = 111;
        event2.msg_id = msg_id_111;
        let mut event3 = make_stored_event("event 1");
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
        let mut event = make_stored_event("deactivate me");
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
    fn test_get_pending() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_stored_event("pending");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id1 = storage.insert_event(&event).unwrap();
        let id2 = storage.insert_event(&event).unwrap();

        storage.mark_inactive(id1).unwrap();

        let pending = storage.get_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id2);
    }

    #[test]
    fn test_delete() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_stored_event("delete me");
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
        let mut event = make_stored_event("test");
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
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 999);
        let mut event = make_stored_event("weekday meeting");
        event.chat_id = 999;
        event.msg_id = ensure_message(&storage, 999);
        event.days = Some("mon,tue,wed,thu,fri".to_string());
        event.time = Some(NaiveTime::from_hms_opt(13, 30, 0).unwrap());
        event.next_datetime = Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            NaiveTime::from_hms_opt(13, 30, 0).unwrap(),
        ));

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.days, Some("mon,tue,wed,thu,fri".to_string()));
        assert_eq!(stored.message, "weekday meeting");
    }

    #[test]
    fn test_days_none_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 999);
        let mut event = make_stored_event("no days");
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
        let mut event = make_stored_event("call office");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);
        event.date = Some(NaiveDate::from_ymd_opt(2027, 5, 20).unwrap());
        event.time = Some(NaiveTime::from_hms_opt(14, 55, 0).unwrap());
        event.year_explicit = false;
        event.repeat_interval = Some(2);
        event.repeat_unit = Some("weeks".to_string());
        event.next_datetime = Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 5, 20).unwrap(),
            NaiveTime::from_hms_opt(14, 55, 0).unwrap(),
        ));

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.repeat_interval, Some(2));
        assert_eq!(stored.repeat_unit, Some("weeks".to_string()));
        assert!(stored.active);
    }

    #[test]
    fn test_repetition_none_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_stored_event("no repeat");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.repeat_interval, None);
        assert_eq!(stored.repeat_unit, None);
        assert!(stored.active);
    }

    #[test]
    fn test_inactive_excluded_from_pending() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_stored_event("pending test");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id1 = storage.insert_event(&event).unwrap();
        let id2 = storage.insert_event(&event).unwrap();

        storage.mark_inactive(id1).unwrap();

        let pending = storage.get_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id2);
    }

    #[test]
    fn test_monthly_pattern_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_stored_event("call mom");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);
        event.date = None;
        event.time = Some(NaiveTime::from_hms_opt(10, 0, 0).unwrap());
        event.year_explicit = false;
        event.monthly_pattern = Some("first_sun".to_string());
        event.next_datetime = Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 3, 7).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        ));

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.monthly_pattern, Some("first_sun".to_string()));
        assert_eq!(stored.message, "call mom");
    }

    #[test]
    fn test_monthly_pattern_last_day_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_stored_event("pay rent");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);
        event.date = None;
        event.time = Some(NaiveTime::from_hms_opt(18, 0, 0).unwrap());
        event.year_explicit = false;
        event.monthly_pattern = Some("last_day".to_string());
        event.next_datetime = Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, 2, 28).unwrap(),
            NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        ));

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.monthly_pattern, Some("last_day".to_string()));
    }

    #[test]
    fn test_monthly_pattern_none_round_trip() {
        let storage = EventStorage::open_in_memory().unwrap();
        ensure_chat(&storage, 123);
        let mut event = make_stored_event("no pattern");
        event.chat_id = 123;
        event.msg_id = ensure_message(&storage, 123);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.monthly_pattern, None);
    }

    #[test]
    fn test_big_chat_id_exceeding_i32() {
        let storage = EventStorage::open_in_memory().unwrap();
        let big_chat_id: i64 = i32::MAX as i64 + 1; // 2_147_483_648
        ensure_chat(&storage, big_chat_id);

        let mut event = make_stored_event("big id test");
        event.chat_id = big_chat_id;
        event.msg_id = ensure_message(&storage, big_chat_id);

        let id = storage.insert_event(&event).unwrap();
        let stored = storage.get(id).unwrap().unwrap();

        assert_eq!(stored.chat_id, big_chat_id);
        assert_eq!(stored.message, "big id test");
    }

    // --- play() tests ---

    #[test]
    fn play_time_only_future_today() {
        let t = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.time = Some(t);
        let result = play(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert!(dt > now || dt.date() == now.date().succ_opt().unwrap());
    }

    #[test]
    fn play_explicit_year_past_returns_inactive() {
        let mut event = make_play_event();
        event.date = NaiveDate::from_ymd_opt(2020, 1, 1);
        event.time = NaiveTime::from_hms_opt(12, 0, 0);
        event.year_explicit = true;
        let result = play(event);
        assert!(!result.active);
        assert!(result.next_datetime.is_none());
    }

    #[test]
    fn play_short_date_past_wraps_to_next_year() {
        let past = Local::now().naive_local() - chrono::Duration::days(2);
        let mut event = make_play_event();
        event.date = Some(past.date());
        event.time = Some(past.time());
        event.year_explicit = false;
        let result = play(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert_eq!(dt.date().year(), past.date().year() + 1);
    }

    #[test]
    fn play_skips_disallowed_day() {
        let t = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
        let now = Local::now().naive_local();
        let today = now.date();
        let target_day = today.weekday().succ();
        let day_str = match target_day {
            Weekday::Mon => "mon",
            Weekday::Tue => "tue",
            Weekday::Wed => "wed",
            Weekday::Thu => "thu",
            Weekday::Fri => "fri",
            Weekday::Sat => "sat",
            Weekday::Sun => "sun",
        };

        let mut event = make_play_event();
        event.time = Some(t);
        event.days = Some(day_str.to_string());
        let result = play(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert_eq!(dt.date().weekday(), target_day);
        assert!(dt > now);
    }

    #[test]
    fn play_in_offset_minutes() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.in_offset = Some(10);
        event.in_offset_unit = Some("minutes".to_string());
        let result = play(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        let diff = dt.signed_duration_since(now).num_minutes();
        assert!(diff >= 9 && diff <= 11);
    }

    #[test]
    fn play_in_offset_hours() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.in_offset = Some(2);
        event.in_offset_unit = Some("hours".to_string());
        let result = play(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        let diff = dt.signed_duration_since(now).num_hours();
        assert!(diff >= 1 && diff <= 3);
    }

    #[test]
    fn play_monthly_first_sunday() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.time = Some(NaiveTime::from_hms_opt(10, 0, 0).unwrap());
        event.monthly_pattern = Some("first_sun".to_string());
        let result = play(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert!(dt > now);
        assert_eq!(dt.date().weekday(), Weekday::Sun);
        assert!(dt.date().day() <= 7);
    }

    #[test]
    fn play_monthly_last_monday() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        event.monthly_pattern = Some("last_mon".to_string());
        let result = play(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert!(dt > now);
        assert_eq!(dt.date().weekday(), Weekday::Mon);
        let next_week = dt.date() + chrono::Duration::days(7);
        assert_ne!(next_week.month(), dt.date().month());
    }

    #[test]
    fn play_monthly_last_day() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.time = Some(NaiveTime::from_hms_opt(18, 0, 0).unwrap());
        event.monthly_pattern = Some("last_day".to_string());
        let result = play(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert!(dt > now);
        let next_day = dt.date().succ_opt().unwrap();
        assert_ne!(next_day.month(), dt.date().month());
    }

    #[test]
    fn play_monthly_no_time_uses_midnight() {
        let now = Local::now().naive_local();
        let mut event = make_play_event();
        event.monthly_pattern = Some("second_wed".to_string());
        let result = play(event);
        assert!(result.active);
        let dt = result.next_datetime.unwrap();
        assert!(dt > now);
        assert_eq!(dt.time(), NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        assert_eq!(dt.date().weekday(), Weekday::Wed);
    }

    // --- Helper function unit tests ---

    #[test]
    fn test_nth_weekday_of_month() {
        // March 2026: first day is Sunday
        // First Monday = March 2
        assert_eq!(
            nth_weekday_of_month(2026, 3, Weekday::Mon, 1),
            NaiveDate::from_ymd_opt(2026, 3, 2)
        );
        // Second Monday = March 9
        assert_eq!(
            nth_weekday_of_month(2026, 3, Weekday::Mon, 2),
            NaiveDate::from_ymd_opt(2026, 3, 9)
        );
        // First Sunday = March 1
        assert_eq!(
            nth_weekday_of_month(2026, 3, Weekday::Sun, 1),
            NaiveDate::from_ymd_opt(2026, 3, 1)
        );
        // Fourth Friday = March 27
        assert_eq!(
            nth_weekday_of_month(2026, 3, Weekday::Fri, 4),
            NaiveDate::from_ymd_opt(2026, 3, 27)
        );
    }

    #[test]
    fn test_last_weekday_of_month() {
        // March 2026: last day is Tuesday March 31
        assert_eq!(
            last_weekday_of_month(2026, 3, Weekday::Tue),
            NaiveDate::from_ymd_opt(2026, 3, 31)
        );
        // Last Sunday in March 2026 = March 29
        assert_eq!(
            last_weekday_of_month(2026, 3, Weekday::Sun),
            NaiveDate::from_ymd_opt(2026, 3, 29)
        );
    }

    #[test]
    fn test_last_day_of_month_date() {
        assert_eq!(
            last_day_of_month_date(2026, 2),
            NaiveDate::from_ymd_opt(2026, 2, 28)
        );
        assert_eq!(
            last_day_of_month_date(2024, 2),
            NaiveDate::from_ymd_opt(2024, 2, 29)
        );
        assert_eq!(
            last_day_of_month_date(2026, 12),
            NaiveDate::from_ymd_opt(2026, 12, 31)
        );
    }
}
