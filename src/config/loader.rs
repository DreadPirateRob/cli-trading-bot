use config::{Config, Environment, File};
use crate::error::ConfigError;
use super::types::AppConfig;
use super::validation::validate_config;

/// Load configuration from layered sources.
/// Priority: defaults < files < env-specific files < environment variables
pub fn load_config(env: &str) -> Result<AppConfig, ConfigError> {
    let config = Config::builder()
        // Base config files
        .add_source(File::with_name("config/exchange").required(true))
        .add_source(File::with_name("config/strategy").required(true))
        .add_source(File::with_name("config/logging").required(true))
        .add_source(File::with_name("config/database").required(true))
        // Environment-specific overrides (optional)
        .add_source(File::with_name(&format!("config/exchange.{}", env)).required(false))
        .add_source(File::with_name(&format!("config/strategy.{}", env)).required(false))
        .add_source(File::with_name(&format!("config/logging.{}", env)).required(false))
        .add_source(File::with_name(&format!("config/database.{}", env)).required(false))
        // Environment variables override everything
        // Use double underscore for nesting: APP__DATABASE__URL
        .add_source(
            Environment::with_prefix("APP")
                .separator("__")
                .try_parsing(true)
        )
        .build()
        .map_err(|e| ConfigError::LoadFailed {
            path: "config/".to_string(),
            source: e,
        })?;

    // Deserialize with path tracking for error messages
    let app_config: AppConfig = config.try_deserialize().map_err(|e| {
        ConfigError::LoadFailed {
            path: "config/".to_string(),
            source: e,
        }
    })?;

    // Validate using two-layer approach
    validate_config(&app_config)?;

    Ok(app_config)
}

/// Get the current environment (defaults to "dev")
pub fn get_environment() -> String {
    std::env::var("APP_ENV").unwrap_or_else(|_| "dev".to_string())
}
