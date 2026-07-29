//! Daily loss limit monitoring and trading halt mechanism
//!
//! Implements RISK-03: Daily loss limit check that rejects orders when
//! accumulated P&L drops below a configured threshold. This is a critical
//! circuit breaker that prevents runaway losses within a single trading day.
//!
//! Key features:
//! - Tracks daily P&L as percentage of day-start equity
//! - UTC day boundary detection with automatic reset
//! - Trading halt when daily loss limit is exceeded

use chrono::{DateTime, Datelike, Utc};
use rust_decimal::Decimal;

use crate::risk::checks::RiskCheck;
use crate::risk::config::StrategyRiskParams;
use crate::risk::types::{OrderRequest, PortfolioState, RiskDecision};

// ============================================================================
// Daily P&L State Tracking
// ============================================================================

/// Tracks daily P&L with automatic day boundary detection
///
/// Uses UTC for day boundaries to ensure consistency across timezones.
/// Automatically resets accumulated P&L when a new trading day begins.
#[derive(Debug, Clone)]
pub struct DailyPnlState {
    /// Equity at start of current trading day
    pub day_start_equity: Decimal,
    /// Accumulated realized P&L for current day
    pub daily_pnl: Decimal,
    /// Day of year for current trading day (1-366)
    current_day: u32,
    /// Year of current trading day
    current_year: i32,
}

impl DailyPnlState {
    /// Create new state with initial equity
    ///
    /// Initializes with current UTC time as the starting day.
    pub fn new(initial_equity: Decimal) -> Self {
        let now = Utc::now();
        Self {
            day_start_equity: initial_equity,
            daily_pnl: Decimal::ZERO,
            current_day: now.ordinal(),
            current_year: now.year(),
        }
    }

    /// Create with specific timestamp (for testing)
    pub fn new_with_time(initial_equity: Decimal, time: DateTime<Utc>) -> Self {
        Self {
            day_start_equity: initial_equity,
            daily_pnl: Decimal::ZERO,
            current_day: time.ordinal(),
            current_year: time.year(),
        }
    }

    /// Record a closed position's P&L
    ///
    /// Automatically detects day boundary and resets if new day.
    /// Returns true if this is a new day (state was reset).
    pub fn record_pnl(&mut self, pnl: Decimal, current_equity: Decimal) -> bool {
        let now = Utc::now();
        let is_new_day = self.check_day_boundary(now, current_equity);

        // Use checked_add for safety
        self.daily_pnl = self
            .daily_pnl
            .checked_add(pnl)
            .unwrap_or(self.daily_pnl);

        is_new_day
    }

    /// Record P&L with explicit timestamp (for testing)
    pub fn record_pnl_at(
        &mut self,
        pnl: Decimal,
        current_equity: Decimal,
        time: DateTime<Utc>,
    ) -> bool {
        let is_new_day = self.check_day_boundary(time, current_equity);

        self.daily_pnl = self
            .daily_pnl
            .checked_add(pnl)
            .unwrap_or(self.daily_pnl);

        is_new_day
    }

    /// Check if we've crossed into a new day, reset if so
    fn check_day_boundary(&mut self, now: DateTime<Utc>, current_equity: Decimal) -> bool {
        let day = now.ordinal();
        let year = now.year();

        if day != self.current_day || year != self.current_year {
            // New day - reset state
            self.day_start_equity = current_equity;
            self.daily_pnl = Decimal::ZERO;
            self.current_day = day;
            self.current_year = year;
            true
        } else {
            false
        }
    }

    /// Calculate daily loss as percentage of day-start equity
    ///
    /// Returns negative value for losses, positive for gains.
    /// Returns ZERO if day_start_equity is zero to avoid division errors.
    pub fn daily_loss_pct(&self) -> Decimal {
        if self.day_start_equity.is_zero() {
            return Decimal::ZERO;
        }

        self.daily_pnl
            .checked_div(self.day_start_equity)
            .unwrap_or(Decimal::ZERO)
    }

    /// Check if daily loss limit is exceeded
    ///
    /// `limit_pct` should be positive (e.g., 0.05 for 5% limit).
    /// Returns true if daily losses have exceeded the limit.
    pub fn is_limit_exceeded(&self, limit_pct: Decimal) -> bool {
        // daily_loss_pct() returns negative for losses
        // If daily_loss_pct is -0.06 and limit is 0.05, we've exceeded
        self.daily_loss_pct() <= -limit_pct
    }
}

