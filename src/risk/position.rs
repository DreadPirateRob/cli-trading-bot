//! Position tracking for stop-loss monitoring
//!
//! OpenPosition tracks individual positions with pre-calculated stop-loss prices.
//! PositionTracker maintains a collection of positions with efficient symbol-based lookup.

use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::data::OrderSide;

/// An open position with pre-calculated stop-loss price
#[derive(Debug, Clone)]
pub struct OpenPosition {
    pub order_id: String,
    pub symbol: String,
    pub side: OrderSide,
    pub entry_price: Decimal,
    pub quantity: Decimal,
    pub stop_loss_price: Decimal,
    pub strategy_id: String,
}

impl OpenPosition {
    /// Create a new position with stop-loss calculated at entry
    ///
    /// # Arguments
    /// * `order_id` - Unique order identifier
    /// * `symbol` - Trading pair (e.g., "BTCUSDT")
    /// * `side` - Buy (long) or Sell (short)
    /// * `entry_price` - Price at which position was entered
    /// * `quantity` - Position size
    /// * `stop_loss_pct` - Stop-loss percentage as decimal (e.g., 0.02 for 2%)
    /// * `strategy_id` - Identifier for the strategy that opened this position
    pub fn new(
        order_id: String,
        symbol: String,
        side: OrderSide,
        entry_price: Decimal,
        quantity: Decimal,
        stop_loss_pct: Decimal,
        strategy_id: String,
    ) -> Self {
        // Pre-calculate stop-loss price at entry
        let stop_loss_price = match side {
            // Long position: stop below entry (protect against price drop)
            OrderSide::Buy => entry_price * (Decimal::ONE - stop_loss_pct),
            // Short position: stop above entry (protect against price rise)
            OrderSide::Sell => entry_price * (Decimal::ONE + stop_loss_pct),
        };

        Self {
            order_id,
            symbol,
            side,
            entry_price,
            quantity,
            stop_loss_price,
            strategy_id,
        }
    }

    /// Check if current price triggers stop-loss
    ///
    /// Uses inclusive comparison (<=, >=) for precision safety
    pub fn is_stop_triggered(&self, current_price: Decimal) -> bool {
        match self.side {
            // Long: triggered when price falls to or below stop
            OrderSide::Buy => current_price <= self.stop_loss_price,
            // Short: triggered when price rises to or above stop
            OrderSide::Sell => current_price >= self.stop_loss_price,
        }
    }

    /// Get the opposite side for closing the position
    pub fn closing_side(&self) -> OrderSide {
        match self.side {
            OrderSide::Buy => OrderSide::Sell,
            OrderSide::Sell => OrderSide::Buy,
        }
    }
}

/// Tracks open positions with efficient symbol-based lookup
#[derive(Debug, Default)]
pub struct PositionTracker {
    /// order_id -> OpenPosition
    positions: HashMap<String, OpenPosition>,
    /// symbol -> list of order_ids (for price update lookups)
    by_symbol: HashMap<String, Vec<String>>,
}

impl PositionTracker {
    /// Create a new empty position tracker
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new position
    pub fn add(&mut self, position: OpenPosition) {
        let order_id = position.order_id.clone();
        let symbol = position.symbol.clone();

        self.positions.insert(order_id.clone(), position);
        self.by_symbol.entry(symbol).or_default().push(order_id);
    }

    /// Remove a position by order_id
    pub fn remove(&mut self, order_id: &str) -> Option<OpenPosition> {
        if let Some(position) = self.positions.remove(order_id) {
            // Clean up symbol index
            if let Some(order_ids) = self.by_symbol.get_mut(&position.symbol) {
                order_ids.retain(|id| id != order_id);
                if order_ids.is_empty() {
                    self.by_symbol.remove(&position.symbol);
                }
            }
            Some(position)
        } else {
            None
        }
    }

    /// Get a position by order_id
    pub fn get(&self, order_id: &str) -> Option<&OpenPosition> {
        self.positions.get(order_id)
    }

