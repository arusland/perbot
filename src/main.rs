use perbot::parser::EventInfo;
use perbot::storage::{ChatInfo, ChatType, EventStorage, MessageInfo};
use perbot::{parser, scheduler};
use std::process;
use std::sync::{Arc, Mutex};
use teloxide::{prelude::*, types::ParseMode};

struct TopEvents {
    events: Vec<EventInfo>,
    abort_handle: Option<tokio::task::AbortHandle>,
}

impl TopEvents {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            abort_handle: None,
        }
    }
}

type TopEventsState = Arc<Mutex<TopEvents>>;

#[tokio::main]
async fn main() {
    init_logger();
    log::info!("Starting bot...");

    let bot = Bot::from_env();

    let storage = Arc::new(Mutex::new(
        EventStorage::open("perbot.db").expect("Failed to open database"),
    ));

    let top_state: TopEventsState = Arc::new(Mutex::new(TopEvents::new()));

    // Load top events from storage on startup
    {
        let storage_guard = storage.lock().unwrap();
        let now = chrono::Local::now().naive_local();
        match storage_guard.get_top_events(now) {
            Ok(events) => {
                log::info!("Loaded {} top events from storage", events.len());
                top_state.lock().unwrap().events = events;
            }
            Err(e) => log::error!("Failed to load top events: {}", e),
        }
    }

    schedule_first_event(bot.clone(), Arc::clone(&storage), Arc::clone(&top_state));

    let admin_id = ChatId(
        std::env::var("TG_ADMIN_ID")
            .expect("ADMIN_ID environment variable not set")
            .parse::<i64>()
            .expect("TG_ADMIN_ID must be a valid i64"),
    );

    bot.send_message(admin_id, "Bot started").await.unwrap();

    let handler_storage = Arc::clone(&storage);
    let handler_top_state = Arc::clone(&top_state);
    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let storage = Arc::clone(&handler_storage);
        let top_state = Arc::clone(&handler_top_state);
        async move {
            println!("msg: {:?}\nkind: {:?}", msg.chat, msg.chat.kind);

            // Save/update chat info
            {
                let chat_info = extract_chat_info(&msg.chat);
                let storage_guard = storage.lock().unwrap();
                if let Err(e) = storage_guard.upsert_chat(&chat_info) {
                    log::error!("Failed to save chat info: {}", e);
                }
            }

            let reply_text = if let Some(text) = msg.text() {
                // Store every incoming user message
                let user_id = msg.from.as_ref().map(|u| u.id.0 as i64);
                let msg_id = {
                    let storage_guard = storage.lock().unwrap();
                    let msg_info = MessageInfo {
                        id: 0,
                        user_id,
                        chat_id: msg.chat.id.0,
                        created_at: None,
                        message: text.to_string(),
                    };
                    match storage_guard.insert_message(&msg_info) {
                        Ok(id) => id,
                        Err(e) => {
                            log::error!("Failed to save message: {}", e);
                            return Ok(());
                        }
                    }
                };

                if text == "exit" && user_id == Some(admin_id.0) {
                    log::info!("Received exit command. Shutting down...");
                    let _ = bot.send_message(admin_id, "Shutting down...").await;
                    tokio::spawn(async {
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        process::exit(0);
                    });

                    return Ok(());
                }

                if let Some(parsed) = parser::parse(text) {
                    let mut event = parsed;
                    event.chat_id = msg.chat.id.0;
                    event.msg_id = msg_id;
                    let stored = scheduler::calc_next(event);

                    // Save event to storage
                    {
                        let storage_guard = storage.lock().unwrap();
                        match storage_guard.insert_event(&stored) {
                            Ok(id) => log::info!("Saved event with id: {}", id),
                            Err(e) => {
                                log::error!("Failed to save event: {}", e);
                                return Ok(());
                            }
                        }
                    }

                    if let Some(dt) = stored.next_datetime {
                        println!("next_datetime: {:?}", dt);

                        // Reload top events and reschedule
                        reload_and_schedule(
                            bot.clone(),
                            Arc::clone(&storage),
                            Arc::clone(&top_state),
                        );

                        format!("Scheduled message for {}", dt.format("%H:%M %d\\.%m\\.%Y"))
                    } else {
                        format!("*{}*", escape_markdown(text))
                    }
                } else {
                    format!("*{}*", escape_markdown(text))
                }
            } else if msg.photo().is_some() {
                "Received a photo\\!".to_string()
            } else if msg.video().is_some() {
                "Received a video\\!".to_string()
            } else if msg.audio().is_some() {
                "Received an audio file\\!".to_string()
            } else if msg.voice().is_some() {
                "Received a voice message\\!".to_string()
            } else if msg.document().is_some() {
                "Received a document\\!".to_string()
            } else if msg.sticker().is_some() {
                "Received a sticker\\!".to_string()
            } else if msg.animation().is_some() {
                "Received an animation\\!".to_string()
            } else if msg.video_note().is_some() {
                "Received a video note\\!".to_string()
            } else if msg.contact().is_some() {
                "Received a contact\\!".to_string()
            } else if msg.location().is_some() {
                "Received a location\\!".to_string()
            } else if msg.venue().is_some() {
                "Received a venue\\!".to_string()
            } else if msg.poll().is_some() {
                "Received a poll\\!".to_string()
            } else if msg.dice().is_some() {
                "Received a dice\\!".to_string()
            } else {
                "Received an unknown message type\\!".to_string()
            };

            bot.send_message(msg.chat.id, reply_text)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;

            Ok(())
        }
    })
    .await;
}

