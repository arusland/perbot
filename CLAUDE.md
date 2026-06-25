# CLAUDE.md

## Workflow Rules

- `cargo fmt` and `cargo clippy` before running tests.
- Print a short commit message when work is done.
- Keep this file accurate after any feature/module/command/behavior change.
- **Adding a datetime format:** give it the same coverage as existing ones — update `parser.rs`, `scheduler.rs` (if math changes), document under **Datetime formats**, add unit tests, and add USER/SYSTEM rows to `test-cases.md`. If it conflicts/overlaps with an existing format, stop and propose resolutions instead of silently changing behavior.

## Project Overview

Perbot is a Telegram reminder bot in Rust (edition 2024). A user sends a message led by a natural-language time expression (`13:30 call the office`); the bot parses it, persists the event to SQLite, schedules it, and fires the reminder when due. Active events reload on restart.

## Build & Test

```bash
cargo build [--release]
cargo test                 # parser/storage/scheduler/table tests
cargo test <name>          # single test/module, e.g. parser::tests
cargo run --bin bench      # storage benchmark (1000 events)
```

## Environment Variables

- `TG_BOT_TOKEN` — bot API token (required)
- `TG_ADMIN_ID` — admin chat ID, i64 (required)
- `RUST_LOG` — `flexi_logger` level (e.g. `info`)
- `LOG_DIR` — log directory (default `logs`)

## Key Invariants

These are the conventions that aren't obvious from any single file:

