//! API request and response types
//!
//! All types derive Serialize for JSON output to the dashboard.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::notifications::AlertEvent;

/// Bot status response - current operational state
#[derive(Debug, Clone, Serialize)]
pub struct BotStatusResponse {
    /// Current bot status: "running", "stopped", "error"
    pub status: String,
    /// Name of the currently active trading strategy
    pub active_strategy: String,
    /// How long the bot has been running in seconds
    pub uptime_seconds: u64,
    /// Whether the bot is in paper trading mode (no real orders)
    pub paper_mode: bool,
}

/// Metrics response - key performance indicators
#[derive(Debug, Clone, Serialize)]
pub struct MetricsResponse {
    /// Current total equity in quote currency
    pub equity: Decimal,
    /// Profit/Loss for today in quote currency
    pub daily_pnl: Decimal,
    /// Win rate as percentage (0.0-100.0), None if no trades
    pub win_rate: Option<f64>,
    /// Sharpe ratio, None if insufficient data
    pub sharpe_ratio: Option<f64>,
    /// Maximum drawdown as percentage, None if no drawdown recorded
    pub max_drawdown: Option<f64>,
}

/// Trade response - individual trade record
#[derive(Debug, Clone, Serialize)]
pub struct TradeResponse {
    /// Unique trade identifier
    pub id: String,
    /// Trading pair symbol (e.g., "BTCUSDT")
    pub symbol: String,
    /// Order side: "buy" or "sell"
    pub side: String,
    /// Trade quantity in base currency
    pub quantity: Decimal,
    /// Entry price
    pub price: Decimal,
    /// Trade timestamp
    pub timestamp: DateTime<Utc>,
    /// Realized P&L if trade is closed, None if open
    pub pnl: Option<Decimal>,
}

/// Equity curve point - single point on equity chart
#[derive(Debug, Clone, Serialize)]
pub struct EquityPointResponse {
    /// Timestamp of this equity value
    pub timestamp: DateTime<Utc>,
    /// Equity value at this point
    pub value: Decimal,
}

/// Query parameters for trades endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct TradesQueryParams {
    /// Maximum number of trades to return (default: 50)
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// Number of trades to skip for pagination (default: 0)
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    50
}

/// Query parameters for equity curve endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct EquityCurveParams {
    /// Number of days of history to return (default: 30)
    #[serde(default = "default_days")]
    pub days: u32,
}

fn default_days() -> u32 {
    30
}

// ============================================================================
// WebSocket message types for real-time dashboard updates
// ============================================================================

/// WebSocket message for real-time bot updates
///
/// All dashboard updates are broadcast through this enum.
/// Uses JSON serialization with type/data structure for easy client parsing.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum BotUpdate {
    /// Bot operational status changed
    Status(BotStatusUpdate),
    /// Position opened, closed, or updated
    Position(PositionUpdate),
    /// Metrics updated (equity, daily P&L)
    Metrics(MetricsUpdate),
    /// Trade executed
    Trade(TradeUpdate),
    /// Price tick received
    Price(PriceUpdate),
    /// Alert event (trade, risk, or system)
    Alert(AlertEvent),
    /// Real-time equity update (paper mode)
    Equity(EquityUpdate),
}

/// Bot status update - operational state changes
#[derive(Debug, Clone, Serialize)]
pub struct BotStatusUpdate {
    /// Current status: "running", "stopped", "error"
    pub status: String,
    /// Name of active trading strategy
    pub active_strategy: String,
}

/// Position update - open/close/modify events
#[derive(Debug, Clone, Serialize)]
pub struct PositionUpdate {
    /// Trading pair symbol
    pub symbol: String,
    /// Position side: "long" or "short"
    pub side: String,
    /// Position quantity
    pub quantity: Decimal,
    /// Entry price
    pub entry_price: Decimal,
    /// Current market price
    pub current_price: Decimal,
    /// Unrealized profit/loss
    pub unrealized_pnl: Decimal,
}

/// Metrics update - periodic performance stats
#[derive(Debug, Clone, Serialize)]
pub struct MetricsUpdate {
    /// Current total equity
    pub equity: Decimal,
    /// Daily profit/loss
    pub daily_pnl: Decimal,
}

/// Trade update - execution notification
#[derive(Debug, Clone, Serialize)]
pub struct TradeUpdate {
    /// Unique trade identifier
    pub id: String,
    /// Trading pair symbol
    pub symbol: String,
    /// Order side: "buy" or "sell"
    pub side: String,
    /// Trade quantity
    pub quantity: Decimal,
    /// Execution price
    pub price: Decimal,
    /// Realized P&L (for closing trades)
    pub pnl: Option<Decimal>,
}

