//! Per-strategy risk configuration types
//!
//! Provides:
//! - `StrategyRiskOverrides`: Optional overrides for strategy-specific risk params
//! - `StrategyRiskParams`: Resolved risk params after merging with global defaults
//! - `GlobalRiskConfig`: Decimal versions of global config for merging

use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Per-strategy risk parameter overrides
///
/// All fields are optional. When None, the global default is used.
/// This allows strategies to selectively override specific parameters.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StrategyRiskOverrides {
    /// Override for max position size (% of portfolio)
    #[serde(default)]
    pub max_position_size_pct: Option<Decimal>,

    /// Override for stop-loss (% from entry)
    #[serde(default)]
    pub stop_loss_pct: Option<Decimal>,

    /// Override for daily loss limit (% of day-start equity)
    #[serde(default)]
    pub daily_loss_limit_pct: Option<Decimal>,

    /// Override for max drawdown circuit breaker (% from peak)
    #[serde(default)]
    pub max_drawdown_pct: Option<Decimal>,
}

/// Resolved risk parameters for a specific strategy
///
/// Created by merging strategy-specific overrides with global defaults.
/// All fields are guaranteed to have values (no Options).
#[derive(Debug, Clone)]
pub struct StrategyRiskParams {
    pub max_position_size_pct: Decimal,
    pub stop_loss_pct: Decimal,
    pub daily_loss_limit_pct: Decimal,
    pub max_drawdown_pct: Decimal,
}

/// Global risk configuration (matches src/config/types.rs RiskConfig)
///
/// This struct provides Decimal versions of the global config for merging.
#[derive(Debug, Clone)]
pub struct GlobalRiskConfig {
    pub max_position_size_pct: Decimal,
    pub stop_loss_pct: Decimal,
    pub daily_loss_limit_pct: Decimal,
    pub max_drawdown_pct: Decimal,
}

impl GlobalRiskConfig {
    /// Convert from config::types::RiskConfig (which uses f64)
    pub fn from_config(config: &crate::config::RiskConfig) -> Self {
        use rust_decimal::prelude::FromPrimitive;

        Self {
            max_position_size_pct: Decimal::from_f64(config.max_position_size_pct)
                .unwrap_or(Decimal::new(10, 2)), // 0.10 default
            stop_loss_pct: Decimal::from_f64(config.stop_loss_pct)
                .unwrap_or(Decimal::new(2, 2)), // 0.02 default
            daily_loss_limit_pct: Decimal::from_f64(config.daily_loss_limit_pct)
                .unwrap_or(Decimal::new(5, 2)), // 0.05 default
            max_drawdown_pct: Decimal::from_f64(config.max_drawdown_pct)
                .unwrap_or(Decimal::new(15, 2)), // 0.15 default
        }
    }
}

impl StrategyRiskParams {
    /// Create new risk parameters with the given percentages
    ///
    /// All values should be decimals (e.g., 0.10 for 10%)
    pub fn new(
        max_position_size_pct: Decimal,
        stop_loss_pct: Decimal,
        daily_loss_limit_pct: Decimal,
        max_drawdown_pct: Decimal,
    ) -> Self {
        Self {
            max_position_size_pct,
            stop_loss_pct,
            daily_loss_limit_pct,
            max_drawdown_pct,
        }
    }

    /// Merge strategy-specific overrides with global defaults
    ///
    /// For each parameter:
    /// - If strategy override exists (Some), use it
    /// - Otherwise, use global default
    pub fn from_config(global: &GlobalRiskConfig, overrides: &StrategyRiskOverrides) -> Self {
        Self {
            max_position_size_pct: overrides
                .max_position_size_pct
                .unwrap_or(global.max_position_size_pct),
            stop_loss_pct: overrides.stop_loss_pct.unwrap_or(global.stop_loss_pct),
            daily_loss_limit_pct: overrides
                .daily_loss_limit_pct
                .unwrap_or(global.daily_loss_limit_pct),
            max_drawdown_pct: overrides
                .max_drawdown_pct
                .unwrap_or(global.max_drawdown_pct),
        }
    }

    /// Create from global config with no overrides
    pub fn from_global(global: &GlobalRiskConfig) -> Self {
        Self::from_config(global, &StrategyRiskOverrides::default())
    }
}

impl Default for StrategyRiskParams {
    fn default() -> Self {
        Self {
            max_position_size_pct: Decimal::new(10, 2),  // 0.10
            stop_loss_pct: Decimal::new(2, 2),           // 0.02
            daily_loss_limit_pct: Decimal::new(5, 2),    // 0.05
            max_drawdown_pct: Decimal::new(15, 2),       // 0.15
        }
    }
}

// ============================================================================
// Mode-specific risk configuration storage
// ============================================================================

/// Mode-specific risk configuration storage
///
/// Keeps paper and live configs fully independent. Both modes start with
/// the same defaults for realistic simulation, but can be independently
/// modified via API endpoints.
#[derive(Debug, Clone)]
pub struct ModeRiskConfigs {
    configs: Arc<RwLock<HashMap<String, GlobalRiskConfig>>>,
}

impl ModeRiskConfigs {
    /// Create new storage with default configs for both modes
    ///
    /// Both modes start with same defaults for realistic simulation.
    pub fn new(default_config: GlobalRiskConfig) -> Self {
        let mut configs = HashMap::new();
        configs.insert("paper".to_string(), default_config.clone());
        configs.insert("live".to_string(), default_config);
        Self {
            configs: Arc::new(RwLock::new(configs)),
        }
    }

