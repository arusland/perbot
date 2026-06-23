use anyhow::Context as _;
use perbot::commands::{self, CmdContext, Command};
use perbot::import::{self, PendingImport};
use perbot::parser;
use perbot::pending::{self, PendingMessage};
use perbot::state::EventProvider;
use perbot::storage::EventStorage;
use perbot::telegram::{extract_chat_info, scheduled_message};
use perbot::types::TgMessage;
use teloxide::{
    prelude::*,
    types::{BotCommandScope, ParseMode},
    utils::command::BotCommands,
    utils::html,
};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    perbot::logger::init();
    log::info!("Starting bot...");

    let admin_id = ChatId(
        std::env::var("TG_ADMIN_ID")
            .context("TG_ADMIN_ID environment variable not set")?
            .parse::<i64>()
            .context("TG_ADMIN_ID must be a valid i64")?,
    );

    let token =
        std::env::var("TG_BOT_TOKEN").context("TG_BOT_TOKEN environment variable not set")?;
    let bot = Bot::new(token);
    if let Err(e) = bot
        .send_message(admin_id, "<b>Bot started</b>")
        .parse_mode(ParseMode::Html)
        .await
    {
        log::warn!("Failed to send startup message: {}", e);
    }

    // The bot username is required to parse commands addressed as `/cmd@bot`, so
    // failing to fetch it is fatal.
    let me = bot.get_me().await.context("Failed to fetch bot info")?;
    let bot_username = me.username().to_string();

    // Clear any commands left over from a previous bot on this token. These can
    // live in more specific scopes that take precedence over the default scope,
    // so we delete them before registering ours.
    for scope in [
        BotCommandScope::AllPrivateChats,
        BotCommandScope::AllGroupChats,
        BotCommandScope::AllChatAdministrators,
    ] {
        if let Err(e) = bot.delete_my_commands().scope(scope).await {
            log::warn!("Failed to clear old commands scope: {}", e);
        }
    }
    if let Err(e) = bot.set_my_commands(Command::bot_commands()).await {
        log::error!("Failed to register bot commands: {}", e);
    }

    let storage = EventStorage::open("perbot.db").context("Failed to open database")?;
    let provider = EventProvider::new(storage);

    // Channel for sending scheduled messages to Telegram
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<Vec<TgMessage>>();
    let sender_bot = bot.clone();
    tokio::spawn(async move {
        while let Some(messages) = msg_rx.recv().await {
            for msg in messages {
                // Every routed `text` is an HTML fragment (the reminder body may
                // carry the user's formatting), so always send as HTML.
                let mut req = sender_bot
                    .send_message(ChatId(msg.chat_id), msg.text)
                    .parse_mode(ParseMode::Html);
                if let Some(kb) = msg.reply_markup {
                    req = req.reply_markup(kb);
                }
                if let Err(e) = req.await {
                    log::error!("Failed to send message to {}: {}", msg.chat_id, e);
                }
            }
        }
    });

    // Start background polling thread: reloads events, sends missed, polls every second
    provider.start(msg_tx);

    // Pending legacy import target (chat id) recorded by `/import <user_id>`.
    let pending_import: PendingImport = import::new_pending();

    // Per-chat events awaiting a reminder body after a time-only message.
    let pending_msg: PendingMessage = pending::new_pending();

    // Dispatcher with two branches: text/document messages and inline-button
    // callbacks (used by paginated `/events`). Shared deps are injected via
    // `dptree::deps!`.
    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(message_handler))
        .branch(Update::filter_callback_query().endpoint(callback_handler));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            provider,
            admin_id,
            bot_username,
            pending_import,
            pending_msg
        ])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

/// Handles an inline-button callback (list pagination for `/events`, `/today`,
/// `/tomorrow`, `/week`, `/month`).
async fn callback_handler(
    bot: Bot,
    q: CallbackQuery,
    provider: EventProvider,
    pending_msg: PendingMessage,
) -> ResponseResult<()> {
    // `eid:<id>:…` is the event-specific envelope (snooze today); `pm:` cancels a
    // pending "send me the reminder text" prompt; everything else is list
    // pagination (`<tag>:<page>`).
    match q.data.as_deref() {
        Some(d) if d.starts_with("eid:") => {
            commands::handle_snooze_callback(&bot, &provider, q).await
        }
        Some(d) if d.starts_with("pm:") => {
            commands::handle_cancel_pending(&bot, &pending_msg, q).await
        }
        _ => commands::handle_list_callback(&bot, &provider, q).await,
    }
}

