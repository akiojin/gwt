use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::debug;

/// A server-sent event that can be broadcast to all WebSocket clients.
#[derive(Debug, Clone, Serialize)]
pub struct ServerEvent {
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    pub payload: serde_json::Value,
}

/// Broadcasts events to all connected WebSocket clients.
#[derive(Clone)]
pub struct EventBroadcaster {
    sender: broadcast::Sender<ServerEvent>,
}

impl EventBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Send an event to all connected clients.
    pub fn send(&self, event: ServerEvent) {
        // Ignore send errors (no active receivers).
        let _ = self.sender.send(event);
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.sender.subscribe()
    }
}

/// axum handler for WebSocket upgrade at `/ws`.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(broadcaster): State<Arc<EventBroadcaster>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_connection(socket, broadcaster))
}

async fn handle_ws_connection(socket: WebSocket, broadcaster: Arc<EventBroadcaster>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = broadcaster.subscribe();

    // Forward broadcast events to this WebSocket client.
    let send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            let msg = match serde_json::to_string(&event) {
                Ok(json) => Message::Text(json.into()),
                Err(_) => continue,
            };
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Read from client (currently we just drain to detect disconnect).
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(_msg)) = receiver.next().await {
            // Client messages are currently unused.
        }
    });

    // Wait for either side to finish.
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    debug!("WebSocket client disconnected");
}
