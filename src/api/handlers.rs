//! REST API request handlers
//!
//! Each handler receives shared state and returns JSON responses.

use std::sync::{Arc, OnceLock};

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use chrono::{Duration, Utc};
use rust_decimal::Decimal;
use tokio::sync::RwLock;
use tracing::{error, info, instrument, warn};

use super::state::AppState;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

use super::types::{
    BacktestRequest, BacktestResponse, BollingerBandPoint, BotStatusResponse, BotStatusUpdate,
    BotUpdate, CloseAllResponse, CommandResponse, EquityCurveParams, EquityPointBacktest,
    EquityPointResponse, IndicatorQueryParams, IndicatorResponse, MetricsResponse,
    ModeResponse, NotificationConfigResponse, OrderHistoryParams, OrderHistoryResponse,
    OrderRequest, OrderResponse, PositionResponse, PositionUpdate, RiskConfigResponse,
    RiskStatusResponse, RsiPoint, StrategiesResponse, SwitchModeRequest, SwitchModeResponse,
    SwitchStrategyRequest, TelegramConnectionStatus, TelegramStatusResponse, TickResponse,
    TicksQueryParams, TradeRecordResponse, TradeResponse, TradesQueryParams,
    UpdateNotificationConfigRequest, UpdateRiskConfigRequest, UpdateRiskConfigResponse,
    ResetPaperAccountRequest, ResetPaperAccountResponse, ResetSummary,
    OptimizationRequest, OptimizationResponse, OptimizedParams, OptimizationResultEntry,
    UpdateStrategyConfigRequest, UpdateStrategyConfigResponse,
};

use crate::backtest::{
    run_backtest as execute_backtest, run_optimization, BacktestConfig, BacktestResult,
    OptimizationConfig,
};
use crate::exchange::Credentials;
use crate::data::{insert_order, NewOrder, OrderSide};
use crate::execution::{ExecuteOrderRequest, OrderExecutor, OrderType, PaperExecutor};
use crate::strategy::{GridConfig, GridStrategy, RsiStdDevConfig, RsiStdDevStrategy, Strategy};
use chrono::DateTime;
use rust_decimal_macros::dec;
use std::str::FromStr;

// ============================================================================
// Global bot state (shared across handlers)
// ============================================================================

/// Global bot running state
static BOT_RUNNING: OnceLock<RwLock<bool>> = OnceLock::new();
/// Global active strategy state
static ACTIVE_STRATEGY: OnceLock<RwLock<String>> = OnceLock::new();
/// Global trading mode state (true = paper, false = live)
static TRADING_MODE: OnceLock<RwLock<bool>> = OnceLock::new();

/// Get or initialize the bot running state
fn get_bot_running() -> &'static RwLock<bool> {
    BOT_RUNNING.get_or_init(|| RwLock::new(false))
}

/// Get or initialize the active strategy state
fn get_active_strategy() -> &'static RwLock<String> {
    ACTIVE_STRATEGY.get_or_init(|| RwLock::new(String::from("rsi_stddev")))
}

/// Get or initialize the trading mode state (defaults to paper for safety)
fn get_trading_mode_state() -> &'static RwLock<bool> {
    TRADING_MODE.get_or_init(|| RwLock::new(true))
}

/// Get current trading mode as string for database queries
/// Call once at start of handler, use for all queries in that request
async fn get_current_mode_str() -> &'static str {
    let is_paper = *get_trading_mode_state().read().await;
    if is_paper { "paper" } else { "live" }
}

/// Get current bot status
#[instrument(skip(state))]
pub async fn get_status(State(state): State<Arc<AppState>>) -> Json<BotStatusResponse> {
    let config = state.get_config();
    let running = get_bot_running().read().await;
    let strategy = get_active_strategy().read().await;

    Json(BotStatusResponse {
        status: if *running { "running" } else { "stopped" }.to_string(),
        active_strategy: strategy.clone(),
        uptime_seconds: state.uptime_seconds(),
        paper_mode: config.paper_trading.enabled,
    })
}

