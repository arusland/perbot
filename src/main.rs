use perbot::parser;
use perbot::state::EventProvider;
use perbot::storage::{ChatInfo, ChatType, EventStorage};
use std::process;
use std::sync::{Arc, Mutex};
use teloxide::{prelude::*, types::ParseMode};
use tokio::sync::mpsc;

type EventProviderState = Arc<Mutex<EventProvider>>;
type MessageSender = mpsc::UnboundedSender<(ChatId, String)>;

#[tokio::main]
async fn main() {
    perbot::logger::init();
    log::info!("Starting bot...");

    let admin_id = ChatId(
        std::env::var("TG_ADMIN_ID")
            .expect("ADMIN_ID environment variable not set")
            .parse::<i64>()
            .expect("TG_ADMIN_ID must be a valid i64"),
    );

    let bot = Bot::from_env();
    bot.send_message(admin_id, "Bot started").await.unwrap();

    let storage = EventStorage::open("perbot.db").expect("Failed to open database");
    let provider = Arc::new(Mutex::new(EventProvider::new(storage)));

    // Load top events from storage on startup
    provider.lock().unwrap().reload();

    // Channel for sending scheduled messages to Telegram
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<(ChatId, String)>();
    let sender_bot = bot.clone();
    tokio::spawn(async move {
        while let Some((chat_id, message)) = msg_rx.recv().await {
            if let Err(e) = sender_bot.send_message(chat_id, &message).await {
                log::error!("Failed to send message to {}: {}", chat_id, e);
            }
        }
    });

    schedule_first_event(bot.clone(), Arc::clone(&provider), msg_tx.clone());

    let handler_provider = Arc::clone(&provider);
    let handler_msg_tx = msg_tx;
    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let provider = Arc::clone(&handler_provider);
        let msg_tx = handler_msg_tx.clone();
        async move {
            println!("msg: {:?}\nkind: {:?}", msg.chat, msg.chat.kind);

            // Save/update chat info
            {
                let chat_info = extract_chat_info(&msg.chat);
                let prov = provider.lock().unwrap();
                if let Err(e) = prov.upsert_chat(&chat_info) {
                    log::error!("Failed to save chat info: {}", e);
                }
            }

            let reply_text = if let Some(text) = msg.text() {
                // Store every incoming user message
                let user_id = msg.from.as_ref().map(|u| u.id.0 as i64);
                let msg_id = {
                    let prov = provider.lock().unwrap();
                    match prov.insert_message(user_id, msg.chat.id.0, text) {
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

                if let Some(mut event) = parser::parse(text) {
                    event.chat_id = msg.chat.id.0;
                    event.msg_id = msg_id;

                    let (stored, _) = {
                        let mut prov = provider.lock().unwrap();
                        prov.insert(event)
                    };

                    if let Some(dt) = stored.next_datetime {
                        println!("next_datetime: {:?}", dt);

                        // Reschedule with updated top events
                        schedule_first_event(bot.clone(), Arc::clone(&provider), msg_tx.clone());

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

/// Schedules the first event from the top events list.
/// Cancels any previously scheduled task, then spawns a new delayed task
/// for the earliest event. After firing, recalculates next datetime,
/// saves to DB, updates the in-memory list, and schedules the next first event.
fn schedule_first_event(bot: Bot, provider: EventProviderState, msg_tx: MessageSender) {
    let mut prov = provider.lock().unwrap();

    // Cancel current scheduled task
    prov.abort_current();

    let Some(event) = prov.get_next() else {
        log::info!("No top events to schedule");
        return;
    };

    let Some(dt) = event.next_datetime else {
        return;
    };

    let chat_id = ChatId(event.chat_id);
    let message = event.message.clone();
    let provider_clone = Arc::clone(&provider);

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

        if let Err(e) = msg_tx.send((chat_id, message)) {
            log::error!("Failed to queue message for event {}: {}", event_id, e);
        }

        {
            let mut prov = provider_clone.lock().unwrap();
            prov.update(event);
        }

        // Schedule the next first event
        schedule_first_event(bot, provider_clone, msg_tx);
    });

    prov.set_abort_handle(join_handle.abort_handle());
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
