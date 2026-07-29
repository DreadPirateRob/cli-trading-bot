//! Performance metrics calculations for backtest analysis
//!
//! Provides comprehensive strategy evaluation through:
//! - Sharpe ratio (risk-adjusted return)
//! - Sortino ratio (downside-only volatility)
//! - Maximum drawdown (peak-to-trough decline)
//! - Win rate (profitable trade percentage)

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use serde::Serialize;

use crate::data::OrderSide;

/// Record of a completed or open trade for metrics calculation
#[derive(Debug, Clone, Serialize)]
pub struct TradeRecord {
    /// Time when position was entered
    pub entry_time: DateTime<Utc>,
    /// Time when position was exited (None if still open)
    pub exit_time: Option<DateTime<Utc>>,
    /// Trade direction (Buy or Sell)
    pub side: OrderSide,
    /// Price at entry
    pub entry_price: Decimal,
    /// Price at exit (None if still open)
    pub exit_price: Option<Decimal>,
    /// Position size
    pub quantity: Decimal,
    /// Realized profit/loss (None if still open)
    pub pnl: Option<Decimal>,
    /// Commission paid for the trade
    pub commission: Decimal,
}

/// Calculate win rate from trade records
///
/// Win rate is the percentage of completed trades that were profitable.
/// Only trades with a PnL value are considered (open trades excluded).
///
/// # Arguments
/// * `trades` - Slice of trade records
///
/// # Returns
/// Win rate as a decimal (0.0 to 1.0), or ZERO if no completed trades
pub fn calculate_win_rate(trades: &[TradeRecord]) -> Decimal {
    let completed: Vec<_> = trades.iter().filter(|t| t.pnl.is_some()).collect();
    if completed.is_empty() {
        return Decimal::ZERO;
    }
    let wins = completed
        .iter()
        .filter(|t| t.pnl.unwrap() > Decimal::ZERO)
        .count();
    Decimal::from(wins as u32) / Decimal::from(completed.len() as u32)
}

