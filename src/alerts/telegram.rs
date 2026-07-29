//! Telegram bot implementation using frankenstein
//!
//! Provides:
//! - Long polling for receiving commands
//! - Proactive notifications via mpsc channel
//! - Command handling for /status, /trades, /balance, /pnl

use std::sync::Arc;
use std::str::FromStr;

use frankenstein::{
    client_reqwest::Bot,
    methods::{GetUpdatesParams, SendMessageParams},
    updates::{Update, UpdateContent},
    AsyncTelegramApi, ParseMode,
};
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::notifications::AlertEvent;
use super::commands::{BotSnapshot, Command};
use super::messages::{format_alert, is_silent};

/// Telegram bot configuration
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    /// Bot token from @BotFather
    pub token: String,
    /// Chat ID to send proactive notifications to
    pub chat_id: Option<i64>,
}

impl TelegramConfig {
    /// Create from environment variables
    pub fn from_env() -> Option<Self> {
        let token = std::env::var("TELOXIDE_TOKEN")
            .or_else(|_| std::env::var("TELEGRAM_BOT_TOKEN"))
            .ok()?;

        let chat_id = std::env::var("TELEGRAM_CHAT_ID")
            .ok()
            .and_then(|s| s.parse::<i64>().ok());

        Some(Self { token, chat_id })
    }
}

/// Telegram bot handle for sending notifications
#[derive(Clone)]
pub struct TelegramBot {
    bot: Bot,
    chat_id: Option<i64>,
}

impl TelegramBot {
    /// Create a new Telegram bot
    pub fn new(config: TelegramConfig) -> Self {
        let bot = Bot::new(&config.token);
        Self {
            bot,
            chat_id: config.chat_id,
        }
    }

    /// Send a notification to the configured chat
    pub async fn send_notification(&self, message: &str, silent: bool) -> Result<(), frankenstein::Error> {
        let Some(chat_id) = self.chat_id else {
            debug!("No chat_id configured, skipping notification");
            return Ok(());
        };

        let mut params = SendMessageParams::builder()
            .chat_id(chat_id)
            .text(message)
            .parse_mode(ParseMode::MarkdownV2)
            .build();

        if silent {
            params.disable_notification = Some(true);
        }

        self.bot.send_message(&params).await?;
        Ok(())
    }

    /// Send an alert event as a notification
    pub async fn send_alert(&self, event: &AlertEvent) -> Result<(), frankenstein::Error> {
        let message = escape_markdown_v2(&format_alert(event));
        let silent = is_silent(event);
        self.send_notification(&message, silent).await
    }

    /// Update the chat_id (called after user sends /start)
    pub fn set_chat_id(&mut self, chat_id: i64) {
        self.chat_id = Some(chat_id);
    }

    /// Check if bot can connect to Telegram API
    ///
    /// Returns (is_connected, Option<bot_username>)
    pub async fn check_connection(&self) -> (bool, Option<String>) {
        match self.bot.get_me().await {
            Ok(response) => {
                let username = response.result.username.clone();
                (true, username)
            }
            Err(e) => {
                warn!(?e, "Telegram connection check failed");
                (false, None)
            }
        }
    }

    /// Check if chat_id is configured for notifications
    pub fn has_chat_id(&self) -> bool {
        self.chat_id.is_some()
    }
}

