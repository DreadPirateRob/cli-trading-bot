//! Start and stop command implementations for the trading bot.
//!
//! Provides graceful shutdown handling using CancellationToken and signal handlers
//! for SIGTERM (Docker) and SIGINT (Ctrl+C).

use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use arc_swap::ArcSwap;
use chrono::Utc;
use nix::sys::signal::Signal;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal_macros::dec;
use tokio::signal;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::alerts::{BotSnapshot, TelegramConfig, run_telegram_bot};
use crate::api::state::AppState;
use crate::api::types::{BotUpdate, PriceUpdate, TradeUpdate};
use crate::api::create_router;
use crate::cli::pid::{self, process_exists, send_signal, PidFile};
use crate::config::{get_environment, load_config};
use crate::data::{create_pool, start_db_writer, NewTick, WriterConfig};
use crate::exchange::{start_reconnecting_stream, Credentials, ReconnectConfig};
use crate::execution::{start_order_executor, ExecutionResult, ExecutorStartConfig};
use crate::logging::{setup_logging, LogConfig};
use crate::notifications::{AlertBroadcaster, AlertEvent, SystemEventType};
use crate::risk::{start_risk_engine, GlobalRiskConfig, RiskEngineConfig, StopLossEvent};
use crate::strategy::{
    RsiStdDevConfig, RsiStdDevStrategy, Signal as StrategySignal, StrategyManager, StrategyTick,
};

/// Start the trading bot with graceful shutdown support.
///
/// Creates a PID file, sets up signal handlers, and runs the bot until
/// a shutdown signal is received (SIGTERM or SIGINT).
///
/// # Arguments
///
/// * `config_dir` - Path to the configuration directory
/// * `_foreground` - Whether to run in foreground mode (reserved for future daemonization)
/// * `pid_file_path` - Path to the PID file
///
/// # Errors
///
/// Returns an error if the bot is already running or if initialization fails.
pub async fn run_start(
    config_dir: &Path,
    _foreground: bool,
    pid_file_path: &Path,
) -> crate::Result<()> {
    // Create PID file (fail if already running)
    let pid_file = PidFile::new(pid_file_path);
    pid_file.create().map_err(|e| crate::Error::Execution(e.to_string()))?;

    // Set up graceful shutdown using CancellationToken
    let shutdown_token = CancellationToken::new();
    let signal_token = shutdown_token.clone();

    // Spawn signal handler task
    tokio::spawn(async move {
        let ctrl_c = signal::ctrl_c();

        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {
                tracing::info!("Received SIGINT (Ctrl+C)");
            }
            _ = terminate => {
                tracing::info!("Received SIGTERM");
            }
        }

        tracing::info!("Shutdown signal received");
        signal_token.cancel();
    });

    // Run the bot
    let result = run_bot(config_dir, shutdown_token).await;

    // PID file drops automatically here, removing the file
    drop(pid_file);

    result
}

