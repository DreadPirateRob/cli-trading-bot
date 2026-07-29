//! Exchange connectivity error types

use thiserror::Error;

/// Errors from exchange connectivity operations
#[derive(Error, Debug)]
pub enum ExchangeError {
    #[error("WebSocket connection failed: {0}")]
    ConnectionFailed(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("Failed to parse WebSocket URL '{url}': {source}")]
    InvalidUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },

    #[error("Subscription to '{stream}' failed: {reason}")]
    SubscriptionFailed { stream: String, reason: String },

    #[error("Failed to parse message: {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("WebSocket closed unexpectedly: {reason}")]
    ConnectionClosed { reason: String },

    #[error("Send failed: {0}")]
    SendFailed(String),
}
