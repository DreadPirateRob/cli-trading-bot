//! Grid trading strategy implementation.
//!
//! Grid strategy places buy orders below current price and sell orders above,
//! profiting from price oscillations within a range. When price moves through
//! a level, the opposite action is taken.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::data::OrderSide;
use crate::strategy::{Signal, Strategy, StrategyTick};

/// Configuration for the grid strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridConfig {
    /// Spacing between levels as a percentage (e.g., 0.01 = 1%)
    pub spacing_pct: Decimal,
    /// Number of levels on each side of center
    pub num_levels: u32,
    /// Quantity to trade at each level
    pub quantity_per_level: Decimal,
    /// Multiplier for escape threshold (default 2.0)
    pub escape_multiplier: Decimal,
}

/// A single grid level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GridLevel {
    /// Price at this level
    pub price: Decimal,
    /// Buy below center, Sell above
    pub side: OrderSide,
    /// Whether this level has been filled
    pub filled: bool,
}

/// Grid trading strategy.
///
/// Places buy orders below current price and sell orders above,
/// profiting from price oscillations within a range.
pub struct GridStrategy {
    /// Current center price around which the grid is built
    center_price: Decimal,
    /// Spacing between levels as a percentage
    spacing_pct: Decimal,
    /// Number of levels on each side of center
    num_levels: u32,
    /// Quantity to trade at each level
    quantity_per_level: Decimal,
    /// Multiplier for escape threshold
    escape_multiplier: Decimal,
    /// Grid levels (buy levels below center, sell levels above)
    levels: Vec<GridLevel>,
    /// Lower bound (lowest buy level price)
    lower_bound: Decimal,
    /// Upper bound (highest sell level price)
    upper_bound: Decimal,
}

impl GridStrategy {
    /// Create a new grid strategy with the given configuration and center price.
    pub fn new(config: GridConfig, center_price: Decimal) -> Self {
        let mut strategy = Self {
            center_price,
            spacing_pct: config.spacing_pct,
            num_levels: config.num_levels,
            quantity_per_level: config.quantity_per_level,
            escape_multiplier: config.escape_multiplier,
            levels: Vec::new(),
            lower_bound: Decimal::ZERO,
            upper_bound: Decimal::ZERO,
        };
        strategy.generate_levels(center_price);
        strategy
    }

    /// Generate grid levels around a center price.
    fn generate_levels(&mut self, center: Decimal) {
        self.center_price = center;
        self.levels.clear();

        // Generate buy levels below center
        // Buy level i: center * (1 - spacing * i) for i = 1..=num_levels
        for i in 1..=self.num_levels {
            let factor = Decimal::ONE - self.spacing_pct * Decimal::from(i);
            let price = center * factor;
            self.levels.push(GridLevel {
                price,
                side: OrderSide::Buy,
                filled: false,
            });
        }

        // Generate sell levels above center
        // Sell level i: center * (1 + spacing * i) for i = 1..=num_levels
        for i in 1..=self.num_levels {
            let factor = Decimal::ONE + self.spacing_pct * Decimal::from(i);
            let price = center * factor;
            self.levels.push(GridLevel {
                price,
                side: OrderSide::Sell,
                filled: false,
            });
        }

        // Calculate bounds
        // Lower bound is the lowest buy level (largest i)
        self.lower_bound = center * (Decimal::ONE - self.spacing_pct * Decimal::from(self.num_levels));
        // Upper bound is the highest sell level (largest i)
        self.upper_bound = center * (Decimal::ONE + self.spacing_pct * Decimal::from(self.num_levels));
    }

    /// Calculate escape threshold prices.
    /// Returns (lower_escape, upper_escape)
    fn escape_thresholds(&self) -> (Decimal, Decimal) {
        let escape_distance = self.center_price * self.spacing_pct * self.escape_multiplier;
        let lower_escape = self.lower_bound - escape_distance;
        let upper_escape = self.upper_bound + escape_distance;
        (lower_escape, upper_escape)
    }

    /// Check if price has escaped the grid bounds.
    fn has_escaped(&self, price: Decimal) -> bool {
        let (lower_escape, upper_escape) = self.escape_thresholds();
        price < lower_escape || price > upper_escape
    }

