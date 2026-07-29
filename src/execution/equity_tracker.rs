//! Equity tracking actor for real-time paper trading equity updates
//!
//! Calculates equity every 1 second, persists to database, and broadcasts
//! via WebSocket for real-time dashboard updates.

use std::sync::Arc;

use chrono::Utc;
use rust_decimal::Decimal;
use sqlx::PgPool;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{interval, Duration, MissedTickBehavior};
use tracing::{debug, error, info, warn};

use crate::api::types::{BotUpdate, EquityUpdate};
use crate::data::insert_equity_point;
use crate::execution::types::OrderExecutor;
use crate::execution::PaperExecutor;

/// Messages to the EquityTracker actor
#[derive(Debug)]
pub enum EquityMessage {
    /// Query current equity (synchronous response)
    QueryEquity {
        respond_to: tokio::sync::oneshot::Sender<Decimal>,
    },
    /// Shutdown the actor
    Shutdown,
}

/// Handle for interacting with EquityTrackerActor
#[derive(Clone)]
pub struct EquityTrackerHandle {
    sender: mpsc::Sender<EquityMessage>,
}

impl EquityTrackerHandle {
    pub fn new(sender: mpsc::Sender<EquityMessage>) -> Self {
        Self { sender }
    }

    /// Query current equity
    pub async fn query_equity(&self) -> Result<Decimal, crate::error::Error> {
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.sender
            .send(EquityMessage::QueryEquity { respond_to: tx })
            .await
            .map_err(|_| crate::error::Error::Execution("EquityTracker channel closed".into()))?;

        rx.await
            .map_err(|_| crate::error::Error::Execution("EquityTracker response channel closed".into()))
    }

    /// Request shutdown
    pub async fn shutdown(&self) -> Result<(), crate::error::Error> {
        self.sender
            .send(EquityMessage::Shutdown)
            .await
            .map_err(|_| crate::error::Error::Execution("EquityTracker channel closed".into()))
    }
}

/// Configuration for starting the equity tracker
pub struct EquityTrackerConfig {
    /// Database pool for persistence
    pub db_pool: PgPool,
    /// Broadcast channel for WebSocket updates
    pub broadcast_tx: broadcast::Sender<BotUpdate>,
    /// Paper executor to query balances from
    pub paper_executor: Arc<PaperExecutor>,
    /// Quote currency for equity calculation (e.g., "USDT")
    pub quote_currency: String,
    /// Trading mode ("paper" or "live")
    pub mode: String,
    /// Channel capacity for messages
    pub channel_capacity: usize,
}

/// Internal actor state
struct EquityTrackerActor {
    receiver: mpsc::Receiver<EquityMessage>,
    db_pool: PgPool,
    broadcast_tx: broadcast::Sender<BotUpdate>,
    paper_executor: Arc<PaperExecutor>,
    quote_currency: String,
    mode: String,
    /// Cached equity value
    current_equity: Decimal,
}

impl EquityTrackerActor {
    fn new(receiver: mpsc::Receiver<EquityMessage>, config: EquityTrackerConfig) -> Self {
        Self {
            receiver,
            db_pool: config.db_pool,
            broadcast_tx: config.broadcast_tx,
            paper_executor: config.paper_executor,
            quote_currency: config.quote_currency,
            mode: config.mode,
            current_equity: Decimal::ZERO,
        }
    }

    async fn run(mut self) {
        info!(mode = %self.mode, "EquityTracker actor started");

        // Create 1-second interval
        let mut tick_interval = interval(Duration::from_secs(1));
        tick_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;

                // Handle messages with priority
                Some(msg) = self.receiver.recv() => {
                    match msg {
                        EquityMessage::Shutdown => {
                            info!(mode = %self.mode, "EquityTracker shutdown requested");
                            break;
                        }
                        EquityMessage::QueryEquity { respond_to } => {
                            let _ = respond_to.send(self.current_equity);
                        }
                    }
                }

                // 1-second tick for equity calculation
                _ = tick_interval.tick() => {
                    self.tick().await;
                }
            }
        }

        info!(
            mode = %self.mode,
            final_equity = %self.current_equity,
            "EquityTracker actor stopped"
        );
    }

    /// Calculate equity, persist, and broadcast
    async fn tick(&mut self) {
        // Calculate current equity from paper executor balance
        let equity = self.calculate_equity().await;
        self.current_equity = equity;

        // Persist to database
        if let Err(e) = self.persist_equity(equity).await {
            error!(error = %e, "Failed to persist equity");
        }

        // Broadcast via WebSocket
        self.broadcast_equity(equity);

        debug!(
            mode = %self.mode,
            equity = %equity,
            "Equity tick"
        );
    }

    /// Calculate total equity from paper executor balances
    async fn calculate_equity(&self) -> Decimal {
        // For paper mode, equity is the quote currency balance
        // In a more complete implementation, we'd include:
        // - Quote currency balance
        // - Mark-to-market value of open positions
        // For MVP, just use quote currency balance
        match self.paper_executor.get_balance(&self.quote_currency).await {
            Ok(balance) => balance,
            Err(e) => {
                warn!(error = %e, "Failed to get balance for equity calculation");
                self.current_equity // Fall back to cached value
            }
        }
    }

    /// Persist equity point to database
    async fn persist_equity(&self, equity: Decimal) -> Result<(), sqlx::Error> {
        insert_equity_point(&self.db_pool, equity, &self.mode).await?;
        Ok(())
    }

    /// Broadcast equity update via WebSocket
    fn broadcast_equity(&self, equity: Decimal) {
        let update = BotUpdate::Equity(EquityUpdate {
            equity,
            timestamp: Utc::now(),
            mode: self.mode.clone(),
        });

        // Ignore send errors (no subscribers is OK)
        let _ = self.broadcast_tx.send(update);
    }
}

/// Start the EquityTracker actor
///
/// Returns a handle for sending messages. The actor runs in a background task.
pub fn start_equity_tracker(config: EquityTrackerConfig) -> EquityTrackerHandle {
    let capacity = config.channel_capacity;
    let (sender, receiver) = mpsc::channel(capacity);

    let actor = EquityTrackerActor::new(receiver, config);
    tokio::spawn(actor.run());

    EquityTrackerHandle::new(sender)
}
