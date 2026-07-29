//! TUI application state (Model in Elm Architecture)

use std::collections::VecDeque;
use rust_decimal::Decimal;
use serde_json::Value;

/// TUI application state
pub struct TuiApp {
    /// Whether the TUI is running
    pub running: bool,
    /// Current bot status: "running", "stopped", "error"
    pub bot_status: String,
    /// Current total equity
    pub equity: Decimal,
    /// Unrealized P&L
    pub unrealized_pnl: Decimal,
    /// Daily P&L
    pub daily_pnl: Decimal,
    /// Active strategy name
    pub strategy_name: String,
    /// Strategy internal state (JSON for flexibility)
    pub strategy_state: Value,
    /// Recent trades (newest first, max 50)
    pub recent_trades: VecDeque<TradeInfo>,
    /// Recent alerts (newest first, max 20)
    pub alerts: VecDeque<AlertInfo>,
    /// Paper mode flag
    pub paper_mode: bool,
    /// Bot uptime in seconds
    pub uptime_seconds: u64,
}

/// Trade information for display
#[derive(Debug, Clone)]
pub struct TradeInfo {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub symbol: String,
    pub side: String,
    pub quantity: Decimal,
    pub price: Decimal,
    pub pnl: Option<Decimal>,
}

/// Alert information for display
#[derive(Debug, Clone)]
pub struct AlertInfo {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub level: AlertLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlertLevel {
    Info,
    Warning,
    Error,
}

impl TuiApp {
    /// Create new TUI app with default state
    pub fn new() -> Self {
        Self {
            running: true,
            bot_status: "stopped".to_string(),
            equity: Decimal::ZERO,
            unrealized_pnl: Decimal::ZERO,
            daily_pnl: Decimal::ZERO,
            strategy_name: "none".to_string(),
            strategy_state: Value::Null,
            recent_trades: VecDeque::with_capacity(50),
            alerts: VecDeque::with_capacity(20),
            paper_mode: true,
            uptime_seconds: 0,
        }
    }

    /// Add a trade, keeping only the last 50
    pub fn add_trade(&mut self, trade: TradeInfo) {
        if self.recent_trades.len() >= 50 {
            self.recent_trades.pop_back();
        }
        self.recent_trades.push_front(trade);
    }

    /// Add an alert, keeping only the last 20
    pub fn add_alert(&mut self, alert: AlertInfo) {
        if self.alerts.len() >= 20 {
            self.alerts.pop_back();
        }
        self.alerts.push_front(alert);
    }

    /// Signal the app to quit
    pub fn quit(&mut self) {
        self.running = false;
    }

    /// Update from a BotUpdate message
    pub fn apply_bot_update(&mut self, update: &crate::api::types::BotUpdate) {
        use crate::api::types::BotUpdate;

        match update {
            BotUpdate::Status(status) => {
                self.bot_status = status.status.clone();
                self.strategy_name = status.active_strategy.clone();
            }
            BotUpdate::Metrics(metrics) => {
                self.equity = metrics.equity;
                self.daily_pnl = metrics.daily_pnl;
            }
            BotUpdate::Trade(trade) => {
                self.add_trade(TradeInfo {
                    timestamp: chrono::Utc::now(),
                    symbol: trade.symbol.clone(),
                    side: trade.side.clone(),
                    quantity: trade.quantity,
                    price: trade.price,
                    pnl: trade.pnl,
                });
            }
            BotUpdate::Position(pos) => {
                self.unrealized_pnl = pos.unrealized_pnl;
            }
            BotUpdate::Price(_) => {
                // Price updates handled elsewhere
            }
            BotUpdate::Alert(alert) => {
                // Apply alert directly
                self.apply_alert(alert);
            }
            BotUpdate::Equity(eq) => {
                // Update equity from equity tracker
                self.equity = eq.equity;
            }
        }
    }

    /// Apply an alert event
    pub fn apply_alert(&mut self, alert: &crate::notifications::AlertEvent) {
        use crate::notifications::AlertEvent;

        let (level, message) = match alert {
            AlertEvent::Trade(t) => (
                AlertLevel::Info,
                format!("{} {} {} @ {}", t.side, t.quantity, t.symbol, t.price),
            ),
            AlertEvent::Risk(r) => (
                AlertLevel::Warning,
                r.message.clone(),
            ),
            AlertEvent::System(s) => {
                let level = if matches!(s.event_type, crate::notifications::SystemEventType::Error) {
                    AlertLevel::Error
                } else {
                    AlertLevel::Info
                };
                (level, s.message.clone())
            }
        };

        self.add_alert(AlertInfo {
            timestamp: chrono::Utc::now(),
            level,
            message,
        });
    }

    /// Create TUI app with sample data for standalone mode demonstration
    pub fn with_sample_data() -> Self {
        use std::str::FromStr;

        let mut app = Self::new();
        app.bot_status = "running".to_string();
        app.paper_mode = true;
        app.strategy_name = "RSI Crossover".to_string();
        app.equity = Decimal::from_str("10000.00").unwrap();
        app.unrealized_pnl = Decimal::from_str("150.00").unwrap();
        app.daily_pnl = Decimal::from_str("75.50").unwrap();
        app.strategy_state = serde_json::json!({
            "rsi_period": 14,
            "oversold": 30,
            "overbought": 70,
            "position_open": false
        });

        // Add sample trades
        app.add_trade(TradeInfo {
            timestamp: chrono::Utc::now() - chrono::Duration::minutes(30),
            symbol: "BTCUSDT".to_string(),
            side: "buy".to_string(),
            quantity: Decimal::from_str("0.1").unwrap(),
            price: Decimal::from_str("50000.00").unwrap(),
            pnl: None,
        });
        app.add_trade(TradeInfo {
            timestamp: chrono::Utc::now() - chrono::Duration::minutes(15),
            symbol: "BTCUSDT".to_string(),
            side: "sell".to_string(),
            quantity: Decimal::from_str("0.1").unwrap(),
            price: Decimal::from_str("50150.00").unwrap(),
            pnl: Some(Decimal::from_str("15.00").unwrap()),
        });

        // Add sample alert
        app.add_alert(AlertInfo {
            timestamp: chrono::Utc::now() - chrono::Duration::minutes(5),
            level: AlertLevel::Info,
            message: "Bot started in paper mode".to_string(),
        });

        app
    }
}

impl Default for TuiApp {
    fn default() -> Self {
        Self::new()
    }
}
