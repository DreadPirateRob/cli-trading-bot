//! Reconnecting WebSocket client with exponential backoff
//!
//! Wraps the base trade stream client to provide automatic reconnection
//! when the connection drops.

use std::time::Duration;

use backoff::{backoff::Backoff, ExponentialBackoff, ExponentialBackoffBuilder};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::exchange::client::connect_trade_stream;
use crate::exchange::error::ExchangeError;
use crate::exchange::messages::TradeEvent;

/// Configuration for reconnection behavior
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Initial delay before first retry (default: 100ms)
    pub initial_interval: Duration,
    /// Maximum delay between retries (default: 60s)
    pub max_interval: Duration,
    /// Maximum total time to keep retrying (default: 5 minutes)
    /// Set to None for unlimited retries
    pub max_elapsed_time: Option<Duration>,
    /// Multiplier for exponential increase (default: 2.0)
    pub multiplier: f64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            initial_interval: Duration::from_millis(100),
            max_interval: Duration::from_secs(60),
            max_elapsed_time: Some(Duration::from_secs(300)), // 5 minutes
            multiplier: 2.0,
        }
    }
}

impl ReconnectConfig {
    /// Create config for unlimited retries (useful for production)
    pub fn unlimited() -> Self {
        Self {
            max_elapsed_time: None,
            ..Default::default()
        }
    }

    fn build_backoff(&self) -> ExponentialBackoff {
        ExponentialBackoffBuilder::default()
            .with_initial_interval(self.initial_interval)
            .with_max_interval(self.max_interval)
            .with_max_elapsed_time(self.max_elapsed_time)
            .with_multiplier(self.multiplier)
            .build()
    }
}

/// Handle to a reconnecting trade stream
///
/// Provides the same interface as TradeStreamHandle but with automatic
/// reconnection when the connection drops.
pub struct ReconnectingTradeStream {
    /// Receiver for trade events (persists across reconnections)
    pub trades: mpsc::Receiver<TradeEvent>,
    /// Signal to stop reconnection attempts
    shutdown_tx: mpsc::Sender<()>,
}

impl ReconnectingTradeStream {
    /// Signal the stream to shut down and stop reconnecting
    pub async fn shutdown(self) {
        drop(self.shutdown_tx);
    }
}

/// Connection status for monitoring
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Currently connected and receiving data
    Connected,
    /// Attempting to reconnect
    Reconnecting { attempt: u32 },
    /// Disconnected and not retrying (max retries exceeded or shutdown)
    Disconnected { reason: String },
}

