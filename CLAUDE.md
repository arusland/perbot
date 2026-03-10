# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Perbot is a Telegram reminder bot written in Rust. Users send messages containing natural language time/date expressions (e.g., "13:30 call the office"), the bot parses the datetime, schedules an async task, persists the event to SQLite, and sends the reminder when the time arrives. On restart, active events are reloaded and rescheduled.

## Build & Test Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo test                     # Run all tests (across parser, storage, and scheduler)
cargo test parser::tests       # Run only parser tests
cargo test storage::tests      # Run only storage tests
cargo test scheduler::tests    # Run only scheduler tests
cargo test <test_name>         # Run a single test by name
```

## Environment Variables

- `TELOXIDE_TOKEN` — Telegram bot API token (required)
- `TG_ADMIN_ID` — Admin chat ID for startup notification (required, i64)
- `RUST_LOG` — Log level for `pretty_env_logger` (e.g., `info`, `debug`)

## Architecture

Five source files in `src/`:

- **main.rs** — Bot entry point. Initializes teloxide REPL. All event and storage access goes through `EventProvider` (shared as `Arc<Mutex<EventProvider>>`). On startup calls `provider.reload()` then `schedule_first_event`. Handles incoming messages: for every message (any type), chat info is saved/updated via `provider.upsert_chat`. For text messages, the message is stored via `provider.insert_message` to obtain a `msg_id`. Then parses with `parser::parse`, sets `chat_id` and `msg_id` on the `EventInfo`, and calls `provider.insert_and_get(event)` which calculates datetime, persists to DB, reloads the next event, and returns the event as read back from DB. After insertion calls `schedule_first_event` to reschedule. When the scheduled event fires: queries storage for ALL events at that timestamp via `provider.get_events_at(dt)`, sends all their messages through the channel as a `Vec<(ChatId, String)>`, then calls `provider.update_and_reload(events)` which recalculates each event and reloads the next one from DB. Then schedules the next event. The Telegram message-sending channel accepts `Vec<(ChatId, String)>` to support batching multiple simultaneous events. All responses use MarkdownV2 parse mode (escaped via `escape_markdown`).

- **state.rs** — `EventProvider` struct encapsulating `EventStorage`, a single `Option<EventInfo>` for the nearest active event, and an `AbortHandle` for the current scheduled task. Key methods: `upsert_chat(&ChatInfo)` delegates chat persistence to storage; `insert_message(user_id, chat_id, text)` constructs a `MessageInfo` and delegates to storage, returning the message ID; `get_next()` returns the nearest active event; `get_event(id)` returns an event by ID; `get_events_at(dt)` queries storage for all active events at a specific datetime; `update(event)` recalculates the event's next occurrence via `scheduler::calc_next_at` using current time and saves to DB; `update_at(event, now)` same as `update` but with a custom datetime; `update_and_reload(events)` updates all given events then reloads the next event from DB; `insert_and_get(event)` calculates datetime via `scheduler::calc_next`, persists via `insert_event`, reloads the next event, and returns the event as read back from DB via `get_event`; `insert_and_get_at(event, now)` same as `insert_and_get` but uses `calc_next_at` with a custom datetime; `reload()` loads the single nearest active event from DB; `abort_current()` and `set_abort_handle()` manage the scheduled task handle.

- **parser.rs** — Stateless datetime extraction. `parse(text) -> Option<EventInfo>` uses regex to extract time from the beginning of a message; remainder becomes the event message. `EventInfo` carries both the parsed datetime fields (rich types: `HashSet<Weekday>`, `Repetition`, `MonthlyPattern`, `Option<HashSet<i32>>` for years, etc.) and DB-tracking fields (`id`, `chat_id`, `active`, `next_datetime`, `created_at`, `msg_id`) defaulted to zero/false/None by `parse()`.

- **scheduler.rs** — Pure datetime computation. `calc_next(EventInfo) -> EventInfo` and `calc_next_at(EventInfo, NaiveDateTime) -> EventInfo` calculate the next occurrence directly from `EventInfo`'s rich-typed fields and return the event with `active` and `next_datetime` set. Contains all related helpers: `calculate_next_datetime`, weekday utilities, monthly pattern logic, and `advance_by`.

- **storage.rs** — SQLite persistence via rusqlite. `EventStorage` manages three tables: `chats` (id, chat_type, title, username, first_name, last_name, updated_at, created_at), `messages` (id, user_id, chat_id, created_at, message), and `events` (id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id, years). `msg_id` in events is a NOT NULL foreign key referencing `messages(id)`. Provides `open(path)` for file-backed DB and `open_in_memory()` for tests. `upsert_chat(&ChatInfo)` inserts or updates chat info. `insert_message(&MessageInfo)` stores every incoming user message and returns its ID. `insert_event(&EventInfo)` serializes rich types to DB strings internally and persists the event. `update_schedule(id, active, next_datetime)` updates the schedule after each fire. `get_event(id)` returns an event by ID. `get_next_event(now)` returns the single nearest active event from `now`. `get_events_at(dt)` returns all active events with the exact given `next_datetime`. `get_active_events()` and other getters deserialize DB rows back into `EventInfo` values.

## Test Cases

`test-cases.md` in the project root contains markdown tables that drive the integration test in `tests/table_tests.rs`. Each table is a scenario; rows alternate between `USER` actions (a raw chat message parsed and inserted via `EventProvider::insert_and_get_at`) and `SYSTEM` actions (update the current event via `EventProvider::update_at` and assert that `next_datetime` equals the expected value, or that the event is inactive when the expected value is `NONE`). To add new scenarios, append new `###` sections with the same table format to `test-cases.md` — no code changes required.

## Datetime formats supported
- `13:23`, `5:24 PM`, `1:23 26.11`, `31.12.2027` — always at the start of the message.
- `13:45 mon-fri` — every Monday, Tuesday, Wednesday, Thursday, and Friday at 13:45.
- `13:25 thu-fri,sun 2023` — every Thursday, Friday, and Sunday in 2023 at 13:25.
- `14:55 20.05 every 2 weeks` - start at 14:55 every year on date 20.05 and then repeat every 2 weeks.
- `15:30 every 3 days` — start at next 15:30 and then repeat every 3 days.
- `8 call Alex` - fire event at next 08:00 (from current time)
- `24 call Poly` - fire event at next 00:00
- `25 call Alex` - do not parse invalid bare hour
- `8 min call her` - fire event in 8 minutes
- `8 min every hour` - fire event in 8 minutes and repeat every hour
- `10:00 first sunday call mom` - fire event at 10:00 on the first Sunday of the month
- `9:30 last monday team sync` - fire event at 9:30 on the last Monday of the month
- `14:00 second thursday board meeting` - fire event at 14:00 on the second Thursday of the month
- `17:00 3rd friday happy hour` - ordinal can also be `1st`, `2nd`, `3rd`, `4th`, `5th`
- `18:00 last day of the month pay rent` - fire event at 18:00 on the last day of the month
- `18:00 last day pay bills` - "of the month" is optional
