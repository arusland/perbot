# CLAUDE.md

## Workflow Rules

- `cargo fmt` and `cargo clippy` before running tests.
- Print a short commit message when work is done.
- Keep this file accurate after any feature/module/command/behavior change.
- **Adding a datetime format:** give it the same coverage as existing ones — locale vocabulary/regex builder (`locale/`, English in `locale/english.rs`), `parser.rs` extraction, `EventInfo::normalize_time` (canonical output), `scheduler.rs` (if math changes), the **Datetime formats** section here, unit tests, and USER/SYSTEM rows in `test-cases.md`. If it conflicts/overlaps with an existing format, stop and propose resolutions instead of silently changing behavior.
- **All user-facing time vocabulary goes through `LocaleProvider`.** Never hardcode a parsed/emitted time word in `parser.rs`/`types.rs`/`view/`. Exception: DB serialization uses the fixed canonical helpers in `types.rs`.

## Project Overview

Perbot is a Telegram reminder bot in Rust (edition 2024). A message leads with a natural-language time expression (`13:30 call the office`); the bot parses it, persists the event to SQLite, and fires the reminder when due. Active events reload on restart.

## Build & Test

```bash
cargo build [--release]
cargo test                 # parser/storage/scheduler/table tests
cargo test <name>          # single test/module, e.g. parser::tests
cargo run --bin bench      # storage benchmark (1000 events)
```

## Environment Variables

`TG_BOT_TOKEN` (bot API token, required), `TG_ADMIN_ID` (admin chat ID, i64, required), `RUST_LOG` (`flexi_logger` level), `LOG_DIR` (default `logs`).

## Key Invariants

