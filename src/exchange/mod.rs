//! Exchange connectivity module for Binance.US
//!
//! This module provides WebSocket connectivity to Binance.US for real-time
//! trade data streaming.

pub mod client;
pub mod credentials;
pub mod error;
pub mod health;
pub mod messages;
pub mod reconnect;

pub use client::{connect_trade_stream, TradeStreamHandle, BINANCE_WS_URL};
pub use credentials::{Credentials, CredentialsError};
pub use error::ExchangeError;
pub use health::ConnectionHealth;
pub use messages::TradeEvent;
pub use reconnect::{
    start_reconnecting_stream, ConnectionStatus, ReconnectConfig, ReconnectingTradeStream,
};
