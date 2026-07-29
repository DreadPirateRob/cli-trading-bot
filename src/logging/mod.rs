//! Structured logging infrastructure for the trading bot.
//!
//! Provides JSON-formatted file logs with daily rotation and
//! optional pretty-printed stdout output for development.

mod setup;

pub use setup::{setup_logging, LogConfig, LogGuard, LoggingError};
