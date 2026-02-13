mod parser;
mod storage;

use std::process;
use std::sync::{Arc, Mutex};
use storage::{ChatInfo, ChatType, EventStorage};
use teloxide::{prelude::*, types::ParseMode};

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    log::info!("Starting bot...");

    let bot = Bot::from_env();

    let storage = Arc::new(Mutex::new(
        EventStorage::open("events.db").expect("Failed to open database"),
    ));

    // Load and reschedule pending events from storage
    let pending_events = {
        let storage_guard = storage.lock().unwrap();
        storage_guard.get_pending()
    };

    match pending_events {
        Ok(events) => {
            log::info!("Loading {} pending events from storage", events.len());
            let now = chrono::Local::now().naive_local();

            for event in events {
                let delay = event.target_datetime.signed_duration_since(now);
                let delay_secs = delay.num_seconds().max(0) as u64;

                let bot_clone = bot.clone();
                let chat_id = ChatId(event.chat_id);
                let message_text = event.message.clone();
                let event_id = event.id;
                let storage_clone = Arc::clone(&storage);

                tokio::spawn(async move {
                    tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;
                    let _ = bot_clone.send_message(chat_id, &message_text).await;

                    // Mark event as fired
                    if let Ok(storage) = storage_clone.lock() {
                        let _ = storage.mark_fired(event_id);
                    }
                });

                log::info!(
                    "Rescheduled event {} for {} (in {} seconds)",
                    event_id,
                    event.target_datetime,
                    delay_secs
                );
            }
        }
        Err(e) => log::error!("Failed to load pending events: {}", e),
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
                if text == "exit" {
                    log::info!("Received exit command. Shutting down...");
                    tokio::spawn(async {
                        // TODO: Implement proper shutdown logic
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        process::exit(0);
                    });

                    // Return Ok - this acknowledges the update to Telegram
                    return Ok(());
                }

                if let Some(event) = parser::parse(text) {
                    if let Some(target_datetime) = parser::resolve_datetime(&event) {
                        let now = chrono::Local::now().naive_local();
                        let delay = target_datetime.signed_duration_since(now);
                        let delay_secs = delay.num_seconds().max(0) as u64;

                        // Save event to storage
                        let event_id = {
                            let storage_guard = storage.lock().unwrap();
                            storage_guard.insert(msg.chat.id.0, &event, target_datetime)
                        };

                        let event_id = match event_id {
                            Ok(id) => {
                                log::info!("Saved event with id: {}", id);
                                Some(id)
                            }
                            Err(e) => {
                                log::error!("Failed to save event: {}", e);
                                None
                            }
                        };

                        let message_text = event.message.clone();
                        let chat_id = msg.chat.id;
                        let bot_clone = bot.clone();
                        let storage_clone = Arc::clone(&storage);

                        tokio::spawn(async move {
                            tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;
                            let _ = bot_clone.send_message(chat_id, &message_text).await;

                            // Mark event as fired
                            if let Some(id) = event_id {
                                if let Ok(storage) = storage_clone.lock() {
                                    let _ = storage.mark_fired(id);
                                }
                            }
                        });

                        println!("target_datetime: {:?}", target_datetime);
                        format!(
                            "Scheduled message for {}",
                            target_datetime.format("%H:%M %d\\.%m\\.%Y")
                        )
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
    }
}