    /// Get risk config for a specific mode
    pub async fn get(&self, mode: &str) -> Option<GlobalRiskConfig> {
        let configs = self.configs.read().await;
        configs.get(mode).cloned()
    }

    /// Update risk config for a specific mode
    pub async fn update(&self, mode: &str, config: GlobalRiskConfig) {
        let mut configs = self.configs.write().await;
        configs.insert(mode.to_string(), config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_merge_with_all_overrides() {
        let global = GlobalRiskConfig {
            max_position_size_pct: dec!(0.10),
            stop_loss_pct: dec!(0.02),
            daily_loss_limit_pct: dec!(0.05),
            max_drawdown_pct: dec!(0.15),
        };

        let overrides = StrategyRiskOverrides {
            max_position_size_pct: Some(dec!(0.05)),
            stop_loss_pct: Some(dec!(0.01)),
            daily_loss_limit_pct: Some(dec!(0.03)),
            max_drawdown_pct: Some(dec!(0.10)),
        };

        let params = StrategyRiskParams::from_config(&global, &overrides);

        // All should use overrides
        assert_eq!(params.max_position_size_pct, dec!(0.05));
        assert_eq!(params.stop_loss_pct, dec!(0.01));
        assert_eq!(params.daily_loss_limit_pct, dec!(0.03));
        assert_eq!(params.max_drawdown_pct, dec!(0.10));
    }

    #[test]
    fn test_merge_with_no_overrides() {
        let global = GlobalRiskConfig {
            max_position_size_pct: dec!(0.10),
            stop_loss_pct: dec!(0.02),
            daily_loss_limit_pct: dec!(0.05),
            max_drawdown_pct: dec!(0.15),
        };

        let overrides = StrategyRiskOverrides::default();

        let params = StrategyRiskParams::from_config(&global, &overrides);

        // All should use global defaults
        assert_eq!(params.max_position_size_pct, dec!(0.10));
        assert_eq!(params.stop_loss_pct, dec!(0.02));
        assert_eq!(params.daily_loss_limit_pct, dec!(0.05));
        assert_eq!(params.max_drawdown_pct, dec!(0.15));
    }

    #[test]
    fn test_merge_with_partial_overrides() {
        let global = GlobalRiskConfig {
            max_position_size_pct: dec!(0.10),
            stop_loss_pct: dec!(0.02),
            daily_loss_limit_pct: dec!(0.05),
            max_drawdown_pct: dec!(0.15),
        };

        // Only override position size and stop loss
        let overrides = StrategyRiskOverrides {
            max_position_size_pct: Some(dec!(0.05)),
            stop_loss_pct: Some(dec!(0.015)),
            daily_loss_limit_pct: None,
            max_drawdown_pct: None,
        };

        let params = StrategyRiskParams::from_config(&global, &overrides);

        // Position size and stop loss use overrides
        assert_eq!(params.max_position_size_pct, dec!(0.05));
        assert_eq!(params.stop_loss_pct, dec!(0.015));

        // Daily loss and drawdown use global
        assert_eq!(params.daily_loss_limit_pct, dec!(0.05));
        assert_eq!(params.max_drawdown_pct, dec!(0.15));
    }

    #[test]
    fn test_from_global_shortcut() {
        let global = GlobalRiskConfig {
            max_position_size_pct: dec!(0.10),
            stop_loss_pct: dec!(0.02),
            daily_loss_limit_pct: dec!(0.05),
            max_drawdown_pct: dec!(0.15),
        };

        let params = StrategyRiskParams::from_global(&global);

        assert_eq!(params.max_position_size_pct, dec!(0.10));
        assert_eq!(params.stop_loss_pct, dec!(0.02));
        assert_eq!(params.daily_loss_limit_pct, dec!(0.05));
        assert_eq!(params.max_drawdown_pct, dec!(0.15));
    }

    #[test]
    fn test_default_strategy_risk_params() {
        let params = StrategyRiskParams::default();

        assert_eq!(params.max_position_size_pct, dec!(0.10));
        assert_eq!(params.stop_loss_pct, dec!(0.02));
        assert_eq!(params.daily_loss_limit_pct, dec!(0.05));
        assert_eq!(params.max_drawdown_pct, dec!(0.15));
    }

    #[test]
    fn test_strategy_risk_overrides_deserialize() {
        // Test that StrategyRiskOverrides deserializes from JSON
        let json = r#"{
            "max_position_size_pct": "0.05",
            "stop_loss_pct": "0.01"
        }"#;

        let overrides: StrategyRiskOverrides = serde_json::from_str(json).unwrap();

        assert_eq!(overrides.max_position_size_pct, Some(dec!(0.05)));
        assert_eq!(overrides.stop_loss_pct, Some(dec!(0.01)));
        assert!(overrides.daily_loss_limit_pct.is_none());
        assert!(overrides.max_drawdown_pct.is_none());
    }

    #[test]
    fn test_strategy_risk_params_new() {
        let params = StrategyRiskParams::new(dec!(0.15), dec!(0.03), dec!(0.06), dec!(0.25));
        assert_eq!(params.max_position_size_pct, dec!(0.15));
        assert_eq!(params.stop_loss_pct, dec!(0.03));
        assert_eq!(params.daily_loss_limit_pct, dec!(0.06));
        assert_eq!(params.max_drawdown_pct, dec!(0.25));
    }
}
