# Perbot

A Telegram reminder bot written in Rust. Send it a message that starts
with a natural-language time expression — like `13:30 call the office` — and it parses
the datetime, schedules the reminder, persists it to SQLite, and pings you back when the
time arrives. Active events survive restarts: on startup they are reloaded and
rescheduled, and any reminders missed while the bot was down are delivered immediately.

## Features

- Natural-language time parsing at the start of a message (AI not used for now).
- One-off, daily, weekly, interval, and monthly-pattern reminders.
- Relative reminders (`8 min call her`) and bare-hour shorthand (`8 call Alex`).
- Persistent storage in SQLite — events, chats, and messages.
- Crash/restart safe: active events are reloaded and missed reminders are sent on boot.
- Batched delivery when multiple events fire at the same moment.
- All replies use Telegram MarkdownV2 (special characters escaped).

## Supported datetime formats

| Example | Meaning |
|---------|---------|
| `13:23` | Today (or next day) at 13:23 |
| `5:24 PM` | 12-hour clock |
| `1:23 26.11` | Time on a specific date |
| `31.12.2027` | A specific date |
| `13:45 mon-fri` | Every weekday at 13:45 |
| `13:25 thu-fri,sun 2023` | Selected weekdays within a given year |
| `14:55 20.05 every 2 weeks` | Start on a date, then repeat every 2 weeks |
| `15:30 every 3 days` | Start at next 15:30, then every 3 days |
| `8 call Alex` | Next 08:00 (bare hour) |
| `24 call Poly` | Next 00:00 |
| `25 call Alex` | Not parsed — invalid bare hour |
| `8 min call her` | In 8 minutes |
| `8 min every hour` | In 8 minutes, then every hour |
| `10:00 first sunday call mom` | First Sunday of the month at 10:00 |
| `9:30 last monday team sync` | Last Monday of the month at 9:30 |
| `14:00 second thursday board meeting` | Second Thursday of the month |
| `17:00 3rd friday happy hour` | Ordinals: `1st`, `2nd`, `3rd`, `4th`, `5th` |
| `18:00 last day of the month pay rent` | Last day of the month |
| `18:00 last day pay bills` | "of the month" is optional |

The time/date expression must always come at the start of the message; the remaining
text becomes the reminder body.

## Getting started

### Prerequisites

- Rust toolchain with edition 2024 support.
- A Telegram bot token from [@BotFather](https://t.me/BotFather).

### Configuration

Set the following environment variables:

| Variable | Required | Description |
|----------|----------|-------------|
| `TG_BOT_TOKEN` | yes | Telegram bot API token |
| `TG_ADMIN_ID` | yes | Admin chat ID (i64) for the startup notification |
| `RUST_LOG` | no | Log level filter for `flexi_logger` (e.g. `info`, `debug`) |
| `LOG_DIR` | no | Directory for log files (defaults to `logs`) |

### Build & run

```bash
cargo build --release
TG_BOT_TOKEN=... TG_ADMIN_ID=... RUST_LOG=info cargo run --release
```

The admin can send `exit` in chat to shut the bot down gracefully.

## Development

```bash
cargo build                    # Debug build
cargo test                     # Run all tests (parser, storage, scheduler)
cargo test parser::tests       # Parser tests only
cargo test storage::tests      # Storage tests only
cargo test scheduler::tests    # Scheduler tests only
cargo run --bin bench          # Storage benchmark (1000 events)
```

## Testing

Integration tests in `tests/table_tests.rs` are data-driven from `test-cases.md`. Each
markdown table is a scenario whose rows alternate between `USER` actions (a parsed and
inserted chat message) and `SYSTEM` actions (advance time and assert the next fire time,
or `NONE` when the event becomes inactive). New scenarios can be added by appending
tables to `test-cases.md` — no code changes required.

## License

Licensed under the [Apache License 2.0](LICENSE).
