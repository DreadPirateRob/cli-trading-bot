//! RSI-StdDev trading strategy implementation.
//!
//! This strategy generates buy signals when RSI crosses above the oversold
//! threshold (recovery from oversold) and sell signals when RSI crosses into
//! the overbought zone. Standard deviation is used to adjust position sizing
//! based on market volatility.
//!
//! # Signal Logic
//!
//! - **Buy**: Previous RSI < oversold AND current RSI >= oversold AND not in position
//! - **Sell**: Previous RSI < overbought AND current RSI >= overbought AND in position
//! - **Hold**: All other cases (including warmup period)

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ta::indicators::{RelativeStrengthIndex, StandardDeviation};
use ta::Next;

use super::{Signal, Strategy, StrategyTick};

/// Configuration for the RSI-StdDev strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsiStdDevConfig {
    /// RSI indicator period (typically 14).
    pub rsi_period: usize,
    /// RSI level below which asset is considered oversold (typically 30).
    pub rsi_oversold: f64,
    /// RSI level above which asset is considered overbought (typically 70).
    pub rsi_overbought: f64,
    /// Standard deviation period for volatility calculation.
    pub stddev_period: usize,
    /// Multiplier for volatility-adjusted position sizing.
    pub stddev_multiplier: f64,
    /// Base quantity for position sizing before volatility adjustment.
    pub base_quantity: Decimal,
}

/// RSI-StdDev trading strategy.
///
/// Generates trading signals based on RSI crossovers with oversold/overbought
/// thresholds, with position sizing adjusted by market volatility.
pub struct RsiStdDevStrategy {
    /// RSI indicator.
    rsi: RelativeStrengthIndex,
    /// Standard deviation indicator for volatility.
    stddev: StandardDeviation,
    /// Configurable thresholds.
    rsi_oversold: f64,
    rsi_overbought: f64,
    stddev_multiplier: f64,
    base_quantity: Decimal,
    /// RSI period for state reporting.
    rsi_period: usize,
    stddev_period: usize,
    /// State tracking.
    last_rsi: Option<f64>,
    prev_rsi: Option<f64>,
    last_stddev: Option<f64>,
    position_open: bool,
    /// Tick counter for warmup detection.
    tick_count: usize,
}

impl RsiStdDevStrategy {
    /// Create a new RSI-StdDev strategy with the given configuration.
    pub fn new(config: RsiStdDevConfig) -> Self {
        Self {
            rsi: RelativeStrengthIndex::new(config.rsi_period)
                .expect("RSI period must be > 0"),
            stddev: StandardDeviation::new(config.stddev_period)
                .expect("StdDev period must be > 0"),
            rsi_oversold: config.rsi_oversold,
            rsi_overbought: config.rsi_overbought,
            stddev_multiplier: config.stddev_multiplier,
            base_quantity: config.base_quantity,
            rsi_period: config.rsi_period,
            stddev_period: config.stddev_period,
            last_rsi: None,
            prev_rsi: None,
            last_stddev: None,
            position_open: false,
            tick_count: 0,
        }
    }

    /// Check if we have enough data for meaningful RSI values.
    fn is_warmed_up(&self) -> bool {
        // RSI needs at least period + 1 data points
        self.tick_count > self.rsi_period
    }

    /// Calculate volatility-adjusted quantity.
    fn calculate_quantity(&self) -> Decimal {
        // If stddev is available and meaningful, adjust quantity
        if let Some(stddev) = self.last_stddev {
            if stddev > 0.0 && stddev.is_finite() {
                // Higher volatility -> smaller position
                // Adjust by inverse of stddev * multiplier
                let adjustment = 1.0 / (1.0 + stddev * self.stddev_multiplier / 100.0);
                if let Some(adjusted) = Decimal::from_f64_retain(adjustment) {
                    return self.base_quantity * adjusted;
                }
            }
        }
        self.base_quantity
    }

    /// Check for oversold recovery (buy signal condition).
    fn is_oversold_recovery(&self) -> bool {
        match (self.prev_rsi, self.last_rsi) {
            (Some(prev), Some(current)) => {
                prev < self.rsi_oversold && current >= self.rsi_oversold
            }
            _ => false,
        }
    }

    /// Check for overbought entry (sell signal condition).
    fn is_overbought_entry(&self) -> bool {
        match (self.prev_rsi, self.last_rsi) {
            (Some(prev), Some(current)) => {
                prev < self.rsi_overbought && current >= self.rsi_overbought
            }
            _ => false,
        }
    }
}

