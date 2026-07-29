//! Core risk control data types
//!
//! Defines the fundamental types used throughout the risk system:
//! - RiskDecision: outcome of risk checks (approved, rejected, scaled)
//! - PortfolioState: current portfolio position for risk calculations
//! - OrderRequest: proposed order to be evaluated by risk checks

use rust_decimal::Decimal;

use crate::data::OrderSide;

/// Outcome of a risk check evaluation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskDecision {
    /// Order passes all checks, proceed as requested
    Approved,

    /// Order denied - must not be executed
    Rejected {
        /// Human-readable reason for rejection
        reason: String,
    },

    /// Order scaled down to comply with limits
    Scaled {
        /// Reduced quantity that complies with limits
        new_quantity: Decimal,
        /// Human-readable reason for scaling
        reason: String,
    },
}

impl RiskDecision {
    /// Check if this decision allows the order to proceed (possibly modified)
    pub fn is_allowed(&self) -> bool {
        matches!(self, RiskDecision::Approved | RiskDecision::Scaled { .. })
    }

    /// Check if this decision blocks the order entirely
    pub fn is_rejected(&self) -> bool {
        matches!(self, RiskDecision::Rejected { .. })
    }
}

/// Current portfolio state for risk calculations
#[derive(Debug, Clone)]
pub struct PortfolioState {
    /// Current total portfolio value (cash + positions)
    pub total_equity: Decimal,

    /// Portfolio equity at start of trading day (for daily P&L calc)
    pub day_start_equity: Decimal,

    /// Realized profit/loss for current trading day
    pub daily_pnl: Decimal,
}

impl PortfolioState {
    /// Create a new portfolio state
    pub fn new(total_equity: Decimal, day_start_equity: Decimal, daily_pnl: Decimal) -> Self {
        Self {
            total_equity,
            day_start_equity,
            daily_pnl,
        }
    }

    /// Calculate daily return percentage
    pub fn daily_return_pct(&self) -> Option<Decimal> {
        if self.day_start_equity.is_zero() {
            return None;
        }
        Some(self.daily_pnl / self.day_start_equity)
    }
}

/// Proposed order to be evaluated by risk checks
#[derive(Debug, Clone)]
pub struct OrderRequest {
    /// Strategy that generated this order
    pub strategy_id: String,

    /// Trading pair symbol (e.g., "BTCUSDT")
    pub symbol: String,

    /// Buy or sell
    pub side: OrderSide,

    /// Requested quantity
    pub quantity: Decimal,

    /// Limit price (or current market price for market orders)
    pub price: Decimal,
}

impl OrderRequest {
    /// Create a new order request
    pub fn new(
        strategy_id: impl Into<String>,
        symbol: impl Into<String>,
        side: OrderSide,
        quantity: Decimal,
        price: Decimal,
    ) -> Self {
        Self {
            strategy_id: strategy_id.into(),
            symbol: symbol.into(),
            side,
            quantity,
            price,
        }
    }

    /// Calculate total order value (quantity * price)
    pub fn order_value(&self) -> Option<Decimal> {
        self.quantity.checked_mul(self.price)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_risk_decision_is_allowed() {
        assert!(RiskDecision::Approved.is_allowed());
        assert!(RiskDecision::Scaled {
            new_quantity: dec!(1),
            reason: "test".into()
        }
        .is_allowed());
        assert!(!RiskDecision::Rejected {
            reason: "test".into()
        }
        .is_allowed());
    }

    #[test]
    fn test_risk_decision_is_rejected() {
        assert!(!RiskDecision::Approved.is_rejected());
        assert!(!RiskDecision::Scaled {
            new_quantity: dec!(1),
            reason: "test".into()
        }
        .is_rejected());
        assert!(RiskDecision::Rejected {
            reason: "test".into()
        }
        .is_rejected());
    }

    #[test]
    fn test_portfolio_state_daily_return() {
        let state = PortfolioState::new(dec!(10500), dec!(10000), dec!(500));
        let return_pct = state.daily_return_pct().unwrap();
        assert_eq!(return_pct, dec!(0.05)); // 5% return
    }

    #[test]
    fn test_portfolio_state_daily_return_zero_start() {
        let state = PortfolioState::new(dec!(100), dec!(0), dec!(100));
        assert!(state.daily_return_pct().is_none());
    }

    #[test]
    fn test_order_request_value() {
        let order = OrderRequest::new("test", "BTCUSDT", OrderSide::Buy, dec!(0.5), dec!(50000));
        assert_eq!(order.order_value(), Some(dec!(25000)));
    }
}