// ============================================================================
// Daily Loss Limit Check
// ============================================================================

/// Daily loss limit check
///
/// Rejects all orders if daily P&L has exceeded the configured threshold.
/// This is a circuit breaker to prevent runaway losses within a trading day.
#[derive(Debug, Default)]
pub struct DailyLossLimitCheck;

impl DailyLossLimitCheck {
    /// Create a new daily loss limit check
    pub fn new() -> Self {
        Self
    }
}

impl RiskCheck for DailyLossLimitCheck {
    /// Check if daily loss limit allows trading
    ///
    /// Returns `RiskDecision::Rejected` if daily loss exceeds the configured
    /// `daily_loss_limit_pct` in the strategy parameters.
    fn check(
        &self,
        _order: &OrderRequest,
        state: &PortfolioState,
        params: &StrategyRiskParams,
    ) -> RiskDecision {
        // Calculate daily loss percentage
        let daily_loss_pct = if state.day_start_equity.is_zero() {
            Decimal::ZERO
        } else {
            state
                .daily_pnl
                .checked_div(state.day_start_equity)
                .unwrap_or(Decimal::ZERO)
        };

        // limit is positive (e.g., 0.05 for 5%)
        // daily_loss_pct is negative when losing
        let limit = -params.daily_loss_limit_pct;

        if daily_loss_pct <= limit {
            RiskDecision::Rejected {
                reason: format!(
                    "Daily loss limit exceeded: {:.2}% loss (limit: {:.2}%)",
                    (daily_loss_pct * Decimal::ONE_HUNDRED).abs(),
                    (params.daily_loss_limit_pct * Decimal::ONE_HUNDRED)
                ),
            }
        } else {
            RiskDecision::Approved
        }
    }

    /// Get the name of this check
    fn name(&self) -> &'static str {
        "DailyLossLimit"
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::OrderSide;
    use chrono::TimeDelta;
    use rust_decimal_macros::dec;

    #[test]
    fn test_daily_pnl_tracking() {
        let mut state = DailyPnlState::new(dec!(100000));

        // Record some losses
        state.record_pnl(dec!(-1000), dec!(99000));
        assert_eq!(state.daily_pnl, dec!(-1000));

        state.record_pnl(dec!(-2000), dec!(97000));
        assert_eq!(state.daily_pnl, dec!(-3000));

        // Daily loss percentage: -3000 / 100000 = -0.03 (-3%)
        assert_eq!(state.daily_loss_pct(), dec!(-0.03));
    }

    #[test]
    fn test_daily_pnl_tracking_gains() {
        let mut state = DailyPnlState::new(dec!(100000));

        // Record some gains
        state.record_pnl(dec!(2000), dec!(102000));
        assert_eq!(state.daily_pnl, dec!(2000));

        // Daily gain percentage: 2000 / 100000 = 0.02 (2%)
        assert_eq!(state.daily_loss_pct(), dec!(0.02));
    }

    #[test]
    fn test_daily_loss_limit_not_exceeded() {
        let state = DailyPnlState::new(dec!(100000));

        // 5% limit - no losses yet
        assert!(!state.is_limit_exceeded(dec!(0.05)));
    }

    #[test]
    fn test_daily_loss_limit_exceeded() {
        let mut state = DailyPnlState::new(dec!(100000));
        state.record_pnl(dec!(-5500), dec!(94500));

        // -5.5% exceeds 5% limit
        assert!(state.is_limit_exceeded(dec!(0.05)));
        // -5.5% does not exceed 6% limit
        assert!(!state.is_limit_exceeded(dec!(0.06)));
    }

    #[test]
    fn test_daily_loss_limit_exactly_at_threshold() {
        let mut state = DailyPnlState::new(dec!(100000));
        state.record_pnl(dec!(-5000), dec!(95000));

        // -5% equals 5% limit - should be exceeded (<=)
        assert!(state.is_limit_exceeded(dec!(0.05)));
    }

