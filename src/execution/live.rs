//! Live order execution via Binance.US REST API
//!
//! Implements OrderExecutor trait for real order placement with
//! HMAC-SHA256 authentication.

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;
use tracing::{debug, error, info, warn};

use crate::data::OrderSide;
use crate::exchange::Credentials;
use crate::execution::signing::build_signed_params;
use crate::execution::types::{
    ExecuteOrderRequest, ExecuteOrderResponse, ExecutionError, Fill, OrderExecutor, OrderStatus,
    OrderType,
};

/// Binance.US API base URL
pub const BINANCE_US_API: &str = "https://api.binance.us";

/// Live executor for Binance.US REST API
pub struct LiveExecutor {
    client: Client,
    credentials: Credentials,
    recv_window: u64,
}

impl LiveExecutor {
    /// Create a new live executor
    ///
    /// # Arguments
    /// * `credentials` - API key and secret for authentication
    pub fn new(credentials: Credentials) -> Self {
        Self {
            client: Client::new(),
            credentials,
            recv_window: 5000,
        }
    }

    /// Create with custom recv_window
    pub fn with_recv_window(credentials: Credentials, recv_window: u64) -> Self {
        Self {
            client: Client::new(),
            credentials,
            recv_window,
        }
    }

    fn api_key(&self) -> &str {
        self.credentials.api_key()
    }

    fn api_secret(&self) -> &str {
        self.credentials.api_secret()
    }

    /// Get all non-zero balances from the exchange
    ///
    /// Useful for reconciliation and portfolio overview.
    pub async fn get_all_balances(&self) -> Result<HashMap<String, Decimal>, ExecutionError> {
        let recv_window_str = self.recv_window.to_string();
        let signed_params =
            build_signed_params(&[("recvWindow", &recv_window_str)], self.api_secret());

        let response = self
            .client
            .get(format!(
                "{}/api/v3/account?{}",
                BINANCE_US_API, signed_params
            ))
            .header("X-MBX-APIKEY", self.api_key())
            .send()
            .await
            .map_err(|e| ExecutionError::Network(format!("Request failed: {}", e)))?;

        let account: BinanceAccountInfo = self.handle_response(response).await?;

        let balances: HashMap<String, Decimal> = account
            .balances
            .into_iter()
            .filter_map(|b| {
                let free = Self::parse_decimal(&b.free).ok()?;
                if free > Decimal::ZERO {
                    Some((b.asset, free))
                } else {
                    None
                }
            })
            .collect();

        debug!(count = balances.len(), "Retrieved all balances");
        Ok(balances)
    }
}

// Binance API response types
#[derive(Debug, Deserialize)]
struct BinanceOrderResponse {
    symbol: String,
    #[serde(rename = "orderId")]
    order_id: u64,
    #[serde(rename = "clientOrderId")]
    client_order_id: String,
    #[serde(rename = "transactTime")]
    transact_time: u64,
    price: String,
    #[allow(dead_code)]
    #[serde(rename = "origQty")]
    orig_qty: String,
    #[serde(rename = "executedQty")]
    executed_qty: String,
    #[serde(rename = "cummulativeQuoteQty")]
    _cumulative_quote_qty: String,
    status: String,
    #[serde(rename = "type")]
    _order_type: String,
    side: String,
    #[serde(default)]
    fills: Vec<BinanceFill>,
}

#[derive(Debug, Deserialize)]
struct BinanceFill {
    price: String,
    qty: String,
    commission: String,
    #[serde(rename = "commissionAsset")]
    commission_asset: String,
}

#[derive(Debug, Deserialize)]
struct BinanceAccountInfo {
    balances: Vec<BinanceBalance>,
    #[serde(rename = "canTrade")]
    _can_trade: bool,
}

#[derive(Debug, Deserialize)]
struct BinanceBalance {
    asset: String,
    free: String,
    #[allow(dead_code)]
    locked: String,
}

#[derive(Debug, Deserialize)]
struct BinanceError {
    code: i32,
    msg: String,
}

#[derive(Debug, Deserialize)]
struct BinanceOrderStatus {
    #[allow(dead_code)]
    symbol: String,
    #[allow(dead_code)]
    #[serde(rename = "orderId")]
    order_id: u64,
    status: String,
    #[allow(dead_code)]
    #[serde(rename = "executedQty")]
    executed_qty: String,
}

impl LiveExecutor {
    fn parse_order_status(status: &str) -> OrderStatus {
        match status {
            "NEW" => OrderStatus::New,
            "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
            "FILLED" => OrderStatus::Filled,
            "CANCELED" => OrderStatus::Canceled,
            "REJECTED" => OrderStatus::Rejected,
            "EXPIRED" | "EXPIRED_IN_MATCH" => OrderStatus::Expired,
            _ => OrderStatus::New, // Default to New for unknown
        }
    }

    fn parse_decimal(s: &str) -> Result<Decimal, ExecutionError> {
        s.parse::<Decimal>()
            .map_err(|e| ExecutionError::InvalidRequest(format!("Failed to parse decimal: {}", e)))
    }

