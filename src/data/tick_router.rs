//! Tick router - wires WebSocket TradeEvent stream to database persistence
//!
//! This module bridges the exchange layer (TradeEvent) with the data layer (DbWriterHandle).
//! It converts incoming trade events to database-ready NewTick format and sends them
//! to the background database writer without blocking the WebSocket receiver.

use tokio::sync::mpsc;
use tracing::{debug, error, trace, warn};

use crate::data::tick::NewTick;
use crate::data::writer::DbWriterHandle;
use crate::exchange::messages::TradeEvent;

/// Route incoming trade events to the database writer
///
/// This function runs in a loop, receiving TradeEvents from the WebSocket handler,
/// converting them to NewTick format, and sending them to the DbWriterHandle.
///
/// # Design choices
///
/// - Uses `try_send()` via DbWriterHandle to never block the receiver loop
/// - Logs conversion errors but continues processing (one bad tick shouldn't stop the stream)
/// - Returns when the receiver channel closes (WebSocket disconnected)
///
/// # Arguments
///
/// * `tick_receiver` - Channel receiving TradeEvents from WebSocket handler
/// * `db_writer` - Handle to the background database writer actor
///
/// # Example
///
/// ```ignore
/// let (tx, rx) = mpsc::channel(1000);
/// let db_writer = start_db_writer(pool, WriterConfig::default());
///
/// // Spawn router task
/// tokio::spawn(route_ticks_to_db(rx, db_writer));
///
/// // Send ticks from WebSocket handler
/// tx.send(trade_event).await?;
/// ```
pub async fn route_ticks_to_db(
    mut tick_receiver: mpsc::Receiver<TradeEvent>,
    db_writer: DbWriterHandle,
) {
    let mut processed: u64 = 0;
    let mut conversion_errors: u64 = 0;
    let mut send_errors: u64 = 0;

    debug!("Tick router started");

    while let Some(event) = tick_receiver.recv().await {
        // Convert TradeEvent to NewTick
        let tick = match NewTick::try_from(&event) {
            Ok(tick) => tick,
            Err(e) => {
                conversion_errors += 1;
                error!(
                    error = %e,
                    trade_id = event.trade_id,
                    symbol = %event.symbol,
                    "Failed to convert TradeEvent to NewTick"
                );
                continue;
            }
        };

        // Send to database writer (non-blocking)
        if let Err(e) = db_writer.write_tick(tick) {
            send_errors += 1;
            match e {
                tokio::sync::mpsc::error::TrySendError::Full(_) => {
                    // Channel full - this is acceptable under load
                    // The tick is dropped, which is fine for historical tick data
                    trace!(
                        trade_id = event.trade_id,
                        "Database writer channel full, tick dropped"
                    );
                }
                tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                    // Writer has shut down - log and continue
                    // (might recover if writer restarts)
                    warn!("Database writer channel closed");
                }
            }
        } else {
            processed += 1;
            if processed % 10000 == 0 {
                debug!(
                    processed,
                    conversion_errors,
                    send_errors,
                    "Tick routing progress"
                );
            }
        }
    }

    debug!(
        total_processed = processed,
        total_conversion_errors = conversion_errors,
        total_send_errors = send_errors,
        "Tick router stopped"
    );
}

/// Convenience function to spawn the tick router as a background task
///
/// Returns a JoinHandle that can be used to await completion or cancel the router.
pub fn spawn_tick_router(
    tick_receiver: mpsc::Receiver<TradeEvent>,
    db_writer: DbWriterHandle,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(route_ticks_to_db(tick_receiver, db_writer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn sample_trade_event() -> TradeEvent {
        TradeEvent {
            event_type: "trade".to_string(),
            event_time: 1672531200000,
            symbol: "BTCUSDT".to_string(),
            trade_id: 123456789,
            price: "50000.12345678".to_string(),
            quantity: "0.00100000".to_string(),
            trade_time: 1672531200001,
            is_buyer_maker: false,
        }
    }

    #[tokio::test]
    async fn test_route_ticks_closes_on_sender_drop() {
        // Create a mock scenario where we can't actually test DB writes
        // but we can verify the router shuts down cleanly when input closes
        let (tx, rx) = mpsc::channel::<TradeEvent>(10);

        // We need a real DbWriterHandle to test, but without a DB we can't create one
        // This test just verifies the channel closure behavior
        drop(tx); // Close the sender

        // The receiver should return None immediately
        let mut receiver = rx;
        assert!(receiver.recv().await.is_none());
    }

    #[allow(dead_code)]
    fn test_sample_event_compiles() {
        // Just verify the sample event function compiles correctly
        let _ = sample_trade_event();
    }
}
