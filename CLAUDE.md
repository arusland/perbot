# CLAUDE.md

## Workflow Rules

- When you complete work, print a short commit message for the changes.
- After a feature, module, command, or behavior change, update this file to keep it accurate.

## Project Overview

Perbot is a Telegram reminder bot in Rust (edition 2024). Users send a message starting with a natural-language time expression (e.g. `13:30 call the office`); the bot parses it, persists the event to SQLite, schedules it, and sends the reminder when due. Active events are reloaded and rescheduled on restart.

## Build & Test

```bash
cargo build [--release]
cargo test                 # all (parser/storage/scheduler/table tests)
cargo test <name>          # single test or module, e.g. parser::tests
cargo run --bin bench      # storage benchmark (1000 events)
```

## Environment Variables

- `TG_BOT_TOKEN` — bot API token (required)
- `TG_ADMIN_ID` — admin chat ID, i64 (required)
- `RUST_LOG` — `flexi_logger` level (e.g. `info`)
- `LOG_DIR` — log directory (default `logs`)

## Architecture

`lib.rs` re-exports all modules. Shared types live in `types.rs`; everything imports from there.

- **types.rs** — Shared types: `EventInfo` (rich parsed fields + DB-tracking fields, including `legacy: bool` for imported alerts), `MessageInfo`, `ChatInfo`/`ChatType`, `TgMessage`, `MessageSender`, plus the time enums (`TimeUnit`, `Repetition`, `Ordinal`, `MonthlyPattern`) and helpers `parse_days`, `unit_from_str`, `day_from_str`, `day_to_str`. `ChatType` implements `FromStr`. Weekday↔string mapping lives only here (`day_from_str`/`day_to_str`); other modules reuse it.

- **parser.rs** — `parse(text) -> Option<EventInfo>`. Stateless; regex-extracts the time/date components, rest becomes the message text. The clock time is matched *anywhere* in the message; the relative offset, bare hour, and short date are anchored to the start. A standalone 4-digit token in 2000..=2100 anywhere is treated as a year restriction. DB fields default to zero/false/None.

- **scheduler.rs** — Pure datetime math. `calc_next(EventInfo)` / `calc_next_at(EventInfo, now)` compute the next occurrence and set `active` + `next_datetime`.

- **error.rs** — Crate error type (`thiserror`) and `Result<T>` alias. Library methods (`storage`, `state`) return this instead of leaking `rusqlite::Error`; binaries (`main.rs`, `bin/bench.rs`) use `anyhow` on top.

- **storage.rs** — `EventStorage` over rusqlite; public methods return `crate::error::Result`. Tables: `chats`, `messages`, `events` (`events.msg_id` is a NOT NULL FK to `messages`; `events.legacy` flags imported alerts, with an idempotent `ALTER TABLE` migration for older DBs). `open(path)` / `open_in_memory()`. Events: `insert_event`, `get_event`, `get_by_chat`, `get_active_events`, `get_active_by_chat`, `get_active_by_chat_on_date(chat_id, date)`, `get_active_by_chat_in_range(chat_id, start, end)` (end exclusive), `get_next_event`, `get_missed_events(now)`, `get_events_at(dt)`, `update_schedule`, `mark_inactive`, `delete`, `delete_inactive`. Chats: `upsert_chat`, `get_chat`, `get_all_chats`. Messages: `insert_message`.

- **state.rs** — `EventProvider`: a `Clone` handle wrapping `Arc<Mutex<EventProviderState>>` (storage + cached nearest `next_event`); all methods take `&self` and lock internally. `start(msg_tx)` sends missed events then spawns a 1s polling thread that fires due events and reschedules. Other methods wrap storage: `upsert_chat`, `insert_message`, `get_next_event`, `get_missed_events`, `get_event`, `get_active_by_chat`, `get_active_by_chat_on_date`, `get_active_by_chat_in_range`, `insert_event_and_get[_at]`, `insert_prebuilt_event` (inserts an already-scheduled event without re-running the scheduler — used by the legacy importer), `update_at_and_reload`.