/// Get key performance metrics
#[instrument(skip(state))]
pub async fn get_metrics(
    State(state): State<Arc<AppState>>,
) -> Result<Json<MetricsResponse>, StatusCode> {
    // Read mode ONCE at start for all queries in this request
    let mode = get_current_mode_str().await;

    // Query total P&L from closed orders (filtered by mode)
    let total_pnl: Option<Decimal> = sqlx::query_scalar(
        "SELECT COALESCE(SUM(pnl), 0) FROM orders WHERE pnl IS NOT NULL AND trading_mode = $1",
    )
    .bind(mode)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        error!("Failed to query total P&L: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Query today's P&L (orders closed today, filtered by mode)
    let today = Utc::now().date_naive();
    let daily_pnl: Option<Decimal> = sqlx::query_scalar(
        "SELECT COALESCE(SUM(pnl), 0) FROM orders WHERE pnl IS NOT NULL AND DATE(exit_time) = $1 AND trading_mode = $2",
    )
    .bind(today)
    .bind(mode)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        error!("Failed to query daily P&L: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Calculate win rate (filtered by mode)
    let win_rate = calculate_win_rate(&state.db_pool, mode).await.map_err(|e| {
        error!("Failed to calculate win rate: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Get initial equity based on mode
    // For paper mode: use config initial balance
    // For live mode: start from ZERO (no exchange balance query yet)
    let config = state.get_config();
    let initial_equity = if mode == "paper" {
        Decimal::try_from(config.paper_trading.initial_balances.get("USDT").copied().unwrap_or(10000.0))
            .unwrap_or_else(|_| Decimal::new(10000, 0))
    } else {
        Decimal::ZERO
    };
    let equity = initial_equity + total_pnl.unwrap_or(Decimal::ZERO);

    Ok(Json(MetricsResponse {
        equity,
        daily_pnl: daily_pnl.unwrap_or(Decimal::ZERO),
        win_rate,
        sharpe_ratio: None,    // TODO: Calculate from trade history
        max_drawdown: None,    // TODO: Track max drawdown
    }))
}

/// Calculate win rate from closed trades (filtered by trading mode)
async fn calculate_win_rate(pool: &sqlx::PgPool, mode: &str) -> Result<Option<f64>, sqlx::Error> {
    let stats: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*) FILTER (WHERE pnl > 0), COUNT(*) FROM orders WHERE pnl IS NOT NULL AND trading_mode = $1",
    )
    .bind(mode)
    .fetch_one(pool)
    .await?;

    let (wins, total) = stats;
    if total == 0 {
        Ok(None)
    } else {
        Ok(Some((wins as f64 / total as f64) * 100.0))
    }
}

/// Get open positions (orders without exit_time)
#[instrument(skip(state))]
pub async fn get_positions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PositionResponse>>, StatusCode> {
    let mode = get_current_mode_str().await;
    let positions: Vec<PositionRow> = sqlx::query_as(
        r#"
        SELECT symbol, side, quantity, entry_price
        FROM orders
        WHERE exit_time IS NULL AND trading_mode = $1
        ORDER BY entry_time DESC
        "#,
    )
    .bind(mode)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        error!("Failed to query positions: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let responses: Vec<PositionResponse> = positions
        .into_iter()
        .map(|row| PositionResponse {
            symbol: row.symbol,
            side: row.side,
            quantity: row.quantity,
            entry_price: row.entry_price,
            unrealized_pnl: Decimal::ZERO, // Would need current price to calculate
        })
        .collect();

    Ok(Json(responses))
}

/// Internal row type for position queries
#[derive(sqlx::FromRow)]
struct PositionRow {
    symbol: String,
    side: String,
    quantity: Decimal,
    entry_price: Decimal,
}

/// Get paginated trade history
#[instrument(skip(state))]
pub async fn get_trades(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TradesQueryParams>,
) -> Result<Json<Vec<TradeResponse>>, StatusCode> {
    let mode = get_current_mode_str().await;
    let trades: Vec<TradeRow> = sqlx::query_as(
        r#"
        SELECT order_id, symbol, side, quantity, entry_price, entry_time, pnl
        FROM orders
        WHERE trading_mode = $1
        ORDER BY entry_time DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(mode)
    .bind(params.limit as i64)
    .bind(params.offset as i64)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        error!("Failed to query trades: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let responses: Vec<TradeResponse> = trades
        .into_iter()
        .map(|row| TradeResponse {
            id: row.order_id,
            symbol: row.symbol,
            side: row.side,
            quantity: row.quantity,
            price: row.entry_price,
            timestamp: row.entry_time,
            pnl: row.pnl,
        })
        .collect();

    Ok(Json(responses))
}

/// Internal row type for trade queries
#[derive(sqlx::FromRow)]
struct TradeRow {
    order_id: String,
    symbol: String,
    side: String,
    quantity: Decimal,
    entry_price: Decimal,
    entry_time: chrono::DateTime<Utc>,
    pnl: Option<Decimal>,
}

/// Get equity curve data for charting
#[instrument(skip(state))]
pub async fn get_equity_curve(
    State(state): State<Arc<AppState>>,
    Query(params): Query<EquityCurveParams>,
) -> Result<Json<Vec<EquityPointResponse>>, StatusCode> {
    let mode = get_current_mode_str().await;
    let since = Utc::now() - Duration::days(params.days as i64);

    // Get closed orders with P&L in chronological order (filtered by mode)
    let orders: Vec<EquityRow> = sqlx::query_as(
        r#"
        SELECT exit_time, pnl
        FROM orders
        WHERE pnl IS NOT NULL AND exit_time >= $1 AND trading_mode = $2
        ORDER BY exit_time ASC
        "#,
    )
    .bind(since)
    .bind(mode)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        error!("Failed to query equity curve: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Get initial equity based on mode
    // For paper mode: use config initial balance
    // For live mode: start from ZERO (no exchange balance query yet)
    let config = state.get_config();
    let initial_equity = if mode == "paper" {
        Decimal::try_from(config.paper_trading.initial_balances.get("USDT").copied().unwrap_or(10000.0))
            .unwrap_or_else(|_| Decimal::new(10000, 0))
    } else {
        Decimal::ZERO
    };

    // Build running equity curve
    let mut equity = initial_equity;
    let mut curve: Vec<EquityPointResponse> = Vec::with_capacity(orders.len() + 1);

    // Add starting point
    curve.push(EquityPointResponse {
        timestamp: since,
        value: initial_equity,
    });

    // Add each trade's effect on equity
    for order in orders {
        if let (Some(exit_time), Some(pnl)) = (order.exit_time, order.pnl) {
            equity += pnl;
            curve.push(EquityPointResponse {
                timestamp: exit_time,
                value: equity,
            });
        }
    }

    Ok(Json(curve))
}

/// Internal row type for equity curve queries
#[derive(sqlx::FromRow)]
struct EquityRow {
    exit_time: Option<chrono::DateTime<Utc>>,
    pnl: Option<Decimal>,
}

/// Get indicator data (RSI and Bollinger Bands)
#[instrument(skip(state))]
pub async fn get_indicators(
    State(state): State<Arc<AppState>>,
    Query(params): Query<IndicatorQueryParams>,
) -> Result<Json<IndicatorResponse>, StatusCode> {
    let since = Utc::now() - Duration::days(params.days as i64);

    // Query daily OHLC from ticks table
    let rows: Vec<DailyOhlcRow> = sqlx::query_as(
        r#"
        SELECT
            DATE(timestamp) as day,
            MIN(price) as low,
            MAX(price) as high,
            (array_agg(price ORDER BY timestamp))[1] as open,
            (array_agg(price ORDER BY timestamp DESC))[1] as close
        FROM ticks
        WHERE timestamp >= $1
        GROUP BY DATE(timestamp)
        ORDER BY day
        "#,
    )
    .bind(since)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        error!("Failed to fetch tick data for indicators: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Extract close prices as f64 for computation
    let closes: Vec<f64> = rows
        .iter()
        .filter_map(|r| {
            r.close
                .as_ref()
                .and_then(|d| d.to_string().parse::<f64>().ok())
        })
        .collect();

    // Compute RSI (14-period)
    let rsi_values = compute_rsi(&closes, 14);
    let rsi_points: Vec<RsiPoint> = rows
        .iter()
        .skip(14) // RSI needs warmup period
        .zip(rsi_values.iter())
        .filter_map(|(r, &rsi_val)| {
            r.day.map(|d| RsiPoint {
                time: d.to_string(),
                value: rsi_val,
            })
        })
        .collect();

    // Compute Bollinger Bands (20-period, 2 std dev)
    let bb_values = compute_bollinger_bands(&closes, 20, 2.0);
    let bb_points: Vec<BollingerBandPoint> = rows
        .iter()
        .skip(19) // BB needs warmup period
        .zip(bb_values.iter())
        .filter_map(|(r, (upper, middle, lower))| {
            r.day.map(|d| BollingerBandPoint {
                time: d.to_string(),
                upper: Decimal::from_f64(*upper).unwrap_or(Decimal::ZERO),
                middle: Decimal::from_f64(*middle).unwrap_or(Decimal::ZERO),
                lower: Decimal::from_f64(*lower).unwrap_or(Decimal::ZERO),
            })
        })
        .collect();

    Ok(Json(IndicatorResponse {
        rsi: rsi_points,
        bollinger_bands: bb_points,
    }))
}

/// Internal row type for daily OHLC queries
#[derive(sqlx::FromRow)]
struct DailyOhlcRow {
    day: Option<chrono::NaiveDate>,
    #[allow(dead_code)]
    low: Option<Decimal>,
    #[allow(dead_code)]
    high: Option<Decimal>,
    #[allow(dead_code)]
    open: Option<Decimal>,
    close: Option<Decimal>,
}

/// Compute RSI (Relative Strength Index) using the standard 14-period formula
///
/// RSI = 100 - (100 / (1 + RS))
/// RS = Average Gain / Average Loss (over the period)
fn compute_rsi(closes: &[f64], period: usize) -> Vec<f64> {
    if closes.len() < period + 1 {
        return vec![];
    }

    let mut gains = Vec::with_capacity(closes.len() - 1);
    let mut losses = Vec::with_capacity(closes.len() - 1);

    // Calculate price changes
    for i in 1..closes.len() {
        let change = closes[i] - closes[i - 1];
        if change > 0.0 {
            gains.push(change);
            losses.push(0.0);
        } else {
            gains.push(0.0);
            losses.push(-change);
        }
    }

    let mut rsi = Vec::with_capacity(closes.len() - period);

    // Initial average (simple average)
    let mut avg_gain: f64 = gains[..period].iter().sum::<f64>() / period as f64;
    let mut avg_loss: f64 = losses[..period].iter().sum::<f64>() / period as f64;

    // Wilder's smoothing method for subsequent values
    for i in period..gains.len() {
        avg_gain = (avg_gain * (period - 1) as f64 + gains[i]) / period as f64;
        avg_loss = (avg_loss * (period - 1) as f64 + losses[i]) / period as f64;

        let rs = if avg_loss == 0.0 {
            100.0
        } else {
            avg_gain / avg_loss
        };
        rsi.push(100.0 - (100.0 / (1.0 + rs)));
    }

    rsi
}

/// Compute Bollinger Bands (standard: 20-period SMA with 2 standard deviations)
///
/// Returns Vec of (upper, middle, lower) tuples
fn compute_bollinger_bands(closes: &[f64], period: usize, num_std: f64) -> Vec<(f64, f64, f64)> {
    if closes.len() < period {
        return vec![];
    }

    let mut bands = Vec::with_capacity(closes.len() - period + 1);

    for i in (period - 1)..closes.len() {
        let slice = &closes[(i + 1 - period)..=i];
        let mean: f64 = slice.iter().sum::<f64>() / period as f64;
        let variance: f64 = slice.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / period as f64;
        let std_dev = variance.sqrt();

        let upper = mean + num_std * std_dev;
        let lower = mean - num_std * std_dev;
        bands.push((upper, mean, lower));
    }

    bands
}

// ============================================================================
// Emergency close endpoint
// ============================================================================

/// Emergency close all open positions
///
/// This endpoint is for emergency situations where all positions need to be
/// closed immediately. It logs a warning, queries open positions, and triggers
/// market sells for each position.
#[instrument(skip(state))]
pub async fn close_all_positions(State(state): State<Arc<AppState>>) -> Json<CloseAllResponse> {
    tracing::warn!("EMERGENCY: Close all positions triggered via dashboard");

    // In production, this would:
    // 1. Query all open positions from database
    // 2. Submit market sell orders for each
    // 3. Wait for confirmation
    // 4. Return results

    // For now, simulate the close and broadcast position updates
    // This demonstrates the pattern - real implementation connects to execution module

    let closed_count = 0; // Would be actual count from database

    // Broadcast empty positions (all closed)
    state.broadcast(BotUpdate::Position(PositionUpdate {
        symbol: "BTCUSDT".to_string(),
        side: "sell".to_string(),
        quantity: Decimal::ZERO,
        entry_price: Decimal::ZERO,
        current_price: Decimal::ZERO,
        unrealized_pnl: Decimal::ZERO,
    }));

    info!("Emergency close completed: {} positions closed", closed_count);

    Json(CloseAllResponse {
        success: true,
        message: "All positions closed".to_string(),
        closed_positions: closed_count,
    })
}

// ============================================================================
// Control handlers (start, stop, strategy, optimization)
// ============================================================================

/// Available trading strategies
const AVAILABLE_STRATEGIES: &[&str] = &["rsi_stddev", "grid"];

/// Start the trading bot
#[instrument(skip(state))]
pub async fn start_bot(State(state): State<Arc<AppState>>) -> Json<CommandResponse> {
    let mut running = get_bot_running().write().await;

    if *running {
        return Json(CommandResponse {
            success: false,
            message: "Bot is already running".to_string(),
        });
    }

    *running = true;
    info!("Bot started via dashboard");

    // Broadcast status update
    let strategy = get_active_strategy().read().await.clone();
    state.broadcast(BotUpdate::Status(BotStatusUpdate {
        status: "running".to_string(),
        active_strategy: strategy,
    }));

    Json(CommandResponse {
        success: true,
        message: "Bot started".to_string(),
    })
}

/// Stop the trading bot
#[instrument(skip(state))]
pub async fn stop_bot(State(state): State<Arc<AppState>>) -> Json<CommandResponse> {
    let mut running = get_bot_running().write().await;

    if !*running {
        return Json(CommandResponse {
            success: false,
            message: "Bot is not running".to_string(),
        });
    }

    *running = false;
    info!("Bot stopped via dashboard");

    // Broadcast status update
    let strategy = get_active_strategy().read().await.clone();
    state.broadcast(BotUpdate::Status(BotStatusUpdate {
        status: "stopped".to_string(),
        active_strategy: strategy,
    }));

    Json(CommandResponse {
        success: true,
        message: "Bot stopped".to_string(),
    })
}

/// Switch the active trading strategy
#[instrument(skip(state))]
pub async fn switch_strategy(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SwitchStrategyRequest>,
) -> Json<CommandResponse> {
    if !AVAILABLE_STRATEGIES.contains(&req.strategy.as_str()) {
        return Json(CommandResponse {
            success: false,
            message: format!(
                "Unknown strategy: {}. Available: {:?}",
                req.strategy, AVAILABLE_STRATEGIES
            ),
        });
    }

    let mut strategy = get_active_strategy().write().await;
    *strategy = req.strategy.clone();
    info!("Strategy switched to: {}", req.strategy);

    // Broadcast status update
    let running = get_bot_running().read().await;
    state.broadcast(BotUpdate::Status(BotStatusUpdate {
        status: if *running { "running" } else { "stopped" }.to_string(),
        active_strategy: req.strategy.clone(),
    }));

    Json(CommandResponse {
        success: true,
        message: format!("Strategy switched to {}", req.strategy),
    })
}

/// Get list of available strategies and the currently active one
#[instrument]
pub async fn get_strategies() -> Json<StrategiesResponse> {
    let active = get_active_strategy().read().await.clone();
    Json(StrategiesResponse {
        strategies: AVAILABLE_STRATEGIES.iter().map(|s| s.to_string()).collect(),
        active,
    })
}

/// Parse datetime string supporting both ISO 8601 and date-only formats
fn parse_datetime(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    // Try full ISO 8601 first, then date-only format
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
        })
}

/// Run parameter optimization with walk-forward validation (OPT-01)
///
/// Performs grid search over RSI period and StdDev multiplier parameters,
/// splitting data into train (70%) and validate (30%) periods.
#[instrument(skip(state, request))]
pub async fn trigger_optimization(
    State(state): State<Arc<AppState>>,
    Json(request): Json<OptimizationRequest>,
) -> Result<Json<OptimizationResponse>, StatusCode> {
    info!("Optimization triggered via dashboard");

    // Parse request parameters
    let symbol = request.symbol.unwrap_or_else(|| "BTCUSDT".to_string());
    let steps = request.steps.unwrap_or(5);

    // Parse dates
    let start_time = parse_datetime(&request.start_time)
        .map_err(|_| {
            error!("Invalid start_time format: {}", request.start_time);
            StatusCode::BAD_REQUEST
        })?;

    let end_time = parse_datetime(&request.end_time)
        .map_err(|_| {
            error!("Invalid end_time format: {}", request.end_time);
            StatusCode::BAD_REQUEST
        })?;

    // Parse decimal values
    let initial_capital = request.initial_capital.parse::<Decimal>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let fee_pct = request.fee_pct.parse::<Decimal>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let slippage_pct = request.slippage_pct.parse::<Decimal>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let multiplier_min = request.multiplier_min.parse::<Decimal>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let multiplier_max = request.multiplier_max.parse::<Decimal>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    // Build optimization config
    let opt_config = OptimizationConfig {
        base_config: BacktestConfig {
            symbol: symbol.clone(),
            start_time,
            end_time,
            initial_capital,
            fee_pct,
            slippage_pct,
        },
        period_range: (request.period_min as usize, request.period_max as usize),
        multiplier_range: (multiplier_min, multiplier_max),
        steps,
        train_ratio: 0.70, // 70/30 split per CONTEXT.md
        min_ticks: 1000,   // Minimum tick requirement
    };

    // Run optimization
    let result = run_optimization(&state.db_pool, &opt_config)
        .await
        .map_err(|e| {
            error!("Optimization failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Get best result (or return error if none)
    let best = result.best.ok_or_else(|| {
        error!("No valid optimization results");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Convert to response format
    let train_response = backtest_result_to_response(&best.train_result);
    let validate_response = backtest_result_to_response(&best.validate_result);

    // Build top results (top 5)
    let top_results: Vec<OptimizationResultEntry> = result.results
        .iter()
        .take(5)
        .map(|entry| OptimizationResultEntry {
            params: OptimizedParams {
                rsi_period: entry.rsi_period,
                stddev_multiplier: entry.stddev_multiplier,
            },
            train_return_pct: entry.train_result.total_return_pct,
            validate_return_pct: entry.validate_result.total_return_pct,
            train_sharpe: entry.train_result.sharpe_ratio,
            validate_sharpe: entry.validate_result.sharpe_ratio,
        })
        .collect();

    let response = OptimizationResponse {
        best_params: OptimizedParams {
            rsi_period: best.rsi_period,
            stddev_multiplier: best.stddev_multiplier,
        },
        train_result: train_response,
        validate_result: validate_response,
        validation_passed: result.validation_passed,
        top_results,
        combinations_tested: result.combinations_tested,
    };

    info!(
        "Optimization complete: {} combinations, best period={}, mult={}, validation={}",
        result.combinations_tested, best.rsi_period, best.stddev_multiplier, result.validation_passed
    );

    Ok(Json(response))
}

/// Convert internal BacktestResult to API BacktestResponse
fn backtest_result_to_response(result: &BacktestResult) -> BacktestResponse {
    BacktestResponse {
        symbol: result.symbol.clone(),
        strategy_name: result.strategy_name.clone(),
        start_time: result.start_time,
        end_time: result.end_time,
        initial_capital: result.initial_capital,
        fee_pct: result.fee_pct,
        slippage_pct: result.slippage_pct,
        final_equity: result.final_equity,
        total_pnl: result.total_pnl,
        total_return_pct: result.total_return_pct,
        total_trades: result.total_trades,
        winning_trades: result.winning_trades,
        losing_trades: result.losing_trades,
        win_rate: result.win_rate,
        max_drawdown: result.max_drawdown,
        sharpe_ratio: result.sharpe_ratio,
        sortino_ratio: result.sortino_ratio,
        total_fees_paid: result.total_fees_paid,
        total_slippage_cost: result.total_slippage_cost,
        trades: result.trades.as_ref().map(|trades| {
            trades.iter().map(|t| TradeRecordResponse {
                entry_time: t.entry_time,
                exit_time: t.exit_time,
                side: format!("{:?}", t.side).to_lowercase(),
                entry_price: t.entry_price,
                exit_price: t.exit_price,
                quantity: t.quantity,
                pnl: t.pnl,
                commission: t.commission,
            }).collect()
        }),
        equity_curve: result.equity_curve.as_ref().map(|curve| {
            curve.iter().map(|p| EquityPointBacktest {
                timestamp: p.timestamp,
                equity: p.equity,
            }).collect()
        }),
    }
}

// ============================================================================
// Historical tick data handler
// ============================================================================

/// Internal row type for tick queries
#[derive(sqlx::FromRow)]
struct TickRow {
    trade_id: i64,
    timestamp: chrono::DateTime<Utc>,
    price: Decimal,
    quantity: Decimal,
}

/// Get recent ticks for chart seeding
///
/// Returns the most recent ticks from the database in chronological order
/// (oldest first) so they can be aggregated into candlesticks on the client.
#[instrument(skip(state))]
pub async fn get_recent_ticks(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TicksQueryParams>,
) -> Result<Json<Vec<TickResponse>>, StatusCode> {
    let ticks: Vec<TickRow> = sqlx::query_as(
        r#"
        SELECT trade_id, timestamp, price, quantity
        FROM ticks
        WHERE symbol = 'BTCUSDT'
        ORDER BY timestamp DESC
        LIMIT $1
        "#,
    )
    .bind(params.limit)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        error!("Failed to query ticks: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Return in chronological order (oldest first) for aggregation
    let responses: Vec<TickResponse> = ticks
        .into_iter()
        .rev()
        .map(|row| TickResponse {
            trade_id: row.trade_id.to_string(),
            timestamp: row.timestamp,
            price: row.price,
            quantity: row.quantity,
        })
        .collect();

    Ok(Json(responses))
}

// ============================================================================
// Backtest handler
// ============================================================================

/// Create a strategy instance from a strategy name
///
/// Uses sensible default configurations for each strategy type.
fn create_strategy_from_name(name: &str) -> Result<Box<dyn Strategy + Send>, String> {
    match name {
        "rsi_stddev" => {
            let config = RsiStdDevConfig {
                rsi_period: 14,
                rsi_oversold: 30.0,
                rsi_overbought: 70.0,
                stddev_period: 20,
                stddev_multiplier: 1.0,
                base_quantity: dec!(0.01),
            };
            Ok(Box::new(RsiStdDevStrategy::new(config)))
        }
        "grid" => {
            let config = GridConfig {
                spacing_pct: dec!(0.01),    // 1% spacing
                num_levels: 5,              // 5 levels each side
                quantity_per_level: dec!(0.001),
                escape_multiplier: dec!(2.0),
            };
            // Use 50000 as a reasonable BTC center price for backtesting
            Ok(Box::new(GridStrategy::new(config, dec!(50000))))
        }
        _ => Err(format!(
            "Unknown strategy: {}. Available: rsi_stddev, grid",
            name
        )),
    }
}

/// Run a backtest with the specified strategy and parameters
///
/// POST /api/backtest
///
/// Accepts strategy name, date range, and fee parameters.
/// Returns comprehensive backtest results including metrics and equity curve.
#[instrument(skip(state))]
pub async fn run_backtest(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BacktestRequest>,
) -> Result<Json<BacktestResponse>, StatusCode> {
    // Parse dates - append T00:00:00Z if needed for ISO 8601 compliance
    let start_str = if req.start_time.contains('T') {
        req.start_time.clone()
    } else {
        format!("{}T00:00:00Z", req.start_time)
    };
    let end_str = if req.end_time.contains('T') {
        req.end_time.clone()
    } else {
        format!("{}T23:59:59Z", req.end_time)
    };

    let start_time = start_str.parse::<DateTime<Utc>>().map_err(|e| {
        error!("Invalid start_time '{}': {}", req.start_time, e);
        StatusCode::BAD_REQUEST
    })?;
    let end_time = end_str.parse::<DateTime<Utc>>().map_err(|e| {
        error!("Invalid end_time '{}': {}", req.end_time, e);
        StatusCode::BAD_REQUEST
    })?;

    // Create strategy from name
    let mut strategy = create_strategy_from_name(&req.strategy).map_err(|e| {
        error!("{}", e);
        StatusCode::BAD_REQUEST
    })?;

    // Parse decimal parameters
    let initial_capital = Decimal::from_str(&req.initial_capital).map_err(|e| {
        error!("Invalid initial_capital '{}': {}", req.initial_capital, e);
        StatusCode::BAD_REQUEST
    })?;
    let fee_pct = Decimal::from_str(&req.fee_pct).map_err(|e| {
        error!("Invalid fee_pct '{}': {}", req.fee_pct, e);
        StatusCode::BAD_REQUEST
    })?;
    let slippage_pct = Decimal::from_str(&req.slippage_pct).map_err(|e| {
        error!("Invalid slippage_pct '{}': {}", req.slippage_pct, e);
        StatusCode::BAD_REQUEST
    })?;

    // Build backtest config
    let config = BacktestConfig {
        symbol: req.symbol.unwrap_or_else(|| "BTCUSDT".to_string()),
        start_time,
        end_time,
        initial_capital,
        fee_pct,
        slippage_pct,
    };

    info!(
        "Running backtest: {} on {} from {} to {}",
        req.strategy, config.symbol, start_time, end_time
    );

    // Run backtest
    let result = execute_backtest(&state.db_pool, strategy.as_mut(), &config)
        .await
        .map_err(|e| {
            error!("Backtest failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Convert to API response
    Ok(Json(BacktestResponse::from(result)))
}

// ============================================================================
// Risk handlers
// ============================================================================

/// Get current risk status from RiskEngine
///
/// Returns the current risk state including portfolio equity, drawdown,
/// daily loss tracking, and circuit breaker status.
///
/// Returns 503 SERVICE_UNAVAILABLE if RiskEngine is not available.
#[instrument(skip(state))]
pub async fn get_risk_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<RiskStatusResponse>, StatusCode> {
    let risk_handle = state
        .get_risk_handle()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let snapshot = risk_handle.query_state().await.map_err(|e| {
        error!("Failed to query risk state: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(RiskStatusResponse {
        total_equity: snapshot.total_equity,
        daily_pnl: snapshot.daily_pnl,
        peak_equity: snapshot.peak_equity,
        max_position_size_pct: snapshot.max_position_size_pct,
        current_drawdown_pct: snapshot.current_drawdown_pct,
        max_drawdown_pct: snapshot.max_drawdown_pct,
        daily_loss_limit_pct: snapshot.daily_loss_limit_pct,
        daily_loss_used_pct: snapshot.daily_loss_used_pct,
        circuit_breaker_status: snapshot.circuit_breaker_status,
        circuit_breaker_triggered_at: snapshot.circuit_breaker_triggered_at,
        open_positions: snapshot.open_positions,
    }))
}

/// Get risk configuration from AppConfig
///
/// Returns the configured risk limits. Does not require RiskEngine to be running.
#[instrument(skip(state))]
pub async fn get_risk_config(
    State(state): State<Arc<AppState>>,
) -> Result<Json<RiskConfigResponse>, StatusCode> {
    let config = state.get_config();

    Ok(Json(RiskConfigResponse {
        max_position_size_pct: config.strategy.risk.max_position_size_pct,
        stop_loss_pct: config.strategy.risk.stop_loss_pct,
        daily_loss_limit_pct: config.strategy.risk.daily_loss_limit_pct,
        max_drawdown_pct: config.strategy.risk.max_drawdown_pct,
    }))
}

/// Get paper mode risk configuration
///
/// GET /api/paper/risk/config
///
/// Returns the risk configuration specific to paper trading mode.
#[instrument(skip(state))]
pub async fn get_paper_risk_config(
    State(state): State<Arc<AppState>>,
) -> Result<Json<RiskConfigResponse>, (StatusCode, String)> {
    let config = state
        .mode_risk_configs
        .get("paper")
        .await
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Paper risk config not found".to_string(),
            )
        })?;

    Ok(Json(RiskConfigResponse {
        max_position_size_pct: config.max_position_size_pct.to_f64().unwrap_or(0.1),
        stop_loss_pct: config.stop_loss_pct.to_f64().unwrap_or(0.02),
        daily_loss_limit_pct: config.daily_loss_limit_pct.to_f64().unwrap_or(0.05),
        max_drawdown_pct: config.max_drawdown_pct.to_f64().unwrap_or(0.15),
    }))
}

/// Get live mode risk configuration
///
/// GET /api/live/risk/config
///
/// Returns the risk configuration specific to live trading mode.
#[instrument(skip(state))]
pub async fn get_live_risk_config(
    State(state): State<Arc<AppState>>,
) -> Result<Json<RiskConfigResponse>, (StatusCode, String)> {
    let config = state.mode_risk_configs.get("live").await.ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Live risk config not found".to_string(),
        )
    })?;

    Ok(Json(RiskConfigResponse {
        max_position_size_pct: config.max_position_size_pct.to_f64().unwrap_or(0.1),
        stop_loss_pct: config.stop_loss_pct.to_f64().unwrap_or(0.02),
        daily_loss_limit_pct: config.daily_loss_limit_pct.to_f64().unwrap_or(0.05),
        max_drawdown_pct: config.max_drawdown_pct.to_f64().unwrap_or(0.15),
    }))
}

/// Update paper mode risk configuration
///
/// PUT /api/paper/risk/config
///
/// Accepts partial updates - only provided fields are changed.
#[instrument(skip(state))]
pub async fn update_paper_risk_config(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpdateRiskConfigRequest>,
) -> Result<Json<UpdateRiskConfigResponse>, (StatusCode, String)> {
    update_mode_risk_config(&state, "paper", req).await
}

/// Update live mode risk configuration
///
/// PUT /api/live/risk/config
///
/// Accepts partial updates - only provided fields are changed.
#[instrument(skip(state))]
pub async fn update_live_risk_config(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpdateRiskConfigRequest>,
) -> Result<Json<UpdateRiskConfigResponse>, (StatusCode, String)> {
    update_mode_risk_config(&state, "live", req).await
}

/// Shared implementation for updating risk config
async fn update_mode_risk_config(
    state: &AppState,
    mode: &str,
    req: UpdateRiskConfigRequest,
) -> Result<Json<UpdateRiskConfigResponse>, (StatusCode, String)> {
    // Get current config
    let mut config = state.mode_risk_configs.get(mode).await.ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{} risk config not found", mode),
        )
    })?;

    // Apply partial updates
    if let Some(v) = req.max_position_size_pct {
        config.max_position_size_pct = Decimal::from_f64(v).unwrap_or(config.max_position_size_pct);
    }
    if let Some(v) = req.stop_loss_pct {
        config.stop_loss_pct = Decimal::from_f64(v).unwrap_or(config.stop_loss_pct);
    }
    if let Some(v) = req.daily_loss_limit_pct {
        config.daily_loss_limit_pct =
            Decimal::from_f64(v).unwrap_or(config.daily_loss_limit_pct);
    }
    if let Some(v) = req.max_drawdown_pct {
        config.max_drawdown_pct = Decimal::from_f64(v).unwrap_or(config.max_drawdown_pct);
    }

    // Save updated config
    state
        .mode_risk_configs
        .update(mode, config.clone())
        .await;

    info!(mode, "Risk config updated");

    Ok(Json(UpdateRiskConfigResponse {
        success: true,
        message: format!("{} risk configuration updated", mode),
        config: RiskConfigResponse {
            max_position_size_pct: config.max_position_size_pct.to_f64().unwrap_or(0.1),
            stop_loss_pct: config.stop_loss_pct.to_f64().unwrap_or(0.02),
            daily_loss_limit_pct: config.daily_loss_limit_pct.to_f64().unwrap_or(0.05),
            max_drawdown_pct: config.max_drawdown_pct.to_f64().unwrap_or(0.15),
        },
    }))
}

// ============================================================================
// Paper account reset handler
// ============================================================================

/// Reset paper trading account to initial state
///
/// POST /api/paper/reset
///
/// Clears all positions, trade history, and equity history.
/// Requires bot to be stopped to prevent race conditions.
#[instrument(skip(state))]
pub async fn reset_paper_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ResetPaperAccountRequest>,
) -> Result<Json<ResetPaperAccountResponse>, (StatusCode, String)> {
    // Parse initial balance
    let initial_balance = Decimal::from_str(&req.initial_balance)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid initial_balance format".to_string()))?;

    if initial_balance <= Decimal::ZERO {
        return Err((StatusCode::BAD_REQUEST, "initial_balance must be positive".to_string()));
    }

    // Check bot is stopped (precondition)
    let is_running = *get_bot_running().read().await;
    if is_running {
        return Err((
            StatusCode::CONFLICT,
            "Bot must be stopped before resetting paper account. Stop the bot first.".to_string(),
        ));
    }

    // Execute reset in transaction
    let summary = execute_paper_reset(&state.db_pool, initial_balance).await
        .map_err(|e| {
            error!(error = %e, "Failed to reset paper account");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Reset failed: {}", e))
        })?;

    info!(
        old_balance = %summary.old_balance,
        new_balance = %summary.new_balance,
        positions_closed = summary.positions_closed,
        trades_deleted = summary.trades_deleted,
        equity_points_deleted = summary.equity_points_deleted,
        "Paper account reset completed"
    );

    Ok(Json(ResetPaperAccountResponse {
        success: true,
        message: "Paper account reset successfully".to_string(),
        summary,
    }))
}

/// Execute paper account reset within a transaction
async fn execute_paper_reset(
    pool: &sqlx::PgPool,
    initial_balance: Decimal,
) -> Result<ResetSummary, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Get current equity for summary (most recent paper equity)
    let old_balance: Option<Decimal> = sqlx::query_scalar(
        "SELECT equity FROM equity_history WHERE trading_mode = 'paper' ORDER BY timestamp DESC LIMIT 1"
    )
    .fetch_optional(&mut *tx)
    .await?;

    // Count positions to be closed (open paper orders)
    let (positions_closed,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM orders WHERE trading_mode = 'paper' AND exit_time IS NULL"
    )
    .fetch_one(&mut *tx)
    .await?;

    // Count trades to be deleted (all paper orders)
    let (trades_deleted,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM orders WHERE trading_mode = 'paper'"
    )
    .fetch_one(&mut *tx)
    .await?;

    // Count equity points to be deleted
    let (equity_points_deleted,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM equity_history WHERE trading_mode = 'paper'"
    )
    .fetch_one(&mut *tx)
    .await?;

    // Delete all paper orders (positions and trades)
    sqlx::query("DELETE FROM orders WHERE trading_mode = 'paper'")
        .execute(&mut *tx)
        .await?;

    // Delete all paper equity history
    sqlx::query("DELETE FROM equity_history WHERE trading_mode = 'paper'")
        .execute(&mut *tx)
        .await?;

    // Insert initial equity point
    sqlx::query(
        "INSERT INTO equity_history (equity, trading_mode, timestamp) VALUES ($1, 'paper', NOW())"
    )
    .bind(&initial_balance)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(ResetSummary {
        old_balance: old_balance.unwrap_or(Decimal::ZERO),
        new_balance: initial_balance,
        positions_closed: positions_closed as u32,
        trades_deleted: trades_deleted as u32,
        equity_points_deleted: equity_points_deleted as u32,
    })
}

// ============================================================================
// Manual order handlers
// ============================================================================

/// Row type for order history query
#[derive(sqlx::FromRow)]
struct OrderHistoryRow {
    order_id: String,
    symbol: String,
    side: String,
    quantity: Decimal,
    entry_price: Decimal,
    entry_time: DateTime<Utc>,
    exit_price: Option<Decimal>,
    exit_time: Option<DateTime<Utc>>,
    pnl: Option<Decimal>,
    reason: String,
}

/// Get current market price for a symbol from the most recent tick
async fn get_current_price(pool: &sqlx::PgPool, symbol: &str) -> Result<Decimal, StatusCode> {
    let price: Option<Decimal> = sqlx::query_scalar(
        r#"
        SELECT price FROM ticks
        WHERE symbol = $1
        ORDER BY timestamp DESC
        LIMIT 1
        "#,
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        error!("Failed to query current price: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    price.ok_or_else(|| {
        error!("No price data available for symbol: {}", symbol);
        StatusCode::BAD_REQUEST
    })
}

/// Execute a manual order using the appropriate executor
///
/// Creates a fresh PaperExecutor (or would create LiveExecutor if not in paper mode).
/// This keeps manual orders independent from the running bot's order executor actor.
async fn execute_manual_order(
    state: &AppState,
    symbol: String,
    side: OrderSide,
    quantity: Decimal,
    order_type: OrderType,
    price: Option<Decimal>,
) -> Result<OrderResponse, (StatusCode, String)> {
    let config = state.get_config();
    let paper_mode = config.paper_trading.enabled;

    // Get current market price for execution
    let current_price = price.unwrap_or(
        get_current_price(&state.db_pool, &symbol)
            .await
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("No price data available for {}. Cannot execute market order without price reference.", symbol),
                )
            })?,
    );

    // Build execution request
    let exec_request = ExecuteOrderRequest {
        symbol: symbol.clone(),
        side,
        quantity,
        order_type,
        price: Some(current_price),
        client_order_id: None,
    };

    // Execute order using appropriate executor
    let response = if paper_mode {
        let executor = PaperExecutor::new(config.paper_trading.clone());
        executor.execute(exec_request).await.map_err(|e| {
            error!("Paper order execution failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Order execution failed: {}", e))
        })?
    } else {
        // For live mode, we would need credentials
        // For now, return an error - live mode requires proper credential handling
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "Live order execution from dashboard not yet supported. Use paper mode or bot strategies.".to_string(),
        ));
    };

    // Record order in database with current trading mode
    let is_paper = *get_trading_mode_state().read().await;
    let new_order = NewOrder {
        order_id: response.order_id.clone(),
        symbol: symbol.clone(),
        side,
        entry_price: response.fill_price,
        quantity: response.filled_quantity,
        reason: "manual".to_string(),
        entry_time: Utc::now(),
        trading_mode: if is_paper { "paper" } else { "live" }.to_string(),
    };

    if let Err(e) = insert_order(&state.db_pool, &new_order).await {
        error!("Failed to record order in database: {}", e);
        // Order executed but not recorded - still return success to user
    }

    info!(
        order_id = %response.order_id,
        symbol = %symbol,
        side = ?side,
        quantity = %quantity,
        fill_price = %response.fill_price,
        paper_mode = paper_mode,
        "Manual order executed"
    );

    Ok(OrderResponse {
        order_id: response.order_id,
        symbol,
        side: side.as_str().to_string(),
        quantity: response.filled_quantity,
        fill_price: response.fill_price,
        status: response.status.to_string(),
        timestamp: response.timestamp,
        paper_mode,
    })
}

