//! Integration tests for backtest system
//!
//! Tests the complete backtest pipeline including:
//! - BacktestService manual trigger (BACK-04)
//! - Profitable trade scenarios
//! - Losing trade scenarios
//! - Fee impact on trades
//! - Error handling for missing data

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use trading_bot::backtest::{
    run_backtest, BacktestConfig, BacktestError, BacktestResult, BacktestService,
};
use trading_bot::config::BacktestSettings;
use trading_bot::strategy::{Signal, Strategy, StrategyTick};

// ============================================================================
// Test Strategy Implementations
// ============================================================================

/// Simple test strategy that buys on tick 2 and sells on tick 4
/// Used for predictable profit/loss scenarios
struct BuySellTestStrategy {
    tick_count: u32,
    buy_quantity: Decimal,
}

impl BuySellTestStrategy {
    fn new(buy_quantity: Decimal) -> Self {
        Self {
            tick_count: 0,
            buy_quantity,
        }
    }
}

impl Strategy for BuySellTestStrategy {
    fn on_tick(&mut self, _tick: &StrategyTick) -> Signal {
        self.tick_count += 1;
        match self.tick_count {
            2 => Signal::Buy {
                quantity: self.buy_quantity,
            },
            4 => Signal::Sell {
                quantity: self.buy_quantity,
            },
            _ => Signal::Hold,
        }
    }

    fn update_config(&mut self, _config: &serde_json::Value) -> Result<(), String> {
        Ok(())
    }

    fn get_state(&self) -> serde_json::Value {
        serde_json::json!({
            "tick_count": self.tick_count,
            "buy_quantity": self.buy_quantity.to_string()
        })
    }

    fn name(&self) -> &'static str {
        "BuySellTestStrategy"
    }
}

/// Strategy that never trades (always holds)
struct NeverTradeStrategy;

impl Strategy for NeverTradeStrategy {
    fn on_tick(&mut self, _tick: &StrategyTick) -> Signal {
        Signal::Hold
    }

    fn update_config(&mut self, _config: &serde_json::Value) -> Result<(), String> {
        Ok(())
    }

