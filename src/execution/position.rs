//! Position tracking and management for order execution
//!
//! Tracks open positions by order ID and symbol, with FIFO closing
//! for P&L calculation.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::HashMap;
use tracing::{debug, info};

use crate::data::{calculate_pnl, OrderSide};

/// An open position from a filled order
#[derive(Debug, Clone)]
pub struct ExecutionPosition {
    pub order_id: String,
    pub symbol: String,
    pub side: OrderSide,
    pub entry_price: Decimal,
    pub quantity: Decimal,
    pub entry_time: DateTime<Utc>,
    pub strategy_id: String,
}

/// Result of closing a position (or partial close)
#[derive(Debug, Clone)]
pub struct ClosedPosition {
    pub order_id: String,
    pub symbol: String,
    pub side: OrderSide,
    pub entry_price: Decimal,
    pub exit_price: Decimal,
    pub quantity: Decimal,
    pub pnl: Decimal,
    pub entry_time: DateTime<Utc>,
    pub exit_time: DateTime<Utc>,
}

/// Manages open positions with FIFO closing
#[derive(Debug)]
pub struct PositionManager {
    /// Positions by order_id for direct lookup
    positions_by_id: HashMap<String, ExecutionPosition>,
    /// Order IDs by symbol for FIFO ordering (oldest first)
    order_ids_by_symbol: HashMap<String, Vec<String>>,
}

impl PositionManager {
    pub fn new() -> Self {
        Self {
            positions_by_id: HashMap::new(),
            order_ids_by_symbol: HashMap::new(),
        }
    }

    /// Add a new open position
    pub fn add(&mut self, position: ExecutionPosition) {
        debug!(
            order_id = %position.order_id,
            symbol = %position.symbol,
            side = ?position.side,
            quantity = %position.quantity,
            entry_price = %position.entry_price,
            "Position opened"
        );

        let symbol = position.symbol.clone();
        let order_id = position.order_id.clone();

        self.positions_by_id.insert(order_id.clone(), position);
        self.order_ids_by_symbol
            .entry(symbol)
            .or_default()
            .push(order_id);
    }

    /// Get a position by order ID
    pub fn get(&self, order_id: &str) -> Option<&ExecutionPosition> {
        self.positions_by_id.get(order_id)
    }

