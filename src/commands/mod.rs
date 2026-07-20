//! Bot commands: the [`Command`] menu, the shared [`CmdContext`], and the
//! dispatch to one module per command (`start`, `help`, `database`, `logs`,
//! `exit`, `import`) or per group of similar commands (`list` for the paginated
//! `/events`/`/today`/`/tomorrow`/`/week`/`/month` lists, `event` for the
//! `/event<id>` view and its callbacks, `user` for the admin `/user<id>` view
//! and its ban toggle). `snooze` and `cancel` hold the remaining
//! button-callback handlers routed here from `main`.

mod cancel;
mod database;
mod event;
mod exit;
mod help;
mod import;
mod list;
mod logs;
mod settings;
mod snooze;
mod start;
mod timezone;
mod user;

pub use cancel::handle_cancel_pending;
pub use event::{handle_event_callback, handle_event_view, parse_event_command};
pub use import::{PendingImport, handle_import_zip, new_pending};
pub use list::handle_list_callback;
pub use settings::handle_settings_callback;
pub use snooze::handle_snooze_callback;
pub use timezone::handle_timezone_callback;
pub use user::{handle_user_callback, handle_user_view, parse_user_command};

use crate::locale::LocaleProvider;
use crate::pending::{PendingEdit, PendingMessage};
use crate::state::EventProvider;
use crate::tgbot::TgBot;
use crate::view::ListKind;
use chrono_tz::Tz;
use teloxide::types::{ChatId, Message};
use teloxide::utils::command::BotCommands;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
pub enum Command {
    #[command(description = "welcome message", hide)]
    Start,
    #[command(description = "show this help message")]
    Help,
    #[command(description = "list upcoming scheduled events")]
    Events,
    #[command(description = "list today's events")]
    Today,
    #[command(description = "list tomorrow's events")]
    Tomorrow,
    #[command(description = "list this week's events")]
    Week,
    #[command(description = "list this month's events")]
    Month,
    #[command(description = "show or change the chat timezone")]
    Timezone,
    #[command(description = "open the settings menu")]
    Settings,
    #[command(
        description = "import legacy alerts for a chat: /import <user_id> <timezone> (admin only)",
        hide
    )]
    Import(String),
    #[command(description = "download the database file (admin only)", hide)]
    Database,
    #[command(description = "download the current log file (admin only)", hide)]
    Logs,
    #[command(description = "shut the bot down (admin only)", hide)]
    Exit(String),
}

/// Shared dependencies of the text-message path: built once per incoming
/// message (after `main`'s timezone gate) and passed to every command handler
/// and text-flow helper.
pub struct CmdContext<'a> {
    pub bot: &'a TgBot,
    /// The incoming Telegram message being handled.
    pub msg: &'a Message,
    /// The message's text (`msg.text()`) — the input every flow parses.
    pub text: &'a str,
    pub chat_id: ChatId,
    /// The chat's timezone, resolved once by `main`'s timezone gate before any
    /// command is dispatched (so it is always the stored setting, never a
    /// fallback).
    pub tz: Tz,
    pub provider: &'a EventProvider,
    pub admin_id: ChatId,
    pub is_admin: bool,
    pub bot_username: &'a str,
    pub pending_import: &'a PendingImport,
    pub pending_msg: &'a PendingMessage,
    pub pending_edit: &'a PendingEdit,
    /// The chat's locale, resolved once alongside the context.
    pub loc: &'static dyn LocaleProvider,
}

impl Command {
    /// Dispatches a parsed command to its handler.
    pub async fn handle(self, ctx: &CmdContext<'_>) -> anyhow::Result<()> {
        match self {
            Command::Start => start::handle_start(ctx).await,
            Command::Help => help::handle_help(ctx).await,
            Command::Events => list::handle_list(ctx, ListKind::Events).await,
            Command::Today => list::handle_list(ctx, ListKind::Today).await,
            Command::Tomorrow => list::handle_list(ctx, ListKind::Tomorrow).await,
            Command::Week => list::handle_list(ctx, ListKind::Week).await,
            Command::Month => list::handle_list(ctx, ListKind::Month).await,
            Command::Timezone => timezone::handle_timezone(ctx).await,
            Command::Settings => settings::handle_settings(ctx).await,
            Command::Import(args) => import::handle_import(ctx, &args).await,
            Command::Database => database::handle_database(ctx).await,
            Command::Logs => logs::handle_logs(ctx).await,
            Command::Exit(arg) => exit::handle_exit(ctx, &arg).await,
        }
    }
}
