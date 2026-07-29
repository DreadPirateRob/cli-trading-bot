//! Core backtest configuration and error types
//!
//! Defines the configuration for running backtests and error types
//! for backtest-specific failures.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::report::BacktestResult;

/// Configuration for running a backtest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestConfig {
    /// Trading pair symbol (e.g., "BTCUSDT")
    pub symbol: String,

    /// Backtest start time (inclusive)
    pub start_time: DateTime<Utc>,

    /// Backtest end time (inclusive)
    pub end_time: DateTime<Utc>,

    /// Starting balance in quote currency (e.g., USDT)
    pub initial_capital: Decimal,

    /// Trading fee percentage (e.g., 0.001 for 0.1%)
    pub fee_pct: Decimal,

    /// Slippage percentage (e.g., 0.0005 for 0.05%)
    pub slippage_pct: Decimal,
}

/// Errors that can occur during backtest execution
#[derive(Debug, thiserror::Error)]
pub enum BacktestError {
    /// Database query failure
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    /// No ticks found in the specified time range
    #[error("No ticks found for {symbol} between {start} and {end}")]
    NoTicksFound {
        symbol: String,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },

    /// Invalid time range (start >= end)
    #[error("Invalid time range: start ({start}) must be before end ({end})")]
    InvalidTimeRange {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },

    /// Strategy execution failure
    #[error("Strategy error: {0}")]
    StrategyError(String),

    /// Insufficient capital to execute trade
    #[error("Insufficient capital: required {required}, available {available}")]
    InsufficientCapital {
        required: Decimal,
        available: Decimal,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_backtest_config_serialize() {
        let config = BacktestConfig {
            symbol: "BTCUSDT".to_string(),
            start_time: DateTime::from_timestamp(1672531200, 0).unwrap(),
            end_time: DateTime::from_timestamp(1672617600, 0).unwrap(),
            initial_capital: dec!(10000),
            fee_pct: dec!(0.001),
            slippage_pct: dec!(0.0005),
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("BTCUSDT"));
        assert!(json.contains("10000"));
    }

    #[test]
    fn test_backtest_config_deserialize() {
        let json = r#"{
            "symbol": "BTCUSDT",
            "start_time": "2023-01-01T00:00:00Z",
            "end_time": "2023-01-02T00:00:00Z",
            "initial_capital": "10000",
            "fee_pct": "0.001",
            "slippage_pct": "0.0005"
        }"#;

        let config: BacktestConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.symbol, "BTCUSDT");
        assert_eq!(config.initial_capital, dec!(10000));
        assert_eq!(config.fee_pct, dec!(0.001));
    }

    #[test]
    fn test_backtest_error_display() {
        let error = BacktestError::NoTicksFound {
            symbol: "BTCUSDT".to_string(),
            start: DateTime::from_timestamp(1672531200, 0).unwrap(),
            end: DateTime::from_timestamp(1672617600, 0).unwrap(),
        };

        let msg = error.to_string();
        assert!(msg.contains("No ticks found"));
        assert!(msg.contains("BTCUSDT"));
    }

    #[test]
    fn test_invalid_time_range_error() {
        let start = DateTime::from_timestamp(1672617600, 0).unwrap();
        let end = DateTime::from_timestamp(1672531200, 0).unwrap();

        let error = BacktestError::InvalidTimeRange { start, end };
        let msg = error.to_string();
        assert!(msg.contains("Invalid time range"));
        assert!(msg.contains("must be before"));
    }

    #[test]
    fn test_insufficient_capital_error() {
        let error = BacktestError::InsufficientCapital {
            required: dec!(5000),
            available: dec!(1000),
        };

        let msg = error.to_string();
        assert!(msg.contains("Insufficient capital"));
        assert!(msg.contains("5000"));
        assert!(msg.contains("1000"));
    }
}

// ============================================================================
// Optimization Types
// ============================================================================

/// Configuration for parameter optimization
#[derive(Debug, Clone)]
pub struct OptimizationConfig {
    /// Base backtest config (symbol, capital, fees, slippage)
    pub base_config: BacktestConfig,
    /// RSI period range (min, max)
    pub period_range: (usize, usize),
    /// StdDev multiplier range (min, max)
    pub multiplier_range: (Decimal, Decimal),
    /// Steps per parameter (default 5)
    pub steps: usize,
    /// Train/validate split ratio (default 0.70 for 70% train)
    pub train_ratio: f64,
    /// Minimum ticks required (default 1000)
    pub min_ticks: i64,
}

/// Single optimization result entry
#[derive(Debug, Clone)]
pub struct OptimizationEntry {
    /// RSI period used
    pub rsi_period: usize,
    /// StdDev multiplier used
    pub stddev_multiplier: Decimal,
    /// Training backtest result
    pub train_result: BacktestResult,
    /// Validation backtest result
    pub validate_result: BacktestResult,
}

/// Complete optimization result
#[derive(Debug, Clone)]
pub struct OptimizationResult {
    /// All tested combinations, sorted by train_result.total_return_pct descending
    pub results: Vec<OptimizationEntry>,
    /// Total combinations tested
    pub combinations_tested: usize,
    /// Best result (highest training return)
    pub best: Option<OptimizationEntry>,
    /// Whether best result passed validation (validate return > 0)
    pub validation_passed: bool,
}

/// Optimization-specific errors
#[derive(Debug, thiserror::Error)]
pub enum OptimizationError {
    #[error("Backtest error: {0}")]
    BacktestError(#[from] BacktestError),

    #[error("Insufficient data: found {found} ticks, need {required} minimum")]
    InsufficientData { found: i64, required: i64 },

    #[error("Invalid parameter range: {param} min ({min}) >= max ({max})")]
    InvalidRange { param: String, min: String, max: String },

    #[error("Invalid train ratio: {0} (must be between 0.1 and 0.9)")]
    InvalidTrainRatio(f64),
}