/// Internal bot execution with shutdown coordination.
async fn run_bot(config_dir: &Path, shutdown_token: CancellationToken) -> crate::Result<()> {
    // Load configuration using environment
    let env = get_environment();
    let config = load_config(&env)?;

    // Note: config_dir parameter is currently unused as load_config uses hardcoded "config" path
    let _ = config_dir;

    // Set up logging with configuration
    let log_config = LogConfig {
        level: config.logging.level.clone(),
        directory: config.logging.directory.clone(),
        retention_days: config.logging.retention_days,
        stdout: config.logging.stdout,
        json_format: config.logging.json_format,
    };

    let _log_guard = setup_logging(&log_config)?;

    // Create alert broadcaster for notification routing
    let (alert_sender, mut alert_broadcaster) = AlertBroadcaster::with_default_capacity();

    // Create shared Telegram bot state (for command responses)
    let tg_state = Arc::new(RwLock::new(BotSnapshot::default()));

    // Subscribe Telegram if configured
    if let Some(tg_config) = TelegramConfig::from_env() {
        let tg_rx = alert_broadcaster.subscribe(50);
        let tg_state_clone = tg_state.clone();
        let tg_shutdown = shutdown_token.clone();
        tokio::spawn(async move {
            run_telegram_bot(tg_config, tg_state_clone, tg_rx, tg_shutdown).await;
        });
        tracing::info!("Telegram notifications enabled");
    } else {
        tracing::info!("Telegram not configured (set TELOXIDE_TOKEN to enable)");
    }

    // Spawn the alert broadcaster task
    tokio::spawn(async move {
        alert_broadcaster.run().await;
    });

    // Create database pool
    let pool = create_pool(&config.database).await
        .map_err(|e| crate::Error::Execution(format!("Database: {}", e)))?;

    // Create stop-loss channel (connects Risk -> Executor)
    let (stop_loss_tx, stop_loss_rx) = mpsc::channel::<StopLossEvent>(100);

    // Create risk engine with config from strategy.risk
    let initial_equity = config.paper_trading.initial_balances
        .get("USDT")
        .copied()
        .unwrap_or(10000.0);

    let risk_config = RiskEngineConfig {
        initial_equity: Decimal::from_f64(initial_equity).unwrap_or(Decimal::new(10000, 0)),
        global_risk: GlobalRiskConfig {
            max_position_size_pct: Decimal::from_f64(config.strategy.risk.max_position_size_pct)
                .unwrap_or(Decimal::new(10, 2)),
            stop_loss_pct: Decimal::from_f64(config.strategy.risk.stop_loss_pct)
                .unwrap_or(Decimal::new(2, 2)),
            daily_loss_limit_pct: Decimal::from_f64(config.strategy.risk.daily_loss_limit_pct)
                .unwrap_or(Decimal::new(5, 2)),
            max_drawdown_pct: Decimal::from_f64(config.strategy.risk.max_drawdown_pct)
                .unwrap_or(Decimal::new(15, 2)),
        },
        channel_capacity: 1000,
    };

    let risk_handle = start_risk_engine(risk_config, Some(stop_loss_tx), Some(alert_sender.clone()));
    tracing::info!("Risk engine started");

    // Create order executor with risk handle
    let credentials = if !config.paper_trading.enabled {
        Some(Credentials::from_env()
            .map_err(|e| crate::Error::Execution(format!("Credentials: {}", e)))?)
    } else {
        None
    };

    let executor_config = ExecutorStartConfig {
        execution_config: config.execution.clone(),
        paper_config: config.paper_trading.clone(),
        credentials,
    };

    let executor_handle = start_order_executor(
        executor_config,
        risk_handle.clone(),
        stop_loss_rx,
        pool.clone(),
        Some(alert_sender.clone()),
    );
    tracing::info!(
        paper_mode = config.paper_trading.enabled,
        "Order executor started"
    );

    // Create strategy manager and register strategies from config
    let mut strategy_manager = StrategyManager::new(&config.strategy.active);

    // Register RSI-StdDev if configured
    // NOTE: Must convert config types (u32 -> usize, add base_quantity, stddev_period)
    if let Some(ref rsi_cfg) = config.strategy.rsi_stddev {
        let strategy_config = RsiStdDevConfig {
            rsi_period: rsi_cfg.rsi_period as usize,
            rsi_oversold: rsi_cfg.rsi_oversold as f64,
            rsi_overbought: rsi_cfg.rsi_overbought as f64,
            stddev_period: 20,  // Default stddev period
            stddev_multiplier: rsi_cfg.stddev_multiplier,
            base_quantity: dec!(0.001),  // Default base quantity for BTC
        };
        let strategy = RsiStdDevStrategy::new(strategy_config);
        strategy_manager.register(Box::new(strategy));
        tracing::debug!("Registered rsi_stddev strategy");
    }

    // NOTE: GridStrategy requires center_price which is only available at runtime
    // (from first tick). Grid cannot be registered at startup - it will be registered
    // lazily when the first tick arrives in Plan 02's tick processing loop.
    // Log a warning if grid is configured but explain it will be activated later.
    if config.strategy.grid.is_some() {
        tracing::info!(
            "Grid strategy configured - will be activated on first tick (requires live price)"
        );
    }

    tracing::info!(
        active = %config.strategy.active,
        registered = ?strategy_manager.list_strategies(),
        "Strategy manager initialized"
    );

    // Create database writer for tick persistence
    let db_writer = start_db_writer(pool.clone(), WriterConfig::default());
    tracing::info!("Database writer started");

    // Start reconnecting WebSocket trade stream
    let mut trade_stream = start_reconnecting_stream(
        &config.exchange.symbol,
        Some(&config.exchange.ws_url),
        1000, // buffer size
        ReconnectConfig::unlimited(),
    ).await;
    tracing::info!(
        symbol = %config.exchange.symbol,
        "WebSocket trade stream started"
    );

    // Create AppState for dashboard WebSocket broadcasts
    let config_arc = Arc::new(ArcSwap::from_pointee(config.clone()));
    let app_state = AppState::new(pool.clone(), config_arc, Some(risk_handle.clone()), None);
    tracing::info!("Dashboard AppState initialized");

    // Start API server for dashboard
    let api_state = Arc::new(app_state.clone());
    let api_shutdown = shutdown_token.clone();
    tokio::spawn(async move {
        let router = create_router(api_state);
        let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
            .await
            .expect("Failed to bind API server to port 3000");
        tracing::info!("API server started on http://0.0.0.0:3000");

        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                api_shutdown.cancelled().await;
                tracing::info!("API server shutting down");
            })
            .await
            .expect("API server error");
    });

    tracing::info!(
        config_dir = %config_dir.display(),
        exchange = %config.exchange.name,
        strategy = %config.strategy.active,
        paper_mode = %config.paper_trading.enabled,
        "Trading bot components initialized"
    );

    // Emit bot started event
    let _ = alert_sender.try_send(AlertEvent::system(
        SystemEventType::BotStarted,
        "Trading bot started",
    ));

    // Spawn task to keep Telegram BotSnapshot updated periodically
    let tg_state_for_update = tg_state.clone();
    let update_shutdown = shutdown_token.clone();
    let paper_mode = config.paper_trading.enabled;
    let active_strategy = config.strategy.active.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        let mut uptime_seconds: u64 = 0;

        loop {
            tokio::select! {
                biased;
                _ = update_shutdown.cancelled() => break,
                _ = interval.tick() => {
                    uptime_seconds += 5;

                    // Update BotSnapshot with current state
                    // In a full implementation, this would read from app_state/metrics
                    let mut snapshot = tg_state_for_update.write().await;
                    snapshot.status = "running".to_string();
                    snapshot.strategy = active_strategy.clone();
                    snapshot.paper_mode = paper_mode;
                    snapshot.uptime_seconds = uptime_seconds;

                    // TODO: When AppState is available in run_bot, populate:
                    // - snapshot.equity from metrics
                    // - snapshot.unrealized_pnl from metrics
                    // - snapshot.daily_pnl from metrics
                    // - snapshot.recent_trades from trade history
                    // - snapshot.balances from balance manager
                }
            }
        }
    });

    // Track whether Grid strategy has been initialized (requires first tick for center_price)
    let mut grid_initialized = false;

    // Main tick processing loop
    loop {
        tokio::select! {
            biased; // Prioritize shutdown over tick processing

            _ = shutdown_token.cancelled() => {
                tracing::info!("Shutdown signal received, stopping tick processing");
                break;
            }

            Some(trade) = trade_stream.trades.recv() => {
                // Parse price (skip tick on parse error)
                let price = match trade.price.parse::<Decimal>() {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(error = %e, price = %trade.price, "Failed to parse price, skipping tick");
                        continue;
                    }
                };

                // Lazy-initialize Grid strategy on first tick (needs center_price)
                if !grid_initialized {
                    if let Some(ref grid_cfg) = config.strategy.grid {
                        use crate::strategy::{GridConfig as StrategyGridConfig, GridStrategy};
                        let strategy_config = StrategyGridConfig {
                            spacing_pct: Decimal::from_f64(grid_cfg.spacing_pct)
                                .unwrap_or(dec!(0.01)),
                            num_levels: grid_cfg.levels,
                            quantity_per_level: dec!(0.001),  // Default quantity
                            escape_multiplier: dec!(2.0),     // Default escape multiplier
                        };
                        let strategy = GridStrategy::new(strategy_config, price);
                        strategy_manager.register(Box::new(strategy));
                        tracing::info!(
                            center_price = %price,
                            levels = grid_cfg.levels,
                            "Grid strategy initialized with first tick price"
                        );
                    }
                    grid_initialized = true;
                }

                // 1. Persist tick to database (non-blocking, OK to fail)
                if let Ok(new_tick) = NewTick::try_from(&trade) {
                    let _ = db_writer.write_tick(new_tick);
                }

                // 2. Update risk engine for stop-loss monitoring
                if let Err(e) = risk_handle.price_update(&trade.symbol, price).await {
                    tracing::debug!(error = %e, "Failed to send price update to risk engine");
                }

                // 3. Create strategy tick and get signal
                let quantity = trade.quantity.parse::<Decimal>().unwrap_or(Decimal::ZERO);
                let tick = StrategyTick {
                    symbol: trade.symbol.clone(),
                    price,
                    quantity,
                    timestamp: trade.trade_time as i64,
                };
                let signal = strategy_manager.on_tick(&tick);

                // 4. Execute signal if not Hold
                if !matches!(signal, StrategySignal::Hold) {
                    match executor_handle.execute_signal(
                        strategy_manager.active(),
                        &tick.symbol,
                        signal,
                        price,
                    ).await {
                        Ok(ExecutionResult::Filled(response)) => {
                            // Broadcast trade to dashboard
                            app_state.broadcast(BotUpdate::Trade(TradeUpdate {
                                id: response.order_id.clone(),
                                symbol: response.symbol.clone(),
                                side: format!("{:?}", response.side).to_lowercase(),
                                quantity: response.filled_quantity,
                                price: response.fill_price,
                                pnl: None,
                            }));
                            tracing::info!(
                                order_id = %response.order_id,
                                symbol = %response.symbol,
                                side = ?response.side,
                                quantity = %response.filled_quantity,
                                price = %response.fill_price,
                                "Trade executed"
                            );
                        }
                        Ok(ExecutionResult::Rejected { reason }) => {
                            tracing::warn!(reason = %reason, "Signal rejected by risk/execution");
                        }
                        Ok(ExecutionResult::NoAction) => {}
                        Ok(ExecutionResult::Disabled) => {
                            tracing::debug!("Execution disabled, signal not executed");
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Signal execution failed");
                        }
                    }
                }

                // 5. Broadcast price to dashboard
                app_state.broadcast(BotUpdate::Price(PriceUpdate {
                    symbol: trade.symbol.clone(),
                    price,
                    timestamp: Utc::now(),
                }));
            }
        }
    }

    // === Graceful Shutdown Sequence ===
    // Shutdown in reverse startup order to ensure clean resource release

    // 1. Stop receiving new ticks
    tracing::info!("Shutting down WebSocket connection");
    trade_stream.shutdown().await;

    // 2. Flush and stop database writer (persists any buffered ticks)
    tracing::info!("Flushing database writer");
    if let Err(e) = db_writer.shutdown().await {
        tracing::warn!(error = %e, "Database writer shutdown error");
    }

    // 3. Shutdown order executor (completes any in-flight orders)
    tracing::info!("Shutting down order executor");
    if let Err(e) = executor_handle.shutdown().await {
        tracing::warn!(error = %e, "Order executor shutdown error");
    }

    // 4. Shutdown risk engine
    tracing::info!("Shutting down risk engine");
    if let Err(e) = risk_handle.shutdown().await {
        tracing::warn!(error = %e, "Risk engine shutdown error");
    }

    // Emit shutdown event
    let _ = alert_sender.try_send(AlertEvent::system(
        SystemEventType::BotStopped,
        "Trading bot stopped (shutdown complete)",
    ));

    tracing::info!("Shutdown complete");
    Ok(())
}