    fn parse_order_side(s: &str) -> OrderSide {
        match s.to_uppercase().as_str() {
            "BUY" => OrderSide::Buy,
            _ => OrderSide::Sell,
        }
    }

    async fn handle_response<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, ExecutionError> {
        let status = response.status();

        if status.is_success() {
            response
                .json::<T>()
                .await
                .map_err(|e| ExecutionError::Network(format!("Failed to parse response: {}", e)))
        } else if status.as_u16() == 429 {
            // Rate limited
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(60000);

            warn!(retry_after_ms = retry_after, "Rate limited by Binance");
            Err(ExecutionError::RateLimited {
                retry_after_ms: retry_after,
            })
        } else {
            // Try to parse Binance error
            let error: BinanceError = response.json().await.unwrap_or(BinanceError {
                code: -1,
                msg: "Unknown error".to_string(),
            });

            error!(code = error.code, msg = %error.msg, "Binance API error");
            Err(ExecutionError::Exchange {
                code: error.code,
                msg: error.msg,
            })
        }
    }
}

#[async_trait]
impl OrderExecutor for LiveExecutor {
    async fn execute(
        &self,
        request: ExecuteOrderRequest,
    ) -> Result<ExecuteOrderResponse, ExecutionError> {
        let side_str = match request.side {
            OrderSide::Buy => "BUY",
            OrderSide::Sell => "SELL",
        };

        let type_str = match request.order_type {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
        };

        let mut params: Vec<(&str, String)> = vec![
            ("symbol", request.symbol.clone()),
            ("side", side_str.to_string()),
            ("type", type_str.to_string()),
            ("quantity", request.quantity.to_string()),
            ("newOrderRespType", "FULL".to_string()),
            ("recvWindow", self.recv_window.to_string()),
        ];

        // Add price for limit orders
        if request.order_type == OrderType::Limit {
            if let Some(price) = request.price {
                params.push(("price", price.to_string()));
                params.push(("timeInForce", "GTC".to_string()));
            }
        }

        // Add client order ID if provided
        if let Some(ref client_id) = request.client_order_id {
            params.push(("newClientOrderId", client_id.clone()));
        }

        let params_refs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let signed_params = build_signed_params(&params_refs, self.api_secret());

        info!(
            symbol = %request.symbol,
            side = %side_str,
            order_type = %type_str,
            quantity = %request.quantity,
            "Placing order on Binance.US"
        );

        let response = self
            .client
            .post(format!("{}/api/v3/order", BINANCE_US_API))
            .header("X-MBX-APIKEY", self.api_key())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(signed_params)
            .send()
            .await
            .map_err(|e| ExecutionError::Network(format!("Request failed: {}", e)))?;

        let binance_response: BinanceOrderResponse = self.handle_response(response).await?;

        let fills: Vec<Fill> = binance_response
            .fills
            .into_iter()
            .map(|f| Fill {
                price: Self::parse_decimal(&f.price).unwrap_or(Decimal::ZERO),
                quantity: Self::parse_decimal(&f.qty).unwrap_or(Decimal::ZERO),
                commission: Self::parse_decimal(&f.commission).unwrap_or(Decimal::ZERO),
                commission_asset: f.commission_asset,
            })
            .collect();

        // Calculate average fill price
        let (total_qty, total_value) = fills
            .iter()
            .fold((Decimal::ZERO, Decimal::ZERO), |(qty, val), f| {
                (qty + f.quantity, val + f.price * f.quantity)
            });
        let avg_fill_price = if total_qty > Decimal::ZERO {
            total_value / total_qty
        } else {
            Self::parse_decimal(&binance_response.price).unwrap_or(Decimal::ZERO)
        };

        info!(
            order_id = binance_response.order_id,
            status = %binance_response.status,
            filled_qty = %binance_response.executed_qty,
            "Order placed successfully"
        );

        Ok(ExecuteOrderResponse {
            order_id: binance_response.order_id.to_string(),
            client_order_id: binance_response.client_order_id,
            symbol: binance_response.symbol,
            side: Self::parse_order_side(&binance_response.side),
            filled_quantity: Self::parse_decimal(&binance_response.executed_qty)?,
            fill_price: avg_fill_price,
            status: Self::parse_order_status(&binance_response.status),
            timestamp: binance_response.transact_time as i64,
            fills,
        })
    }

    async fn get_balance(&self, asset: &str) -> Result<Decimal, ExecutionError> {
        let recv_window_str = self.recv_window.to_string();
        let signed_params =
            build_signed_params(&[("recvWindow", &recv_window_str)], self.api_secret());

        let response = self
            .client
            .get(format!(
                "{}/api/v3/account?{}",
                BINANCE_US_API, signed_params
            ))
            .header("X-MBX-APIKEY", self.api_key())
            .send()
            .await
            .map_err(|e| ExecutionError::Network(format!("Request failed: {}", e)))?;

        let account: BinanceAccountInfo = self.handle_response(response).await?;

        let balance = account
            .balances
            .iter()
            .find(|b| b.asset == asset)
            .map(|b| Self::parse_decimal(&b.free))
            .transpose()?
            .unwrap_or(Decimal::ZERO);

        debug!(asset = %asset, balance = %balance, "Retrieved balance");
        Ok(balance)
    }

