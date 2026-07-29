//! Order execution system
//!
//! Provides order execution capabilities with both live (Binance.US) and
//! paper trading implementations, balance and position management, and
//! the central OrderExecutorActor.
//!
//! # Modules
//!
//! - `types`: Core types (OrderExecutor trait, request/response structs, errors)
//! - `signing`: HMAC-SHA256 signing for Binance API authentication
//! - `live`: Live order execution via Binance.US REST API
//! - `paper`: Paper trading (simulated) executor
//! - `balance`: Balance tracking and reservation management
//! - `position`: Position tracking with FIFO closing
//! - `executor`: Central OrderExecutorActor for signal and stop-loss handling
//!
//! # Example
//!
//! ```ignore
//! use trading_bot::execution::{OrderExecutor, ExecuteOrderRequest, OrderType};
//! use trading_bot::data::OrderSide;
//! use rust_decimal_macros::dec;
//!
//! let request = ExecuteOrderRequest {
//!     symbol: "BTCUSDT".to_string(),
//!     side: OrderSide::Buy,
//!     quantity: dec!(0.001),
//!     order_type: OrderType::Market,
//!     price: None,
//!     client_order_id: None,
//! };
//! ```

pub mod balance;
pub mod equity_tracker;
pub mod executor;
pub mod live;
pub mod paper;
pub mod position;
pub mod signing;
pub mod types;

pub use balance::{BalanceManager, ReserveConfig};
pub use executor::{
    start_order_executor, ExecutionMessage, ExecutionResult, ExecutorStartConfig,
    OrderExecutorHandle,
};
pub use live::{LiveExecutor, BINANCE_US_API};
pub use paper::PaperExecutor;
pub use position::{ClosedPosition, ExecutionPosition, PositionManager};
pub use signing::{build_signed_params, sign_request};
pub use types::{
    ExecuteOrderRequest, ExecuteOrderResponse, ExecutionError, Fill, OrderExecutor, OrderStatus,
    OrderType,
};
pub use equity_tracker::{EquityTrackerConfig, EquityTrackerHandle, start_equity_tracker};
