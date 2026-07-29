//! Logging setup with JSON file output and pretty stdout output.
//!
//! Uses tracing ecosystem for structured, span-based logging.

use std::path::Path;
use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

/// Guard that must be kept alive to ensure logs are flushed.
/// Dropping this will stop file logging.
pub struct LogGuard {
    _file_guard: tracing_appender::non_blocking::WorkerGuard,
}

/// Configuration for logging setup
pub struct LogConfig {
    pub level: String,
    pub directory: String,
    pub retention_days: u32,
    pub stdout: bool,
    pub json_format: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            directory: "./logs".to_string(),
            retention_days: 30,
            stdout: true,
            json_format: true,
        }
    }
}

/// Initialize the logging system.
///
/// Returns a guard that MUST be kept alive for the duration of the program.
/// Dropping the guard will stop file logging.
///
/// # Arguments
/// * `config` - Logging configuration
///
/// # Example
/// ```no_run
/// use trading_bot::logging::{setup_logging, LogConfig};
/// let _guard = setup_logging(&LogConfig::default()).expect("logging failed");
/// tracing::info!("Bot started");
/// ```
pub fn setup_logging(config: &LogConfig) -> Result<LogGuard, LoggingError> {
    // Ensure log directory exists
    let log_dir = Path::new(&config.directory);
    std::fs::create_dir_all(log_dir).map_err(|e| LoggingError::DirectoryCreation {
        path: config.directory.clone(),
        source: e,
    })?;

    // File appender with daily rotation
    // Note: tracing-appender doesn't support max_log_files directly,
    // we'd need a cleanup task for retention. For now, rotation only.
    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("trading-bot")
        .filename_suffix("log")
        .build(log_dir)
        .map_err(|e| LoggingError::AppenderCreation(e.to_string()))?;

    // Non-blocking writer to avoid blocking on file I/O
    let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);

    // Parse log level with fallback
    let _level_filter = config.level.parse::<Level>().unwrap_or(Level::INFO);

    // Build the subscriber based on configuration
    let registry = tracing_subscriber::registry();

    // Environment filter allows per-module levels via RUST_LOG
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.level));

    if config.json_format {
        // JSON format for file (machine-readable)
        let file_layer = fmt::layer()
            .json()
            .with_writer(non_blocking)
            .with_span_events(FmtSpan::CLOSE)
            .with_current_span(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true);

        if config.stdout {
            // Pretty format for stdout (human-readable)
            let stdout_layer = fmt::layer()
                .pretty()
                .with_target(true)
                .with_thread_ids(false)
                .with_file(false);

            registry
                .with(env_filter)
                .with(file_layer)
                .with(stdout_layer)
                .init();
        } else {
            registry
                .with(env_filter)
                .with(file_layer)
                .init();
        }
    } else {
        // Plain text format for file
        let file_layer = fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false);

        if config.stdout {
            let stdout_layer = fmt::layer()
                .pretty()
                .with_target(true);

            registry
                .with(env_filter)
                .with(file_layer)
                .with(stdout_layer)
                .init();
        } else {
            registry
                .with(env_filter)
                .with(file_layer)
                .init();
        }
    }

    tracing::info!(
        level = %config.level,
        directory = %config.directory,
        json_format = %config.json_format,
        stdout = %config.stdout,
        "Logging initialized"
    );

    Ok(LogGuard {
        _file_guard: file_guard,
    })
}

/// Errors that can occur during logging setup
#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("Failed to create log directory '{path}': {source}")]
    DirectoryCreation {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to create file appender: {0}")]
    AppenderCreation(String),
}