/// Run the Telegram bot with command handling and notification sending
///
/// # Arguments
/// * `config` - Telegram bot configuration
/// * `state` - Shared bot state for command responses
/// * `alert_rx` - Receiver for alert events to send as notifications
/// * `shutdown_token` - Token for graceful shutdown
pub async fn run_telegram_bot(
    config: TelegramConfig,
    state: Arc<RwLock<BotSnapshot>>,
    mut alert_rx: mpsc::Receiver<AlertEvent>,
    shutdown_token: CancellationToken,
) {
    let bot = Bot::new(&config.token);
    let chat_id = Arc::new(RwLock::new(config.chat_id));

    info!("Starting Telegram bot");

    // Track last update ID for long polling
    let mut last_update_id: Option<i64> = None;

    loop {
        tokio::select! {
            biased;

            _ = shutdown_token.cancelled() => {
                info!("Telegram bot shutting down");
                break;
            }

            // Handle incoming alerts to send as notifications
            Some(event) = alert_rx.recv() => {
                if let Some(cid) = *chat_id.read().await {
                    let message = escape_markdown_v2(&format_alert(&event));
                    let silent = is_silent(&event);

                    let mut params = SendMessageParams::builder()
                        .chat_id(cid)
                        .text(&message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .build();

                    if silent {
                        params.disable_notification = Some(true);
                    }

                    if let Err(e) = bot.send_message(&params).await {
                        error!(?e, "Failed to send Telegram notification");
                    }
                }
            }

            // Poll for updates (commands from users)
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
                let mut params = GetUpdatesParams::builder()
                    .timeout(30u32)
                    .build();

                if let Some(id) = last_update_id {
                    params.offset = Some(id + 1);
                }

                match bot.get_updates(&params).await {
                    Ok(response) => {
                        for update in response.result {
                            last_update_id = Some(update.update_id as i64);

                            if let Some(response_text) = handle_update(&update, &state, &chat_id).await {
                                let chat_id_for_reply = extract_chat_id(&update);
                                if let Some(cid) = chat_id_for_reply {
                                    let reply_params = SendMessageParams::builder()
                                        .chat_id(cid)
                                        .text(&response_text)
                                        .parse_mode(ParseMode::MarkdownV2)
                                        .build();

                                    if let Err(e) = bot.send_message(&reply_params).await {
                                        error!(?e, "Failed to send command response");
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(?e, "Failed to get Telegram updates");
                        // Brief pause before retry on error
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

/// Handle a single Telegram update, returning a response message if applicable
async fn handle_update(
    update: &Update,
    state: &Arc<RwLock<BotSnapshot>>,
    chat_id: &Arc<RwLock<Option<i64>>>,
) -> Option<String> {
    let message = match &update.content {
        UpdateContent::Message(msg) => msg,
        _ => return None,
    };

    let text = message.text.as_ref()?;

    // Check if it's a command (starts with /)
    if !text.starts_with('/') {
        return None;
    }

    // Extract command (first word)
    let cmd_text = text.split_whitespace().next()?;

    let command = match Command::from_str(cmd_text) {
        Ok(cmd) => cmd,
        Err(_) => {
            debug!(cmd = %cmd_text, "Unknown command received");
            return Some("Unknown command\\. Use /help to see available commands\\.".to_string());
        }
    };

    let snapshot = state.read().await;

    match command {
        Command::Start => {
            // Store chat_id for future notifications
            let new_chat_id = message.chat.id;
            *chat_id.write().await = Some(new_chat_id);
            info!(chat_id = new_chat_id, "User started bot, notifications enabled");
            Some("*Trading Bot Connected*\n\nYou will now receive notifications\\. Use /help to see available commands\\.".to_string())
        }
        Command::Help => {
            Some(escape_markdown_v2(&Command::descriptions()))
        }
        Command::Status => {
            Some(escape_markdown_v2(&snapshot.format_status()))
        }
        Command::Trades => {
            Some(escape_markdown_v2(&snapshot.format_trades()))
        }
        Command::Balance => {
            Some(escape_markdown_v2(&snapshot.format_balance()))
        }
        Command::Pnl => {
            Some(escape_markdown_v2(&snapshot.format_pnl()))
        }
    }
}

/// Extract chat ID from an update for sending replies
fn extract_chat_id(update: &Update) -> Option<i64> {
    match &update.content {
        UpdateContent::Message(msg) => Some(msg.chat.id),
        _ => None,
    }
}

/// Escape special characters for MarkdownV2 format
fn escape_markdown_v2(text: &str) -> String {
    // MarkdownV2 requires escaping: _ * [ ] ( ) ~ ` > # + - = | { } . !
    let special_chars = ['_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!'];
    let mut result = String::with_capacity(text.len() * 2);

    for c in text.chars() {
        if special_chars.contains(&c) {
            result.push('\\');
        }
        result.push(c);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env() {
        // This test just verifies the function doesn't panic
        // Actual env var testing would need mocking
        let _config = TelegramConfig::from_env();
    }

    #[test]
    fn test_telegram_bot_creation() {
        let config = TelegramConfig {
            token: "test_token".to_string(),
            chat_id: Some(12345),
        };

        let bot = TelegramBot::new(config);
        assert_eq!(bot.chat_id, Some(12345));
    }

    #[test]
    fn test_escape_markdown_v2() {
        let text = "Hello *world* (test)";
        let escaped = escape_markdown_v2(text);
        assert_eq!(escaped, "Hello \\*world\\* \\(test\\)");
    }
}
