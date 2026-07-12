//! Bot commands: the [`Command`] menu, the shared [`CmdContext`], and the
//! dispatch to one module per command (`help`, `database`, `logs`, `exit`,
//! `import`) or per group of similar commands (`list` for the paginated
//! `/events`/`/today`/`/tomorrow`/`/week`/`/month` lists, `event` for the
//! `/event<id>` view and its callbacks). `snooze` and `cancel` hold the
//! remaining button-callback handlers routed here from `main`.

mod cancel;
mod database;
mod event;
mod exit;
mod help;
mod import;
mod list;
mod logs;
mod snooze;
mod timezone;

pub use cancel::handle_cancel_pending;
pub use event::{handle_event_callback, handle_event_view, parse_event_command};
pub use import::handle_import_zip;
pub use list::handle_list_callback;
pub use snooze::handle_snooze_callback;
pub use timezone::handle_timezone_callback;

use crate::import::PendingImport;
use crate::state::EventProvider;
use crate::tgbot::TgBot;
use crate::view::ListKind;
use teloxide::types::ChatId;
use teloxide::utils::command::BotCommands;

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
    #[command(description = "list this week's events")]
    Week,
    #[command(description = "list this month's events")]
    Month,
    #[command(description = "show or change the chat timezone")]
    Timezone,
    #[command(description = "import legacy alerts for a chat (admin only)", hide)]
    Import(i64),
    #[command(description = "download the database file (admin only)", hide)]
    Database,
    #[command(description = "download the current log file (admin only)", hide)]
    Logs,
    #[command(description = "shut the bot down (admin only)", hide)]
    Exit(String),
}

/// Shared dependencies passed to every command handler.
pub struct CmdContext<'a> {
    pub bot: &'a TgBot,
    pub chat_id: ChatId,
    pub provider: &'a EventProvider,
    pub admin_id: ChatId,
    pub is_admin: bool,
    pub pending_import: &'a PendingImport,
}

impl Command {
    /// Dispatches a parsed command to its handler.
    pub async fn handle(self, ctx: CmdContext<'_>) -> anyhow::Result<()> {
        match self {
            Command::Help => help::handle_help(&ctx).await,
            Command::Events => list::handle_list(&ctx, ListKind::Events).await,
            Command::Today => list::handle_list(&ctx, ListKind::Today).await,
            Command::Tomorrow => list::handle_list(&ctx, ListKind::Tomorrow).await,
            Command::Week => list::handle_list(&ctx, ListKind::Week).await,
            Command::Month => list::handle_list(&ctx, ListKind::Month).await,
            Command::Timezone => timezone::handle_timezone(&ctx).await,
            Command::Import(user_id) => import::handle_import(&ctx, user_id).await,
            Command::Database => database::handle_database(&ctx).await,
            Command::Logs => logs::handle_logs(&ctx).await,
            Command::Exit(arg) => exit::handle_exit(&ctx, &arg).await,
        }
    }
}
