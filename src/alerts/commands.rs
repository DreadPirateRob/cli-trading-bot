//! Telegram command definitions and handlers

use std::str::FromStr;

/// Telegram bot commands
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Show help message
    Help,
    /// Show current bot status
    Status,
    /// Show recent trades
    Trades,
    /// Show portfolio balances
    Balance,
    /// Show today's P&L
    Pnl,
    /// Start the bot (required for Telegram to enable messaging)
    Start,
}

impl Command {
    /// Get command description for help message
    pub fn description(&self) -> &'static str {
        match self {
            Command::Help => "Show this help message",
            Command::Status => "Show current bot status, equity, and strategy",
            Command::Trades => "Show recent trade history",
            Command::Balance => "Show current portfolio balances",
            Command::Pnl => "Show today's profit/loss summary",
            Command::Start => "Start receiving notifications",
        }
    }

    /// Get all commands with descriptions for help output
    pub fn descriptions() -> String {
        let commands = [
            Command::Help,
            Command::Status,
            Command::Trades,
            Command::Balance,
            Command::Pnl,
            Command::Start,
        ];

        let mut result = String::from("Trading bot commands:\n\n");
        for cmd in commands {
            result.push_str(&format!("/{} - {}\n", cmd.as_str(), cmd.description()));
        }
        result
    }

    /// Get the command string
    pub fn as_str(&self) -> &'static str {
        match self {
            Command::Help => "help",
            Command::Status => "status",
            Command::Trades => "trades",
            Command::Balance => "balance",
            Command::Pnl => "pnl",
            Command::Start => "start",
        }
    }
}

impl FromStr for Command {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Strip leading slash if present
        let cmd = s.strip_prefix('/').unwrap_or(s).to_lowercase();

        match cmd.as_str() {
            "help" => Ok(Command::Help),
            "status" => Ok(Command::Status),
            "trades" => Ok(Command::Trades),
            "balance" => Ok(Command::Balance),
            "pnl" => Ok(Command::Pnl),
            "start" => Ok(Command::Start),
            _ => Err(()),
        }
    }
}

/// Bot state snapshot for command responses
#[derive(Debug, Clone)]
pub struct BotSnapshot {
    /// Current status: "running", "stopped", "error"
    pub status: String,
    /// Active strategy name
    pub strategy: String,
    /// Paper mode flag
    pub paper_mode: bool,
    /// Uptime in seconds
    pub uptime_seconds: u64,
    /// Current equity
    pub equity: rust_decimal::Decimal,
    /// Unrealized P&L
    pub unrealized_pnl: rust_decimal::Decimal,
    /// Daily P&L
    pub daily_pnl: rust_decimal::Decimal,
    /// Recent trades (last 10)
    pub recent_trades: Vec<TradeSnapshot>,
    /// Portfolio balances
    pub balances: Vec<BalanceSnapshot>,
}

impl Default for BotSnapshot {
    fn default() -> Self {
        Self {
            status: "initializing".to_string(),
            strategy: "none".to_string(),
            paper_mode: true,
            uptime_seconds: 0,
            equity: rust_decimal::Decimal::ZERO,
            unrealized_pnl: rust_decimal::Decimal::ZERO,
            daily_pnl: rust_decimal::Decimal::ZERO,
            recent_trades: Vec::new(),
            balances: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TradeSnapshot {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub symbol: String,
    pub side: String,
    pub quantity: rust_decimal::Decimal,
    pub price: rust_decimal::Decimal,
    pub pnl: Option<rust_decimal::Decimal>,
}

#[derive(Debug, Clone)]
pub struct BalanceSnapshot {
    pub asset: String,
    pub free: rust_decimal::Decimal,
    pub locked: rust_decimal::Decimal,
}

impl BotSnapshot {
    /// Format status response
    pub fn format_status(&self) -> String {
        let mode = if self.paper_mode { "PAPER" } else { "LIVE" };
        let uptime = format_uptime(self.uptime_seconds);

        format!(
            "*Bot Status*\n\n\
             Mode: `{}`\n\
             Status: `{}`\n\
             Strategy: `{}`\n\
             Uptime: `{}`\n\n\
             *Equity*: `${:.2}`\n\
             Unrealized P&L: `${:+.2}`\n\
             Daily P&L: `${:+.2}`",
            mode,
            self.status,
            self.strategy,
            uptime,
            self.equity,
            self.unrealized_pnl,
            self.daily_pnl
        )
    }

    /// Format trades response
    pub fn format_trades(&self) -> String {
        if self.recent_trades.is_empty() {
            return "*Recent Trades*\n\nNo trades yet.".to_string();
        }

        let mut msg = "*Recent Trades*\n\n".to_string();
        for trade in self.recent_trades.iter().take(10) {
            let side_indicator = if trade.side.to_lowercase() == "buy" { "BUY" } else { "SELL" };
            let pnl_str = trade.pnl
                .map(|p| format!(" (P&L: ${:+.2})", p))
                .unwrap_or_default();

            msg.push_str(&format!(
                "{} {} {} {} @ ${:.2}{}\n",
                trade.timestamp.format("%H:%M"),
                side_indicator,
                trade.quantity,
                trade.symbol,
                trade.price,
                pnl_str
            ));
        }
        msg
    }

    /// Format balance response
    pub fn format_balance(&self) -> String {
        if self.balances.is_empty() {
            return "*Portfolio Balances*\n\nNo balances available.".to_string();
        }

        let mut msg = "*Portfolio Balances*\n\n".to_string();
        for balance in &self.balances {
            if balance.free > rust_decimal::Decimal::ZERO || balance.locked > rust_decimal::Decimal::ZERO {
                msg.push_str(&format!(
                    "*{}*: `{:.8}` (locked: `{:.8}`)\n",
                    balance.asset,
                    balance.free,
                    balance.locked
                ));
            }
        }
        msg
    }

    /// Format P&L response
    pub fn format_pnl(&self) -> String {
        let trend = if self.daily_pnl >= rust_decimal::Decimal::ZERO { "UP" } else { "DOWN" };

        format!(
            "*Today's P&L* ({})\n\n\
             Daily P&L: `${:+.2}`\n\
             Unrealized: `${:+.2}`\n\
             Total Equity: `${:.2}`",
            trend,
            self.daily_pnl,
            self.unrealized_pnl,
            self.equity
        )
    }
}

fn format_uptime(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_parsing() {
        assert_eq!(Command::from_str("/help"), Ok(Command::Help));
        assert_eq!(Command::from_str("help"), Ok(Command::Help));
        assert_eq!(Command::from_str("/STATUS"), Ok(Command::Status));
        assert_eq!(Command::from_str("unknown"), Err(()));
    }

    #[test]
    fn test_descriptions() {
        let desc = Command::descriptions();
        assert!(desc.contains("/help"));
        assert!(desc.contains("/status"));
        assert!(desc.contains("/trades"));
    }
}