/// Price update - market data tick
#[derive(Debug, Clone, Serialize)]
pub struct PriceUpdate {
    /// Trading pair symbol
    pub symbol: String,
    /// Current price
    pub price: Decimal,
    /// Timestamp of price update
    pub timestamp: DateTime<Utc>,
}

/// Real-time equity update for paper mode
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquityUpdate {
    /// Current total equity in quote currency
    pub equity: Decimal,
    /// Timestamp of this equity calculation
    pub timestamp: DateTime<Utc>,
    /// Trading mode: "paper" or "live"
    pub mode: String,
}

// ============================================================================
// Indicator response types
// ============================================================================

/// RSI data point for charting
#[derive(Debug, Clone, Serialize)]
pub struct RsiPoint {
    /// Date string for chart (YYYY-MM-DD)
    pub time: String,
    /// RSI value (0-100)
    pub value: f64,
}

/// Bollinger Band data point
#[derive(Debug, Clone, Serialize)]
pub struct BollingerBandPoint {
    /// Date string for chart (YYYY-MM-DD)
    pub time: String,
    /// Upper band value (middle + 2 std dev)
    pub upper: Decimal,
    /// Middle band value (20-period SMA)
    pub middle: Decimal,
    /// Lower band value (middle - 2 std dev)
    pub lower: Decimal,
}

/// Indicator response containing RSI and Bollinger Bands
#[derive(Debug, Clone, Serialize)]
pub struct IndicatorResponse {
    /// RSI values (14-period)
    pub rsi: Vec<RsiPoint>,
    /// Bollinger Bands (20-period, 2 std dev)
    pub bollinger_bands: Vec<BollingerBandPoint>,
}

/// Query parameters for indicators endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct IndicatorQueryParams {
    /// Number of days of indicator data to return (default: 30)
    #[serde(default = "default_indicator_days")]
    pub days: u32,
}

fn default_indicator_days() -> u32 {
    30
}

// ============================================================================
// Control endpoint types
// ============================================================================

/// Request to switch the active trading strategy
#[derive(Debug, Clone, Deserialize)]
pub struct SwitchStrategyRequest {
    /// Name of the strategy to switch to (e.g., "rsi_stddev", "grid")
    pub strategy: String,
}

/// Generic command response for control actions
#[derive(Debug, Clone, Serialize)]
pub struct CommandResponse {
    /// Whether the command was successful
    pub success: bool,
    /// Human-readable message describing the result
    pub message: String,
}

/// Response listing available strategies
#[derive(Debug, Clone, Serialize)]
pub struct StrategiesResponse {
    /// List of available strategy names
    pub strategies: Vec<String>,
    /// Currently active strategy
    pub active: String,
}

/// Response from emergency close all positions endpoint
#[derive(Debug, Clone, Serialize)]
pub struct CloseAllResponse {
    /// Whether the operation succeeded
    pub success: bool,
    /// Human-readable message about the operation
    pub message: String,
    /// Number of positions that were closed
    pub closed_positions: u32,
}

// ============================================================================
// Risk endpoint types
// ============================================================================

/// Risk status response (camelCase for frontend)
///
/// Returns current risk engine state including portfolio, limits, and circuit breaker.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskStatusResponse {
    // Portfolio state
    /// Current total portfolio equity
    pub total_equity: Decimal,
    /// Profit/loss for current day
    pub daily_pnl: Decimal,
    /// High water mark (peak equity)
    pub peak_equity: Decimal,

    // Position limits
    /// Maximum position size as percentage of equity
    pub max_position_size_pct: Decimal,
    /// Current drawdown from peak as percentage (0-1)
    pub current_drawdown_pct: Decimal,
    /// Maximum allowed drawdown percentage
    pub max_drawdown_pct: Decimal,

    // Daily loss tracking
    /// Daily loss limit as percentage of equity
    pub daily_loss_limit_pct: Decimal,
    /// Current daily loss used as percentage of equity
    pub daily_loss_used_pct: Decimal,

    // Circuit breaker
    /// Status: "normal", "warning", or "triggered"
    pub circuit_breaker_status: String,
    /// When circuit breaker was triggered (if halted)
    pub circuit_breaker_triggered_at: Option<DateTime<Utc>>,

    // Position tracking
    /// Number of currently open positions
    pub open_positions: usize,
}

