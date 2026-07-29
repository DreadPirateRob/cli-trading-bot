use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::risk::config::StrategyRiskOverrides;

/// Root configuration combining all config files
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct AppConfig {
    #[validate(nested)]
    pub exchange: ExchangeConfig,

    #[validate(nested)]
    pub strategy: StrategyConfig,

    #[validate(nested)]
    pub logging: LoggingConfig,

    #[validate(nested)]
    pub database: DatabaseConfig,

    /// Paper trading configuration (optional, defaults to disabled)
    #[serde(default)]
    pub paper_trading: PaperTradingConfig,

    /// Execution configuration (optional, defaults enabled)
    #[serde(default)]
    pub execution: ExecutionConfig,

    /// Backtest settings (optional, defaults provided)
    #[serde(default)]
    pub backtest: BacktestSettings,
}

/// Exchange connection settings (exchange.yaml)
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct ExchangeConfig {
    #[validate(length(min = 1, message = "Exchange name cannot be empty"))]
    pub name: String,

    #[validate(url(message = "Must be a valid URL"))]
    pub api_url: String,

    #[validate(url(message = "Must be a valid WebSocket URL"))]
    pub ws_url: String,

    #[validate(range(min = 100, max = 60000, message = "Timeout must be 100-60000ms"))]
    pub timeout_ms: u32,

    #[validate(nested)]
    pub rate_limits: RateLimitConfig,

    /// Trading pair symbol (e.g., "btcusdt")
    #[serde(default = "default_symbol")]
    #[validate(length(min = 1, message = "Symbol cannot be empty"))]
    pub symbol: String,
}

