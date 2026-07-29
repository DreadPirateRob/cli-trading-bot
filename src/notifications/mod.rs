//! Notification system for alerting via TUI, Telegram, and web dashboard
//!
//! Provides unified AlertEvent types and broadcasting infrastructure
//! for routing events to multiple consumers.

mod types;
mod broadcaster;

pub use types::{AlertEvent, RiskEvent, RiskEventType, SystemEvent, SystemEventType, TradeEvent};
pub use broadcaster::{AlertBroadcaster, AlertSender};