/// Risk configuration response (camelCase for frontend)
///
/// Returns the configured risk limits from the application configuration.
/// Uses f64 to match the config source types and serialize as JSON numbers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskConfigResponse {
    /// Maximum position size as percentage of equity
    pub max_position_size_pct: f64,
    /// Stop loss percentage
    pub stop_loss_pct: f64,
    /// Daily loss limit as percentage of equity
    pub daily_loss_limit_pct: f64,
    /// Maximum allowed drawdown percentage
    pub max_drawdown_pct: f64,
}

/// Response for open position data (camelCase for frontend compatibility)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionResponse {
    /// Trading pair symbol
    pub symbol: String,
    /// Position side: "buy" (long)
    pub side: String,
    /// Position quantity
    pub quantity: Decimal,
    /// Entry price
    pub entry_price: Decimal,
    /// Unrealized P&L (0 for positions without current price)
    pub unrealized_pnl: Decimal,
}

// ============================================================================
// Historical tick data types
// ============================================================================

/// Query parameters for ticks endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct TicksQueryParams {
    /// Maximum number of ticks to return (default: 500)
    #[serde(default = "default_tick_limit")]
    pub limit: i64,
}

fn default_tick_limit() -> i64 {
    500
}

/// Response for individual tick data
#[derive(Debug, Clone, Serialize)]
pub struct TickResponse {
    /// Unique trade identifier
    pub trade_id: String,
    /// Timestamp of the tick
    pub timestamp: DateTime<Utc>,
    /// Price at this tick
    pub price: Decimal,
    /// Quantity traded
    pub quantity: Decimal,
}

// ============================================================================
// Backtest endpoint types
// ============================================================================

/// Request to run a backtest with a strategy
#[derive(Debug, Clone, Deserialize)]
pub struct BacktestRequest {
    /// Strategy name: "rsi_stddev" or "grid"
    pub strategy: String,
    /// Trading pair symbol (default: "BTCUSDT")
    pub symbol: Option<String>,
    /// Backtest start time (ISO 8601 date string, e.g., "2024-01-01")
    pub start_time: String,
    /// Backtest end time (ISO 8601 date string, e.g., "2024-01-31")
    pub end_time: String,
    /// Initial capital in quote currency (decimal as string, e.g., "10000")
    pub initial_capital: String,
    /// Trading fee percentage (decimal as string, e.g., "0.001" for 0.1%)
    pub fee_pct: String,
    /// Slippage percentage (decimal as string, e.g., "0.0005" for 0.05%)
    pub slippage_pct: String,
}

/// Complete backtest response with all metrics (camelCase for frontend)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestResponse {
    // Configuration
    /// Trading pair symbol
    pub symbol: String,
    /// Name of the strategy tested
    pub strategy_name: String,
    /// Backtest start time
    pub start_time: DateTime<Utc>,
    /// Backtest end time
    pub end_time: DateTime<Utc>,
    /// Starting capital in quote currency
    pub initial_capital: Decimal,
    /// Fee percentage used
    pub fee_pct: Decimal,
    /// Slippage percentage used
    pub slippage_pct: Decimal,

    // Summary metrics
    /// Final portfolio equity
    pub final_equity: Decimal,
    /// Total profit/loss
    pub total_pnl: Decimal,
    /// Total return as percentage
    pub total_return_pct: Decimal,
    /// Total number of completed trades
    pub total_trades: u32,
    /// Number of profitable trades
    pub winning_trades: u32,
    /// Number of losing trades
    pub losing_trades: u32,
    /// Win rate (0.0 to 1.0)
    pub win_rate: Decimal,

    // Risk metrics
    /// Maximum drawdown from peak to trough
    pub max_drawdown: Decimal,
    /// Sharpe ratio (risk-adjusted return)
    pub sharpe_ratio: Option<Decimal>,
    /// Sortino ratio (downside-adjusted return)
    pub sortino_ratio: Option<Decimal>,

    // Cost analysis
    /// Total fees paid across all trades
    pub total_fees_paid: Decimal,
    /// Total slippage cost across all trades
    pub total_slippage_cost: Decimal,

    // Optional detailed data
    /// Complete trade history
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trades: Option<Vec<TradeRecordResponse>>,
    /// Equity curve snapshots
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equity_curve: Option<Vec<EquityPointBacktest>>,
}

/// Individual trade record in backtest response (camelCase for frontend)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeRecordResponse {
    /// Time when position was entered
    pub entry_time: DateTime<Utc>,
    /// Time when position was exited (None if still open)
    pub exit_time: Option<DateTime<Utc>>,
    /// Trade direction: "buy" or "sell"
    pub side: String,
    /// Price at entry
    pub entry_price: Decimal,
    /// Price at exit (None if still open)
    pub exit_price: Option<Decimal>,
    /// Position size
    pub quantity: Decimal,
    /// Realized profit/loss (None if still open)
    pub pnl: Option<Decimal>,
    /// Commission paid for the trade
    pub commission: Decimal,
}

