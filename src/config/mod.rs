//! Configuration management for the trading bot.
//!
//! Provides:
//! - Strongly-typed config structs with validation
//! - Layered loading from YAML files + environment variables
//! - Hot-reload via ArcSwap pattern
//! - File watching for automatic reloads
//! - Override source abstraction for database integration

mod types;
mod loader;
mod validation;
mod manager;
mod watcher;
mod override_source;

pub use types::*;
pub use loader::{load_config, get_environment};
pub use manager::ConfigManager;
pub use watcher::{ConfigWatcher, WatcherError};
pub use override_source::{OverrideSource, OverrideError, ConfigValue, NoOpOverrides};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Test that example configs pass validation
    #[test]
    fn test_example_configs_valid() {
        // This test requires the example files to exist
        // Skip if they don't (e.g., in CI without full checkout)
        if !std::path::Path::new("config/exchange.example.yaml").exists() {
            return;
        }
        // Also skip if database config doesn't exist yet
        if !std::path::Path::new("config/database.example.yaml").exists() {
            return;
        }

        // Copy example files to temp dir as actual configs
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();

        fs::copy("config/exchange.example.yaml", config_dir.join("exchange.yaml")).unwrap();
        fs::copy("config/strategy.example.yaml", config_dir.join("strategy.yaml")).unwrap();
        fs::copy("config/logging.example.yaml", config_dir.join("logging.yaml")).unwrap();
        fs::copy("config/database.example.yaml", config_dir.join("database.yaml")).unwrap();

        // Change to temp dir to load config
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        let result = load_config("dev");

        std::env::set_current_dir(original_dir).unwrap();

        assert!(result.is_ok(), "Example configs should be valid: {:?}", result.err());
    }

    /// Test that validation catches invalid values
    #[test]
    fn test_validation_rejects_invalid() {
        use super::validation::validate_config;
        use super::types::*;

        let invalid_config = AppConfig {
            exchange: ExchangeConfig {
                name: "".to_string(), // Invalid: empty
                api_url: "not-a-url".to_string(), // Invalid: not a URL
                ws_url: "wss://valid.url".to_string(),
                timeout_ms: 50, // Invalid: below minimum
                rate_limits: RateLimitConfig {
                    requests_per_second: 10,
                    burst_size: 20,
                },
                symbol: "btcusdt".to_string(),
            },
            strategy: StrategyConfig {
                active: "test".to_string(),
                risk: RiskConfig {
                    max_position_size_pct: 0.1,
                    stop_loss_pct: 0.02,
                    daily_loss_limit_pct: 0.05,
                    max_drawdown_pct: 0.15,
                },
                rsi_stddev: None,
                grid: None,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                directory: "./logs".to_string(),
                retention_days: 30,
                stdout: true,
                json_format: true,
            },
            database: DatabaseConfig {
                url: "postgres://localhost/test".to_string(),
                max_connections: 5,
                min_connections: 1,
                acquire_timeout_secs: 3,
            },
            paper_trading: PaperTradingConfig::default(),
            execution: ExecutionConfig::default(),
            backtest: BacktestSettings::default(),
        };

        let result = validate_config(&invalid_config);
        assert!(result.is_err(), "Invalid config should fail validation");
    }
}