/// Reloads top events from DB and schedules the first one.
fn reload_and_schedule(bot: Bot, storage: Arc<Mutex<EventStorage>>, top_state: TopEventsState) {
    {
        let storage_guard = storage.lock().unwrap();
        let now = chrono::Local::now().naive_local();
        match storage_guard.get_top_events(now) {
            Ok(events) => {
                log::info!("Reloaded {} top events", events.len());
                top_state.lock().unwrap().events = events;
            }
            Err(e) => log::error!("Failed to reload top events: {}", e),
        }
    }
    schedule_first_event(bot, storage, top_state);
}

/// Schedules the first event from the top events list.
/// Cancels any previously scheduled task, then spawns a new delayed task
/// for the earliest event. After firing, recalculates next datetime,
/// saves to DB, updates the in-memory list, and schedules the next first event.
fn schedule_first_event(bot: Bot, storage: Arc<Mutex<EventStorage>>, top_state: TopEventsState) {
    let mut top = top_state.lock().unwrap();

    // Cancel current scheduled task
    if let Some(handle) = top.abort_handle.take() {
        log::info!("Aborting previously scheduled task");
        handle.abort();
    }

    let Some(event) = top.events.first().cloned() else {
        log::info!("No top events to schedule");
        return;
    };

    let Some(dt) = event.next_datetime else {
        return;
    };

    let chat_id = ChatId(event.chat_id);
    let message = event.message.clone();
    let top_clone = Arc::clone(&top_state);
    let storage_clone = Arc::clone(&storage);

    let join_handle = tokio::spawn(async move {
        let now = chrono::Local::now().naive_local();
        let delay_secs = dt.signed_duration_since(now).num_seconds().max(0) as u64;
        let event_id = event.id;

        log::info!(
            "Scheduling event {} for {:?} (in {})",
            event_id,
            dt,
            format_duration(delay_secs)
        );

        tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;
        let now = chrono::Local::now().naive_local();
        // TODO: make separate thread for sending messages
        if let Err(e) = bot.send_message(chat_id, &message).await {
            log::error!("Failed to send message for event {}: {}", event_id, e);
        }

        // Recalc next datetime and save to DB
        let next = scheduler::calc_next_at(event, now);
        {
            let Ok(storage_guard) = storage_clone.lock() else {
                log::error!("Failed to lock storage after event fired");
                return;
            };
            if let Err(e) = storage_guard.update_schedule(event_id, next.active, next.next_datetime)
            {
                log::error!("Failed to update schedule for event {}: {}", event_id, e);
            }
        }

        // Update in-memory top events list
        {
            let mut top = top_clone.lock().unwrap();
            top.abort_handle = None;
            // Remove fired event
            top.events.retain(|e| e.id != event_id);
            // If still active, insert back sorted by next_datetime
            if next.active {
                let pos = top
                    .events
                    .iter()
                    .position(|e| e.next_datetime > next.next_datetime)
                    .unwrap_or(top.events.len());
                top.events.insert(pos, next);
            }
        }

        // If list is empty, reload from DB
        let is_empty = top_clone.lock().unwrap().events.is_empty();
        if is_empty {
            let events = {
                let Ok(storage_guard) = storage_clone.lock() else {
                    log::error!("Failed to lock storage for reload");
                    return;
                };
                storage_guard.get_top_events(now)
            };
            match events {
                Ok(events) => {
                    log::info!("Reloaded {} top events after list emptied", events.len());
                    top_clone.lock().unwrap().events = events;
                }
                Err(e) => log::error!("Failed to reload top events: {}", e),
            }
        }

        // Schedule the next first event
        schedule_first_event(bot, storage_clone, top_clone);
    });

    top.abort_handle = Some(join_handle.abort_handle());
}

