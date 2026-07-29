//! Core backtest engine for historical strategy validation
//!
//! Provides the `run_backtest()` function that replays ticks through
//! a strategy, simulates execution with fees/slippage, and produces
//! comprehensive performance metrics.

use chrono::{DateTime, Utc};
use futures_util::TryStreamExt;
use rust_decimal::Decimal;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use crate::data::OrderSide;
use crate::strategy::{Signal, Strategy, StrategyTick};

use super::execution::{simulate_fill, ExecutionSimConfig};
use super::metrics::{
    calculate_max_drawdown, calculate_returns, calculate_sharpe_ratio, calculate_sortino_ratio,
    calculate_win_rate, TradeRecord,
};
use super::report::{BacktestResult, EquityPoint};
use super::repository::{count_ticks_in_range, query_ticks_in_range};
use super::types::{BacktestConfig, BacktestError};

/// Equity sampling interval (sample every N ticks)
const EQUITY_SAMPLE_INTERVAL: u64 = 1000;

/// Annual risk-free rate for Sharpe/Sortino calculations (e.g., 5%)
const RISK_FREE_RATE: &str = "0.05";

/// Trading days per year for annualization
const TRADING_DAYS_PER_YEAR: u32 = 365;

/// Run a backtest with the given strategy and configuration
///
/// Replays historical ticks through the strategy, simulating fills with
/// realistic fees and slippage. Returns comprehensive performance metrics.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `strategy` - Strategy implementation to test (mutable for state updates)
/// * `config` - Backtest configuration (symbol, time range, costs)
///
/// # Returns
/// `BacktestResult` with all performance metrics, or error if backtest fails
///
/// # Errors
/// - `InvalidTimeRange` if start >= end
/// - `NoTicksFound` if no ticks exist in the specified range
/// - `DatabaseError` for database connectivity issues
pub async fn run_backtest(
    pool: &PgPool,
    strategy: &mut dyn Strategy,
    config: &BacktestConfig,
) -> Result<BacktestResult, BacktestError> {
    // Validate time range
    if config.start_time >= config.end_time {
        return Err(BacktestError::InvalidTimeRange {
            start: config.start_time,
            end: config.end_time,
        });
    }

    // Check tick count before streaming
    let tick_count = count_ticks_in_range(pool, &config.symbol, config.start_time, config.end_time)
        .await?;

    if tick_count == 0 {
        return Err(BacktestError::NoTicksFound {
            symbol: config.symbol.clone(),
            start: config.start_time,
            end: config.end_time,
        });
    }

    info!(
        "Starting backtest: {} on {} ({} ticks, {} to {})",
        strategy.name(),
        config.symbol,
        tick_count,
        config.start_time,
        config.end_time
    );

    // Initialize tracking state
    let exec_config = ExecutionSimConfig {
        slippage_pct: config.slippage_pct,
        fee_pct: config.fee_pct,
    };

    let mut cash = config.initial_capital;
    let mut position_qty = Decimal::ZERO;
    let mut position_entry_price = Decimal::ZERO;
    let mut position_entry_time: Option<DateTime<Utc>> = None;

    let mut trades: Vec<TradeRecord> = Vec::new();
    let mut equity_curve: Vec<EquityPoint> = Vec::new();
    let mut equity_values: Vec<Decimal> = Vec::new();

    let mut total_fees_paid = Decimal::ZERO;
    let mut total_slippage_cost = Decimal::ZERO;
    let mut signals_generated: u32 = 0;
    let mut ticks_processed: u64 = 0;

    // Sample initial equity
    equity_curve.push(EquityPoint {
        timestamp: config.start_time,
        equity: config.initial_capital,
    });
    equity_values.push(config.initial_capital);

    // Stream ticks and process
    let mut tick_stream =
        query_ticks_in_range(pool, &config.symbol, config.start_time, config.end_time);

    let mut last_price = Decimal::ZERO;
    let mut last_timestamp = config.start_time;

    while let Some(tick) = tick_stream.try_next().await? {
        ticks_processed += 1;
        last_price = tick.price;
        last_timestamp = tick.timestamp;

        // Convert to StrategyTick
        let strategy_tick = StrategyTick {
            symbol: tick.symbol.clone(),
            price: tick.price,
            quantity: tick.quantity,
            timestamp: tick.timestamp.timestamp_millis(),
        };

        // Get strategy signal
        let signal = strategy.on_tick(&strategy_tick);

        // Process signal
        match signal {
            Signal::Buy { quantity } => {
                if position_qty.is_zero() {
                    // Open new position
                    let fill = simulate_fill(OrderSide::Buy, tick.price, quantity, &exec_config);

                    // Check we have enough capital
                    if fill.net_value > cash {
                        debug!(
                            "Insufficient capital for buy: need {}, have {}",
                            fill.net_value, cash
                        );
                        continue;
                    }

                    cash -= fill.net_value;
                    position_qty = quantity;
                    position_entry_price = fill.fill_price;
                    position_entry_time = Some(tick.timestamp);

                    total_fees_paid += fill.commission;
                    total_slippage_cost += fill.slippage_amount;
                    signals_generated += 1;

                    debug!(
                        "BUY {} @ {} (fill: {})",
                        quantity, tick.price, fill.fill_price
                    );
                }
            }
            Signal::Sell { quantity: _ } => {
                if !position_qty.is_zero() {
                    // Close position
                    let fill =
                        simulate_fill(OrderSide::Sell, tick.price, position_qty, &exec_config);

                    cash += fill.net_value;

                    // Calculate P&L
                    let entry_cost = position_entry_price * position_qty;
                    let exit_proceeds = fill.fill_price * position_qty;
                    let pnl = exit_proceeds - entry_cost - fill.commission;

                    // Record trade
                    trades.push(TradeRecord {
                        entry_time: position_entry_time.unwrap_or(config.start_time),
                        exit_time: Some(tick.timestamp),
                        side: OrderSide::Buy, // We bought, then sold
                        entry_price: position_entry_price,
                        exit_price: Some(fill.fill_price),
                        quantity: position_qty,
                        pnl: Some(pnl),
                        commission: fill.commission,
                    });

                    total_fees_paid += fill.commission;
                    total_slippage_cost += fill.slippage_amount;
                    signals_generated += 1;

                    debug!(
                        "SELL {} @ {} (fill: {}, pnl: {})",
                        position_qty, tick.price, fill.fill_price, pnl
                    );

                    position_qty = Decimal::ZERO;
                    position_entry_price = Decimal::ZERO;
                    position_entry_time = None;
                }
            }
            Signal::Hold => {}
        }

        // Sample equity periodically
        if ticks_processed % EQUITY_SAMPLE_INTERVAL == 0 {
            let equity = cash + (position_qty * tick.price);
            equity_curve.push(EquityPoint {
                timestamp: tick.timestamp,
                equity,
            });
            equity_values.push(equity);
        }

        // Progress logging for long backtests
        if ticks_processed % 100000 == 0 {
            info!(
                "Backtest progress: {} ticks processed, {} trades",
                ticks_processed,
                trades.len()
            );
        }
    }

    // Calculate final equity
    let final_equity = cash + (position_qty * last_price);

    // Add final equity point if not already sampled
    if equity_curve.last().map(|e| e.timestamp) != Some(last_timestamp) {
        equity_curve.push(EquityPoint {
            timestamp: last_timestamp,
            equity: final_equity,
        });
        equity_values.push(final_equity);
    }

    // Calculate metrics
    let total_pnl = final_equity - config.initial_capital;
    let total_return_pct = if config.initial_capital.is_zero() {
        Decimal::ZERO
    } else {
        total_pnl / config.initial_capital
    };

    let win_rate = calculate_win_rate(&trades);
    let max_drawdown = calculate_max_drawdown(&equity_values);

    let returns = calculate_returns(&equity_values);
    let risk_free = RISK_FREE_RATE.parse::<Decimal>().unwrap_or(Decimal::ZERO);
    let sharpe_ratio = calculate_sharpe_ratio(&returns, risk_free, TRADING_DAYS_PER_YEAR);
    let sortino_ratio = calculate_sortino_ratio(&returns, risk_free, TRADING_DAYS_PER_YEAR);

    // Count winning/losing trades
    let winning_trades = trades
        .iter()
        .filter(|t| t.pnl.map(|p| p > Decimal::ZERO).unwrap_or(false))
        .count() as u32;
    let losing_trades = trades
        .iter()
        .filter(|t| t.pnl.map(|p| p < Decimal::ZERO).unwrap_or(false))
        .count() as u32;

    // Handle open position at end (record as incomplete trade)
    if !position_qty.is_zero() {
        warn!(
            "Backtest ended with open position: {} @ {}",
            position_qty, position_entry_price
        );
        trades.push(TradeRecord {
            entry_time: position_entry_time.unwrap_or(config.start_time),
            exit_time: None,
            side: OrderSide::Buy,
            entry_price: position_entry_price,
            exit_price: None,
            quantity: position_qty,
            pnl: None,
            commission: Decimal::ZERO,
        });
    }

    let result = BacktestResult {
        symbol: config.symbol.clone(),
        strategy_name: strategy.name().to_string(),
        start_time: config.start_time,
        end_time: config.end_time,
        initial_capital: config.initial_capital,
        fee_pct: config.fee_pct,
        slippage_pct: config.slippage_pct,
        final_equity,
        total_pnl,
        total_return_pct,
        total_trades: winning_trades + losing_trades, // Only completed trades
        winning_trades,
        losing_trades,
        win_rate,
        max_drawdown,
        sharpe_ratio,
        sortino_ratio,
        total_fees_paid,
        total_slippage_cost,
        total_ticks_processed: ticks_processed,
        signals_generated,
        trades: Some(trades),
        equity_curve: Some(equity_curve),
    };

    info!("{}", result.summary());

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    /// Simple test strategy that buys on first tick, sells on second
    struct TestStrategy {
        tick_count: u32,
    }

    impl Strategy for TestStrategy {
        fn on_tick(&mut self, _tick: &StrategyTick) -> Signal {
            self.tick_count += 1;
            match self.tick_count {
                1 => Signal::Buy { quantity: dec!(1) },
                3 => Signal::Sell { quantity: dec!(1) },
                _ => Signal::Hold,
            }
        }

        fn update_config(&mut self, _config: &serde_json::Value) -> Result<(), String> {
            Ok(())
        }

        fn get_state(&self) -> serde_json::Value {
            serde_json::json!({"tick_count": self.tick_count})
        }

        fn name(&self) -> &'static str {
            "TestStrategy"
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn test_run_backtest_basic(pool: PgPool) {
        // Insert test ticks
        let base_time = DateTime::from_timestamp(1672531200, 0).unwrap();
        for i in 0..5 {
            let timestamp = base_time + chrono::Duration::seconds(i * 60);
            let price = dec!(100) + Decimal::from(i * 10);
            sqlx::query(
                r#"
                INSERT INTO ticks (timestamp, symbol, price, quantity, trade_id, is_buyer_maker)
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (trade_id) DO NOTHING
                "#,
            )
            .bind(timestamp)
            .bind("BTCUSDT")
            .bind(price)
            .bind(dec!(1))
            .bind(2000000i64 + i)
            .bind(false)
            .execute(&pool)
            .await
            .unwrap();
        }

        let config = BacktestConfig {
            symbol: "BTCUSDT".to_string(),
            start_time: base_time,
            end_time: base_time + chrono::Duration::minutes(10),
            initial_capital: dec!(10000),
            fee_pct: Decimal::ZERO, // No fees for predictable test
            slippage_pct: Decimal::ZERO,
        };

        let mut strategy = TestStrategy { tick_count: 0 };

        let result = run_backtest(&pool, &mut strategy, &config).await.unwrap();

        assert_eq!(result.symbol, "BTCUSDT");
        assert_eq!(result.strategy_name, "TestStrategy");
        assert_eq!(result.total_ticks_processed, 5);
        assert_eq!(result.signals_generated, 2); // 1 buy + 1 sell
        assert_eq!(result.total_trades, 1);
        assert!(result.trades.is_some());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn test_run_backtest_invalid_time_range(pool: PgPool) {
        let time = DateTime::from_timestamp(1672531200, 0).unwrap();
        let config = BacktestConfig {
            symbol: "BTCUSDT".to_string(),
            start_time: time + chrono::Duration::hours(1),
            end_time: time, // End before start
            initial_capital: dec!(10000),
            fee_pct: Decimal::ZERO,
            slippage_pct: Decimal::ZERO,
        };

        let mut strategy = TestStrategy { tick_count: 0 };
        let result = run_backtest(&pool, &mut strategy, &config).await;

        assert!(matches!(result, Err(BacktestError::InvalidTimeRange { .. })));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn test_run_backtest_no_ticks(pool: PgPool) {
        let time = DateTime::from_timestamp(1672531200, 0).unwrap();
        let config = BacktestConfig {
            symbol: "BTCUSDT".to_string(),
            start_time: time,
            end_time: time + chrono::Duration::hours(1),
            initial_capital: dec!(10000),
            fee_pct: Decimal::ZERO,
            slippage_pct: Decimal::ZERO,
        };

        let mut strategy = TestStrategy { tick_count: 0 };
        let result = run_backtest(&pool, &mut strategy, &config).await;

        assert!(matches!(result, Err(BacktestError::NoTicksFound { .. })));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn test_run_backtest_fees_applied(pool: PgPool) {
        // Insert test ticks
        let base_time = DateTime::from_timestamp(1672531200, 0).unwrap();
        for i in 0..5 {
            let timestamp = base_time + chrono::Duration::seconds(i * 60);
            sqlx::query(
                r#"
                INSERT INTO ticks (timestamp, symbol, price, quantity, trade_id, is_buyer_maker)
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (trade_id) DO NOTHING
                "#,
            )
            .bind(timestamp)
            .bind("BTCUSDT")
            .bind(dec!(100)) // Constant price
            .bind(dec!(1))
            .bind(3000000i64 + i)
            .bind(false)
            .execute(&pool)
            .await
            .unwrap();
        }

        let config = BacktestConfig {
            symbol: "BTCUSDT".to_string(),
            start_time: base_time,
            end_time: base_time + chrono::Duration::minutes(10),
            initial_capital: dec!(10000),
            fee_pct: dec!(0.001), // 0.1% fee
            slippage_pct: Decimal::ZERO,
        };

        let mut strategy = TestStrategy { tick_count: 0 };

        let result = run_backtest(&pool, &mut strategy, &config).await.unwrap();

        // Fees should be tracked
        assert!(result.total_fees_paid > Decimal::ZERO);
    }
}
