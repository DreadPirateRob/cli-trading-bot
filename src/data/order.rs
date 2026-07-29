//! Order data structures for trade tracking
//!
//! Orders track executed trades with entry/exit prices and P&L.
//! An order starts as "open" (NULL exit fields) and is "closed"
//! when the position is exited.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tracing::{debug, instrument};

/// Order side - buy or sell
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderSide::Buy => "buy",
            OrderSide::Sell => "sell",
        }
    }
}

impl std::fmt::Display for OrderSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Order as stored in database (includes auto-generated fields)
#[derive(Debug, Clone, FromRow)]
pub struct Order {
    pub id: i64,
    pub order_id: String,
    pub symbol: String,
    pub side: String,
    pub entry_price: Decimal,
    pub exit_price: Option<Decimal>,
    pub quantity: Decimal,
    pub pnl: Option<Decimal>,
    pub reason: String,
    pub entry_time: DateTime<Utc>,
    pub exit_time: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub trading_mode: String,
}

impl Order {
    /// Check if this order is still open (no exit yet)
    pub fn is_open(&self) -> bool {
        self.exit_price.is_none()
    }

    /// Get the order side as enum
    pub fn side(&self) -> Option<OrderSide> {
        match self.side.as_str() {
            "buy" => Some(OrderSide::Buy),
            "sell" => Some(OrderSide::Sell),
            _ => None,
        }
    }
}

/// New order for insertion (open position)
#[derive(Debug, Clone)]
pub struct NewOrder {
    pub order_id: String,
    pub symbol: String,
    pub side: OrderSide,
    pub entry_price: Decimal,
    pub quantity: Decimal,
    pub reason: String,
    pub entry_time: DateTime<Utc>,
    pub trading_mode: String,
}

/// Data for closing an order (updating with exit)
#[derive(Debug, Clone)]
pub struct CloseOrder {
    pub exit_price: Decimal,
    pub exit_time: DateTime<Utc>,
    pub pnl: Decimal,
}

/// Insert a new order (open position)
#[instrument(skip(pool), fields(order_id = %order.order_id, symbol = %order.symbol))]
pub async fn insert_order(pool: &PgPool, order: &NewOrder) -> Result<i64, sqlx::Error> {
    let result = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO orders (order_id, symbol, side, entry_price, quantity, reason, entry_time, trading_mode)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id
        "#,
    )
    .bind(&order.order_id)
    .bind(&order.symbol)
    .bind(order.side.as_str())
    .bind(&order.entry_price)
    .bind(&order.quantity)
    .bind(&order.reason)
    .bind(&order.entry_time)
    .bind(&order.trading_mode)
    .fetch_one(pool)
    .await?;

    debug!(id = result, mode = %order.trading_mode, "Order inserted");
    Ok(result)
}

/// Close an order (update with exit price, time, and P&L)
/// Filters by trading mode to prevent cross-mode closes
#[instrument(skip(pool), fields(order_id = %order_id))]
pub async fn close_order(
    pool: &PgPool,
    order_id: &str,
    close: &CloseOrder,
    mode: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE orders
        SET exit_price = $1, exit_time = $2, pnl = $3
        WHERE order_id = $4 AND exit_price IS NULL AND trading_mode = $5
        "#,
    )
    .bind(&close.exit_price)
    .bind(&close.exit_time)
    .bind(&close.pnl)
    .bind(order_id)
    .bind(mode)
    .execute(pool)
    .await?;

    let updated = result.rows_affected() > 0;
    if updated {
        debug!(pnl = %close.pnl, "Order closed");
    } else {
        debug!("Order not found or already closed");
    }

    Ok(updated)
}

/// Get an order by order_id (filtered by trading mode)
#[instrument(skip(pool))]
pub async fn get_order(pool: &PgPool, order_id: &str, mode: &str) -> Result<Option<Order>, sqlx::Error> {
    sqlx::query_as::<_, Order>(
        r#"
        SELECT id, order_id, symbol, side, entry_price, exit_price,
               quantity, pnl, reason, entry_time, exit_time, created_at, trading_mode
        FROM orders
        WHERE order_id = $1 AND trading_mode = $2
        "#,
    )
    .bind(order_id)
    .bind(mode)
    .fetch_optional(pool)
    .await
}

/// Get all open orders for a symbol (filtered by trading mode)
#[instrument(skip(pool))]
pub async fn get_open_orders(pool: &PgPool, symbol: &str, mode: &str) -> Result<Vec<Order>, sqlx::Error> {
    sqlx::query_as::<_, Order>(
        r#"
        SELECT id, order_id, symbol, side, entry_price, exit_price,
               quantity, pnl, reason, entry_time, exit_time, created_at, trading_mode
        FROM orders
        WHERE symbol = $1 AND exit_price IS NULL AND trading_mode = $2
        ORDER BY entry_time ASC
        "#,
    )
    .bind(symbol)
    .bind(mode)
    .fetch_all(pool)
    .await
}

/// Calculate P&L for closing a position
///
/// For a BUY order: pnl = (exit_price - entry_price) * quantity
/// For a SELL order: pnl = (entry_price - exit_price) * quantity
pub fn calculate_pnl(
    side: OrderSide,
    entry_price: Decimal,
    exit_price: Decimal,
    quantity: Decimal,
) -> Decimal {
    match side {
        OrderSide::Buy => (exit_price - entry_price) * quantity,
        OrderSide::Sell => (entry_price - exit_price) * quantity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_calculate_pnl_buy_profit() {
        let pnl = calculate_pnl(OrderSide::Buy, dec!(100), dec!(110), dec!(1));
        assert_eq!(pnl, dec!(10));
    }

    #[test]
    fn test_calculate_pnl_buy_loss() {
        let pnl = calculate_pnl(OrderSide::Buy, dec!(100), dec!(90), dec!(1));
        assert_eq!(pnl, dec!(-10));
    }

    #[test]
    fn test_calculate_pnl_sell_profit() {
        // Short sell: profit when price goes down
        let pnl = calculate_pnl(OrderSide::Sell, dec!(100), dec!(90), dec!(1));
        assert_eq!(pnl, dec!(10));
    }

    #[test]
    fn test_calculate_pnl_sell_loss() {
        let pnl = calculate_pnl(OrderSide::Sell, dec!(100), dec!(110), dec!(1));
        assert_eq!(pnl, dec!(-10));
    }

    #[test]
    fn test_order_side_display() {
        assert_eq!(OrderSide::Buy.to_string(), "buy");
        assert_eq!(OrderSide::Sell.to_string(), "sell");
    }
}
