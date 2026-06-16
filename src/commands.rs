use crate::state::EventProvider;
use crate::telegram::{format_events_list, format_today_list, format_tomorrow_list};
use chrono::{Duration, Local};
use std::process;
use teloxide::{prelude::*, types::ParseMode, utils::command::BotCommands};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
pub enum Command {
    #[command(description = "show this help message")]
    Help,
    #[command(description = "list upcoming scheduled events")]
    Events,
    #[command(description = "list today's events")]
    Today,
    #[command(description = "list tomorrow's events")]
    Tomorrow,
    #[command(description = "shut the bot down (admin only)", hide)]
    Exit,
}

/// Shared dependencies passed to every command handler.
pub struct CmdContext<'a> {
    pub bot: &'a Bot,
    pub chat_id: ChatId,
    pub provider: &'a EventProvider,
    pub admin_id: ChatId,
    pub is_admin: bool,
}

impl Command {
    /// Dispatches a parsed command to its handler.
    pub async fn handle(self, ctx: CmdContext<'_>) -> ResponseResult<()> {
        match self {
            Command::Help => handle_help(&ctx).await,
            Command::Events => handle_events(&ctx).await,
            Command::Today => handle_today(&ctx).await,
            Command::Tomorrow => handle_tomorrow(&ctx).await,
            Command::Exit => handle_exit(&ctx).await,
        }
    }
}

/// Replies with the list of commands. Admins additionally see admin-only commands.
async fn handle_help(ctx: &CmdContext<'_>) -> ResponseResult<()> {
    let mut help = Command::descriptions().to_string();
    if ctx.is_admin {
        help.push_str("\n\nAdmin commands:\n/exit — shut the bot down");
    }
    ctx.bot.send_message(ctx.chat_id, help).await?;
    Ok(())
}

/// Replies with the chat's active upcoming events.
async fn handle_events(ctx: &CmdContext<'_>) -> ResponseResult<()> {
    let events = ctx.provider.get_active_by_chat(ctx.chat_id.0);
    ctx.bot
        .send_message(ctx.chat_id, format_events_list(&events))
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
    Ok(())
}

/// Replies with the chat's active events scheduled for today.
async fn handle_today(ctx: &CmdContext<'_>) -> ResponseResult<()> {
    let today = Local::now().naive_local().date();
    let events = ctx
        .provider
        .get_active_by_chat_on_date(ctx.chat_id.0, today);
    ctx.bot
        .send_message(ctx.chat_id, format_today_list(&events))
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
    Ok(())
}

/// Replies with the chat's active events scheduled for tomorrow.
async fn handle_tomorrow(ctx: &CmdContext<'_>) -> ResponseResult<()> {
    let tomorrow = Local::now().naive_local().date() + Duration::days(1);
    let events = ctx
        .provider
        .get_active_by_chat_on_date(ctx.chat_id.0, tomorrow);
    ctx.bot
        .send_message(ctx.chat_id, format_tomorrow_list(&events))
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
    Ok(())
}

/// Shuts the bot down. Admin-only; non-admins get a rejection reply.
async fn handle_exit(ctx: &CmdContext<'_>) -> ResponseResult<()> {
    if !ctx.is_admin {
        ctx.bot
            .send_message(ctx.chat_id, "Not authorized\\.")
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        return Ok(());
    }
    log::info!("Received /exit command. Shutting down...");
    let _ = ctx.bot.send_message(ctx.admin_id, "Shutting down...").await;
    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        process::exit(0);
    });
    Ok(())
}
