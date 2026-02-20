use telegram_bot::{mapper, parser, storage};
use chrono::NaiveDateTime;

/// Parse all markdown tables from the given content.
///
/// Each consecutive block of `|`-prefixed lines is treated as one table.
/// The header row (where column[1] is not "USER" or "SYSTEM") and the
/// separator row (e.g. `|---|---|---|`) are silently skipped.
///
/// Returns a list of tables; each table is a list of `(timestamp, actor, value)`.
fn parse_tables(content: &str) -> Vec<Vec<(NaiveDateTime, String, String)>> {
    let mut tables: Vec<Vec<(NaiveDateTime, String, String)>> = Vec::new();
    let mut current: Vec<(NaiveDateTime, String, String)> = Vec::new();
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

            current.push((ts, actor, cols[2].to_string()));
        } else if in_table {
            if !current.is_empty() {
                tables.push(std::mem::take(&mut current));
            }
            in_table = false;
        }
    }

    if !current.is_empty() {
        tables.push(current);
    }

    tables
}

/// Execute a single table: walk the rows in order, maintaining a "current
/// StoredEvent" that is updated by USER rows (parse + map) and SYSTEM rows
/// (play_at, then assert next_datetime or active==false for NONE).
fn run_table(table_idx: usize, rows: &[(NaiveDateTime, String, String)]) {
    let mut current_event: Option<storage::StoredEvent> = None;

    for (step, (ts, actor, value)) in rows.iter().enumerate() {
        match actor.as_str() {
            "USER" => {
                let parsed = parser::parse(value).unwrap_or_else(|| {
                    panic!(
                        "Table {}, step {}: failed to parse '{}'",
                        table_idx, step, value
                    )
                });
                current_event = Some(mapper::map(parsed, 0, 0));
            }
            "SYSTEM" => {
                let event = current_event.take().unwrap_or_else(|| {
                    panic!(
                        "Table {}, step {}: SYSTEM row but no current event",
                        table_idx, step
                    )
                });
                let result = storage::play_at(event, *ts);
                if value == "NONE" {
                    assert!(
                        !result.active,
                        "Table {}, step {}: expected active=false, got active=true (next_datetime={:?})",
                        table_idx, step, result.next_datetime
                    );
                    assert!(
                        result.next_datetime.is_none(),
                        "Table {}, step {}: expected next_datetime=None, got {:?}",
                        table_idx, step, result.next_datetime
                    );
                } else {
                    let expected = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                        .unwrap_or_else(|_| {
                            panic!(
                                "Table {}, step {}: invalid expected datetime '{}'",
                                table_idx, step, value
                            )
                        });
                    assert_eq!(
                        result.next_datetime,
                        Some(expected),
                        "Table {}, step {}: wrong next_datetime",
                        table_idx,
                        step
                    );
                }
                current_event = Some(result);
            }
            other => panic!("Table {}, step {}: unknown actor '{}'", table_idx, step, other),
        }
    }
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
