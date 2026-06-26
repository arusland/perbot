//! A thin logging wrapper around teloxide's [`Bot`].
//!
//! Every outbound Telegram call in the bot goes through [`TgBot`] instead of the
//! raw `Bot`, so each method can log what it is about to send — the payload size
//! (message character length, document byte size, or downloaded byte count) and
//! the target `chat_id` (or callback/file id). The wrapper also owns the
//! per-method `ParseMode` choice and collapses teloxide's request builders so
//! callers pass an `Option<InlineKeyboardMarkup>` rather than chaining
//! `.parse_mode(...)`/`.reply_markup(...)` themselves.
//!
//! `TgBot` is `Clone` (the inner `Bot` is a cheap handle), so it can be cloned
//! into the sender task and injected into the dispatcher via `dptree::deps!`.

use std::path::Path;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{
    BotCommand, BotCommandScope, CallbackQueryId, File, FileId, InlineKeyboardMarkup, InputFile,
    Me, MessageId, ParseMode,
};

/// Logging wrapper over teloxide's [`Bot`]. See the module docs.
#[derive(Clone)]
pub struct TgBot {
    inner: Bot,
}

impl TgBot {
    /// Wraps a teloxide [`Bot`].
    pub fn new(bot: Bot) -> Self {
        Self { inner: bot }
    }

    /// Sends an HTML message, optionally with an inline keyboard.
    pub async fn send_html(
        &self,
        chat_id: ChatId,
        text: impl Into<String>,
        markup: Option<InlineKeyboardMarkup>,
    ) -> ResponseResult<()> {
        let text = text.into();
        log::info!(
            "send_html → chat {}: {} chars, keyboard={}",
            chat_id.0,
            text.chars().count(),
            markup.is_some()
        );
        // Keep `text` for the error log; the builder consumes a clone.
        let mut req = self
            .inner
            .send_message(chat_id, text.clone())
            .parse_mode(ParseMode::Html);
        if let Some(kb) = markup {
            req = req.reply_markup(kb);
        }
        if let Err(e) = req.await {
            log::error!("send_html failed → chat {}: {e}; text={text}", chat_id.0);
            return Err(e);
        }
        Ok(())
    }

    /// Sends a plain-text message (no parse mode), optionally with a keyboard.
    pub async fn send_text(
        &self,
        chat_id: ChatId,
        text: impl Into<String>,
        markup: Option<InlineKeyboardMarkup>,
    ) -> ResponseResult<()> {
        let text = text.into();
        log::info!(
            "send_text → chat {}: {} chars, keyboard={}",
            chat_id.0,
            text.chars().count(),
            markup.is_some()
        );
        let mut req = self.inner.send_message(chat_id, text.clone());
        if let Some(kb) = markup {
            req = req.reply_markup(kb);
        }
        if let Err(e) = req.await {
            log::error!("send_text failed → chat {}: {e}; text={text}", chat_id.0);
            return Err(e);
        }
        Ok(())
    }

    /// Sends a MarkdownV2 message (used only for the `/exit` rejection reply).
    pub async fn send_markdown(
        &self,
        chat_id: ChatId,
        text: impl Into<String>,
    ) -> ResponseResult<()> {
        let text = text.into();
        log::info!(
            "send_markdown → chat {}: {} chars",
            chat_id.0,
            text.chars().count()
        );
        if let Err(e) = self
            .inner
            .send_message(chat_id, text.clone())
            .parse_mode(ParseMode::MarkdownV2)
            .await
        {
            log::error!(
                "send_markdown failed → chat {}: {e}; text={text}",
                chat_id.0
            );
            return Err(e);
        }
        Ok(())
    }

    /// Edits an existing message's text as HTML, optionally replacing its keyboard.
    pub async fn edit_html(
        &self,
        chat_id: ChatId,
        message_id: MessageId,
        text: impl Into<String>,
        markup: Option<InlineKeyboardMarkup>,
    ) -> ResponseResult<()> {
        let text = text.into();
        log::info!(
            "edit_html → chat {} msg {}: {} chars, keyboard={}",
            chat_id.0,
            message_id.0,
            text.chars().count(),
            markup.is_some()
        );
        let mut req = self
            .inner
            .edit_message_text(chat_id, message_id, text.clone())
            .parse_mode(ParseMode::Html);
        if let Some(kb) = markup {
            req = req.reply_markup(kb);
        }
        if let Err(e) = req.await {
            log::error!(
                "edit_html failed → chat {} msg {}: {e}; text={text}",
                chat_id.0,
                message_id.0
            );
            return Err(e);
        }
        Ok(())
    }

