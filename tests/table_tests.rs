use chrono::NaiveDateTime;
use perbot::parser;
use perbot::state::EventProvider;
use perbot::storage::EventStorage;
use perbot::types::{ChatInfo, ChatType};

struct TableRow {
    ts: NaiveDateTime,
    actor: String,
    value: String,
    original: String,
}

struct Table {
    name: String,
    rows: Vec<TableRow>,
}

/// Parse all markdown tables from the given content.
///
/// Each consecutive block of `|`-prefixed lines is treated as one table.
/// The header row (where column[1] is not "USER" or "SYSTEM") and the
/// separator row (e.g. `|---|---|---|`) are silently skipped.
/// `### Heading` lines are captured as the table name.
///
/// Returns a list of tables; each table is a list of rows.
fn parse_tables(content: &str) -> Vec<Table> {
    let mut tables: Vec<Table> = Vec::new();
    let mut current_rows: Vec<TableRow> = Vec::new();
    let mut current_name = String::new();
    let mut in_table = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('|') {
            in_table = true;

            let cols: Vec<&str> = trimmed
                .split('|')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();

            if cols.len() < 3 {
                continue;
            }

            let actor = cols[1].to_uppercase();
            if actor != "USER" && actor != "SYSTEM" {
                // Header or separator row — skip
                continue;
            }

            let ts = match NaiveDateTime::parse_from_str(cols[0], "%Y-%m-%d %H:%M:%S") {
                Ok(dt) => dt,
                Err(_) => continue,
            };

            current_rows.push(TableRow {
                ts,
                actor,
                value: cols[2].to_string(),
                original: trimmed.to_string(),
            });
        } else {
            if in_table && !current_rows.is_empty() {
                tables.push(Table {
                    name: current_name.clone(),
                    rows: std::mem::take(&mut current_rows),
                });
                in_table = false;
            }
            if trimmed.starts_with("### ") {
                current_name = trimmed.trim_start_matches('#').trim().to_string();
            }
        }
    }

    if !current_rows.is_empty() {
        tables.push(Table {
            name: current_name,
            rows: current_rows,
        });
    }

    tables
}

fn format_table_with_arrows(rows: &[TableRow], actuals: &[(usize, String)]) -> String {
    let actual_map: std::collections::HashMap<usize, &str> =
        actuals.iter().map(|(s, v)| (*s, v.as_str())).collect();
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            if let Some(actual) = actual_map.get(&i) {
                format!("  {} <-- {} (actual)", row.original, actual)
            } else {
                format!("  {}", row.original)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn fmt_dt(dt: Option<chrono::NaiveDateTime>) -> String {
    match dt {
        Some(d) => d.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => "None".to_string(),
    }
}

/// Execute a single table: walk the rows in order, maintaining a "current
/// EventInfo" that is updated by USER rows (parse) and SYSTEM rows
/// (calc_next_at, then assert next_datetime or active==false for NONE).
/// Panics on the first failure with a table display showing the error.
/// Each table gets its own in-memory EventStorage so storage round-trips
/// are exercised for every scenario.
fn run_table(table_idx: usize, table: &Table) {
    const CHAT_ID: i64 = 1;
    let storage = EventStorage::open_in_memory().unwrap();
    let provider = EventProvider::new(storage);
    provider
        .upsert_chat(&ChatInfo {
            id: CHAT_ID,
            chat_type: ChatType::Private,
            title: None,
            username: None,
            first_name: None,
            last_name: None,
            updated_at: None,
            created_at: None,
        })
        .unwrap();

    let mut current_id: Option<i64> = None;

    for (step, row) in table.rows.iter().enumerate() {
        match row.actor.as_str() {
            "USER" => {
                current_id = parser::parse(&row.value).map(|mut event| {
                    let msg_id = provider.insert_message(None, CHAT_ID, &row.value).unwrap();
                    event.chat_id = CHAT_ID;
                    event.msg_id = msg_id;
                    let event = provider.insert_event_and_get_at(event, row.ts);
                    event.id
                });
            }
            "SYSTEM" => {
                let id = match current_id {
                    Some(id) => id,
                    None => {
                        if row.value == "NONE" {
                            continue;
                        }
                        fail_at(
                            table_idx,
                            table,
                            step,
                            &format!("no current event (parse failed), expected {}", row.value),
                            &String::new(),
                        );
                    }
                };
                let event = match provider.get_event(id) {
                    Some(event) => event,
                    None => {
                        fail_at(
                            table_idx,
                            table,
                            step,
                            &format!("event {} not found in storage", id),
                            &String::new(),
                        );
                    }
                };

                // Only call update (simulate fire) when time has reached the event's next_datetime
                if let Some(next_dt) = event.next_datetime {
                    if row.ts >= next_dt {
                        provider.update_at_and_reload(vec![event.clone()], row.ts);
                    }
                }

                // Re-read the event after potential update
                let result = provider.get_next_event();

                if row.value == "NONE" {
                    match &result {
                        Some(e) if e.active => {
                            fail_at(
                                table_idx,
                                table,
                                step,
                                &format!(
                                    "expected inactive/none, got active=true (next_datetime={:?})",
                                    e.next_datetime
                                ),
                                &fmt_dt(e.next_datetime),
                            );
                        }
                        _ => {} // None or inactive — pass
                    }
                } else {
                    let actual_dt = result.as_ref().and_then(|e| e.next_datetime);
                    match NaiveDateTime::parse_from_str(&row.value, "%Y-%m-%d %H:%M:%S") {
                        Ok(expected) => {
                            if actual_dt != Some(expected) {
                                fail_at(
                                    table_idx,
                                    table,
                                    step,
                                    &format!("expected {:?}, got {:?}", Some(expected), actual_dt),
                                    &fmt_dt(actual_dt),
                                );
                            }
                        }
                        Err(_) => {
                            fail_at(
                                table_idx,
                                table,
                                step,
                                &format!("invalid expected datetime '{}'", row.value),
                                &String::new(),
                            );
                        }
                    }
                }
            }
            other => {
                fail_at(
                    table_idx,
                    table,
                    step,
                    &format!("unknown actor '{}'", other),
                    &String::new(),
                );
            }
        }
    }
}

fn fail_at(table_idx: usize, table: &Table, step: usize, msg: &str, actual: &str) -> ! {
    let actuals = vec![(step, actual.to_string())];
    let table_display = format_table_with_arrows(&table.rows, &actuals);
    panic!(
        "\nTable {} — {} FAILED:\n{}\n\nDetails:\n  step {}: {}",
        table_idx, table.name, table_display, step, msg,
    );
}

#[test]
fn test_table_driven() {
    let content = include_str!("../test-cases.md");
    let tables = parse_tables(content);
    assert!(
        !tables.is_empty(),
        "No tables found in test-cases.md — check the format"
    );
    for (i, table) in tables.iter().enumerate() {
        run_table(i + 1, table);
    }
}