    /// Find the nearest unfilled level that the price has crossed.
    /// For buy levels: price <= level.price
    /// For sell levels: price >= level.price
    /// Returns the index of the nearest level that should trigger.
    fn find_nearest_triggerable_level(&self, price: Decimal) -> Option<usize> {
        let mut nearest_buy: Option<(usize, Decimal)> = None;
        let mut nearest_sell: Option<(usize, Decimal)> = None;

        for (i, level) in self.levels.iter().enumerate() {
            if level.filled {
                continue;
            }

            match level.side {
                OrderSide::Buy => {
                    // Buy triggers when price <= level.price
                    if price <= level.price {
                        // Track the nearest (highest) buy level
                        match nearest_buy {
                            None => nearest_buy = Some((i, level.price)),
                            Some((_, best_price)) if level.price > best_price => {
                                nearest_buy = Some((i, level.price));
                            }
                            _ => {}
                        }
                    }
                }
                OrderSide::Sell => {
                    // Sell triggers when price >= level.price
                    if price >= level.price {
                        // Track the nearest (lowest) sell level
                        match nearest_sell {
                            None => nearest_sell = Some((i, level.price)),
                            Some((_, best_price)) if level.price < best_price => {
                                nearest_sell = Some((i, level.price));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Return the nearest one (closest to center price)
        match (nearest_buy, nearest_sell) {
            (Some((bi, bp)), Some((si, sp))) => {
                // Both could trigger - pick the one closer to center
                let buy_distance = (self.center_price - bp).abs();
                let sell_distance = (sp - self.center_price).abs();
                if buy_distance <= sell_distance {
                    Some(bi)
                } else {
                    Some(si)
                }
            }
            (Some((bi, _)), None) => Some(bi),
            (None, Some((si, _))) => Some(si),
            (None, None) => None,
        }
    }
}

impl Strategy for GridStrategy {
    fn on_tick(&mut self, tick: &StrategyTick) -> Signal {
        let price = tick.price;

        // Check for escape condition - reset grid if price escaped
        if self.has_escaped(price) {
            self.generate_levels(price);
            return Signal::Hold;
        }

        // Find nearest triggerable level
        if let Some(index) = self.find_nearest_triggerable_level(price) {
            let level = &mut self.levels[index];
            level.filled = true;

            match level.side {
                OrderSide::Buy => Signal::Buy {
                    quantity: self.quantity_per_level,
                },
                OrderSide::Sell => Signal::Sell {
                    quantity: self.quantity_per_level,
                },
            }
        } else {
            Signal::Hold
        }
    }

    fn update_config(&mut self, config: &serde_json::Value) -> Result<(), String> {
        let new_config: GridConfig =
            serde_json::from_value(config.clone()).map_err(|e| e.to_string())?;

        self.spacing_pct = new_config.spacing_pct;
        self.num_levels = new_config.num_levels;
        self.quantity_per_level = new_config.quantity_per_level;
        self.escape_multiplier = new_config.escape_multiplier;

        // Regenerate grid with current center price
        self.generate_levels(self.center_price);

        Ok(())
    }

    fn get_state(&self) -> serde_json::Value {
        // Sort levels for consistent output: buy levels first (descending price),
        // then sell levels (ascending price)
        let mut buy_levels: Vec<&GridLevel> = self
            .levels
            .iter()
            .filter(|l| l.side == OrderSide::Buy)
            .collect();
        buy_levels.sort_by(|a, b| b.price.cmp(&a.price)); // Descending (99, 98, 97)

        let mut sell_levels: Vec<&GridLevel> = self
            .levels
            .iter()
            .filter(|l| l.side == OrderSide::Sell)
            .collect();
        sell_levels.sort_by(|a, b| a.price.cmp(&b.price)); // Ascending (101, 102, 103)

        let sorted_levels: Vec<GridLevel> = buy_levels
            .into_iter()
            .chain(sell_levels.into_iter())
            .cloned()
            .collect();

        json!({
            "center_price": self.center_price,
            "spacing_pct": self.spacing_pct,
            "num_levels": self.num_levels,
            "quantity_per_level": self.quantity_per_level,
            "escape_multiplier": self.escape_multiplier,
            "lower_bound": self.lower_bound,
            "upper_bound": self.upper_bound,
            "levels": sorted_levels,
        })
    }

    fn name(&self) -> &'static str {
        "grid"
    }
}