/// Handles a single incoming message: stores chat/message info, dispatches
/// commands, parses event text, or acknowledges other media types.
async fn message_handler(
    bot: Bot,
    msg: Message,
    provider: EventProvider,
    admin_id: ChatId,
    bot_username: String,
    pending_import: PendingImport,
    pending_msg: PendingMessage,
) -> ResponseResult<()> {
    // Save/update chat info
    let chat_info = extract_chat_info(&msg.chat);
    if let Err(e) = provider.upsert_chat(&chat_info) {
        log::error!("Failed to save chat info: {}", e);
    }

    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64);
    let is_admin = user_id == Some(admin_id.0);

    // Legacy import: the admin sends the zip after `/import <user_id>`.
    let pending_target = *pending_import.lock().unwrap();
    if is_admin && let (Some(target), Some(doc)) = (pending_target, msg.document()) {
        *pending_import.lock().unwrap() = None;
        commands::handle_import_zip(&bot, &provider, msg.chat.id, target, doc.file.id.clone())
            .await?;
        return Ok(());
    }

    let reply_text = if let Some(text) = msg.text() {
        // Store every incoming user message
        let msg_id = match provider.insert_message(user_id, msg.chat.id.0, text) {
            Ok(id) => id,
            Err(e) => {
                log::error!("Failed to save message: {}", e);
                return Ok(());
            }
        };
        if let Ok(cmd) = Command::parse(text, &bot_username) {
            let ctx = CmdContext {
                bot: &bot,
                chat_id: msg.chat.id,
                provider: &provider,
                admin_id,
                is_admin,
                pending_import: &pending_import,
            };
            cmd.handle(ctx).await?;
            return Ok(());
        }

        // Completing a pending "send me the reminder text": once a chat is waiting
        // for a body, the next non-command text is used verbatim as that body.
        let pending_event = pending_msg.lock().unwrap().remove(&msg.chat.id.0);
        if let Some(mut event) = pending_event {
            let entities = msg.parse_entities().unwrap_or_default();
            // The whole reply text is the body, so a single span covers all of it.
            let span = 0..text.len();
            let body = perbot::richtext::render_html(text, std::slice::from_ref(&span), &entities);
            if body.is_empty() {
                // Whitespace-only reply carries no usable body: keep waiting and
                // re-prompt with the Cancel button.
                pending_msg.lock().unwrap().insert(msg.chat.id.0, event);
                bot.send_message(msg.chat.id, pending::ASK_TEXT)
                    .reply_markup(pending::cancel_keyboard())
                    .await?;
                return Ok(());
            }
            event.chat_id = msg.chat.id.0;
            event.msg_id = msg_id;
            event.message = body;
            let stored = provider.insert_event_and_get(event);
            let reply = if let Some(dt) = stored.next_datetime {
                let now = chrono::Local::now().naive_local();
                scheduled_message(now, dt, &stored)
            } else {
                format!("<b>{}</b>", html::escape(text))
            };
            bot.send_message(msg.chat.id, reply)
                .parse_mode(ParseMode::Html)
                .await?;
            return Ok(());
        }

        if let Some((mut event, spans)) = parser::parse_full(text) {
            event.chat_id = msg.chat.id.0;
            event.msg_id = msg_id;
            // Preserve the user's formatting: render the surviving message body
            // as an HTML fragment, re-mapping the message's entities onto it.
            let entities = msg.parse_entities().unwrap_or_default();
            event.message = perbot::richtext::render_html(text, &spans, &entities);

            let stored = provider.insert_event_and_get(event);

            if let Some(dt) = stored.next_datetime {
                let now = chrono::Local::now().naive_local();
                scheduled_message(now, dt, &stored)
            } else {
                format!("<b>{}</b>", html::escape(text))
            }
        } else if let Some(event) = parser::parse_time_only(text) {
            // A time was given but no reminder body: hold the parsed event and ask
            // for the text, offering a Cancel button.
            pending_msg.lock().unwrap().insert(msg.chat.id.0, event);
            bot.send_message(msg.chat.id, pending::ASK_TEXT)
                .reply_markup(pending::cancel_keyboard())
                .await?;
            return Ok(());
        } else {
            format!("Unparsable message: <b>{}</b>", html::escape(text))
        }
    } else if msg.photo().is_some() {
        "Received a photo!".to_string()
    } else if msg.video().is_some() {
        "Received a video!".to_string()
    } else if msg.audio().is_some() {
        "Received an audio file!".to_string()
    } else if msg.voice().is_some() {
        "Received a voice message!".to_string()
    } else if msg.document().is_some() {
        "Received a document!".to_string()
    } else if msg.sticker().is_some() {
        "Received a sticker!".to_string()
    } else if msg.animation().is_some() {
        "Received an animation!".to_string()
    } else if msg.video_note().is_some() {
        "Received a video note!".to_string()
    } else if msg.contact().is_some() {
        "Received a contact!".to_string()
    } else if msg.location().is_some() {
        "Received a location!".to_string()
    } else if msg.venue().is_some() {
        "Received a venue!".to_string()
    } else if msg.poll().is_some() {
        "Received a poll!".to_string()
    } else if msg.dice().is_some() {
        "Received a dice!".to_string()
    } else {
        "Received an unknown message type!".to_string()
    };

    bot.send_message(msg.chat.id, reply_text)
        .parse_mode(ParseMode::Html)
        .await?;

    Ok(())
}