impl Strategy for RsiStdDevStrategy {
    fn on_tick(&mut self, tick: &StrategyTick) -> Signal {
        // Convert price to f64 for indicators
        let price = tick.price.to_string().parse::<f64>().unwrap_or(0.0);

        // Update indicators
        let rsi_value = self.rsi.next(price);
        let stddev_value = self.stddev.next(price);

        // Shift RSI values for crossover detection
        self.prev_rsi = self.last_rsi;
        self.last_rsi = Some(rsi_value);
        self.last_stddev = Some(stddev_value);
        self.tick_count += 1;

        // During warmup, always return Hold
        if !self.is_warmed_up() {
            return Signal::Hold;
        }

        // Check for buy signal: oversold recovery AND not in position
        if !self.position_open && self.is_oversold_recovery() {
            self.position_open = true;
            return Signal::Buy {
                quantity: self.calculate_quantity(),
            };
        }

        // Check for sell signal: overbought entry AND in position
        if self.position_open && self.is_overbought_entry() {
            self.position_open = false;
            return Signal::Sell {
                quantity: self.calculate_quantity(),
            };
        }

        Signal::Hold
    }

    fn update_config(&mut self, config: &Value) -> Result<(), String> {
        // Parse new threshold values
        let new_oversold = config
            .get("rsi_oversold")
            .and_then(|v| v.as_f64())
            .unwrap_or(self.rsi_oversold);

        let new_overbought = config
            .get("rsi_overbought")
            .and_then(|v| v.as_f64())
            .unwrap_or(self.rsi_overbought);

        let new_multiplier = config
            .get("stddev_multiplier")
            .and_then(|v| v.as_f64())
            .unwrap_or(self.stddev_multiplier);

        // Validate: oversold must be less than overbought
        if new_oversold >= new_overbought {
            return Err(format!(
                "rsi_oversold ({}) must be less than rsi_overbought ({})",
                new_oversold, new_overbought
            ));
        }

        // Validate RSI thresholds are in valid range
        if new_oversold < 0.0 || new_oversold > 100.0 {
            return Err(format!(
                "rsi_oversold must be between 0 and 100, got {}",
                new_oversold
            ));
        }

        if new_overbought < 0.0 || new_overbought > 100.0 {
            return Err(format!(
                "rsi_overbought must be between 0 and 100, got {}",
                new_overbought
            ));
        }

        // Apply valid configuration
        self.rsi_oversold = new_oversold;
        self.rsi_overbought = new_overbought;
        self.stddev_multiplier = new_multiplier;

        Ok(())
    }

    fn get_state(&self) -> Value {
        json!({
            "last_rsi": self.last_rsi,
            "prev_rsi": self.prev_rsi,
            "last_stddev": self.last_stddev,
            "position_open": self.position_open,
            "tick_count": self.tick_count,
            "warmed_up": self.is_warmed_up(),
            "config": {
                "rsi_period": self.rsi_period,
                "rsi_oversold": self.rsi_oversold,
                "rsi_overbought": self.rsi_overbought,
                "stddev_period": self.stddev_period,
                "stddev_multiplier": self.stddev_multiplier,
                "base_quantity": self.base_quantity.to_string()
            }
        })
    }

    fn name(&self) -> &'static str {
        "rsi_stddev"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn default_config() -> RsiStdDevConfig {
        RsiStdDevConfig {
            rsi_period: 14,
            rsi_oversold: 30.0,
            rsi_overbought: 70.0,
            stddev_period: 20,
            stddev_multiplier: 1.0,
            base_quantity: dec!(0.01),
        }
    }

    #[test]
    fn test_new_creates_strategy() {
        let strategy = RsiStdDevStrategy::new(default_config());
        assert_eq!(strategy.name(), "rsi_stddev");
        assert!(!strategy.position_open);
        assert!(strategy.last_rsi.is_none());
    }

    #[test]
    fn test_warmup_detection() {
        let mut strategy = RsiStdDevStrategy::new(default_config());
        assert!(!strategy.is_warmed_up());

        // Feed enough ticks to warm up
        for _ in 0..15 {
            strategy.tick_count += 1;
        }
        assert!(strategy.is_warmed_up());
    }

    #[test]
    fn test_oversold_recovery_detection() {
        let mut strategy = RsiStdDevStrategy::new(default_config());

        // No crossover
        strategy.prev_rsi = Some(25.0);
        strategy.last_rsi = Some(28.0);
        assert!(!strategy.is_oversold_recovery());

        // Crossover
        strategy.prev_rsi = Some(25.0);
        strategy.last_rsi = Some(35.0);
        assert!(strategy.is_oversold_recovery());
    }

    #[test]
    fn test_overbought_entry_detection() {
        let mut strategy = RsiStdDevStrategy::new(default_config());

        // No crossover
        strategy.prev_rsi = Some(65.0);
        strategy.last_rsi = Some(68.0);
        assert!(!strategy.is_overbought_entry());

        // Crossover
        strategy.prev_rsi = Some(65.0);
        strategy.last_rsi = Some(75.0);
        assert!(strategy.is_overbought_entry());
    }
}
