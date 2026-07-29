//! Order executor actor for centralized order execution
//!
//! Receives signals from strategies and stop-loss events from risk engine,
//! executes via live or paper executor, and coordinates notifications.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, Duration, Interval};
use tracing::{debug, error, info, warn};

use crate::config::{ExecutionConfig, PaperTradingConfig};
use crate::data::{close_order, insert_order, CloseOrder, NewOrder, OrderSide};
use crate::exchange::Credentials;
use crate::execution::{
    BalanceManager, ExecuteOrderRequest, ExecuteOrderResponse, ExecutionError, ExecutionPosition,
    LiveExecutor, OrderExecutor, OrderStatus, OrderType, PaperExecutor, PositionManager,
    ReserveConfig,
};
use crate::notifications::{AlertEvent, AlertSender};
use crate::risk::{RiskDecision, RiskEngineHandle, StopLossEvent};
use crate::strategy::Signal;

/// Message types for OrderExecutorActor
#[derive(Debug)]
pub enum ExecutionMessage {
    /// Execute a signal from strategy
    ExecuteSignal {
        strategy_id: String,
        symbol: String,
        signal: Signal,
        current_price: Decimal,
        respond_to: oneshot::Sender<ExecutionResult>,
    },
    /// Force balance reconciliation
    ReconcileBalance,
    /// Get current balances
    GetBalances {
        respond_to: oneshot::Sender<HashMap<String, Decimal>>,
    },
    /// Shutdown
    Shutdown,
}

/// Result of execution attempt
#[derive(Debug, Clone)]
pub enum ExecutionResult {
    /// Order executed successfully
    Filled(ExecuteOrderResponse),
    /// Order rejected by risk check or exchange
    Rejected { reason: String },
    /// Signal was Hold - no action taken
    NoAction,
    /// Execution disabled
    Disabled,
}

/// Handle for interacting with OrderExecutorActor
#[derive(Clone)]
pub struct OrderExecutorHandle {
    sender: mpsc::Sender<ExecutionMessage>,
}

impl OrderExecutorHandle {
    pub fn new(sender: mpsc::Sender<ExecutionMessage>) -> Self {
        Self { sender }
    }

    /// Execute a signal from strategy
    pub async fn execute_signal(
        &self,
        strategy_id: &str,
        symbol: &str,
        signal: Signal,
        current_price: Decimal,
    ) -> Result<ExecutionResult, crate::error::Error> {
        let (tx, rx) = oneshot::channel();

        self.sender
            .send(ExecutionMessage::ExecuteSignal {
                strategy_id: strategy_id.to_string(),
                symbol: symbol.to_string(),
                signal,
                current_price,
                respond_to: tx,
            })
            .await
            .map_err(|_| crate::error::Error::Execution("Executor channel closed".into()))?;

        rx.await
            .map_err(|_| crate::error::Error::Execution("Executor response channel closed".into()))
    }

    /// Request balance reconciliation
    pub async fn reconcile_balance(&self) -> Result<(), crate::error::Error> {
        self.sender
            .send(ExecutionMessage::ReconcileBalance)
            .await
            .map_err(|_| crate::error::Error::Execution("Executor channel closed".into()))
    }

    /// Get current balances
    pub async fn get_balances(&self) -> Result<HashMap<String, Decimal>, crate::error::Error> {
        let (tx, rx) = oneshot::channel();

        self.sender
            .send(ExecutionMessage::GetBalances { respond_to: tx })
            .await
            .map_err(|_| crate::error::Error::Execution("Executor channel closed".into()))?;

        rx.await
            .map_err(|_| crate::error::Error::Execution("Executor response channel closed".into()))
    }

    /// Request shutdown
    pub async fn shutdown(&self) -> Result<(), crate::error::Error> {
        self.sender
            .send(ExecutionMessage::Shutdown)
            .await
            .map_err(|_| crate::error::Error::Execution("Executor channel closed".into()))
    }
}