    async fn get_order_status(
        &self,
        order_id: &str,
        symbol: &str,
    ) -> Result<OrderStatus, ExecutionError> {
        let recv_window_str = self.recv_window.to_string();
        let signed_params = build_signed_params(
            &[
                ("symbol", symbol),
                ("orderId", order_id),
                ("recvWindow", &recv_window_str),
            ],
            self.api_secret(),
        );

        let response = self
            .client
            .get(format!("{}/api/v3/order?{}", BINANCE_US_API, signed_params))
            .header("X-MBX-APIKEY", self.api_key())
            .send()
            .await
            .map_err(|e| ExecutionError::Network(format!("Request failed: {}", e)))?;

        let order: BinanceOrderStatus = self.handle_response(response).await?;
        Ok(Self::parse_order_status(&order.status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_order_status_new() {
        assert_eq!(LiveExecutor::parse_order_status("NEW"), OrderStatus::New);
    }

    #[test]
    fn test_parse_order_status_filled() {
        assert_eq!(
            LiveExecutor::parse_order_status("FILLED"),
            OrderStatus::Filled
        );
    }

    #[test]
    fn test_parse_order_status_partially_filled() {
        assert_eq!(
            LiveExecutor::parse_order_status("PARTIALLY_FILLED"),
            OrderStatus::PartiallyFilled
        );
    }

    #[test]
    fn test_parse_order_status_canceled() {
        assert_eq!(
            LiveExecutor::parse_order_status("CANCELED"),
            OrderStatus::Canceled
        );
    }

    #[test]
    fn test_parse_order_status_rejected() {
        assert_eq!(
            LiveExecutor::parse_order_status("REJECTED"),
            OrderStatus::Rejected
        );
    }

    #[test]
    fn test_parse_order_status_expired() {
        assert_eq!(
            LiveExecutor::parse_order_status("EXPIRED"),
            OrderStatus::Expired
        );
    }

    #[test]
    fn test_parse_order_status_expired_in_match() {
        assert_eq!(
            LiveExecutor::parse_order_status("EXPIRED_IN_MATCH"),
            OrderStatus::Expired
        );
    }

    #[test]
    fn test_parse_order_status_unknown_defaults_to_new() {
        assert_eq!(
            LiveExecutor::parse_order_status("UNKNOWN_STATUS"),
            OrderStatus::New
        );
    }

    #[test]
    fn test_parse_decimal_valid() {
        assert_eq!(
            LiveExecutor::parse_decimal("123.456").unwrap(),
            Decimal::new(123456, 3)
        );
    }

    #[test]
    fn test_parse_decimal_small_value() {
        assert_eq!(
            LiveExecutor::parse_decimal("0.00000001").unwrap(),
            Decimal::new(1, 8)
        );
    }

    #[test]
    fn test_parse_decimal_large_value() {
        assert_eq!(
            LiveExecutor::parse_decimal("100000000.00000001").unwrap(),
            Decimal::new(10000000000000001, 8)
        );
    }

    #[test]
    fn test_parse_decimal_zero() {
        assert_eq!(
            LiveExecutor::parse_decimal("0").unwrap(),
            Decimal::ZERO
        );
    }

    #[test]
    fn test_parse_decimal_invalid() {
        assert!(LiveExecutor::parse_decimal("invalid").is_err());
    }

    #[test]
    fn test_parse_decimal_empty() {
        assert!(LiveExecutor::parse_decimal("").is_err());
    }

    #[test]
    fn test_parse_order_side_buy_uppercase() {
        assert_eq!(LiveExecutor::parse_order_side("BUY"), OrderSide::Buy);
    }

    #[test]
    fn test_parse_order_side_buy_lowercase() {
        assert_eq!(LiveExecutor::parse_order_side("buy"), OrderSide::Buy);
    }

    #[test]
    fn test_parse_order_side_buy_mixed_case() {
        assert_eq!(LiveExecutor::parse_order_side("Buy"), OrderSide::Buy);
    }

    #[test]
    fn test_parse_order_side_sell_uppercase() {
        assert_eq!(LiveExecutor::parse_order_side("SELL"), OrderSide::Sell);
    }

    #[test]
    fn test_parse_order_side_sell_lowercase() {
        assert_eq!(LiveExecutor::parse_order_side("sell"), OrderSide::Sell);
    }

    #[test]
    fn test_parse_order_side_unknown_defaults_to_sell() {
        // Unknown defaults to Sell
        assert_eq!(LiveExecutor::parse_order_side("UNKNOWN"), OrderSide::Sell);
    }

    #[test]
    fn test_binance_us_api_url() {
        assert_eq!(BINANCE_US_API, "https://api.binance.us");
    }
}
