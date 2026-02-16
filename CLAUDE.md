# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Perbot is a Telegram reminder bot written in Rust. Users send messages containing natural language time/date expressions (e.g., "13:30 call the office"), the bot parses the datetime, schedules an async task, persists the event to SQLite, and sends the reminder when the time arrives. On restart, pending events are reloaded and rescheduled.

## Build & Test Commands

```bash
cargo build                  # Debug build
cargo build --release        # Release build
cargo test                   # Run all tests (28 tests across parser and storage)
cargo test parser::tests     # Run only parser tests
cargo test storage::tests    # Run only storage tests
cargo test <test_name>       # Run a single test by name
```

## Environment Variables

- `TELOXIDE_TOKEN` — Telegram bot API token (required)
- `TG_ADMIN_ID` — Admin chat ID for startup notification (required, i64)
- `RUST_LOG` — Log level for `pretty_env_logger` (e.g., `info`, `debug`)

## Architecture

Three source files in `src/`:

- **main.rs** — Bot entry point. Initializes teloxide REPL, loads pending events from SQLite on startup and reschedules them via `tokio::spawn` + `tokio::time::sleep`. Handles incoming messages: parses text for datetime, stores events, spawns delayed send tasks, marks events fired. All responses use MarkdownV2 parse mode (escaped via `escape_markdown`). Storage is shared as `Arc<Mutex<EventStorage>>`.

- **parser.rs** — Stateless datetime extraction. `parse(text) -> Option<ParsedEvent>` uses regex to extract time from the beginning of a message; remainder becomes the event message. `resolve_datetime(&ParsedEvent) -> Option<NaiveDateTime>` resolves to a future datetime (advances to next day/year if the parsed time/date is in the past and no explicit year was given).

- **storage.rs** — SQLite persistence via rusqlite. `EventStorage` manages two tables: `events` (id, chat_id, date, time, year_explicit, message, target_datetime, created_at, fired) and `chats` (id, chat_type, title, username, first_name, last_name, updated_at). Provides `open(path)` for file-backed DB and `open_in_memory()` for tests.

## Datetime formats supported
- `13:23`, `5:24 PM`, `1:23 26.11`, `31.12.2027` — always at the start of the message.
- `13:45 mon-fri` — every Monday, Tuesday, Wednesday, Thursday, and Friday at 13:45.
- `13:25 thu-fri,sun 2023` — every Thursday, Friday, and Sunday in 2023 at 13:25.
- `14:55 20.05 every 2 weeks` - start at 14:55 every year on date 20.05 and then repeat every 2 weeks.
- `15:30 every 3 days` — start at next at 15:30 and then repeat every 3 days.

## TODO:
- `8 call Alex` - fire event next 08:00 (from current time)
- `24 call Poly` - fire event next 00:00
- `25 call Alex` - do not parse any time
- `8 min call her` - fire event in 8 minutes 
- `8 min every hour` - fire event in 8 minutes and repeat every hour