    /// Get all positions for a symbol
    pub fn get_by_symbol(&self, symbol: &str) -> Vec<&ExecutionPosition> {
        self.order_ids_by_symbol
            .get(symbol)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.positions_by_id.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get total quantity for a symbol (all positions)
    pub fn total_quantity(&self, symbol: &str) -> Decimal {
        self.get_by_symbol(symbol).iter().map(|p| p.quantity).sum()
    }

    /// Get total quantity for a symbol and side
    pub fn total_quantity_by_side(&self, symbol: &str, side: OrderSide) -> Decimal {
        self.get_by_symbol(symbol)
            .iter()
            .filter(|p| p.side == side)
            .map(|p| p.quantity)
            .sum()
    }

    /// Close a specific position by order ID
    pub fn close(&mut self, order_id: &str, exit_price: Decimal) -> Option<ClosedPosition> {
        let position = self.positions_by_id.remove(order_id)?;

        // Remove from symbol index
        if let Some(ids) = self.order_ids_by_symbol.get_mut(&position.symbol) {
            ids.retain(|id| id != order_id);
            if ids.is_empty() {
                self.order_ids_by_symbol.remove(&position.symbol);
            }
        }

        let pnl = calculate_pnl(
            position.side,
            position.entry_price,
            exit_price,
            position.quantity,
        );
        let exit_time = Utc::now();

        info!(
            order_id = %order_id,
            symbol = %position.symbol,
            pnl = %pnl,
            "Position closed"
        );

        Some(ClosedPosition {
            order_id: position.order_id,
            symbol: position.symbol,
            side: position.side,
            entry_price: position.entry_price,
            exit_price,
            quantity: position.quantity,
            pnl,
            entry_time: position.entry_time,
            exit_time,
        })
    }

    /// Close positions using FIFO ordering (oldest first)
    ///
    /// Closes positions until `quantity_to_close` is satisfied.
    /// Returns list of closed positions (may be multiple for FIFO).
    pub fn close_fifo(
        &mut self,
        symbol: &str,
        side: OrderSide,
        quantity_to_close: Decimal,
        exit_price: Decimal,
    ) -> Vec<ClosedPosition> {
        let mut remaining = quantity_to_close;
        let mut closed = Vec::new();

        // Get order IDs for this symbol (oldest first)
        let order_ids: Vec<String> = self
            .order_ids_by_symbol
            .get(symbol)
            .map(|ids| ids.clone())
            .unwrap_or_default();

        for order_id in order_ids {
            if remaining <= Decimal::ZERO {
                break;
            }

            let Some(position) = self.positions_by_id.get(&order_id) else {
                continue;
            };

            // Only close positions of matching side
            if position.side != side {
                continue;
            }

            if position.quantity <= remaining {
                // Close entire position
                remaining -= position.quantity;
                if let Some(closed_pos) = self.close(&order_id, exit_price) {
                    closed.push(closed_pos);
                }
            } else {
                // Partial close - reduce position quantity
                let partial_qty = remaining;
                remaining = Decimal::ZERO;

                let position = self.positions_by_id.get_mut(&order_id).unwrap();
                let pnl = calculate_pnl(position.side, position.entry_price, exit_price, partial_qty);

                let partial_close = ClosedPosition {
                    order_id: position.order_id.clone(),
                    symbol: position.symbol.clone(),
                    side: position.side,
                    entry_price: position.entry_price,
                    exit_price,
                    quantity: partial_qty,
                    pnl,
                    entry_time: position.entry_time,
                    exit_time: Utc::now(),
                };

                position.quantity -= partial_qty;
                debug!(
                    order_id = %order_id,
                    remaining_qty = %position.quantity,
                    closed_qty = %partial_qty,
                    "Partial position close"
                );

                closed.push(partial_close);
            }
        }

        // Clean up any zero-quantity positions
        self.positions_by_id
            .retain(|_, p| p.quantity > Decimal::ZERO);

        closed
    }

    /// Get count of open positions
    pub fn len(&self) -> usize {
        self.positions_by_id.len()
    }

    /// Check if no open positions
    pub fn is_empty(&self) -> bool {
        self.positions_by_id.is_empty()
    }

    /// Get all open positions
    pub fn all_positions(&self) -> impl Iterator<Item = &ExecutionPosition> {
        self.positions_by_id.values()
    }
}

impl Default for PositionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn create_position(
        order_id: &str,
        side: OrderSide,
        quantity: Decimal,
        price: Decimal,
    ) -> ExecutionPosition {
        ExecutionPosition {
            order_id: order_id.to_string(),
            symbol: "BTCUSDT".to_string(),
            side,
            entry_price: price,
            quantity,
            entry_time: Utc::now(),
            strategy_id: "test".to_string(),
        }
    }

    #[test]
    fn test_add_and_get() {
        let mut mgr = PositionManager::new();
        let pos = create_position("order-1", OrderSide::Buy, dec!(1), dec!(50000));

        mgr.add(pos);

        assert!(mgr.get("order-1").is_some());
        assert_eq!(mgr.len(), 1);
    }

    #[test]
    fn test_close_position() {
        let mut mgr = PositionManager::new();
        mgr.add(create_position(
            "order-1",
            OrderSide::Buy,
            dec!(1),
            dec!(50000),
        ));

        let closed = mgr.close("order-1", dec!(55000)).unwrap();

        assert_eq!(closed.pnl, dec!(5000)); // (55000 - 50000) * 1
        assert!(mgr.is_empty());
    }

    #[test]
    fn test_close_position_sell_side() {
        let mut mgr = PositionManager::new();
        mgr.add(create_position(
            "order-1",
            OrderSide::Sell,
            dec!(1),
            dec!(50000),
        ));

        // Short position profits when price goes down
        let closed = mgr.close("order-1", dec!(48000)).unwrap();

        assert_eq!(closed.pnl, dec!(2000)); // (50000 - 48000) * 1
    }

