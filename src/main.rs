use perbot::parser::EventInfo;
use perbot::storage::{ChatInfo, ChatType, EventStorage, MessageInfo};
use perbot::{parser, scheduler};
use std::process;
use std::sync::{Arc, Mutex};
use teloxide::{prelude::*, types::ParseMode};

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    log::info!("Starting bot...");

    let bot = Bot::from_env();

    let storage = Arc::new(Mutex::new(
        EventStorage::open("perbot.db").expect("Failed to open database"),
    ));

    // Load and reschedule active events from storage
    let active_events = {
        let storage_guard = storage.lock().unwrap();
        storage_guard.get_active_events()
    };

    match active_events {
        Ok(events) => {
            log::info!("Loading {} active events from storage", events.len());
            for event in events {
                let event_id = event.id;
                log::info!(
                    "Rescheduled event {} for {:?}",
                    event_id,
                    event.next_datetime
                );
                schedule_event(bot.clone(), event_id, event, Arc::clone(&storage));
            }
        }
        Err(e) => log::error!("Failed to load active events: {}", e),
    }

    let admin_id = ChatId(
        std::env::var("TG_ADMIN_ID")
            .expect("ADMIN_ID environment variable not set")
            .parse::<i64>()
            .expect("TG_ADMIN_ID must be a valid i64"),
    );

    bot.send_message(admin_id, "Bot started").await.unwrap();

    let handler_storage = Arc::clone(&storage);
    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let storage = Arc::clone(&handler_storage);
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
                        // TODO: Implement proper shutdown logic
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        process::exit(0);
                    });

                    // Return Ok - this acknowledges the update to Telegram
                    return Ok(());
                }

                if let Some(parsed) = parser::parse(text) {
                    let mut event = parsed;
                    event.chat_id = msg.chat.id.0;
                    event.msg_id = msg_id;
                    let stored = scheduler::calc_next(event);

                    // Save event to storage
                    let event_id = {
                        let storage_guard = storage.lock().unwrap();
                        storage_guard.insert_event(&stored)
                    };

                    let event_id = match event_id {
                        Ok(id) => {
                            log::info!("Saved event with id: {}", id);
                            id
                        }
                        Err(e) => {
                            log::error!("Failed to save event: {}", e);
                            return Ok(());
                        }
                    };

                    if let Some(dt) = stored.next_datetime {
                        println!("next_datetime: {:?}", dt);
                        schedule_event(bot.clone(), event_id, stored, Arc::clone(&storage));
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

/// Spawns a delayed task that sends the event message when due, then calls
/// `calc_next` to compute the next occurrence and saves the result to the database.
/// If the event is still active after `calc_next`, a new task is spawned recursively.
fn schedule_event(bot: Bot, event_id: i64, event: EventInfo, storage: Arc<Mutex<EventStorage>>) {
    let Some(dt) = event.next_datetime else {
        return;
    };
    let now = chrono::Local::now().naive_local();
    let delay_secs = dt.signed_duration_since(now).num_seconds().max(0) as u64;
    let chat_id = ChatId(event.chat_id);
    let message = event.message.clone();

    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;
        let _ = bot.send_message(chat_id, &message).await;

        // Compute next occurrence and persist it
        let next = scheduler::calc_next(event);
        {
            let Ok(storage_guard) = storage.lock() else {
                return;
            };
            let _ = storage_guard.update_schedule(event_id, next.active, next.next_datetime);
        }

        // Schedule the next occurrence if still active
        if next.active {
            schedule_event(bot, event_id, next, storage);
        }
    });
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
