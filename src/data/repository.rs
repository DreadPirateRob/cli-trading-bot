//! Database repository functions for tick persistence
//!
//! Uses PostgreSQL UNNEST for efficient batch inserts.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::PgPool;
use tracing::{debug, instrument};

use crate::data::tick::NewTick;

/// Batch insert ticks using PostgreSQL UNNEST
///
/// This is 10-100x faster than individual inserts for batches of 100-10000 rows.
/// Uses ON CONFLICT DO NOTHING to handle duplicate trade_ids gracefully.
#[instrument(skip(pool, ticks), fields(count = ticks.len()))]
pub async fn batch_insert_ticks(pool: &PgPool, ticks: &[NewTick]) -> Result<u64, sqlx::Error> {
    if ticks.is_empty() {
        return Ok(0);
    }

    // Decompose structs into column arrays for UNNEST
    let timestamps: Vec<DateTime<Utc>> = ticks.iter().map(|t| t.timestamp).collect();
    let symbols: Vec<&str> = ticks.iter().map(|t| t.symbol.as_str()).collect();
    let prices: Vec<Decimal> = ticks.iter().map(|t| t.price).collect();
    let quantities: Vec<Decimal> = ticks.iter().map(|t| t.quantity).collect();
    let trade_ids: Vec<i64> = ticks.iter().map(|t| t.trade_id).collect();
    let is_buyer_makers: Vec<bool> = ticks.iter().map(|t| t.is_buyer_maker).collect();

    let result = sqlx::query(
        r#"
        INSERT INTO ticks (timestamp, symbol, price, quantity, trade_id, is_buyer_maker)
        SELECT * FROM UNNEST(
            $1::TIMESTAMPTZ[],
            $2::TEXT[],
            $3::NUMERIC[],
            $4::NUMERIC[],
            $5::BIGINT[],
            $6::BOOLEAN[]
        )
        ON CONFLICT (trade_id) DO NOTHING
        "#,
    )
    .bind(&timestamps)
    .bind(&symbols)
    .bind(&prices)
    .bind(&quantities)
    .bind(&trade_ids)
    .bind(&is_buyer_makers)
    .execute(pool)
    .await?;

    let rows_inserted = result.rows_affected();
    debug!(rows_inserted, "Batch insert complete");

    Ok(rows_inserted)
}