/// Calculate maximum drawdown from equity curve
///
/// Maximum drawdown measures the largest peak-to-trough decline
/// in portfolio value, expressed as a percentage of the peak.
///
/// # Arguments
/// * `equity_curve` - Slice of equity values over time
///
/// # Returns
/// Maximum drawdown as a decimal (0.0 to 1.0), or ZERO if empty
pub fn calculate_max_drawdown(equity_curve: &[Decimal]) -> Decimal {
    if equity_curve.is_empty() {
        return Decimal::ZERO;
    }
    let mut peak = equity_curve[0];
    let mut max_dd = Decimal::ZERO;
    for &equity in equity_curve {
        if equity > peak {
            peak = equity;
        }
        if !peak.is_zero() {
            let dd = (peak - equity) / peak;
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    max_dd
}

/// Calculate periodic returns from equity curve
///
/// Converts an equity curve into periodic returns (e.g., daily returns).
/// Each return is calculated as (current - previous) / previous.
///
/// # Arguments
/// * `equity_curve` - Slice of equity values over time
///
/// # Returns
/// Vector of periodic returns, empty if fewer than 2 data points
pub fn calculate_returns(equity_curve: &[Decimal]) -> Vec<Decimal> {
    if equity_curve.len() < 2 {
        return vec![];
    }
    equity_curve
        .windows(2)
        .filter_map(|w| {
            if w[0].is_zero() {
                None
            } else {
                Some((w[1] - w[0]) / w[0])
            }
        })
        .collect()
}

/// Calculate Sharpe ratio from periodic returns
///
/// The Sharpe ratio measures risk-adjusted return by comparing
/// excess return (over risk-free rate) to volatility.
///
/// # Arguments
/// * `returns` - Periodic returns (e.g., daily returns as decimals)
/// * `risk_free_rate` - Annualized risk-free rate (e.g., 0.05 for 5%)
/// * `periods_per_year` - Number of return periods per year (365 for daily, 252 for trading days)
///
/// # Returns
/// Sharpe ratio or None if returns are empty or standard deviation is zero
pub fn calculate_sharpe_ratio(
    returns: &[Decimal],
    risk_free_rate: Decimal,
    periods_per_year: u32,
) -> Option<Decimal> {
    if returns.is_empty() {
        return None;
    }

    let n = Decimal::from(returns.len() as u32);
    let mean_return = returns.iter().copied().sum::<Decimal>() / n;

    // Variance = sum((r - mean)^2) / n
    let variance = returns
        .iter()
        .map(|r| {
            let diff = *r - mean_return;
            diff * diff
        })
        .sum::<Decimal>()
        / n;

    let std_dev = variance.sqrt()?;
    if std_dev.is_zero() {
        return None;
    }

    // Annualize
    let periods = Decimal::from(periods_per_year);
    let annualized_return = mean_return * periods;
    let annualization_factor = periods.sqrt()?;
    let annualized_std = std_dev * annualization_factor;

    Some((annualized_return - risk_free_rate) / annualized_std)
}

/// Calculate Sortino ratio from periodic returns
///
/// Similar to Sharpe ratio, but only penalizes downside volatility.
/// This is often preferred for strategies where upside volatility is desirable.
///
/// # Arguments
/// * `returns` - Periodic returns as decimals
/// * `risk_free_rate` - Annualized risk-free rate
/// * `periods_per_year` - Number of return periods per year
///
/// # Returns
/// Sortino ratio or None if insufficient data or no downside deviation
pub fn calculate_sortino_ratio(
    returns: &[Decimal],
    risk_free_rate: Decimal,
    periods_per_year: u32,
) -> Option<Decimal> {
    if returns.is_empty() {
        return None;
    }

    let n = Decimal::from(returns.len() as u32);
    let mean_return = returns.iter().copied().sum::<Decimal>() / n;

    // Downside deviation: sqrt(sum(min(r, 0)^2) / n)
    let downside_variance = returns
        .iter()
        .map(|r| {
            if *r < Decimal::ZERO {
                *r * *r
            } else {
                Decimal::ZERO
            }
        })
        .sum::<Decimal>()
        / n;

    let downside_dev = downside_variance.sqrt()?;
    if downside_dev.is_zero() {
        return None; // No downside = infinite Sortino, return None
    }

    // Annualize
    let periods = Decimal::from(periods_per_year);
    let annualized_return = mean_return * periods;
    let annualization_factor = periods.sqrt()?;
    let annualized_downside = downside_dev * annualization_factor;

    Some((annualized_return - risk_free_rate) / annualized_downside)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_trade(pnl: Option<Decimal>) -> TradeRecord {
        TradeRecord {
            entry_time: Utc::now(),
            exit_time: pnl.map(|_| Utc::now()),
            side: OrderSide::Buy,
            entry_price: dec!(100),
            exit_price: pnl.map(|p| if p > Decimal::ZERO { dec!(110) } else { dec!(90) }),
            quantity: dec!(1),
            pnl,
            commission: dec!(0.1),
        }
    }

    #[test]
    fn test_win_rate_all_winners() {
        let trades = vec![make_trade(Some(dec!(10))), make_trade(Some(dec!(5)))];
        assert_eq!(calculate_win_rate(&trades), dec!(1));
    }

    #[test]
    fn test_win_rate_mixed() {
        let trades = vec![
            make_trade(Some(dec!(10))),  // win
            make_trade(Some(dec!(-10))), // loss
        ];
        assert_eq!(calculate_win_rate(&trades), dec!(0.5));
    }

    #[test]
    fn test_win_rate_empty() {
        let trades: Vec<TradeRecord> = vec![];
        assert_eq!(calculate_win_rate(&trades), Decimal::ZERO);
    }

    #[test]
    fn test_win_rate_ignores_open_trades() {
        let trades = vec![
            make_trade(Some(dec!(10))), // closed, win
            make_trade(None),           // open, ignored
        ];
        assert_eq!(calculate_win_rate(&trades), dec!(1));
    }

    #[test]
    fn test_max_drawdown_simple() {
        // Goes up to 120, then drops to 90 = 25% drawdown
        let equity = vec![dec!(100), dec!(120), dec!(110), dec!(90), dec!(100)];
        let mdd = calculate_max_drawdown(&equity);
        assert_eq!(mdd, dec!(0.25)); // (120-90)/120 = 0.25
    }

    #[test]
    fn test_max_drawdown_no_drawdown() {
        let equity = vec![dec!(100), dec!(110), dec!(120), dec!(130)];
        assert_eq!(calculate_max_drawdown(&equity), Decimal::ZERO);
    }

    #[test]
    fn test_max_drawdown_empty() {
        let equity: Vec<Decimal> = vec![];
        assert_eq!(calculate_max_drawdown(&equity), Decimal::ZERO);
    }

    #[test]
    fn test_max_drawdown_multiple_drawdowns() {
        // First drawdown: 100 -> 80 (20%)
        // Second drawdown: 120 -> 60 (50%) - this is larger
        let equity = vec![dec!(100), dec!(80), dec!(120), dec!(60), dec!(100)];
        let mdd = calculate_max_drawdown(&equity);
        assert_eq!(mdd, dec!(0.5)); // (120-60)/120 = 0.5
    }

    #[test]
    fn test_calculate_returns() {
        let equity = vec![dec!(100), dec!(110), dec!(99)];
        let returns = calculate_returns(&equity);
        assert_eq!(returns.len(), 2);
        assert_eq!(returns[0], dec!(0.1)); // (110-100)/100
        assert_eq!(returns[1], dec!(-0.1)); // (99-110)/110
    }

    #[test]
    fn test_calculate_returns_empty() {
        let equity: Vec<Decimal> = vec![];
        assert!(calculate_returns(&equity).is_empty());
    }

    #[test]
    fn test_calculate_returns_single() {
        let equity = vec![dec!(100)];
        assert!(calculate_returns(&equity).is_empty());
    }

    #[test]
    fn test_calculate_returns_skips_zero() {
        // Zero in equity curve should be skipped
        let equity = vec![dec!(100), dec!(0), dec!(50)];
        let returns = calculate_returns(&equity);
        // First return is (0-100)/100 = -1
        // Second would be (50-0)/0 = undefined, skipped
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0], dec!(-1));
    }

    #[test]
    fn test_sharpe_ratio_positive() {
        // Consistent positive returns
        let returns = vec![dec!(0.01), dec!(0.02), dec!(0.01), dec!(0.015)];
        let sharpe = calculate_sharpe_ratio(&returns, dec!(0.02), 252);
        assert!(sharpe.is_some());
        assert!(sharpe.unwrap() > Decimal::ZERO);
    }

    #[test]
    fn test_sharpe_ratio_empty() {
        let returns: Vec<Decimal> = vec![];
        assert!(calculate_sharpe_ratio(&returns, dec!(0.02), 252).is_none());
    }

    #[test]
    fn test_sharpe_ratio_zero_std() {
        // All identical returns = zero std dev
        let returns = vec![dec!(0.01), dec!(0.01), dec!(0.01)];
        assert!(calculate_sharpe_ratio(&returns, dec!(0.02), 252).is_none());
    }

    #[test]
    fn test_sortino_ratio_with_downside() {
        // Mix of positive and negative returns
        let returns = vec![dec!(0.05), dec!(-0.02), dec!(0.03), dec!(-0.01)];
        let sortino = calculate_sortino_ratio(&returns, dec!(0.02), 252);
        assert!(sortino.is_some());
    }

    #[test]
    fn test_sortino_ratio_no_downside() {
        // All positive returns = no downside deviation
        let returns = vec![dec!(0.01), dec!(0.02), dec!(0.03)];
        assert!(calculate_sortino_ratio(&returns, dec!(0.02), 252).is_none());
    }

    #[test]
    fn test_sortino_empty() {
        let returns: Vec<Decimal> = vec![];
        assert!(calculate_sortino_ratio(&returns, dec!(0.02), 252).is_none());
    }

    #[test]
    fn test_sortino_higher_than_sharpe_with_upside_volatility() {
        // Returns with more upside volatility than downside
        // Sortino should be higher than Sharpe because it ignores upside vol
        let returns = vec![
            dec!(0.10),  // big up
            dec!(-0.01), // small down
            dec!(0.08),  // big up
            dec!(-0.02), // small down
        ];
        let sharpe = calculate_sharpe_ratio(&returns, dec!(0.02), 252);
        let sortino = calculate_sortino_ratio(&returns, dec!(0.02), 252);

        assert!(sharpe.is_some());
        assert!(sortino.is_some());
        // Sortino should be higher because downside volatility is lower
        assert!(sortino.unwrap() > sharpe.unwrap());
    }
}
