//! Binance.US WebSocket client for trade streams
//!
//! This client connects to Binance.US public WebSocket streams and
//! yields parsed trade events. It handles the connection lifecycle
//! but does NOT handle automatic reconnection (see Plan 03).

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::exchange::error::ExchangeError;
use crate::exchange::messages::{SubscriptionResponse, TradeEvent};

/// Default Binance.US WebSocket base URL
pub const BINANCE_WS_URL: &str = "wss://stream.binance.us:9443/ws";

/// Trade stream client handle
///
/// Provides access to the trade event channel and connection status.
pub struct TradeStreamHandle {
    /// Receiver for parsed trade events
    pub trades: mpsc::Receiver<TradeEvent>,
    /// Sender to signal shutdown
    shutdown_tx: mpsc::Sender<()>,
}

impl TradeStreamHandle {
    /// Signal the stream to shut down gracefully
    pub async fn shutdown(self) {
        // Just drop the sender - the task will see the channel close
        drop(self.shutdown_tx);
    }
}

/// Connect to Binance.US and subscribe to trade stream
///
/// Returns a handle with a channel receiver for trade events.
/// The connection runs in a spawned task until shutdown or error.
///
/// # Arguments
/// * `symbol` - Trading pair (e.g., "btcusdt")
/// * `ws_url` - Optional custom WebSocket URL (defaults to Binance.US)
/// * `buffer_size` - Channel buffer size (default 1000)
///
/// # Example
/// ```ignore
/// let handle = connect_trade_stream("btcusdt", None, 1000).await?;
/// while let Some(trade) = handle.trades.recv().await {
///     println!("Trade: {} @ {}", trade.quantity, trade.price);
/// }
/// ```
pub async fn connect_trade_stream(
    symbol: &str,
    ws_url: Option<&str>,
    buffer_size: usize,
) -> Result<TradeStreamHandle, ExchangeError> {
    let base_url = ws_url.unwrap_or(BINANCE_WS_URL);
    let stream_name = format!("{}@trade", symbol.to_lowercase());
    let full_url = format!("{}/{}", base_url, stream_name);

    info!(url = %full_url, symbol = %symbol, "Connecting to trade stream");

    // Validate URL format before connecting
    url::Url::parse(&full_url).map_err(|e| ExchangeError::InvalidUrl {
        url: full_url.clone(),
        source: e,
    })?;

    // Connect to WebSocket (using string - connect_async accepts &str)
    let (ws_stream, response) = connect_async(&full_url).await?;

    info!(
        status = %response.status(),
        "WebSocket connected"
    );

    // Split into read/write halves
    let (mut write, mut read) = ws_stream.split();

    // Create channels
    let (trades_tx, trades_rx) = mpsc::channel(buffer_size);
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    // Spawn task to handle WebSocket messages
    tokio::spawn(async move {
        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = shutdown_rx.recv() => {
                    info!("Trade stream shutdown requested");
                    // Send close frame
                    if let Err(e) = write.close().await {
                        warn!(error = %e, "Error sending close frame");
                    }
                    break;
                }

                // Process incoming messages
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            // Try to parse as trade event
                            match serde_json::from_str::<TradeEvent>(&text) {
                                Ok(trade) => {
                                    debug!(
                                        trade_id = trade.trade_id,
                                        price = %trade.price,
                                        quantity = %trade.quantity,
                                        "Trade received"
                                    );
                                    if trades_tx.send(trade).await.is_err() {
                                        // Receiver dropped, shut down
                                        info!("Trade receiver dropped, shutting down");
                                        break;
                                    }
                                }
                                Err(e) => {
                                    // Might be subscription response or other message
                                    if let Ok(sub_response) = serde_json::from_str::<SubscriptionResponse>(&text) {
                                        if sub_response.is_success() {
                                            info!(id = sub_response.id, "Subscription confirmed");
                                        } else {
                                            warn!(
                                                id = sub_response.id,
                                                result = ?sub_response.result,
                                                "Subscription response"
                                            );
                                        }
                                    } else {
                                        debug!(
                                            error = %e,
                                            text = %text,
                                            "Non-trade message received"
                                        );
                                    }
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            debug!("Ping received, pong auto-sent by tungstenite");
                            // Note: tungstenite automatically responds to pings
                            let _ = data; // Acknowledge we saw it
                        }
                        Some(Ok(Message::Pong(_))) => {
                            debug!("Pong received");
                        }
                        Some(Ok(Message::Close(frame))) => {
                            let reason = frame
                                .map(|f| f.reason.to_string())
                                .unwrap_or_else(|| "No reason".to_string());
                            info!(reason = %reason, "WebSocket close received");
                            break;
                        }
                        Some(Ok(Message::Binary(_))) => {
                            debug!("Binary message ignored");
                        }
                        Some(Ok(Message::Frame(_))) => {
                            // Raw frame, shouldn't happen with tungstenite
                        }
                        Some(Err(e)) => {
                            error!(error = %e, "WebSocket error");
                            break;
                        }
                        None => {
                            info!("WebSocket stream ended");
                            break;
                        }
                    }
                }
            }
        }

        info!("Trade stream task exiting");
    });

    Ok(TradeStreamHandle {
        trades: trades_rx,
        shutdown_tx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_construction() {
        let symbol = "btcusdt";
        let stream_name = format!("{}@trade", symbol.to_lowercase());
        let full_url = format!("{}/{}", BINANCE_WS_URL, stream_name);
        assert_eq!(full_url, "wss://stream.binance.us:9443/ws/btcusdt@trade");
    }
}