    /// Check all positions for a symbol against stop-loss, return triggered ones
    pub fn check_stop_losses(&self, symbol: &str, price: Decimal) -> Vec<OpenPosition> {
        self.by_symbol
            .get(symbol)
            .map(|order_ids| {
                order_ids
                    .iter()
                    .filter_map(|id| self.positions.get(id))
                    .filter(|pos| pos.is_stop_triggered(price))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get count of open positions
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    /// Check if tracker is empty
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Get all positions for a symbol
    pub fn positions_for_symbol(&self, symbol: &str) -> Vec<&OpenPosition> {
        self.by_symbol
            .get(symbol)
            .map(|order_ids| {
                order_ids
                    .iter()
                    .filter_map(|id| self.positions.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_stop_loss_long_position() {
        let pos = OpenPosition::new(
            "order-1".to_string(),
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            dec!(100),  // entry at $100
            dec!(1),
            dec!(0.02), // 2% stop-loss
            "test-strategy".to_string(),
        );

        // Stop should be at $98 (100 * 0.98)
        assert_eq!(pos.stop_loss_price, dec!(98));

        // Not triggered at $99
        assert!(!pos.is_stop_triggered(dec!(99)));

        // Triggered at $98 (exactly at stop)
        assert!(pos.is_stop_triggered(dec!(98)));

        // Triggered below stop
        assert!(pos.is_stop_triggered(dec!(97)));
    }

    #[test]
    fn test_stop_loss_short_position() {
        let pos = OpenPosition::new(
            "order-2".to_string(),
            "BTCUSDT".to_string(),
            OrderSide::Sell,
            dec!(100),  // entry at $100
            dec!(1),
            dec!(0.02), // 2% stop-loss
            "test-strategy".to_string(),
        );

        // Stop should be at $102 (100 * 1.02)
        assert_eq!(pos.stop_loss_price, dec!(102));

        // Not triggered at $101
        assert!(!pos.is_stop_triggered(dec!(101)));

        // Triggered at $102 (exactly at stop)
        assert!(pos.is_stop_triggered(dec!(102)));

        // Triggered above stop
        assert!(pos.is_stop_triggered(dec!(103)));
    }

    #[test]
    fn test_closing_side() {
        let long_pos = OpenPosition::new(
            "order-1".to_string(),
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            dec!(100),
            dec!(1),
            dec!(0.02),
            "test".to_string(),
        );
        assert_eq!(long_pos.closing_side(), OrderSide::Sell);

        let short_pos = OpenPosition::new(
            "order-2".to_string(),
            "BTCUSDT".to_string(),
            OrderSide::Sell,
            dec!(100),
            dec!(1),
            dec!(0.02),
            "test".to_string(),
        );
        assert_eq!(short_pos.closing_side(), OrderSide::Buy);
    }

    #[test]
    fn test_position_tracker_stop_loss_check() {
        let mut tracker = PositionTracker::new();

        tracker.add(OpenPosition::new(
            "order-1".to_string(),
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            dec!(100),
            dec!(1),
            dec!(0.02),
            "strategy-a".to_string(),
        ));

        tracker.add(OpenPosition::new(
            "order-2".to_string(),
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            dec!(110),
            dec!(1),
            dec!(0.02), // stop at $107.80
            "strategy-b".to_string(),
        ));

        // Price at $99 - only triggers order-1 (stop at $98)
        // Order-1: $99 <= $98? No. Order-2: $99 <= $107.80? Yes
        let triggered = tracker.check_stop_losses("BTCUSDT", dec!(99));
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0].order_id, "order-2");

        // Price at $107 - triggers only order-2 (stop at $107.80)
        // Order-1: $107 <= $98? No. Order-2: $107 <= $107.80? Yes
        let triggered = tracker.check_stop_losses("BTCUSDT", dec!(107));
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0].order_id, "order-2");

        // Price at $97 - triggers both
        // Order-1: $97 <= $98? Yes. Order-2: $97 <= $107.80? Yes
        let triggered = tracker.check_stop_losses("BTCUSDT", dec!(97));
        assert_eq!(triggered.len(), 2);
    }

    #[test]
    fn test_position_tracker_remove() {
        let mut tracker = PositionTracker::new();

        tracker.add(OpenPosition::new(
            "order-1".to_string(),
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            dec!(100),
            dec!(1),
            dec!(0.02),
            "test".to_string(),
        ));

        assert_eq!(tracker.len(), 1);

        let removed = tracker.remove("order-1");
        assert!(removed.is_some());
        assert_eq!(tracker.len(), 0);

        // Stop-loss check should return empty now
        let triggered = tracker.check_stop_losses("BTCUSDT", dec!(90));
        assert!(triggered.is_empty());
    }

    #[test]
    fn test_position_tracker_positions_for_symbol() {
        let mut tracker = PositionTracker::new();

        tracker.add(OpenPosition::new(
            "order-1".to_string(),
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            dec!(100),
            dec!(1),
            dec!(0.02),
            "test".to_string(),
        ));

        tracker.add(OpenPosition::new(
            "order-2".to_string(),
            "ETHUSDT".to_string(),
            OrderSide::Buy,
            dec!(50),
            dec!(2),
            dec!(0.02),
            "test".to_string(),
        ));

        let btc_positions = tracker.positions_for_symbol("BTCUSDT");
        assert_eq!(btc_positions.len(), 1);
        assert_eq!(btc_positions[0].order_id, "order-1");

        let eth_positions = tracker.positions_for_symbol("ETHUSDT");
        assert_eq!(eth_positions.len(), 1);
        assert_eq!(eth_positions[0].order_id, "order-2");

        let none_positions = tracker.positions_for_symbol("XRPUSDT");
        assert!(none_positions.is_empty());
    }

    #[test]
    fn test_position_tracker_get() {
        let mut tracker = PositionTracker::new();

        tracker.add(OpenPosition::new(
            "order-1".to_string(),
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            dec!(100),
            dec!(1),
            dec!(0.02),
            "test".to_string(),
        ));

        let pos = tracker.get("order-1");
        assert!(pos.is_some());
        assert_eq!(pos.unwrap().entry_price, dec!(100));

        let none = tracker.get("nonexistent");
        assert!(none.is_none());
    }
}
