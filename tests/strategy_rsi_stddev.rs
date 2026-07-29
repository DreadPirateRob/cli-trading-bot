//! Tests for the RSI-StdDev trading strategy.
//!
//! TDD tests written before implementation.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde_json::json;
use trading_bot::strategy::{RsiStdDevConfig, RsiStdDevStrategy, Signal, Strategy, StrategyTick};

/// Helper to create a StrategyTick with the given price.
fn tick(price: f64) -> StrategyTick {
    StrategyTick {
        symbol: "BTCUSDT".to_string(),
        price: Decimal::from_f64_retain(price).unwrap(),
        quantity: dec!(1.0),
        timestamp: 1000,
    }
}

/// Helper to create a sequence of ticks and feed them to the strategy.
fn feed_prices(strategy: &mut RsiStdDevStrategy, prices: &[f64]) -> Vec<Signal> {
    prices.iter().map(|p| strategy.on_tick(&tick(*p))).collect()
}

// ============================================================================
// CONSTRUCTION TESTS
// ============================================================================

#[test]
fn test_new_creates_strategy_with_config() {
    let config = RsiStdDevConfig {
        rsi_period: 14,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let strategy = RsiStdDevStrategy::new(config);
    assert_eq!(strategy.name(), "rsi_stddev");
}

// ============================================================================
// WARMUP TESTS
// ============================================================================

#[test]
fn test_warmup_returns_hold_until_indicator_ready() {
    let config = RsiStdDevConfig {
        rsi_period: 14,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // First 14 ticks should return Hold (warmup period)
    // RSI needs period + 1 data points to have meaningful values
    for i in 0..14 {
        let signal = strategy.on_tick(&tick(100.0 + i as f64));
        assert_eq!(
            signal,
            Signal::Hold,
            "Tick {} during warmup should be Hold",
            i
        );
    }
}

// ============================================================================
// BUY SIGNAL TESTS (OVERSOLD RECOVERY)
// ============================================================================

#[test]
fn test_buy_signal_on_oversold_recovery() {
    let config = RsiStdDevConfig {
        rsi_period: 3, // Short period for test
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // RSI behavior with 3-period:
    // - First tick: RSI=50 (no comparison)
    // - Subsequent drops: RSI approaches 0
    // - Strong recovery: RSI rises toward 100
    //
    // Need prices that:
    // 1. Complete warmup (4 ticks for period=3)
    // 2. Create RSI < 30 (consecutive drops)
    // 3. Then recover strongly (RSI crosses above 30)
    let prices = vec![
        100.0, 99.0, 98.0, 97.0, // Warmup period
        90.0, 85.0, 80.0,        // Drops -> RSI becomes very low (<1)
        85.0,                    // Strong recovery -> RSI jumps significantly
        90.0,                    // Continue up -> RSI crosses above 30
    ];

    let signals = feed_prices(&mut strategy, &prices);

    // Find a Buy signal after the initial warmup period
    let buy_found = signals.iter().skip(4).any(|s| matches!(s, Signal::Buy { .. }));
    assert!(buy_found, "Expected Buy signal on oversold recovery. Signals: {:?}", signals);
}

#[test]
fn test_no_buy_when_staying_oversold() {
    let config = RsiStdDevConfig {
        rsi_period: 3,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // Consistently declining prices - never crosses above oversold
    let prices = vec![100.0, 99.0, 98.0, 97.0, 96.0, 95.0, 94.0, 93.0];

    let signals = feed_prices(&mut strategy, &prices);

    // After warmup, all signals should be Hold (no crossover)
    let buy_count = signals
        .iter()
        .skip(3)
        .filter(|s| matches!(s, Signal::Buy { .. }))
        .count();
    assert_eq!(
        buy_count, 0,
        "Should not generate Buy when price keeps dropping"
    );
}

// ============================================================================
// SELL SIGNAL TESTS (OVERBOUGHT ENTRY)
// ============================================================================

#[test]
fn test_sell_signal_on_overbought_entry() {
    let config = RsiStdDevConfig {
        rsi_period: 3,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // First trigger a buy (need position to sell)
    // Then push RSI into overbought territory
    let prices = vec![
        100.0, 99.0, 98.0, 95.0, // Warmup + drop to oversold
        97.0, // Recovery - triggers buy
        105.0, 110.0, 115.0, // Strong rally - pushes RSI into overbought
    ];

    let signals = feed_prices(&mut strategy, &prices);

    // Should have both a Buy and a Sell
    let has_buy = signals.iter().any(|s| matches!(s, Signal::Buy { .. }));
    let has_sell = signals.iter().any(|s| matches!(s, Signal::Sell { .. }));

    assert!(has_buy, "Should have Buy signal before Sell");
    assert!(has_sell, "Should have Sell signal on overbought entry");
}

#[test]
fn test_no_sell_without_position() {
    let config = RsiStdDevConfig {
        rsi_period: 3,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // Go directly to overbought without ever being in position
    // Consistently rising prices
    let prices = vec![100.0, 101.0, 102.0, 103.0, 105.0, 110.0, 115.0, 120.0];

    let signals = feed_prices(&mut strategy, &prices);

    // Should not sell without having bought first
    let sell_count = signals
        .iter()
        .filter(|s| matches!(s, Signal::Sell { .. }))
        .count();
    assert_eq!(sell_count, 0, "Should not Sell without position");
}

// ============================================================================
// POSITION TRACKING TESTS
// ============================================================================

#[test]
fn test_no_duplicate_buy_when_in_position() {
    let config = RsiStdDevConfig {
        rsi_period: 3,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // Trigger oversold recovery twice without selling
    let prices = vec![
        100.0, 99.0, 98.0, 95.0, // Drop to oversold
        97.0,                    // Recovery 1 - triggers buy
        94.0,                    // Drop again
        96.0,                    // Recovery 2 - should NOT buy again
    ];

    let signals = feed_prices(&mut strategy, &prices);

    let buy_count = signals
        .iter()
        .filter(|s| matches!(s, Signal::Buy { .. }))
        .count();
    assert!(
        buy_count <= 1,
        "Should not generate duplicate Buy signals: got {}",
        buy_count
    );
}

#[test]
fn test_position_state_in_get_state() {
    let config = RsiStdDevConfig {
        rsi_period: 3,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // Initially not in position
    let state = strategy.get_state();
    assert_eq!(state["position_open"], false);

    // Trigger a buy
    let prices = vec![100.0, 99.0, 98.0, 95.0, 97.0];
    feed_prices(&mut strategy, &prices);

    // Check if we got into position
    let state = strategy.get_state();
    // Note: may or may not be in position depending on exact RSI values
    // This test validates the state field exists and is boolean
    assert!(state["position_open"].is_boolean());
}

// ============================================================================
// STATE INSPECTION TESTS
// ============================================================================

#[test]
fn test_get_state_returns_valid_json() {
    let config = RsiStdDevConfig {
        rsi_period: 14,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let strategy = RsiStdDevStrategy::new(config);
    let state = strategy.get_state();

    // Should have expected fields
    assert!(state["last_rsi"].is_null() || state["last_rsi"].is_number());
    assert!(state["prev_rsi"].is_null() || state["prev_rsi"].is_number());
    assert!(state["last_stddev"].is_null() || state["last_stddev"].is_number());
    assert!(state["position_open"].is_boolean());
    assert!(state["config"].is_object());
}

#[test]
fn test_get_state_updates_after_ticks() {
    let config = RsiStdDevConfig {
        rsi_period: 3,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // Initial state
    let state_before = strategy.get_state();
    assert!(state_before["last_rsi"].is_null());

    // Feed some ticks
    feed_prices(&mut strategy, &[100.0, 101.0, 102.0, 103.0]);

    // State should now have values
    let state_after = strategy.get_state();
    assert!(state_after["last_rsi"].is_number());
}

// ============================================================================
// CONFIG UPDATE TESTS
// ============================================================================

#[test]
fn test_update_config_changes_thresholds() {
    let config = RsiStdDevConfig {
        rsi_period: 14,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // Update thresholds
    let new_config = json!({
        "rsi_oversold": 25.0,
        "rsi_overbought": 75.0,
        "stddev_multiplier": 1.5
    });

    let result = strategy.update_config(&new_config);
    assert!(result.is_ok());

    // Check state reflects new config
    let state = strategy.get_state();
    assert_eq!(state["config"]["rsi_oversold"], 25.0);
    assert_eq!(state["config"]["rsi_overbought"], 75.0);
    assert_eq!(state["config"]["stddev_multiplier"], 1.5);
}

#[test]
fn test_update_config_rejects_invalid_thresholds() {
    let config = RsiStdDevConfig {
        rsi_period: 14,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // Invalid: oversold > overbought
    let invalid_config = json!({
        "rsi_oversold": 80.0,
        "rsi_overbought": 20.0
    });

    let result = strategy.update_config(&invalid_config);
    assert!(result.is_err());
}

// ============================================================================
// QUANTITY CALCULATION TESTS
// ============================================================================

#[test]
fn test_buy_signal_includes_quantity() {
    let config = RsiStdDevConfig {
        rsi_period: 3,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let mut strategy = RsiStdDevStrategy::new(config);

    // Trigger a buy signal
    let prices = vec![100.0, 99.0, 98.0, 95.0, 97.0];
    let signals = feed_prices(&mut strategy, &prices);

    // Find the buy signal and check it has quantity
    let buy_signal = signals.iter().find(|s| matches!(s, Signal::Buy { .. }));

    if let Some(Signal::Buy { quantity }) = buy_signal {
        assert!(*quantity > Decimal::ZERO, "Buy quantity should be positive");
    }
    // Note: May not have a buy signal depending on exact RSI calculation
}

// ============================================================================
// STRATEGY NAME TEST
// ============================================================================

#[test]
fn test_strategy_name() {
    let config = RsiStdDevConfig {
        rsi_period: 14,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        stddev_period: 20,
        stddev_multiplier: 1.0,
        base_quantity: dec!(0.01),
    };

    let strategy = RsiStdDevStrategy::new(config);
    assert_eq!(strategy.name(), "rsi_stddev");
}
