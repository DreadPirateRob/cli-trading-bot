//! Strategy manager for routing ticks to the active strategy.
//!
//! The StrategyManager holds multiple strategies but only one is active at a time.
//! Ticks are routed only to the active strategy, and strategies can be switched
//! at runtime.

use std::collections::HashMap;

use super::{Signal, Strategy, StrategyTick};

/// Manager for multiple trading strategies with one active at a time.
///
/// The StrategyManager fulfills STRT-04: only one strategy is active,
/// with the ability to switch between registered strategies at runtime.
pub struct StrategyManager {
    /// Registered strategies indexed by name.
    strategies: HashMap<String, Box<dyn Strategy>>,
    /// Name of the currently active strategy.
    active_strategy: String,
}

impl StrategyManager {
    /// Create a new StrategyManager with the specified active strategy name.
    ///
    /// # Arguments
    ///
    /// * `active` - Name of the strategy that should be active initially
    ///
    /// # Note
    ///
    /// The active strategy name is set but no strategy is registered yet.
    /// Call `register()` to add strategies.
    pub fn new(active: impl Into<String>) -> Self {
        Self {
            strategies: HashMap::new(),
            active_strategy: active.into(),
        }
    }

    /// Register a strategy with the manager.
    ///
    /// The strategy's `name()` method is used as the registration key.
    pub fn register(&mut self, strategy: Box<dyn Strategy>) {
        let name = strategy.name().to_string();
        self.strategies.insert(name, strategy);
    }

    /// Process a tick through the active strategy.
    ///
    /// Routes the tick only to the currently active strategy.
    ///
    /// # Returns
    ///
    /// The signal from the active strategy, or `Signal::Hold` if the
    /// active strategy is not registered (graceful degradation).
    pub fn on_tick(&mut self, tick: &StrategyTick) -> Signal {
        self.strategies
            .get_mut(&self.active_strategy)
            .map(|s| s.on_tick(tick))
            .unwrap_or(Signal::Hold)
    }

    /// Switch to a different strategy.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the strategy to switch to
    ///
    /// # Returns
    ///
    /// Ok(()) if the strategy exists and switch was successful,
    /// Err with message if the strategy is not registered.
    pub fn switch_strategy(&mut self, name: &str) -> Result<(), String> {
        if self.strategies.contains_key(name) {
            self.active_strategy = name.to_string();
            Ok(())
        } else {
            Err(format!("Unknown strategy: {}", name))
        }
    }

    /// Get the name of the currently active strategy.
    pub fn active(&self) -> &str {
        &self.active_strategy
    }

    /// Get the state of the active strategy.
    ///
    /// # Returns
    ///
    /// JSON state of the active strategy, or None if not registered.
    pub fn get_active_state(&self) -> Option<serde_json::Value> {
        self.strategies.get(&self.active_strategy).map(|s| s.get_state())
    }

    /// Update the configuration of the active strategy.
    ///
    /// # Arguments
    ///
    /// * `config` - JSON configuration to apply
    ///
    /// # Returns
    ///
    /// Ok(()) if configuration was applied successfully,
    /// Err if active strategy not found or config invalid.
    pub fn update_active_config(&mut self, config: &serde_json::Value) -> Result<(), String> {
        self.strategies
            .get_mut(&self.active_strategy)
            .ok_or_else(|| format!("Active strategy {} not found", self.active_strategy))?
            .update_config(config)
    }