fn format_duration(total_secs: u64) -> String {
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{} day{}", days, if days == 1 { "" } else { "s" }));
    }
    if hours > 0 {
        parts.push(format!(
            "{} hour{}",
            hours,
            if hours == 1 { "" } else { "s" }
        ));
    }
    if minutes > 0 {
        parts.push(format!("{} min", minutes));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{} sec", seconds));
    }
    parts.join(" ")
}

fn escape_markdown(text: &str) -> String {
    let special_chars = [
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];
    let mut result = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if special_chars.contains(&c) {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

fn extract_chat_info(chat: &teloxide::types::Chat) -> ChatInfo {
    use teloxide::types::{ChatKind, PublicChatChannel, PublicChatKind, PublicChatSupergroup};

    let (chat_type, title, username, first_name, last_name) = match &chat.kind {
        ChatKind::Private(private) => (
            ChatType::Private,
            None,
            private.username.clone(),
            private.first_name.clone(),
            private.last_name.clone(),
        ),
        ChatKind::Public(public) => {
            let (chat_type, username) = match &public.kind {
                PublicChatKind::Group => (ChatType::Group, None),
                PublicChatKind::Supergroup(PublicChatSupergroup { username, .. }) => {
                    (ChatType::Supergroup, username.clone())
                }
                PublicChatKind::Channel(PublicChatChannel { username, .. }) => {
                    (ChatType::Channel, username.clone())
                }
            };
            (chat_type, public.title.clone(), username, None, None)
        }
    };

    ChatInfo {
        id: chat.id.0,
        chat_type,
        title,
        username,
        first_name,
        last_name,
        updated_at: None,
        created_at: None,
    }
}

fn init_logger() {
    let log_dir = std::env::var("LOG_DIR").unwrap_or_else(|_| "logs".to_string());
    flexi_logger::Logger::try_with_env_or_str("info")
        .expect("Failed to initialize logger")
        .log_to_file(flexi_logger::FileSpec::default().directory(&log_dir))
        .rotate(
            flexi_logger::Criterion::Age(flexi_logger::Age::Day),
            flexi_logger::Naming::Timestamps,
            flexi_logger::Cleanup::KeepLogFiles(365),
        )
        .format_for_files(log_format)
        .format_for_stdout(log_format)
        .duplicate_to_stdout(flexi_logger::Duplicate::All)
        .start()
        .expect("Failed to start logger");
}

fn log_format(
    w: &mut dyn std::io::Write,
    now: &mut flexi_logger::DeferredNow,
    record: &log::Record,
) -> std::io::Result<()> {
    write!(
        w,
        "[{}] {:5} [{}:{}] {}",
        now.format("%Y-%m-%d %H:%M:%S"),
        record.level(),
        record.module_path().unwrap_or("<unknown>"),
        record.line().unwrap_or(0),
        record.args()
    )
}
