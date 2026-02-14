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

- **parser.rs** — Stateless datetime extraction. `parse(text) -> Option<ParsedEvent>` uses regex to extract time (24h/12h AM/PM) and date (DD.MM / DD.MM.YYYY) from the beginning of a message; remainder becomes the event message. `resolve_datetime(&ParsedEvent) -> Option<NaiveDateTime>` resolves to a future datetime (advances to next day/year if the parsed time/date is in the past and no explicit year was given).

- **storage.rs** — SQLite persistence via rusqlite. `EventStorage` manages two tables: `events` (id, chat_id, date, time, year_explicit, message, target_datetime, created_at, fired) and `chats` (id, chat_type, title, username, first_name, last_name, updated_at). Provides `open(path)` for file-backed DB and `open_in_memory()` for tests.

## Key Patterns

- Datetime formats supported: `13:23`, `5:24 PM`, `1:23 26.11`, `31.12.2027` — always at the start of the message.
- Period and repetition fields exist in `ParsedEvent` but are not yet implemented (see SPEC.md).
- The database file `events.db` is created at runtime in the project root and is gitignored.
- Tests use in-memory SQLite (`EventStorage::open_in_memory()`), no external services needed.