    #[test]
    fn test_fifo_closing() {
        let mut mgr = PositionManager::new();

        // Add positions in order (oldest first)
        mgr.add(create_position(
            "order-1",
            OrderSide::Buy,
            dec!(0.5),
            dec!(50000),
        ));
        mgr.add(create_position(
            "order-2",
            OrderSide::Buy,
            dec!(0.5),
            dec!(51000),
        ));
        mgr.add(create_position(
            "order-3",
            OrderSide::Buy,
            dec!(0.5),
            dec!(52000),
        ));

        // Close 0.7 BTC at 55000 - should close order-1 (0.5) and partial order-2 (0.2)
        let closed = mgr.close_fifo("BTCUSDT", OrderSide::Buy, dec!(0.7), dec!(55000));

        assert_eq!(closed.len(), 2);

        // First closed: order-1, full 0.5 BTC
        assert_eq!(closed[0].order_id, "order-1");
        assert_eq!(closed[0].quantity, dec!(0.5));
        assert_eq!(closed[0].pnl, dec!(2500)); // (55000 - 50000) * 0.5

        // Second closed: order-2, partial 0.2 BTC
        assert_eq!(closed[1].order_id, "order-2");
        assert_eq!(closed[1].quantity, dec!(0.2));
        assert_eq!(closed[1].pnl, dec!(800)); // (55000 - 51000) * 0.2

        // Remaining: order-2 (0.3) and order-3 (0.5)
        assert_eq!(mgr.len(), 2);
        assert_eq!(mgr.get("order-2").unwrap().quantity, dec!(0.3));
    }

    #[test]
    fn test_fifo_closes_all_needed() {
        let mut mgr = PositionManager::new();

        mgr.add(create_position(
            "order-1",
            OrderSide::Buy,
            dec!(0.3),
            dec!(50000),
        ));
        mgr.add(create_position(
            "order-2",
            OrderSide::Buy,
            dec!(0.3),
            dec!(51000),
        ));
        mgr.add(create_position(
            "order-3",
            OrderSide::Buy,
            dec!(0.3),
            dec!(52000),
        ));

        // Close all 0.9 BTC
        let closed = mgr.close_fifo("BTCUSDT", OrderSide::Buy, dec!(0.9), dec!(55000));

        assert_eq!(closed.len(), 3);
        assert!(mgr.is_empty());
    }

    #[test]
    fn test_fifo_only_closes_matching_side() {
        let mut mgr = PositionManager::new();

        mgr.add(create_position(
            "buy-1",
            OrderSide::Buy,
            dec!(1),
            dec!(50000),
        ));
        mgr.add(create_position(
            "sell-1",
            OrderSide::Sell,
            dec!(1),
            dec!(51000),
        ));

        // Only close buy positions
        let closed = mgr.close_fifo("BTCUSDT", OrderSide::Buy, dec!(1), dec!(55000));

        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].order_id, "buy-1");
        assert_eq!(mgr.len(), 1); // sell-1 still open
        assert!(mgr.get("sell-1").is_some());
    }

    #[test]
    fn test_total_quantity() {
        let mut mgr = PositionManager::new();
        mgr.add(create_position(
            "order-1",
            OrderSide::Buy,
            dec!(1),
            dec!(50000),
        ));
        mgr.add(create_position(
            "order-2",
            OrderSide::Buy,
            dec!(0.5),
            dec!(51000),
        ));

        assert_eq!(mgr.total_quantity("BTCUSDT"), dec!(1.5));
        assert_eq!(
            mgr.total_quantity_by_side("BTCUSDT", OrderSide::Buy),
            dec!(1.5)
        );
        assert_eq!(
            mgr.total_quantity_by_side("BTCUSDT", OrderSide::Sell),
            dec!(0)
        );
    }

    #[test]
    fn test_get_by_symbol() {
        let mut mgr = PositionManager::new();
        mgr.add(create_position(
            "order-1",
            OrderSide::Buy,
            dec!(1),
            dec!(50000),
        ));

        let positions = mgr.get_by_symbol("BTCUSDT");
        assert_eq!(positions.len(), 1);

        let positions = mgr.get_by_symbol("ETHUSDT");
        assert_eq!(positions.len(), 0);
    }

    #[test]
    fn test_close_nonexistent() {
        let mut mgr = PositionManager::new();
        let closed = mgr.close("nonexistent", dec!(50000));
        assert!(closed.is_none());
    }

    #[test]
    fn test_fifo_with_no_positions() {
        let mut mgr = PositionManager::new();
        let closed = mgr.close_fifo("BTCUSDT", OrderSide::Buy, dec!(1), dec!(50000));
        assert!(closed.is_empty());
    }

    #[test]
    fn test_all_positions() {
        let mut mgr = PositionManager::new();
        mgr.add(create_position(
            "order-1",
            OrderSide::Buy,
            dec!(1),
            dec!(50000),
        ));
        mgr.add(create_position(
            "order-2",
            OrderSide::Buy,
            dec!(0.5),
            dec!(51000),
        ));

        let all: Vec<_> = mgr.all_positions().collect();
        assert_eq!(all.len(), 2);
    }
}
