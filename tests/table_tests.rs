use chrono::NaiveDateTime;
use perbot::{mapper, parser, scheduler, storage};

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
/// StoredEvent" that is updated by USER rows (parse + map) and SYSTEM rows
/// (calc_next_at, then assert next_datetime or active==false for NONE).
/// Collects all failures before panicking so every failing row is shown.
fn run_table(table_idx: usize, table: &Table) {
    let mut current_event: Option<storage::StoredEvent> = None;
    // (step, detail_message, actual_value_for_arrow)
    let mut failures: Vec<(usize, String, String)> = Vec::new();

    for (step, row) in table.rows.iter().enumerate() {
        match row.actor.as_str() {
            "USER" => match parser::parse(&row.value) {
                Some(parsed) => current_event = Some(mapper::map(parsed, 0, 0)),
                None => {
                    failures.push((
                        step,
                        format!("failed to parse '{}'", row.value),
                        "None".to_string(),
                    ));
                    current_event = None;
                }
            },
            "SYSTEM" => {
                let event = match current_event.take() {
                    Some(e) => e,
                    None => {
                        failures.push((
                            step,
                            "SYSTEM row but no current event".to_string(),
                            String::new(),
                        ));
                        continue;
                    }
                };
                let result = scheduler::calc_next_at(event, row.ts);
                if row.value == "NONE" {
                    if result.active {
                        failures.push((
                            step,
                            format!(
                                "expected active=false, got active=true (next_datetime={:?})",
                                result.next_datetime
                            ),
                            fmt_dt(result.next_datetime),
                        ));
                    } else if result.next_datetime.is_some() {
                        failures.push((
                            step,
                            format!(
                                "expected next_datetime=None, got {:?}",
                                result.next_datetime
                            ),
                            fmt_dt(result.next_datetime),
                        ));
                    }
                } else {
                    match NaiveDateTime::parse_from_str(&row.value, "%Y-%m-%d %H:%M:%S") {
                        Ok(expected) => {
                            if result.next_datetime != Some(expected) {
                                failures.push((
                                    step,
                                    format!(
                                        "expected {:?}, got {:?}",
                                        Some(expected),
                                        result.next_datetime
                                    ),
                                    fmt_dt(result.next_datetime),
                                ));
                            }
                        }
                        Err(_) => {
                            failures.push((
                                step,
                                format!("invalid expected datetime '{}'", row.value),
                                String::new(),
                            ));
                        }
                    }
                }
                current_event = Some(result);
            }
            other => {
                failures.push((step, format!("unknown actor '{}'", other), String::new()));
            }
        }
    }

    if !failures.is_empty() {
        let actuals: Vec<(usize, String)> =
            failures.iter().map(|(s, _, v)| (*s, v.clone())).collect();
        let table_display = format_table_with_arrows(&table.rows, &actuals);
        let failure_msgs: Vec<String> = failures
            .iter()
            .map(|(s, msg, _)| format!("  step {}: {}", s, msg))
            .collect();
        panic!(
            "\nTable {} — {} FAILED:\n{}\n\nDetails:\n{}",
            table_idx,
            table.name,
            table_display,
            failure_msgs.join("\n"),
        );
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
