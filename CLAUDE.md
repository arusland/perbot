# CLAUDE.md

## Workflow Rules

- Format code (`cargo fmt`) before running tests.
- When you complete work, print a short commit message for the changes.
- After a feature, module, command, or behavior change, update this file to keep it accurate.
- When asked to add a new datetime format, treat it with the same coverage as existing formats: update the parser (`parser.rs`), scheduling math (`scheduler.rs`) as needed, document it under **Datetime formats supported**, add unit tests alongside the existing ones, and add USER/SYSTEM scenarios to `test-cases.md`. If the new format conflicts or ambiguously overlaps with an existing format, stop and propose resolution options instead of silently changing existing behavior.

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

- **types.rs** — Shared types: `EventInfo` (parsed + DB fields, incl. `legacy` and `snoozed` flags), `MessageInfo`, `ChatInfo`/`ChatType` (`FromStr`), `TgMessage` (`chat_id`, final `text`, `reply_markup`: optional inline keyboard the producer chose; the sender task forwards it verbatim, deciding nothing), `MessageSender`, time enums (`TimeUnit`, `Repetition`, `Ordinal`, `MonthlyPattern`), and helpers `parse_days`, `unit_from_str`, `day_from_str`, `day_to_str` (weekday↔string mapping lives only here).

- **parser.rs** — `parse(text) -> Option<EventInfo>`. Stateless regex extraction; clock time matched anywhere, relative offset/bare hour/short date anchored to start. A standalone 4-digit token in 2000..=2100 is a year restriction. Rest of text becomes the message.

- **scheduler.rs** — Pure datetime math. `calc_next(EventInfo)` / `calc_next_at(EventInfo, now)` set `active` + `next_datetime`.

- **error.rs** — Crate error type (`thiserror`) + `Result<T>`. Library modules return this; binaries use `anyhow` on top.

- **storage.rs** — `EventStorage` over rusqlite, returning `crate::error::Result`. Tables `chats`, `messages`, `events` (`events.msg_id` NOT NULL FK; `legacy`/`snoozed` flags in original schema). `open(path)`/`open_in_memory()`. Events: `insert_event`, `get_event`, `get_by_chat`, `get_active_events`, `get_active_by_chat`, `get_active_by_chat_on_date`, `get_active_by_chat_in_range` (end exclusive), `get_next_event`, `get_missed_events`, `get_events_at`, `update_schedule`, `mark_inactive`, `delete`, `delete_inactive`. Chats: `upsert_chat`, `get_chat`, `get_all_chats`. Messages: `insert_message`.

- **state.rs** — `EventProvider`: `Clone` handle over `Arc<Mutex<EventProviderState>>` (storage + cached `next_event`). `start(msg_tx)` sends missed events (no keyboard) then spawns a 1s poll thread firing due events (each built here with a "Next launches" preview and the `SNOOZE_HINT` appended and `snooze_keyboard(id)` in `reply_markup`) and rescheduling. The fired message text is `<message><preview>\n\n<SNOOZE_HINT>`, where the preview is `telegram::next_launches_preview(event, fire_dt)` (plain text, no escaping since the fired message is sent without a parse mode). Snooze presentation (`SNOOZE_OPTIONS`/`SNOOZE_HINT`/`snooze_keyboard`, callback `eid:<id>:sn:<minutes>`: 1/5/10/30 min, 1/2/8 h, 1 day) lives here, next to the only code that uses it. Wraps storage plus `insert_event_and_get[_at]`, `insert_prebuilt_event` (insert already-scheduled event without re-scheduling; used by importer and snooze), `update_at_and_reload`.

