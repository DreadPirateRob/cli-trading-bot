//! Parameter optimization via grid search with walk-forward validation
//!
//! Provides grid search over RSI-StdDev strategy parameters with
//! train/validate splitting to prevent overfitting.

use chrono::Duration;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use sqlx::PgPool;
use tracing::{info, warn};

use crate::strategy::{RsiStdDevConfig, RsiStdDevStrategy};

use super::engine::run_backtest;
use super::repository::count_ticks_in_range;
use super::types::{BacktestConfig, OptimizationConfig, OptimizationEntry, OptimizationError, OptimizationResult};

/// Default RSI oversold/overbought thresholds (not optimized)
const DEFAULT_RSI_OVERSOLD: f64 = 30.0;
const DEFAULT_RSI_OVERBOUGHT: f64 = 70.0;
const DEFAULT_BASE_QUANTITY: Decimal = dec!(0.01);

/// Generate integer grid values (for period)
fn generate_period_grid(min: usize, max: usize, steps: usize) -> Vec<usize> {
    if steps <= 1 {
        return vec![min];
    }
    let range = max - min;
    (0..steps)
        .map(|i| min + (range * i) / (steps - 1))
        .collect()
}

/// Generate Decimal grid values (for multiplier)
fn generate_decimal_grid(min: Decimal, max: Decimal, steps: usize) -> Vec<Decimal> {
    if steps <= 1 {
        return vec![min];
    }
    let step_size = (max - min) / Decimal::from(steps - 1);
    (0..steps)
        .map(|i| min + step_size * Decimal::from(i))
        .collect()
}