- **Everything is HTML.** `EventInfo.message` is an HTML fragment (user's Telegram formatting preserved as tags; plain text escaped). All message-bearing output is `ParseMode::Html`; the only MarkdownV2 site is `TgBot::send_markdown` (`/exit` rejection). Only `< > &` need escaping (`teloxide::utils::html::escape`). Conversion happens at ingestion: `main` via `richtext::render_html`, `converter` escapes legacy text, snooze reuses stored HTML.
- **All outbound Telegram calls go through `TgBot`** (`tgbot.rs`), never the raw teloxide `Bot` (constructed only to clone into `TgBot::new` and for the dispatcher's updater). The wrapper owns the per-call `ParseMode` and logs payload size + chat/callback/file id on every call.
- **`TgMessage` is opaque to the sender**: final `text` (HTML) + optional `reply_markup`, forwarded verbatim by the sender task in `main`. Producers decide *which* view builder to call; **all composed message text and inline keyboards live only in `view/`** — `state.rs`, `commands/*`, `main.rs`, `pending.rs` never build message HTML or `InlineKeyboardMarkup` themselves (fixed literal one-liners like toasts and admin diagnostics stay at the call site).
- **List reads page in SQL** (`LIMIT`/`OFFSET` + paired `count_…` queries); a chat's full list is never loaded whole. `PageRequest` (`page`/`size`) in → `PageResponse` (`events` + `total`) out, count + page fetched under one provider lock.
- **Time/recurrence fields are mutually exclusive** — a message populates whichever one matched (see EventInfo table).
- **`normalize_time(loc)` is canonical and re-parseable** (idempotent): parses back to the same event under the same locale. Drives the edit/round-trip flow.
- **Localization is threaded, not global.** Every time-format input (parse) and time-bearing output (canonical string, recurrence description, relative time, date pattern, `Next launches:`) flows through an explicit `&dyn LocaleProvider`; `locale::for_chat(chat_id)` resolves it (English for every chat today). DB serialization is deliberately **not** localized — `storage.rs`/`converter.rs` persist weekday/unit strings via the fixed canonical free functions in `types.rs` (`day_to_str`, `parse_days`, `unit_from_str`, `TimeUnit::label`), which the English locale reuses.
- **Callback envelope** for event actions: `eid:<id>:<action>:<args>`. Every handler acting on an id **access-checks** `event.chat_id` against the pressing chat (callback ids are user-influenceable).
- **Pending flows are in-memory only** (restart drops them).

## Modules

`lib.rs` re-exports all modules. Shared types live in `types.rs`.

- **locale/** — Localization seam (`mod.rs` + `english.rs`). `LocaleProvider` threads all time vocabulary/regexes/format patterns; `locale::EN`/`for_chat` resolve the active locale. **Adding a locale = supplying data:** fill a `GrammarVocab` and call `TimeGrammar::build` (the shared builder owns the regex shapes), map words to the shared enums, provide output vocabulary/patterns/relative-time. A byte-identity test pins the built English regexes to the historical strings. User-facing name helpers (`weekday_full`/`ordinal_word`/`ordinal_suffix`/`weekday_abbrev_cap`) live here.
- **types.rs** — `EventInfo`, `MessageInfo`, `ChatInfo`/`ChatType` (`ChatInfo::from_chat` maps a teloxide `Chat`), `TgMessage`, `MessageSender`, `PageRequest`/`PageResponse`, time enums (`TimeUnit`/`Repetition`/`Ordinal`/`MonthlyPattern`), `NextSource`. The fixed canonical (storage) helpers live only here: `day_to_str`, `parse_days`, `unit_from_str`, `TimeUnit::label`, `NextSource::as_str`/`from_str`. `EventInfo::normalize_time(loc)` pulls all vocabulary from `loc`.
- **parser.rs** — Stateless extraction over the locale's regexes/word maps. `parse` / `parse_full` (also returns surviving body byte-ranges for `richtext`) / `parse_time_only` (time present, body empty → main's "send me the text" flow). Clock time matches anywhere; offset/bare-hour/short-date must lead. Standalone 4-digit token in 2000..=2100 is a year restriction. Body derived via `richtext::normalize`; `main` overwrites it with the HTML render before persisting.
- **richtext.rs** — Pure. `normalize` is the single source of truth for body normalization: collapses intra-line whitespace, **preserves line breaks verbatim**, tracks each char's source byte offset. `render_html` rebuilds `MessageEntity`s (UTF-16) over the leftover text via teloxide's `Renderer`; falls back to `html::escape`.
- **scheduler.rs** — Pure datetime math. `calc_next[_at](EventInfo[, now])` set `active` + `next_datetime` + `source` (`None` when inactive).
- **error.rs** — Crate error (`thiserror`) + `Result<T>`. Libraries use it; binaries wrap with `anyhow`.
- **storage.rs** — `EventStorage` over rusqlite. Tables `chats`, `messages`, `events` (`msg_id` NOT NULL FK; `legacy`/`snoozed` flags; `last_next_datetime`; `source` TEXT = `NextSource::as_str()`), and `missed_events` — the startup missed snapshot (autoincrement `id` preserves missed order; `event_id` UNIQUE FK `ON DELETE CASCADE`; `missed_at` = the datetime the event should have fired at, captured before rescheduling; cleared and repopulated every start). `get_missed_snapshot_by_chat` returns events with `missed_at` substituted into `next_datetime` (display-only — the missed list shows the missed moment, not the post-reschedule state). CRUD + paged active/range/missed-snapshot queries, `update_schedule` (persists a reschedule incl. source), `backup_to` (`VACUUM INTO`). `get_missed_events(now, limit)` batches the missed backlog with no offset — rescheduling a batch removes it from the predicate.
- **state.rs** — `EventProvider`: `Clone` handle over `Arc<Mutex<_>>` (storage + cached next event). **Storage-backed methods return `crate::error::Result`** and `?`-bubble DB errors; only `get_next_event` (in-memory cache) is infallible. `start(msg_tx)`: clears the `missed_events` table, `move_missed_events` records + reschedules the missed backlog in `MISSED_MOVE_BATCH`-sized batches (never whole in memory), sends each chat from `get_missed_chat_ids` page 0 via `get_missed_snapshot_events` (the same call backing the `ms:<page>` page-turn callbacks), then spawns a 1s poll thread that fires due events. A **startup** DB error returns `Err` before the thread spawns so `main` can abort; the detached thread logs per-tick errors in place. Fired messages are built by `view::fired_message`; the poll loop predicts the post-fire `active`/`source` with `calc_next_at` (before `update_and_reload` persists it) and hands the flags in so the dismiss row matches the state the buttons will act on. Mutations: `insert_event_and_get[_at]`, `insert_prebuilt_event` (no re-scheduling; importer + snooze), `update_event_and_get[_at]` (edit flow), `delete`, `dismiss(id, chat_id) -> DismissOutcome` (access-checks, reschedules at `next_datetime + 1s`; returns updated event / `Inactive` / `NotFound`), `dismiss_repetition` (steps `calc_next_at(_, prev + 1s)` until `source` is no longer `Repetition`, i.e. the next anchor, capped at 100 years; events with no anchor field — no short date or `monthly_pattern` — fall back to a single ordinary dismiss).
- **commands/** — One module per command (or group). `mod.rs`: `Command` (`/help`, `/events`, `/today`, `/tomorrow`, `/week`, `/month`, admin `/import <user_id>`/`/database`/`/logs`, hidden `/exit`), shared `CmdContext`, `Command::handle` dispatch, `pub use` re-exports (flat `commands::X` paths).
  - **list.rs** — paginated list commands. `fetch_page(kind, provider, chat_id, page)` returns one page + total (paging in storage); `handle_list`/`handle_list_callback` edit in place with `view::list_keyboard`; a page-turn to a vanished page clamps to the last page and refetches. `ListKind` itself (tags/titles/row styles) lives in `view/list.rs`.
  - **event.rs** — single-event view `/event<id>` (matched manually in `main` via `parse_event_command`, not in the menu): `handle_event_view` → `view::event_detail` + `view::event_actions_keyboard` (`⏭ Dismiss` only when active; `⏩ Dismiss repetition` only when active **and** `source` is `Repetition`). Callbacks decoded by `parse_event_callback`, dispatched by `handle_event_callback`: `dis` dismiss, `disr` dismiss-repetition, `sn` snooze, `del`/`delyes`/`delno` delete, `ed`/`edno` edit. Dismiss variants delegate to `EventProvider` and re-render the detail in place; delete/edit prompts edit keyboard/message in place. The `:n`-suffixed variants (`dis:n`/`disr:n`/`del:n`/`delyes:n`/`delno:n`) are the notification-keyboard flavor — they keep the fired text: dismiss rebuilds `view::notification_keyboard` from the advanced schedule (via `refresh_dismissed_view`), delete-confirm clears markup (toast only), delete-cancel restores the notification keyboard from the stored event's state.
  - **snooze.rs** — the `sn` buttons: inserts a one-off `snoozed` event reusing the original's HTML.
  - **cancel.rs** — `pm:` Cancel of the time-only flow. **help.rs**, **import.rs**, **database.rs**, **logs.rs**, **exit.rs** — one file per remaining command.
- **converter.rs** — Pure. Legacy MateBot `.alert` files (`OLD-SPEC.md`) → `EventInfo` (`legacy=true`). Future `lastActivePeriodTime` used directly; stale rolled forward; else `calc_next_at`. Unparsable inputs kept as inactive raw-text events.
- **import.rs** — Admin `/import` orchestration. `PendingImport` holds target chat between command and zip; `import_zip` converts each entry via `insert_prebuilt_event`, returns counts + HTML report.
- **pending.rs** — In-memory flow state only: `PendingMessage` (chat→body-less `EventInfo`, time-only flow) + `PendingEdit` (chat→event id, edit flow). Their prompt strings and `cancel_keyboard` live in `view/prompt.rs`.
- **view/** — The presentation layer (see Invariants): all output HTML + inline keyboards; every time-bearing helper takes `loc`. `mod.rs` re-exports everything flat (`view::X`); shared `test_support::sample_event` for the co-located tests.
  - **message.rs** — text primitives: `format_when`, `html_to_plain`/`message_preview` (strip+truncate, locale-free), the composed one-liners `unparsable_message`/`inactive_event_reply`, `MESSAGE_TRUNCATED`, and the **length clamp**: `rendered_len` (UTF-16 units — how Telegram counts) and `clamp_message` (cap at `MESSAGE_MAX_LEN` = 4096 − reserve, falling back to escaped plain text).
  - **event.rs** — `next_launches_preview` (≤3 upcoming; "" for one-off), `scheduled_message` (new parse + snooze), `event_when_line` (recurrence appended inside the relative `(…)` via `describe_recurrence`), `event_detail` (inactive events render a single bold "Event is out of date. Last fired at <`last_next_datetime`>" notice instead of when-line/launches), `event_source_input`/`edit_prompt` (re-parseable input as tap-to-copy `<code>`), and the event keyboards: `event_actions_keyboard`, `edit_cancel_keyboard`, `delete_confirm_keyboard`.
  - **list.rs** — `ListKind` (tag/title/empty/row-style per list; fetching stays in `commands::list`), `format_page_at` (renders an already-fetched page slice + total; `RowStyle` — `SingleLine`/`TwoLine`/`PreviewLink` — picks per-row layout; `/events` → two-line, hidden `Missed` kind (`ms` tag) → `missed-datetime — preview /event<id>` rows (absolute datetime only, no relative part; the snapshot carries the missed moment in `next_datetime`), rest → single-line), `list_keyboard` (`◀ Prev` / `<page>/<total>` no-op / `Next ▶`), `format_missed_page` (page 0 of the startup missed send), `total_pages`, `LIST_PAGE_SIZE`.
  - **notification.rs** — fired-reminder presentation: `fired_message(event, now, due, post_fire_active, post_fire_is_repetition, loc) -> TgMessage` (text = `<message><preview>\n\n<SNOOZE_HINT>`) and pub `notification_keyboard(id, active, is_repetition)`: snooze rows (callback `eid:<id>:sn:<minutes>`); a dismiss row when the event stays active post-fire — `eid:<id>:dis:n`, plus `eid:<id>:disr:n` when the upcoming `source` is `Repetition`; plus an Edit/Delete row — `eid:<id>:ed` / `eid:<id>:del:n`.
  - **prompt.rs** — pending-flow prompts (`ASK_TEXT`, `EDIT_ASK_TEXT`, `EDIT_NEED_TEXT`, `EDIT_NEED_TIME`) and `cancel_keyboard` (`CANCEL_DATA` = `pm:cancel`).
- **main.rs** — Entry point + teloxide `Dispatcher`. Startup clears stale command scopes then `set_my_commands`; a failing `provider.start(msg_tx)` notifies the admin and returns before the dispatcher is built. `message_handler` + `callback_handler` (routes `eid:`/`pm:`/list) return `anyhow::Result` and are wrapped by `*_safe` endpoints that catch errors, tell the user "Something goes wrong!", and forward detail to the admin. Holds the two text-completion flows: **time-only** (`PendingMessage` → render body → schedule) and **edit-completion** (`PendingEdit` → re-parse, copy identity fields, `update_event_and_get`). Applies `clamp_message` to reminder bodies at ingestion, warning with `view::MESSAGE_TRUNCATED`. Sender task is a dumb HTML pump.
- **tgbot.rs** — `TgBot`, the `Clone` logging wrapper every outbound call uses (see Invariants): `send_html`/`send_text`, `send_markdown`, `edit_html`/`edit_text`/`edit_markup`, `answer_callback` (`Option<text>` toast), `send_document` (`&Path` + optional name), `get_file`/`download_file`, `get_me`, `set_my_commands`/`delete_my_commands`.
- **logger.rs** — `init()` sets up `flexi_logger` (daily rotation to `LOG_DIR` + stdout); `current_log_path`.
- **bin/bench.rs** — Storage throughput benchmark.

## EventInfo fields

`EventInfo` (`types.rs`) carries one reminder end to end. Parser sets the first group; storage/scheduler fill the rest. Time/recurrence fields are largely mutually exclusive.

| Field | Type | Set by | Notes |
|-------|------|--------|-------|
| `date` | `Option<NaiveDate>` | parser | Short date (no year) → **yearly** (unless a non-year `repetition` makes it the start anchor). Full date → one-off (unless `every [N] year[s]`). |
| `time` | `Option<NaiveTime>` | parser | Clock time anywhere. Absent for `in_offset`/`bare_hour`. |
| `year_explicit` | `bool` | parser | `true` only when a full date spelled the year — honors `date`'s year vs rolling it forward yearly. |
| `days` | `Option<HashSet<Weekday>>` | parser | Weekday-set recurrence; pairs with `years`. |
| `years` | `Option<HashSet<i32>>` | parser | Standalone year token(s) 2000..=2100; restricts a `days` schedule. |
| `repetition` | `Option<Repetition>` | parser | `every <n> <unit>` interval. On a short date, a non-year repetition fills between the yearly anchors (which keep priority); a year-unit one is dropped as redundant. |
| `in_offset` | `Option<(u32, TimeUnit)>` | parser | Relative offset (`now + offset`); with `repetition` repeats. Exclusive with `time`/`date`. |
| `bare_hour` | `Option<u32>` | parser | Leading bare hour 0..=24 → next occurrence of that hour. |
| `monthly_pattern` | `Option<MonthlyPattern>` | parser | Ordinal weekday / last day / fixed `DayOfMonth`. With a `repetition`, the day-of-month anchor has priority. |
| `message` | `String` | parser → `main` | Body as **HTML fragment** (see Invariants). |
| `id` | `i64` | storage | PK; `0` before insert. |
| `chat_id` | `i64` | storage/caller | Destination chat. |
| `active` | `bool` | scheduler | `true` while a future occurrence remains. |
| `next_datetime` | `Option<NaiveDateTime>` | scheduler | Next fire; `None` → inactive. |
| `source` | `Option<NextSource>` | scheduler | Which field produced `next_datetime` (`Time`/`Date`/`BareHour`/`InOffset`/`Repetition`/`MonthlyPattern`/`Years`/`Weekdays`). `Some` iff `next_datetime` is `Some`. Persisted. |
| `last_next_datetime` | `Option<NaiveDateTime>` | scheduler | Most recent non-null `next_datetime`; retained when inactive (drives the "out of date / last fired at" notice). |
| `created_at` | `NaiveDateTime` | storage/converter | Insertion time (legacy: from `.alert` filename). |
| `msg_id` | `i64` | storage/caller | FK to originating `messages` row. |
| `legacy` | `bool` | converter | Imported from legacy `.alert`. |
| `snoozed` | `bool` | snooze flow | One-off snooze copy. |

## Test Cases

`test-cases.md` holds markdown tables driving `tests/table_tests.rs`. Rows alternate `USER` (parse + `insert_event_and_get_at`) and `SYSTEM` (`update_at_and_reload`, assert `next_datetime` or `NONE`). A USER Input starting with `!` is an **action row**: `!Dismiss` / `!Dismiss repetition` (case-insensitive) call `EventProvider::dismiss` / `dismiss_repetition` on the current event (columns 4–5 empty; the timestamp is unused — both advance from stored `next_datetime + 1s`); any other `!command` fails the table. Column 4 by actor: USER rows = expected `event.message`; SYSTEM rows = expected source as `source=<name>` (empty on `NONE` rows). Column 5 = expected `normalize_time()` (USER rows; empty when input doesn't parse). A literal `\n` in Input/Message decodes to a real newline. Add scenarios by appending `###` sections — no code changes needed.

## Datetime formats

- `13:23`, `5:24 PM`, `1:23 26.11`, `31.12.2027` — clock time anywhere; bare hour / offset / short date must lead. Minutes accept 1-2 digits (`10:6` → `10:06`).
- **Short date, no year** (`10:03 15.12`) → **yearly**; redundant `every year`/`yearly` absorbed (canonical `10:03 15.12 yearly`). **Full date + `every [N] year[s]`** → true yearly repetition (first fire on the date). **Full date alone** → one-off.
- **Short date + non-year repetition** (`11:07 05.11 every 2 days`) → date is the start anchor; the repetition fills between fires, but the **yearly date anchor has priority** (interval steps never skip it). Canonical keeps trailing `yearly`: `11:07 05.11 every 2 days yearly`.
- `13:45 mon-fri`, `13:25 thu-fri,sun 2023` — weekday sets, optional year. Leading `every` absorbed.
- `14:55 20.05 every 2 weeks`, `15:30 every 3 days` — start datetime then interval.
- `8 call Alex` → next 08:00; `24` → 00:00; `25` → invalid.
- `8 min call her`, `in 8 min every 2 hours test` — relative offset, optionally repeating; leading `in` absorbed (and canonical).
- `10:00 first sunday`, `17:00 3rd friday` — ordinal weekday (`1st`–`5th`, `last`) of the month.
- `18:00 last day of the month`, `18:00 last day` — last day of month.
- `12:05 28th of the month`, `every 28 of the month`, `each 5 of the month` — fixed calendar day (`1`–`31`); `of [the] month` required, optional `day`/ordinal/`every`/`each` absorbed. Missing days (Feb 31) skipped. Combinable with an interval (anchor has priority). Canonical `each <N><ord> day of the month`.
