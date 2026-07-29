//! Shared notification types for TUI and Telegram

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Events that trigger notifications across all channels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertEvent {
    /// Trade was executed (buy or sell)
    Trade(TradeEvent),
    /// Risk event occurred (stop-loss, daily limit, circuit breaker)
    Risk(RiskEvent),
    /// System event (start/stop, reconnect, error)
    System(SystemEvent),
}

/// Trade execution event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeEvent {
    /// Trade ID
    pub id: String,
    /// Trading pair symbol (e.g., "BTCUSDT")
    pub symbol: String,
    /// Order side: "buy" or "sell"
    pub side: String,
    /// Trade quantity
    pub quantity: Decimal,
    /// Execution price
    pub price: Decimal,
    /// Balance after trade (in quote currency)
    pub balance_after: Decimal,
    /// Realized P&L (for closing trades)
    pub pnl: Option<Decimal>,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

/// Risk-related event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskEvent {
    /// Type of risk event
    pub event_type: RiskEventType,
    /// Human-readable message
    pub message: String,
    /// Associated symbol (if applicable)
    pub symbol: Option<String>,
    /// Current drawdown percentage (if applicable)
    pub drawdown_pct: Option<Decimal>,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

/// Types of risk events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskEventType {
    /// Stop-loss was triggered
    StopLossTriggered,
    /// Daily loss limit reached, trading halted
    DailyLimitReached,
    /// Circuit breaker triggered due to drawdown
    CircuitBreakerTriggered,
}

/// System-level event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemEvent {
    /// Type of system event
    pub event_type: SystemEventType,
    /// Human-readable message
    pub message: String,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

/// Types of system events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemEventType {
    /// Bot started
    BotStarted,
    /// Bot stopped (graceful shutdown)
    BotStopped,
    /// WebSocket reconnected after disconnect
    WebSocketReconnect,
    /// Error occurred
    Error,
}

impl AlertEvent {
    /// Create a trade event
    pub fn trade(
        id: impl Into<String>,
        symbol: impl Into<String>,
        side: impl Into<String>,
        quantity: Decimal,
        price: Decimal,
        balance_after: Decimal,
        pnl: Option<Decimal>,
    ) -> Self {
        Self::Trade(TradeEvent {
            id: id.into(),
            symbol: symbol.into(),
            side: side.into(),
            quantity,
            price,
            balance_after,
            pnl,
            timestamp: Utc::now(),
        })
    }

    /// Create a risk event
    pub fn risk(event_type: RiskEventType, message: impl Into<String>) -> Self {
        Self::Risk(RiskEvent {
            event_type,
            message: message.into(),
            symbol: None,
            drawdown_pct: None,
            timestamp: Utc::now(),
        })
    }

    /// Create a risk event with symbol
    pub fn risk_with_symbol(
        event_type: RiskEventType,
        message: impl Into<String>,
        symbol: impl Into<String>,
    ) -> Self {
        Self::Risk(RiskEvent {
            event_type,
            message: message.into(),
            symbol: Some(symbol.into()),
            drawdown_pct: None,
            timestamp: Utc::now(),
        })
    }

    /// Create a circuit breaker event with drawdown
    pub fn circuit_breaker(message: impl Into<String>, drawdown_pct: Decimal) -> Self {
        Self::Risk(RiskEvent {
            event_type: RiskEventType::CircuitBreakerTriggered,
            message: message.into(),
            symbol: None,
            drawdown_pct: Some(drawdown_pct),
            timestamp: Utc::now(),
        })
    }

    /// Create a system event
    pub fn system(event_type: SystemEventType, message: impl Into<String>) -> Self {
        Self::System(SystemEvent {
            event_type,
            message: message.into(),
            timestamp: Utc::now(),
        })
    }

    /// Check if this is an error-level event (should have notification sound)
    pub fn is_urgent(&self) -> bool {
        match self {
            AlertEvent::Trade(_) => false, // Trades are silent
            AlertEvent::Risk(_) => true,   // All risk events are urgent
            AlertEvent::System(sys) => matches!(sys.event_type, SystemEventType::Error),
        }
    }

    /// Get emoji for this event type (for Telegram formatting)
    pub fn emoji(&self) -> &'static str {
        match self {
            AlertEvent::Trade(t) if t.side.to_lowercase() == "buy" => "green_check",
            AlertEvent::Trade(_) => "money_bag",
            AlertEvent::Risk(r) => match r.event_type {
                RiskEventType::StopLossTriggered => "stop_sign",
                RiskEventType::DailyLimitReached => "warning",
                RiskEventType::CircuitBreakerTriggered => "rotating_light",
            },
            AlertEvent::System(s) => match s.event_type {
                SystemEventType::BotStarted => "green_circle",
                SystemEventType::BotStopped => "red_circle",
                SystemEventType::WebSocketReconnect => "arrows_counterclockwise",
                SystemEventType::Error => "stop_sign",
            },
        }
    }
}

impl std::fmt::Display for RiskEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskEventType::StopLossTriggered => write!(f, "Stop-Loss Triggered"),
            RiskEventType::DailyLimitReached => write!(f, "Daily Limit Reached"),
            RiskEventType::CircuitBreakerTriggered => write!(f, "Circuit Breaker Triggered"),
        }
    }
}

impl std::fmt::Display for SystemEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemEventType::BotStarted => write!(f, "Bot Started"),
            SystemEventType::BotStopped => write!(f, "Bot Stopped"),
            SystemEventType::WebSocketReconnect => write!(f, "WebSocket Reconnected"),
            SystemEventType::Error => write!(f, "Error"),
        }
    }
}
