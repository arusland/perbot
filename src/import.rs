//! Admin `/import` flow: turns a zip of legacy MateBot `.alert` files into
//! events in the live database and produces an HTML old→new report.
//!
//! The actual old→new mapping lives in [`crate::converter`]; this module handles
//! the zip, the per-event chat/message rows, and the report.

use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};

use chrono::{Local, NaiveDateTime};

use crate::converter::{self, Status};
use crate::state::EventProvider;
use crate::types::{ChatInfo, ChatType};

/// Pending import target (chat id) recorded by `/import <user_id>` until the
/// admin sends the zip. A single admin means a single slot is enough.
pub type PendingImport = Arc<Mutex<Option<i64>>>;

pub fn new_pending() -> PendingImport {
    Arc::new(Mutex::new(None))
}

/// Result of importing one zip.
pub struct ImportOutcome {
    pub total: usize,
    pub scheduled: usize,
    pub inactive: usize,
    pub unparsed: usize,
    pub recalculated: usize,
    pub html: String,
}

impl ImportOutcome {
    /// One-line human summary for the admin reply.
    pub fn summary(&self) -> String {
        format!(
            "Imported {} event(s): {} scheduled, {} inactive, {} unparsed, {} recalculated.",
            self.total, self.scheduled, self.inactive, self.unparsed, self.recalculated
        )
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn fmt_dt(dt: Option<NaiveDateTime>) -> String {
    dt.map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".to_string())
}

/// Imports every `.alert` entry of `zip_bytes` for `target_user_id`, writing the
/// converted events into the database behind `provider`. Returns counts plus the
/// HTML report.
pub fn import_zip(
    provider: &EventProvider,
    target_user_id: i64,
    zip_bytes: &[u8],
) -> anyhow::Result<ImportOutcome> {
    let now = Local::now().naive_local();

    // Ensure the destination chat exists (FK for events.chat_id).
    provider.upsert_chat(&ChatInfo {
        id: target_user_id,
        chat_type: ChatType::Private,
        title: None,
        username: None,
        first_name: None,
        last_name: None,
        updated_at: None,
        created_at: None,
    })?;

    let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes))?;
    log::info!(
        "Importing legacy alerts for chat {target_user_id} ({} entries)",
        archive.len()
    );

    let mut rows = String::new();
    let mut outcome = ImportOutcome {
        total: 0,
        scheduled: 0,
        inactive: 0,
        unparsed: 0,
        recalculated: 0,
        html: String::new(),
    };

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let base = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_string();
        if !base.ends_with(".alert") {
            continue;
        }

        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        let contents = String::from_utf8_lossy(&buf);

        let (input, last_active) = converter::extract_input(&contents);
        let created_at = converter::created_at_from_filename(&base).unwrap_or(now);
        let converted = converter::convert(&input, created_at, last_active, target_user_id, now);

        // Per-event synthetic message satisfies the events.msg_id FK. The synthetic
        // message's user_id matches the target chat id.
        let msg_id = provider.insert_message(
            Some(target_user_id),
            target_user_id,
            &format!("Import legacy events from {base}"),
        )?;
        let mut event = converted.event;
        event.chat_id = target_user_id;
        event.msg_id = msg_id;
        provider.insert_prebuilt_event(&event)?;

        outcome.total += 1;
        match converted.status {
            Status::Scheduled => outcome.scheduled += 1,
            Status::Inactive => outcome.inactive += 1,
            Status::Unparsed => outcome.unparsed += 1,
        }
        if converted.recalculated {
            outcome.recalculated += 1;
        }

        // One log line per row, mirroring the HTML report.
        log::info!(
            "[{idx}] {file} | {status}{recalc} | next={next} active={active} | in: {input} | {summary}",
            idx = outcome.total,
            file = base,
            status = converted.status.label(),
            recalc = if converted.recalculated {
                " (recalculated)"
            } else {
                ""
            },
            next = fmt_dt(event.next_datetime),
            active = event.active,
            input = input,
            summary = converted.summary,
        );

        let status_cell = if converted.recalculated {
            format!(
                "{}<br><span style=\"color:#c00;font-weight:bold\">recalculated from stale lastActivePeriodTime</span>",
                escape_html(converted.status.label())
            )
        } else {
            escape_html(converted.status.label())
        };
        let row_style = if converted.recalculated {
            " style=\"background:#fff3f3\""
        } else if !event.active {
            " style=\"background:#f6f6f6;color:#777\""
        } else {
            ""
        };

        rows.push_str(&format!(
            "<tr{row_style}><td>{idx}</td><td>{file}</td><td>{created}</td><td>{input}</td><td>{summary}</td><td>{next}</td><td>{active}</td><td>{status}</td></tr>\n",
            idx = outcome.total,
            file = escape_html(&base),
            created = fmt_dt(Some(created_at)),
            input = escape_html(&input),
            summary = escape_html(&converted.summary),
            next = fmt_dt(event.next_datetime),
            active = if event.active { "yes" } else { "no" },
            status = status_cell,
        ));
    }

    log::info!("{}", outcome.summary());
    outcome.html = build_report_html(target_user_id, now, &outcome, &rows);
    Ok(outcome)
}

fn build_report_html(
    target_user_id: i64,
    now: NaiveDateTime,
    outcome: &ImportOutcome,
    rows: &str,
) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Legacy import report</title>\
<style>body{{font-family:sans-serif;margin:24px}}\
table{{border-collapse:collapse;width:100%}}\
th,td{{border:1px solid #ccc;padding:6px 8px;text-align:left;vertical-align:top;font-size:14px}}\
th{{background:#222;color:#fff}}\
caption{{text-align:left;margin-bottom:8px;font-size:13px;color:#555}}</style></head><body>\
<h1>Legacy import report</h1>\
<p>Chat <b>{chat}</b> · generated {now} · {total} event(s): \
{scheduled} scheduled, {inactive} inactive, {unparsed} unparsed, \
<span style=\"color:#c00\">{recalc} recalculated</span>.</p>\
<table><caption>Old data → new event</caption><thead><tr>\
<th>#</th><th>File</th><th>Created at</th><th>Old input</th><th>Parsed</th>\
<th>Next datetime</th><th>Active</th><th>Status</th></tr></thead><tbody>\n{rows}</tbody></table>\
</body></html>",
        chat = target_user_id,
        now = now.format("%Y-%m-%d %H:%M"),
        total = outcome.total,
        scheduled = outcome.scheduled,
        inactive = outcome.inactive,
        unparsed = outcome.unparsed,
        recalc = outcome.recalculated,
        rows = rows,
    )
}
