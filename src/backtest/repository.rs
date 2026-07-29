//! Repository functions for backtest tick queries
//!
//! Provides streaming queries for historical tick replay,
//! enabling memory-efficient backtesting of large datasets.

use chrono::{DateTime, Utc};
use futures_util::Stream;
use sqlx::PgPool;

use crate::data::Tick;

/// Query ticks within a time range as a stream
///
/// Returns ticks ordered by timestamp ascending for chronological replay.
/// Uses streaming (fetch) instead of fetch_all to minimize memory usage
/// when processing large historical datasets.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
/// * `start` - Start of time range (inclusive)
/// * `end` - End of time range (inclusive)
///
/// # Returns
/// A stream of Tick results that can be processed one at a time
pub fn query_ticks_in_range<'a>(
    pool: &'a PgPool,
    symbol: &'a str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> impl Stream<Item = Result<Tick, sqlx::Error>> + 'a {
    sqlx::query_as::<_, Tick>(
        r#"
        SELECT id, timestamp, symbol, price, quantity, trade_id, is_buyer_maker, created_at
        FROM ticks
        WHERE symbol = $1 AND timestamp >= $2 AND timestamp <= $3
        ORDER BY timestamp ASC
        "#,
    )
    .bind(symbol)
    .bind(start)
    .bind(end)
    .fetch(pool)
}

/// Count ticks within a time range
///
/// Useful for validation before starting a backtest to ensure
/// there's data in the requested time range.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `symbol` - Trading pair symbol (e.g., "BTCUSDT")
/// * `start` - Start of time range (inclusive)
/// * `end` - End of time range (inclusive)
///
/// # Returns
/// The count of ticks in the specified range
pub async fn count_ticks_in_range(
    pool: &PgPool,
    symbol: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let result: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM ticks
        WHERE symbol = $1 AND timestamp >= $2 AND timestamp <= $3
        "#,
    )
    .bind(symbol)
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await?;

    Ok(result.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use rust_decimal_macros::dec;

    // Helper to create test tick data
    async fn insert_test_ticks(pool: &PgPool) {
        // Insert test ticks spanning a time range
        let base_time = DateTime::from_timestamp(1672531200, 0).unwrap(); // 2023-01-01 00:00:00

        for i in 0..10 {
            let timestamp = base_time + chrono::Duration::seconds(i * 60); // 1 minute apart
            sqlx::query(
                r#"
                INSERT INTO ticks (timestamp, symbol, price, quantity, trade_id, is_buyer_maker)
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (trade_id) DO NOTHING
                "#,
            )
            .bind(timestamp)
            .bind("BTCUSDT")
            .bind(dec!(50000) + rust_decimal::Decimal::from(i))
            .bind(dec!(0.01))
            .bind(1000000i64 + i)
            .bind(i % 2 == 0)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn test_query_ticks_in_range_streams_results(pool: PgPool) {
        insert_test_ticks(&pool).await;

        let start = DateTime::from_timestamp(1672531200, 0).unwrap();
        let end = DateTime::from_timestamp(1672531800, 0).unwrap(); // 10 minutes later

        let mut stream = query_ticks_in_range(&pool, "BTCUSDT", start, end);
        let mut count = 0;
        let mut prev_timestamp = None;

        while let Some(result) = stream.next().await {
            let tick = result.unwrap();
            assert_eq!(tick.symbol, "BTCUSDT");

            // Verify ordering is ascending
            if let Some(prev) = prev_timestamp {
                assert!(tick.timestamp >= prev, "Ticks should be ordered by timestamp");
            }
            prev_timestamp = Some(tick.timestamp);
            count += 1;
        }

        assert_eq!(count, 10, "Should stream all 10 test ticks");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn test_query_ticks_filters_by_time_range(pool: PgPool) {
        insert_test_ticks(&pool).await;

        // Query only middle portion (minutes 2-5)
        let start = DateTime::from_timestamp(1672531320, 0).unwrap(); // +2 minutes
        let end = DateTime::from_timestamp(1672531500, 0).unwrap(); // +5 minutes

        let mut stream = query_ticks_in_range(&pool, "BTCUSDT", start, end);
        let mut count = 0;

        while let Some(result) = stream.next().await {
            let tick = result.unwrap();
            assert!(tick.timestamp >= start && tick.timestamp <= end);
            count += 1;
        }

        assert_eq!(count, 4, "Should only return ticks in range");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn test_query_ticks_filters_by_symbol(pool: PgPool) {
        insert_test_ticks(&pool).await;

        let start = DateTime::from_timestamp(1672531200, 0).unwrap();
        let end = DateTime::from_timestamp(1672531800, 0).unwrap();

        // Query for non-existent symbol
        let mut stream = query_ticks_in_range(&pool, "ETHUSDT", start, end);
        let mut count = 0;

        while let Some(result) = stream.next().await {
            result.unwrap();
            count += 1;
        }

        assert_eq!(count, 0, "Should return no ticks for different symbol");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn test_count_ticks_in_range(pool: PgPool) {
        insert_test_ticks(&pool).await;

        let start = DateTime::from_timestamp(1672531200, 0).unwrap();
        let end = DateTime::from_timestamp(1672531800, 0).unwrap();

        let count = count_ticks_in_range(&pool, "BTCUSDT", start, end).await.unwrap();
        assert_eq!(count, 10);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn test_count_ticks_empty_range(pool: PgPool) {
        insert_test_ticks(&pool).await;

        // Query outside the data range
        let start = DateTime::from_timestamp(1672617600, 0).unwrap(); // +1 day
        let end = DateTime::from_timestamp(1672704000, 0).unwrap(); // +2 days

        let count = count_ticks_in_range(&pool, "BTCUSDT", start, end).await.unwrap();
        assert_eq!(count, 0);
    }
}
