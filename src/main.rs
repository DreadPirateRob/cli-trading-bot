use std::path::PathBuf;

use clap::Parser;
use trading_bot::{cli::{run_start, stop_bot}, Cli, Commands};

fn main() {
    // Install rustls CryptoProvider before any TLS operations
    // Required for rustls 0.23+ when multiple crypto backends are available
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls CryptoProvider");

    // Parse CLI arguments
    let cli = Cli::parse();

    // Default PID file path
    let default_pid = PathBuf::from("/tmp/trading-bot.pid");

    // Print debug info in verbose mode
    if cli.verbose > 0 {
        eprintln!("Config dir: {:?}", cli.config_dir);
        eprintln!("Verbose level: {}", cli.verbose);
        eprintln!("Command: {:?}", cli.command);
    }

    // Handle commands that don't need async runtime
    match &cli.command {
        Commands::Stop { pid_file } => {
            if let Err(e) = stop_bot(pid_file) {
                eprintln!("Failed to stop bot: {e}");
                std::process::exit(1);
            }
            return;
        }
        Commands::CheckConfig => {
            if let Err(e) = trading_bot::cli::check_config(&cli.config_dir) {
                eprintln!("Configuration invalid: {e}");
                std::process::exit(1);
            }
            println!("Configuration valid");
            return;
        }
        Commands::Status { format } => {
            let status = trading_bot::cli::check_status(&default_pid, &cli.config_dir);
            trading_bot::cli::print_status(&status, format);
            return;
        }
        _ => {}
    }

    // Commands that need async runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");

    let result = runtime.block_on(async {
        match &cli.command {
            Commands::Start { foreground } => {
                run_start(&cli.config_dir, *foreground, &default_pid).await
            }
            Commands::Buy { symbol, quantity } => {
                trading_bot::cli::execute_buy(&cli.config_dir, symbol, quantity).await
            }
            Commands::Sell { symbol, quantity } => {
                trading_bot::cli::execute_sell(&cli.config_dir, symbol, quantity).await
            }
            Commands::Tui => {
                // TUI runs standalone with sample data
                if let Err(e) = trading_bot::tui::run_tui(None, None).await {
                    eprintln!("TUI error: {}", e);
                    std::process::exit(1);
                }
                Ok(())
            }
            _ => unreachable!(),
        }
    });

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