    /// List all registered strategy names.
    pub fn list_strategies(&self) -> Vec<&str> {
        self.strategies.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    /// Mock strategy for testing the manager.
    struct MockStrategy {
        name: &'static str,
        signal: Signal,
    }

    impl Strategy for MockStrategy {
        fn on_tick(&mut self, _tick: &StrategyTick) -> Signal {
            self.signal.clone()
        }

        fn update_config(&mut self, _config: &serde_json::Value) -> Result<(), String> {
            Ok(())
        }

        fn get_state(&self) -> serde_json::Value {
            json!({"name": self.name})
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    fn sample_tick() -> StrategyTick {
        StrategyTick {
            symbol: "BTCUSDT".to_string(),
            price: dec!(50000),
            quantity: dec!(1),
            timestamp: 1234567890000,
        }
    }

    #[test]
    fn test_manager_routes_to_active() {
        let mut manager = StrategyManager::new("strategy-a");
        manager.register(Box::new(MockStrategy {
            name: "strategy-a",
            signal: Signal::Buy { quantity: dec!(1) },
        }));
        manager.register(Box::new(MockStrategy {
            name: "strategy-b",
            signal: Signal::Sell { quantity: dec!(1) },
        }));

        let signal = manager.on_tick(&sample_tick());
        assert_eq!(signal, Signal::Buy { quantity: dec!(1) });
    }

    #[test]
    fn test_manager_switch_strategy() {
        let mut manager = StrategyManager::new("strategy-a");
        manager.register(Box::new(MockStrategy {
            name: "strategy-a",
            signal: Signal::Buy { quantity: dec!(1) },
        }));
        manager.register(Box::new(MockStrategy {
            name: "strategy-b",
            signal: Signal::Sell { quantity: dec!(1) },
        }));

        // Switch to strategy-b
        manager.switch_strategy("strategy-b").unwrap();
        let signal = manager.on_tick(&sample_tick());
        assert_eq!(signal, Signal::Sell { quantity: dec!(1) });
    }

    #[test]
    fn test_manager_switch_unknown_fails() {
        let mut manager = StrategyManager::new("strategy-a");
        let result = manager.switch_strategy("unknown");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown strategy"));
    }

    #[test]
    fn test_manager_hold_when_no_strategy() {
        let mut manager = StrategyManager::new("nonexistent");
        let signal = manager.on_tick(&sample_tick());
        assert_eq!(signal, Signal::Hold);
    }

    #[test]
    fn test_manager_list_strategies() {
        let mut manager = StrategyManager::new("a");
        manager.register(Box::new(MockStrategy {
            name: "a",
            signal: Signal::Hold,
        }));
        manager.register(Box::new(MockStrategy {
            name: "b",
            signal: Signal::Hold,
        }));

        let list = manager.list_strategies();
        assert!(list.contains(&"a"));
        assert!(list.contains(&"b"));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_manager_get_active_state() {
        let mut manager = StrategyManager::new("test");
        manager.register(Box::new(MockStrategy {
            name: "test",
            signal: Signal::Hold,
        }));

        let state = manager.get_active_state().unwrap();
        assert_eq!(state["name"], "test");
    }

    #[test]
    fn test_manager_get_active_state_missing() {
        let manager = StrategyManager::new("nonexistent");
        let state = manager.get_active_state();
        assert!(state.is_none());
    }

    #[test]
    fn test_manager_update_active_config() {
        let mut manager = StrategyManager::new("test");
        manager.register(Box::new(MockStrategy {
            name: "test",
            signal: Signal::Hold,
        }));

        let result = manager.update_active_config(&json!({"param": "value"}));
        assert!(result.is_ok());
    }

    #[test]
    fn test_manager_update_active_config_missing() {
        let mut manager = StrategyManager::new("nonexistent");
        let result = manager.update_active_config(&json!({"param": "value"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_manager_active_name() {
        let manager = StrategyManager::new("my-strategy");
        assert_eq!(manager.active(), "my-strategy");
    }

    #[test]
    fn test_manager_active_name_after_switch() {
        let mut manager = StrategyManager::new("a");
        manager.register(Box::new(MockStrategy {
            name: "a",
            signal: Signal::Hold,
        }));
        manager.register(Box::new(MockStrategy {
            name: "b",
            signal: Signal::Hold,
        }));

        assert_eq!(manager.active(), "a");
        manager.switch_strategy("b").unwrap();
        assert_eq!(manager.active(), "b");
    }
}