- **commands.rs** — `Command` enum (`BotCommands`): `/help`, `/events`, `/today`, `/tomorrow`, `/week`, `/month`, admin-only `/import <user_id>` and `/exit` (hidden). `CmdContext` bundles deps; `Command::handle(ctx)` dispatches. All five list commands are paginated via private `ListKind` (tags `ev`/`td`/`tm`/`wk`/`mo`; date ranges recomputed relative to now). `handle_list` renders page 0 (`telegram::format_page_at`, `LIST_PAGE_SIZE`/page) with a `◀ Prev`/`Next ▶` keyboard (callback `<tag>:<page>`); on send failure it warns admin instead of bubbling up. `handle_list_callback` decodes the tag, re-fetches, edits in place. `handle_import_zip` downloads the zip, runs `import::import_zip`, replies with summary + HTML report. Replies use MarkdownV2 except `/help` and `/import` (plain text). **Snooze:** fired reminders carry the snooze keyboard + hint, built by the producer in `state.rs` (keyboard, callback `eid:<id>:sn:<minutes>`, and hint live in `state.rs`). Event-specific callbacks use the reusable `eid:<id>:<action>:<args>` envelope. `parse_snooze_callback` decodes the data to `(event_id, minutes)`; `handle_snooze_callback` loads the event by id (`provider.get_event`), **access-checks** that `event.chat_id` matches the chat the button was pressed in (rejects otherwise), uses the stored `event.message` as the title, and inserts a one-off `snoozed_event` at `now + minutes` via `insert_message` + `insert_prebuilt_event`; original event untouched. On success it sends the shared `telegram::scheduled_message(now, next, &event)` confirmation (MarkdownV2) to the chat (one-off snooze → no preview).

- **converter.rs** — Pure, unit-tested conversion of legacy MateBot `.alert` files (see `OLD-SPEC.md`) into `EventInfo` (legacy=true). `created_at_from_filename` parses `YYYYMMDD_HHMMSS_mmm.alert`; `extract_input` handles plain-text vs JSON; `convert(...)` maps the old grammar onto `EventInfo`. Future `lastActivePeriodTime` used directly; stale one rolled forward (flagged `recalculated`); else `scheduler::calc_next_at`. Unparsable inputs kept as inactive raw-text events.

- **import.rs** — Admin `/import` orchestration. `PendingImport = Arc<Mutex<Option<i64>>>` holds the target chat between `/import <user_id>` and the zip. `import_zip(provider, target, bytes)` converts each `.alert` entry, upserts the chat, inserts a synthetic message + event via `insert_prebuilt_event`, returns counts + an HTML old→new report.

- **main.rs** — Entry point + teloxide `Dispatcher` + wiring. At startup clears stale commands from non-default scopes then `set_my_commands`. Two branches share deps via `dptree::deps!`: `message_handler` and `callback_handler` (routes `eid:` → `handle_snooze_callback`, else `handle_list_callback`). The sender task is a dumb pump: it sends each `TgMessage`'s `text` and attaches its `reply_markup` if present, deciding nothing about buttons (the producer in `state.rs` does). `message_handler`: upserts chat, computes `is_admin`; admin document during pending import → `handle_import_zip`; else store message, `Command::parse` → `cmd.handle(ctx)`, non-command text → `parser::parse` → `insert_event_and_get` (MarkdownV2, escaped).

- **telegram.rs** — `escape_markdown`; `format_when(now, dt)` → plain-text `HH:MM dd.mm.yyyy (relative)` for a single datetime (unescaped); `next_launches_preview(event, after)` walks `scheduler::calc_next_at` forward from `after` to list up to `MAX_NEXT_PREVIEW` (3) upcoming launches as `• <format_when>` bullets, plus a trailing `• ...` when more remain (empty string for one-off events); returns plain text (callers targeting MarkdownV2 escape it). `scheduled_message(now, dt, event)` → shared `Scheduled message for *<format_when, escaped>*` confirmation (bolded absolute datetime + relative time from `now`, e.g. `13:30 22\.06\.2026 \(1d\)`; used on new parse in `main.rs` and on snooze), with the (escaped) `next_launches_preview` appended for recurring events; `format_page[_at](...)` → `(text, total_pages)` MarkdownV2 paginated lists (title heading + `(page x/y)` only when >1 page; rows show absolute datetime + short relative time). Helpers `LIST_PAGE_SIZE`, `total_pages`. `extract_chat_info(chat)` → `ChatInfo`.

- **logger.rs** — `init()` sets up `flexi_logger` (daily rotation to `LOG_DIR` + stdout).

- **bin/bench.rs** — Storage throughput benchmark.

## EventInfo fields

`EventInfo` (`types.rs`) carries one reminder end to end. The first group is set by `parser::parse`; the trailing group is filled by storage/scheduling and defaults to zero/`false`/`None` on a freshly parsed value. The time/recurrence fields are largely mutually exclusive — a given message populates whichever one matched.

