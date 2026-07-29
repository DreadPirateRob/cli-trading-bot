//! Core execution types and OrderExecutor trait
//!
//! Defines the abstractions for order execution, including request/response
//! types, order status, and the OrderExecutor trait that all executors implement.

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::data::OrderSide;

/// Request to execute an order
#[derive(Debug, Clone)]
pub struct ExecuteOrderRequest {
    /// Trading pair symbol (e.g., "BTCUSDT")
    pub symbol: String,
    /// Buy or sell
    pub side: OrderSide,
    /// Quantity to trade
    pub quantity: Decimal,
    /// Order type (market or limit)
    pub order_type: OrderType,
    /// Limit price (required for limit orders)
    pub price: Option<Decimal>,
    /// Client-defined order ID for tracking
    pub client_order_id: Option<String>,
}

/// Response from order execution
#[derive(Debug, Clone)]
pub struct ExecuteOrderResponse {
    /// Exchange-assigned order ID
    pub order_id: String,
    /// Client-defined order ID
    pub client_order_id: String,
    /// Trading pair symbol
    pub symbol: String,
    /// Order side
    pub side: OrderSide,
    /// Quantity that was filled
    pub filled_quantity: Decimal,
    /// Average fill price
    pub fill_price: Decimal,
    /// Order status
    pub status: OrderStatus,
    /// Execution timestamp (milliseconds since epoch)
    pub timestamp: i64,
    /// Individual fill details
    pub fills: Vec<Fill>,
}

/// Individual fill within an order
#[derive(Debug, Clone)]
pub struct Fill {
    /// Fill price
    pub price: Decimal,
    /// Fill quantity
    pub quantity: Decimal,
    /// Commission charged
    pub commission: Decimal,
    /// Asset used for commission (e.g., "USDT", "BNB")
    pub commission_asset: String,
}

/// Order status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    /// Order accepted, not yet filled
    New,
    /// Order partially filled
    PartiallyFilled,
    /// Order completely filled
    Filled,
    /// Order canceled by user
    Canceled,
    /// Order rejected by exchange
    Rejected,
    /// Order expired (for time-limited orders)
    Expired,
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderStatus::New => write!(f, "NEW"),
            OrderStatus::PartiallyFilled => write!(f, "PARTIALLY_FILLED"),
            OrderStatus::Filled => write!(f, "FILLED"),
            OrderStatus::Canceled => write!(f, "CANCELED"),
            OrderStatus::Rejected => write!(f, "REJECTED"),
            OrderStatus::Expired => write!(f, "EXPIRED"),
        }
    }
}

/// Order type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    /// Market order - executes at current market price
    Market,
    /// Limit order - executes at specified price or better
    Limit,
}

impl std::fmt::Display for OrderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderType::Market => write!(f, "MARKET"),
            OrderType::Limit => write!(f, "LIMIT"),
        }
    }
}

/// Errors that can occur during order execution
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// Exchange API error
    #[error("Exchange error {code}: {msg}")]
    Exchange {
        /// Error code from exchange
        code: i32,
        /// Error message from exchange
        msg: String,
    },

    /// Network/HTTP error
    #[error("Network error: {0}")]
    Network(String),

    /// HMAC signing error
    #[error("Signing error: {0}")]
    Signing(String),

    /// Insufficient balance to execute order
    #[error("Insufficient {asset} balance: required {required}, available {available}")]
    InsufficientBalance {
        /// Asset that is insufficient
        asset: String,
        /// Required amount
        required: Decimal,
        /// Available amount
        available: Decimal,
    },

    /// Rate limited by exchange
    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited {
        /// Milliseconds until rate limit resets
        retry_after_ms: u64,
    },

    /// Invalid request parameters
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
}

/// Trait for order execution implementations
///
/// Implemented by both paper trading (simulated) and live (real) executors.
#[async_trait]
pub trait OrderExecutor: Send + Sync {
    /// Execute an order
    async fn execute(
        &self,
        request: ExecuteOrderRequest,
    ) -> Result<ExecuteOrderResponse, ExecutionError>;

    /// Get balance for an asset
    async fn get_balance(&self, asset: &str) -> Result<Decimal, ExecutionError>;

    /// Get status of an existing order
    async fn get_order_status(
        &self,
        order_id: &str,
        symbol: &str,
    ) -> Result<OrderStatus, ExecutionError>;
}