/// Single point on the equity curve for backtest (camelCase for frontend)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquityPointBacktest {
    /// Timestamp of this equity snapshot
    pub timestamp: DateTime<Utc>,
    /// Total portfolio equity at this point
    pub equity: Decimal,
}

// ============================================================================
// Manual order endpoint types
// ============================================================================

/// Request to submit a manual order (buy or sell)
#[derive(Debug, Clone, Deserialize)]
pub struct OrderRequest {
    /// Trading pair symbol (e.g., "BTCUSDT")
    pub symbol: String,
    /// Quantity to trade (Decimal as string for precision)
    pub quantity: String,
    /// Order type: "market" or "limit"
    pub order_type: String,
    /// Limit price (required if order_type is "limit")
    pub price: Option<String>,
}

/// Response from manual order submission (camelCase for frontend)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse {
    /// Unique order identifier
    pub order_id: String,
    /// Trading pair symbol
    pub symbol: String,
    /// Order side: "buy" or "sell"
    pub side: String,
    /// Quantity filled
    pub quantity: Decimal,
    /// Fill price (average if multiple fills)
    pub fill_price: Decimal,
    /// Order status
    pub status: String,
    /// Execution timestamp (milliseconds since epoch)
    pub timestamp: i64,
    /// Whether this order was executed in paper trading mode
    pub paper_mode: bool,
}

/// Query parameters for order history endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct OrderHistoryParams {
    /// Maximum number of orders to return (default: 50)
    #[serde(default = "default_order_limit")]
    pub limit: u32,
    /// Number of orders to skip for pagination (default: 0)
    #[serde(default)]
    pub offset: u32,
}

fn default_order_limit() -> u32 {
    50
}

/// Order history item (camelCase for frontend)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderHistoryResponse {
    /// Unique order identifier
    pub order_id: String,
    /// Trading pair symbol
    pub symbol: String,
    /// Order side: "buy" or "sell"
    pub side: String,
    /// Trade quantity
    pub quantity: Decimal,
    /// Entry price
    pub entry_price: Decimal,
    /// Entry timestamp
    pub entry_time: DateTime<Utc>,
    /// Exit price (None if position still open)
    pub exit_price: Option<Decimal>,
    /// Exit timestamp (None if position still open)
    pub exit_time: Option<DateTime<Utc>>,
    /// Realized P&L (None if position still open)
    pub pnl: Option<Decimal>,
    /// Reason/source of the order (e.g., "manual", "strategy:rsi_stddev")
    pub reason: String,
}

// ============================================================================
// Notification endpoint types
// ============================================================================

/// Notification configuration response (camelCase for frontend)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationConfigResponse {
    /// Whether trade notifications are enabled
    pub trades_enabled: bool,
    /// Whether risk alert notifications are enabled
    pub risk_alerts_enabled: bool,
    /// Whether system event notifications are enabled
    pub system_events_enabled: bool,
}

/// Request for updating notification configuration (camelCase for frontend)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNotificationConfigRequest {
    /// Whether trade notifications are enabled (optional for partial updates)
    pub trades_enabled: Option<bool>,
    /// Whether risk alert notifications are enabled (optional for partial updates)
    pub risk_alerts_enabled: Option<bool>,
    /// Whether system event notifications are enabled (optional for partial updates)
    pub system_events_enabled: Option<bool>,
}

/// Telegram connection status enum (snake_case for JSON)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramConnectionStatus {
    /// Bot is connected to Telegram API
    Connected,
    /// Bot is configured but cannot reach Telegram API
    Disconnected,
    /// Telegram bot token not configured
    NotConfigured,
}

/// Telegram status response (camelCase for frontend)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramStatusResponse {
    /// Connection status
    pub status: TelegramConnectionStatus,
    /// Bot username (if connected)
    pub bot_username: Option<String>,
    /// Whether chat_id is configured for notifications
    pub chat_id_configured: bool,
}

// ============================================================================
// Trading mode endpoint types
// ============================================================================

/// Response for GET /api/mode
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeResponse {
    /// Current trading mode: "paper" or "live"
    pub mode: String,
    /// Whether live trading is available (credentials configured)
    pub live_enabled: bool,
}

