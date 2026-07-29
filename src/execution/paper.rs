//! Paper trading executor for simulated order execution
//!
//! Simulates order fills with configurable latency, slippage, and fees
//! while maintaining isolated paper balances.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rand::Rng;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use tracing::info;
use uuid::Uuid;

use crate::config::PaperTradingConfig;
use crate::data::OrderSide;
use crate::execution::types::{
    ExecuteOrderRequest, ExecuteOrderResponse, ExecutionError, Fill, OrderExecutor, OrderStatus,
};

/// Paper trading executor with simulated fills
pub struct PaperExecutor {
    config: PaperTradingConfig,
    /// Balances by asset (e.g., "USDT" -> 10000, "BTC" -> 0.5)
    balances: Arc<RwLock<HashMap<String, Decimal>>>,
    /// Track orders for status queries
    orders: Arc<RwLock<HashMap<String, OrderStatus>>>,
}

impl PaperExecutor {
    pub fn new(config: PaperTradingConfig) -> Self {
        let balances: HashMap<String, Decimal> = config
            .initial_balances
            .iter()
            .map(|(k, v)| (k.clone(), Decimal::from_f64(*v).unwrap_or(Decimal::ZERO)))
            .collect();

        Self {
            config,
            balances: Arc::new(RwLock::new(balances)),
            orders: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Simulate network latency
    async fn simulate_latency(&self) {
        let (min, max) = self.config.latency_range_ms;
        let latency = rand::thread_rng().gen_range(min..=max);
        sleep(Duration::from_millis(latency)).await;
    }

    /// Calculate random slippage factor
    fn random_slippage(&self) -> Decimal {
        let (min, max) = self.config.slippage_range;
        let slippage = rand::thread_rng().gen_range(min..=max);
        Decimal::from_f64(slippage).unwrap_or(Decimal::ZERO)
    }

    /// Get fee as Decimal
    fn fee_pct(&self) -> Decimal {
        Decimal::from_f64(self.config.fee_pct).unwrap_or(Decimal::ZERO)
    }

    /// Extract base and quote assets from symbol (e.g., "BTCUSDT" -> ("BTC", "USDT"))
    pub fn parse_symbol(symbol: &str) -> (&str, &str) {
        // Common quote currencies to check
        for quote in &["USDT", "USDC", "BUSD", "USD"] {
            if symbol.ends_with(quote) {
                let base_len = symbol.len() - quote.len();
                return (&symbol[..base_len], *quote);
            }
        }
        // Fallback: assume last 4 chars are quote (works for most pairs)
        let split = symbol.len().saturating_sub(4);
        (&symbol[..split], &symbol[split..])
    }
}

#[async_trait]
impl OrderExecutor for PaperExecutor {
    async fn execute(
        &self,
        request: ExecuteOrderRequest,
    ) -> Result<ExecuteOrderResponse, ExecutionError> {
        // Simulate network latency
        self.simulate_latency().await;

        // Parse symbol and convert to owned strings early to avoid borrow issues
        let (base_asset_ref, quote_asset_ref) = Self::parse_symbol(&request.symbol);
        let base_asset = base_asset_ref.to_string();
        let quote_asset = quote_asset_ref.to_string();
        let market_price = request.price.unwrap_or(Decimal::ZERO);

        // Apply slippage to fill price
        let slippage = self.random_slippage();
        let fill_price = match request.side {
            OrderSide::Buy => market_price * (Decimal::ONE + slippage),
            OrderSide::Sell => market_price * (Decimal::ONE - slippage),
        };

        // Calculate costs
        let notional = fill_price * request.quantity;
        let commission = notional * self.fee_pct();

        // Check and update balances
        {
            let mut balances = self.balances.write().await;

            match request.side {
                OrderSide::Buy => {
                    // Need quote currency to buy base
                    let quote_balance =
                        balances.get(&quote_asset).copied().unwrap_or(Decimal::ZERO);
                    let required = notional + commission;

                    if quote_balance < required {
                        return Err(ExecutionError::InsufficientBalance {
                            asset: quote_asset,
                            required,
                            available: quote_balance,
                        });
                    }

                    // Deduct quote, add base
                    *balances
                        .entry(quote_asset.clone())
                        .or_insert(Decimal::ZERO) -= required;
                    *balances
                        .entry(base_asset.clone())
                        .or_insert(Decimal::ZERO) += request.quantity;
                }
                OrderSide::Sell => {
                    // Need base currency to sell
                    let base_balance = balances.get(&base_asset).copied().unwrap_or(Decimal::ZERO);

                    if base_balance < request.quantity {
                        return Err(ExecutionError::InsufficientBalance {
                            asset: base_asset,
                            required: request.quantity,
                            available: base_balance,
                        });
                    }

                    // Deduct base, add quote (minus commission)
                    *balances
                        .entry(base_asset.clone())
                        .or_insert(Decimal::ZERO) -= request.quantity;
                    *balances
                        .entry(quote_asset.clone())
                        .or_insert(Decimal::ZERO) += notional - commission;
                }
            }
        }

        let order_id = Uuid::new_v4().to_string();
        let client_order_id = request
            .client_order_id
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let timestamp = chrono::Utc::now().timestamp_millis();

        // Store order status
        {
            let mut orders = self.orders.write().await;
            orders.insert(order_id.clone(), OrderStatus::Filled);
        }

        info!(
            order_id = %order_id,
            symbol = %request.symbol,
            side = ?request.side,
            quantity = %request.quantity,
            fill_price = %fill_price,
            commission = %commission,
            "Paper order filled"
        );

        Ok(ExecuteOrderResponse {
            order_id,
            client_order_id,
            symbol: request.symbol,
            side: request.side,
            filled_quantity: request.quantity,
            fill_price,
            status: OrderStatus::Filled,
            timestamp,
            fills: vec![Fill {
                price: fill_price,
                quantity: request.quantity,
                commission,
                commission_asset: quote_asset,
            }],
        })
    }

    async fn get_balance(&self, asset: &str) -> Result<Decimal, ExecutionError> {
        let balances = self.balances.read().await;
        Ok(balances.get(asset).copied().unwrap_or(Decimal::ZERO))
    }

    async fn get_order_status(
        &self,
        order_id: &str,
        _symbol: &str,
    ) -> Result<OrderStatus, ExecutionError> {
        let orders = self.orders.read().await;
        orders
            .get(order_id)
            .cloned()
            .ok_or_else(|| ExecutionError::InvalidRequest(format!("Order not found: {}", order_id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::types::OrderType;
    use rust_decimal_macros::dec;
    use std::collections::HashMap;

    fn test_config() -> PaperTradingConfig {
        let mut balances = HashMap::new();
        balances.insert("USDT".to_string(), 10000.0);
        balances.insert("BTC".to_string(), 1.0);

        PaperTradingConfig {
            enabled: true,
            latency_range_ms: (1, 5), // Fast for tests
            slippage_range: (0.0, 0.0), // No slippage for predictable tests
            fee_pct: 0.001, // 0.1% fee
            initial_balances: balances,
        }
    }

    #[tokio::test]
    async fn test_paper_buy_order() {
        let executor = PaperExecutor::new(test_config());

        let request = ExecuteOrderRequest {
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Buy,
            quantity: dec!(0.1),
            order_type: OrderType::Market,
            price: Some(dec!(50000)),
            client_order_id: None,
        };

        let response = executor.execute(request).await.unwrap();

        assert_eq!(response.status, OrderStatus::Filled);
        assert_eq!(response.filled_quantity, dec!(0.1));
        assert_eq!(response.side, OrderSide::Buy);

        // Check balances updated
        let btc = executor.get_balance("BTC").await.unwrap();
        assert_eq!(btc, dec!(1.1)); // Started with 1, bought 0.1

        let usdt = executor.get_balance("USDT").await.unwrap();
        // Paid: 0.1 * 50000 = 5000 + 0.1% fee = 5005
        assert!(usdt < dec!(5000)); // Less than 5000 remaining
    }

    #[tokio::test]
    async fn test_paper_sell_order() {
        let executor = PaperExecutor::new(test_config());

        let request = ExecuteOrderRequest {
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Sell,
            quantity: dec!(0.5),
            order_type: OrderType::Market,
            price: Some(dec!(50000)),
            client_order_id: None,
        };

        let response = executor.execute(request).await.unwrap();

        assert_eq!(response.status, OrderStatus::Filled);
        assert_eq!(response.filled_quantity, dec!(0.5));

        // Check balances
        let btc = executor.get_balance("BTC").await.unwrap();
        assert_eq!(btc, dec!(0.5)); // Started with 1, sold 0.5

        let usdt = executor.get_balance("USDT").await.unwrap();
        // Received: 0.5 * 50000 = 25000 - 0.1% fee = 24975
        // Starting 10000 + 24975 = 34975
        assert!(usdt > dec!(34000)); // More than starting 10000 + 24000
    }

    #[tokio::test]
    async fn test_paper_insufficient_balance() {
        let executor = PaperExecutor::new(test_config());

        let request = ExecuteOrderRequest {
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Buy,
            quantity: dec!(1.0), // 1 BTC = 50000 USDT, we only have 10000
            order_type: OrderType::Market,
            price: Some(dec!(50000)),
            client_order_id: None,
        };

        let result = executor.execute(request).await;
        assert!(matches!(result, Err(ExecutionError::InsufficientBalance { .. })));
    }

    #[tokio::test]
    async fn test_paper_order_status() {
        let executor = PaperExecutor::new(test_config());

        let request = ExecuteOrderRequest {
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Buy,
            quantity: dec!(0.01),
            order_type: OrderType::Market,
            price: Some(dec!(50000)),
            client_order_id: None,
        };

        let response = executor.execute(request).await.unwrap();
        let status = executor.get_order_status(&response.order_id, "BTCUSDT").await.unwrap();

        assert_eq!(status, OrderStatus::Filled);
    }

    #[test]
    fn test_paper_parse_symbol() {
        assert_eq!(PaperExecutor::parse_symbol("BTCUSDT"), ("BTC", "USDT"));
        assert_eq!(PaperExecutor::parse_symbol("ETHUSDC"), ("ETH", "USDC"));
        assert_eq!(PaperExecutor::parse_symbol("BTCUSD"), ("BTC", "USD"));
    }

    #[tokio::test]
    async fn test_paper_sell_insufficient_base() {
        let executor = PaperExecutor::new(test_config());

        let request = ExecuteOrderRequest {
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Sell,
            quantity: dec!(2.0), // We only have 1 BTC
            order_type: OrderType::Market,
            price: Some(dec!(50000)),
            client_order_id: None,
        };

        let result = executor.execute(request).await;
        assert!(matches!(result, Err(ExecutionError::InsufficientBalance { .. })));
    }

    #[tokio::test]
    async fn test_paper_client_order_id() {
        let executor = PaperExecutor::new(test_config());

        let request = ExecuteOrderRequest {
            symbol: "BTCUSDT".to_string(),
            side: OrderSide::Buy,
            quantity: dec!(0.01),
            order_type: OrderType::Market,
            price: Some(dec!(50000)),
            client_order_id: Some("my-order-123".to_string()),
        };

        let response = executor.execute(request).await.unwrap();
        assert_eq!(response.client_order_id, "my-order-123");
    }
}