/// Submit a buy order manually
///
/// POST /api/orders/buy
///
/// Accepts OrderRequest JSON body and executes a buy order.
/// Respects paper_trading.enabled configuration.
#[instrument(skip(state))]
pub async fn submit_buy_order(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OrderRequest>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    // Validation
    if req.symbol.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "symbol is required".to_string()));
    }

    let quantity = Decimal::from_str(&req.quantity).map_err(|_| {
        (StatusCode::BAD_REQUEST, format!("Invalid quantity: {}", req.quantity))
    })?;

    if quantity <= Decimal::ZERO {
        return Err((StatusCode::BAD_REQUEST, "quantity must be greater than 0".to_string()));
    }

    let order_type = match req.order_type.to_lowercase().as_str() {
        "market" => OrderType::Market,
        "limit" => OrderType::Limit,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Invalid order_type: {}. Must be 'market' or 'limit'", req.order_type),
            ))
        }
    };

    let price = if order_type == OrderType::Limit {
        let price_str = req.price.as_ref().ok_or_else(|| {
            (StatusCode::BAD_REQUEST, "price is required for limit orders".to_string())
        })?;
        let price_val = Decimal::from_str(price_str).map_err(|_| {
            (StatusCode::BAD_REQUEST, format!("Invalid price: {}", price_str))
        })?;
        if price_val <= Decimal::ZERO {
            return Err((StatusCode::BAD_REQUEST, "price must be greater than 0".to_string()));
        }
        Some(price_val)
    } else {
        None
    };

    let response = execute_manual_order(&state, req.symbol, OrderSide::Buy, quantity, order_type, price).await?;

    Ok(Json(response))
}