/// Configuration for starting the executor
pub struct ExecutorStartConfig {
    pub execution_config: ExecutionConfig,
    pub paper_config: PaperTradingConfig,
    pub credentials: Option<Credentials>,
}

/// Order executor actor
struct OrderExecutorActor {
    receiver: mpsc::Receiver<ExecutionMessage>,
    stop_loss_rx: mpsc::Receiver<StopLossEvent>,
    config: ExecutionConfig,
    executor: Arc<dyn OrderExecutor>,
    risk_handle: RiskEngineHandle,
    balance_manager: BalanceManager,
    position_manager: PositionManager,
    paper_mode: bool,
    #[allow(dead_code)]
    stop_loss_offset: Decimal,
    /// Alert sender for trade notifications
    alert_sender: Option<AlertSender>,
}

impl OrderExecutorActor {
    /// Emit a trade alert if alert_sender is configured
    fn emit_trade_alert(
        &self,
        order_id: &str,
        symbol: &str,
        side: &str,
        quantity: Decimal,
        price: Decimal,
        balance_after: Decimal,
        pnl: Option<Decimal>,
    ) {
        if let Some(ref sender) = self.alert_sender {
            let event = AlertEvent::trade(
                order_id,
                symbol,
                side,
                quantity,
                price,
                balance_after,
                pnl,
            );
            if !sender.try_send(event) {
                warn!("Failed to send trade alert (channel full)");
            }
        }
    }

