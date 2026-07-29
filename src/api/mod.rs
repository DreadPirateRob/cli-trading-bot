//! Web API module for the trading bot dashboard
//!
//! Provides REST endpoints for bot status, metrics, trade history,
//! and equity curve data. Includes WebSocket support for real-time updates.
//! Designed to be consumed by the React dashboard.

pub mod handlers;
pub mod routes;
pub mod state;
pub mod types;
pub mod websocket;

pub use routes::create_router;
pub use state::AppState;
