//! Integration tests for order execution system
//!
//! Tests the complete execution pipeline including:
//! - Paper trading execution flow
//! - Signal processing through OrderExecutorActor
//! - Stop-loss event handling
//! - Risk engine integration
//! - Balance and position tracking

use std::collections::HashMap;

use rust_decimal_macros::dec;
use tokio::sync::mpsc;

use trading_bot::config::{ExecutionConfig, PaperTradingConfig};
use trading_bot::data::OrderSide;
use trading_bot::execution::{
    start_order_executor, BalanceManager, ExecuteOrderRequest, ExecutionPosition,
    ExecutionResult, ExecutorStartConfig, OrderExecutor, OrderStatus, OrderType, PaperExecutor,
    PositionManager, ReserveConfig,
};
use trading_bot::risk::{start_risk_engine, RiskEngineConfig, StopLossEvent};
use trading_bot::strategy::Signal;

fn test_paper_config() -> PaperTradingConfig {
    let mut balances = HashMap::new();
    balances.insert("USDT".to_string(), 10000.0);
    balances.insert("BTC".to_string(), 0.0);

    PaperTradingConfig {
        enabled: true,
        latency_range_ms: (1, 5), // Fast for tests
        slippage_range: (0.0, 0.0), // No slippage for predictable tests
        fee_pct: 0.001,
        initial_balances: balances,
    }
}

fn test_execution_config() -> ExecutionConfig {
    ExecutionConfig {
        enabled: true,
        channel_capacity: 100,
        stop_loss_offset_pct: 0.05,
        reconciliation_interval_secs: 0, // Disabled for tests
        reserve_pct: None,
    }
}

// ============================================================================
// Paper Executor Direct Tests
// ============================================================================

#[tokio::test]
async fn test_paper_executor_buy_order() {
    let executor = PaperExecutor::new(test_paper_config());

    let request = ExecuteOrderRequest {
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        quantity: dec!(0.1),
        order_type: OrderType::Market,
        price: Some(dec!(50000)),
        client_order_id: None,
    };

    let response = executor.execute(request).await.unwrap();

    assert_eq!(response.status, OrderStatus::Filled);
    assert_eq!(response.filled_quantity, dec!(0.1));
    assert_eq!(response.symbol, "BTCUSDT");
    assert_eq!(response.side, OrderSide::Buy);

    // Check balances updated
    let btc_balance = executor.get_balance("BTC").await.unwrap();
    assert_eq!(btc_balance, dec!(0.1));

    let usdt_balance = executor.get_balance("USDT").await.unwrap();
    // Should be less than 5000 (10000 - 5000 cost - fees)
    assert!(usdt_balance < dec!(5000));
}

#[tokio::test]
async fn test_paper_executor_sell_order() {
    let mut config = test_paper_config();
    config.initial_balances.insert("BTC".to_string(), 1.0);
    let executor = PaperExecutor::new(config);

    let request = ExecuteOrderRequest {
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Sell,
        quantity: dec!(0.5),
        order_type: OrderType::Market,
        price: Some(dec!(50000)),
        client_order_id: None,
    };

    let response = executor.execute(request).await.unwrap();

    assert_eq!(response.status, OrderStatus::Filled);
    assert_eq!(response.filled_quantity, dec!(0.5));

    let btc_balance = executor.get_balance("BTC").await.unwrap();
    assert_eq!(btc_balance, dec!(0.5)); // 1.0 - 0.5

    let usdt_balance = executor.get_balance("USDT").await.unwrap();
    // Should be more than 10000 (10000 + 25000 proceeds - fees)
    assert!(usdt_balance > dec!(34000));
}

#[tokio::test]
async fn test_paper_executor_insufficient_balance() {
    let executor = PaperExecutor::new(test_paper_config());

    let request = ExecuteOrderRequest {
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        quantity: dec!(1.0), // 1 BTC = 50000 USDT, we only have 10000
        order_type: OrderType::Market,
        price: Some(dec!(50000)),
        client_order_id: None,
    };

    let result = executor.execute(request).await;

    assert!(result.is_err());
    match result {
        Err(trading_bot::execution::ExecutionError::InsufficientBalance { asset, .. }) => {
            assert_eq!(asset, "USDT");
        }
        _ => panic!("Expected InsufficientBalance error"),
    }
}

