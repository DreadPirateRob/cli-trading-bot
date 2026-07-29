//! Integration tests for exchange connectivity
//!
//! These tests connect to the real Binance.US API.
//! Run with: cargo test --test exchange_integration -- --ignored
//!
//! Note: Requires network connectivity to Binance.US

use std::time::Duration;
use tokio::time::timeout;
use trading_bot::exchange::{
    connect_trade_stream, start_reconnecting_stream, ReconnectConfig, BINANCE_WS_URL,
};

/// Test that we can connect and receive at least one trade
///
/// This test is ignored by default because it requires network access.
/// BTCUSDT is a high-volume pair, so we should receive trades quickly.
#[tokio::test]
#[ignore] // Run with: cargo test --test exchange_integration -- --ignored
async fn test_live_trade_stream() {
    // Initialize tracing for test visibility
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();

    // Connect to trade stream
    let mut handle = connect_trade_stream("btcusdt", None, 100)
        .await
        .expect("Failed to connect");

    // Wait for at least one trade (timeout after 30 seconds)
    let trade = timeout(Duration::from_secs(30), handle.trades.recv())
        .await
        .expect("Timeout waiting for trade")
        .expect("Channel closed without receiving trade");

    // Verify trade has expected fields
    assert_eq!(trade.event_type, "trade");
    assert_eq!(trade.symbol, "BTCUSDT");
    assert!(!trade.price.is_empty());
    assert!(!trade.quantity.is_empty());
    assert!(trade.trade_id > 0);

    println!(
        "Received trade: id={} price={} qty={} is_buy={}",
        trade.trade_id, trade.price, trade.quantity, trade.is_buy()
    );

    // Clean shutdown
    handle.shutdown().await;
}

/// Test that connection to invalid URL fails appropriately
#[tokio::test]
async fn test_invalid_url_fails() {
    let result = connect_trade_stream("btcusdt", Some("wss://invalid.example.com:9443/ws"), 100).await;
    assert!(result.is_err());
}

/// Test URL construction with different symbols
#[tokio::test]
async fn test_url_construction() {
    // This just tests that we can construct URLs without panicking
    // Actual connection would require network
    let url = format!("{}/{}@trade", BINANCE_WS_URL, "ethusdt");
    assert_eq!(url, "wss://stream.binance.us:9443/ws/ethusdt@trade");
}

/// Test reconnecting stream can connect and receive trades
///
/// This verifies the reconnection wrapper works for the happy path.
/// Testing actual reconnection would require simulating network failure.
#[tokio::test]
#[ignore] // Run with: cargo test --test exchange_integration -- --ignored
async fn test_reconnecting_trade_stream() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();

    // Use short timeouts for testing
    let config = ReconnectConfig {
        max_elapsed_time: Some(Duration::from_secs(30)),
        ..Default::default()
    };

    let mut stream = start_reconnecting_stream("btcusdt", None, 100, config).await;

    // Wait for at least one trade
    let trade = timeout(Duration::from_secs(30), stream.trades.recv())
        .await
        .expect("Timeout waiting for trade")
        .expect("Stream closed without trade");

    assert_eq!(trade.event_type, "trade");
    assert_eq!(trade.symbol, "BTCUSDT");

    println!(
        "Reconnecting stream received trade: id={} price={}",
        trade.trade_id, trade.price
    );

    stream.shutdown().await;
}