    fn get_state(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    fn name(&self) -> &'static str {
        "NeverTradeStrategy"
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

async fn insert_test_ticks(
    pool: &sqlx::PgPool,
    symbol: &str,
    base_time: DateTime<Utc>,
    prices: &[Decimal],
) {
    for (i, price) in prices.iter().enumerate() {
        let timestamp = base_time + Duration::seconds(i as i64 * 60);
        sqlx::query(
            r#"
            INSERT INTO ticks (timestamp, symbol, price, quantity, trade_id, is_buyer_maker)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (trade_id) DO NOTHING
            "#,
        )
        .bind(timestamp)
        .bind(symbol)
        .bind(*price)
        .bind(dec!(1))
        .bind(4000000i64 + i as i64 + (base_time.timestamp() % 1000000))
        .bind(false)
        .execute(pool)
        .await
        .expect("Failed to insert test tick");
    }
}

fn test_backtest_settings() -> BacktestSettings {
    BacktestSettings {
        default_fee_pct: 0.001,       // 0.1%
        default_slippage_pct: 0.0005, // 0.05%
        default_initial_capital: 10000.0,
        risk_free_rate: 0.02, // 2%
    }
}

// ============================================================================
// Profit Scenario Tests
// ============================================================================

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_with_profit(pool: sqlx::PgPool) {
    // Set up ticks with rising prices: buy at 100, sell at 120 = 20% profit
    let base_time = DateTime::from_timestamp(1672531200, 0).unwrap();
    let prices = vec![dec!(100), dec!(100), dec!(110), dec!(120), dec!(125)];
    //                 tick 1     tick 2     tick 3     tick 4     tick 5
    //                           ^buy here            ^sell here

    insert_test_ticks(&pool, "BTCUSDT", base_time, &prices).await;

    let config = BacktestConfig {
        symbol: "BTCUSDT".to_string(),
        start_time: base_time,
        end_time: base_time + Duration::minutes(10),
        initial_capital: dec!(10000),
        fee_pct: Decimal::ZERO, // No fees for clean profit calculation
        slippage_pct: Decimal::ZERO,
    };

    let mut strategy = BuySellTestStrategy::new(dec!(1));

    let result = run_backtest(&pool, &mut strategy, &config).await.unwrap();

    // Buy 1 unit at tick 2 (price 100), sell at tick 4 (price 120)
    // Profit = 120 - 100 = 20 per unit
    assert!(result.total_pnl > Decimal::ZERO, "Should have positive P&L");
    assert_eq!(result.total_pnl, dec!(20), "Profit should be exactly 20");
    assert_eq!(result.total_trades, 1, "Should have 1 completed trade");
    assert_eq!(result.winning_trades, 1, "Should be a winning trade");
    assert_eq!(result.losing_trades, 0, "Should have no losing trades");
    assert_eq!(result.win_rate, dec!(1), "Win rate should be 100%");
}

// ============================================================================
// Loss Scenario Tests
// ============================================================================

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_with_loss(pool: sqlx::PgPool) {
    // Set up ticks with falling prices: buy at 100, sell at 80 = 20% loss
    let base_time = DateTime::from_timestamp(1672532200, 0).unwrap();
    let prices = vec![dec!(100), dec!(100), dec!(90), dec!(80), dec!(75)];
    //                 tick 1     tick 2     tick 3     tick 4     tick 5
    //                           ^buy here            ^sell here

    insert_test_ticks(&pool, "BTCUSDT", base_time, &prices).await;

    let config = BacktestConfig {
        symbol: "BTCUSDT".to_string(),
        start_time: base_time,
        end_time: base_time + Duration::minutes(10),
        initial_capital: dec!(10000),
        fee_pct: Decimal::ZERO,
        slippage_pct: Decimal::ZERO,
    };

    let mut strategy = BuySellTestStrategy::new(dec!(1));

    let result = run_backtest(&pool, &mut strategy, &config).await.unwrap();

    // Buy 1 unit at tick 2 (price 100), sell at tick 4 (price 80)
    // Loss = 80 - 100 = -20 per unit
    assert!(result.total_pnl < Decimal::ZERO, "Should have negative P&L");
    assert_eq!(result.total_pnl, dec!(-20), "Loss should be exactly -20");
    assert_eq!(result.total_trades, 1, "Should have 1 completed trade");
    assert_eq!(result.winning_trades, 0, "Should have no winning trades");
    assert_eq!(result.losing_trades, 1, "Should be a losing trade");
    assert_eq!(result.win_rate, Decimal::ZERO, "Win rate should be 0%");
}

// ============================================================================
// Fee Impact Tests
// ============================================================================

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_with_fees(pool: sqlx::PgPool) {
    // Constant price scenario - with fees, should result in loss
    let base_time = DateTime::from_timestamp(1672533200, 0).unwrap();
    let prices = vec![dec!(1000), dec!(1000), dec!(1000), dec!(1000), dec!(1000)];

    insert_test_ticks(&pool, "BTCUSDT", base_time, &prices).await;

    let config = BacktestConfig {
        symbol: "BTCUSDT".to_string(),
        start_time: base_time,
        end_time: base_time + Duration::minutes(10),
        initial_capital: dec!(10000),
        fee_pct: dec!(0.001), // 0.1% fee
        slippage_pct: Decimal::ZERO,
    };

    let mut strategy = BuySellTestStrategy::new(dec!(1));

    let result = run_backtest(&pool, &mut strategy, &config).await.unwrap();

    // Buy and sell at same price - fees should cause loss
    assert!(
        result.total_fees_paid > Decimal::ZERO,
        "Should have paid fees"
    );
    assert!(
        result.total_pnl < Decimal::ZERO,
        "Should have loss due to fees"
    );

    // Verify fees were tracked correctly
    // Buy fee: 1000 * 1 * 0.001 = 1.0
    // Sell fee: 1000 * 1 * 0.001 = 1.0
    // Total fees = 2.0
    assert_eq!(
        result.total_fees_paid,
        dec!(2),
        "Total fees should be 2 (0.1% on buy + 0.1% on sell)"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_fees_reduce_profits(pool: sqlx::PgPool) {
    // Price goes up, but fees reduce the profit
    let base_time = DateTime::from_timestamp(1672534200, 0).unwrap();
    let prices = vec![dec!(1000), dec!(1000), dec!(1050), dec!(1100), dec!(1100)];

    insert_test_ticks(&pool, "BTCUSDT", base_time, &prices).await;

    // First run without fees
    let config_no_fees = BacktestConfig {
        symbol: "BTCUSDT".to_string(),
        start_time: base_time,
        end_time: base_time + Duration::minutes(10),
        initial_capital: dec!(10000),
        fee_pct: Decimal::ZERO,
        slippage_pct: Decimal::ZERO,
    };

    let mut strategy_no_fees = BuySellTestStrategy::new(dec!(1));
    let result_no_fees = run_backtest(&pool, &mut strategy_no_fees, &config_no_fees)
        .await
        .unwrap();

    // Then run with fees
    let config_with_fees = BacktestConfig {
        symbol: "BTCUSDT".to_string(),
        start_time: base_time,
        end_time: base_time + Duration::minutes(10),
        initial_capital: dec!(10000),
        fee_pct: dec!(0.001),
        slippage_pct: Decimal::ZERO,
    };

    let mut strategy_with_fees = BuySellTestStrategy::new(dec!(1));
    let result_with_fees = run_backtest(&pool, &mut strategy_with_fees, &config_with_fees)
        .await
        .unwrap();

    // Both should be profitable but fees should reduce profit
    assert!(result_no_fees.total_pnl > Decimal::ZERO);
    assert!(result_with_fees.total_pnl > Decimal::ZERO);
    assert!(
        result_with_fees.total_pnl < result_no_fees.total_pnl,
        "Fees should reduce profits"
    );
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_no_ticks_error(pool: sqlx::PgPool) {
    // Don't insert any ticks - should return error
    let base_time = DateTime::from_timestamp(1672535200, 0).unwrap();

    let config = BacktestConfig {
        symbol: "NONEXISTENT".to_string(),
        start_time: base_time,
        end_time: base_time + Duration::hours(1),
        initial_capital: dec!(10000),
        fee_pct: Decimal::ZERO,
        slippage_pct: Decimal::ZERO,
    };

    let mut strategy = BuySellTestStrategy::new(dec!(1));

    let result = run_backtest(&pool, &mut strategy, &config).await;

    assert!(result.is_err(), "Should return error for no ticks");
    match result {
        Err(BacktestError::NoTicksFound { symbol, .. }) => {
            assert_eq!(symbol, "NONEXISTENT");
        }
        _ => panic!("Expected NoTicksFound error"),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_invalid_time_range(pool: sqlx::PgPool) {
    let base_time = DateTime::from_timestamp(1672536200, 0).unwrap();

    let config = BacktestConfig {
        symbol: "BTCUSDT".to_string(),
        start_time: base_time + Duration::hours(1), // Start after end
        end_time: base_time,
        initial_capital: dec!(10000),
        fee_pct: Decimal::ZERO,
        slippage_pct: Decimal::ZERO,
    };

    let mut strategy = BuySellTestStrategy::new(dec!(1));

    let result = run_backtest(&pool, &mut strategy, &config).await;

    assert!(result.is_err(), "Should return error for invalid time range");
    assert!(matches!(result, Err(BacktestError::InvalidTimeRange { .. })));
}

// ============================================================================
// BacktestService Tests (BACK-04 Manual Trigger)
// ============================================================================

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_service_manual_trigger(pool: sqlx::PgPool) {
    // This test verifies BACK-04: manual trigger for backtest optimization
    let base_time = DateTime::from_timestamp(1672537200, 0).unwrap();
    let prices = vec![dec!(100), dec!(100), dec!(110), dec!(120), dec!(125)];

    insert_test_ticks(&pool, "BTCUSDT", base_time, &prices).await;

    let settings = test_backtest_settings();
    let service = BacktestService::new(pool.clone(), settings);

    let mut strategy = BuySellTestStrategy::new(dec!(1));

    // Use the service's run() method - the primary manual trigger API
    let result = service
        .run(
            &mut strategy,
            "BTCUSDT",
            base_time,
            base_time + Duration::minutes(10),
            dec!(10000),
        )
        .await
        .unwrap();

    // Verify backtest executed correctly
    assert_eq!(result.symbol, "BTCUSDT");
    assert_eq!(result.strategy_name, "BuySellTestStrategy");
    assert_eq!(result.initial_capital, dec!(10000));
    assert!(
        result.total_ticks_processed > 0,
        "Should process ticks via manual trigger"
    );
    assert_eq!(result.total_trades, 1, "Should complete one trade");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_service_run_with_config(pool: sqlx::PgPool) {
    let base_time = DateTime::from_timestamp(1672538200, 0).unwrap();
    let prices = vec![dec!(100), dec!(100), dec!(110), dec!(120), dec!(125)];

    insert_test_ticks(&pool, "BTCUSDT", base_time, &prices).await;

    let settings = test_backtest_settings();
    let service = BacktestService::new(pool.clone(), settings);

    // Custom config with specific fee settings
    let config = BacktestConfig {
        symbol: "BTCUSDT".to_string(),
        start_time: base_time,
        end_time: base_time + Duration::minutes(10),
        initial_capital: dec!(5000),
        fee_pct: dec!(0.002), // Custom 0.2% fee
        slippage_pct: dec!(0.001),
    };

    let mut strategy = BuySellTestStrategy::new(dec!(1));

    let result = service
        .run_with_config(&mut strategy, &config)
        .await
        .unwrap();

    // Verify custom config was used
    assert_eq!(result.initial_capital, dec!(5000));
    assert_eq!(result.fee_pct, dec!(0.002));
    assert_eq!(result.slippage_pct, dec!(0.001));
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_service_settings_accessor(pool: sqlx::PgPool) {
    let settings = BacktestSettings {
        default_fee_pct: 0.002,
        default_slippage_pct: 0.001,
        default_initial_capital: 5000.0,
        risk_free_rate: 0.05,
    };

    let service = BacktestService::new(pool.clone(), settings);

    // Verify settings are accessible
    let retrieved = service.settings();
    assert!((retrieved.default_fee_pct - 0.002).abs() < f64::EPSILON);
    assert!((retrieved.default_slippage_pct - 0.001).abs() < f64::EPSILON);
    assert!((retrieved.default_initial_capital - 5000.0).abs() < f64::EPSILON);
    assert!((retrieved.risk_free_rate - 0.05).abs() < f64::EPSILON);
}

// ============================================================================
// Metrics Tests
// ============================================================================

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_equity_curve_generated(pool: sqlx::PgPool) {
    let base_time = DateTime::from_timestamp(1672539200, 0).unwrap();
    let prices = vec![dec!(100), dec!(100), dec!(110), dec!(120), dec!(125)];

    insert_test_ticks(&pool, "BTCUSDT", base_time, &prices).await;

    let config = BacktestConfig {
        symbol: "BTCUSDT".to_string(),
        start_time: base_time,
        end_time: base_time + Duration::minutes(10),
        initial_capital: dec!(10000),
        fee_pct: Decimal::ZERO,
        slippage_pct: Decimal::ZERO,
    };

    let mut strategy = BuySellTestStrategy::new(dec!(1));

    let result = run_backtest(&pool, &mut strategy, &config).await.unwrap();

    // Equity curve should be present
    assert!(result.equity_curve.is_some(), "Equity curve should exist");
    let equity_curve = result.equity_curve.unwrap();
    assert!(!equity_curve.is_empty(), "Equity curve should not be empty");

    // First point should be initial capital
    assert_eq!(
        equity_curve[0].equity,
        dec!(10000),
        "First equity point should be initial capital"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_trades_recorded(pool: sqlx::PgPool) {
    let base_time = DateTime::from_timestamp(1672540200, 0).unwrap();
    let prices = vec![dec!(100), dec!(100), dec!(110), dec!(120), dec!(125)];

    insert_test_ticks(&pool, "BTCUSDT", base_time, &prices).await;

    let config = BacktestConfig {
        symbol: "BTCUSDT".to_string(),
        start_time: base_time,
        end_time: base_time + Duration::minutes(10),
        initial_capital: dec!(10000),
        fee_pct: Decimal::ZERO,
        slippage_pct: Decimal::ZERO,
    };

    let mut strategy = BuySellTestStrategy::new(dec!(1));

    let result = run_backtest(&pool, &mut strategy, &config).await.unwrap();

    // Trades should be recorded
    assert!(result.trades.is_some(), "Trades should be recorded");
    let trades = result.trades.unwrap();
    assert_eq!(trades.len(), 1, "Should have one completed trade");

    let trade = &trades[0];
    assert!(trade.exit_time.is_some(), "Trade should have exit time");
    assert!(trade.exit_price.is_some(), "Trade should have exit price");
    assert!(trade.pnl.is_some(), "Trade should have P&L calculated");
    assert_eq!(
        trade.pnl.unwrap(),
        dec!(20),
        "Trade P&L should be 20 (120-100)"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backtest_no_trades_strategy(pool: sqlx::PgPool) {
    let base_time = DateTime::from_timestamp(1672541200, 0).unwrap();
    let prices = vec![dec!(100), dec!(110), dec!(120), dec!(130), dec!(140)];

    insert_test_ticks(&pool, "BTCUSDT", base_time, &prices).await;

    let config = BacktestConfig {
        symbol: "BTCUSDT".to_string(),
        start_time: base_time,
        end_time: base_time + Duration::minutes(10),
        initial_capital: dec!(10000),
        fee_pct: Decimal::ZERO,
        slippage_pct: Decimal::ZERO,
    };

    let mut strategy = NeverTradeStrategy;

    let result = run_backtest(&pool, &mut strategy, &config).await.unwrap();

    // No trades - final equity should equal initial capital
    assert_eq!(result.total_trades, 0, "Should have no trades");
    assert_eq!(result.winning_trades, 0);
    assert_eq!(result.losing_trades, 0);
    assert_eq!(result.final_equity, dec!(10000), "No change in equity");
    assert_eq!(result.total_pnl, Decimal::ZERO);
    assert_eq!(result.signals_generated, 0);
}

// ============================================================================
// Summary Tests
// ============================================================================

#[test]
fn test_backtest_result_summary_format() {
    let result = BacktestResult {
        symbol: "BTCUSDT".to_string(),
        strategy_name: "TestStrategy".to_string(),
        start_time: DateTime::from_timestamp(1672531200, 0).unwrap(),
        end_time: DateTime::from_timestamp(1672617600, 0).unwrap(),
        initial_capital: dec!(10000),
        fee_pct: dec!(0.001),
        slippage_pct: dec!(0.0005),
        final_equity: dec!(11000),
        total_pnl: dec!(1000),
        total_return_pct: dec!(0.10),
        total_trades: 10,
        winning_trades: 7,
        losing_trades: 3,
        win_rate: dec!(0.7),
        max_drawdown: dec!(0.05),
        sharpe_ratio: Some(dec!(1.5)),
        sortino_ratio: Some(dec!(2.0)),
        total_fees_paid: dec!(50),
        total_slippage_cost: dec!(25),
        total_ticks_processed: 10000,
        signals_generated: 20,
        trades: None,
        equity_curve: None,
    };

    let summary = result.summary();

    assert!(summary.contains("TestStrategy"));
    assert!(summary.contains("BTCUSDT"));
    assert!(summary.contains("10.00%")); // return
    assert!(summary.contains("70.0%")); // win rate
    assert!(summary.contains("5.00%")); // max drawdown
    assert!(summary.contains("10 trades"));
}
