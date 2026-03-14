use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use std::path::PathBuf;
use std::time::Instant;

use perbot::storage::EventStorage;
use perbot::types::{ChatInfo, ChatType, EventInfo, MessageInfo};

fn make_event(i: u32, msg_id: i64) -> EventInfo {
    EventInfo {
        id: 0,
        chat_id: 1,
        date: Some(
            NaiveDate::from_ymd_opt(2027, (i % 12 + 1) as u32, (i % 28 + 1) as u32).unwrap(),
        ),
        time: Some(NaiveTime::from_hms_opt(i % 24, i % 60, 0).unwrap()),
        year_explicit: false,
        days: None,
        years: None,
        message: format!("reminder #{i}"),
        active: true,
        next_datetime: Some(NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2027, (i % 12 + 1) as u32, (i % 28 + 1) as u32).unwrap(),
            NaiveTime::from_hms_opt(i % 24, i % 60, 0).unwrap(),
        )),
        created_at: NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        ),
        repetition: None,
        in_offset: None,
        bare_hour: None,
        monthly_pattern: None,
        msg_id,
    }
}

fn main() {
    let db_path = PathBuf::from("target").join("bench_storage.db");
    // Remove leftovers from previous run
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-journal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));

    let storage = EventStorage::open(&db_path).unwrap();

    // Create a chat and a message for foreign key constraints
    storage
        .upsert_chat(&ChatInfo {
            id: 1,
            chat_type: ChatType::Private,
            title: None,
            username: Some("bench_user".into()),
            first_name: Some("Bench".into()),
            last_name: None,
            updated_at: None,
            created_at: None,
        })
        .unwrap();

    let msg_id = storage
        .insert_message(&MessageInfo {
            id: 0,
            user_id: Some(1),
            chat_id: 1,
            created_at: None,
            message: "bench message".into(),
        })
        .unwrap();

    let count = 1000u32;

    // Benchmark inserts
    let start = Instant::now();
    for i in 0..count {
        let event = make_event(i, msg_id);
        storage.insert_event(&event).unwrap();
    }
    let insert_elapsed = start.elapsed();
    let insert_rps = count as f64 / insert_elapsed.as_secs_f64();

    // Benchmark reads (get by id)
    let start = Instant::now();
    for id in 1..=count as i64 {
        storage.get(id).unwrap();
    }
    let read_elapsed = start.elapsed();
    let read_rps = count as f64 / read_elapsed.as_secs_f64();

    // Benchmark get_active_events (full table scan of active)
    let start = Instant::now();
    let active = storage.get_active_events().unwrap();
    let active_elapsed = start.elapsed();

    // Benchmark get_next_event
    let now = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
        NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
    );
    let start = Instant::now();
    for _ in 0..count {
        storage.get_next_event(now).unwrap();
    }
    let next_elapsed = start.elapsed();
    let next_rps = count as f64 / next_elapsed.as_secs_f64();

    // File size
    let file_size = std::fs::metadata(&db_path).unwrap().len();

    let insert_ms_each = insert_elapsed.as_secs_f64() * 1000.0 / count as f64;
    let read_ms_each = read_elapsed.as_secs_f64() * 1000.0 / count as f64;
    let next_ms_each = next_elapsed.as_secs_f64() * 1000.0 / count as f64;

    println!("=== Storage Benchmark ({count} events) ===");
    println!(
        "Insert:           {:.0} rps  {:.2} ms avg  ({:.1} ms total)",
        insert_rps,
        insert_ms_each,
        insert_elapsed.as_secs_f64() * 1000.0
    );
    println!(
        "Get by ID:        {:.0} rps  {:.3} ms avg  ({:.1} ms total)",
        read_rps,
        read_ms_each,
        read_elapsed.as_secs_f64() * 1000.0
    );
    println!(
        "Get active ({} rows): {:.1} ms",
        active.len(),
        active_elapsed.as_secs_f64() * 1000.0
    );
    println!(
        "Get next event:   {:.0} rps  {:.3} ms avg  ({:.1} ms total)",
        next_rps,
        next_ms_each,
        next_elapsed.as_secs_f64() * 1000.0
    );
    println!(
        "DB file size:     {:.1} KB ({} bytes)",
        file_size as f64 / 1024.0,
        file_size
    );
    println!("==========================================");
}
