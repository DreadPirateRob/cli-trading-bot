//! WebSocket handler for real-time dashboard updates
//!
//! Handles WebSocket upgrades and broadcasts BotUpdate messages
//! to all connected clients using tokio broadcast channels.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tracing::{debug, error, info};

use super::state::AppState;

/// Handle WebSocket upgrade requests
///
/// Upgrades the HTTP connection to WebSocket and spawns the handler.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// Handle an established WebSocket connection
///
/// Subscribes to the broadcast channel and forwards all updates
/// to the connected client as JSON messages.
async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.bot_updates.subscribe();

    info!("WebSocket client connected");

    // Spawn task to forward broadcast messages to client
    let send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(update) => {
                    match serde_json::to_string(&update) {
                        Ok(json) => {
                            if sender.send(Message::Text(json.into())).await.is_err() {
                                // Client disconnected
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Failed to serialize update: {}", e);
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    // Client is slow, missed some messages
                    debug!("WebSocket client lagged, missed {} messages", count);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    // Channel closed, server shutting down
                    break;
                }
            }
        }
    });

    // Handle incoming messages (commands from client)
    while let Some(result) = receiver.next().await {
        match result {
            Ok(Message::Text(text)) => {
                debug!("Received client message: {}", text);
                // Command handling will be added in Plan 07
            }
            Ok(Message::Close(_)) => {
                info!("WebSocket client disconnected gracefully");
                break;
            }
            Ok(Message::Ping(data)) => {
                // Axum handles Pong automatically, but log for debugging
                debug!("Received ping: {:?}", data);
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {
                // Ignore binary, pong messages
            }
        }
    }

    // Clean up: abort the send task
    send_task.abort();
    info!("WebSocket connection closed");
}

use tokio::sync::broadcast;