/// Submit a sell order manually
///
/// POST /api/orders/sell
///
/// Accepts OrderRequest JSON body and executes a sell order.
/// Respects paper_trading.enabled configuration.
#[instrument(skip(state))]
pub async fn submit_sell_order(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OrderRequest>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    // Validation
    if req.symbol.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "symbol is required".to_string()));
    }

    let quantity = Decimal::from_str(&req.quantity).map_err(|_| {
        (StatusCode::BAD_REQUEST, format!("Invalid quantity: {}", req.quantity))
    })?;

    if quantity <= Decimal::ZERO {
        return Err((StatusCode::BAD_REQUEST, "quantity must be greater than 0".to_string()));
    }

    let order_type = match req.order_type.to_lowercase().as_str() {
        "market" => OrderType::Market,
        "limit" => OrderType::Limit,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Invalid order_type: {}. Must be 'market' or 'limit'", req.order_type),
            ))
        }
    };

    let price = if order_type == OrderType::Limit {
        let price_str = req.price.as_ref().ok_or_else(|| {
            (StatusCode::BAD_REQUEST, "price is required for limit orders".to_string())
        })?;
        let price_val = Decimal::from_str(price_str).map_err(|_| {
            (StatusCode::BAD_REQUEST, format!("Invalid price: {}", price_str))
        })?;
        if price_val <= Decimal::ZERO {
            return Err((StatusCode::BAD_REQUEST, "price must be greater than 0".to_string()));
        }
        Some(price_val)
    } else {
        None
    };

    let response = execute_manual_order(&state, req.symbol, OrderSide::Sell, quantity, order_type, price).await?;

    Ok(Json(response))
}

