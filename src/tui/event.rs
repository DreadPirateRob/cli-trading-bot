//! TUI event handling
//!
//! Uses crossterm's EventStream for async terminal input.

use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::api::types::BotUpdate;
use crate::notifications::AlertEvent;

/// Events that can occur in the TUI
#[derive(Debug, Clone)]
pub enum TuiEvent {
    /// Keyboard input
    Key(KeyEvent),
    /// Tick event for periodic UI refresh
    Tick,
    /// Bot data update received
    BotUpdate(BotUpdate),
    /// Alert event received
    Alert(AlertEvent),
    /// Shutdown signal received
    Shutdown,
}

/// Handles terminal events asynchronously
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<TuiEvent>,
    _task: tokio::task::JoinHandle<()>,
}

impl EventHandler {
    /// Create a new event handler with optional live data receivers
    ///
    /// # Arguments
    /// * `tick_rate` - How often to send Tick events for UI refresh
    /// * `shutdown_token` - Cancellation token for graceful shutdown
    /// * `bot_rx` - Optional receiver for bot updates (from AppState broadcast)
    /// * `alert_rx` - Optional receiver for alert events (from AlertBroadcaster)
    pub fn new(
        tick_rate: Duration,
        shutdown_token: CancellationToken,
        mut bot_rx: Option<tokio::sync::broadcast::Receiver<BotUpdate>>,
        mut alert_rx: Option<mpsc::Receiver<AlertEvent>>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        let task = tokio::spawn(async move {
            let mut reader = EventStream::new();
            let mut tick_interval = tokio::time::interval(tick_rate);

            loop {
                tokio::select! {
                    biased;

                    // Check shutdown first
                    _ = shutdown_token.cancelled() => {
                        let _ = tx.send(TuiEvent::Shutdown);
                        break;
                    }

                    // Process bot updates if receiver provided
                    update = async {
                        match &mut bot_rx {
                            Some(rx) => rx.recv().await.ok(),
                            None => std::future::pending().await,
                        }
                    } => {
                        if let Some(update) = update {
                            let _ = tx.send(TuiEvent::BotUpdate(update));
                        }
                    }

                    // Process alerts if receiver provided
                    alert = async {
                        match &mut alert_rx {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        if let Some(alert) = alert {
                            let _ = tx.send(TuiEvent::Alert(alert));
                        }
                    }

                    // Send tick for UI refresh
                    _ = tick_interval.tick() => {
                        let _ = tx.send(TuiEvent::Tick);
                    }

                    // Read terminal events
                    Some(Ok(event)) = reader.next() => {
                        if let Event::Key(key) = event {
                            let _ = tx.send(TuiEvent::Key(key));
                        }
                    }
                }
            }
        });

        Self { rx, _task: task }
    }

    /// Create a simple event handler without live data (standalone mode)
    pub fn simple(tick_rate: Duration, shutdown_token: CancellationToken) -> Self {
        Self::new(tick_rate, shutdown_token, None, None)
    }

    /// Get the next event
    pub async fn next(&mut self) -> Option<TuiEvent> {
        self.rx.recv().await
    }
}

/// Handle a key event, returns true if should quit
pub fn handle_key_event(key: KeyEvent) -> bool {
    match key.code {
        // Quit on 'q' or Ctrl+C or Esc
        KeyCode::Char('q') => true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => true,
        KeyCode::Esc => true,
        _ => false,
    }
}
