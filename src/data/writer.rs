//! Async database writer actor with buffered batch writes
//!
//! Accumulates ticks and flushes to database when:
//! - Buffer reaches 1000 ticks, OR
//! - 1 second has elapsed since last flush
//!
//! This decouples tick processing latency from database write latency.

use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, Interval};
use tracing::{debug, error, info};

use crate::data::repository::batch_insert_ticks;
use crate::data::tick::NewTick;

/// Configuration for the database writer
#[derive(Debug, Clone)]
pub struct WriterConfig {
    /// Max ticks to buffer before flush (default: 1000)
    pub buffer_size: usize,
    /// Max time between flushes (default: 1 second)
    pub flush_interval: Duration,
    /// Channel capacity for incoming ticks (default: 10000)
    pub channel_capacity: usize,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            buffer_size: 1000,
            flush_interval: Duration::from_secs(1),
            channel_capacity: 10000,
        }
    }
}

/// Messages the writer actor can receive
enum WriterMessage {
    WriteTick(NewTick),
    Flush,
    Shutdown,
}

/// Handle to send ticks to the database writer
#[derive(Clone)]
pub struct DbWriterHandle {
    sender: mpsc::Sender<WriterMessage>,
}

impl DbWriterHandle {
    /// Send a tick for database storage
    ///
    /// Uses try_send to avoid blocking - if buffer is full, tick is dropped.
    /// This ensures the strategy processing path never blocks on database.
    pub fn write_tick(&self, tick: NewTick) -> Result<(), mpsc::error::TrySendError<()>> {
        self.sender
            .try_send(WriterMessage::WriteTick(tick))
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => mpsc::error::TrySendError::Full(()),
                mpsc::error::TrySendError::Closed(_) => mpsc::error::TrySendError::Closed(()),
            })
    }

    /// Request immediate flush of buffered ticks
    pub async fn flush(&self) -> Result<(), mpsc::error::SendError<()>> {
        self.sender
            .send(WriterMessage::Flush)
            .await
            .map_err(|_| mpsc::error::SendError(()))
    }

    /// Request graceful shutdown (flushes remaining buffer)
    pub async fn shutdown(&self) -> Result<(), mpsc::error::SendError<()>> {
        self.sender
            .send(WriterMessage::Shutdown)
            .await
            .map_err(|_| mpsc::error::SendError(()))
    }
}

/// Internal actor state
struct DbWriterActor {
    pool: PgPool,
    receiver: mpsc::Receiver<WriterMessage>,
    buffer: Vec<NewTick>,
    buffer_capacity: usize,
    flush_interval: Interval,
    ticks_written: u64,
    flushes: u64,
}

impl DbWriterActor {
    fn new(
        pool: PgPool,
        receiver: mpsc::Receiver<WriterMessage>,
        config: &WriterConfig,
    ) -> Self {
        Self {
            pool,
            receiver,
            buffer: Vec::with_capacity(config.buffer_size),
            buffer_capacity: config.buffer_size,
            flush_interval: interval(config.flush_interval),
            ticks_written: 0,
            flushes: 0,
        }
    }

    async fn run(mut self) {
        info!(
            buffer_capacity = self.buffer_capacity,
            "Database writer actor started"
        );

        loop {
            tokio::select! {
                // Bias towards processing messages over timer
                biased;

                msg = self.receiver.recv() => {
                    match msg {
                        Some(WriterMessage::WriteTick(tick)) => {
                            self.buffer.push(tick);
                            if self.buffer.len() >= self.buffer_capacity {
                                self.flush_buffer().await;
                            }
                        }
                        Some(WriterMessage::Flush) => {
                            self.flush_buffer().await;
                        }
                        Some(WriterMessage::Shutdown) | None => {
                            info!("Shutdown requested, flushing remaining buffer");
                            self.flush_buffer().await;
                            break;
                        }
                    }
                }
                _ = self.flush_interval.tick() => {
                    if !self.buffer.is_empty() {
                        debug!(buffered = self.buffer.len(), "Timer flush triggered");
                        self.flush_buffer().await;
                    }
                }
            }
        }

        info!(
            total_ticks = self.ticks_written,
            total_flushes = self.flushes,
            "Database writer actor stopped"
        );
    }

    async fn flush_buffer(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        let ticks: Vec<NewTick> = self.buffer.drain(..).collect();
        let count = ticks.len();

        match batch_insert_ticks(&self.pool, &ticks).await {
            Ok(rows) => {
                self.ticks_written += rows;
                self.flushes += 1;
                debug!(
                    batch_size = count,
                    rows_inserted = rows,
                    total_written = self.ticks_written,
                    "Flush complete"
                );
            }
            Err(e) => {
                error!(
                    error = %e,
                    batch_size = count,
                    "Failed to write ticks to database"
                );
                // Ticks are lost on error - this is acceptable for tick data
                // (better than blocking or unbounded retry)
            }
        }
    }
}

/// Start the database writer actor
///
/// Returns a handle for sending ticks. The actor runs in a background task
/// and will flush remaining ticks on shutdown.
pub fn start_db_writer(pool: PgPool, config: WriterConfig) -> DbWriterHandle {
    let (sender, receiver) = mpsc::channel(config.channel_capacity);

    let actor = DbWriterActor::new(pool, receiver, &config);
    tokio::spawn(actor.run());

    DbWriterHandle { sender }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WriterConfig::default();
        assert_eq!(config.buffer_size, 1000);
        assert_eq!(config.flush_interval, Duration::from_secs(1));
        assert_eq!(config.channel_capacity, 10000);
    }
}