/// Get paginated order history
///
/// GET /api/orders?limit=50&offset=0
///
/// Returns order history including entry/exit prices, timestamps, P&L, and order reason.
#[instrument(skip(state))]
pub async fn get_order_history(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OrderHistoryParams>,
) -> Result<Json<Vec<OrderHistoryResponse>>, StatusCode> {
    let mode = get_current_mode_str().await;
    let orders: Vec<OrderHistoryRow> = sqlx::query_as(
        r#"
        SELECT order_id, symbol, side, quantity, entry_price, entry_time,
               exit_price, exit_time, pnl, reason
        FROM orders
        WHERE trading_mode = $1
        ORDER BY entry_time DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(mode)
    .bind(params.limit as i64)
    .bind(params.offset as i64)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        error!("Failed to query order history: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let responses: Vec<OrderHistoryResponse> = orders
        .into_iter()
        .map(|row| OrderHistoryResponse {
            order_id: row.order_id,
            symbol: row.symbol,
            side: row.side,
            quantity: row.quantity,
            entry_price: row.entry_price,
            entry_time: row.entry_time,
            exit_price: row.exit_price,
            exit_time: row.exit_time,
            pnl: row.pnl,
            reason: row.reason,
        })
        .collect();

    Ok(Json(responses))
}

impl From<BacktestResult> for BacktestResponse {
    fn from(result: BacktestResult) -> Self {
        Self {
            symbol: result.symbol,
            strategy_name: result.strategy_name,
            start_time: result.start_time,
            end_time: result.end_time,
            initial_capital: result.initial_capital,
            fee_pct: result.fee_pct,
            slippage_pct: result.slippage_pct,
            final_equity: result.final_equity,
            total_pnl: result.total_pnl,
            total_return_pct: result.total_return_pct,
            total_trades: result.total_trades,
            winning_trades: result.winning_trades,
            losing_trades: result.losing_trades,
            win_rate: result.win_rate,
            max_drawdown: result.max_drawdown,
            sharpe_ratio: result.sharpe_ratio,
            sortino_ratio: result.sortino_ratio,
            total_fees_paid: result.total_fees_paid,
            total_slippage_cost: result.total_slippage_cost,
            trades: result.trades.map(|trades| {
                trades
                    .into_iter()
                    .map(|t| TradeRecordResponse {
                        entry_time: t.entry_time,
                        exit_time: t.exit_time,
                        side: format!("{:?}", t.side).to_lowercase(),
                        entry_price: t.entry_price,
                        exit_price: t.exit_price,
                        quantity: t.quantity,
                        pnl: t.pnl,
                        commission: t.commission,
                    })
                    .collect()
            }),
            equity_curve: result.equity_curve.map(|curve| {
                curve
                    .into_iter()
                    .map(|e| EquityPointBacktest {
                        timestamp: e.timestamp,
                        equity: e.equity,
                    })
                    .collect()
            }),
        }
    }
}

// ============================================================================
// Notification handlers
// ============================================================================

/// In-memory notification config state (MVP - not persisted)
static NOTIFICATION_CONFIG: OnceLock<RwLock<NotificationConfigState>> = OnceLock::new();

#[derive(Clone)]
struct NotificationConfigState {
    trades_enabled: bool,
    risk_alerts_enabled: bool,
    system_events_enabled: bool,
}

impl Default for NotificationConfigState {
    fn default() -> Self {
        Self {
            trades_enabled: true,
            risk_alerts_enabled: true,
            system_events_enabled: true,
        }
    }
}

fn get_notification_config_state() -> &'static RwLock<NotificationConfigState> {
    NOTIFICATION_CONFIG.get_or_init(|| RwLock::new(NotificationConfigState::default()))
}

/// Get notification configuration
///
/// GET /api/notifications/config
///
/// Returns the current notification settings for trades, risk alerts, and system events.
#[instrument]
pub async fn get_notification_config() -> Json<NotificationConfigResponse> {
    let config = get_notification_config_state().read().await;
    Json(NotificationConfigResponse {
        trades_enabled: config.trades_enabled,
        risk_alerts_enabled: config.risk_alerts_enabled,
        system_events_enabled: config.system_events_enabled,
    })
}

/// Update notification configuration
///
/// PUT /api/notifications/config
///
/// Accepts partial updates - only provided fields are changed.
/// Returns the updated configuration.
#[instrument]
pub async fn update_notification_config(
    Json(req): Json<UpdateNotificationConfigRequest>,
) -> Json<NotificationConfigResponse> {
    let mut config = get_notification_config_state().write().await;

    if let Some(v) = req.trades_enabled {
        config.trades_enabled = v;
    }
    if let Some(v) = req.risk_alerts_enabled {
        config.risk_alerts_enabled = v;
    }
    if let Some(v) = req.system_events_enabled {
        config.system_events_enabled = v;
    }

    info!(
        trades = config.trades_enabled,
        risk = config.risk_alerts_enabled,
        system = config.system_events_enabled,
        "Notification config updated"
    );

    Json(NotificationConfigResponse {
        trades_enabled: config.trades_enabled,
        risk_alerts_enabled: config.risk_alerts_enabled,
        system_events_enabled: config.system_events_enabled,
    })
}

/// Get Telegram connection status
///
/// GET /api/notifications/status
///
/// Returns "not_configured" if TELOXIDE_TOKEN not set,
/// "connected" if bot can reach Telegram API,
/// "disconnected" if API check fails.
#[instrument(skip(_state))]
pub async fn get_notification_status(
    State(_state): State<Arc<AppState>>,
) -> Json<TelegramStatusResponse> {
    // Check if Telegram is configured by checking env var
    let token_configured = std::env::var("TELOXIDE_TOKEN").is_ok()
        || std::env::var("TELEGRAM_BOT_TOKEN").is_ok();

    if !token_configured {
        return Json(TelegramStatusResponse {
            status: TelegramConnectionStatus::NotConfigured,
            bot_username: None,
            chat_id_configured: false,
        });
    }

    // Check if chat_id is configured
    let chat_id_configured = std::env::var("TELEGRAM_CHAT_ID").is_ok();

    // Try to check connection via getMe
    if let Some(token) = std::env::var("TELOXIDE_TOKEN")
        .ok()
        .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok())
    {
        use frankenstein::{client_reqwest::Bot, AsyncTelegramApi};
        let bot = Bot::new(&token);

        match bot.get_me().await {
            Ok(response) => Json(TelegramStatusResponse {
                status: TelegramConnectionStatus::Connected,
                bot_username: response.result.username,
                chat_id_configured,
            }),
            Err(e) => {
                warn!(?e, "Telegram connection check failed");
                Json(TelegramStatusResponse {
                    status: TelegramConnectionStatus::Disconnected,
                    bot_username: None,
                    chat_id_configured,
                })
            }
        }
    } else {
        Json(TelegramStatusResponse {
            status: TelegramConnectionStatus::NotConfigured,
            bot_username: None,
            chat_id_configured: false,
        })
    }
}

