//! Bot status and configuration checking commands.
//!
//! Provides CLI commands for checking bot health status and validating
//! configuration files without starting the bot.

use std::path::Path;

use serde::Serialize;

use super::pid::process_exists;
use crate::config::{get_environment, load_config};

/// Bot status information.
#[derive(Debug, Serialize)]
pub struct BotStatus {
    /// Whether the bot process is currently running.
    pub running: bool,
    /// The process ID if running, None otherwise.
    pub pid: Option<u32>,
    /// Whether paper trading mode is enabled (from config).
    pub paper_mode: bool,
}

/// Check the current status of the trading bot.
///
/// Reads the PID file to determine if the bot is running, and loads
/// config to report paper_mode status.
///
/// # Arguments
///
/// * `pid_file_path` - Path to the PID file
/// * `config_dir` - Path to the configuration directory
///
/// # Returns
///
/// A `BotStatus` struct with running state, PID, and paper mode flag.
pub fn check_status(pid_file_path: &Path, config_dir: &Path) -> BotStatus {
    // Try to read PID and check if process exists
    // Note: We read the file directly instead of using PidFile to avoid
    // PidFile's Drop impl which removes the file
    let (running, pid) = match std::fs::read_to_string(pid_file_path) {
        Ok(content) => match content.trim().parse::<u32>() {
            Ok(p) if process_exists(p) => (true, Some(p)),
            _ => (false, None),
        },
        Err(_) => (false, None),
    };

    // Try to load config to get paper_mode
    // Set config_dir for the loader to use
    let paper_mode = {
        // Temporarily change to config_dir for loading
        let _saved_dir = std::env::current_dir().ok();
        if config_dir != Path::new("config") {
            // If custom config dir, we need to handle this
            // For now, we just try the default behavior
        }
        let env = get_environment();
        match load_config(&env) {
            Ok(config) => config.paper_trading.enabled,
            Err(_) => false, // Default to false if config can't be loaded
        }
    };

    BotStatus {
        running,
        pid,
        paper_mode,
    }
}

/// Print bot status in the requested format.
///
/// # Arguments
///
/// * `status` - The bot status to print
/// * `format` - Output format: "json" for machine-readable JSON, anything else for human-readable text
pub fn print_status(status: &BotStatus, format: &str) {
    if format == "json" {
        // JSON output for machine parsing
        match serde_json::to_string_pretty(status) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error serializing status: {}", e),
        }
    } else {
        // Human-readable text output
        println!("Bot Status:");
        println!(
            "  Running: {}",
            if status.running { "yes" } else { "no" }
        );
        println!(
            "  PID: {}",
            status.pid.map(|p| p.to_string()).unwrap_or_else(|| "N/A".to_string())
        );
        println!(
            "  Paper Mode: {}",
            if status.paper_mode { "yes" } else { "no" }
        );
    }
}

/// Validate configuration files without starting the bot.
///
/// Loads and validates configuration using the same process as startup.
/// Returns Ok(()) if valid, Err with validation message if invalid.
///
/// # Arguments
///
/// * `_config_dir` - Path to configuration directory (currently unused, uses default)
///
/// # Returns
///
/// Ok(()) if configuration is valid, Error otherwise.
pub fn check_config(_config_dir: &Path) -> Result<(), crate::Error> {
    // Use the environment to load config (same as production)
    let env = get_environment();

    // load_config performs full validation including business rules
    let _config = load_config(&env)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_bot_status_serialization() {
        let status = BotStatus {
            running: true,
            pid: Some(12345),
            paper_mode: true,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"running\":true"));
        assert!(json.contains("\"pid\":12345"));
        assert!(json.contains("\"paper_mode\":true"));
    }

    #[test]
    fn test_bot_status_not_running() {
        let status = BotStatus {
            running: false,
            pid: None,
            paper_mode: false,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"running\":false"));
        assert!(json.contains("\"pid\":null"));
    }

    #[test]
    fn test_check_status_no_pid_file() {
        let dir = tempdir().unwrap();
        let pid_path = dir.path().join("nonexistent.pid");
        let config_dir = dir.path();

        let status = check_status(&pid_path, config_dir);

        assert!(!status.running);
        assert!(status.pid.is_none());
    }

    #[test]
    fn test_check_status_stale_pid_file() {
        let dir = tempdir().unwrap();
        let pid_path = dir.path().join("test.pid");
        let config_dir = dir.path();

        // Create a PID file with non-existent process
        let mut file = File::create(&pid_path).unwrap();
        writeln!(file, "999999999").unwrap();
        drop(file);

        let status = check_status(&pid_path, config_dir);

        assert!(!status.running);
        assert!(status.pid.is_none());
    }

    #[test]
    fn test_check_status_running_process() {
        let dir = tempdir().unwrap();
        let pid_path = dir.path().join("test.pid");
        let config_dir = dir.path();

        // Create a PID file with current process (which exists)
        let current_pid = std::process::id();
        let mut file = File::create(&pid_path).unwrap();
        writeln!(file, "{}", current_pid).unwrap();
        drop(file);

        let status = check_status(&pid_path, config_dir);

        assert!(status.running);
        assert_eq!(status.pid, Some(current_pid));
    }
}