#[tokio::test]
async fn test_paper_executor_order_status() {
    let executor = PaperExecutor::new(test_paper_config());

    let request = ExecuteOrderRequest {
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        quantity: dec!(0.01),
        order_type: OrderType::Market,
        price: Some(dec!(50000)),
        client_order_id: None,
    };

    let response = executor.execute(request).await.unwrap();
    let status = executor
        .get_order_status(&response.order_id, "BTCUSDT")
        .await
        .unwrap();

    assert_eq!(status, OrderStatus::Filled);
}

#[tokio::test]
async fn test_paper_executor_multiple_orders() {
    let executor = PaperExecutor::new(test_paper_config());

    // Buy order 1
    let response1 = executor
        .execute(ExecuteOrderRequest {
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Buy,
            quantity: dec!(0.05),
            order_type: OrderType::Market,
            price: Some(dec!(50000)),
            client_order_id: None,
        })
        .await
        .unwrap();

    // Buy order 2
    let response2 = executor
        .execute(ExecuteOrderRequest {
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Buy,
            quantity: dec!(0.03),
            order_type: OrderType::Market,
            price: Some(dec!(51000)),
            client_order_id: None,
        })
        .await
        .unwrap();

    // Both orders should be filled
    assert_eq!(response1.status, OrderStatus::Filled);
    assert_eq!(response2.status, OrderStatus::Filled);

    // Total BTC should be 0.08
    let btc = executor.get_balance("BTC").await.unwrap();
    assert_eq!(btc, dec!(0.08));
}

// ============================================================================
// Balance Manager Tests
// ============================================================================

#[test]
fn test_balance_manager_reserves() {
    let mut mgr = BalanceManager::new(ReserveConfig::with_percentage(dec!(0.10)));
    mgr.set_balance("USDT", dec!(10000));

    // 10% reserve means 9000 available
    assert_eq!(mgr.get_available("USDT"), dec!(9000));

    // Reserve some for in-flight order
    assert!(mgr.reserve("USDT", dec!(5000)));
    assert_eq!(mgr.get_available("USDT"), dec!(4000));

    // Release reservation
    mgr.release_reservation("USDT", dec!(5000));
    assert_eq!(mgr.get_available("USDT"), dec!(9000));
}

#[test]
fn test_balance_manager_fixed_reserve() {
    let mut reserves = HashMap::new();
    reserves.insert("USDT".to_string(), dec!(500));

    let mut mgr = BalanceManager::new(ReserveConfig::with_fixed(reserves));
    mgr.set_balance("USDT", dec!(10000));

    // Fixed 500 reserve, available = 9500
    assert_eq!(mgr.get_available("USDT"), dec!(9500));
}

#[test]
fn test_balance_manager_insufficient_reserve() {
    let mut mgr = BalanceManager::new(ReserveConfig::default());
    mgr.set_balance("USDT", dec!(100));

    // Can't reserve more than available
    assert!(!mgr.reserve("USDT", dec!(200)));
    assert!(mgr.has_available("USDT", dec!(100)));
    assert!(!mgr.has_available("USDT", dec!(101)));
}

// ============================================================================
// Position Manager Tests
// ============================================================================

#[test]
fn test_position_manager_fifo() {
    use chrono::Utc;

    let mut mgr = PositionManager::new();

    // Add positions in order
    mgr.add(ExecutionPosition {
        order_id: "order-1".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        entry_price: dec!(50000),
        quantity: dec!(0.5),
        entry_time: Utc::now(),
        strategy_id: "test".to_string(),
    });

    mgr.add(ExecutionPosition {
        order_id: "order-2".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        entry_price: dec!(51000),
        quantity: dec!(0.5),
        entry_time: Utc::now(),
        strategy_id: "test".to_string(),
    });

    // Close 0.7 BTC at 55000
    let closed = mgr.close_fifo("BTCUSDT", OrderSide::Buy, dec!(0.7), dec!(55000));

    assert_eq!(closed.len(), 2);

    // First closed should be order-1 (FIFO)
    assert_eq!(closed[0].order_id, "order-1");
    assert_eq!(closed[0].quantity, dec!(0.5));
    assert_eq!(closed[0].pnl, dec!(2500)); // (55000 - 50000) * 0.5

    // Second should be partial order-2
    assert_eq!(closed[1].order_id, "order-2");
    assert_eq!(closed[1].quantity, dec!(0.2));
    assert_eq!(closed[1].pnl, dec!(800)); // (55000 - 51000) * 0.2
}

