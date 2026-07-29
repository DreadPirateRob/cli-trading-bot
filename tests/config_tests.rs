use trading_bot::config::load_config;
use validator::Validate;

#[test]
fn test_load_config_success() {
    // Load config using layered sources
    let config = load_config("dev").expect("Failed to load config");

    // Verify exchange config
    assert_eq!(config.exchange.name, "binance_us");
    assert_eq!(config.exchange.api_url, "https://api.binance.us");
    assert_eq!(config.exchange.timeout_ms, 5000);
    assert_eq!(config.exchange.rate_limits.requests_per_second, 10);

    // Verify strategy config
    assert_eq!(config.strategy.active, "rsi_stddev");
    assert!((config.strategy.risk.max_position_size_pct - 0.1).abs() < f64::EPSILON);
    assert!((config.strategy.risk.stop_loss_pct - 0.02).abs() < f64::EPSILON);

    // Verify logging config
    assert_eq!(config.logging.level, "info");
    assert_eq!(config.logging.retention_days, 30);
    assert!(config.logging.stdout);
}

#[test]
fn test_config_passes_validation() {
    let config = load_config("dev").expect("Failed to load config");

    // Struct-level validation should pass
    config.validate().expect("Config validation failed");

    // Business rule validation should pass
    config
        .strategy
        .validate_business_rules()
        .expect("Business rules validation failed");
}

#[test]
fn test_invalid_stop_loss_business_rule() {
    // Create a config where stop_loss_pct >= max_position_size_pct
    // This should fail business rule validation
    use trading_bot::config::{RiskConfig, StrategyConfig};

    let strategy = StrategyConfig {
        active: "test".to_string(),
        risk: RiskConfig {
            max_position_size_pct: 0.1,
            stop_loss_pct: 0.15, // Greater than max_position_size_pct
            daily_loss_limit_pct: 0.05,
            max_drawdown_pct: 0.15,
        },
        rsi_stddev: None,
        grid: None,
    };

    let result = strategy.validate_business_rules();
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("stop_loss_pct"));
}