/// Run parameter optimization with walk-forward validation
///
/// Performs grid search over RSI period and StdDev multiplier parameters,
/// running backtests on 70% training data and validating on remaining 30%.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `config` - Optimization configuration
///
/// # Returns
/// `OptimizationResult` with all tested combinations ranked by training performance
pub async fn run_optimization(
    pool: &PgPool,
    config: &OptimizationConfig,
) -> Result<OptimizationResult, OptimizationError> {
    // Validate configuration
    validate_config(config)?;

    // Check tick count
    let total_ticks = count_ticks_in_range(
        pool,
        &config.base_config.symbol,
        config.base_config.start_time,
        config.base_config.end_time,
    ).await.map_err(|e| OptimizationError::BacktestError(e.into()))?;

    if total_ticks < config.min_ticks {
        return Err(OptimizationError::InsufficientData {
            found: total_ticks,
            required: config.min_ticks,
        });
    }

    // Calculate train/validate split point
    let total_duration = config.base_config.end_time - config.base_config.start_time;
    let train_duration = Duration::milliseconds(
        (total_duration.num_milliseconds() as f64 * config.train_ratio) as i64
    );
    let train_end = config.base_config.start_time + train_duration;

    // Build train and validate configs
    let train_config = BacktestConfig {
        start_time: config.base_config.start_time,
        end_time: train_end,
        ..config.base_config.clone()
    };

    let validate_config = BacktestConfig {
        start_time: train_end,
        end_time: config.base_config.end_time,
        ..config.base_config.clone()
    };

    info!(
        "Starting optimization: {} period values x {} multiplier values = {} combinations",
        config.steps, config.steps, config.steps * config.steps
    );
    info!(
        "Train period: {} to {}, Validate period: {} to {}",
        train_config.start_time, train_config.end_time,
        validate_config.start_time, validate_config.end_time
    );

    // Generate parameter grids
    let periods = generate_period_grid(config.period_range.0, config.period_range.1, config.steps);
    let multipliers = generate_decimal_grid(config.multiplier_range.0, config.multiplier_range.1, config.steps);

    let mut results: Vec<OptimizationEntry> = Vec::new();
    let total_combinations = periods.len() * multipliers.len();
    let mut tested = 0;

    // Grid search: iterate over all parameter combinations
    for &period in &periods {
        for &multiplier in &multipliers {
            tested += 1;

            // Create fresh strategy for training
            let strategy_config = RsiStdDevConfig {
                rsi_period: period,
                rsi_oversold: DEFAULT_RSI_OVERSOLD,
                rsi_overbought: DEFAULT_RSI_OVERBOUGHT,
                stddev_period: period, // Use same period for both
                stddev_multiplier: multiplier.to_string().parse().unwrap_or(1.0),
                base_quantity: DEFAULT_BASE_QUANTITY,
            };

            // Run training backtest
            let mut train_strategy = RsiStdDevStrategy::new(strategy_config.clone());
            let train_result = match run_backtest(pool, &mut train_strategy, &train_config).await {
                Ok(result) => result,
                Err(e) => {
                    warn!("Train backtest failed for period={}, mult={}: {}", period, multiplier, e);
                    continue;
                }
            };

            // Run validation backtest with fresh strategy
            let mut validate_strategy = RsiStdDevStrategy::new(strategy_config);
            let validate_result = match run_backtest(pool, &mut validate_strategy, &validate_config).await {
                Ok(result) => result,
                Err(e) => {
                    warn!("Validate backtest failed for period={}, mult={}: {}", period, multiplier, e);
                    continue;
                }
            };

            results.push(OptimizationEntry {
                rsi_period: period,
                stddev_multiplier: multiplier,
                train_result,
                validate_result,
            });

            if tested % 5 == 0 {
                info!("Optimization progress: {}/{} combinations tested", tested, total_combinations);
            }
        }
    }

    // Sort by training return (descending)
    results.sort_by(|a, b| {
        b.train_result.total_return_pct
            .partial_cmp(&a.train_result.total_return_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Determine best and validation status
    let best = results.first().cloned();
    let validation_passed = best
        .as_ref()
        .map(|b| b.validate_result.total_return_pct > Decimal::ZERO)
        .unwrap_or(false);

    info!(
        "Optimization complete: {} combinations tested, best train return: {:?}%, validation passed: {}",
        results.len(),
        best.as_ref().map(|b| b.train_result.total_return_pct),
        validation_passed
    );

    Ok(OptimizationResult {
        results,
        combinations_tested: tested,
        best,
        validation_passed,
    })
}

/// Validate optimization configuration
fn validate_config(config: &OptimizationConfig) -> Result<(), OptimizationError> {
    // Check period range
    if config.period_range.0 >= config.period_range.1 {
        return Err(OptimizationError::InvalidRange {
            param: "period".to_string(),
            min: config.period_range.0.to_string(),
            max: config.period_range.1.to_string(),
        });
    }

    // Check multiplier range
    if config.multiplier_range.0 >= config.multiplier_range.1 {
        return Err(OptimizationError::InvalidRange {
            param: "multiplier".to_string(),
            min: config.multiplier_range.0.to_string(),
            max: config.multiplier_range.1.to_string(),
        });
    }

    // Check train ratio
    if config.train_ratio < 0.1 || config.train_ratio > 0.9 {
        return Err(OptimizationError::InvalidTrainRatio(config.train_ratio));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_period_grid() {
        let grid = generate_period_grid(10, 20, 5);
        assert_eq!(grid, vec![10, 12, 15, 17, 20]);
    }

    #[test]
    fn test_generate_period_grid_single() {
        let grid = generate_period_grid(14, 14, 1);
        assert_eq!(grid, vec![14]);
    }

    #[test]
    fn test_generate_decimal_grid() {
        let grid = generate_decimal_grid(dec!(1.0), dec!(3.0), 5);
        assert_eq!(grid.len(), 5);
        assert_eq!(grid[0], dec!(1.0));
        assert_eq!(grid[4], dec!(3.0));
    }

    #[test]
    fn test_generate_decimal_grid_single() {
        let grid = generate_decimal_grid(dec!(2.0), dec!(2.0), 1);
        assert_eq!(grid, vec![dec!(2.0)]);
    }

    #[test]
    fn test_generate_period_grid_three_steps() {
        let grid = generate_period_grid(10, 20, 3);
        assert_eq!(grid, vec![10, 15, 20]);
    }
}
