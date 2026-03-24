use perbot::parser;
use perbot::state::EventProvider;
use perbot::storage::EventStorage;
use perbot::telegram::{escape_markdown, extract_chat_info};
use perbot::types::TgMessage;
use std::process;
use teloxide::{prelude::*, types::ParseMode};
use tokio::sync::mpsc;

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

    let token = std::env::var("TG_BOT_TOKEN").expect("TG_BOT_TOKEN environment variable not set");
    let bot = Bot::new(token);
    bot.send_message(admin_id, "*Bot started*")
        .parse_mode(ParseMode::MarkdownV2)
        .await
        .unwrap();

    let storage = EventStorage::open("perbot.db").expect("Failed to open database");
    let provider = EventProvider::new(storage);

    // Channel for sending scheduled messages to Telegram
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<Vec<TgMessage>>();
    let sender_bot = bot.clone();
    tokio::spawn(async move {
        while let Some(messages) = msg_rx.recv().await {
            for msg in messages {
                if let Err(e) = sender_bot
                    .send_message(ChatId(msg.chat_id), &msg.text)
                    .await
                {
                    log::error!("Failed to send message to {}: {}", msg.chat_id, e);
                }
            }
        }
    });

    // Start background polling thread: reloads events, sends missed, polls every second
    provider.start(msg_tx);

    let handler_provider = provider.clone();
    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let provider = handler_provider.clone();
        async move {
            // Save/update chat info
            let chat_info = extract_chat_info(&msg.chat);
            if let Err(e) = provider.upsert_chat(&chat_info) {
                log::error!("Failed to save chat info: {}", e);
            }

            let reply_text = if let Some(text) = msg.text() {
                // Store every incoming user message
                let user_id = msg.from.as_ref().map(|u| u.id.0 as i64);
                let msg_id = match provider.insert_message(user_id, msg.chat.id.0, text) {
                    Ok(id) => id,
                    Err(e) => {
                        log::error!("Failed to save message: {}", e);
                        return Ok(());
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

                    let stored = provider.insert_and_get(event);

                    if let Some(dt) = stored.next_datetime {
                        format!(
                            "Scheduled message for *{}*",
                            dt.format("%H:%M %d\\.%m\\.%Y")
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
