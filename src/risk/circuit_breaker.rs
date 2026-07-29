//! Circuit breaker for catastrophic loss prevention
//!
//! Tracks drawdown from peak equity (high water mark) and halts trading
//! when losses exceed a configured threshold. Once triggered, requires
//! manual reset - this is a safety feature to ensure human review.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

/// Trading state controlled by circuit breaker
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradingState {
    /// Normal operation - trading allowed
    Active,
    /// Trading halted due to circuit breaker trigger
    Halted {
        /// Human-readable reason for halt
        reason: String,
        /// When the circuit breaker was triggered
        triggered_at: DateTime<Utc>,
        /// Drawdown percentage that triggered the halt
        drawdown_pct: Decimal,
        /// Peak equity when halt occurred
        peak_equity: Decimal,
        /// Current equity when halt occurred
        equity_at_halt: Decimal,
    },
}

impl TradingState {
    /// Check if trading is halted
    pub fn is_halted(&self) -> bool {
        matches!(self, TradingState::Halted { .. })
    }

    /// Check if trading is active
    pub fn is_active(&self) -> bool {
        matches!(self, TradingState::Active)
    }
}

impl Default for TradingState {
    fn default() -> Self {
        TradingState::Active
    }
}

impl std::fmt::Display for TradingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TradingState::Active => write!(f, "Active"),
            TradingState::Halted {
                reason,
                triggered_at,
                ..
            } => {
                write!(f, "Halted: {} (at {})", reason, triggered_at)
            }
        }
    }
}

