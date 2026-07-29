//! Binance message types for WebSocket streams
//!
//! These types match the Binance.US WebSocket API format exactly.
//! See: https://github.com/binance-us/binance-us-api-docs/blob/master/web-socket-streams.md

use serde::Deserialize;

/// Trade event from btcusdt@trade stream
///
/// Binance uses single-letter JSON keys for bandwidth efficiency.
/// We use serde rename to map to readable Rust field names.
///
/// Example JSON:
/// ```json
/// {
///   "e": "trade",
///   "E": 123456789,
///   "s": "BTCUSDT",
///   "t": 12345,
///   "p": "50000.00",
///   "q": "0.001",
///   "T": 123456785,
///   "m": true
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct TradeEvent {
    /// Event type - always "trade"
    #[serde(rename = "e")]
    pub event_type: String,

    /// Event timestamp (Unix ms)
    #[serde(rename = "E")]
    pub event_time: u64,

    /// Trading pair symbol (e.g., "BTCUSDT")
    #[serde(rename = "s")]
    pub symbol: String,

    /// Unique trade ID
    #[serde(rename = "t")]
    pub trade_id: u64,

    /// Trade price as string (preserves precision for financial calculations)
    #[serde(rename = "p")]
    pub price: String,

    /// Trade quantity as string (preserves precision)
    #[serde(rename = "q")]
    pub quantity: String,

    /// Trade timestamp (Unix ms)
    #[serde(rename = "T")]
    pub trade_time: u64,

    /// Is the buyer the market maker?
    /// true = sell order was filled (buyer was maker)
    /// false = buy order was filled (seller was maker)
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,
}

impl TradeEvent {
    /// Check if this is a buy (taker bought) or sell (taker sold)
    pub fn is_buy(&self) -> bool {
        // When is_buyer_maker is true, the SELLER was the taker (they hit the bid)
        // When is_buyer_maker is false, the BUYER was the taker (they lifted the ask)
        !self.is_buyer_maker
    }
}

/// Subscription response from Binance WebSocket
#[derive(Debug, Deserialize)]
pub struct SubscriptionResponse {
    /// null on success, error message on failure
    pub result: Option<serde_json::Value>,
    /// Request ID echoed back
    pub id: u64,
}

impl SubscriptionResponse {
    /// Check if subscription was successful
    pub fn is_success(&self) -> bool {
        self.result.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_trade_event() {
        let json = r#"{
            "e": "trade",
            "E": 1672531200000,
            "s": "BTCUSDT",
            "t": 123456789,
            "p": "16500.50",
            "q": "0.00100000",
            "T": 1672531200001,
            "m": false
        }"#;

        let event: TradeEvent = serde_json::from_str(json).unwrap();

        assert_eq!(event.event_type, "trade");
        assert_eq!(event.symbol, "BTCUSDT");
        assert_eq!(event.price, "16500.50");
        assert_eq!(event.quantity, "0.00100000");
        assert!(event.is_buy()); // is_buyer_maker=false means buyer was taker
    }

    #[test]
    fn test_parse_subscription_response_success() {
        let json = r#"{"result": null, "id": 1}"#;
        let response: SubscriptionResponse = serde_json::from_str(json).unwrap();
        assert!(response.is_success());
    }

    #[test]
    fn test_parse_subscription_response_error() {
        let json = r#"{"result": "Invalid symbol", "id": 1}"#;
        let response: SubscriptionResponse = serde_json::from_str(json).unwrap();
        assert!(!response.is_success());
    }
}