fn default_symbol() -> String {
    "btcusdt".to_string()
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct RateLimitConfig {
    #[validate(range(min = 1, max = 1000, message = "Requests per second must be 1-1000"))]
    pub requests_per_second: u32,

    #[validate(range(min = 1, max = 100, message = "Burst size must be 1-100"))]
    pub burst_size: u32,
}

/// Strategy parameters (strategy.yaml)
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct StrategyConfig {
    #[validate(length(min = 1, message = "Active strategy name cannot be empty"))]
    pub active: String,

    #[validate(nested)]
    pub risk: RiskConfig,

    // Strategy-specific configs are optional, validated at runtime
    #[serde(default)]
    pub rsi_stddev: Option<RsiStdDevConfig>,

    #[serde(default)]
    pub grid: Option<GridConfig>,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct RiskConfig {
    #[validate(range(min = 0.001, max = 1.0, message = "Position size must be 0.1%-100%"))]
    pub max_position_size_pct: f64,

    #[validate(range(min = 0.001, max = 0.5, message = "Stop loss must be 0.1%-50%"))]
    pub stop_loss_pct: f64,

    #[validate(range(min = 0.001, max = 1.0, message = "Daily loss limit must be 0.1%-100%"))]
    pub daily_loss_limit_pct: f64,

    #[validate(range(min = 0.01, max = 1.0, message = "Max drawdown must be 1%-100%"))]
    pub max_drawdown_pct: f64,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct RsiStdDevConfig {
    #[validate(range(min = 2, max = 100, message = "RSI period must be 2-100"))]
    pub rsi_period: u32,

    #[validate(range(min = 0, max = 50, message = "RSI oversold must be 0-50"))]
    pub rsi_oversold: u32,

    #[validate(range(min = 50, max = 100, message = "RSI overbought must be 50-100"))]
    pub rsi_overbought: u32,

    #[validate(range(min = 0.1, max = 5.0, message = "StdDev multiplier must be 0.1-5.0"))]
    pub stddev_multiplier: f64,

    /// Strategy-specific risk overrides (optional)
    #[serde(default)]
    pub risk: Option<StrategyRiskOverrides>,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct GridConfig {
    #[validate(range(min = 2, max = 100, message = "Grid levels must be 2-100"))]
    pub levels: u32,

    #[validate(range(min = 0.001, max = 0.1, message = "Grid spacing must be 0.1%-10%"))]
    pub spacing_pct: f64,

    /// Strategy-specific risk overrides (optional)
    #[serde(default)]
    pub risk: Option<StrategyRiskOverrides>,
}

/// Logging configuration (logging.yaml)
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct LoggingConfig {
    #[validate(length(min = 1, message = "Log level cannot be empty"))]
    pub level: String,

    #[validate(length(min = 1, message = "Log directory cannot be empty"))]
    pub directory: String,

    #[validate(range(min = 1, max = 365, message = "Retention days must be 1-365"))]
    pub retention_days: u32,

    pub stdout: bool,
    pub json_format: bool,
}

/// Database connection settings (database.yaml)
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct DatabaseConfig {
    #[validate(url(message = "Must be a valid PostgreSQL URL"))]
    pub url: String,

    #[serde(default = "default_max_connections")]
    #[validate(range(min = 1, max = 100, message = "Max connections must be 1-100"))]
    pub max_connections: u32,

    #[serde(default = "default_min_connections")]
    #[validate(range(min = 0, max = 50, message = "Min connections must be 0-50"))]
    pub min_connections: u32,

    #[serde(default = "default_acquire_timeout_secs")]
    #[validate(range(min = 1, max = 60, message = "Acquire timeout must be 1-60 seconds"))]
    pub acquire_timeout_secs: u32,
}

fn default_max_connections() -> u32 { 5 }
fn default_min_connections() -> u32 { 1 }
fn default_acquire_timeout_secs() -> u32 { 3 }

/// Order execution configuration
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct ExecutionConfig {
    /// Enable execution (false = signals logged but not executed)
    #[serde(default = "default_execution_enabled")]
    pub enabled: bool,

    /// Channel capacity for execution messages
    #[serde(default = "default_execution_channel_capacity")]
    pub channel_capacity: usize,

    /// Marketable limit order offset for stop-loss (e.g., 0.05 = 5%)
    #[serde(default = "default_stop_loss_offset")]
    pub stop_loss_offset_pct: f64,

    /// Balance reconciliation interval in seconds (0 = disabled)
    #[serde(default = "default_reconciliation_interval")]
    pub reconciliation_interval_secs: u64,

    /// Reserve percentage of balance (e.g., 0.10 = 10% safety buffer)
    #[serde(default)]
    pub reserve_pct: Option<f64>,
}

fn default_execution_enabled() -> bool {
    true
}

fn default_execution_channel_capacity() -> usize {
    1000
}

fn default_stop_loss_offset() -> f64 {
    0.05
}

fn default_reconciliation_interval() -> u64 {
    300
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            enabled: default_execution_enabled(),
            channel_capacity: default_execution_channel_capacity(),
            stop_loss_offset_pct: default_stop_loss_offset(),
            reconciliation_interval_secs: default_reconciliation_interval(),
            reserve_pct: None,
        }
    }
}

/// Backtest configuration defaults
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestSettings {
    /// Default fee percentage (e.g., 0.001 for 0.1%)
    #[serde(default = "default_backtest_fee")]
    pub default_fee_pct: f64,

    /// Default slippage percentage (e.g., 0.0005 for 0.05%)
    #[serde(default = "default_backtest_slippage")]
    pub default_slippage_pct: f64,

    /// Default initial capital in quote currency
    #[serde(default = "default_initial_capital")]
    pub default_initial_capital: f64,

    /// Risk-free rate for Sharpe ratio calculation (annualized)
    #[serde(default = "default_risk_free_rate")]
    pub risk_free_rate: f64,
}

fn default_backtest_fee() -> f64 {
    0.001
} // 0.1%

fn default_backtest_slippage() -> f64 {
    0.0005
} // 0.05%

fn default_initial_capital() -> f64 {
    10000.0
} // $10k

fn default_risk_free_rate() -> f64 {
    0.02
} // 2%

impl Default for BacktestSettings {
    fn default() -> Self {
        Self {
            default_fee_pct: default_backtest_fee(),
            default_slippage_pct: default_backtest_slippage(),
            default_initial_capital: default_initial_capital(),
            risk_free_rate: default_risk_free_rate(),
        }
    }
}

/// Paper trading simulation configuration
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct PaperTradingConfig {
    /// Enable paper trading mode (no real orders)
    #[serde(default)]
    pub enabled: bool,

    /// Simulated latency range in milliseconds (min, max)
    #[serde(default = "default_latency_range")]
    pub latency_range_ms: (u64, u64),

    /// Slippage range as decimal (e.g., 0.001 = 0.1%)
    #[serde(default = "default_slippage_range")]
    pub slippage_range: (f64, f64),

    /// Trading fee percentage (e.g., 0.001 = 0.1%)
    #[serde(default = "default_fee_pct")]
    pub fee_pct: f64,

    /// Starting balances by asset
    #[serde(default = "default_paper_balances")]
    pub initial_balances: HashMap<String, f64>,
}

fn default_latency_range() -> (u64, u64) {
    (50, 200)
}

fn default_slippage_range() -> (f64, f64) {
    (0.002, 0.005) // 0.2% to 0.5% - matches PAPER-04 requirement
}

fn default_fee_pct() -> f64 {
    0.001
}

fn default_paper_balances() -> HashMap<String, f64> {
    let mut m = HashMap::new();
    m.insert("USDT".to_string(), 10000.0);
    m.insert("BTC".to_string(), 0.0);
    m
}

impl Default for PaperTradingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            latency_range_ms: default_latency_range(),
            slippage_range: default_slippage_range(),
            fee_pct: default_fee_pct(),
            initial_balances: default_paper_balances(),
        }
    }
}