/// Circuit breaker that halts trading when drawdown exceeds threshold
///
/// Tracks the "high water mark" (peak equity) and triggers when current
/// equity drops too far below the peak. Once triggered, remains halted
/// until explicitly reset (manual intervention required for safety).
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    /// Peak equity (high water mark)
    peak_equity: Decimal,
    /// Maximum allowed drawdown as decimal (e.g., 0.15 for 15%)
    max_drawdown_pct: Decimal,
    /// Current trading state
    state: TradingState,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with initial equity and drawdown limit
    pub fn new(initial_equity: Decimal, max_drawdown_pct: Decimal) -> Self {
        Self {
            peak_equity: initial_equity,
            max_drawdown_pct,
            state: TradingState::Active,
        }
    }

    /// Update with current equity, may trigger halt
    ///
    /// Returns the new state (which may be unchanged if already halted
    /// or if drawdown is within limits).
    pub fn update(&mut self, current_equity: Decimal) -> &TradingState {
        // If already halted, stay halted (manual reset required)
        if self.state.is_halted() {
            return &self.state;
        }

        // Update high water mark if equity increased
        if current_equity > self.peak_equity {
            self.peak_equity = current_equity;
            return &self.state;
        }

        // Calculate drawdown from peak
        let drawdown = self.calculate_drawdown(current_equity);

        // Check if drawdown exceeds limit
        if drawdown >= self.max_drawdown_pct {
            self.state = TradingState::Halted {
                reason: format!(
                    "Maximum drawdown exceeded: {:.2}% (limit: {:.2}%)",
                    drawdown * Decimal::ONE_HUNDRED,
                    self.max_drawdown_pct * Decimal::ONE_HUNDRED
                ),
                triggered_at: Utc::now(),
                drawdown_pct: drawdown,
                peak_equity: self.peak_equity,
                equity_at_halt: current_equity,
            };
        }

        &self.state
    }

    /// Calculate current drawdown from peak (as decimal)
    ///
    /// Returns 0 if peak is zero or current > peak
    pub fn calculate_drawdown(&self, current_equity: Decimal) -> Decimal {
        if self.peak_equity.is_zero() || current_equity >= self.peak_equity {
            return Decimal::ZERO;
        }

        (self.peak_equity - current_equity)
            .checked_div(self.peak_equity)
            .unwrap_or(Decimal::ZERO)
    }

    /// Check if trading is currently halted
    pub fn is_halted(&self) -> bool {
        self.state.is_halted()
    }

    /// Get current state
    pub fn state(&self) -> &TradingState {
        &self.state
    }

    /// Get peak equity (high water mark)
    pub fn peak_equity(&self) -> Decimal {
        self.peak_equity
    }

    /// Get configured max drawdown percentage
    pub fn max_drawdown_pct(&self) -> Decimal {
        self.max_drawdown_pct
    }

    /// Manually reset circuit breaker to Active state
    ///
    /// This should only be called after human review of the situation.
    /// The new equity becomes the new high water mark.
    pub fn reset(&mut self, new_equity: Decimal) {
        self.peak_equity = new_equity;
        self.state = TradingState::Active;
        tracing::warn!(
            new_equity = %new_equity,
            max_drawdown_pct = %self.max_drawdown_pct,
            "Circuit breaker manually reset"
        );
    }

    /// Update the max drawdown threshold
    pub fn set_max_drawdown_pct(&mut self, pct: Decimal) {
        self.max_drawdown_pct = pct;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_circuit_breaker_stays_active_within_limit() {
        let mut cb = CircuitBreaker::new(dec!(100000), dec!(0.15)); // 15% max drawdown

        // 10% drawdown - should stay active
        cb.update(dec!(90000));
        assert!(cb.state().is_active());

        // 14% drawdown - still active
        cb.update(dec!(86000));
        assert!(cb.state().is_active());
    }

    #[test]
    fn test_circuit_breaker_triggers_on_threshold() {
        let mut cb = CircuitBreaker::new(dec!(100000), dec!(0.15));

        // Exactly 15% drawdown - should trigger
        cb.update(dec!(85000));
        assert!(cb.is_halted());

        match cb.state() {
            TradingState::Halted { drawdown_pct, .. } => {
                assert_eq!(*drawdown_pct, dec!(0.15));
            }
            _ => panic!("Expected Halted state"),
        }
    }

    #[test]
    fn test_circuit_breaker_triggers_beyond_threshold() {
        let mut cb = CircuitBreaker::new(dec!(100000), dec!(0.15));

        // 20% drawdown - definitely triggers
        cb.update(dec!(80000));
        assert!(cb.is_halted());

        match cb.state() {
            TradingState::Halted { drawdown_pct, .. } => {
                assert_eq!(*drawdown_pct, dec!(0.20));
            }
            _ => panic!("Expected Halted state"),
        }
    }

    #[test]
    fn test_circuit_breaker_stays_halted_on_recovery() {
        let mut cb = CircuitBreaker::new(dec!(100000), dec!(0.15));

        // Trigger halt
        cb.update(dec!(80000));
        assert!(cb.is_halted());

        // Price recovers - should STAY halted
        cb.update(dec!(95000));
        assert!(cb.is_halted());

        // Full recovery - still halted
        cb.update(dec!(110000));
        assert!(cb.is_halted());
    }

    #[test]
    fn test_circuit_breaker_manual_reset() {
        let mut cb = CircuitBreaker::new(dec!(100000), dec!(0.15));

        // Trigger halt
        cb.update(dec!(80000));
        assert!(cb.is_halted());

        // Manual reset with new equity
        cb.reset(dec!(85000));
        assert!(cb.state().is_active());
        assert_eq!(cb.peak_equity(), dec!(85000));

        // Should work normally now
        cb.update(dec!(90000));
        assert!(cb.state().is_active());
        assert_eq!(cb.peak_equity(), dec!(90000)); // New high water mark
    }

    #[test]
    fn test_high_water_mark_updates() {
        let mut cb = CircuitBreaker::new(dec!(100000), dec!(0.15));

        // New high
        cb.update(dec!(110000));
        assert_eq!(cb.peak_equity(), dec!(110000));

        // Lower - no change to peak
        cb.update(dec!(105000));
        assert_eq!(cb.peak_equity(), dec!(110000));

        // New high again
        cb.update(dec!(120000));
        assert_eq!(cb.peak_equity(), dec!(120000));
    }

    #[test]
    fn test_drawdown_calculation() {
        let cb = CircuitBreaker::new(dec!(100000), dec!(0.15));

        assert_eq!(cb.calculate_drawdown(dec!(100000)), dec!(0));
        assert_eq!(cb.calculate_drawdown(dec!(90000)), dec!(0.10));
        assert_eq!(cb.calculate_drawdown(dec!(85000)), dec!(0.15));
        assert_eq!(cb.calculate_drawdown(dec!(50000)), dec!(0.50));

        // Above peak = 0 drawdown
        assert_eq!(cb.calculate_drawdown(dec!(110000)), dec!(0));
    }

    #[test]
    fn test_drawdown_from_new_peak() {
        let mut cb = CircuitBreaker::new(dec!(100000), dec!(0.20));

        // Rise to new peak
        cb.update(dec!(150000));
        assert_eq!(cb.peak_equity(), dec!(150000));

        // 15% drawdown from new peak (still within 20% limit)
        cb.update(dec!(127500));
        assert!(cb.state().is_active());

        // 21% drawdown from new peak - triggers
        cb.update(dec!(118500)); // 150000 * 0.79 = 118500
        assert!(cb.is_halted());
    }

    #[test]
    fn test_trading_state_display() {
        assert_eq!(format!("{}", TradingState::Active), "Active");

        // Halted state includes reason
        let halted = TradingState::Halted {
            reason: "Test halt".to_string(),
            triggered_at: Utc::now(),
            drawdown_pct: dec!(0.20),
            peak_equity: dec!(100000),
            equity_at_halt: dec!(80000),
        };
        let display = format!("{}", halted);
        assert!(display.contains("Halted"));
        assert!(display.contains("Test halt"));
    }

    #[test]
    fn test_trading_state_default() {
        let state = TradingState::default();
        assert!(state.is_active());
        assert!(!state.is_halted());
    }

    #[test]
    fn test_zero_peak_equity_safety() {
        // Edge case: zero peak equity shouldn't cause division by zero
        let cb = CircuitBreaker::new(dec!(0), dec!(0.15));
        assert_eq!(cb.calculate_drawdown(dec!(100)), dec!(0));
    }

    #[test]
    fn test_set_max_drawdown_pct() {
        let mut cb = CircuitBreaker::new(dec!(100000), dec!(0.15));
        assert_eq!(cb.max_drawdown_pct(), dec!(0.15));

        cb.set_max_drawdown_pct(dec!(0.20));
        assert_eq!(cb.max_drawdown_pct(), dec!(0.20));
    }
}
