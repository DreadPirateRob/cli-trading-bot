pub mod alerts;
pub mod api;
pub mod backtest;
pub mod cli;
pub mod config;
pub mod data;
pub mod error;
pub mod exchange;
pub mod execution;
pub mod logging;
pub mod notifications;
pub mod risk;
pub mod strategy;
pub mod tui;

pub use cli::{Cli, Commands};
pub use error::{Error, Result};

use std::sync::Arc;
use std::path::Path;
use config::{load_config, get_environment, ConfigManager, ConfigWatcher};
use logging::{setup_logging, LogConfig};

/// Main entry point for the trading bot.
/// Returns Ok(()) on clean shutdown, Err on fatal error.
#[tokio::main]
pub async fn run() -> Result<()> {
    // Load configuration first (fail fast if invalid)
    let env = get_environment();
    let initial_config = load_config(&env)?;

    // Initialize logging with config values
    let log_config = LogConfig {
        level: initial_config.logging.level.clone(),
        directory: initial_config.logging.directory.clone(),
        retention_days: initial_config.logging.retention_days,
        stdout: initial_config.logging.stdout,
        json_format: initial_config.logging.json_format,
    };

    let _log_guard = setup_logging(&log_config)?;

    tracing::info!(
        environment = %env,
        exchange = %initial_config.exchange.name,
        strategy = %initial_config.strategy.active,
        "Trading bot initialized"
    );

    // Create config manager for hot-reload
    let config_manager = Arc::new(ConfigManager::new(initial_config, env));

    // Start file watcher for automatic config reload
    let _watcher = ConfigWatcher::start(
        Path::new("config"),
        Arc::clone(&config_manager),
        500, // 500ms debounce
    )?;

    // Example: read config without blocking
    let config = config_manager.get();
    tracing::info!(
        active_strategy = %config.strategy.active,
        max_position = %config.strategy.risk.max_position_size_pct,
        "Current configuration loaded"
    );

    // TODO: Main loop will be implemented in later phases
    // For now, just demonstrate config works
    tracing::info!("Trading bot shutting down");

    Ok(())
}