    #[test]
    fn test_day_boundary_reset() {
        let yesterday = Utc::now() - TimeDelta::days(1);
        let mut state = DailyPnlState::new_with_time(dec!(100000), yesterday);

        // Record loss yesterday
        state.record_pnl_at(dec!(-5000), dec!(95000), yesterday);
        assert_eq!(state.daily_pnl, dec!(-5000));

        // Today's trade should reset
        let today = Utc::now();
        let is_new_day = state.record_pnl_at(dec!(-1000), dec!(94000), today);

        assert!(is_new_day);
        assert_eq!(state.daily_pnl, dec!(-1000)); // Reset to just today's loss
        assert_eq!(state.day_start_equity, dec!(94000)); // New day start equity
    }

    #[test]
    fn test_year_boundary_reset() {
        // Simulate December 31st to January 1st transition
        let dec_31 = chrono::NaiveDate::from_ymd_opt(2025, 12, 31)
            .unwrap()
            .and_hms_opt(23, 59, 0)
            .unwrap()
            .and_utc();

        let jan_1 = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 1, 0)
            .unwrap()
            .and_utc();

        let mut state = DailyPnlState::new_with_time(dec!(100000), dec_31);
        state.record_pnl_at(dec!(-3000), dec!(97000), dec_31);
        assert_eq!(state.daily_pnl, dec!(-3000));

        // New year should trigger reset
        let is_new_day = state.record_pnl_at(dec!(-500), dec!(96500), jan_1);
        assert!(is_new_day);
        assert_eq!(state.daily_pnl, dec!(-500));
    }

    #[test]
    fn test_zero_equity_handling() {
        let state = DailyPnlState::new(Decimal::ZERO);

        // Should not panic, returns zero
        assert_eq!(state.daily_loss_pct(), Decimal::ZERO);
        assert!(!state.is_limit_exceeded(dec!(0.05)));
    }

    #[test]
    fn test_daily_loss_check_rejects_when_exceeded() {
        let check = DailyLossLimitCheck::new();

        let order = OrderRequest::new("test", "BTCUSDT", OrderSide::Buy, dec!(1), dec!(50000));

        // State with 6% loss today
        let state = PortfolioState::new(dec!(94000), dec!(100000), dec!(-6000));

        let params = StrategyRiskParams {
            max_position_size_pct: dec!(0.10),
            stop_loss_pct: dec!(0.02),
            daily_loss_limit_pct: dec!(0.05), // 5% limit
            max_drawdown_pct: dec!(0.20),
        };

        let decision = check.check(&order, &state, &params);
        assert!(matches!(decision, RiskDecision::Rejected { .. }));

        if let RiskDecision::Rejected { reason } = decision {
            assert!(reason.contains("Daily loss limit exceeded"));
            assert!(reason.contains("6.00%"));
        }
    }

    #[test]
    fn test_daily_loss_check_approves_within_limit() {
        let check = DailyLossLimitCheck::new();

        let order = OrderRequest::new("test", "BTCUSDT", OrderSide::Buy, dec!(1), dec!(50000));

        // State with 3% loss today (within 5% limit)
        let state = PortfolioState::new(dec!(97000), dec!(100000), dec!(-3000));

        let params = StrategyRiskParams {
            max_position_size_pct: dec!(0.10),
            stop_loss_pct: dec!(0.02),
            daily_loss_limit_pct: dec!(0.05),
            max_drawdown_pct: dec!(0.20),
        };

        let decision = check.check(&order, &state, &params);
        assert!(matches!(decision, RiskDecision::Approved));
    }

    #[test]
    fn test_daily_loss_check_approves_gains() {
        let check = DailyLossLimitCheck::new();

        let order = OrderRequest::new("test", "BTCUSDT", OrderSide::Buy, dec!(1), dec!(50000));

        // State with gains (positive P&L)
        let state = PortfolioState::new(dec!(105000), dec!(100000), dec!(5000));

        let params = StrategyRiskParams {
            max_position_size_pct: dec!(0.10),
            stop_loss_pct: dec!(0.02),
            daily_loss_limit_pct: dec!(0.05),
            max_drawdown_pct: dec!(0.20),
        };

        let decision = check.check(&order, &state, &params);
        assert!(matches!(decision, RiskDecision::Approved));
    }

    #[test]
    fn test_daily_loss_check_name() {
        let check = DailyLossLimitCheck::new();
        assert_eq!(check.name(), "DailyLossLimit");
    }

    #[test]
    fn test_daily_loss_check_default() {
        let check = DailyLossLimitCheck::default();
        assert_eq!(check.name(), "DailyLossLimit");
    }
}
