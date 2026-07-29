//! Telegram notification system
//!
//! Provides Telegram bot integration for:
//! - On-demand commands (/status, /trades, /balance, /pnl)
//! - Proactive notifications for trades and alerts
//!
//! Requires TELOXIDE_TOKEN environment variable with bot token from @BotFather.

mod commands;
mod messages;
mod telegram;

pub use commands::{Command, BotSnapshot, TradeSnapshot, BalanceSnapshot};
pub use messages::{format_trade, format_risk_event, format_system_event};
pub use telegram::{TelegramBot, TelegramConfig, run_telegram_bot};
