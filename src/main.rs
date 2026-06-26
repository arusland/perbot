use anyhow::Context as _;
use perbot::commands::{self, CmdContext, Command};
use perbot::import::{self, PendingImport};
use perbot::parser;
use perbot::pending::{self, PendingEdit, PendingMessage};
use perbot::state::EventProvider;
use perbot::storage::EventStorage;
use perbot::telegram::{clamp_message, edit_prompt, extract_chat_info, scheduled_message};
use perbot::tgbot::TgBot;
use perbot::types::TgMessage;
use teloxide::{prelude::*, types::BotCommandScope, utils::command::BotCommands, utils::html};
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
    // Every outbound call goes through the logging wrapper; the raw `bot` is kept
    // only to hand to the dispatcher's updater below.
    let tg = TgBot::new(bot.clone());
    if let Err(e) = tg.send_html(admin_id, "<b>Bot started</b>", None).await {
        log::warn!("Failed to send startup message: {}", e);
    }

    // The bot username is required to parse commands addressed as `/cmd@bot`, so
    // failing to fetch it is fatal.
    let me = tg.get_me().await.context("Failed to fetch bot info")?;
    let bot_username = me.username().to_string();

    // Clear any commands left over from a previous bot on this token. These can
    // live in more specific scopes that take precedence over the default scope,
    // so we delete them before registering ours.
    for scope in [
        BotCommandScope::AllPrivateChats,
        BotCommandScope::AllGroupChats,
        BotCommandScope::AllChatAdministrators,
    ] {
        if let Err(e) = tg.delete_my_commands(scope).await {
            log::warn!("Failed to clear old commands scope: {}", e);
        }
    }
    if let Err(e) = tg.set_my_commands(Command::bot_commands()).await {
        log::error!("Failed to register bot commands: {}", e);
    }

    let storage = EventStorage::open("perbot.db").context("Failed to open database")?;
    let provider = EventProvider::new(storage);

    // Channel for sending scheduled messages to Telegram
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<Vec<TgMessage>>();
    let sender_bot = tg.clone();
    tokio::spawn(async move {
        while let Some(messages) = msg_rx.recv().await {
            for msg in messages {
                // Every routed `text` is an HTML fragment (the reminder body may
                // carry the user's formatting), so always send as HTML.
                if let Err(e) = sender_bot
                    .send_html(ChatId(msg.chat_id), msg.text, msg.reply_markup)
                    .await
                {
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

    // Per-chat events being edited after tapping Edit on the `/event<id>` view.
    let pending_edit: PendingEdit = pending::new_pending_edit();

    // Dispatcher with two branches: text/document messages and inline-button
    // callbacks (used by paginated `/events`). Shared deps are injected via
    // `dptree::deps!`.
    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(message_handler_safe))
        .branch(Update::filter_callback_query().endpoint(callback_handler));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            provider,
            admin_id,
            bot_username,
            pending_import,
            pending_msg,
            pending_edit,
            tg
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
    bot: TgBot,
    q: CallbackQuery,
    provider: EventProvider,
    pending_msg: PendingMessage,
    pending_edit: PendingEdit,
) -> ResponseResult<()> {
    // `eid:<id>:…` is the event-specific envelope (snooze / delete / edit); `pm:`
    // cancels a pending "send me the reminder text" prompt; everything else is list
    // pagination (`<tag>:<page>`).
    match q.data.as_deref() {
        Some(d) if d.starts_with("eid:") => {
            commands::handle_event_callback(&bot, &provider, &pending_edit, q).await
        }
        Some(d) if d.starts_with("pm:") => {
            commands::handle_cancel_pending(&bot, &pending_msg, q).await
        }
        _ => commands::handle_list_callback(&bot, &provider, q).await,
    }
}

/// Wraps `message_handler` so a failure can never bubble up to the dispatcher's
/// default error path. On error it logs the chat/message context, tells the user
/// "Something goes wrong!", and forwards the error detail to the admin.
// Dependencies are injected individually by dptree, so the arg count is expected.
#[allow(clippy::too_many_arguments)]
async fn message_handler_safe(
    bot: TgBot,
    msg: Message,
    provider: EventProvider,
    admin_id: ChatId,
    bot_username: String,
    pending_import: PendingImport,
    pending_msg: PendingMessage,
    pending_edit: PendingEdit,
) -> ResponseResult<()> {
    // `message_handler` consumes its arguments, so capture the reporting context
    // (and a bot handle for the follow-up sends) before handing them over.
    let bot_for_err = bot.clone();
    let chat_id = msg.chat.id;
    let chat_info = extract_chat_info(&msg.chat);
    let msg_text = msg.text().map(str::to_owned);

    if let Err(e) = message_handler(
        bot,
        msg,
        provider,
        admin_id,
        bot_username,
        pending_import,
        pending_msg,
        pending_edit,
    )
    .await
    {
        log::error!(
            "message_handler failed (chat {:?}, msg {:?}): {}",
            chat_info,
            msg_text,
            e
        );

        // Best-effort notifications: ignore secondary send errors so reporting a
        // failure can't itself re-trigger the dispatcher's error path.
        let _ = bot_for_err
            .send_text(chat_id, "⚠️<b>Something goes wrong!</b>", None)
            .await;
        let _ = bot_for_err
            .send_html(
                admin_id,
                format!("<b>Error:</b> {}", html::escape(&e.to_string())),
                None,
            )
            .await;
    }

    Ok(())
}

/// Handles a single incoming message: stores chat/message info, dispatches
/// commands, parses event text, or acknowledges other media types.
// Dependencies are injected individually by dptree, so the arg count is expected.
#[allow(clippy::too_many_arguments)]
async fn message_handler(
    bot: TgBot,
    msg: Message,
    provider: EventProvider,
    admin_id: ChatId,
    bot_username: String,
    pending_import: PendingImport,
    pending_msg: PendingMessage,
    pending_edit: PendingEdit,
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
        // Resolve the chat's locale once; every parse/format below threads it.
        let loc = perbot::locale::for_chat(msg.chat.id.0);
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

        // `/event<id>` (no space before the id) opens the single-event detail view.
        // It can't go through the `BotCommands` parser, so it is matched manually.
        if let Some(id) = commands::parse_event_command(text, &bot_username) {
            commands::handle_event_view(&bot, &provider, msg.chat.id, id).await?;
            return Ok(());
        }

        // Completing a pending edit (the chat tapped Edit on an `/event<id>` view):
        // the next message replaces the event's time and message. A time-only or
        // unparsable reply re-prompts instead of applying.
        let editing = pending_edit.lock().unwrap().get(&msg.chat.id.0).copied();
        if let Some(event_id) = editing {
            // Re-load the event once and verify it still belongs to this chat; a
            // pending edit can outlive the event (deleted meanwhile).
            let Some(old) = provider
                .get_event(event_id)
                .filter(|e| e.chat_id == msg.chat.id.0)
            else {
                pending_edit.lock().unwrap().remove(&msg.chat.id.0);
                bot.send_text(msg.chat.id, "Event not found.", None).await?;
                return Ok(());
            };

            if let Some((mut event, spans)) = parser::parse_full(text, loc) {
                let entities = msg.parse_entities().unwrap_or_default();
                event.id = old.id;
                event.chat_id = old.chat_id;
                event.created_at = old.created_at;
                event.msg_id = msg_id;
                event.legacy = old.legacy;
                event.snoozed = old.snoozed;
                let rendered = perbot::richtext::render_html(text, &spans, &entities);
                let (clamped, truncated) = clamp_message(&rendered);
                event.message = clamped;

                let stored = provider.update_event_and_get(event);
                pending_edit.lock().unwrap().remove(&msg.chat.id.0);
                if truncated {
                    bot.send_text(msg.chat.id, pending::MESSAGE_TRUNCATED, None)
                        .await?;
                }
                let reply = if let Some(dt) = stored.next_datetime {
                    let now = chrono::Local::now().naive_local();
                    scheduled_message(now, dt, &stored, loc)
                } else {
                    format!("<b>{}</b>", html::escape(text))
                };
                bot.send_html(msg.chat.id, reply, None).await?;
            } else {
                // A time-only or unparsable reply: re-prompt (keeping the pending
                // edit) with the copyable current input still attached.
                let lead = if parser::parse_time_only(text, loc).is_some() {
                    pending::EDIT_NEED_TEXT
                } else {
                    pending::EDIT_NEED_TIME
                };
                bot.send_html(
                    msg.chat.id,
                    edit_prompt(lead, &old, loc),
                    Some(commands::edit_cancel_keyboard(event_id)),
                )
                .await?;
            }
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
                bot.send_text(
                    msg.chat.id,
                    pending::ASK_TEXT,
                    Some(pending::cancel_keyboard()),
                )
                .await?;
                return Ok(());
            }
            event.chat_id = msg.chat.id.0;
            event.msg_id = msg_id;
            let (clamped, truncated) = clamp_message(&body);
            event.message = clamped;
            let stored = provider.insert_event_and_get(event);
            if truncated {
                bot.send_text(msg.chat.id, pending::MESSAGE_TRUNCATED, None)
                    .await?;
            }
            let reply = if let Some(dt) = stored.next_datetime {
                let now = chrono::Local::now().naive_local();
                scheduled_message(now, dt, &stored, loc)
            } else {
                format!("<b>{}</b>", html::escape(text))
            };
            bot.send_html(msg.chat.id, reply, None).await?;
            return Ok(());
        }

        if let Some((mut event, spans)) = parser::parse_full(text, loc) {
            event.chat_id = msg.chat.id.0;
            event.msg_id = msg_id;
            // Preserve the user's formatting: render the surviving message body
            // as an HTML fragment, re-mapping the message's entities onto it.
            let entities = msg.parse_entities().unwrap_or_default();
            let rendered = perbot::richtext::render_html(text, &spans, &entities);
            let (clamped, truncated) = clamp_message(&rendered);
            event.message = clamped;

            let stored = provider.insert_event_and_get(event);
            if truncated {
                bot.send_text(msg.chat.id, pending::MESSAGE_TRUNCATED, None)
                    .await?;
            }

            if let Some(dt) = stored.next_datetime {
                let now = chrono::Local::now().naive_local();
                scheduled_message(now, dt, &stored, loc)
            } else {
                format!("<b>{}</b>", html::escape(text))
            }
        } else if let Some(event) = parser::parse_time_only(text, loc) {
            // A time was given but no reminder body: hold the parsed event and ask
            // for the text, offering a Cancel button.
            pending_msg.lock().unwrap().insert(msg.chat.id.0, event);
            bot.send_text(
                msg.chat.id,
                pending::ASK_TEXT,
                Some(pending::cancel_keyboard()),
            )
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

    bot.send_html(msg.chat.id, reply_text, None).await?;

    Ok(())
}
