//! HMAC-SHA256 signing for Binance API authentication
//!
//! Binance requires all authenticated endpoints to include a signature
//! generated using HMAC-SHA256 with the API secret as the key.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Sign a request string using HMAC-SHA256
///
/// # Arguments
/// * `params` - The query string to sign (without signature parameter)
/// * `api_secret` - The API secret key
///
/// # Returns
/// Hex-encoded signature string
pub fn sign_request(params: &str, api_secret: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(api_secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(params.as_bytes());
    let result = mac.finalize();
    hex::encode(result.into_bytes())
}

/// Build signed query parameters for Binance API
///
/// Appends timestamp and signature to the provided parameters.
///
/// # Arguments
/// * `params` - Slice of (key, value) parameter pairs
/// * `api_secret` - The API secret key
///
/// # Returns
/// Complete query string with timestamp and signature
pub fn build_signed_params(params: &[(&str, &str)], api_secret: &str) -> String {
    let timestamp = chrono::Utc::now().timestamp_millis();

    // Build query string from params
    let mut query_parts: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();

    // Append timestamp
    query_parts.push(format!("timestamp={}", timestamp));

    let query_string = query_parts.join("&");

    // Generate signature
    let signature = sign_request(&query_string, api_secret);

    // Return complete query string with signature
    format!("{}&signature={}", query_string, signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_request_known_signature() {
        // Test vector from Binance API documentation
        // https://binance-docs.github.io/apidocs/spot/en/#signed-trade-and-user_data-endpoint-security
        let params = "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559";
        let secret = "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j";

        let signature = sign_request(params, secret);

        assert_eq!(
            signature,
            "c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71"
        );
    }

    #[test]
    fn test_sign_request_empty_params() {
        let params = "";
        let secret = "test_secret";

        let signature = sign_request(params, secret);

        // Empty params should still produce valid signature
        assert!(!signature.is_empty());
        assert_eq!(signature.len(), 64); // SHA256 produces 32 bytes = 64 hex chars
    }

    #[test]
    fn test_build_signed_params_includes_timestamp() {
        let params = [("symbol", "BTCUSDT"), ("side", "BUY")];
        let secret = "test_secret";

        let result = build_signed_params(&params, secret);

        assert!(result.contains("timestamp="));
        assert!(result.contains("signature="));
        assert!(result.starts_with("symbol=BTCUSDT&side=BUY&timestamp="));
    }

    #[test]
    fn test_build_signed_params_empty() {
        let params: [(&str, &str); 0] = [];
        let secret = "test_secret";

        let result = build_signed_params(&params, secret);

        assert!(result.starts_with("timestamp="));
        assert!(result.contains("&signature="));
    }
}
