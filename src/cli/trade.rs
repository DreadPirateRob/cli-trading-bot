//! Manual trade command implementations
//!
//! Provides CLI commands for executing one-off trades for testing exchange
//! connectivity and order flow. Respects paper_mode configuration for safe testing.

use std::path::Path;
use std::str::FromStr;

use rust_decimal::Decimal;
use tracing::info;

use crate::config::{get_environment, load_config};
use crate::data::OrderSide;
use crate::exchange::Credentials;
use crate::execution::{
    ExecuteOrderRequest, LiveExecutor, OrderExecutor, OrderType, PaperExecutor,
};

/// Execute a manual trade (buy or sell)
///
/// Loads configuration, creates appropriate executor based on paper_mode setting,
/// and executes the order.
async fn execute_manual_trade(
    _config_dir: &Path,
    symbol: &str,
    side: OrderSide,
    quantity_str: &str,
) -> crate::Result<()> {
    // Parse quantity
    let quantity = Decimal::from_str(quantity_str).map_err(|_| {
        crate::Error::Config(crate::error::ConfigError::Validation {
            field: "quantity".to_string(),
            message: format!("Invalid quantity: {}", quantity_str),
        })
    })?;

    // Load config
    let env = get_environment();
    let config = load_config(&env)?;

    // Determine paper mode
    let paper_mode = config.paper_trading.enabled;

    // Create appropriate executor
    let executor: Box<dyn OrderExecutor> = if paper_mode {
        Box::new(PaperExecutor::new(config.paper_trading.clone()))
    } else {
        // Live mode requires credentials from environment
        let credentials = Credentials::from_env().map_err(|e| {
            crate::Error::Config(crate::error::ConfigError::Validation {
                field: "credentials".to_string(),
                message: format!(
                    "Live trading requires API credentials: {}. Set BINANCE_API_KEY and BINANCE_API_SECRET environment variables.",
                    e
                ),
            })
        })?;
        Box::new(LiveExecutor::new(credentials))
    };

    // Create order request
    let request = ExecuteOrderRequest {
        symbol: symbol.to_uppercase(),
        side,
        quantity,
        order_type: OrderType::Market,
        price: None,
        client_order_id: Some(format!("manual-{}", uuid::Uuid::new_v4())),
    };

    info!(
        symbol = %request.symbol,
        side = ?side,
        quantity = %quantity,
        paper_mode = paper_mode,
        "Executing manual trade"
    );

    // Execute the order
    let response = executor.execute(request).await.map_err(|e| {
        crate::Error::Execution(format!("Order execution failed: {}", e))
    })?;

    // Print result to stdout
    println!("Order executed:");
    println!("  Order ID: {}", response.order_id);
    println!("  Symbol: {}", response.symbol);
    println!("  Side: {:?}", response.side);
    println!(
        "  Filled: {} @ {}",
        response.filled_quantity, response.fill_price
    );
    println!("  Status: {}", response.status);
    println!("  Paper Mode: {}", paper_mode);

    Ok(())
}

/// Execute a manual buy order
///
/// # Arguments
/// * `config_dir` - Path to configuration directory
/// * `symbol` - Trading symbol (e.g., "BTCUSDT")
/// * `quantity` - Quantity to buy as a string
pub async fn execute_buy(config_dir: &Path, symbol: &str, quantity: &str) -> crate::Result<()> {
    execute_manual_trade(config_dir, symbol, OrderSide::Buy, quantity).await
}

/// Execute a manual sell order
///
/// # Arguments
/// * `config_dir` - Path to configuration directory
/// * `symbol` - Trading symbol (e.g., "BTCUSDT")
/// * `quantity` - Quantity to sell as a string
pub async fn execute_sell(config_dir: &Path, symbol: &str, quantity: &str) -> crate::Result<()> {
    execute_manual_trade(config_dir, symbol, OrderSide::Sell, quantity).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_quantity_parsing() {
        // Test that invalid quantity produces an error
        let result = Decimal::from_str("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_quantity_parsing() {
        let result = Decimal::from_str("0.001");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Decimal::new(1, 3));
    }

    #[test]
    fn test_order_side_buy() {
        assert_eq!(format!("{:?}", OrderSide::Buy), "Buy");
    }

    #[test]
    fn test_order_side_sell() {
        assert_eq!(format!("{:?}", OrderSide::Sell), "Sell");
    }
}
