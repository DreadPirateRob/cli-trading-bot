//! Configuration manager with lock-free hot-reload support.
//!
//! Uses arc-swap for wait-free reads and atomic updates.
//! Validation happens before any update to ensure consistency.

use std::sync::Arc;
use arc_swap::ArcSwap;
use crate::error::ConfigError;
use super::types::AppConfig;
use super::loader::load_config;
use super::validation::validate_config;

/// Thread-safe configuration manager with hot-reload support.
///
/// Uses arc-swap for lock-free reads - extremely fast with no contention.
/// Updates are atomic - readers see either old or new config, never partial.
pub struct ConfigManager {
    config: ArcSwap<AppConfig>,
    environment: String,
}

impl ConfigManager {
    /// Create a new ConfigManager with the initial configuration.
    pub fn new(initial_config: AppConfig, environment: String) -> Self {
        Self {
            config: ArcSwap::from(Arc::new(initial_config)),
            environment,
        }
    }

    /// Get the current configuration.
    ///
    /// This is a lock-free operation - extremely fast with no contention.
    /// Returns an Arc to the current config that remains valid even if
    /// the config is updated after this call.
    #[inline]
    pub fn get(&self) -> Arc<AppConfig> {
        self.config.load_full()
    }

    /// Reload configuration from files.
    ///
    /// Validates the new configuration before applying. If validation fails,
    /// the old configuration is retained and an error is returned.
    ///
    /// # Returns
    /// - `Ok(())` if reload succeeded
    /// - `Err(ConfigError)` if loading or validation failed (old config retained)
    pub fn reload(&self) -> Result<(), ConfigError> {
        tracing::info!("Reloading configuration...");

        // Load new config from files
        let new_config = match load_config(&self.environment) {
            Ok(config) => config,
            Err(e) => {
                tracing::warn!(error = %e, "Config reload failed: load error, keeping old config");
                return Err(e);
            }
        };

        // Validate new config
        if let Err(e) = validate_config(&new_config) {
            tracing::warn!(error = %e, "Config reload failed: validation error, keeping old config");
            return Err(e);
        }

        // Atomic swap - readers see old or new, never partial
        let old = self.config.swap(Arc::new(new_config));

        tracing::info!(
            old_strategy = %old.strategy.active,
            new_strategy = %self.get().strategy.active,
            "Configuration reloaded successfully"
        );

        Ok(())
    }

    /// Get the current environment name.
    pub fn environment(&self) -> &str {
        &self.environment
    }
}

impl std::fmt::Debug for ConfigManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigManager")
            .field("environment", &self.environment)
            .field("strategy", &self.get().strategy.active)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests will be added when we have test fixtures
}
