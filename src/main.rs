use chrono::{Duration, Local, NaiveTime};
use regex::Regex;
use std::process;
use teloxide::{prelude::*, types::ParseMode};

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    log::info!("Starting bot...");

    let bot = Bot::from_env();

    // TODO: send message to admin that bot started

    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        println!("msg: {:?}\nkind: {:?}", msg.chat, msg.chat.kind);

        let reply_text = if let Some(text) = msg.text() {
            if text == "exit" {
                log::info!("Received exit command. Shutting down...");
                process::exit(0);
            }

            // Check for time-based scheduled message pattern "hh:mm message"
            let time_regex = Regex::new(r"^(\d{1,2}):(\d{2})\s+(.+)$").unwrap();
            if let Some(caps) = time_regex.captures(text) {
                let hour: u32 = caps[1].parse().unwrap_or(0);
                let minute: u32 = caps[2].parse().unwrap_or(0);
                let message_text = caps[3].to_string();

                if hour < 24 && minute < 60 {
                    let now = Local::now();
                    let target_time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap();
                    let current_time = now.time();

                    let mut target_datetime = now.date_naive().and_time(target_time);

                    // If time has already passed today, add 24 hours
                    if target_time <= current_time {
                        target_datetime = target_datetime + Duration::days(1);
                    }

                    let current_datetime = now.naive_local();
                    let delay = target_datetime.signed_duration_since(current_datetime);
                    let delay_secs = delay.num_seconds().max(0) as u64;

                    let chat_id = msg.chat.id;
                    let bot_clone = bot.clone();

                    tokio::spawn(async move {
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;
                        let _ = bot_clone.send_message(chat_id, &message_text).await;
                    });

                    println!("target_datetime: {:?}", target_datetime);
                    format!("Scheduled message for {:02}:{:02}", hour, minute)
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