/// Request for POST /api/mode
#[derive(Debug, Clone, Deserialize)]
pub struct SwitchModeRequest {
    /// Target mode: "paper" or "live"
    pub mode: String,
}

/// Response for POST /api/mode
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchModeResponse {
    pub success: bool,
    pub mode: String,
    pub message: String,
}

// ============================================================================
// Mode-specific risk config endpoint types
// ============================================================================

/// Request to update risk configuration (partial updates supported)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRiskConfigRequest {
    /// Maximum position size as percentage of equity (optional for partial update)
    pub max_position_size_pct: Option<f64>,
    /// Stop loss percentage (optional for partial update)
    pub stop_loss_pct: Option<f64>,
    /// Daily loss limit as percentage of equity (optional for partial update)
    pub daily_loss_limit_pct: Option<f64>,
    /// Maximum allowed drawdown percentage (optional for partial update)
    pub max_drawdown_pct: Option<f64>,
}

/// Response for risk config update
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRiskConfigResponse {
    pub success: bool,
    pub message: String,
    pub config: RiskConfigResponse,
}

// ============================================================================
// Paper account reset types
// ============================================================================

/// Request to reset paper trading account
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPaperAccountRequest {
    /// Desired initial balance after reset (in quote currency, e.g., USDT)
    pub initial_balance: String,
}

/// Summary of what was cleared during reset
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetSummary {
    /// Previous equity before reset
    pub old_balance: Decimal,
    /// New balance after reset
    pub new_balance: Decimal,
    /// Number of open positions that were closed
    pub positions_closed: u32,
    /// Number of trade records deleted
    pub trades_deleted: u32,
    /// Number of equity history points deleted
    pub equity_points_deleted: u32,
}

/// Response from paper account reset
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPaperAccountResponse {
    /// Whether the reset was successful
    pub success: bool,
    /// Human-readable message
    pub message: String,
    /// Summary of cleared data
    pub summary: ResetSummary,
}

// ============================================================================
// Optimization endpoint types
// ============================================================================

/// Request to run parameter optimization
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationRequest {
    /// Trading pair symbol (default: "BTCUSDT")
    pub symbol: Option<String>,
    /// Optimization start time (ISO 8601 date string)
    pub start_time: String,
    /// Optimization end time (ISO 8601 date string)
    pub end_time: String,
    /// Initial capital in quote currency
    pub initial_capital: String,
    /// Trading fee percentage
    pub fee_pct: String,
    /// Slippage percentage
    pub slippage_pct: String,
    /// RSI period minimum
    pub period_min: u32,
    /// RSI period maximum
    pub period_max: u32,
    /// StdDev multiplier minimum
    pub multiplier_min: String,
    /// StdDev multiplier maximum
    pub multiplier_max: String,
    /// Steps per parameter (default: 5)
    pub steps: Option<usize>,
}

/// Optimization response (camelCase for frontend)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationResponse {
    /// Best parameters found
    pub best_params: OptimizedParams,
    /// Training metrics for best params
    pub train_result: BacktestResponse,
    /// Validation metrics for best params
    pub validate_result: BacktestResponse,
    /// Whether validation passed (out-of-sample return > 0)
    pub validation_passed: bool,
    /// Top N results ranked by training performance
    pub top_results: Vec<OptimizationResultEntry>,
    /// Total parameter combinations tested
    pub combinations_tested: usize,
}

/// Optimized parameter values
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizedParams {
    /// RSI period value
    pub rsi_period: usize,
    /// StdDev multiplier value
    pub stddev_multiplier: Decimal,
}

/// Single entry in top results list
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationResultEntry {
    /// Parameter values for this entry
    pub params: OptimizedParams,
    /// Training return percentage
    pub train_return_pct: Decimal,
    /// Validation return percentage
    pub validate_return_pct: Decimal,
    /// Training Sharpe ratio (if available)
    pub train_sharpe: Option<Decimal>,
    /// Validation Sharpe ratio (if available)
    pub validate_sharpe: Option<Decimal>,
}

// ============================================================================
// Strategy config update types
// ============================================================================

/// Request to update RSI-StdDev strategy configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStrategyConfigRequest {
    /// RSI period (e.g., 14)
    pub rsi_period: Option<usize>,
    /// StdDev multiplier (e.g., 1.5)
    pub stddev_multiplier: Option<f64>,
}

/// Response from strategy config update
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStrategyConfigResponse {
    pub success: bool,
    pub message: String,
    /// Updated RSI period
    pub rsi_period: usize,
    /// Updated StdDev multiplier
    pub stddev_multiplier: f64,
}
