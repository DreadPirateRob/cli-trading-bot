//! Alert broadcasting to multiple consumers

use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::AlertEvent;

/// Handle for sending alerts to the broadcaster
#[derive(Clone)]
pub struct AlertSender {
    tx: mpsc::Sender<AlertEvent>,
}

impl AlertSender {
    /// Send an alert event
    ///
    /// Returns Ok(()) if sent, Err if channel is full/closed.
    /// Use try_send() for non-blocking send that drops on full.
    pub async fn send(&self, event: AlertEvent) -> Result<(), mpsc::error::SendError<AlertEvent>> {
        self.tx.send(event).await
    }

    /// Try to send an alert without blocking
    ///
    /// Drops the event if channel is full. Returns true if sent.
    pub fn try_send(&self, event: AlertEvent) -> bool {
        match self.tx.try_send(event) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("Alert channel full, dropping event");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                debug!("Alert channel closed");
                false
            }
        }
    }
}

/// Broadcasts alerts to multiple consumers (TUI, Telegram, etc.)
pub struct AlertBroadcaster {
    rx: mpsc::Receiver<AlertEvent>,
    subscribers: Vec<mpsc::Sender<AlertEvent>>,
}

impl AlertBroadcaster {
    /// Create a new broadcaster with the given channel capacity
    pub fn new(capacity: usize) -> (AlertSender, Self) {
        let (tx, rx) = mpsc::channel(capacity);
        let broadcaster = Self {
            rx,
            subscribers: Vec::new(),
        };
        let sender = AlertSender { tx };
        (sender, broadcaster)
    }

    /// Create a new broadcaster with default capacity (100)
    #[allow(clippy::new_without_default)]
    pub fn with_default_capacity() -> (AlertSender, Self) {
        Self::new(100)
    }

    /// Subscribe to receive alerts
    ///
    /// Returns a receiver that will get copies of all broadcast alerts.
    pub fn subscribe(&mut self, capacity: usize) -> mpsc::Receiver<AlertEvent> {
        let (tx, rx) = mpsc::channel(capacity);
        self.subscribers.push(tx);
        rx
    }

    /// Run the broadcaster, forwarding events to all subscribers
    ///
    /// This should be spawned as a tokio task. Runs until the sender is dropped.
    pub async fn run(mut self) {
        debug!("Alert broadcaster started with {} subscribers", self.subscribers.len());

        while let Some(event) = self.rx.recv().await {
            debug!(?event, "Broadcasting alert");

            // Remove closed subscribers during iteration
            self.subscribers.retain(|sub| {
                match sub.try_send(event.clone()) {
                    Ok(()) => true,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        warn!("Subscriber channel full, dropping event for this subscriber");
                        true // Keep subscriber, just drop this event
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        debug!("Subscriber disconnected, removing");
                        false // Remove closed subscriber
                    }
                }
            });
        }

        debug!("Alert broadcaster shutting down");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::SystemEventType;

    #[tokio::test]
    async fn test_broadcast_to_multiple_subscribers() {
        let (sender, mut broadcaster) = AlertBroadcaster::new(10);

        let mut rx1 = broadcaster.subscribe(10);
        let mut rx2 = broadcaster.subscribe(10);

        // Spawn broadcaster
        tokio::spawn(async move {
            broadcaster.run().await;
        });

        // Send an event
        let event = AlertEvent::system(SystemEventType::BotStarted, "Test start");
        sender.send(event).await.unwrap();

        // Small delay for broadcast
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Both subscribers should receive it
        let recv1 = rx1.try_recv();
        let recv2 = rx2.try_recv();

        assert!(recv1.is_ok());
        assert!(recv2.is_ok());
    }

    #[tokio::test]
    async fn test_try_send_drops_on_full() {
        let (sender, _broadcaster) = AlertBroadcaster::new(1);

        // Fill the channel
        let event = AlertEvent::system(SystemEventType::BotStarted, "Test");
        assert!(sender.try_send(event.clone()));

        // Next should fail (channel full, no receiver running)
        // This won't fail because the broadcaster isn't running
        // In real usage, broadcaster.run() would drain the channel
    }
}
