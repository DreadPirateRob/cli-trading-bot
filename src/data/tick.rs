//! Tick data structures for database persistence
//!
//! Converts WebSocket TradeEvent (String prices) to database-ready
//! Tick structs (Decimal prices) without precision loss.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::FromRow;
use std::str::FromStr;

use crate::exchange::messages::TradeEvent;

/// Tick data as stored in database (includes auto-generated fields)
#[derive(Debug, Clone, FromRow)]
pub struct Tick {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub price: Decimal,
    pub quantity: Decimal,
    pub trade_id: i64,
    pub is_buyer_maker: bool,
    pub created_at: DateTime<Utc>,
}

/// Tick data for insertion (no id or created_at - database generates these)
#[derive(Debug, Clone)]
pub struct NewTick {
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub price: Decimal,
    pub quantity: Decimal,
    pub trade_id: i64,
    pub is_buyer_maker: bool,
}

/// Error converting TradeEvent to NewTick
#[derive(Debug, thiserror::Error)]
pub enum TickConversionError {
    #[error("Invalid price format '{0}': {1}")]
    InvalidPrice(String, rust_decimal::Error),

    #[error("Invalid quantity format '{0}': {1}")]
    InvalidQuantity(String, rust_decimal::Error),

    #[error("Invalid timestamp {0}: out of range")]
    InvalidTimestamp(u64),
}

impl TryFrom<&TradeEvent> for NewTick {
    type Error = TickConversionError;

    fn try_from(event: &TradeEvent) -> Result<Self, Self::Error> {
        let price = Decimal::from_str(&event.price)
            .map_err(|e| TickConversionError::InvalidPrice(event.price.clone(), e))?;

        let quantity = Decimal::from_str(&event.quantity)
            .map_err(|e| TickConversionError::InvalidQuantity(event.quantity.clone(), e))?;

        let timestamp = DateTime::from_timestamp_millis(event.trade_time as i64)
            .ok_or(TickConversionError::InvalidTimestamp(event.trade_time))?;

        Ok(Self {
            timestamp,
            symbol: event.symbol.clone(),
            price,
            quantity,
            trade_id: event.trade_id as i64,
            is_buyer_maker: event.is_buyer_maker,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trade_event() -> TradeEvent {
        TradeEvent {
            event_type: "trade".to_string(),
            event_time: 1672531200000,
            symbol: "BTCUSDT".to_string(),
            trade_id: 123456789,
            price: "50000.12345678".to_string(),
            quantity: "0.00100000".to_string(),
            trade_time: 1672531200001,
            is_buyer_maker: false,
        }
    }

    #[test]
    fn test_convert_trade_event_to_tick() {
        let event = sample_trade_event();
        let tick = NewTick::try_from(&event).unwrap();

        assert_eq!(tick.symbol, "BTCUSDT");
        assert_eq!(tick.price.to_string(), "50000.12345678");
        assert_eq!(tick.quantity.to_string(), "0.00100000");
        assert_eq!(tick.trade_id, 123456789);
        assert!(!tick.is_buyer_maker);
    }

    #[test]
    fn test_preserves_decimal_precision() {
        // Ensure no floating point precision loss
        let event = TradeEvent {
            price: "0.00000001".to_string(),
            quantity: "123456789.12345678".to_string(),
            ..sample_trade_event()
        };

        let tick = NewTick::try_from(&event).unwrap();

        // These would fail with f64 due to precision loss
        assert_eq!(tick.price.to_string(), "0.00000001");
        assert_eq!(tick.quantity.to_string(), "123456789.12345678");
    }

    #[test]
    fn test_invalid_price_format() {
        let event = TradeEvent {
            price: "not_a_number".to_string(),
            ..sample_trade_event()
        };

        let result = NewTick::try_from(&event);
        assert!(matches!(result, Err(TickConversionError::InvalidPrice(_, _))));
    }

    #[test]
    fn test_timestamp_conversion() {
        let event = sample_trade_event();
        let tick = NewTick::try_from(&event).unwrap();

        // 1672531200001 ms = 2023-01-01 00:00:00.001 UTC
        assert_eq!(tick.timestamp.timestamp_millis(), 1672531200001);
    }
}
