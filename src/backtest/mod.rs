//! Backtesting module for historical strategy validation
//!
//! Provides tick replay through strategies with realistic fee and slippage
//! simulation, producing comprehensive performance metrics.
//!
//! ## Key Components
//!
//! - [`BacktestService`] - High-level API for running backtests (BACK-04 manual trigger)
//! - [`run_backtest`] - Core backtest engine function
//! - [`BacktestConfig`] - Configuration for individual backtests
//! - [`BacktestResult`] - Comprehensive result metrics

pub mod engine;
pub mod execution;
pub mod metrics;
pub mod optimization;
pub mod report;
pub mod repository;
pub mod types;

pub use engine::*;
pub use execution::*;
pub use metrics::*;
pub use optimization::*;
pub use report::*;
pub use repository::*;
pub use types::*;

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::config::BacktestSettings;
use crate::strategy::Strategy;

/// High-level backtest service for orchestrating backtests
///
/// This is the primary API for triggering backtests programmatically.
/// It implements BACK-04 (manual trigger for optimization) by providing
/// methods that:
/// - Phase 8 CLI will call via `trading-bot backtest run` command
/// - Phase 9 Dashboard will expose via HTTP API endpoint
///
/// # Example
///
/// ```ignore
/// let service = BacktestService::new(pool, settings);
/// let result = service.run(
///     &mut strategy,
///     "BTCUSDT",
///     start_time,
///     end_time,
///     Decimal::from(10000),
/// ).await?;
/// ```
pub struct BacktestService {
    pool: PgPool,
    settings: BacktestSettings,
}

impl BacktestService {
    /// Create a new BacktestService with the given database pool and settings
    ///
    /// # Arguments
    /// * `pool` - Database connection pool for tick data access
    /// * `settings` - Default backtest settings from configuration
    pub fn new(pool: PgPool, settings: BacktestSettings) -> Self {
        Self { pool, settings }
    }

    /// Run a backtest with explicit parameters (BACK-04 manual trigger)
    ///
    /// This is the primary method for triggering backtests. It allows full
    /// control over all parameters.
    ///
    /// # Arguments
    /// * `strategy` - Mutable reference to the strategy to test
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `start_time` - Backtest start time (inclusive)
    /// * `end_time` - Backtest end time (inclusive)
    /// * `initial_capital` - Starting capital in quote currency
    ///
    /// # Returns
    /// `BacktestResult` with comprehensive performance metrics
    ///
    /// # Errors
    /// - `BacktestError::InvalidTimeRange` if start >= end
    /// - `BacktestError::NoTicksFound` if no data exists for the range
    /// - `BacktestError::DatabaseError` for connectivity issues
    pub async fn run(
        &self,
        strategy: &mut dyn Strategy,
        symbol: &str,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        initial_capital: Decimal,
    ) -> Result<BacktestResult, BacktestError> {
        let config = BacktestConfig {
            symbol: symbol.to_string(),
            start_time,
            end_time,
            initial_capital,
            fee_pct: Decimal::try_from(self.settings.default_fee_pct)
                .unwrap_or(Decimal::ZERO),
            slippage_pct: Decimal::try_from(self.settings.default_slippage_pct)
                .unwrap_or(Decimal::ZERO),
        };

        run_backtest(&self.pool, strategy, &config).await
    }

    /// Run a backtest for the last N days (convenience method)
    ///
    /// Calculates the time range automatically based on the current time
    /// and the specified number of days.
    ///
    /// # Arguments
    /// * `strategy` - Mutable reference to the strategy to test
    /// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
    /// * `days` - Number of days to backtest (e.g., 30 for last month)
    /// * `initial_capital` - Starting capital in quote currency
    ///
    /// # Returns
    /// `BacktestResult` with comprehensive performance metrics
    pub async fn run_recent(
        &self,
        strategy: &mut dyn Strategy,
        symbol: &str,
        days: u32,
        initial_capital: Decimal,
    ) -> Result<BacktestResult, BacktestError> {
        let end_time = Utc::now();
        let start_time = end_time - Duration::days(i64::from(days));

        self.run(strategy, symbol, start_time, end_time, initial_capital)
            .await
    }

    /// Run a backtest with custom fee and slippage settings
    ///
    /// Use this when you need to override the default cost parameters.
    ///
    /// # Arguments
    /// * `strategy` - Mutable reference to the strategy to test
    /// * `config` - Full backtest configuration
    ///
    /// # Returns
    /// `BacktestResult` with comprehensive performance metrics
    pub async fn run_with_config(
        &self,
        strategy: &mut dyn Strategy,
        config: &BacktestConfig,
    ) -> Result<BacktestResult, BacktestError> {
        run_backtest(&self.pool, strategy, config).await
    }

    /// Get the default settings for this service
    pub fn settings(&self) -> &BacktestSettings {
        &self.settings
    }
}