/// Start a reconnecting trade stream
///
/// Spawns a task that maintains the WebSocket connection, automatically
/// reconnecting with exponential backoff when disconnected.
///
/// # Arguments
/// * `symbol` - Trading pair (e.g., "btcusdt")
/// * `ws_url` - Optional custom WebSocket URL
/// * `buffer_size` - Channel buffer size for trade events
/// * `reconnect_config` - Configuration for backoff behavior
///
/// # Example
/// ```ignore
/// let stream = start_reconnecting_stream(
///     "btcusdt",
///     None,
///     1000,
///     ReconnectConfig::unlimited(),
/// ).await;
///
/// while let Some(trade) = stream.trades.recv().await {
///     process_trade(trade);
/// }
/// ```
pub async fn start_reconnecting_stream(
    symbol: &str,
    ws_url: Option<&str>,
    buffer_size: usize,
    reconnect_config: ReconnectConfig,
) -> ReconnectingTradeStream {
    let (trades_tx, trades_rx) = mpsc::channel(buffer_size);
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    let symbol = symbol.to_string();
    let ws_url = ws_url.map(String::from);

    tokio::spawn(async move {
        let mut backoff = reconnect_config.build_backoff();
        let mut attempt: u32 = 0;

        loop {
            // Check for shutdown before attempting connection
            if shutdown_rx.try_recv().is_ok() {
                info!("Reconnecting stream shutdown requested");
                break;
            }

            attempt = attempt.saturating_add(1);
            info!(
                attempt = attempt,
                symbol = %symbol,
                "Attempting connection"
            );

            // Try to connect
            let connect_result = connect_trade_stream(
                &symbol,
                ws_url.as_deref(),
                buffer_size,
            ).await;

            match connect_result {
                Ok(mut handle) => {
                    // Reset backoff on successful connection
                    backoff.reset();
                    attempt = 0;
                    info!(symbol = %symbol, "Connected successfully");

                    // Forward trades until disconnection
                    loop {
                        tokio::select! {
                            _ = shutdown_rx.recv() => {
                                info!("Shutdown during active connection");
                                handle.shutdown().await;
                                return;
                            }
                            trade = handle.trades.recv() => {
                                match trade {
                                    Some(t) => {
                                        if trades_tx.send(t).await.is_err() {
                                            info!("Trade receiver dropped");
                                            return;
                                        }
                                    }
                                    None => {
                                        // Inner stream ended, will reconnect
                                        warn!(symbol = %symbol, "Trade stream disconnected");
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    // Check if error is permanent (shouldn't retry)
                    if is_permanent_error(&e) {
                        error!(
                            error = %e,
                            "Permanent error, not retrying"
                        );
                        break;
                    }

                    warn!(
                        attempt = attempt,
                        error = %e,
                        "Connection failed"
                    );
                }
            }

            // Calculate backoff delay
            match backoff.next_backoff() {
                Some(delay) => {
                    info!(
                        delay_ms = delay.as_millis(),
                        "Waiting before reconnect"
                    );

                    // Wait with shutdown check
                    tokio::select! {
                        _ = shutdown_rx.recv() => {
                            info!("Shutdown during backoff");
                            return;
                        }
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
                None => {
                    error!("Max retry time exceeded, giving up");
                    break;
                }
            }
        }

        info!("Reconnecting stream task exiting");
    });

    ReconnectingTradeStream {
        trades: trades_rx,
        shutdown_tx,
    }
}

/// Determine if an error is permanent (should not retry)
fn is_permanent_error(error: &ExchangeError) -> bool {
    match error {
        // URL errors are permanent - bad config
        ExchangeError::InvalidUrl { .. } => true,
        // Parse errors might be temporary (bad message)
        ExchangeError::ParseError(_) => false,
        // Connection errors are usually transient
        ExchangeError::ConnectionFailed(_) => false,
        // Subscription errors depend on the reason
        ExchangeError::SubscriptionFailed { reason, .. } => {
            // "Invalid symbol" is permanent, network issues are not
            reason.contains("Invalid") || reason.contains("invalid")
        }
        // Closed connections are transient (server-side disconnect)
        ExchangeError::ConnectionClosed { .. } => false,
        // Send failures are usually transient
        ExchangeError::SendFailed(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ReconnectConfig::default();
        assert_eq!(config.initial_interval, Duration::from_millis(100));
        assert_eq!(config.max_interval, Duration::from_secs(60));
        assert_eq!(config.max_elapsed_time, Some(Duration::from_secs(300)));
    }

    #[test]
    fn test_unlimited_config() {
        let config = ReconnectConfig::unlimited();
        assert!(config.max_elapsed_time.is_none());
    }

    #[test]
    fn test_permanent_error_detection() {
        // URL error is permanent
        assert!(is_permanent_error(&ExchangeError::InvalidUrl {
            url: "bad".to_string(),
            source: url::Url::parse("not-a-url").unwrap_err(),
        }));

        // Connection closed is transient
        assert!(!is_permanent_error(&ExchangeError::ConnectionClosed {
            reason: "Server disconnect".to_string(),
        }));

        // Invalid symbol subscription is permanent
        assert!(is_permanent_error(&ExchangeError::SubscriptionFailed {
            stream: "xxx@trade".to_string(),
            reason: "Invalid symbol".to_string(),
        }));

        // Network subscription failure is transient
        assert!(!is_permanent_error(&ExchangeError::SubscriptionFailed {
            stream: "btcusdt@trade".to_string(),
            reason: "Network timeout".to_string(),
        }));
    }

    #[test]
    fn test_backoff_builds() {
        let config = ReconnectConfig::default();
        let mut backoff = config.build_backoff();

        // First backoff should be around initial_interval (with jitter)
        let first = backoff.next_backoff().unwrap();
        assert!(first >= Duration::from_millis(50)); // Allow for jitter
        assert!(first <= Duration::from_millis(200));
    }
}