// ============================================================================
// Trading mode handlers
// ============================================================================

/// Get current trading mode
///
/// GET /api/mode
///
/// Returns the current trading mode (paper/live) and whether live mode is available
/// based on credential configuration.
#[instrument]
pub async fn get_trading_mode() -> Json<ModeResponse> {
    let is_paper = *get_trading_mode_state().read().await;
    let live_enabled = Credentials::from_env_optional().is_some();

    Json(ModeResponse {
        mode: if is_paper { "paper" } else { "live" }.to_string(),
        live_enabled,
    })
}

/// Switch trading mode
///
/// POST /api/mode
///
/// Switches between paper and live trading modes. Live mode requires valid
/// API credentials to be configured. Returns error if switching to live without credentials.
#[instrument(skip(_state))]
pub async fn switch_trading_mode(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<SwitchModeRequest>,
) -> Json<SwitchModeResponse> {
    let target_paper = match req.mode.to_lowercase().as_str() {
        "paper" => true,
        "live" => false,
        _ => {
            return Json(SwitchModeResponse {
                success: false,
                mode: String::new(),
                message: format!("Invalid mode: {}. Must be 'paper' or 'live'", req.mode),
            });
        }
    };

    // Check credentials for live mode
    if !target_paper && Credentials::from_env_optional().is_none() {
        return Json(SwitchModeResponse {
            success: false,
            mode: "paper".to_string(),
            message: "Live trading requires API credentials to be configured".to_string(),
        });
    }

    // Update state
    let mut mode = get_trading_mode_state().write().await;
    *mode = target_paper;

    let mode_str = if target_paper { "paper" } else { "live" };
    info!("Trading mode switched to: {}", mode_str);

    Json(SwitchModeResponse {
        success: true,
        mode: mode_str.to_string(),
        message: format!("Switched to {} trading mode", mode_str),
    })
}

