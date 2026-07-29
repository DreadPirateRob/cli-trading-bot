//! Router configuration with REST and WebSocket endpoints
//!
//! All API routes are nested under /api prefix.
//! WebSocket endpoint at /api/ws for real-time updates.

use std::sync::Arc;

use axum::{routing::{get, post}, Router};
use http::Method;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use super::handlers;
use super::state::AppState;
use super::websocket::ws_handler;

/// Create the API router with all endpoints configured
pub fn create_router(state: Arc<AppState>) -> Router {
    // API routes under /api prefix
    let api_routes = Router::new()
        // Read-only endpoints
        .route("/status", get(handlers::get_status))
        .route("/metrics", get(handlers::get_metrics))
        .route("/positions", get(handlers::get_positions))
        .route("/trades", get(handlers::get_trades))
        .route("/equity-curve", get(handlers::get_equity_curve))
        .route("/indicators", get(handlers::get_indicators))
        .route("/strategies", get(handlers::get_strategies))
        .route("/ticks", get(handlers::get_recent_ticks))
        // Risk endpoints
        .route("/risk/status", get(handlers::get_risk_status))
        .route("/risk/config", get(handlers::get_risk_config))
        // Mode-specific risk config endpoints
        .route("/paper/risk/config", get(handlers::get_paper_risk_config).put(handlers::update_paper_risk_config))
        .route("/live/risk/config", get(handlers::get_live_risk_config).put(handlers::update_live_risk_config))
        // Paper account reset endpoint
        .route("/paper/reset", post(handlers::reset_paper_account))
        // Order endpoints (manual trading)
        .route("/orders/buy", post(handlers::submit_buy_order))
        .route("/orders/sell", post(handlers::submit_sell_order))
        .route("/orders", get(handlers::get_order_history))
        // Control endpoints
        .route("/start", post(handlers::start_bot))
        .route("/stop", post(handlers::stop_bot))
        .route("/strategy", post(handlers::switch_strategy))
        .route("/strategy/config", post(handlers::update_strategy_config))
        .route("/optimize", post(handlers::trigger_optimization))
        .route("/backtest", post(handlers::run_backtest))
        .route("/close-all", post(handlers::close_all_positions))
        // WebSocket endpoint
        .route("/ws", get(ws_handler))
        // Notification endpoints
        .route("/notifications/config", get(handlers::get_notification_config).put(handlers::update_notification_config))
        .route("/notifications/status", get(handlers::get_notification_status))
        // Mode endpoints
        .route("/mode", get(handlers::get_trading_mode).post(handlers::switch_trading_mode));

    // Main router with middleware
    Router::new()
        .nest("/api", api_routes)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::PUT])
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