    /// Edits an existing message's text as plain text (no parse mode, no keyboard).
    pub async fn edit_text(
        &self,
        chat_id: ChatId,
        message_id: MessageId,
        text: impl Into<String>,
    ) -> ResponseResult<()> {
        let text = text.into();
        log::info!(
            "edit_text → chat {} msg {}: {} chars",
            chat_id.0,
            message_id.0,
            text.chars().count()
        );
        if let Err(e) = self
            .inner
            .edit_message_text(chat_id, message_id, text.clone())
            .await
        {
            log::error!(
                "edit_text failed → chat {} msg {}: {e}; text={text}",
                chat_id.0,
                message_id.0
            );
            return Err(e);
        }
        Ok(())
    }

    /// Replaces an existing message's inline keyboard, leaving its text untouched.
    pub async fn edit_markup(
        &self,
        chat_id: ChatId,
        message_id: MessageId,
        markup: InlineKeyboardMarkup,
    ) -> ResponseResult<()> {
        log::info!("edit_markup → chat {} msg {}", chat_id.0, message_id.0);
        if let Err(e) = self
            .inner
            .edit_message_reply_markup(chat_id, message_id)
            .reply_markup(markup)
            .await
        {
            log::error!(
                "edit_markup failed → chat {} msg {}: {e}",
                chat_id.0,
                message_id.0
            );
            return Err(e);
        }
        Ok(())
    }

    /// Answers a callback query (clears the client spinner), optionally with a
    /// toast `text`.
    pub async fn answer_callback(
        &self,
        id: CallbackQueryId,
        text: Option<String>,
    ) -> ResponseResult<()> {
        log::info!(
            "answer_callback → query {}: text={:?} chars",
            id.0,
            text.as_ref().map(|t| t.chars().count())
        );
        let query_id = id.0.clone();
        let mut req = self.inner.answer_callback_query(id);
        if let Some(t) = text.clone() {
            req = req.text(t);
        }
        if let Err(e) = req.await {
            log::error!("answer_callback failed → query {query_id}: {e}; text={text:?}");
            return Err(e);
        }
        Ok(())
    }

    /// Sends a file at `path` as a document, optionally overriding its file name.
    /// Logs the on-disk size of the file.
    pub async fn send_document(
        &self,
        chat_id: ChatId,
        path: &Path,
        file_name: Option<&str>,
    ) -> ResponseResult<()> {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        log::info!(
            "send_document → chat {}: {} ({} bytes, name={:?})",
            chat_id.0,
            path.display(),
            size,
            file_name
        );
        let mut file = InputFile::file(path);
        if let Some(name) = file_name {
            file = file.file_name(name.to_owned());
        }
        if let Err(e) = self.inner.send_document(chat_id, file).await {
            log::error!(
                "send_document failed → chat {}: {} ({} bytes, name={:?}): {e}",
                chat_id.0,
                path.display(),
                size,
                file_name
            );
            return Err(e);
        }
        Ok(())
    }

    /// Fetches file metadata (path, size) for a `file_id` so it can be downloaded.
    pub async fn get_file(&self, file_id: FileId) -> ResponseResult<File> {
        log::info!("get_file → {}", file_id);
        match self.inner.get_file(file_id.clone()).await {
            Ok(file) => {
                log::info!("get_file: path={} ({} bytes)", file.path, file.size);
                Ok(file)
            }
            Err(e) => {
                log::error!("get_file failed → {file_id}: {e}");
                Err(e)
            }
        }
    }

    /// Downloads the Telegram file at `path` into `dest`, logging the byte count.
    pub async fn download_file(&self, path: &str, dest: &mut Vec<u8>) -> ResponseResult<()> {
        if let Err(e) = self.inner.download_file(path, dest).await {
            log::error!("download_file failed → {path}: {e}");
            return Err(e.into());
        }
        log::info!("download_file: {} → {} bytes", path, dest.len());
        Ok(())
    }

    /// Fetches this bot's own user info (needed to parse `/cmd@bot` commands).
    pub async fn get_me(&self) -> ResponseResult<Me> {
        match self.inner.get_me().await {
            Ok(me) => {
                log::info!("get_me: @{}", me.username());
                Ok(me)
            }
            Err(e) => {
                log::error!("get_me failed: {e}");
                Err(e)
            }
        }
    }

    /// Registers the bot's command menu.
    pub async fn set_my_commands(&self, commands: Vec<BotCommand>) -> ResponseResult<()> {
        log::info!("set_my_commands: {} commands", commands.len());
        if let Err(e) = self.inner.set_my_commands(commands).await {
            log::error!("set_my_commands failed: {e}");
            return Err(e);
        }
        Ok(())
    }

    /// Clears the bot's command menu for a given scope.
    pub async fn delete_my_commands(&self, scope: BotCommandScope) -> ResponseResult<()> {
        log::info!("delete_my_commands: scope={:?}", scope);
        if let Err(e) = self.inner.delete_my_commands().scope(scope).await {
            log::error!("delete_my_commands failed: {e}");
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_clones() {
        // Construction and cloning must not panic; the inner handle is cheap.
        let tg = TgBot::new(Bot::new("123:fake-token"));
        let _clone = tg.clone();
    }
}