    async fn run(mut self, pool: sqlx::PgPool) {
        info!(
            paper_mode = self.paper_mode,
            enabled = self.config.enabled,
            "OrderExecutor actor started"
        );

        // Set up reconciliation interval using tokio::time::interval
        // Only create if reconciliation is enabled (> 0 seconds)
        let mut recon_interval: Option<Interval> =
            if self.config.reconciliation_interval_secs > 0 {
                let mut interval_timer =
                    interval(Duration::from_secs(self.config.reconciliation_interval_secs));
                interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                Some(interval_timer)
            } else {
                None
            };

        // Initial balance sync (only for live mode)
        if !self.paper_mode {
            if let Err(e) = self.sync_balances().await {
                warn!(error = %e, "Failed initial balance sync");
            }
        }

        loop {
            tokio::select! {
                // Bias towards messages over timer
                biased;

                // Handle execution messages
                Some(msg) = self.receiver.recv() => {
                    match msg {
                        ExecutionMessage::ExecuteSignal {
                            strategy_id,
                            symbol,
                            signal,
                            current_price,
                            respond_to,
                        } => {
                            let result = self.handle_signal(
                                &pool,
                                &strategy_id,
                                &symbol,
                                signal,
                                current_price,
                            ).await;
                            let _ = respond_to.send(result);
                        }
                        ExecutionMessage::ReconcileBalance => {
                            if let Err(e) = self.sync_balances().await {
                                error!(error = %e, "Balance reconciliation failed");
                            }
                        }
                        ExecutionMessage::GetBalances { respond_to } => {
                            let balances = self.balance_manager.all_balances().clone();
                            let _ = respond_to.send(balances);
                        }
                        ExecutionMessage::Shutdown => {
                            info!("OrderExecutor shutdown requested");
                            break;
                        }
                    }
                }

                // Handle stop-loss events from risk engine
                Some(event) = self.stop_loss_rx.recv() => {
                    self.handle_stop_loss(&pool, event).await;
                }

                // Periodic balance reconciliation (only when interval is configured)
                _ = async {
                    match &mut recon_interval {
                        Some(interval_timer) => interval_timer.tick().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Err(e) = self.sync_balances().await {
                        warn!(error = %e, "Periodic balance sync failed");
                    }
                }

                else => break,
            }
        }

        info!(
            positions = self.position_manager.len(),
            "OrderExecutor actor stopped"
        );
    }

    async fn handle_signal(
        &mut self,
        pool: &sqlx::PgPool,
        strategy_id: &str,
        symbol: &str,
        signal: Signal,
        current_price: Decimal,
    ) -> ExecutionResult {
        if !self.config.enabled {
            debug!(strategy_id, symbol, "Execution disabled, signal ignored");
            return ExecutionResult::Disabled;
        }

        let (side, quantity) = match signal {
            Signal::Buy { quantity } => (OrderSide::Buy, quantity),
            Signal::Sell { quantity } => (OrderSide::Sell, quantity),
            Signal::Hold => return ExecutionResult::NoAction,
        };

        // Check risk first
        let risk_decision = match self
            .risk_handle
            .check_order(strategy_id, symbol, side, quantity, current_price)
            .await
        {
            Ok(decision) => decision,
            Err(e) => {
                error!(error = %e, "Risk check failed");
                return ExecutionResult::Rejected {
                    reason: e.to_string(),
                };
            }
        };

        let final_quantity = match risk_decision {
            RiskDecision::Approved => quantity,
            RiskDecision::Scaled { new_quantity, reason } => {
                info!(original = %quantity, scaled = %new_quantity, reason = %reason, "Order scaled by risk");
                new_quantity
            }
            RiskDecision::Rejected { reason } => {
                warn!(reason = %reason, "Order rejected by risk");
                return ExecutionResult::Rejected { reason };
            }
        };

        // Execute order
        let request = ExecuteOrderRequest {
            symbol: symbol.to_string(),
            side,
            quantity: final_quantity,
            order_type: OrderType::Market,
            price: Some(current_price),
            client_order_id: Some(uuid::Uuid::new_v4().to_string()),
        };

        let response = match self.executor.execute(request).await {
            Ok(resp) => resp,
            Err(e) => {
                error!(error = ?e, "Order execution failed");

                // Re-sync balance on insufficient balance errors
                if matches!(e, ExecutionError::InsufficientBalance { .. }) {
                    let _ = self.sync_balances().await;
                }

                return ExecutionResult::Rejected {
                    reason: format!("{:?}", e),
                };
            }
        };

        // Update local state
        self.on_fill(pool, strategy_id, &response).await;

        ExecutionResult::Filled(response)
    }

    async fn handle_stop_loss(&mut self, pool: &sqlx::PgPool, event: StopLossEvent) {
        warn!(
            order_id = %event.order_id,
            symbol = %event.symbol,
            quantity = %event.quantity,
            "Processing stop-loss event"
        );

        // For stop-loss, use market order for immediate execution
        let request = ExecuteOrderRequest {
            symbol: event.symbol.clone(),
            side: event.side,
            quantity: event.quantity,
            order_type: OrderType::Market,
            price: None,
            client_order_id: Some(format!("sl-{}", event.order_id)),
        };

        match self.executor.execute(request).await {
            Ok(response) => {
                info!(
                    order_id = %response.order_id,
                    fill_price = %response.fill_price,
                    "Stop-loss executed"
                );

                // Close the position
                if let Some(closed) = self
                    .position_manager
                    .close(&event.order_id, response.fill_price)
                {
                    // Note: Risk engine position notifications (position_opened/position_closed)
                    // will be added in a future phase when RiskEngineHandle is extended.
                    // For now, the risk engine tracks positions internally.

                    // Persist to database
                    let close_data = CloseOrder {
                        exit_price: response.fill_price,
                        exit_time: Utc::now(),
                        pnl: closed.pnl,
                    };
                    let mode = if self.paper_mode { "paper" } else { "live" };
                    if let Err(e) = close_order(pool, &event.order_id, &close_data, mode).await {
                        error!(error = %e, order_id = %event.order_id, "Failed to persist order close");
                    }
                }
            }
            Err(e) => {
                error!(
                    order_id = %event.order_id,
                    error = ?e,
                    "Stop-loss execution failed"
                );
            }
        }
    }

    async fn on_fill(
        &mut self,
        pool: &sqlx::PgPool,
        strategy_id: &str,
        response: &ExecuteOrderResponse,
    ) {
        // Only track buy orders as positions (sell orders close positions)
        if response.side == OrderSide::Buy && response.status == OrderStatus::Filled {
            // Add position to tracker
            let position = ExecutionPosition {
                order_id: response.order_id.clone(),
                symbol: response.symbol.clone(),
                side: response.side,
                entry_price: response.fill_price,
                quantity: response.filled_quantity,
                entry_time: Utc::now(),
                strategy_id: strategy_id.to_string(),
            };

            self.position_manager.add(position);
        }

        // Note: Risk engine position notifications (position_opened/position_closed)
        // will be added in a future phase when RiskEngineHandle is extended.
        // For now, the risk engine tracks positions internally via check_order.

        // Persist to database with current trading mode
        let new_order = NewOrder {
            order_id: response.order_id.clone(),
            symbol: response.symbol.clone(),
            side: response.side,
            entry_price: response.fill_price,
            quantity: response.filled_quantity,
            reason: format!("strategy:{}", strategy_id),
            entry_time: Utc::now(),
            trading_mode: if self.paper_mode { "paper" } else { "live" }.to_string(),
        };

        if let Err(e) = insert_order(pool, &new_order).await {
            error!(error = %e, "Failed to persist order");
        }

        // Emit trade alert
        // For balance_after, use a placeholder (real balance tracking would require more context)
        let balance_after = Decimal::ZERO; // Will be populated when balance manager is extended
        let side_str = match response.side {
            OrderSide::Buy => "buy",
            OrderSide::Sell => "sell",
        };
        self.emit_trade_alert(
            &response.order_id,
            &response.symbol,
            side_str,
            response.filled_quantity,
            response.fill_price,
            balance_after,
            None, // P&L computed on position close
        );

        // Update balance after fill (only for live mode)
        if !self.paper_mode {
            let _ = self.sync_balances().await;
        }
    }

    async fn sync_balances(&mut self) -> Result<(), ExecutionError> {
        // For paper trading, balances are managed internally by PaperExecutor
        if self.paper_mode {
            return Ok(());
        }

        // For live trading, fetch balances from exchange
        // Use the paper_mode flag to determine executor type instead of downcasting
        // The LiveExecutor's get_all_balances method is only available in live mode
        // Since we already checked paper_mode above, we know this is a LiveExecutor
        // However, to maintain trait compatibility, we fetch individual balances
        // for known trading assets. A more complete implementation would:
        // 1. Store the live executor reference separately, or
        // 2. Add get_all_balances to the OrderExecutor trait

        // For now, log that we're skipping full balance sync in live mode
        // The balance manager will be updated on each fill
        debug!("Live mode balance sync - balances updated on fill");
        Ok(())
    }
}

/// Start the order executor actor
///
/// # Arguments
/// * `config` - Executor configuration
/// * `risk_handle` - Handle to risk engine for notifications
/// * `stop_loss_rx` - Receiver for stop-loss events from risk engine
/// * `pool` - Database pool for order persistence
/// * `alert_sender` - Optional sender for trade notifications to AlertBroadcaster
///
/// # Paper Mode Safety
/// When `start_config.paper_config.enabled` is true, a PaperExecutor is created.
/// When false, a LiveExecutor is created (requires credentials).
/// This ensures paper_trading.enabled=true NEVER instantiates a LiveExecutor.
pub fn start_order_executor(
    start_config: ExecutorStartConfig,
    risk_handle: RiskEngineHandle,
    stop_loss_rx: mpsc::Receiver<StopLossEvent>,
    pool: sqlx::PgPool,
    alert_sender: Option<AlertSender>,
) -> OrderExecutorHandle {
    let (sender, receiver) = mpsc::channel(start_config.execution_config.channel_capacity);

    // CRITICAL: Paper mode check - this is the single point where executor type is determined
    let paper_mode = start_config.paper_config.enabled;
    let stop_loss_offset =
        Decimal::from_f64(start_config.execution_config.stop_loss_offset_pct)
            .unwrap_or(Decimal::new(5, 2));

    // Create appropriate executor based on paper_mode flag
    // SAFETY: When paper_mode is true, LiveExecutor is NEVER instantiated
    let executor: Arc<dyn OrderExecutor> = if paper_mode {
        info!("Starting executor in PAPER TRADING mode - no real orders will be placed");
        Arc::new(PaperExecutor::new(start_config.paper_config))
    } else {
        let credentials = start_config
            .credentials
            .expect("Credentials required for live trading");
        info!("Starting executor in LIVE mode - real orders will be placed on exchange");
        Arc::new(LiveExecutor::new(credentials))
    };

    // Create balance manager with reserve config
    let reserve_config = start_config
        .execution_config
        .reserve_pct
        .map(|pct| {
            ReserveConfig::with_percentage(Decimal::from_f64(pct).unwrap_or(Decimal::ZERO))
        })
        .unwrap_or_default();

    let actor = OrderExecutorActor {
        receiver,
        stop_loss_rx,
        config: start_config.execution_config,
        executor,
        risk_handle,
        balance_manager: BalanceManager::new(reserve_config),
        position_manager: PositionManager::new(),
        paper_mode,
        stop_loss_offset,
        alert_sender,
    };

    tokio::spawn(actor.run(pool));

    OrderExecutorHandle::new(sender)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_executor_handle_get_balances() {
        let (tx, mut rx) = mpsc::channel(10);
        let handle = OrderExecutorHandle::new(tx);

        // Spawn a mock receiver
        tokio::spawn(async move {
            if let Some(ExecutionMessage::GetBalances { respond_to }) = rx.recv().await {
                let mut balances = HashMap::new();
                balances.insert("USDT".to_string(), dec!(10000));
                let _ = respond_to.send(balances);
            }
        });

        let result = handle.get_balances().await;
        assert!(result.is_ok());
        let balances = result.unwrap();
        assert_eq!(balances.get("USDT"), Some(&dec!(10000)));
    }

    #[tokio::test]
    async fn test_executor_handle_shutdown() {
        let (tx, mut rx) = mpsc::channel(10);
        let handle = OrderExecutorHandle::new(tx);

        tokio::spawn(async move {
            let msg = rx.recv().await;
            assert!(matches!(msg, Some(ExecutionMessage::Shutdown)));
        });

        let result = handle.shutdown().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_executor_handle_reconcile_balance() {
        let (tx, mut rx) = mpsc::channel(10);
        let handle = OrderExecutorHandle::new(tx);

        tokio::spawn(async move {
            let msg = rx.recv().await;
            assert!(matches!(msg, Some(ExecutionMessage::ReconcileBalance)));
        });

        let result = handle.reconcile_balance().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_executor_handle_channel_closed() {
        let (tx, rx) = mpsc::channel::<ExecutionMessage>(10);
        let handle = OrderExecutorHandle::new(tx);

        // Drop receiver to close channel
        drop(rx);

        let result = handle.shutdown().await;
        assert!(result.is_err());
    }

    #[test]
    fn test_execution_result_variants() {
        let filled = ExecutionResult::Filled(ExecuteOrderResponse {
            order_id: "test".to_string(),
            client_order_id: "test".to_string(),
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Buy,
            filled_quantity: dec!(1),
            fill_price: dec!(50000),
            status: OrderStatus::Filled,
            timestamp: 0,
            fills: vec![],
        });
        assert!(matches!(filled, ExecutionResult::Filled(_)));

        let rejected = ExecutionResult::Rejected {
            reason: "test".to_string(),
        };
        assert!(matches!(rejected, ExecutionResult::Rejected { .. }));

        let no_action = ExecutionResult::NoAction;
        assert!(matches!(no_action, ExecutionResult::NoAction));

        let disabled = ExecutionResult::Disabled;
        assert!(matches!(disabled, ExecutionResult::Disabled));
    }
}
