//! Database integration tests
//!
//! Uses #[sqlx::test] for automatic test database isolation.
//! Each test gets a fresh database with migrations applied.
//!
//! Requires DATABASE_URL environment variable pointing to PostgreSQL.
//! Run with: DATABASE_URL=postgres://... cargo test --test database_integration

use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use sqlx::PgPool;
use std::str::FromStr;

use trading_bot::data::{
    batch_insert_ticks, calculate_pnl, close_order, get_open_orders, get_order,
    insert_order, CloseOrder, NewOrder, NewTick, OrderSide,
};

// Helper to create test ticks
fn make_test_tick(trade_id: i64, price: &str, quantity: &str) -> NewTick {
    NewTick {
        timestamp: Utc::now(),
        symbol: "BTCUSDT".to_string(),
        price: Decimal::from_str(price).unwrap(),
        quantity: Decimal::from_str(quantity).unwrap(),
        trade_id,
        is_buyer_maker: false,
    }
}

// ============================================================================
// Tick Tests
// ============================================================================

#[sqlx::test(migrations = "./migrations")]
async fn test_batch_insert_single_tick(pool: PgPool) {
    let ticks = vec![make_test_tick(1, "50000.12345678", "0.001")];

    let rows = batch_insert_ticks(&pool, &ticks).await.unwrap();
    assert_eq!(rows, 1);

    // Verify the tick was stored with correct precision
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ticks")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);

    // Verify decimal precision preserved
    let (price,): (Decimal,) = sqlx::query_as("SELECT price FROM ticks WHERE trade_id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(price.to_string(), "50000.12345678");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_batch_insert_multiple_ticks(pool: PgPool) {
    let ticks: Vec<NewTick> = (1..=100)
        .map(|i| make_test_tick(i, "50000.00", "0.001"))
        .collect();

    let rows = batch_insert_ticks(&pool, &ticks).await.unwrap();
    assert_eq!(rows, 100);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_batch_insert_empty_vec(pool: PgPool) {
    let rows = batch_insert_ticks(&pool, &[]).await.unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_batch_insert_duplicate_trade_id(pool: PgPool) {
    let ticks = vec![
        make_test_tick(1, "50000.00", "0.001"),
        make_test_tick(1, "50001.00", "0.002"), // Duplicate trade_id
    ];

    // Should insert first, skip duplicate (ON CONFLICT DO NOTHING)
    let rows = batch_insert_ticks(&pool, &ticks).await.unwrap();
    assert_eq!(rows, 1);

    // Verify only first tick's price was stored
    let (price,): (Decimal,) = sqlx::query_as("SELECT price FROM ticks WHERE trade_id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(price, dec!(50000.00));
}

#[sqlx::test(migrations = "./migrations")]
async fn test_batch_insert_preserves_small_decimals(pool: PgPool) {
    // Test precision for very small numbers (like satoshi prices)
    let ticks = vec![make_test_tick(1, "0.00000001", "100000000.00000001")];

    batch_insert_ticks(&pool, &ticks).await.unwrap();

    let row: (Decimal, Decimal) =
        sqlx::query_as("SELECT price, quantity FROM ticks WHERE trade_id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(row.0.to_string(), "0.00000001");
    assert_eq!(row.1.to_string(), "100000000.00000001");
}

// ============================================================================
// Order Tests
// ============================================================================

#[sqlx::test(migrations = "./migrations")]
async fn test_insert_order(pool: PgPool) {
    let order = NewOrder {
        order_id: "ORD-001".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        entry_price: dec!(50000.00),
        quantity: dec!(0.1),
        reason: "RSI oversold signal".to_string(),
        entry_time: Utc::now(),
        trading_mode: "paper".to_string(),
    };

    let id = insert_order(&pool, &order).await.unwrap();
    assert!(id > 0);

    // Verify order was stored
    let stored = get_order(&pool, "ORD-001", "paper").await.unwrap().unwrap();
    assert_eq!(stored.symbol, "BTCUSDT");
    assert_eq!(stored.side, "buy");
    assert_eq!(stored.entry_price, dec!(50000.00));
    assert!(stored.is_open());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_close_order(pool: PgPool) {
    // Insert open order
    let order = NewOrder {
        order_id: "ORD-002".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        entry_price: dec!(50000.00),
        quantity: dec!(0.1),
        reason: "Test order".to_string(),
        entry_time: Utc::now(),
        trading_mode: "paper".to_string(),
    };
    insert_order(&pool, &order).await.unwrap();

    // Close the order
    let close = CloseOrder {
        exit_price: dec!(51000.00),
        exit_time: Utc::now(),
        pnl: calculate_pnl(OrderSide::Buy, dec!(50000.00), dec!(51000.00), dec!(0.1)),
    };
    let closed = close_order(&pool, "ORD-002", &close, "paper").await.unwrap();
    assert!(closed);

    // Verify order was closed
    let stored = get_order(&pool, "ORD-002", "paper").await.unwrap().unwrap();
    assert!(!stored.is_open());
    assert_eq!(stored.exit_price, Some(dec!(51000.00)));
    assert_eq!(stored.pnl, Some(dec!(100.00))); // (51000-50000) * 0.1
}

#[sqlx::test(migrations = "./migrations")]
async fn test_close_already_closed_order(pool: PgPool) {
    // Insert and close order
    let order = NewOrder {
        order_id: "ORD-003".to_string(),
        symbol: "BTCUSDT".to_string(),
        side: OrderSide::Buy,
        entry_price: dec!(50000.00),
        quantity: dec!(0.1),
        reason: "Test".to_string(),
        entry_time: Utc::now(),
        trading_mode: "paper".to_string(),
    };
    insert_order(&pool, &order).await.unwrap();

    let close = CloseOrder {
        exit_price: dec!(51000.00),
        exit_time: Utc::now(),
        pnl: dec!(100.00),
    };
    close_order(&pool, "ORD-003", &close, "paper").await.unwrap();

    // Try to close again - should return false
    let close2 = CloseOrder {
        exit_price: dec!(52000.00),
        exit_time: Utc::now(),
        pnl: dec!(200.00),
    };
    let closed = close_order(&pool, "ORD-003", &close2, "paper").await.unwrap();
    assert!(!closed);

    // Verify original close values unchanged
    let stored = get_order(&pool, "ORD-003", "paper").await.unwrap().unwrap();
    assert_eq!(stored.exit_price, Some(dec!(51000.00)));
    assert_eq!(stored.pnl, Some(dec!(100.00)));
}

#[sqlx::test(migrations = "./migrations")]
async fn test_get_open_orders(pool: PgPool) {
    // Insert mix of open and closed orders
    let orders = vec![
        NewOrder {
            order_id: "OPEN-1".to_string(),
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Buy,
            entry_price: dec!(50000.00),
            quantity: dec!(0.1),
            reason: "Open 1".to_string(),
            entry_time: Utc::now(),
            trading_mode: "paper".to_string(),
        },
        NewOrder {
            order_id: "OPEN-2".to_string(),
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Sell,
            entry_price: dec!(51000.00),
            quantity: dec!(0.2),
            reason: "Open 2".to_string(),
            entry_time: Utc::now(),
            trading_mode: "paper".to_string(),
        },
        NewOrder {
            order_id: "CLOSED-1".to_string(),
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Buy,
            entry_price: dec!(49000.00),
            quantity: dec!(0.1),
            reason: "To be closed".to_string(),
            entry_time: Utc::now(),
            trading_mode: "paper".to_string(),
        },
    ];

    for order in &orders {
        insert_order(&pool, order).await.unwrap();
    }

    // Close one order
    let close = CloseOrder {
        exit_price: dec!(49500.00),
        exit_time: Utc::now(),
        pnl: dec!(50.00),
    };
    close_order(&pool, "CLOSED-1", &close, "paper").await.unwrap();

    // Get open orders
    let open = get_open_orders(&pool, "BTCUSDT", "paper").await.unwrap();
    assert_eq!(open.len(), 2);
    assert!(open.iter().all(|o| o.is_open()));
}

#[sqlx::test(migrations = "./migrations")]
async fn test_order_not_found(pool: PgPool) {
    let result = get_order(&pool, "NONEXISTENT", "paper").await.unwrap();
    assert!(result.is_none());
}