- **Everything is HTML.** `EventInfo.message` is an **HTML fragment** (user's Telegram formatting preserved as tags; plain text escaped). All message-bearing output is sent with `ParseMode::Html` — there is no MarkdownV2 in the codebase. Only `< > &` need escaping (`teloxide::utils::html::escape`); bot-generated bits (datetimes, `•`, `(1d)`) have no specials. Conversion to HTML happens at ingestion: `main` via `richtext::render_html`, `converter` escapes legacy text, snooze reuses stored HTML.
- **`TgMessage` is opaque to the sender.** It carries final `text` (HTML) + optional `reply_markup`; the sender task in `main` forwards both verbatim. Producers (mainly `state.rs`) decide buttons.
- **Time/recurrence fields are mutually exclusive** — a message populates whichever one matched (see EventInfo table).
- **`normalize_time()` is canonical and re-parseable** (idempotent): collapses loose spellings into a string that parses back to the same event. Drives the edit/round-trip flow.
- **Callback envelope** for event actions: `eid:<id>:<action>:<args>`. Every handler that acts on an id **access-checks** `event.chat_id` against the chat the button was pressed in (callback ids are user-influenceable).
- **Pending flows are in-memory only** (restart drops them).

## Modules

`lib.rs` re-exports all modules. Shared types live in `types.rs`.

- **types.rs** — `EventInfo` (parsed + DB fields), `MessageInfo`, `ChatInfo`/`ChatType`, `TgMessage`, `MessageSender`, time enums (`TimeUnit`/`Repetition`/`Ordinal`/`MonthlyPattern`). Weekday↔string mapping and parse helpers (`parse_days`, `unit_from_str`, …) live only here. `EventInfo::normalize_time()` (canonical re-parseable string — see Invariants) and name helpers (`day_to_str_cap`/`weekday_full`/`ordinal_word`/`ordinal_suffix`, reused by `telegram`).

- **parser.rs** — Stateless regex extraction. `parse` / `parse_full` (also returns surviving body byte-ranges for `richtext`) / `parse_time_only` (time present, body empty → drives main's "send me the text" flow). Clock time matches anywhere; offset/bare-hour/short-date must lead. Standalone 4-digit token in 2000..=2100 is a year restriction. Body derived via `richtext::normalize`; `main` overwrites it with the HTML render before persisting.

- **richtext.rs** — Pure. `normalize` is the single source of truth for body normalization: collapses intra-line whitespace to single spaces, **preserves line breaks verbatim**, tracks each char's source byte offset. `render_html` rebuilds `MessageEntity`s (UTF-16) over the leftover text and renders via teloxide's `Renderer`; falls back to `html::escape` with no entities.

- **scheduler.rs** — Pure datetime math. `calc_next[_at](EventInfo[, now])` set `active` + `next_datetime`.

- **error.rs** — Crate error (`thiserror`) + `Result<T>`. Libraries use it; binaries wrap with `anyhow`.

- **storage.rs** — `EventStorage` over rusqlite. Tables `chats`, `messages`, `events` (`events.msg_id` NOT NULL FK; `legacy`/`snoozed` flags). Standard CRUD + range/active/missed queries + `backup_to` (`VACUUM INTO`).

- **state.rs** — `EventProvider`: `Clone` handle over `Arc<Mutex<_>>` (storage + cached `next_event`). `start(msg_tx)` sends missed events then spawns a 1s poll thread that fires due events and reschedules. Fired text = `<message><preview>\n\n<SNOOZE_HINT>` (message + preview are HTML fragments, hint escaped). **Snooze presentation lives here** (`SNOOZE_OPTIONS`/`SNOOZE_HINT`/`snooze_keyboard`, callback `eid:<id>:sn:<minutes>`). Insert/update wrappers: `insert_event_and_get[_at]`, `insert_prebuilt_event` (no re-scheduling; importer + snooze), `update_event_and_get[_at]` (edit flow), `delete` (reloads cached next).

- **commands.rs** — `Command` (`BotCommands`): `/help`, `/events`, `/today`, `/tomorrow`, `/week`, `/month`, admin `/import <user_id>`/`/database`/`/logs`, hidden `/exit`. List commands paginate via `ListKind`; `handle_list`/`handle_list_callback` edit in place with `list_keyboard` (`◀ Prev` / `<page>/<total>` no-op indicator / `Next ▶`). `/events` uses the two-line row layout. **Single-event view** `/event<id>` (matched manually in `main`, not in the command menu): `handle_event_view` → `event_detail` + `event_actions_keyboard` (Edit/Delete). Event callbacks decoded by `parse_event_callback`, dispatched by `handle_event_callback`: `sn` snooze, `del`/`delyes`/`delno` delete, `ed`/`edno` edit. Snooze inserts a one-off `snoozed` event reusing the original's HTML message. Delete/edit prompts edit the keyboard/message in place. All id-bearing handlers access-check.

- **converter.rs** — Pure. Converts legacy MateBot `.alert` files (`OLD-SPEC.md`) into `EventInfo` (legacy=true). Future `lastActivePeriodTime` used directly; stale rolled forward; else `calc_next_at`. Unparsable inputs kept as inactive raw-text events.

- **import.rs** — Admin `/import` orchestration. `PendingImport` holds target chat between command and zip; `import_zip` converts each entry via `insert_prebuilt_event`, returns counts + HTML report.

- **pending.rs** — In-memory flow state. `PendingMessage` (chat→body-less `EventInfo`) for the time-only flow + `ASK_TEXT`/`CANCEL_DATA`/`cancel_keyboard`. `PendingEdit` (chat→event id) for the edit flow + `EDIT_ASK_TEXT`/`EDIT_NEED_TEXT`/`EDIT_NEED_TIME` prompts.

- **main.rs** — Entry point + teloxide `Dispatcher`. Startup clears stale command scopes then `set_my_commands`. `message_handler` + `callback_handler` (routes `eid:`/`pm:`/list). Holds the two text-completion flows: **time-only** (`PendingMessage` → render body → schedule) and **edit-completion** (`PendingEdit` → re-parse, copy identity fields, `update_event_and_get`). Sender task is a dumb HTML pump.

- **telegram.rs** — All output HTML. `format_when` (single datetime + relative), `next_launches_preview` (up to 3 upcoming launches under a bold header; "" for one-off), `scheduled_message` (confirmation, used on new parse + snooze), `format_page[_at]` (paginated lists; `two_line` flag picks single- vs two-line rows, latter only for `/events`). `event_when_line` (shared bold line, recurrence appended inside the relative `(…)` via `describe_recurrence`), `event_detail` (`/event<id>` view). `html_to_plain`/`message_preview` (strip+truncate by char count), `event_source_input`/`edit_prompt` (reconstruct re-parseable input as tap-to-copy `<code>`).

- **logger.rs** — `init()` sets up `flexi_logger` (daily rotation to `LOG_DIR` + stdout); `current_log_path`.

- **bin/bench.rs** — Storage throughput benchmark.

## EventInfo fields

`EventInfo` (`types.rs`) carries one reminder end to end. Parser sets the first group; storage/scheduler fill the rest (defaulting to zero/`false`/`None` on a fresh parse). Time/recurrence fields are largely mutually exclusive.

| Field | Type | Set by | Notes |
|-------|------|--------|-------|
| `date` | `Option<NaiveDate>` | parser | Short or full date. Short date (no year) → **yearly** (unless a non-year `repetition` makes it the start anchor). Full date → one-off (unless `every year`/`every N years`). |
| `time` | `Option<NaiveTime>` | parser | Clock time anywhere. Absent for `in_offset`/`bare_hour`. |
| `year_explicit` | `bool` | parser | `true` only when a full date spelled the year — honors `date`'s year vs rolling it forward yearly. |
| `days` | `Option<HashSet<Weekday>>` | parser | Weekday-set recurrence; pairs with `years`. |
| `years` | `Option<HashSet<i32>>` | parser | Standalone year token(s) 2000..=2100; restricts a `days` schedule. |
| `repetition` | `Option<Repetition>` | parser | `every <n> <unit>` interval. On a short date, a non-year repetition overrides the yearly wrap; a year-unit one is dropped as redundant. |
| `in_offset` | `Option<(u32, TimeUnit)>` | parser | Relative offset (`now + offset`); with `repetition` repeats. Exclusive with `time`/`date`. |
| `bare_hour` | `Option<u32>` | parser | Leading bare hour 0..=24 → next occurrence of that hour. |
| `monthly_pattern` | `Option<MonthlyPattern>` | parser | Ordinal weekday / last day / fixed `DayOfMonth`. With a `repetition`, the day-of-month anchor has priority. |
| `message` | `String` | parser → `main` | Body as **HTML fragment** (see Invariants). |
| `id` | `i64` | storage | PK; `0` before insert. |
| `chat_id` | `i64` | storage/caller | Destination chat. |
| `active` | `bool` | scheduler | `true` while a future occurrence remains. |
| `next_datetime` | `Option<NaiveDateTime>` | scheduler | Next fire; `None` → inactive. |
| `created_at` | `NaiveDateTime` | storage/converter | Insertion time (legacy: from `.alert` filename). |
| `msg_id` | `i64` | storage/caller | FK to originating `messages` row. |
| `legacy` | `bool` | converter | Imported from legacy `.alert`. |
| `snoozed` | `bool` | snooze flow | One-off snooze copy. |

## Test Cases

`test-cases.md` holds markdown tables driving `tests/table_tests.rs`. Rows alternate `USER` (parse + `insert_event_and_get_at`) and `SYSTEM` (`update_at_and_reload`, assert `next_datetime` or `NONE`). Column 4 = expected `event.message` (asserted on USER rows). Column 5 = expected `normalize_time()` (asserted on USER rows; empty when input doesn't parse). A literal `\n` in Input/Message is decoded to a real newline. Add scenarios by appending `###` sections — no code changes needed.

## Datetime formats

- `13:23`, `5:24 PM`, `1:23 26.11`, `31.12.2027` — clock time anywhere; bare hour / offset / short date must lead. Minutes accept 1-2 digits (`10:6` → `10:06`).
- **Short date, no year** (`10:03 15.12`) → **yearly**; a redundant `every year`/`yearly` is absorbed (canonical `10:03 15.12 yearly`). **Full date + `every year`/`every N years`** → true yearly repetition (first fire on the date). **Full date alone** → one-off.
- **Short date + non-year repetition** (`11:07 05.11 every 2 days`) → date is the start anchor, repetition governs. Canonical keeps trailing `yearly`: `11:07 05.11 every 2 days yearly`.
- `13:45 mon-fri`, `13:25 thu-fri,sun 2023` — weekday sets, optional year. Leading `every` absorbed.
- `14:55 20.05 every 2 weeks`, `15:30 every 3 days` — start datetime then interval.
- `8 call Alex` → next 08:00; `24` → 00:00; `25` → invalid.
- `8 min call her`, `in 8 min every 2 hours test` — relative offset, optionally repeating; leading `in` absorbed (and canonical).
- `10:00 first sunday`, `17:00 3rd friday` — ordinal weekday (`1st`–`5th`, `last`) of the month.
- `18:00 last day of the month`, `18:00 last day` — last day of month.
- `12:05 28th of the month`, `every 28 of the month`, `each 5 of the month` — fixed calendar day (`1`–`31`); `of [the] month` required, optional `day`/ordinal/`every`/`each` absorbed. Missing days (Feb 31) skipped. Combinable with an interval (anchor has priority). Canonical `each <N><ord> day of the month`.
</content>
</invoke>