| Field | Type | Set by | Used when / for |
|-------|------|--------|-----------------|
| `date` | `Option<NaiveDate>` | parser | Short date / full date given (`1:23 26.11`, `31.12.2027`). One-off on that calendar day. |
| `time` | `Option<NaiveTime>` | parser | Clock time given anywhere (`13:23`, `5:24 PM`). Combined with `date`/`days`/`monthly_pattern`/`years`; absent for `in_offset`/`bare_hour`. |
| `year_explicit` | `bool` | parser | `true` only when a full date spelled the year (`31.12.2027`); controls whether the year in `date` is honored or rolled forward. |
| `days` | `Option<HashSet<Weekday>>` | parser | Weekday-set recurrence (`13:45 mon-fri`). Fires at `time` on each listed weekday; pairs with `years` to restrict to given years. |
| `years` | `Option<HashSet<i32>>` | parser | Standalone 4-digit year token(s) in 2000..=2100 (`13:25 2027 fri,sun`). Restricts a `days` schedule to those years. |
| `repetition` | `Option<Repetition>` | parser | `every <n> <unit>` interval (`every 2 weeks`, `every hour`). Recurs from the start datetime / offset; reused by `scheduler` to advance `next_datetime`. |
| `in_offset` | `Option<(u32, TimeUnit)>` | parser | Relative offset (`8 min call her`, `2 hours reminder`). Schedules at `now + offset`; with `repetition`, repeats by that interval. Mutually exclusive with `time`/`date`. |
| `bare_hour` | `Option<u32>` | parser | Leading bare hour 0..=24 (`8 call Alex` → next 08:00, `24` → 00:00). Next occurrence of that hour. |
| `monthly_pattern` | `Option<MonthlyPattern>` | parser | Ordinal weekday (`first sunday`, `3rd friday`) or last day of month (`last day`). Fires monthly at `time` on the matching day. |
| `message` | `String` | parser | Remainder after extracting time/date components — the reminder text sent to the user. |
| `id` | `i64` | storage | DB primary key; `0` before insert, real id after. |
| `chat_id` | `i64` | storage/caller | Destination chat; set when associating the event with a chat. |
| `active` | `bool` | scheduler | `true` while the event still has a future occurrence; `mark_inactive` / scheduling clears it when exhausted. |
| `next_datetime` | `Option<NaiveDateTime>` | scheduler | Next fire time computed by `calc_next[_at]`; `None` when no future occurrence (event becomes inactive). |
| `created_at` | `NaiveDateTime` | storage/converter | Insertion time; for legacy imports parsed from the `.alert` filename. |
| `msg_id` | `i64` | storage/caller | FK to the originating `messages` row (`events.msg_id` NOT NULL). |
| `legacy` | `bool` | converter | `true` for events imported from legacy MateBot `.alert` files. |
| `snoozed` | `bool` | snooze flow | `true` for one-off events created by `handle_snooze_callback`. |

## Test Cases

`test-cases.md` holds markdown tables driving `tests/table_tests.rs`. Rows alternate `USER` (parse + `insert_event_and_get_at`) and `SYSTEM` (`update_at_and_reload`, assert `next_datetime` or `NONE`). A 4th column carries the expected reminder message: on `USER` rows it must equal the parsed `event.message` (asserted); on `SYSTEM` rows it is empty. Add scenarios by appending `###` sections — no code changes needed.

## Datetime formats supported
- `13:23`, `5:24 PM`, `1:23 26.11`, `31.12.2027` — clock time anywhere; bare hour / relative offset / short date must lead. Minutes accept 1-2 digits, so `10:6` means `10:06` (`9:5 PM` → 21:05).
- `13:45 mon-fri`, `13:25 thu-fri,sun 2023` — weekday sets, optional year. An optional leading `every` is absorbed, so `10:30 every fri` is identical to `10:30 fri`.
- `14:55 20.05 every 2 weeks`, `15:30 every 3 days` — start datetime then repeat interval.
- `8 call Alex` → next 08:00; `24 ...` → 00:00; `25 ...` → invalid (not parsed).
- `8 min call her`, `8 min every hour` — relative offset, optionally repeating.
- `10:00 first sunday`, `9:30 last monday`, `17:00 3rd friday` — ordinal weekday (`1st`–`5th`, `last`) of the month.
- `18:00 last day of the month`, `18:00 last day` — last day of month.
