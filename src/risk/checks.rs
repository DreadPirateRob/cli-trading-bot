//! Risk check trait and implementations
//!
//! Defines the RiskCheck trait for composable risk rules and
//! implements standard checks like MaxPositionSizeCheck.

use rust_decimal::Decimal;

pub use crate::risk::config::StrategyRiskParams;
use crate::risk::types::{OrderRequest, PortfolioState, RiskDecision};

/// Trait for composable risk checks
///
/// Each risk check evaluates an order request against portfolio state
/// and strategy parameters, returning a decision to approve, reject,
/// or scale the order.
pub trait RiskCheck: Send + Sync {
    /// Evaluate an order against this risk rule
    fn check(
        &self,
        order: &OrderRequest,
        state: &PortfolioState,
        params: &StrategyRiskParams,
    ) -> RiskDecision;

    /// Human-readable name of this risk check
    fn name(&self) -> &'static str;
}

/// Maximum position size check (RISK-01)
///
/// Ensures no single position exceeds the configured percentage of
/// total portfolio equity. Orders that exceed the limit are scaled
/// down rather than rejected, allowing partial execution.
#[derive(Debug, Default)]
pub struct MaxPositionSizeCheck;

impl MaxPositionSizeCheck {
    /// Create a new maximum position size check
    pub fn new() -> Self {
        Self
    }
}

impl RiskCheck for MaxPositionSizeCheck {
    fn check(
        &self,
        order: &OrderRequest,
        state: &PortfolioState,
        params: &StrategyRiskParams,
    ) -> RiskDecision {
        // Calculate order value using checked multiplication for overflow safety
        // On overflow, treat as exceeding limit (conservative approach)
        let order_value = order
            .quantity
            .checked_mul(order.price)
            .unwrap_or(Decimal::MAX);

        // Calculate maximum allowed position value
        let max_allowed = state
            .total_equity
            .checked_mul(params.max_position_size_pct)
            .unwrap_or(Decimal::ZERO);

        // If order is within limits, approve
        if order_value <= max_allowed {
            return RiskDecision::Approved;
        }

        // Order exceeds limit - scale down quantity
        // scaled_quantity = max_allowed / price
        // Use checked_div and fall back to ZERO if price is zero or overflow
        let scaled_quantity = if order.price.is_zero() {
            Decimal::ZERO
        } else {
            max_allowed
                .checked_div(order.price)
                .unwrap_or(Decimal::ZERO)
        };

        // If scaled quantity is effectively zero, reject entirely
        if scaled_quantity.is_zero() {
            return RiskDecision::Rejected {
                reason: format!(
                    "Order value {} exceeds max position size ({:.1}% of {} = {}), cannot scale",
                    order_value,
                    params.max_position_size_pct * Decimal::ONE_HUNDRED,
                    state.total_equity,
                    max_allowed,
                ),
            };
        }

        RiskDecision::Scaled {
            new_quantity: scaled_quantity,
            reason: format!(
                "Scaled from {} to {} (max position size: {:.1}% of {} = {})",
                order.quantity,
                scaled_quantity,
                params.max_position_size_pct * Decimal::ONE_HUNDRED,
                state.total_equity,
                max_allowed,
            ),
        }
    }

    fn name(&self) -> &'static str {
        "MaxPositionSize"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::OrderSide;
    use rust_decimal_macros::dec;

    fn test_order() -> OrderRequest {
        OrderRequest::new("test-strategy", "BTCUSDT", OrderSide::Buy, dec!(1), dec!(50000))
    }

    fn test_state() -> PortfolioState {
        PortfolioState::new(dec!(100000), dec!(100000), Decimal::ZERO)
    }

    fn test_params() -> StrategyRiskParams {
        StrategyRiskParams::new(
            dec!(0.10), // 10% max position
            dec!(0.02), // 2% stop loss
            dec!(0.05), // 5% daily loss limit
            dec!(0.20), // 20% max drawdown
        )
    }

    #[test]
    fn test_max_position_size_approves_within_limit() {
        let check = MaxPositionSizeCheck::new();
        let order = OrderRequest::new("test", "BTCUSDT", OrderSide::Buy, dec!(0.1), dec!(50000));
        // 0.1 * 50000 = 5000, which is 5% of 100000 (within 10% limit)

        let decision = check.check(&order, &test_state(), &test_params());
        assert_eq!(decision, RiskDecision::Approved);
    }

    #[test]
    fn test_max_position_size_scales_exceeding_order() {
        let check = MaxPositionSizeCheck::new();
        let order = OrderRequest::new("test", "BTCUSDT", OrderSide::Buy, dec!(0.5), dec!(50000));
        // 0.5 * 50000 = 25000, which is 25% of 100000 (exceeds 10% limit)
        // max_allowed = 100000 * 0.10 = 10000
        // scaled_quantity = 10000 / 50000 = 0.2

        let decision = check.check(&order, &test_state(), &test_params());
        match decision {
            RiskDecision::Scaled { new_quantity, .. } => {
                assert_eq!(new_quantity, dec!(0.2));
            }
            _ => panic!("Expected Scaled decision, got {:?}", decision),
        }
    }

    #[test]
    fn test_max_position_size_exact_limit() {
        let check = MaxPositionSizeCheck::new();
        let order = OrderRequest::new("test", "BTCUSDT", OrderSide::Buy, dec!(0.2), dec!(50000));
        // 0.2 * 50000 = 10000, which is exactly 10% of 100000

        let decision = check.check(&order, &test_state(), &test_params());
        assert_eq!(decision, RiskDecision::Approved);
    }

    #[test]
    fn test_max_position_size_zero_equity() {
        let check = MaxPositionSizeCheck::new();
        let order = test_order();
        let state = PortfolioState::new(Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);

        let decision = check.check(&order, &state, &test_params());
        match decision {
            RiskDecision::Rejected { reason } => {
                assert!(reason.contains("cannot scale"));
            }
            _ => panic!("Expected Rejected decision, got {:?}", decision),
        }
    }

    #[test]
    fn test_max_position_size_zero_price() {
        let check = MaxPositionSizeCheck::new();
        let order = OrderRequest::new("test", "BTCUSDT", OrderSide::Buy, dec!(1), Decimal::ZERO);

        let decision = check.check(&order, &test_state(), &test_params());
        // Zero price means order value is 0, which is within any limit
        assert_eq!(decision, RiskDecision::Approved);
    }

    #[test]
    fn test_risk_check_name() {
        let check = MaxPositionSizeCheck::new();
        assert_eq!(check.name(), "MaxPositionSize");
    }

    #[test]
    fn test_strategy_risk_params_new() {
        let params = StrategyRiskParams::new(dec!(0.15), dec!(0.03), dec!(0.06), dec!(0.25));
        assert_eq!(params.max_position_size_pct, dec!(0.15));
        assert_eq!(params.stop_loss_pct, dec!(0.03));
        assert_eq!(params.daily_loss_limit_pct, dec!(0.06));
        assert_eq!(params.max_drawdown_pct, dec!(0.25));
    }
}
