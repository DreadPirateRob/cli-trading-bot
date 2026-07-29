use thiserror::Error;

use crate::config::WatcherError;
use crate::exchange::ExchangeError;
use crate::logging::LoggingError;

/// Application-wide error type
#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Config watcher error: {0}")]
    Watcher(#[from] WatcherError),

    #[error("Logging error: {0}")]
    Logging(#[from] LoggingError),

    #[error("Exchange error: {0}")]
    Exchange(#[from] ExchangeError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Risk engine error: {0}")]
    Risk(String),

    #[error("Execution error: {0}")]
    Execution(String),
}

/// Configuration-specific errors with actionable messages
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to load config file '{path}': {source}")]
    LoadFailed {
        path: String,
        #[source]
        source: config::ConfigError,
    },

    #[error("Invalid config at '{field}': {message}")]
    Validation { field: String, message: String },

    #[error("Missing required field '{field}' in {file}")]
    MissingField { field: String, file: String },

    #[error("Invalid value for '{field}': expected {expected}, got {actual}")]
    InvalidValue {
        field: String,
        expected: String,
        actual: String,
    },
}

/// Convenience type alias
pub type Result<T> = std::result::Result<T, Error>;
