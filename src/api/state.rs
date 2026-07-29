//! Shared application state for API handlers
//!
//! Uses Arc for thread-safe sharing across handlers.
//! Includes broadcast channel for WebSocket real-time updates.

use std::sync::Arc;

use arc_swap::ArcSwap;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use sqlx::PgPool;
use tokio::sync::broadcast;

use crate::config::AppConfig;
use crate::execution::EquityTrackerHandle;
use crate::notifications::AlertEvent;
use crate::risk::config::{GlobalRiskConfig, ModeRiskConfigs};
use crate::risk::RiskEngineHandle;

use super::types::BotUpdate;

/// Shared application state accessible to all API handlers
#[derive(Clone)]
pub struct AppState {
    /// Database connection pool
    pub db_pool: PgPool,
    /// Hot-reloadable application configuration
    pub config: Arc<ArcSwap<AppConfig>>,
    /// Unix timestamp when the bot started (for uptime calculation)
    pub start_time: std::time::Instant,
    /// Broadcast channel for WebSocket updates
    pub bot_updates: broadcast::Sender<BotUpdate>,
    /// Optional handle to RiskEngine (None if risk engine not started)
    pub risk_handle: Option<RiskEngineHandle>,
    /// Mode-specific risk configurations (paper/live)
    pub mode_risk_configs: ModeRiskConfigs,
    /// Optional equity tracker handle (for paper mode)
    pub equity_tracker: Option<EquityTrackerHandle>,
}

impl AppState {
    /// Create new application state
    ///
    /// # Arguments
    ///
    /// * `db_pool` - Database connection pool
    /// * `config` - Hot-reloadable configuration wrapped in ArcSwap
    /// * `risk_handle` - Optional handle to RiskEngine (None if not running)
    /// * `equity_tracker` - Optional handle to EquityTracker (None if not running)
    pub fn new(
        db_pool: PgPool,
        config: Arc<ArcSwap<AppConfig>>,
        risk_handle: Option<RiskEngineHandle>,
        equity_tracker: Option<EquityTrackerHandle>,
    ) -> Self {
        // Create broadcast channel with 100-message buffer
        // Slow receivers will drop oldest messages (lagged)
        let (bot_updates, _) = broadcast::channel(100);

        // Initialize mode-specific risk configs from global config
        let app_config = config.load();
        let default_risk = GlobalRiskConfig {
            max_position_size_pct: Decimal::from_f64(app_config.strategy.risk.max_position_size_pct)
                .unwrap_or_else(|| Decimal::new(10, 2)),
            stop_loss_pct: Decimal::from_f64(app_config.strategy.risk.stop_loss_pct)
                .unwrap_or_else(|| Decimal::new(2, 2)),
            daily_loss_limit_pct: Decimal::from_f64(app_config.strategy.risk.daily_loss_limit_pct)
                .unwrap_or_else(|| Decimal::new(5, 2)),
            max_drawdown_pct: Decimal::from_f64(app_config.strategy.risk.max_drawdown_pct)
                .unwrap_or_else(|| Decimal::new(15, 2)),
        };
        let mode_risk_configs = ModeRiskConfigs::new(default_risk);

        Self {
            db_pool,
            config,
            start_time: std::time::Instant::now(),
            bot_updates,
            risk_handle,
            mode_risk_configs,
            equity_tracker,
        }
    }

    /// Get the RiskEngineHandle if available
    pub fn get_risk_handle(&self) -> Option<&RiskEngineHandle> {
        self.risk_handle.as_ref()
    }

    /// Get the current configuration
    pub fn get_config(&self) -> arc_swap::Guard<Arc<AppConfig>> {
        self.config.load()
    }

    /// Get uptime in seconds
    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Broadcast an update to all connected WebSocket clients
    ///
    /// Ignores send errors (no subscribers is OK).
    pub fn broadcast(&self, update: BotUpdate) {
        let _ = self.bot_updates.send(update);
    }

    /// Forward an alert event to WebSocket clients
    ///
    /// Converts AlertEvent to BotUpdate::Alert and broadcasts to all connected clients.
    /// This ensures all alert types (trade, risk, system) are visible in the dashboard.
    pub fn forward_alert_to_websocket(&self, event: AlertEvent) {
        self.broadcast(BotUpdate::Alert(event));
    }
}