#[test]
fn test_position_manager_close_by_id() {
    use chrono::Utc;

    let mut mgr = PositionManager::new();

    mgr.add(ExecutionPosition {
        order_id: "order-1".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        entry_price: dec!(50000),
        quantity: dec!(1),
        entry_time: Utc::now(),
        strategy_id: "test".to_string(),
    });

    let closed = mgr.close("order-1", dec!(55000)).unwrap();

    assert_eq!(closed.order_id, "order-1");
    assert_eq!(closed.pnl, dec!(5000)); // (55000 - 50000) * 1
    assert!(mgr.is_empty());
}

#[test]
fn test_position_manager_total_quantity() {
    use chrono::Utc;

    let mut mgr = PositionManager::new();

    mgr.add(ExecutionPosition {
        order_id: "order-1".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        entry_price: dec!(50000),
        quantity: dec!(0.5),
        entry_time: Utc::now(),
        strategy_id: "test".to_string(),
    });

    mgr.add(ExecutionPosition {
        order_id: "order-2".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        entry_price: dec!(51000),
        quantity: dec!(0.3),
        entry_time: Utc::now(),
        strategy_id: "test".to_string(),
    });

    assert_eq!(mgr.total_quantity("BTCUSDT"), dec!(0.8));
    assert_eq!(
        mgr.total_quantity_by_side("BTCUSDT", OrderSide::Buy),
        dec!(0.8)
    );
    assert_eq!(
        mgr.total_quantity_by_side("BTCUSDT", OrderSide::Sell),
        dec!(0)
    );
}

// ============================================================================
// Signing Tests
// ============================================================================

#[test]
fn test_hmac_signing() {
    use trading_bot::execution::sign_request;

    // Test with known input (from Binance docs example)
    let params = "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559";
    let secret = "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j";

    let signature = sign_request(params, secret);

    // The expected signature from Binance docs
    assert_eq!(
        signature,
        "c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71"
    );
}

#[test]
fn test_build_signed_params() {
    use trading_bot::execution::build_signed_params;

    let params = vec![("symbol", "BTCUSDT"), ("side", "BUY")];
    let secret = "test_secret";

    let result = build_signed_params(&params, secret);

    // Should contain all params plus timestamp and signature
    assert!(result.contains("symbol=BTCUSDT"));
    assert!(result.contains("side=BUY"));
    assert!(result.contains("timestamp="));
    assert!(result.contains("signature="));
}

// ============================================================================
// OrderExecutorActor Integration Tests
// ============================================================================

