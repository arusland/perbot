use chrono::NaiveDateTime;
use perbot::parser;
use perbot::state::EventProvider;
use perbot::storage::EventStorage;
use perbot::types::{ChatInfo, ChatType};

#[derive(Clone)]
struct TableRow {
    ts: NaiveDateTime,
    actor: String,
    value: String,
    /// Expected reminder message for USER rows (4th column); `None` for SYSTEM
    /// rows (whose 4th cell is empty and dropped by the empty-cell filter).
    message: Option<String>,
    /// Expected `EventInfo::normalize_time()` for USER rows (5th column); `None`
    /// for SYSTEM rows and for USER rows whose input fails to parse (empty cell).
    normalized: Option<String>,
    original: String,
}

#[derive(Clone)]
struct Table {
    name: String,
    rows: Vec<TableRow>,
}

/// Expands a single cell containing optional `(...)` groups into every concrete
/// variant. `(in) 8 min call her` yields `["in 8 min call her", "8 min call her"]`:
/// the "include" branch keeps the inner text (parens dropped), the "exclude"
/// branch removes the whole group and collapses the leftover whitespace. Multiple
/// groups expand via the cartesian product (recursion on the remainder); a cell
/// with no `(...)` yields a single, unchanged variant.
fn expand_optionals(value: &str) -> Vec<String> {
    let Some(open) = value.find('(') else {
        return vec![value.to_string()];
    };
    let Some(rel_close) = value[open..].find(')') else {
        return vec![value.to_string()];
    };
    let close = open + rel_close;

    let before = &value[..open];
    let inner = &value[open + 1..close];
    let after = &value[close + 1..];

    let include = format!("{before}{inner}{after}");
    // Drop the group entirely, then squeeze the seam (e.g. "X  Y" -> "X Y") and trim.
    let exclude = format!("{before}{after}")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let mut out = Vec::new();
    for variant in [include, exclude] {
        out.extend(expand_optionals(&variant));
    }
    out
}

/// Expands a table whose rows may contain optional `(...)` groups into one table
/// per combination of choices. Each variant clones the rows with the concrete
/// value substituted and gets a disambiguating name suffix so a failure panic
/// identifies which expansion broke. A table with no optional groups is returned
/// unchanged.
fn expand_table(table: &Table) -> Vec<Table> {
    // Per-row variants; rows without `(...)` contribute exactly one.
    let per_row: Vec<Vec<String>> = table
        .rows
        .iter()
        .map(|row| expand_optionals(&row.value))
        .collect();

    let has_optional = per_row.iter().any(|v| v.len() > 1);
    if !has_optional {
        return vec![table.clone()];
    }

    // Cartesian product across rows.
    let mut combos: Vec<Vec<String>> = vec![Vec::new()];
    for variants in &per_row {
        let mut next = Vec::new();
        for combo in &combos {
            for variant in variants {
                let mut extended = combo.clone();
                extended.push(variant.clone());
                next.push(extended);
            }
        }
        combos = next;
    }

    combos
        .into_iter()
        .map(|combo| {
            let rows: Vec<TableRow> = table
                .rows
                .iter()
                .zip(combo.iter())
                .map(|(row, value)| {
                    let mut row = row.clone();
                    row.original = row.original.replacen(&row.value, value, 1);
                    row.value = value.clone();
                    row
                })
                .collect();
            // Label the variant by the USER input(s) that actually differ.
            let label = rows
                .iter()
                .zip(per_row.iter())
                .filter(|(_, variants)| variants.len() > 1)
                .map(|(row, _)| row.value.clone())
                .collect::<Vec<_>>()
                .join(", ");
            Table {
                name: format!("{} (variant: {:?})", table.name, label),
                rows,
            }
        })
        .collect()
}

/// Parse all markdown tables from the given content.
///
/// Each consecutive block of `|`-prefixed lines is treated as one table.
/// The header row (where column[1] is not "USER" or "SYSTEM") and the
/// separator row (e.g. `|---|---|---|`) are silently skipped.
/// `### Heading` lines are captured as the table name.
///
/// USER rows carry an optional 4th column with the expected reminder message
/// (must equal `EventInfo.message`) and an optional 5th column with the expected
/// canonical time expression (must equal `EventInfo.normalize_time()`); SYSTEM
/// rows leave both empty, so the empty-cell filter drops them and they yield only
/// 3 columns.
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
                value: unescape(cols[2]),
                message: cols.get(3).map(|s| unescape(s)),
                normalized: cols.get(4).map(|s| unescape(s)),
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

/// Decodes the literal two-char `\n` escape into a real newline, so a multiline
/// message can be expressed inside a single markdown table cell (which cannot
/// contain a real line break).
fn unescape(s: &str) -> String {
    s.replace("\\n", "\n")
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
                    if let Some(expected) = &row.message
                        && &event.message != expected
                    {
                        fail_at(
                            table_idx,
                            table,
                            step,
                            &format!("expected message {:?}, got {:?}", expected, event.message),
                            &event.message,
                        );
                    }
                    if let Some(expected) = &row.normalized {
                        let actual = event.normalize_time();
                        if &actual != expected {
                            fail_at(
                                table_idx,
                                table,
                                step,
                                &format!("expected normalized {:?}, got {:?}", expected, actual),
                                &actual,
                            );
                        }
                    }
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
                            "",
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
                            "",
                        );
                    }
                };

                // Only call update (simulate fire) when time has reached the event's next_datetime
                if let Some(next_dt) = event.next_datetime
                    && row.ts >= next_dt
                {
                    provider.update_at_and_reload(vec![event.clone()], row.ts);
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
                                "",
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
                    "",
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
    let tables: Vec<Table> = parse_tables(content)
        .iter()
        .flat_map(expand_table)
        .collect();
    assert!(
        !tables.is_empty(),
        "No tables found in test-cases.md — check the format"
    );
    for (i, table) in tables.iter().enumerate() {
        run_table(i + 1, table);
    }
}
