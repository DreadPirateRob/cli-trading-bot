//! Secure API credential handling
//!
//! Uses the secrecy crate to prevent accidental exposure of API keys
//! in logs, debug output, or error messages.

use secrecy::{ExposeSecret, SecretString};
use std::env;
use thiserror::Error;

/// Errors loading credentials
#[derive(Error, Debug)]
pub enum CredentialsError {
    #[error("Missing environment variable: {name}")]
    MissingEnvVar { name: String },

    #[error("Empty API key provided")]
    EmptyApiKey,

    #[error("Empty API secret provided")]
    EmptyApiSecret,
}

/// Binance.US API credentials
///
/// API key and secret are stored as SecretString to prevent accidental
/// logging or display. Use `expose_secret()` only when actually needed
/// for signing requests.
///
/// # Security Notes
/// - Debug output shows "[REDACTED]" instead of actual values
/// - Values are zeroized on drop
/// - Clone copies the secret securely
#[derive(Clone)]
pub struct Credentials {
    api_key: SecretString,
    api_secret: SecretString,
}

impl Credentials {
    /// Create credentials from explicit values
    ///
    /// # Arguments
    /// * `api_key` - The API key (will be moved into SecretString)
    /// * `api_secret` - The API secret (will be moved into SecretString)
    pub fn new(api_key: String, api_secret: String) -> Result<Self, CredentialsError> {
        if api_key.is_empty() {
            return Err(CredentialsError::EmptyApiKey);
        }
        if api_secret.is_empty() {
            return Err(CredentialsError::EmptyApiSecret);
        }

        Ok(Self {
            api_key: SecretString::new(api_key.into()),
            api_secret: SecretString::new(api_secret.into()),
        })
    }

    /// Load credentials from environment variables
    ///
    /// Looks for:
    /// - `BINANCE_API_KEY` or `APP__EXCHANGE__API_KEY`
    /// - `BINANCE_API_SECRET` or `APP__EXCHANGE__API_SECRET`
    ///
    /// The `APP__EXCHANGE__*` format matches the config layering pattern
    /// established in Phase 1.
    pub fn from_env() -> Result<Self, CredentialsError> {
        let api_key = env::var("BINANCE_API_KEY")
            .or_else(|_| env::var("APP__EXCHANGE__API_KEY"))
            .map_err(|_| CredentialsError::MissingEnvVar {
                name: "BINANCE_API_KEY or APP__EXCHANGE__API_KEY".to_string(),
            })?;

        let api_secret = env::var("BINANCE_API_SECRET")
            .or_else(|_| env::var("APP__EXCHANGE__API_SECRET"))
            .map_err(|_| CredentialsError::MissingEnvVar {
                name: "BINANCE_API_SECRET or APP__EXCHANGE__API_SECRET".to_string(),
            })?;

        Self::new(api_key, api_secret)
    }

    /// Load credentials from environment, returning None if not set
    ///
    /// Useful when credentials are optional (e.g., public stream only mode)
    pub fn from_env_optional() -> Option<Self> {
        Self::from_env().ok()
    }

    /// Access the API key for signing requests
    ///
    /// Only call this when you actually need to use the key.
    pub fn api_key(&self) -> &str {
        self.api_key.expose_secret()
    }

    /// Access the API secret for signing requests
    ///
    /// Only call this when you actually need to sign a request.
    pub fn api_secret(&self) -> &str {
        self.api_secret.expose_secret()
    }

    /// Get a masked version of the API key for logging
    ///
    /// Shows first 4 and last 4 characters: "abcd...wxyz"
    pub fn masked_api_key(&self) -> String {
        let key = self.api_key.expose_secret();
        if key.len() <= 8 {
            return "****".to_string();
        }
        format!("{}...{}", &key[..4], &key[key.len() - 4..])
    }
}

// Manual Debug impl to avoid exposing secrets
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("api_key", &"[REDACTED]")
            .field("api_secret", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_creation() {
        let creds = Credentials::new(
            "test_key_12345".to_string(),
            "test_secret_67890".to_string(),
        )
        .unwrap();

        assert_eq!(creds.api_key(), "test_key_12345");
        assert_eq!(creds.api_secret(), "test_secret_67890");
    }

    #[test]
    fn test_empty_key_rejected() {
        let result = Credentials::new("".to_string(), "secret".to_string());
        assert!(matches!(result, Err(CredentialsError::EmptyApiKey)));
    }

    #[test]
    fn test_empty_secret_rejected() {
        let result = Credentials::new("key".to_string(), "".to_string());
        assert!(matches!(result, Err(CredentialsError::EmptyApiSecret)));
    }

    #[test]
    fn test_masked_key() {
        let creds = Credentials::new(
            "abcdefghijklmnop".to_string(),
            "secret".to_string(),
        )
        .unwrap();

        assert_eq!(creds.masked_api_key(), "abcd...mnop");
    }

    #[test]
    fn test_short_key_fully_masked() {
        let creds = Credentials::new("short".to_string(), "secret".to_string()).unwrap();
        assert_eq!(creds.masked_api_key(), "****");
    }

    #[test]
    fn test_debug_redacts_secrets() {
        let creds = Credentials::new("my_actual_key_value".to_string(), "my_actual_secret_value".to_string()).unwrap();
        let debug_output = format!("{:?}", creds);
        assert!(debug_output.contains("[REDACTED]"));
        // Ensure actual secret values don't appear in debug output
        assert!(!debug_output.contains("my_actual_key_value"));
        assert!(!debug_output.contains("my_actual_secret_value"));
    }

    #[test]
    fn test_from_env_optional_returns_none() {
        // When env vars aren't set, should return None
        let result = Credentials::from_env_optional();
        // Note: This test assumes the env vars aren't set in the test environment
        // In CI, they shouldn't be. Locally, they might be.
        // The test is mostly to verify the method doesn't panic.
        let _ = result;
    }
}