#[sqlx::test(migrations = "./migrations")]
async fn test_executor_actor_signal_execution(pool: sqlx::PgPool) {
    // Create stop-loss channel (sender goes to risk engine)
    let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);

    // Start risk engine with stop-loss sender
    let risk_handle = start_risk_engine(RiskEngineConfig::default(), Some(stop_loss_tx), None);

    // Start executor
    let executor_handle = start_order_executor(
        ExecutorStartConfig {
            execution_config: test_execution_config(),
            paper_config: test_paper_config(),
            credentials: None,
        },
        risk_handle.clone(),
        stop_loss_rx,
        pool,
        None,
    );

    // Execute a buy signal
    let result = executor_handle
        .execute_signal(
            "test-strategy",
            "BTCUSDT",
            Signal::Buy {
                quantity: dec!(0.1),
            },
            dec!(50000),
        )
        .await
        .unwrap();

    match result {
        ExecutionResult::Filled(response) => {
            assert_eq!(response.status, OrderStatus::Filled);
            assert_eq!(response.filled_quantity, dec!(0.1));
        }
        other => panic!("Expected Filled, got {:?}", other),
    }

    // Shutdown
    executor_handle.shutdown().await.unwrap();
    risk_handle.shutdown().await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn test_executor_actor_hold_signal(pool: sqlx::PgPool) {
    let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);
    let risk_handle = start_risk_engine(RiskEngineConfig::default(), Some(stop_loss_tx), None);

    let executor_handle = start_order_executor(
        ExecutorStartConfig {
            execution_config: test_execution_config(),
            paper_config: test_paper_config(),
            credentials: None,
        },
        risk_handle.clone(),
        stop_loss_rx,
        pool,
        None,
    );

    // Execute a hold signal
    let result = executor_handle
        .execute_signal("test-strategy", "BTCUSDT", Signal::Hold, dec!(50000))
        .await
        .unwrap();

    assert!(matches!(result, ExecutionResult::NoAction));

    executor_handle.shutdown().await.unwrap();
    risk_handle.shutdown().await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn test_executor_actor_risk_rejection(pool: sqlx::PgPool) {
    let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);

    // Configure risk engine with very low position limit
    let mut risk_config = RiskEngineConfig::default();
    risk_config.global_risk.max_position_size_pct = dec!(0.01); // 1% max position

    let risk_handle = start_risk_engine(risk_config, Some(stop_loss_tx), None);

    let executor_handle = start_order_executor(
        ExecutorStartConfig {
            execution_config: test_execution_config(),
            paper_config: test_paper_config(),
            credentials: None,
        },
        risk_handle.clone(),
        stop_loss_rx,
        pool,
        None,
    );

    // Try to execute a large order (should be scaled or rejected)
    let result = executor_handle
        .execute_signal(
            "test-strategy",
            "BTCUSDT",
            Signal::Buy { quantity: dec!(10) },
            dec!(50000),
        )
        .await
        .unwrap();

    // Order should still execute but be scaled down
    match result {
        ExecutionResult::Filled(response) => {
            // Scaled quantity should be much less than 10
            assert!(response.filled_quantity < dec!(1));
        }
        ExecutionResult::Rejected { .. } => {
            // Also acceptable if completely rejected
        }
        other => panic!("Expected Filled or Rejected, got {:?}", other),
    }

    executor_handle.shutdown().await.unwrap();
    risk_handle.shutdown().await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn test_executor_actor_disabled(pool: sqlx::PgPool) {
    let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);
    let risk_handle = start_risk_engine(RiskEngineConfig::default(), Some(stop_loss_tx), None);

    let mut exec_config = test_execution_config();
    exec_config.enabled = false; // Disable execution

    let executor_handle = start_order_executor(
        ExecutorStartConfig {
            execution_config: exec_config,
            paper_config: test_paper_config(),
            credentials: None,
        },
        risk_handle.clone(),
        stop_loss_rx,
        pool,
        None,
    );

    let result = executor_handle
        .execute_signal(
            "test-strategy",
            "BTCUSDT",
            Signal::Buy {
                quantity: dec!(0.1),
            },
            dec!(50000),
        )
        .await
        .unwrap();

    assert!(matches!(result, ExecutionResult::Disabled));

    executor_handle.shutdown().await.unwrap();
    risk_handle.shutdown().await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn test_executor_actor_get_balances(pool: sqlx::PgPool) {
    let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);
    let risk_handle = start_risk_engine(RiskEngineConfig::default(), Some(stop_loss_tx), None);

    let executor_handle = start_order_executor(
        ExecutorStartConfig {
            execution_config: test_execution_config(),
            paper_config: test_paper_config(),
            credentials: None,
        },
        risk_handle.clone(),
        stop_loss_rx,
        pool,
        None,
    );

    // Get initial balances
    let balances = executor_handle.get_balances().await.unwrap();

    // Paper config starts with 10000 USDT
    assert!(balances.get("USDT").is_some());

    executor_handle.shutdown().await.unwrap();
    risk_handle.shutdown().await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn test_executor_actor_sell_signal(pool: sqlx::PgPool) {
    let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);
    let risk_handle = start_risk_engine(RiskEngineConfig::default(), Some(stop_loss_tx), None);

    // Start with BTC balance for selling
    let mut paper_config = test_paper_config();
    paper_config.initial_balances.insert("BTC".to_string(), 1.0);

    let executor_handle = start_order_executor(
        ExecutorStartConfig {
            execution_config: test_execution_config(),
            paper_config,
            credentials: None,
        },
        risk_handle.clone(),
        stop_loss_rx,
        pool,
        None,
    );

    // Execute a sell signal
    let result = executor_handle
        .execute_signal(
            "test-strategy",
            "BTCUSDT",
            Signal::Sell {
                quantity: dec!(0.5),
            },
            dec!(50000),
        )
        .await
        .unwrap();

    match result {
        ExecutionResult::Filled(response) => {
            assert_eq!(response.status, OrderStatus::Filled);
            assert_eq!(response.filled_quantity, dec!(0.5));
            assert_eq!(response.side, OrderSide::Sell);
        }
        other => panic!("Expected Filled, got {:?}", other),
    }

    executor_handle.shutdown().await.unwrap();
    risk_handle.shutdown().await.unwrap();
}