impl StrategyConfig {
    /// Business rule validation: stop_loss_pct must be less than max_position_size_pct
    pub fn validate_business_rules(&self) -> Result<(), String> {
        if self.risk.stop_loss_pct >= self.risk.max_position_size_pct {
            return Err(format!(
                "risk.stop_loss_pct ({:.1}%) must be less than risk.max_position_size_pct ({:.1}%)",
                self.risk.stop_loss_pct * 100.0,
                self.risk.max_position_size_pct * 100.0
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_rsi_stddev_config_with_risk_yaml_deserialization() {
        let yaml = r#"
rsi_period: 14
rsi_oversold: 30
rsi_overbought: 70
stddev_multiplier: 2.0
risk:
  max_position_size_pct: "0.05"
  stop_loss_pct: "0.015"
"#;
        let config: RsiStdDevConfig = serde_yml::from_str(yaml).unwrap();

        assert_eq!(config.rsi_period, 14);
        assert_eq!(config.rsi_oversold, 30);
        assert_eq!(config.rsi_overbought, 70);
        assert!((config.stddev_multiplier - 2.0).abs() < f64::EPSILON);

        // Risk overrides should be present
        assert!(config.risk.is_some());
        let risk = config.risk.unwrap();
        assert_eq!(risk.max_position_size_pct, Some(dec!(0.05)));
        assert_eq!(risk.stop_loss_pct, Some(dec!(0.015)));
        assert!(risk.daily_loss_limit_pct.is_none());
        assert!(risk.max_drawdown_pct.is_none());
    }

    #[test]
    fn test_rsi_stddev_config_without_risk_yaml_deserialization() {
        let yaml = r#"
rsi_period: 14
rsi_oversold: 30
rsi_overbought: 70
stddev_multiplier: 2.0
"#;
        let config: RsiStdDevConfig = serde_yml::from_str(yaml).unwrap();

        assert_eq!(config.rsi_period, 14);
        // Risk overrides should be None
        assert!(config.risk.is_none());
    }

    #[test]
    fn test_grid_config_with_risk_yaml_deserialization() {
        let yaml = r#"
levels: 10
spacing_pct: 0.02
risk:
  max_position_size_pct: "0.02"
  max_drawdown_pct: "0.10"
"#;
        let config: GridConfig = serde_yml::from_str(yaml).unwrap();

        assert_eq!(config.levels, 10);
        assert!((config.spacing_pct - 0.02).abs() < f64::EPSILON);

        // Risk overrides should be present
        assert!(config.risk.is_some());
        let risk = config.risk.unwrap();
        assert_eq!(risk.max_position_size_pct, Some(dec!(0.02)));
        assert_eq!(risk.max_drawdown_pct, Some(dec!(0.10)));
        assert!(risk.stop_loss_pct.is_none());
        assert!(risk.daily_loss_limit_pct.is_none());
    }

    #[test]
    fn test_grid_config_without_risk_yaml_deserialization() {
        let yaml = r#"
levels: 10
spacing_pct: 0.02
"#;
        let config: GridConfig = serde_yml::from_str(yaml).unwrap();

        assert_eq!(config.levels, 10);
        // Risk overrides should be None
        assert!(config.risk.is_none());
    }

    #[test]
    fn test_default_slippage_range() {
        let config = PaperTradingConfig::default();
        // 0.2% to 0.5% as per PAPER-04 requirement
        assert_eq!(config.slippage_range, (0.002, 0.005));
    }
}
