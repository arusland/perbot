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

**Rich text / HTML rendering.** `EventInfo.message` holds the reminder body as an **HTML fragment** (the user's Telegram formatting — bold, italic, links, … — preserved as HTML tags; plain text is HTML-escaped). Every message-bearing output renders in `ParseMode::Html` (fired reminders, the `scheduled_message` confirmation, and the `/events`/`/today`/`/week`/`/month` lists + missed events), so there is no MarkdownV2/`escape_markdown` in the message path. Only `< > &` need escaping (via `teloxide::utils::html::escape`); bot-generated bits (datetimes, `•` bullets, `(1d)`) contain no HTML specials. Ingestion converts to HTML at the boundary: `main` renders the parsed body with `richtext::render_html`; `converter` escapes legacy text; snooze reuses the stored HTML.

- **types.rs** — Shared types: `EventInfo` (parsed + DB fields, incl. `legacy` and `snoozed` flags; `message` is an HTML fragment), `MessageInfo`, `ChatInfo`/`ChatType` (`FromStr`), `TgMessage` (`chat_id`, final `text` (always an HTML fragment), `reply_markup`: optional inline keyboard the producer chose; the sender task forwards both verbatim and always sends with `ParseMode::Html`), `MessageSender`, time enums (`TimeUnit`, `Repetition`, `Ordinal`, `MonthlyPattern`), and helpers `parse_days`, `unit_from_str`, `day_from_str`, `day_to_str` (weekday↔string mapping lives only here).

- **parser.rs** — `parse(text) -> Option<EventInfo>` (thin wrapper over `parse_full`). `parse_full(text) -> Option<(EventInfo, Vec<Range<usize>>)>` also returns the byte ranges of the input that compose the message body (a `Remaining` helper tracks which original spans survive each deletion), used by `richtext` to re-map formatting entities onto the leftover body. Both delegate to private `parse_components` (does the regex extraction; returns `None` only when **no** time component is found, body may be empty); `parse_full` adds the empty-body `None` guard. `parse_time_only(text) -> Option<EventInfo>` is the inverse guard: `Some` exactly when a time component is present but the body is empty (e.g. `13:30`, `8 min`) — drives `main`'s "send me the reminder text" flow. Stateless regex extraction; clock time matched anywhere, relative offset/bare hour/short date anchored to start. A standalone 4-digit token in 2000..=2100 is a year restriction. Rest of text becomes the (plain) message — the parser derives it via `richtext::normalize(input, &rem.spans)` (single source of truth for normalization: horizontal whitespace within a line collapses to single spaces, line breaks are preserved verbatim, so multiline messages keep their structure); `main` replaces it with the HTML rendering before persisting.

- **richtext.rs** — Pure, unit-tested. `pub(crate) normalize(input, spans) -> (String, Vec<OutChar>)` is the single source of truth for message-body normalization (also used by `parser` for the plain `message`): collapses intra-line horizontal whitespace to single spaces while **preserving line breaks verbatim** (one `\n` per source newline, so blank lines survive; leading/trailing whitespace dropped), tracking each output char's original byte offset (a join-space inherits the run's first whitespace offset; a preserved newline inherits the source newline's offset). `render_html(input, spans, entities) -> String` renders the surviving message body as an HTML fragment: runs `normalize`, rebuilds `MessageEntity`s (UTF-16 offsets) over the leftover text per the original `entities` (`Message::parse_entities`), and feeds them to teloxide's `utils::render::Renderer::as_html()`. With no applicable entity it returns `html::escape(message)`. A collapsed join-space (or preserved newline) inherits the run offset so a multi-word entity — even one spanning a line break — stays one run.

- **scheduler.rs** — Pure datetime math. `calc_next(EventInfo)` / `calc_next_at(EventInfo, now)` set `active` + `next_datetime`.

- **error.rs** — Crate error type (`thiserror`) + `Result<T>`. Library modules return this; binaries use `anyhow` on top.

- **storage.rs** — `EventStorage` over rusqlite, returning `crate::error::Result`. Tables `chats`, `messages`, `events` (`events.msg_id` NOT NULL FK; `legacy`/`snoozed` flags in original schema). `open(path)`/`open_in_memory()`. Events: `insert_event`, `get_event`, `get_by_chat`, `get_active_events`, `get_active_by_chat`, `get_active_by_chat_on_date`, `get_active_by_chat_in_range` (end exclusive), `get_next_event`, `get_missed_events`, `get_events_at`, `update_schedule`, `mark_inactive`, `delete`, `delete_inactive`. Chats: `upsert_chat`, `get_chat`, `get_all_chats`. Messages: `insert_message`.

- **state.rs** — `EventProvider`: `Clone` handle over `Arc<Mutex<EventProviderState>>` (storage + cached `next_event`). `start(msg_tx)` sends missed events (no keyboard) then spawns a 1s poll thread firing due events (each built here with a "Next launches" preview and the `SNOOZE_HINT` appended and `snooze_keyboard(id)` in `reply_markup`) and rescheduling. The fired message text is `<message><preview>\n\n<SNOOZE_HINT>` (sent as HTML by the sender task), where `message` is the event's HTML fragment and the preview/hint are plain text wrapped with `html::escape`; the preview is `telegram::next_launches_preview(event, fire_dt)`. Snooze presentation (`SNOOZE_OPTIONS`/`SNOOZE_HINT`/`snooze_keyboard`, callback `eid:<id>:sn:<minutes>`: 1/5/10/30 min, 1/2/8 h, 1 day) lives here, next to the only code that uses it. Wraps storage plus `insert_event_and_get[_at]`, `insert_prebuilt_event` (insert already-scheduled event without re-scheduling; used by importer and snooze), `update_at_and_reload`.

- **commands.rs** — `Command` enum (`BotCommands`): `/help`, `/events`, `/today`, `/tomorrow`, `/week`, `/month`, admin-only `/import <user_id>`, `/database`, and `/exit` (hidden). `CmdContext` bundles deps; `Command::handle(ctx)` dispatches. All five list commands are paginated via private `ListKind` (tags `ev`/`td`/`tm`/`wk`/`mo`; date ranges recomputed relative to now). `handle_list` renders page 0 (`telegram::format_page_at`, `LIST_PAGE_SIZE`/page) with a `◀ Prev`/`Next ▶` keyboard (callback `<tag>:<page>`); on send failure it warns admin instead of bubbling up. `handle_list_callback` decodes the tag, re-fetches, edits in place. `handle_import_zip` downloads the zip, runs `import::import_zip`, replies with summary + HTML report. `handle_database` (admin-only; "Not authorized." otherwise) snapshots the live DB via `EventStorage::backup_to` (`VACUUM INTO` to a temp file) and sends it as a `perbot.db` document, then deletes the snapshot. List replies use `ParseMode::Html` (titles `<b>…:</b>`, rows `• <when> — <message HTML>`); `/help` and `/import` are plain text. **Snooze:** fired reminders carry the snooze keyboard + hint, built by the producer in `state.rs` (keyboard, callback `eid:<id>:sn:<minutes>`, and hint live in `state.rs`). Event-specific callbacks use the reusable `eid:<id>:<action>:<args>` envelope. `parse_snooze_callback` decodes the data to `(event_id, minutes)`; `handle_snooze_callback` loads the event by id (`provider.get_event`), **access-checks** that `event.chat_id` matches the chat the button was pressed in (rejects otherwise), uses the stored `event.message` as the title, and inserts a one-off `snoozed_event` at `now + minutes` via `insert_message` + `insert_prebuilt_event`; original event untouched. On success it sends the shared `telegram::scheduled_message(now, next, &event)` confirmation (HTML) to the chat (one-off snooze → no preview). The snoozed copy reuses the original event's HTML `message`, so it keeps the user's formatting. **Cancel pending:** `handle_cancel_pending` (routed from `main`'s `pm:`-prefixed callbacks) drops the chat's entry in `PendingMessage` and edits the prompt to "Cancelled."

- **converter.rs** — Pure, unit-tested conversion of legacy MateBot `.alert` files (see `OLD-SPEC.md`) into `EventInfo` (legacy=true). `created_at_from_filename` parses `YYYYMMDD_HHMMSS_mmm.alert`; `extract_input` handles plain-text vs JSON; `convert(...)` maps the old grammar onto `EventInfo`. Future `lastActivePeriodTime` used directly; stale one rolled forward (flagged `recalculated`); else `scheduler::calc_next_at`. Unparsable inputs kept as inactive raw-text events.

- **import.rs** — Admin `/import` orchestration. `PendingImport = Arc<Mutex<Option<i64>>>` holds the target chat between `/import <user_id>` and the zip. `import_zip(provider, target, bytes)` converts each `.alert` entry, upserts the chat, inserts a synthetic message + event via `insert_prebuilt_event`, returns counts + an HTML old→new report.

- **pending.rs** — In-memory "send me the reminder text" flow for time-only messages. `PendingMessage = Arc<Mutex<HashMap<i64, EventInfo>>>` (keyed by chat id) holds a parsed-but-body-less `EventInfo` between a time-only message and the follow-up text; `new_pending()` constructs it. `ASK_TEXT` is the prompt; `CANCEL_DATA` (`pm:cancel`) + `cancel_keyboard()` are the single-button Cancel keyboard. State is in-memory only (a restart drops pending requests).

- **main.rs** — Entry point + teloxide `Dispatcher` + wiring. At startup clears stale commands from non-default scopes then `set_my_commands`. Two branches share deps via `dptree::deps!`: `message_handler` and `callback_handler` (routes `eid:` → `handle_snooze_callback`, `pm:` → `handle_cancel_pending`, else `handle_list_callback`). The sender task is a dumb pump: it sends each `TgMessage`'s `text` with `ParseMode::Html` and attaches its `reply_markup` if present, deciding nothing about buttons (the producer in `state.rs` does). `message_handler`: upserts chat, computes `is_admin`; admin document during pending import → `handle_import_zip`; else store message, `Command::parse` → `cmd.handle(ctx)`. **Time-only flow** (via `PendingMessage`): a non-command text for a chat that already has a pending entry is rendered (HTML) and used verbatim as the body, then scheduled (a whitespace-only reply re-prompts with `ASK_TEXT` + `cancel_keyboard`); otherwise `parser::parse_full` → set `event.message = richtext::render_html(text, &spans, &msg.parse_entities())` → `insert_event_and_get` + the HTML `scheduled_message` confirmation; otherwise `parser::parse_time_only` → store the body-less event in `PendingMessage` and reply with `ASK_TEXT` + `cancel_keyboard`; otherwise the unparsable fallback. Fallback/unparsable replies and the startup banner are HTML too.

- **telegram.rs** — All output is HTML (`teloxide::utils::html::escape` for user/bot text; `event.message` embedded verbatim as an HTML fragment). `format_when(now, dt)` → plain-text `HH:MM dd.mm.yyyy (relative)` for a single datetime; `next_launches_preview(event, after)` walks `scheduler::calc_next_at` forward from `after` to list up to `MAX_NEXT_PREVIEW` (3) upcoming launches as `• <format_when>` bullets, plus a trailing `• ...` when more remain (empty string for one-off events); returns plain text (callers `html::escape` it, a no-op for these chars). `scheduled_message(now, dt, event)` → shared `Scheduled message for <b><format_when></b>` confirmation (bolded absolute datetime + relative time from `now`, e.g. `13:30 22.06.2026 (1d)`; used on new parse in `main.rs` and on snooze) followed by a `Message: <event.message>` line (HTML fragment), with the `next_launches_preview` appended for recurring events; `format_page[_at](...)` → `(text, total_pages)` HTML paginated lists (title `<b>…(page x/y):</b>`, `(page x/y)` only when >1 page; rows `• <when> — <message>`). Helpers `LIST_PAGE_SIZE`, `total_pages`. `extract_chat_info(chat)` → `ChatInfo`.

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
| `message` | `String` | parser → `main`/`richtext` | Remainder after extracting time/date components, as an **HTML fragment** (user formatting preserved, plain text escaped). The parser fills it with the plain leftover (via `richtext::normalize`, which collapses intra-line whitespace but **preserves line breaks**, so multiline messages keep their structure); `main` replaces it with `richtext::render_html` before persisting. Sent to the user in HTML. |
| `id` | `i64` | storage | DB primary key; `0` before insert, real id after. |
| `chat_id` | `i64` | storage/caller | Destination chat; set when associating the event with a chat. |
| `active` | `bool` | scheduler | `true` while the event still has a future occurrence; `mark_inactive` / scheduling clears it when exhausted. |
| `next_datetime` | `Option<NaiveDateTime>` | scheduler | Next fire time computed by `calc_next[_at]`; `None` when no future occurrence (event becomes inactive). |
| `created_at` | `NaiveDateTime` | storage/converter | Insertion time; for legacy imports parsed from the `.alert` filename. |
| `msg_id` | `i64` | storage/caller | FK to the originating `messages` row (`events.msg_id` NOT NULL). |
| `legacy` | `bool` | converter | `true` for events imported from legacy MateBot `.alert` files. |
| `snoozed` | `bool` | snooze flow | `true` for one-off events created by `handle_snooze_callback`. |

## Test Cases

`test-cases.md` holds markdown tables driving `tests/table_tests.rs`. Rows alternate `USER` (parse + `insert_event_and_get_at`) and `SYSTEM` (`update_at_and_reload`, assert `next_datetime` or `NONE`). A 4th column carries the expected reminder message: on `USER` rows it must equal the parsed `event.message` (asserted); on `SYSTEM` rows it is empty. A literal `\n` in the Input or Message column is decoded to a real newline by the harness (`unescape`), so multiline messages can be expressed in a single table cell. Add scenarios by appending `###` sections — no code changes needed.

## Datetime formats supported
- `13:23`, `5:24 PM`, `1:23 26.11`, `31.12.2027` — clock time anywhere; bare hour / relative offset / short date must lead. Minutes accept 1-2 digits, so `10:6` means `10:06` (`9:5 PM` → 21:05).
- `13:45 mon-fri`, `13:25 thu-fri,sun 2023` — weekday sets, optional year. An optional leading `every` is absorbed, so `10:30 every fri` is identical to `10:30 fri`.
- `14:55 20.05 every 2 weeks`, `15:30 every 3 days` — start datetime then repeat interval.
- `8 call Alex` → next 08:00; `24 ...` → 00:00; `25 ...` → invalid (not parsed).
- `8 min call her`, `8 min every hour` — relative offset, optionally repeating.
- `10:00 first sunday`, `9:30 last monday`, `17:00 3rd friday` — ordinal weekday (`1st`–`5th`, `last`) of the month.
- `18:00 last day of the month`, `18:00 last day` — last day of month.