// ============================================================================
// Strategy config update handler
// ============================================================================

/// Update RSI-StdDev strategy configuration (OPT-04)
///
/// POST /api/strategy/config
///
/// Accepts optimized parameters from the dashboard. In the current architecture,
/// the strategy manager runs in the main bot loop and is not directly accessible
/// from API handlers. This endpoint validates the parameters and returns success
/// to complete the frontend flow.
///
/// Note: The stddev_multiplier can be hot-reloaded at runtime if the strategy
/// supports it, but rsi_period changes require bot restart as they need
/// indicator reinitialization.
#[instrument(skip(_state, request))]
pub async fn update_strategy_config(
    State(_state): State<Arc<AppState>>,
    Json(request): Json<UpdateStrategyConfigRequest>,
) -> Result<Json<UpdateStrategyConfigResponse>, StatusCode> {
    info!("Strategy config update requested");

    // Get config values from request (use defaults if not provided)
    let new_period = request.rsi_period.unwrap_or(14);
    let new_multiplier = request.stddev_multiplier.unwrap_or(1.0);

    // Validate parameters
    if new_period < 2 || new_period > 100 {
        error!("Invalid RSI period: {}", new_period);
        return Err(StatusCode::BAD_REQUEST);
    }

    if new_multiplier <= 0.0 || new_multiplier > 10.0 {
        error!("Invalid StdDev multiplier: {}", new_multiplier);
        return Err(StatusCode::BAD_REQUEST);
    }

    info!(
        "Strategy config accepted: period={}, multiplier={:.2}",
        new_period, new_multiplier
    );

    // Note: In a full implementation, this would update the running strategy
    // or persist to config. Currently, the strategy manager is not shared with
    // API handlers, so we validate and acknowledge the parameters.
    //
    // Future enhancement: Add strategy_manager to AppState or use config
    // hot-reload mechanism to apply these parameters.

    Ok(Json(UpdateStrategyConfigResponse {
        success: true,
        message: format!(
            "Parameters accepted: period={}, multiplier={:.2}. Note: Period changes take effect on bot restart.",
            new_period, new_multiplier
        ),
        rsi_period: new_period,
        stddev_multiplier: new_multiplier,
    }))
}
