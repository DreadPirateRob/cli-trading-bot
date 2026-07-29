//! Balance tracking and management for order execution
//!
//! Tracks available balances per asset with configurable reserve amounts
//! to prevent overdrawing configured safety margins.

use rust_decimal::Decimal;
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Balance reserve configuration
#[derive(Debug, Clone)]
pub struct ReserveConfig {
    /// Reserve as percentage of total (e.g., 0.10 = 10%)
    pub reserve_pct: Option<Decimal>,
    /// Reserve as fixed amount per asset
    pub reserve_amount: HashMap<String, Decimal>,
}

impl Default for ReserveConfig {
    fn default() -> Self {
        Self {
            reserve_pct: None,
            reserve_amount: HashMap::new(),
        }
    }
}

impl ReserveConfig {
    /// Create reserve config with percentage-based reserve
    pub fn with_percentage(pct: Decimal) -> Self {
        Self {
            reserve_pct: Some(pct),
            reserve_amount: HashMap::new(),
        }
    }

    /// Create reserve config with fixed amount reserve
    pub fn with_fixed(reserves: HashMap<String, Decimal>) -> Self {
        Self {
            reserve_pct: None,
            reserve_amount: reserves,
        }
    }
}

/// Manages balances for trading with reserve support
#[derive(Debug)]
pub struct BalanceManager {
    /// Current balances by asset
    balances: HashMap<String, Decimal>,
    /// Reserve configuration
    reserve_config: ReserveConfig,
    /// Amount currently reserved for in-flight orders
    in_flight_reserves: HashMap<String, Decimal>,
}

impl BalanceManager {
    /// Create a new balance manager
    pub fn new(reserve_config: ReserveConfig) -> Self {
        Self {
            balances: HashMap::new(),
            reserve_config,
            in_flight_reserves: HashMap::new(),
        }
    }

    /// Create with initial balances
    pub fn with_balances(
        balances: HashMap<String, Decimal>,
        reserve_config: ReserveConfig,
    ) -> Self {
        Self {
            balances,
            reserve_config,
            in_flight_reserves: HashMap::new(),
        }
    }

    /// Set balance for an asset (from exchange sync)
    pub fn set_balance(&mut self, asset: &str, amount: Decimal) {
        debug!(asset = %asset, amount = %amount, "Balance updated");
        self.balances.insert(asset.to_string(), amount);
    }

    /// Set all balances (from exchange sync)
    pub fn set_all_balances(&mut self, balances: HashMap<String, Decimal>) {
        info!(count = balances.len(), "All balances synced");
        self.balances = balances;
    }

    /// Get raw balance for an asset
    pub fn get_balance(&self, asset: &str) -> Decimal {
        self.balances.get(asset).copied().unwrap_or(Decimal::ZERO)
    }

    /// Get available balance (after reserves and in-flight)
    pub fn get_available(&self, asset: &str) -> Decimal {
        let total = self.get_balance(asset);
        let reserve = self.calculate_reserve(asset, total);
        let in_flight = self
            .in_flight_reserves
            .get(asset)
            .copied()
            .unwrap_or(Decimal::ZERO);

        let available = total - reserve - in_flight;
        available.max(Decimal::ZERO)
    }

    /// Calculate reserve amount for an asset
    fn calculate_reserve(&self, asset: &str, total: Decimal) -> Decimal {
        // Fixed reserve takes precedence
        if let Some(&fixed) = self.reserve_config.reserve_amount.get(asset) {
            return fixed;
        }

        // Percentage reserve
        if let Some(pct) = self.reserve_config.reserve_pct {
            return total * pct;
        }

        Decimal::ZERO
    }

    /// Check if we have enough available balance
    pub fn has_available(&self, asset: &str, amount: Decimal) -> bool {
        self.get_available(asset) >= amount
    }

    /// Reserve amount for an in-flight order
    ///
    /// Returns true if reservation succeeded, false if insufficient funds.
    pub fn reserve(&mut self, asset: &str, amount: Decimal) -> bool {
        let available = self.get_available(asset);

        if available < amount {
            warn!(
                asset = %asset,
                required = %amount,
                available = %available,
                "Insufficient balance for reservation"
            );
            return false;
        }

        *self
            .in_flight_reserves
            .entry(asset.to_string())
            .or_insert(Decimal::ZERO) += amount;
        debug!(asset = %asset, amount = %amount, "Reserved for order");
        true
    }

    /// Release reservation (order completed or canceled)
    pub fn release_reservation(&mut self, asset: &str, amount: Decimal) {
        if let Some(reserved) = self.in_flight_reserves.get_mut(asset) {
            *reserved = (*reserved - amount).max(Decimal::ZERO);
            debug!(asset = %asset, amount = %amount, "Reservation released");
        }
    }

    /// Deduct from balance (order filled)
    pub fn deduct(&mut self, asset: &str, amount: Decimal) {
        if let Some(balance) = self.balances.get_mut(asset) {
            *balance = (*balance - amount).max(Decimal::ZERO);
            debug!(asset = %asset, amount = %amount, new_balance = %balance, "Balance deducted");
        }
    }

