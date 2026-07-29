//! Message formatting for Telegram notifications
//!
//! Follows CONTEXT.md decisions:
//! - Emoji for visual scanning: green_check buys, money_bag sells, warning warnings, stop_sign errors
//! - Detailed multi-line messages with full context

use crate::notifications::{AlertEvent, RiskEvent, RiskEventType, SystemEvent, SystemEventType, TradeEvent};

/// Format a trade event for Telegram
pub fn format_trade(trade: &TradeEvent) -> String {
    let emoji = if trade.side.to_lowercase() == "buy" {
        "\u{2705}" // green check mark
    } else {
        "\u{1F4B0}" // money bag
    };

    let pnl_line = trade.pnl
        .map(|p| format!("P&L: `${:+.2}`\n", p))
        .unwrap_or_default();

    format!(
        "{} *{} {}*\n\n\
         Quantity: `{}`\n\
         Price: `${:.2}`\n\
         {}\
         Balance: `${:.2}`",
        emoji,
        trade.side.to_uppercase(),
        trade.symbol,
        trade.quantity,
        trade.price,
        pnl_line,
        trade.balance_after
    )
}

/// Format a risk event for Telegram
pub fn format_risk_event(event: &RiskEvent) -> String {
    let emoji = match event.event_type {
        RiskEventType::StopLossTriggered => "\u{1F6D1}", // stop sign
        RiskEventType::DailyLimitReached => "\u{26A0}\u{FE0F}", // warning
        RiskEventType::CircuitBreakerTriggered => "\u{1F6A8}", // rotating light
    };

    let symbol_line = event.symbol
        .as_ref()
        .map(|s| format!("Symbol: `{}`\n", s))
        .unwrap_or_default();

    let drawdown_line = event.drawdown_pct
        .map(|d| format!("Drawdown: `{:.2}%`\n", d * rust_decimal::Decimal::ONE_HUNDRED))
        .unwrap_or_default();

    format!(
        "{} *{}*\n\n\
         {}\
         {}\
         {}",
        emoji,
        event.event_type,
        event.message,
        symbol_line,
        drawdown_line
    )
}

/// Format a system event for Telegram
pub fn format_system_event(event: &SystemEvent) -> String {
    let emoji = match event.event_type {
        SystemEventType::BotStarted => "\u{1F7E2}", // green circle
        SystemEventType::BotStopped => "\u{1F534}", // red circle
        SystemEventType::WebSocketReconnect => "\u{1F504}", // arrows counterclockwise
        SystemEventType::Error => "\u{1F6D1}", // stop sign
    };

    format!(
        "{} *{}*\n\n{}",
        emoji,
        event.event_type,
        event.message
    )
}

/// Format an alert event for Telegram
pub fn format_alert(event: &AlertEvent) -> String {
    match event {
        AlertEvent::Trade(t) => format_trade(t),
        AlertEvent::Risk(r) => format_risk_event(r),
        AlertEvent::System(s) => format_system_event(s),
    }
}

/// Check if an alert should be sent silently (no notification sound)
pub fn is_silent(event: &AlertEvent) -> bool {
    // Per CONTEXT.md: trades silent, errors with sound
    !event.is_urgent()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rust_decimal_macros::dec;

    #[test]
    fn test_format_trade_buy() {
        let trade = TradeEvent {
            id: "123".to_string(),
            symbol: "BTCUSDT".to_string(),
            side: "buy".to_string(),
            quantity: dec!(0.1),
            price: dec!(50000.00),
            balance_after: dec!(5000.00),
            pnl: None,
            timestamp: Utc::now(),
        };

        let msg = format_trade(&trade);
        assert!(msg.contains("BUY"));
        assert!(msg.contains("BTCUSDT"));
        assert!(msg.contains("0.1"));
    }

    #[test]
    fn test_format_trade_sell_with_pnl() {
        let trade = TradeEvent {
            id: "124".to_string(),
            symbol: "BTCUSDT".to_string(),
            side: "sell".to_string(),
            quantity: dec!(0.1),
            price: dec!(51000.00),
            balance_after: dec!(6100.00),
            pnl: Some(dec!(100.00)),
            timestamp: Utc::now(),
        };

        let msg = format_trade(&trade);
        assert!(msg.contains("SELL"));
        assert!(msg.contains("P&L"));
    }

    #[test]
    fn test_format_risk_event() {
        let event = RiskEvent {
            event_type: RiskEventType::StopLossTriggered,
            message: "Stop loss triggered at -5%".to_string(),
            symbol: Some("BTCUSDT".to_string()),
            drawdown_pct: None,
            timestamp: Utc::now(),
        };

        let msg = format_risk_event(&event);
        assert!(msg.contains("Stop-Loss Triggered"));
        assert!(msg.contains("BTCUSDT"));
    }

    #[test]
    fn test_is_silent() {
        let trade = AlertEvent::trade(
            "1", "BTCUSDT", "buy",
            dec!(1), dec!(50000), dec!(5000), None
        );
        assert!(is_silent(&trade));

        let risk = AlertEvent::risk(
            RiskEventType::StopLossTriggered,
            "Stop triggered"
        );
        assert!(!is_silent(&risk));
    }
}
