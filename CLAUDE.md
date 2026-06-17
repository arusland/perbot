# CLAUDE.md

## Workflow Rules

- When you complete work, print a short commit message for the changes.
- After a feature, module, command, or behavior change, update this file to keep it accurate.

## Project Overview

Perbot is a Telegram reminder bot in Rust (edition 2024). Users send a message starting with a natural-language time expression (e.g. `13:30 call the office`); the bot parses it, persists the event to SQLite, schedules it, and fires the reminder when due. Active events are reloaded on restart.

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

- **types.rs** — Shared types: `EventInfo` (parsed + DB fields, incl. `legacy` and `snoozed` flags), `MessageInfo`, `ChatInfo`/`ChatType` (`FromStr`), `TgMessage` (`chat_id`, `text`, `snooze`), `MessageSender`, time enums (`TimeUnit`, `Repetition`, `Ordinal`, `MonthlyPattern`), and helpers `parse_days`, `unit_from_str`, `day_from_str`, `day_to_str` (weekday↔string mapping lives only here).

- **parser.rs** — `parse(text) -> Option<EventInfo>`. Stateless regex extraction; clock time matched anywhere, relative offset/bare hour/short date anchored to start. A standalone 4-digit token in 2000..=2100 is a year restriction. Rest of text becomes the message.

- **scheduler.rs** — Pure datetime math. `calc_next(EventInfo)` / `calc_next_at(EventInfo, now)` set `active` + `next_datetime`.

- **error.rs** — Crate error type (`thiserror`) + `Result<T>`. Library modules return this; binaries use `anyhow` on top.

- **storage.rs** — `EventStorage` over rusqlite, returning `crate::error::Result`. Tables `chats`, `messages`, `events` (`events.msg_id` NOT NULL FK; `legacy`/`snoozed` flags in original schema). `open(path)`/`open_in_memory()`. Events: `insert_event`, `get_event`, `get_by_chat`, `get_active_events`, `get_active_by_chat`, `get_active_by_chat_on_date`, `get_active_by_chat_in_range` (end exclusive), `get_next_event`, `get_missed_events`, `get_events_at`, `update_schedule`, `mark_inactive`, `delete`, `delete_inactive`. Chats: `upsert_chat`, `get_chat`, `get_all_chats`. Messages: `insert_message`.

- **state.rs** — `EventProvider`: `Clone` handle over `Arc<Mutex<EventProviderState>>` (storage + cached `next_event`). `start(msg_tx)` sends missed events (`snooze: false`) then spawns a 1s poll thread firing due events (`snooze: true`) and rescheduling. Wraps storage plus `insert_event_and_get[_at]`, `insert_prebuilt_event` (insert already-scheduled event without re-scheduling; used by importer and snooze), `update_at_and_reload`.

- **commands.rs** — `Command` enum (`BotCommands`): `/help`, `/events`, `/today`, `/tomorrow`, `/week`, `/month`, admin-only `/import <user_id>` and `/exit` (hidden). `CmdContext` bundles deps; `Command::handle(ctx)` dispatches. All five list commands are paginated via private `ListKind` (tags `ev`/`td`/`tm`/`wk`/`mo`; date ranges recomputed relative to now). `handle_list` renders page 0 (`telegram::format_page_at`, `LIST_PAGE_SIZE`/page) with a `◀ Prev`/`Next ▶` keyboard (callback `<tag>:<page>`); on send failure it warns admin instead of bubbling up. `handle_list_callback` decodes the tag, re-fetches, edits in place. `handle_import_zip` downloads the zip, runs `import::import_zip`, replies with summary + HTML report. Replies use MarkdownV2 except `/help` and `/import` (plain text). **Snooze:** fired reminders carry `snooze_keyboard` (`SNOOZE_OPTIONS`, callback `sn:<minutes>`: 1/5/10/30 min, 1/8 h, 1 day). `handle_snooze_callback` recovers the title from the message text and inserts a one-off `snoozed_event` at `now + minutes` via `insert_message` + `insert_prebuilt_event`; original event untouched.

- **converter.rs** — Pure, unit-tested conversion of legacy MateBot `.alert` files (see `OLD-SPEC.md`) into `EventInfo` (legacy=true). `created_at_from_filename` parses `YYYYMMDD_HHMMSS_mmm.alert`; `extract_input` handles plain-text vs JSON; `convert(...)` maps the old grammar onto `EventInfo`. Future `lastActivePeriodTime` used directly; stale one rolled forward (flagged `recalculated`); else `scheduler::calc_next_at`. Unparsable inputs kept as inactive raw-text events.

- **import.rs** — Admin `/import` orchestration. `PendingImport = Arc<Mutex<Option<i64>>>` holds the target chat between `/import <user_id>` and the zip. `import_zip(provider, target, bytes)` converts each `.alert` entry, upserts the chat, inserts a synthetic message + event via `insert_prebuilt_event`, returns counts + an HTML old→new report.

- **main.rs** — Entry point + teloxide `Dispatcher` + wiring. At startup clears stale commands from non-default scopes then `set_my_commands`. Two branches share deps via `dptree::deps!`: `message_handler` and `callback_handler` (routes `sn:` → `handle_snooze_callback`, else `handle_list_callback`). The sender task attaches `snooze_keyboard()` to `TgMessage`s flagged `snooze`. `message_handler`: upserts chat, computes `is_admin`; admin document during pending import → `handle_import_zip`; else store message, `Command::parse` → `cmd.handle(ctx)`, non-command text → `parser::parse` → `insert_event_and_get` (MarkdownV2, escaped).

- **telegram.rs** — `escape_markdown`; `format_page[_at](...)` → `(text, total_pages)` MarkdownV2 paginated lists (title heading + `(page x/y)` only when >1 page; rows show absolute datetime + short relative time). Helpers `LIST_PAGE_SIZE`, `total_pages`. `extract_chat_info(chat)` → `ChatInfo`.

- **logger.rs** — `init()` sets up `flexi_logger` (daily rotation to `LOG_DIR` + stdout).

- **bin/bench.rs** — Storage throughput benchmark.

## Test Cases

`test-cases.md` holds markdown tables driving `tests/table_tests.rs`. Rows alternate `USER` (parse + `insert_event_and_get_at`) and `SYSTEM` (`update_at_and_reload`, assert `next_datetime` or `NONE`). Add scenarios by appending `###` sections — no code changes needed.

## Datetime formats supported
- `13:23`, `5:24 PM`, `1:23 26.11`, `31.12.2027` — clock time anywhere; bare hour / relative offset / short date must lead.
- `13:45 mon-fri`, `13:25 thu-fri,sun 2023` — weekday sets, optional year.
- `14:55 20.05 every 2 weeks`, `15:30 every 3 days` — start datetime then repeat interval.
- `8 call Alex` → next 08:00; `24 ...` → 00:00; `25 ...` → invalid (not parsed).
- `8 min call her`, `8 min every hour` — relative offset, optionally repeating.
- `10:00 first sunday`, `9:30 last monday`, `17:00 3rd friday` — ordinal weekday (`1st`–`5th`, `last`) of the month.
- `18:00 last day of the month`, `18:00 last day` — last day of month.