    /// Credit to balance (order filled, received asset)
    pub fn credit(&mut self, asset: &str, amount: Decimal) {
        *self
            .balances
            .entry(asset.to_string())
            .or_insert(Decimal::ZERO) += amount;
        debug!(asset = %asset, amount = %amount, "Balance credited");
    }

    /// Get all balances (for monitoring/display)
    pub fn all_balances(&self) -> &HashMap<String, Decimal> {
        &self.balances
    }

    /// Get total in-flight reservations
    pub fn total_in_flight(&self) -> Decimal {
        self.in_flight_reserves.values().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_balance_manager_basic() {
        let mut mgr = BalanceManager::new(ReserveConfig::default());
        mgr.set_balance("USDT", dec!(10000));

        assert_eq!(mgr.get_balance("USDT"), dec!(10000));
        assert_eq!(mgr.get_available("USDT"), dec!(10000));
    }

    #[test]
    fn test_balance_with_percentage_reserve() {
        let mut mgr = BalanceManager::new(ReserveConfig::with_percentage(dec!(0.10)));
        mgr.set_balance("USDT", dec!(10000));

        // 10% reserve = 1000, available = 9000
        assert_eq!(mgr.get_available("USDT"), dec!(9000));
    }

    #[test]
    fn test_balance_with_fixed_reserve() {
        let mut reserves = HashMap::new();
        reserves.insert("USDT".to_string(), dec!(500));

        let mut mgr = BalanceManager::new(ReserveConfig::with_fixed(reserves));
        mgr.set_balance("USDT", dec!(10000));

        // Fixed 500 reserve, available = 9500
        assert_eq!(mgr.get_available("USDT"), dec!(9500));
    }

    #[test]
    fn test_reserve_and_release() {
        let mut mgr = BalanceManager::new(ReserveConfig::default());
        mgr.set_balance("USDT", dec!(1000));

        // Reserve 300
        assert!(mgr.reserve("USDT", dec!(300)));
        assert_eq!(mgr.get_available("USDT"), dec!(700));

        // Release 300
        mgr.release_reservation("USDT", dec!(300));
        assert_eq!(mgr.get_available("USDT"), dec!(1000));
    }

    #[test]
    fn test_insufficient_balance() {
        let mut mgr = BalanceManager::new(ReserveConfig::default());
        mgr.set_balance("USDT", dec!(100));

        assert!(!mgr.reserve("USDT", dec!(200)));
        assert!(!mgr.has_available("USDT", dec!(200)));
    }

    #[test]
    fn test_deduct_and_credit() {
        let mut mgr = BalanceManager::new(ReserveConfig::default());
        mgr.set_balance("USDT", dec!(1000));
        mgr.set_balance("BTC", dec!(0));

        mgr.deduct("USDT", dec!(500));
        mgr.credit("BTC", dec!(0.01));

        assert_eq!(mgr.get_balance("USDT"), dec!(500));
        assert_eq!(mgr.get_balance("BTC"), dec!(0.01));
    }

    #[test]
    fn test_with_balances_constructor() {
        let mut balances = HashMap::new();
        balances.insert("USDT".to_string(), dec!(5000));
        balances.insert("BTC".to_string(), dec!(1));

        let mgr = BalanceManager::with_balances(balances, ReserveConfig::default());

        assert_eq!(mgr.get_balance("USDT"), dec!(5000));
        assert_eq!(mgr.get_balance("BTC"), dec!(1));
    }

    #[test]
    fn test_set_all_balances() {
        let mut mgr = BalanceManager::new(ReserveConfig::default());
        mgr.set_balance("OLD", dec!(100));

        let mut new_balances = HashMap::new();
        new_balances.insert("USDT".to_string(), dec!(5000));

        mgr.set_all_balances(new_balances);

        assert_eq!(mgr.get_balance("OLD"), dec!(0)); // Old balance gone
        assert_eq!(mgr.get_balance("USDT"), dec!(5000));
    }

    #[test]
    fn test_total_in_flight() {
        let mut mgr = BalanceManager::new(ReserveConfig::default());
        mgr.set_balance("USDT", dec!(10000));
        mgr.set_balance("BTC", dec!(10));

        mgr.reserve("USDT", dec!(500));
        mgr.reserve("BTC", dec!(1));

        assert_eq!(mgr.total_in_flight(), dec!(501));
    }

    #[test]
    fn test_combined_reserve_and_in_flight() {
        let mut mgr = BalanceManager::new(ReserveConfig::with_percentage(dec!(0.10)));
        mgr.set_balance("USDT", dec!(1000));

        // 10% reserve = 100, available = 900
        assert_eq!(mgr.get_available("USDT"), dec!(900));

        // Reserve 200 for order
        mgr.reserve("USDT", dec!(200));
        // Available = 1000 - 100 (reserve) - 200 (in-flight) = 700
        assert_eq!(mgr.get_available("USDT"), dec!(700));
    }
}
