//! Abstraction for configuration override sources.
//!
//! The base config is loaded from YAML files. Override sources can
//! provide additional values that take precedence over file config.
//! This enables database-driven config overrides (CONF-02).

use async_trait::async_trait;
use std::collections::HashMap;

/// A source that can provide configuration overrides.
///
/// Override sources are queried after loading base config from files.
/// Values from override sources take precedence over file values.
///
/// # Implementing
/// Implement this trait to add new override sources (database, remote config service, etc.)
///
/// # Example
/// ```ignore
/// struct DatabaseOverrides {
///     pool: PgPool,
/// }
///
/// #[async_trait]
/// impl OverrideSource for DatabaseOverrides {
///     async fn get_overrides(&self, keys: &[&str]) -> Result<HashMap<String, ConfigValue>, OverrideError> {
///         // Query database for overrides
///     }
/// }
/// ```
#[async_trait]
pub trait OverrideSource: Send + Sync {
    /// Get override values for the specified config keys.
    ///
    /// # Arguments
    /// * `keys` - Dot-separated config paths (e.g., "strategy.risk.stop_loss_pct")
    ///
    /// # Returns
    /// Map of key -> value for any overrides found. Keys not in the map
    /// will use their file-based values.
    async fn get_overrides(&self, keys: &[&str]) -> Result<HashMap<String, ConfigValue>, OverrideError>;

    /// Check if this source is available/healthy.
    async fn health_check(&self) -> Result<(), OverrideError>;

    /// Human-readable name for logging.
    fn name(&self) -> &str;
}

/// A configuration value from an override source.
#[derive(Debug, Clone)]
pub enum ConfigValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
}

impl ConfigValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ConfigValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ConfigValue::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ConfigValue::Float(f) => Some(*f),
            ConfigValue::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConfigValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }
}

/// Errors from override sources
#[derive(Debug, thiserror::Error)]
pub enum OverrideError {
    #[error("Override source '{name}' unavailable: {reason}")]
    Unavailable { name: String, reason: String },

    #[error("Failed to fetch overrides from '{name}': {reason}")]
    FetchFailed { name: String, reason: String },

    #[error("Invalid override value for '{key}': {reason}")]
    InvalidValue { key: String, reason: String },
}

/// A no-op override source that returns no overrides.
/// Used as default when no database or other source is configured.
pub struct NoOpOverrides;

#[async_trait]
impl OverrideSource for NoOpOverrides {
    async fn get_overrides(&self, _keys: &[&str]) -> Result<HashMap<String, ConfigValue>, OverrideError> {
        Ok(HashMap::new())
    }

    async fn health_check(&self) -> Result<(), OverrideError> {
        Ok(())
    }

    fn name(&self) -> &str {
        "none"
    }
}
