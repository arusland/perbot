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

- `TG_BOT_TOKEN` — Telegram bot API token (required)
- `TG_ADMIN_ID` — Admin chat ID for startup notification (required, i64)
- `RUST_LOG` — Log level for `pretty_env_logger` (e.g., `info`, `debug`)

## Architecture

Seven source files in `src/`:

- **types.rs** — All shared data types used across modules. Contains: `TimeUnit`, `Repetition`, `Ordinal`, `MonthlyPattern`, `EventInfo` (parsed datetime fields plus DB-tracking fields), `MessageInfo`, `ChatType`, `ChatInfo`, `TgMessage`, `MessageSender` type alias, and shared helper functions `parse_days` (weekday string parsing) and `unit_from_str` (time unit string parsing).

- **telegram.rs** — Telegram-specific utility functions. `escape_markdown(text)` escapes MarkdownV2 special characters for safe use in Telegram messages. `extract_chat_info(chat)` converts a teloxide `Chat` into the internal `ChatInfo` type, mapping private/group/supergroup/channel variants.

- **main.rs** — Bot entry point. Initializes teloxide REPL. All event and storage access goes through `EventProvider`, which is `Clone` and internally thread-safe (no external `Arc<Mutex<>>` needed). On startup calls `provider.start(msg_tx)` which reloads events, sends missed events, and spawns a background polling thread. Handles incoming messages: for every message (any type), chat info is saved/updated via `provider.upsert_chat`. For text messages, the message is stored via `provider.insert_message` to obtain a `msg_id`. Then parses with `parser::parse`, sets `chat_id` and `msg_id` on the `EventInfo`, and calls `provider.insert_and_get(event)` which calculates datetime, persists to DB, reloads the next event, and returns the event as read back from DB. The Telegram message-sending channel accepts `Vec<TgMessage>` to support batching multiple simultaneous events; the receiver wraps chat IDs in `ChatId` before sending. All responses use MarkdownV2 parse mode (escaped via `telegram::escape_markdown`).

- **state.rs** — `EventProvider` is a `Clone` handle wrapping `Arc<Mutex<EventProviderState>>`, encapsulating all synchronization internally. `EventProviderState` holds `EventStorage`, a single `Option<EventInfo>` for the nearest active event, and a `Vec<EventInfo>` of missed events. All public methods take `&self` and lock the mutex internally — callers never deal with `Arc` or `Mutex` directly. Key methods: `start(&self, msg_tx: MessageSender)` — reloads events from DB, sends missed events via the channel, then spawns a single background `std::thread` (using `self.clone()`) that polls every second: checks the nearest event's `next_datetime`, and when the current time reaches it, queries all events at that datetime, sends their messages, and calls `update_and_reload`; `upsert_chat(&ChatInfo)` delegates chat persistence to storage; `insert_message(user_id, chat_id, text)` constructs a `MessageInfo` and delegates to storage, returning the message ID; `get_next()` returns the nearest active event; `get_missed_events()` returns missed events as a cloned `Vec`; `get_event(id)` returns an event by ID; `get_events_at(dt)` queries storage for all active events at a specific datetime; `update(event)` recalculates the event's next occurrence via `scheduler::calc_next_at` using current time and saves to DB; `update_at(event, now)` same as `update` but with a custom datetime; `update_and_reload(events)` updates all given events then reloads the next event from DB (single lock acquisition); `insert_and_get(event)` calculates datetime via `scheduler::calc_next`, persists via `insert_event`, reloads the next event, and returns the event as read back from DB via `get_event` (single lock acquisition); `insert_and_get_at(event, now)` same as `insert_and_get` but uses `calc_next_at` with a custom datetime; `reload()` loads the single nearest active event and all missed events from DB. Private helper `reload_inner(&mut EventProviderState, now)` is used by methods that already hold the lock to avoid deadlocks.

- **parser.rs** — Stateless datetime extraction. `parse(text) -> Option<EventInfo>` uses regex to extract time from the beginning of a message; remainder becomes the event message. All types (`EventInfo`, `Repetition`, etc.) are imported from `types.rs`. Returns `EventInfo` with DB-tracking fields (`id`, `chat_id`, `active`, `next_datetime`, `created_at`, `msg_id`) defaulted to zero/false/None.

- **scheduler.rs** — Pure datetime computation. `calc_next(EventInfo) -> EventInfo` and `calc_next_at(EventInfo, NaiveDateTime) -> EventInfo` calculate the next occurrence directly from `EventInfo`'s rich-typed fields and return the event with `active` and `next_datetime` set. Contains all related helpers: `calculate_next_datetime`, weekday utilities, monthly pattern logic, and `advance_by`.

- **storage.rs** — SQLite persistence via rusqlite. `EventStorage` manages three tables: `chats` (id, chat_type, title, username, first_name, last_name, updated_at, created_at), `messages` (id, user_id, chat_id, created_at, message), and `events` (id, chat_id, date, time, year_explicit, message, active, next_datetime, created_at, days, repeat_interval, repeat_unit, in_offset, in_offset_unit, bare_hour, monthly_pattern, msg_id, years). `msg_id` in events is a NOT NULL foreign key referencing `messages(id)`. All types (`EventInfo`, `ChatInfo`, etc.) are imported from `types.rs`. Provides `open(path)` for file-backed DB and `open_in_memory()` for tests. `insert_event(&EventInfo)` serializes rich types to DB strings internally and persists the event. `update_schedule(id, active, next_datetime)` updates the schedule after each fire. `get_event(id)` returns an event by ID. `get_next_event(now)` returns the single nearest active event from `now`. `get_missed_events(now)` returns all active events whose `next_datetime` is before `now`. `get_events_at(dt)` returns all active events with the exact given `next_datetime`. `get_active_events()` and other getters deserialize DB rows back into `EventInfo` values.

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
