//! CLI argument parsing for the trading bot.
//!
//! Uses clap 4.5 derive macros to define the CLI interface with subcommands
//! for daemon control, configuration validation, and manual trading operations.

pub mod pid;
pub mod run;
pub mod status;
pub mod trade;

use std::path::PathBuf;

pub use run::{run_start, stop_bot};
pub use status::{check_config, check_status, print_status};
pub use trade::{execute_buy, execute_sell};

use clap::{ArgAction, Parser, Subcommand};

/// Binance.US cryptocurrency trading bot
#[derive(Parser, Debug)]
#[command(name = "trading-bot")]
#[command(version, about = "Binance.US cryptocurrency trading bot", long_about = None)]
pub struct Cli {
    /// Path to configuration directory
    #[arg(short = 'c', long = "config-dir", default_value = "config")]
    pub config_dir: PathBuf,

    /// Increase logging verbosity (-v, -vv, -vvv)
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbose: u8,

    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands for the trading bot
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the trading bot daemon
    Start {
        /// Run in foreground mode (do not daemonize)
        #[arg(short = 'f', long = "foreground")]
        foreground: bool,
    },

    /// Stop a running trading bot daemon
    Stop {
        /// Path to PID file
        #[arg(long = "pid-file", default_value = "/tmp/trading-bot.pid")]
        pid_file: PathBuf,
    },

    /// Check the status of the trading bot daemon
    Status {
        /// Output format (text or json)
        #[arg(short = 'f', long = "format", default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },

    /// Validate configuration files without starting the bot
    CheckConfig,

    /// Execute a manual buy order
    Buy {
        /// Trading symbol (e.g., BTCUSDT)
        symbol: String,

        /// Quantity to buy
        quantity: String,
    },

    /// Execute a manual sell order
    Sell {
        /// Trading symbol (e.g., BTCUSDT)
        symbol: String,

        /// Quantity to sell
        quantity: String,
    },

    /// Launch terminal user interface
    Tui,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        // Ensure CLI definition is valid
        Cli::command().debug_assert();
    }

    #[test]
    fn parse_start_foreground() {
        let cli = Cli::parse_from(["trading-bot", "start", "-f"]);
        match cli.command {
            Commands::Start { foreground } => assert!(foreground),
            _ => panic!("Expected Start command"),
        }
    }

    #[test]
    fn parse_verbose_levels() {
        let cli = Cli::parse_from(["trading-bot", "-vvv", "start"]);
        assert_eq!(cli.verbose, 3);
    }

    #[test]
    fn parse_config_dir() {
        let cli = Cli::parse_from(["trading-bot", "-c", "/custom/config", "check-config"]);
        assert_eq!(cli.config_dir, PathBuf::from("/custom/config"));
    }

    #[test]
    fn parse_buy_command() {
        let cli = Cli::parse_from(["trading-bot", "buy", "BTCUSDT", "0.5"]);
        match cli.command {
            Commands::Buy { symbol, quantity } => {
                assert_eq!(symbol, "BTCUSDT");
                assert_eq!(quantity, "0.5");
            }
            _ => panic!("Expected Buy command"),
        }
    }

    #[test]
    fn parse_sell_command() {
        let cli = Cli::parse_from(["trading-bot", "sell", "ETHUSDT", "1.0"]);
        match cli.command {
            Commands::Sell { symbol, quantity } => {
                assert_eq!(symbol, "ETHUSDT");
                assert_eq!(quantity, "1.0");
            }
            _ => panic!("Expected Sell command"),
        }
    }

    #[test]
    fn parse_status_json_format() {
        let cli = Cli::parse_from(["trading-bot", "status", "-f", "json"]);
        match cli.command {
            Commands::Status { format } => assert_eq!(format, "json"),
            _ => panic!("Expected Status command"),
        }
    }

    #[test]
    fn parse_stop_custom_pid() {
        let cli = Cli::parse_from(["trading-bot", "stop", "--pid-file", "/var/run/bot.pid"]);
        match cli.command {
            Commands::Stop { pid_file } => {
                assert_eq!(pid_file, PathBuf::from("/var/run/bot.pid"));
            }
            _ => panic!("Expected Stop command"),
        }
    }
}
