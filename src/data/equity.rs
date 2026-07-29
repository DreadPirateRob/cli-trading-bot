//! Equity history persistence layer
//!
//! Provides CRUD operations for tracking equity over time by trading mode.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use tracing::{debug, instrument};

/// A single equity data point
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct EquityPoint {
    pub id: i64,
    pub equity: Decimal,
    pub trading_mode: String,
    pub timestamp: DateTime<Utc>,
}

/// Insert a new equity point for the given trading mode.
///
/// Returns the ID of the inserted record.
#[instrument(skip(pool), fields(equity = %equity, mode = %trading_mode))]
pub async fn insert_equity_point(
    pool: &PgPool,
    equity: Decimal,
    trading_mode: &str,
) -> Result<i64, sqlx::Error> {
    let record = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO equity_history (equity, trading_mode, timestamp)
        VALUES ($1, $2, NOW())
        RETURNING id
        "#,
    )
    .bind(&equity)
    .bind(trading_mode)
    .fetch_one(pool)
    .await?;

    debug!(id = record, "Equity point inserted");
    Ok(record)
}

/// Get equity history for a trading mode since a given timestamp.
///
/// Returns points ordered by timestamp ascending (oldest first).
#[instrument(skip(pool))]
pub async fn get_equity_history(
    pool: &PgPool,
    trading_mode: &str,
    since: DateTime<Utc>,
) -> Result<Vec<EquityPoint>, sqlx::Error> {
    let points = sqlx::query_as::<_, EquityPoint>(
        r#"
        SELECT id, equity, trading_mode, timestamp
        FROM equity_history
        WHERE trading_mode = $1 AND timestamp >= $2
        ORDER BY timestamp ASC
        "#,
    )
    .bind(trading_mode)
    .bind(since)
    .fetch_all(pool)
    .await?;

    debug!(count = points.len(), "Retrieved equity history");
    Ok(points)
}

/// Get the most recent equity point for a trading mode.
///
/// Returns None if no equity history exists for the mode.
#[instrument(skip(pool))]
pub async fn get_latest_equity(
    pool: &PgPool,
    trading_mode: &str,
) -> Result<Option<EquityPoint>, sqlx::Error> {
    let point = sqlx::query_as::<_, EquityPoint>(
        r#"
        SELECT id, equity, trading_mode, timestamp
        FROM equity_history
        WHERE trading_mode = $1
        ORDER BY timestamp DESC
        LIMIT 1
        "#,
    )
    .bind(trading_mode)
    .fetch_optional(pool)
    .await?;

    if let Some(ref p) = point {
        debug!(equity = %p.equity, "Retrieved latest equity");
    } else {
        debug!("No equity history found");
    }
    Ok(point)
}

/// Delete all equity history for a trading mode.
///
/// Used for resetting paper trading or clearing test data.
/// Returns the number of rows deleted.
#[instrument(skip(pool))]
pub async fn delete_equity_history(
    pool: &PgPool,
    trading_mode: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        DELETE FROM equity_history
        WHERE trading_mode = $1
        "#,
    )
    .bind(trading_mode)
    .execute(pool)
    .await?;

    let deleted = result.rows_affected();
    debug!(rows_deleted = deleted, "Deleted equity history");
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equity_point_serialization() {
        use rust_decimal_macros::dec;

        let point = EquityPoint {
            id: 1,
            equity: dec!(10500.50),
            trading_mode: "paper".to_string(),
            timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&point).unwrap();
        assert!(json.contains("10500.50"));
        assert!(json.contains("paper"));
    }
}