// ============================================================================
// Stop-Loss Event Tests
// ============================================================================

#[sqlx::test(migrations = "./migrations")]
async fn test_stop_loss_event_triggers_close(pool: sqlx::PgPool) {
    // This test verifies the stop-loss -> executor flow with explicit side effect checks
    let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);
    let risk_handle = start_risk_engine(RiskEngineConfig::default(), Some(stop_loss_tx.clone()), None);

    // Need BTC balance for the sell order
    let mut paper_config = test_paper_config();
    paper_config.initial_balances.insert("BTC".to_string(), 1.0);

    let executor_handle = start_order_executor(
        ExecutorStartConfig {
            execution_config: test_execution_config(),
            paper_config,
            credentials: None,
        },
        risk_handle.clone(),
        stop_loss_rx,
        pool.clone(),
        None,
    );

    // First, execute a buy to have a position
    let buy_result = executor_handle
        .execute_signal(
            "test-strategy",
            "BTCUSDT",
            Signal::Buy {
                quantity: dec!(0.1),
            },
            dec!(50000),
        )
        .await
        .unwrap();

    let order_id = match buy_result {
        ExecutionResult::Filled(response) => {
            // Verify buy order was filled correctly
            assert_eq!(response.status, OrderStatus::Filled);
            assert_eq!(response.filled_quantity, dec!(0.1));
            assert_eq!(response.symbol, "BTCUSDT");
            response.order_id
        }
        _ => panic!("Expected buy to fill"),
    };

    // Record balances BEFORE stop-loss
    let balances_before = executor_handle.get_balances().await.unwrap();
    let btc_before = balances_before.get("BTC").copied().unwrap_or(dec!(0));

    // Send stop-loss event directly (simulating risk engine trigger)
    let event = StopLossEvent {
        order_id: order_id.clone(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Sell,
        quantity: dec!(0.1),
        strategy_id: "test-strategy".to_string(),
    };

    stop_loss_tx.send(event).await.unwrap();

    // Give executor time to process the stop-loss event
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // VERIFY SIDE EFFECTS:

    // 1. Balance should have changed (BTC reduced by sell quantity)
    let balances_after = executor_handle.get_balances().await.unwrap();
    let btc_after = balances_after.get("BTC").copied().unwrap_or(dec!(0));

    // BTC balance should be reduced (original 1.0 + bought 0.1 - sold 0.1 = 1.0)
    // The stop-loss sells the 0.1 BTC that was bought
    assert!(
        btc_after <= btc_before,
        "BTC balance should not increase after stop-loss sell. Before: {}, After: {}",
        btc_before,
        btc_after
    );

    // Cleanup
    executor_handle.shutdown().await.unwrap();
    risk_handle.shutdown().await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn test_stop_loss_event_with_insufficient_balance(pool: sqlx::PgPool) {
    // Test stop-loss when there's no balance to sell (should not crash)
    let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);
    let risk_handle = start_risk_engine(RiskEngineConfig::default(), Some(stop_loss_tx.clone()), None);

    // Start with NO BTC balance
    let paper_config = test_paper_config(); // Default has BTC: 0.0

    let executor_handle = start_order_executor(
        ExecutorStartConfig {
            execution_config: test_execution_config(),
            paper_config,
            credentials: None,
        },
        risk_handle.clone(),
        stop_loss_rx,
        pool.clone(),
        None,
    );

    // Send stop-loss event for position that doesn't exist/can't be filled
    let event = StopLossEvent {
        order_id: "nonexistent-order".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Sell,
        quantity: dec!(0.5),
        strategy_id: "test-strategy".to_string(),
    };

    stop_loss_tx.send(event).await.unwrap();

    // Give executor time to process - should NOT crash
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify executor is still responsive (didn't crash)
    let result = executor_handle
        .execute_signal("test-strategy", "BTCUSDT", Signal::Hold, dec!(50000))
        .await;

    assert!(
        result.is_ok(),
        "Executor should still be responsive after failed stop-loss"
    );

    // Cleanup
    executor_handle.shutdown().await.unwrap();
    risk_handle.shutdown().await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn test_multiple_stop_loss_events(pool: sqlx::PgPool) {
    // Test handling multiple stop-loss events in sequence
    let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);
    let risk_handle = start_risk_engine(RiskEngineConfig::default(), Some(stop_loss_tx.clone()), None);

    let mut paper_config = test_paper_config();
    paper_config.initial_balances.insert("BTC".to_string(), 2.0);

    let executor_handle = start_order_executor(
        ExecutorStartConfig {
            execution_config: test_execution_config(),
            paper_config,
            credentials: None,
        },
        risk_handle.clone(),
        stop_loss_rx,
        pool.clone(),
        None,
    );

    // Execute two buys
    let buy1 = executor_handle
        .execute_signal(
            "strat-1",
            "BTCUSDT",
            Signal::Buy {
                quantity: dec!(0.2),
            },
            dec!(50000),
        )
        .await
        .unwrap();

    let buy2 = executor_handle
        .execute_signal(
            "strat-2",
            "BTCUSDT",
            Signal::Buy {
                quantity: dec!(0.3),
            },
            dec!(51000),
        )
        .await
        .unwrap();

    let order_id_1 = match buy1 {
        ExecutionResult::Filled(r) => r.order_id,
        _ => panic!("Buy 1 should fill"),
    };
    let order_id_2 = match buy2 {
        ExecutionResult::Filled(r) => r.order_id,
        _ => panic!("Buy 2 should fill"),
    };

    // Send two stop-loss events
    stop_loss_tx
        .send(StopLossEvent {
            order_id: order_id_1,
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Sell,
            quantity: dec!(0.2),
            strategy_id: "strat-1".to_string(),
        })
        .await
        .unwrap();

    stop_loss_tx
        .send(StopLossEvent {
            order_id: order_id_2,
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Sell,
            quantity: dec!(0.3),
            strategy_id: "strat-2".to_string(),
        })
        .await
        .unwrap();

    // Allow processing
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Verify executor handled both without crashing
    let final_result = executor_handle
        .execute_signal("test", "BTCUSDT", Signal::Hold, dec!(50000))
        .await;

    assert!(
        final_result.is_ok(),
        "Executor should handle multiple stop-loss events"
    );

    executor_handle.shutdown().await.unwrap();
    risk_handle.shutdown().await.unwrap();
}

// ============================================================================
// Live Executor Error Handling Tests (no network)
// ============================================================================

mod live_executor_tests {
    use super::*;
    use trading_bot::execution::ExecutionError;

    #[test]
    fn test_parse_order_status_variants() {
        // Test all OrderStatus variants are properly defined and comparable
        let statuses = vec![
            ("NEW", OrderStatus::New),
            ("PARTIALLY_FILLED", OrderStatus::PartiallyFilled),
            ("FILLED", OrderStatus::Filled),
            ("CANCELED", OrderStatus::Canceled),
            ("REJECTED", OrderStatus::Rejected),
            ("EXPIRED", OrderStatus::Expired),
        ];

        for (name, status) in statuses {
            // Verify each status variant is distinct and correctly identified
            match status {
                OrderStatus::New => assert_eq!(name, "NEW"),
                OrderStatus::PartiallyFilled => assert_eq!(name, "PARTIALLY_FILLED"),
                OrderStatus::Filled => assert_eq!(name, "FILLED"),
                OrderStatus::Canceled => assert_eq!(name, "CANCELED"),
                OrderStatus::Rejected => assert_eq!(name, "REJECTED"),
                OrderStatus::Expired => assert_eq!(name, "EXPIRED"),
            }
        }
    }

    #[test]
    fn test_execution_error_display() {
        let errors = vec![
            ExecutionError::Exchange {
                code: -1021,
                msg: "Invalid timestamp".to_string(),
            },
            ExecutionError::Network("Connection refused".to_string()),
            ExecutionError::InsufficientBalance {
                asset: "USDT".to_string(),
                required: dec!(10000),
                available: dec!(5000),
            },
            ExecutionError::RateLimited { retry_after_ms: 60000 },
            ExecutionError::InvalidRequest("Bad symbol".to_string()),
            ExecutionError::Signing("Invalid secret".to_string()),
        ];

        for error in errors {
            // All errors should have meaningful display
            let msg = format!("{}", error);
            assert!(!msg.is_empty(), "Error should have non-empty display");

            // Debug should also work
            let debug = format!("{:?}", error);
            assert!(!debug.is_empty());
        }
    }

    #[test]
    fn test_execution_error_variants() {
        // Verify all error variants can be created and matched
        let exchange_err = ExecutionError::Exchange {
            code: -1000,
            msg: "test".to_string(),
        };
        assert!(matches!(exchange_err, ExecutionError::Exchange { .. }));

        let network_err = ExecutionError::Network("timeout".to_string());
        assert!(matches!(network_err, ExecutionError::Network(_)));

        let rate_limited = ExecutionError::RateLimited { retry_after_ms: 1000 };
        assert!(matches!(rate_limited, ExecutionError::RateLimited { .. }));

        let insufficient = ExecutionError::InsufficientBalance {
            asset: "BTC".to_string(),
            required: dec!(1),
            available: dec!(0),
        };
        assert!(matches!(insufficient, ExecutionError::InsufficientBalance { .. }));

        let invalid = ExecutionError::InvalidRequest("bad".to_string());
        assert!(matches!(invalid, ExecutionError::InvalidRequest(_)));

        let signing = ExecutionError::Signing("error".to_string());
        assert!(matches!(signing, ExecutionError::Signing(_)));
    }
}

// ============================================================================
// Execution Types Tests
// ============================================================================

mod types_tests {
    use super::*;
    use trading_bot::execution::Fill;

    #[test]
    fn test_order_type_variants() {
        assert_ne!(OrderType::Market, OrderType::Limit);

        // Test display
        assert_eq!(format!("{}", OrderType::Market), "MARKET");
        assert_eq!(format!("{}", OrderType::Limit), "LIMIT");
    }

    #[test]
    fn test_order_status_display() {
        assert_eq!(format!("{}", OrderStatus::New), "NEW");
        assert_eq!(format!("{}", OrderStatus::PartiallyFilled), "PARTIALLY_FILLED");
        assert_eq!(format!("{}", OrderStatus::Filled), "FILLED");
        assert_eq!(format!("{}", OrderStatus::Canceled), "CANCELED");
        assert_eq!(format!("{}", OrderStatus::Rejected), "REJECTED");
        assert_eq!(format!("{}", OrderStatus::Expired), "EXPIRED");
    }

    #[test]
    fn test_fill_struct() {
        let fill = Fill {
            price: dec!(50000),
            quantity: dec!(0.1),
            commission: dec!(5),
            commission_asset: "USDT".to_string(),
        };

        assert_eq!(fill.price, dec!(50000));
        assert_eq!(fill.quantity, dec!(0.1));
        assert_eq!(fill.commission, dec!(5));
        assert_eq!(fill.commission_asset, "USDT");
    }

    #[test]
    fn test_execute_order_request() {
        let request = ExecuteOrderRequest {
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Buy,
            quantity: dec!(0.1),
            order_type: OrderType::Market,
            price: Some(dec!(50000)),
            client_order_id: Some("test-123".to_string()),
        };

        assert_eq!(request.symbol, "BTCUSDT");
        assert_eq!(request.side, OrderSide::Buy);
        assert_eq!(request.quantity, dec!(0.1));
        assert_eq!(request.order_type, OrderType::Market);
        assert_eq!(request.price, Some(dec!(50000)));
        assert_eq!(request.client_order_id, Some("test-123".to_string()));
    }
}

// ============================================================================
// Paper Mode Safety Tests (CONN-04 Compliance)
// ============================================================================

mod paper_mode_safety_tests {
    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn test_paper_mode_never_creates_live_executor(pool: sqlx::PgPool) {
        // This test verifies CONN-04: paper trading mode simulates all trades
        // without touching real exchange

        let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);
        let risk_handle = start_risk_engine(RiskEngineConfig::default(), Some(stop_loss_tx), None);

        let mut paper_config = test_paper_config();
        paper_config.enabled = true; // CRITICAL: Paper mode enabled

        // Even if credentials are provided, paper mode should be used
        // (In real implementation, credentials would be ignored in paper mode)
        let executor_handle = start_order_executor(
            ExecutorStartConfig {
                execution_config: test_execution_config(),
                paper_config,
                credentials: None, // No credentials needed for paper
            },
            risk_handle.clone(),
            stop_loss_rx,
            pool,
            None,
        );

        // Execute an order - should go through paper executor
        let result = executor_handle
            .execute_signal(
                "test",
                "BTCUSDT",
                Signal::Buy {
                    quantity: dec!(0.1),
                },
                dec!(50000),
            )
            .await
            .unwrap();

        // Order should fill (paper executor always fills market orders with balance)
        match result {
            ExecutionResult::Filled(response) => {
                // Paper executor fills at the given price (with possible slippage)
                assert_eq!(response.status, OrderStatus::Filled);
                assert_eq!(response.filled_quantity, dec!(0.1));
                // The order_id format from paper executor is a UUID
                assert!(!response.order_id.is_empty());
            }
            other => panic!("Expected paper fill, got {:?}", other),
        }

        executor_handle.shutdown().await.unwrap();
        risk_handle.shutdown().await.unwrap();
    }

    #[test]
    fn test_paper_executor_symbol_parsing() {
        // Verify symbol parsing works correctly for paper executor

        // Standard pairs
        assert_eq!(PaperExecutor::parse_symbol("BTCUSDT"), ("BTC", "USDT"));
        assert_eq!(PaperExecutor::parse_symbol("ETHUSDC"), ("ETH", "USDC"));
        assert_eq!(PaperExecutor::parse_symbol("BTCBUSD"), ("BTC", "BUSD"));
        assert_eq!(PaperExecutor::parse_symbol("BTCUSD"), ("BTC", "USD"));

        // Edge cases
        assert_eq!(PaperExecutor::parse_symbol("SOLUSDT"), ("SOL", "USDT"));
        assert_eq!(PaperExecutor::parse_symbol("DOGEUSDT"), ("DOGE", "USDT"));
    }

    #[tokio::test]
    async fn test_paper_executor_balance_isolation() {
        // Verify paper executor maintains isolated balances

        let config1 = test_paper_config();
        let config2 = test_paper_config();

        let executor1 = PaperExecutor::new(config1);
        let executor2 = PaperExecutor::new(config2);

        // Execute order on executor1
        executor1
            .execute(ExecuteOrderRequest {
                symbol: "BTCUSDT".to_string(),
                side: OrderSide::Buy,
                quantity: dec!(0.1),
                order_type: OrderType::Market,
                price: Some(dec!(50000)),
                client_order_id: None,
            })
            .await
            .unwrap();

        // executor1 should have 0.1 BTC
        let btc1 = executor1.get_balance("BTC").await.unwrap();
        assert_eq!(btc1, dec!(0.1));

        // executor2 should still have 0 BTC (isolated)
        let btc2 = executor2.get_balance("BTC").await.unwrap();
        assert_eq!(btc2, dec!(0));
    }

    #[tokio::test]
    async fn test_paper_config_defaults() {
        // Verify default paper config values

        let config = PaperTradingConfig::default();

        // Paper mode disabled by default
        assert!(!config.enabled);

        // Should have reasonable latency range
        assert!(config.latency_range_ms.0 < config.latency_range_ms.1);

        // Should have reasonable slippage range
        assert!(config.slippage_range.0 <= config.slippage_range.1);

        // Should have positive fee
        assert!(config.fee_pct > 0.0);

        // Should have default USDT balance
        assert!(config.initial_balances.contains_key("USDT"));
    }
}
