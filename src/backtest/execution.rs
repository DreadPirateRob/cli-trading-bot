//! Execution simulation for backtesting
//!
//! Simulates trade execution with configurable fees and slippage.
//! Extracted from PaperExecutor patterns to share logic between
//! paper trading and backtesting.

use rust_decimal::Decimal;
use std::str::FromStr;

use crate::data::OrderSide;

/// Configuration for execution simulation
#[derive(Debug, Clone)]
pub struct ExecutionSimConfig {
    /// Slippage percentage (e.g., 0.001 for 0.1%)
    pub slippage_pct: Decimal,
    /// Trading fee percentage (e.g., 0.001 for 0.1%)
    pub fee_pct: Decimal,
}

impl Default for ExecutionSimConfig {
    fn default() -> Self {
        Self {
            slippage_pct: Decimal::ZERO, // Configurable, no default slippage
            // 0.1% default - Binance maker fee
            fee_pct: Decimal::from_str("0.001").expect("valid decimal literal"),
        }
    }
}

/// Result of simulated order execution
#[derive(Debug, Clone)]
pub struct SimulatedFill {
    /// Actual fill price after slippage
    pub fill_price: Decimal,
    /// Quantity filled
    pub quantity: Decimal,
    /// Commission paid
    pub commission: Decimal,
    /// Total cost/proceeds after fees
    /// For buys: notional + commission
    /// For sells: notional - commission
    pub net_value: Decimal,
    /// Slippage amount (absolute)
    pub slippage_amount: Decimal,
}

/// Simulate order fill with slippage and fees
///
/// Applies slippage based on order side:
/// - Buy orders fill at higher price (adverse slippage)
/// - Sell orders fill at lower price (adverse slippage)
///
/// # Arguments
/// * `side` - Order side (Buy or Sell)
/// * `price` - Market price at time of order
/// * `quantity` - Order quantity
/// * `config` - Execution simulation configuration
///
/// # Returns
/// SimulatedFill with fill price, commission, and net value
pub fn simulate_fill(
    side: OrderSide,
    price: Decimal,
    quantity: Decimal,
    config: &ExecutionSimConfig,
) -> SimulatedFill {
    // Apply slippage based on side:
    // - Buy: fill_price = price * (1 + slippage_pct) - fills higher
    // - Sell: fill_price = price * (1 - slippage_pct) - fills lower
    let fill_price = match side {
        OrderSide::Buy => price * (Decimal::ONE + config.slippage_pct),
        OrderSide::Sell => price * (Decimal::ONE - config.slippage_pct),
    };

    // Calculate notional value
    let notional = fill_price * quantity;

    // Calculate commission as percentage of notional
    let commission = notional * config.fee_pct;

    // Calculate net value:
    // - Buy: notional + commission (total cost)
    // - Sell: notional - commission (net proceeds)
    let net_value = match side {
        OrderSide::Buy => notional + commission,
        OrderSide::Sell => notional - commission,
    };

    // Calculate absolute slippage amount
    let slippage_amount = if fill_price >= price {
        (fill_price - price) * quantity
    } else {
        (price - fill_price) * quantity
    };

    SimulatedFill {
        fill_price,
        quantity,
        commission,
        net_value,
        slippage_amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_buy_slippage_increases_price() {
        let config = ExecutionSimConfig {
            slippage_pct: dec!(0.001), // 0.1%
            fee_pct: dec!(0.001),
        };

        let fill = simulate_fill(OrderSide::Buy, dec!(50000), dec!(1), &config);

        // Price should be higher than market
        assert!(fill.fill_price > dec!(50000));
        assert_eq!(fill.fill_price, dec!(50050)); // 50000 * 1.001
    }

    #[test]
    fn test_sell_slippage_decreases_price() {
        let config = ExecutionSimConfig {
            slippage_pct: dec!(0.001), // 0.1%
            fee_pct: dec!(0.001),
        };

        let fill = simulate_fill(OrderSide::Sell, dec!(50000), dec!(1), &config);

        // Price should be lower than market
        assert!(fill.fill_price < dec!(50000));
        assert_eq!(fill.fill_price, dec!(49950)); // 50000 * 0.999
    }

    #[test]
    fn test_commission_calculation() {
        let config = ExecutionSimConfig {
            slippage_pct: Decimal::ZERO,
            fee_pct: dec!(0.001), // 0.1%
        };

        let fill = simulate_fill(OrderSide::Buy, dec!(50000), dec!(1), &config);

        // Commission = 50000 * 0.001 = 50
        assert_eq!(fill.commission, dec!(50));
    }

    #[test]
    fn test_net_value_buy() {
        let config = ExecutionSimConfig {
            slippage_pct: Decimal::ZERO,
            fee_pct: dec!(0.001),
        };

        let fill = simulate_fill(OrderSide::Buy, dec!(50000), dec!(1), &config);

        // Net value for buy = notional + commission = 50000 + 50 = 50050
        assert_eq!(fill.net_value, dec!(50050));
    }

    #[test]
    fn test_net_value_sell() {
        let config = ExecutionSimConfig {
            slippage_pct: Decimal::ZERO,
            fee_pct: dec!(0.001),
        };

        let fill = simulate_fill(OrderSide::Sell, dec!(50000), dec!(1), &config);

        // Net value for sell = notional - commission = 50000 - 50 = 49950
        assert_eq!(fill.net_value, dec!(49950));
    }

    #[test]
    fn test_zero_slippage() {
        let config = ExecutionSimConfig {
            slippage_pct: Decimal::ZERO,
            fee_pct: Decimal::ZERO,
        };

        let fill = simulate_fill(OrderSide::Buy, dec!(50000), dec!(1), &config);

        assert_eq!(fill.fill_price, dec!(50000));
        assert_eq!(fill.slippage_amount, Decimal::ZERO);
    }

    #[test]
    fn test_default_config() {
        let config = ExecutionSimConfig::default();

        assert_eq!(config.slippage_pct, Decimal::ZERO);
        assert_eq!(config.fee_pct, dec!(0.001));
    }

    #[test]
    fn test_slippage_amount_calculation() {
        let config = ExecutionSimConfig {
            slippage_pct: dec!(0.001), // 0.1%
            fee_pct: Decimal::ZERO,
        };

        // Buy: slippage amount = (50050 - 50000) * 1 = 50
        let buy_fill = simulate_fill(OrderSide::Buy, dec!(50000), dec!(1), &config);
        assert_eq!(buy_fill.slippage_amount, dec!(50));

        // Sell: slippage amount = (50000 - 49950) * 1 = 50
        let sell_fill = simulate_fill(OrderSide::Sell, dec!(50000), dec!(1), &config);
        assert_eq!(sell_fill.slippage_amount, dec!(50));
    }

    #[test]
    fn test_quantity_multiplier() {
        let config = ExecutionSimConfig {
            slippage_pct: Decimal::ZERO,
            fee_pct: dec!(0.001),
        };

        // 2 units at 50000 = 100000 notional, 100 commission
        let fill = simulate_fill(OrderSide::Buy, dec!(50000), dec!(2), &config);

        assert_eq!(fill.quantity, dec!(2));
        assert_eq!(fill.commission, dec!(100));
        assert_eq!(fill.net_value, dec!(100100)); // 100000 + 100
    }
}
