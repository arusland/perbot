//! The admin `/import <user_id> <timezone>` command and the conversion of the
//! zip of legacy MateBot `.alert` files the admin sends next into events in the
//! live database (plus an HTML old→new report).
//!
//! [`handle_import`] records the pending target and timezone; [`handle_import_zip`]
//! processes the zip; [`import_zip`] does the per-event chat/message rows and the
//! report. The actual old→new mapping lives in [`crate::converter`].

use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};

use chrono::{NaiveDateTime, Utc};
use chrono_tz::Tz;
use teloxide::types::{ChatId, FileId};

use super::CmdContext;
use crate::converter::{self, Status};
use crate::locale;
use crate::state::EventProvider;
use crate::tgbot::TgBot;
use crate::types::{ChatInfo, ChatType, fmt_dt};

/// Pending import target (chat id + the timezone the legacy wall-clock data is
/// read in) recorded by `/import <user_id> <timezone>` until the admin sends
/// the zip. A single admin means a single slot is enough.
pub type PendingImport = Arc<Mutex<Option<(i64, Tz)>>>;

pub fn new_pending() -> PendingImport {
    Arc::new(Mutex::new(None))
}

const IMPORT_USAGE: &str = "Usage: /import <user_id> <timezone>, e.g. /import 12345 Europe/Berlin";

/// Parses the `/import` arguments: exactly a chat id and an IANA timezone.
fn parse_import_args(args: &str) -> Option<(i64, Tz)> {
    let mut words = args.split_whitespace();
    let Some(id_word) = words.next() else {
        log::warn!("/import: missing arguments");
        return None;
    };
    let Ok(user_id) = id_word.parse() else {
        log::warn!("/import: invalid chat id {id_word:?} (args: {args:?})");
        return None;
    };
    let Some(tz_word) = words.next() else {
        log::warn!("/import: missing timezone (args: {args:?})");
        return None;
    };
    let Some(tz) = crate::tz::parse_tz(tz_word) else {
        log::warn!("/import: unknown timezone {tz_word:?} (args: {args:?})");
        return None;
    };
    if let Some(extra) = words.next() {
        log::warn!("/import: unexpected extra argument {extra:?} (args: {args:?})");
        return None;
    }
    Some((user_id, tz))
}

/// Begins a legacy import. Admin-only; records the pending target chat and
/// timezone and asks the admin to send the zip of `.alert` files next.
pub(super) async fn handle_import(ctx: &CmdContext<'_>, args: &str) -> anyhow::Result<()> {
    if !ctx.is_admin {
        ctx.bot
            .send_text(ctx.chat_id, "Not authorized.", None)
            .await?;
        return Ok(());
    }
    let Some((user_id, tz)) = parse_import_args(args) else {
        ctx.bot.send_text(ctx.chat_id, IMPORT_USAGE, None).await?;
        return Ok(());
    };
    *ctx.pending_import.lock().unwrap() = Some((user_id, tz));
    ctx.bot
        .send_text(
            ctx.chat_id,
            format!("Send the .zip of legacy alerts now to import them for chat {user_id} ({tz})."),
            None,
        )
        .await?;
    Ok(())
}

/// Downloads the admin's zip, imports the legacy alerts for `target` reading
/// their wall-clock data in `tz`, and replies with a summary plus the HTML
/// report as a document. Driven from `main` when the admin sends the zip after
/// `/import <user_id> <timezone>`.
pub async fn handle_import_zip(
    bot: &TgBot,
    provider: &EventProvider,
    chat_id: ChatId,
    target: i64,
    tz: Tz,
    file_id: FileId,
) -> anyhow::Result<()> {
    let file = bot.get_file(file_id).await?;
    let mut buf: Vec<u8> = Vec::new();
    if let Err(e) = bot.download_file(&file.path, &mut buf).await {
        bot.send_text(chat_id, format!("Failed to download the zip: {e}"), None)
            .await?;
        return Ok(());
    }

    bot.send_text(chat_id, "Importing events from file...", None)
        .await?;

    match import_zip(provider, target, &buf, tz) {
        Ok(outcome) => {
            let report_path = std::env::temp_dir().join("perbot-legacy-report.html");
            bot.send_text(chat_id, outcome.summary(), None).await?;
            match std::fs::write(&report_path, &outcome.html) {
                Ok(()) => {
                    bot.send_document(chat_id, &report_path, None).await?;
                }
                Err(e) => {
                    bot.send_text(chat_id, format!("Failed to write report: {e}"), None)
                        .await?;
                }
            }
        }
        Err(e) => {
            bot.send_text(chat_id, format!("Import failed: {e}"), None)
                .await?;
        }
    }
    Ok(())
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

/// Imports every `.alert` entry of `zip_bytes` for `target_user_id`, writing the
/// converted events into the database behind `provider`. Legacy wall-clock data
/// is interpreted in `tz` (the zone the admin passed to `/import`); a chat with
/// no timezone setting yet also adopts `tz` as its setting. Returns counts plus
/// the HTML report.
pub fn import_zip(
    provider: &EventProvider,
    target_user_id: i64,
    zip_bytes: &[u8],
    tz: Tz,
) -> anyhow::Result<ImportOutcome> {
    let now = Utc::now().naive_utc();
    let loc = locale::for_chat(target_user_id);

    // Ensure the destination chat exists (FK for events.chat_id).
    provider.upsert_chat(&ChatInfo {
        id: target_user_id,
        chat_type: ChatType::Private,
        title: None,
        username: None,
        first_name: None,
        last_name: None,
        banned: false,
        updated_at: None,
        created_at: None,
    })?;

    // A chat without a timezone adopts the import's zone (before converting, so
    // the stored setting and the conversion frame agree); an existing setting is
    // never overwritten — the passed zone still drives this conversion.
    if provider.get_timezone(target_user_id)?.is_none() {
        provider.set_timezone(target_user_id, tz)?;
    }

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
        // The filename stamp is a wall-clock reading in the chat's timezone.
        let created_at = converter::created_at_from_filename(&base)
            .map(|local| crate::tz::to_utc(local, tz))
            .unwrap_or(now);
        let converted =
            converter::convert(&input, created_at, last_active, target_user_id, now, tz);

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
            "<tr{row_style}><td>{idx}</td><td>{file}</td><td>{created}</td><td>{input}</td><td>{new_input}</td><td>{next}</td><td>{active}</td><td>{status}</td></tr>\n",
            idx = outcome.total,
            file = escape_html(&base),
            created = fmt_dt(Some(crate::tz::to_local(created_at, tz))),
            input = escape_html(&input),
            new_input = escape_html(&event.normalize_time(loc)),
            next = fmt_dt(event.next_datetime.map(|dt| crate::tz::to_local(dt, tz))),
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
<th>#</th><th>File</th><th>Created at</th><th>Old input</th><th>New input</th>\
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_import_args_accepts_id_and_timezone() {
        assert_eq!(
            parse_import_args("123 Europe/Berlin"),
            Some((123, Tz::Europe__Berlin))
        );
        assert_eq!(parse_import_args("  -42   UTC  "), Some((-42, Tz::UTC)));
    }

    #[test]
    fn parse_import_args_rejects_bad_input() {
        assert_eq!(parse_import_args(""), None);
        assert_eq!(parse_import_args("123"), None);
        assert_eq!(parse_import_args("123 Nowhere/Nothing"), None);
        assert_eq!(parse_import_args("abc Europe/Berlin"), None);
        assert_eq!(parse_import_args("123 Europe/Berlin extra"), None);
    }
}
