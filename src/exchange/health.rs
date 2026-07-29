//! Connection health monitoring
//!
//! Tracks connection state for operational visibility and proactive
//! reconnection before hitting Binance's 24-hour connection limit.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tracing::info;

/// Connection health metrics
///
/// Thread-safe tracking of connection state that can be shared
/// across tasks for monitoring and logging.
#[derive(Debug)]
pub struct ConnectionHealth {
    /// When the current connection was established
    connected_at: RwLock<Option<Instant>>,
    /// When the last message was received
    last_message_at: RwLock<Option<Instant>>,
    /// Total reconnection count since startup
    reconnect_count: AtomicU32,
    /// Total messages received since startup
    message_count: AtomicU64,
}

impl ConnectionHealth {
    /// Create a new health tracker
    pub fn new() -> Self {
        Self {
            connected_at: RwLock::new(None),
            last_message_at: RwLock::new(None),
            reconnect_count: AtomicU32::new(0),
            message_count: AtomicU64::new(0),
        }
    }

    /// Create a shareable handle to the health tracker
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Record that a connection was established
    pub async fn on_connected(&self) {
        let now = Instant::now();
        *self.connected_at.write().await = Some(now);
        *self.last_message_at.write().await = Some(now);
        let count = self.reconnect_count.fetch_add(1, Ordering::Relaxed) + 1;

        info!(
            reconnect_count = count,
            "Connection established"
        );
    }

    /// Record that a message was received
    pub async fn on_message(&self) {
        *self.last_message_at.write().await = Some(Instant::now());
        self.message_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Get time elapsed since connection was established
    pub async fn connection_age(&self) -> Option<Duration> {
        self.connected_at.read().await.map(|t| t.elapsed())
    }

    /// Get time elapsed since last message
    pub async fn time_since_last_message(&self) -> Option<Duration> {
        self.last_message_at.read().await.map(|t| t.elapsed())
    }

    /// Check if connection is stale (no messages for given duration)
    pub async fn is_stale(&self, threshold: Duration) -> bool {
        match *self.last_message_at.read().await {
            Some(t) => t.elapsed() > threshold,
            None => true, // Never received a message
        }
    }

    /// Check if proactive reconnection is needed (approaching 24h limit)
    ///
    /// Binance.US disconnects all connections after exactly 24 hours.
    /// We recommend reconnecting at 23 hours to avoid unexpected drops.
    pub async fn should_reconnect_proactively(&self) -> bool {
        const RECONNECT_THRESHOLD: Duration = Duration::from_secs(23 * 60 * 60); // 23 hours

        match *self.connected_at.read().await {
            Some(t) => t.elapsed() > RECONNECT_THRESHOLD,
            None => false,
        }
    }

    /// Get total reconnection count
    pub fn reconnect_count(&self) -> u32 {
        self.reconnect_count.load(Ordering::Relaxed)
    }

    /// Get total message count
    pub fn message_count(&self) -> u64 {
        self.message_count.load(Ordering::Relaxed)
    }

    /// Log current health status
    pub async fn log_status(&self) {
        let connection_age = self.connection_age().await;
        let since_message = self.time_since_last_message().await;
        let reconnects = self.reconnect_count();
        let messages = self.message_count();

        info!(
            connection_age_secs = connection_age.map(|d| d.as_secs()),
            since_last_message_ms = since_message.map(|d| d.as_millis() as u64),
            reconnect_count = reconnects,
            message_count = messages,
            "Connection health status"
        );
    }
}

impl Default for ConnectionHealth {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_tracking() {
        let health = ConnectionHealth::new();

        // Initially no connection
        assert!(health.connection_age().await.is_none());
        assert!(health.is_stale(Duration::from_secs(1)).await);
        assert_eq!(health.reconnect_count(), 0);

        // Connect
        health.on_connected().await;
        assert!(health.connection_age().await.is_some());
        assert_eq!(health.reconnect_count(), 1);

        // Receive message
        health.on_message().await;
        assert_eq!(health.message_count(), 1);
        assert!(!health.is_stale(Duration::from_secs(1)).await);
    }

    #[tokio::test]
    async fn test_proactive_reconnect_detection() {
        let health = ConnectionHealth::new();

        // No connection - no reconnect needed
        assert!(!health.should_reconnect_proactively().await);

        // Fresh connection - no reconnect needed
        health.on_connected().await;
        assert!(!health.should_reconnect_proactively().await);

        // Note: We can't easily test the 23-hour threshold without mocking time
    }
}