- **commands.rs** — Bot command handling. Defines the `Command` enum (`BotCommands`): `/help`, `/events`, `/today`, `/tomorrow`, `/week`, `/month`, and admin-only `/import <user_id>` and `/exit` (`#[command(hide)]`). `CmdContext<'a>` bundles the shared deps (`bot`, `chat_id`, `provider`, `admin_id`, `is_admin`, `pending_import`) every handler needs; `Command::handle(ctx)` dispatches to the private `handle_help`/`handle_events`/`handle_today`/`handle_tomorrow`/`handle_week`/`handle_month`/`handle_import`/`handle_exit`, each taking only `&CmdContext`. `/week` covers the current week Mon–Sun. The list commands send their reply via the private `send_list_reply(ctx, command, count, text)`, which on send failure (e.g. `MessageIsTooLong` when a list exceeds Telegram's 4096-char limit) logs the command/chat/event-count/length and warns the admin instead of letting the error bubble up to the REPL. `/import <user_id>` records the pending import target and prompts the admin to send the zip. The public `handle_import_zip(bot, provider, chat_id, target, file_id)` (called from `main` once the admin sends the zip) downloads it, runs `import::import_zip`, and replies with the summary + HTML report document. Replies use MarkdownV2 except `/help` and `/import`, which are plain text.

- **converter.rs** — Pure, unit-tested conversion of legacy MateBot `.alert` files (see `OLD-SPEC.md`) into `EventInfo`. `created_at_from_filename` parses `YYYYMMDD_HHMMSS_mmm.alert`; `extract_input` handles plain-text vs JSON `{"input", "lastActivePeriodTime"?}`; `convert(...)` maps the old grammar (`HH:MM [date|weekdays] [msg]` with `/N` periods) onto `EventInfo` (legacy=true). Day+month → yearly short date; `DD:MM:YYYY` → exact one-off; bare `DD:` → one-shot concrete date; ISO weekdays → `days`; `/N` → `repetition`. Scheduling: a future `lastActivePeriodTime` is used directly; a stale one is rolled forward via the input's period (flagged `recalculated`); otherwise `scheduler::calc_next_at` computes the next occurrence (recurring→active, one-off-past→inactive). Unparsable inputs are kept as inactive raw-text events.

- **import.rs** — Admin `/import` orchestration. `PendingImport = Arc<Mutex<Option<i64>>>` holds the target chat between `/import <user_id>` and the zip. `import_zip(provider, target, bytes)` opens the zip, runs `converter::convert` per `.alert` entry, upserts the target chat, inserts a synthetic `"Import legacy events from <file>"` message + the event via `insert_prebuilt_event`, and returns counts plus an HTML old→new report (recalculated rows are flagged in red).

- **main.rs** — Entry point + teloxide REPL + wiring (env vars, message channel, `provider.start`, a `PendingImport` slot). At startup it clears stale commands from non-default scopes (`AllPrivateChats`/`AllGroupChats`/`AllChatAdministrators`) then `set_my_commands(Command::bot_commands())`. Per message: upserts chat, computes `is_admin`; if the admin sends a document while an import is pending, it calls `commands::handle_import_zip`, which downloads the zip, calls `import::import_zip`, and replies with a summary + the HTML report document. Otherwise, per text message it stores the message and `Command::parse` → builds a `CmdContext` → `cmd.handle(ctx)`; non-command text goes through `parser::parse` → `provider.insert_event_and_get`. Non-command replies use MarkdownV2 (escaped via `telegram::escape_markdown`).

- **telegram.rs** — `escape_markdown`, `format_events_list(events)` / `format_events_list_at(events, now)`, `format_today_list(events)` / `format_today_list_at(events, now)`, `format_tomorrow_list(events)` / `format_tomorrow_list_at(events, now)`, `format_week_list(events)` / `format_week_list_at(events, now)`, and `format_month_list(events)` / `format_month_list_at(events, now)` (MarkdownV2 event lists built by the private `format_list`; each row shows the absolute datetime plus a short relative time like `13 mins`, `1h`, `2d`, `1w` via the private `format_relative`), `extract_chat_info(chat)` → `ChatInfo`.

- **logger.rs** — `init()` sets up `flexi_logger` with daily rotation to `LOG_DIR` + stdout.

- **bin/bench.rs** — Storage throughput benchmark.

## Test Cases

`test-cases.md` holds markdown tables that drive `tests/table_tests.rs`. Rows alternate `USER` (parse + insert via `insert_event_and_get_at`) and `SYSTEM` (`update_at_and_reload`, then assert `next_datetime`, or `NONE` for inactive). Add scenarios by appending `###` sections — no code changes needed.

## Datetime formats supported
- `13:23`, `5:24 PM`, `1:23 26.11`, `31.12.2027` — clock time matched anywhere; bare hour / relative offset / short date must lead the message.
- `13:45 mon-fri`, `13:25 thu-fri,sun 2023` — weekday sets, optional year.
- `14:55 20.05 every 2 weeks`, `15:30 every 3 days` — start datetime then repeat interval.
- `8 call Alex` → next 08:00; `24 call Poly` → next 00:00; `25 ...` → invalid bare hour (not parsed).
- `8 min call her`, `8 min every hour` — relative offset, optionally repeating.
- `10:00 first sunday`, `9:30 last monday`, `14:00 second thursday`, `17:00 3rd friday` — ordinal weekday (`1st`–`5th`, `last`) of the month.
- `18:00 last day of the month`, `18:00 last day` — last day of month ("of the month" optional).