/// Stop a running trading bot by sending SIGTERM.
///
/// Reads the PID from the PID file, signals the process, and waits for it
/// to exit gracefully. If the process doesn't exit within 30 seconds,
/// SIGKILL is sent.
///
/// # Arguments
///
/// * `pid_file_path` - Path to the PID file
///
/// # Errors
///
/// Returns an error if the bot is not running or if signaling fails.
pub fn stop_bot(pid_file_path: &Path) -> Result<(), pid::Error> {
    let pid_file = PidFile::new(pid_file_path);

    // Read PID from file
    let pid = match pid_file.read_pid() {
        Ok(pid) => pid,
        Err(pid::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(pid::Error::NotRunning("PID file not found".to_string()));
        }
        Err(e) => return Err(e),
    };

    // Check if process exists
    if !process_exists(pid) {
        // Clean up stale PID file
        let _ = pid_file.remove();
        return Err(pid::Error::NotRunning(format!(
            "Process {} not running, removed stale PID file",
            pid
        )));
    }

    println!("Stopping trading bot (PID: {})...", pid);

    // Send SIGTERM for graceful shutdown
    send_signal(pid, Signal::SIGTERM)
        .map_err(|e| pid::Error::Io(e))?;

    // Wait for process to exit with timeout
    const TIMEOUT_SECS: u64 = 30;
    const POLL_INTERVAL_MS: u64 = 100;

    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(TIMEOUT_SECS);

    while process_exists(pid) {
        if start.elapsed() > timeout {
            println!("Process did not exit gracefully, sending SIGKILL...");
            send_signal(pid, Signal::SIGKILL)
                .map_err(|e| pid::Error::Io(e))?;
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            break;
        }
        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }

    // Clean up PID file if still exists
    let _ = pid_file.remove();

    println!("Trading bot stopped successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_stop_not_running() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.pid");

        let result = stop_bot(&path);
        assert!(matches!(result, Err(pid::Error::NotRunning(_))));
    }

    #[test]
    fn test_stop_stale_pid_file() {
        use std::fs::File;
        use std::io::Write;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.pid");

        // Create PID file with non-existent PID
        let mut file = File::create(&path).unwrap();
        writeln!(file, "999999999").unwrap();
        drop(file);

        let result = stop_bot(&path);
        assert!(matches!(result, Err(pid::Error::NotRunning(_))));

        // PID file should be cleaned up
        assert!(!path.exists());
    }
}
