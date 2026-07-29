//! Integration tests for the risk control system
//!
//! These tests complete TEST-02 coverage by verifying all risk components
//! work together correctly. Unit tests for individual components are in
//! their respective #[cfg(test)] modules (plans 04-01 through 04-05).

use rust_decimal_macros::dec;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

use trading_bot::data::OrderSide;
use trading_bot::risk::config::{GlobalRiskConfig, StrategyRiskOverrides};
use trading_bot::risk::engine::{start_risk_engine, RiskEngineConfig, StopLossEvent};
use trading_bot::risk::types::RiskDecision;

fn test_config() -> RiskEngineConfig {
    RiskEngineConfig {
        initial_equity: dec!(100000),
        global_risk: GlobalRiskConfig {
            max_position_size_pct: dec!(0.10), // 10%
            stop_loss_pct: dec!(0.02),         // 2%
            daily_loss_limit_pct: dec!(0.05),  // 5%
            max_drawdown_pct: dec!(0.15),      // 15%
        },
        channel_capacity: 100,
    }
}

#[tokio::test]
async fn test_position_size_limit_approved() {
    let handle = start_risk_engine(test_config(), None, None);

    // Order within limit (10% of 100k = $10k max, order is $5k)
    let decision = handle
        .check_order("test", "BTCUSDT", OrderSide::Buy, dec!(0.1), dec!(50000))
        .await
        .unwrap();
    assert!(matches!(decision, RiskDecision::Approved));

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_position_size_limit_scaled() {
    let handle = start_risk_engine(test_config(), None, None);

    // Order exceeding limit ($50k > $10k limit)
    let decision = handle
        .check_order("test", "BTCUSDT", OrderSide::Buy, dec!(1), dec!(50000))
        .await
        .unwrap();

    match decision {
        RiskDecision::Scaled { new_quantity, .. } => {
            // Should scale to $10k / $50k = 0.2 BTC
            assert_eq!(new_quantity, dec!(0.2));
        }
        _ => panic!("Expected Scaled decision, got {:?}", decision),
    }

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_daily_loss_limit() {
    let handle = start_risk_engine(test_config(), None, None);

    // Open and close a losing position
    handle
        .position_opened("test", "order-1", "BTCUSDT", OrderSide::Buy, dec!(50000), dec!(0.2))
        .await
        .unwrap();

    // Close with $6k loss (6% of starting equity)
    handle
        .position_closed("order-1", dec!(-6000))
        .await
        .unwrap();

    // Small delay for message processing
    sleep(Duration::from_millis(50)).await;

    // New order should be rejected (daily loss > 5%)
    let decision = handle
        .check_order("test", "BTCUSDT", OrderSide::Buy, dec!(0.1), dec!(50000))
        .await
        .unwrap();

    match decision {
        RiskDecision::Rejected { reason } => {
            assert!(
                reason.contains("Daily loss limit"),
                "Expected daily loss reason, got: {}",
                reason
            );
        }
        _ => panic!("Expected Rejected decision, got {:?}", decision),
    }

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_stop_loss_trigger() {
    let (stop_loss_tx, mut stop_loss_rx) = mpsc::channel::<StopLossEvent>(10);
    let handle = start_risk_engine(test_config(), Some(stop_loss_tx), None);

    // Open a long position at $50,000 with 2% stop loss ($49,000)
    handle
        .position_opened("test", "order-1", "BTCUSDT", OrderSide::Buy, dec!(50000), dec!(1))
        .await
        .unwrap();

    // Small delay for position to be registered
    sleep(Duration::from_millis(20)).await;

    // Price drops to $48,900 (below stop)
    handle
        .price_update("BTCUSDT", dec!(48900))
        .await
        .unwrap();

    // Should receive stop-loss event
    let event = tokio::time::timeout(Duration::from_millis(500), stop_loss_rx.recv())
        .await
        .expect("Timeout waiting for stop-loss event")
        .expect("Channel closed");

    assert_eq!(event.order_id, "order-1");
    assert_eq!(event.side, OrderSide::Sell); // Closing side
    assert_eq!(event.quantity, dec!(1));

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_stop_loss_not_triggered_above_threshold() {
    let (stop_loss_tx, mut stop_loss_rx) = mpsc::channel::<StopLossEvent>(10);
    let handle = start_risk_engine(test_config(), Some(stop_loss_tx), None);

    // Open a long position at $50,000 with 2% stop loss ($49,000)
    handle
        .position_opened("test", "order-1", "BTCUSDT", OrderSide::Buy, dec!(50000), dec!(1))
        .await
        .unwrap();

    // Small delay for position to be registered
    sleep(Duration::from_millis(20)).await;

    // Price at $49,500 (above stop at $49,000) - should NOT trigger
    handle
        .price_update("BTCUSDT", dec!(49500))
        .await
        .unwrap();

    // Should NOT receive stop-loss event (use short timeout)
    let result =
        tokio::time::timeout(Duration::from_millis(100), stop_loss_rx.recv()).await;

    assert!(
        result.is_err(),
        "Should not receive stop-loss event when price is above threshold"
    );

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_circuit_breaker() {
    let config = RiskEngineConfig {
        initial_equity: dec!(100000),
        global_risk: GlobalRiskConfig {
            max_position_size_pct: dec!(0.10),
            stop_loss_pct: dec!(0.02),
            daily_loss_limit_pct: dec!(0.50), // High limit so daily loss doesn't trigger first
            max_drawdown_pct: dec!(0.15),
        },
        channel_capacity: 100,
    };

    let handle = start_risk_engine(config, None, None);

    // Simulate series of losses totaling 16% drawdown
    handle
        .position_opened("test", "order-1", "BTCUSDT", OrderSide::Buy, dec!(50000), dec!(0.5))
        .await
        .unwrap();

    handle
        .position_closed("order-1", dec!(-16000)) // 16% of 100k
        .await
        .unwrap();

    sleep(Duration::from_millis(50)).await;

    // Circuit breaker should be tripped (16% > 15% threshold)
    let decision = handle
        .check_order("test", "BTCUSDT", OrderSide::Buy, dec!(0.1), dec!(50000))
        .await
        .unwrap();

    match decision {
        RiskDecision::Rejected { reason } => {
            assert!(
                reason.contains("circuit breaker"),
                "Expected circuit breaker reason, got: {}",
                reason
            );
        }
        _ => panic!("Expected Rejected due to circuit breaker, got {:?}", decision),
    }

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_per_strategy_risk_params() {
    let handle = start_risk_engine(test_config(), None, None);

    // Register strategy with lower position size limit (5%)
    handle
        .register_strategy(
            "conservative",
            StrategyRiskOverrides {
                max_position_size_pct: Some(dec!(0.05)),
                stop_loss_pct: None,
                daily_loss_limit_pct: None,
                max_drawdown_pct: None,
            },
        )
        .await
        .unwrap();

    sleep(Duration::from_millis(20)).await;

    // $8k order should be scaled for conservative strategy (5% of 100k = $5k max)
    let decision = handle
        .check_order(
            "conservative",
            "BTCUSDT",
            OrderSide::Buy,
            dec!(0.16),
            dec!(50000),
        )
        .await
        .unwrap();

    match decision {
        RiskDecision::Scaled { new_quantity, .. } => {
            // $5k / $50k = 0.1 BTC
            assert_eq!(new_quantity, dec!(0.1));
        }
        _ => panic!("Expected Scaled decision for conservative, got {:?}", decision),
    }

    // Same order should be approved for default strategy (10% = $10k max)
    // 0.16 * 50000 = 8000, which is 8% of 100k (within 10% limit)
    let decision = handle
        .check_order("default", "BTCUSDT", OrderSide::Buy, dec!(0.16), dec!(50000))
        .await
        .unwrap();

    assert!(
        matches!(decision, RiskDecision::Approved),
        "Expected Approved for default strategy, got {:?}",
        decision
    );

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_multiple_positions_stop_loss() {
    let (stop_loss_tx, mut stop_loss_rx) = mpsc::channel::<StopLossEvent>(10);
    let handle = start_risk_engine(test_config(), Some(stop_loss_tx), None);

    // Open two positions with different entry prices
    handle
        .position_opened("test", "order-1", "BTCUSDT", OrderSide::Buy, dec!(50000), dec!(1))
        .await
        .unwrap();

    handle
        .position_opened("test", "order-2", "BTCUSDT", OrderSide::Buy, dec!(52000), dec!(1))
        .await
        .unwrap();

    sleep(Duration::from_millis(20)).await;

    // Stop prices:
    // order-1: $50000 * 0.98 = $49000
    // order-2: $52000 * 0.98 = $50960

    // Price drops to $48,500 - triggers both (below both stops)
    handle
        .price_update("BTCUSDT", dec!(48500))
        .await
        .unwrap();

    // Should receive stop-loss events for both
    let event1 = tokio::time::timeout(Duration::from_millis(500), stop_loss_rx.recv())
        .await
        .expect("Timeout waiting for first stop-loss event")
        .expect("Channel closed");

    let event2 = tokio::time::timeout(Duration::from_millis(500), stop_loss_rx.recv())
        .await
        .expect("Timeout waiting for second stop-loss event")
        .expect("Channel closed");

    // Both orders should have triggered (order doesn't matter)
    let triggered_ids: Vec<&str> = vec![&event1.order_id, &event2.order_id];
    assert!(triggered_ids.contains(&"order-1"));
    assert!(triggered_ids.contains(&"order-2"));

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_partial_stop_loss_trigger() {
    let (stop_loss_tx, mut stop_loss_rx) = mpsc::channel::<StopLossEvent>(10);
    let handle = start_risk_engine(test_config(), Some(stop_loss_tx), None);

    // Open two positions with different entry prices
    handle
        .position_opened("test", "order-1", "BTCUSDT", OrderSide::Buy, dec!(50000), dec!(1))
        .await
        .unwrap();

    handle
        .position_opened("test", "order-2", "BTCUSDT", OrderSide::Buy, dec!(52000), dec!(1))
        .await
        .unwrap();

    sleep(Duration::from_millis(20)).await;

    // Stop prices:
    // order-1: $50000 * 0.98 = $49000
    // order-2: $52000 * 0.98 = $50960

    // Price at $50500 - triggers order-2 (below $50960) but NOT order-1 (above $49000)
    handle
        .price_update("BTCUSDT", dec!(50500))
        .await
        .unwrap();

    // Should receive stop-loss for order-2 only
    let event = tokio::time::timeout(Duration::from_millis(500), stop_loss_rx.recv())
        .await
        .expect("Timeout waiting for stop-loss event")
        .expect("Channel closed");

    assert_eq!(event.order_id, "order-2");

    // Should NOT receive another event (short timeout)
    let result =
        tokio::time::timeout(Duration::from_millis(100), stop_loss_rx.recv()).await;

    assert!(
        result.is_err(),
        "Should not receive stop-loss event for order-1"
    );

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_short_position_stop_loss() {
    let (stop_loss_tx, mut stop_loss_rx) = mpsc::channel::<StopLossEvent>(10);
    let handle = start_risk_engine(test_config(), Some(stop_loss_tx), None);

    // Open a short position at $50,000 with 2% stop loss ($51,000)
    handle
        .position_opened("test", "order-1", "BTCUSDT", OrderSide::Sell, dec!(50000), dec!(1))
        .await
        .unwrap();

    sleep(Duration::from_millis(20)).await;

    // Price rises to $51,500 (above stop at $51,000)
    handle
        .price_update("BTCUSDT", dec!(51500))
        .await
        .unwrap();

    // Should receive stop-loss event
    let event = tokio::time::timeout(Duration::from_millis(500), stop_loss_rx.recv())
        .await
        .expect("Timeout waiting for stop-loss event")
        .expect("Channel closed");

    assert_eq!(event.order_id, "order-1");
    assert_eq!(event.side, OrderSide::Buy); // Closing side for short is Buy
    assert_eq!(event.quantity, dec!(1));

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_position_closed_removes_from_stop_loss_tracking() {
    let (stop_loss_tx, mut stop_loss_rx) = mpsc::channel::<StopLossEvent>(10);
    let handle = start_risk_engine(test_config(), Some(stop_loss_tx), None);

    // Open a long position
    handle
        .position_opened("test", "order-1", "BTCUSDT", OrderSide::Buy, dec!(50000), dec!(1))
        .await
        .unwrap();

    sleep(Duration::from_millis(20)).await;

    // Close the position (manually, not via stop-loss)
    handle.position_closed("order-1", dec!(1000)).await.unwrap();

    sleep(Duration::from_millis(20)).await;

    // Price drops below where stop would have been
    handle.price_update("BTCUSDT", dec!(48000)).await.unwrap();

    // Should NOT receive stop-loss event (position was already closed)
    let result =
        tokio::time::timeout(Duration::from_millis(100), stop_loss_rx.recv()).await;

    assert!(
        result.is_err(),
        "Should not receive stop-loss event for closed position"
    );

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_equity_update_affects_position_sizing() {
    let handle = start_risk_engine(test_config(), None, None);

    // Update equity to $200k
    handle.update_equity(dec!(200000)).await.unwrap();

    sleep(Duration::from_millis(20)).await;

    // Now 10% is $20k, so a $15k order should be approved
    // $15k = 0.3 BTC at $50k
    let decision = handle
        .check_order("test", "BTCUSDT", OrderSide::Buy, dec!(0.3), dec!(50000))
        .await
        .unwrap();

    assert!(
        matches!(decision, RiskDecision::Approved),
        "Expected Approved with higher equity, got {:?}",
        decision
    );

    // But $25k order should be scaled (exceeds $20k limit)
    // $25k = 0.5 BTC at $50k
    let decision = handle
        .check_order("test", "BTCUSDT", OrderSide::Buy, dec!(0.5), dec!(50000))
        .await
        .unwrap();

    match decision {
        RiskDecision::Scaled { new_quantity, .. } => {
            // max_allowed = 200000 * 0.10 = 20000
            // scaled_quantity = 20000 / 50000 = 0.4
            assert_eq!(new_quantity, dec!(0.4));
        }
        _ => panic!("Expected Scaled decision, got {:?}", decision),
    }

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_order_approved_within_daily_loss_limit() {
    let handle = start_risk_engine(test_config(), None, None);

    // Record a 4% loss (within 5% limit)
    handle
        .position_opened("test", "order-1", "BTCUSDT", OrderSide::Buy, dec!(50000), dec!(0.1))
        .await
        .unwrap();

    handle
        .position_closed("order-1", dec!(-4000))
        .await
        .unwrap();

    sleep(Duration::from_millis(50)).await;

    // Order should still be approved (4% < 5% limit)
    let decision = handle
        .check_order("test", "BTCUSDT", OrderSide::Buy, dec!(0.1), dec!(50000))
        .await
        .unwrap();

    assert!(
        matches!(decision, RiskDecision::Approved),
        "Expected Approved within daily loss limit, got {:?}",
        decision
    );

    handle.shutdown().await.unwrap();
}
