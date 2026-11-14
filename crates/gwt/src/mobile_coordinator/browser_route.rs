//! Browser invite-claim WebSocket (Issue #6476).
//!
//! A browser must send exactly the configured Origin (a missing Origin is
//! refused here; a different one is refused by the HTTP boundary). Its first
//! and only message, within `CLAIM_TIMEOUT`, is
//! `{"type":"claim_invite","secret"}` with the 64-hex invite secret (never
//! escaped). The browser names no Host or Project: the coordinator routes by
//! the invite's saved scope and claims it once for a connection ID it
//! generates. It answers `claimed`, or `claim_rejected` without a reason and
//! closes. A claim is not device approval or read/input authority, and no
//! later message is defined yet, so any data frame closes the socket.
//!
//! A claimed socket ends when the server stops, when its Host registration is
//! logically closed or its lease passes. Every exit after a claim logically
//! closes that connection once; the invite stays a replay fence until its TTL.

use super::host_route::{call, HostState, SharedSessions};
use super::registration::RegistrationHandle;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;
use uuid::Uuid;
use zeroize::Zeroizing;

pub(super) const BROWSER_CLAIM_PATH: &str = "/browser/v1/claim";
/// The same bound as HTTP request bodies (the mobile policy's maximum SDP).
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// A browser that has not claimed within this bound releases its connection.
const CLAIM_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserMessage {
    ClaimInvite {
        #[serde(deserialize_with = "super::borrowed_hex_secret")]
        secret: Zeroizing<[u8; 32]>,
    },
}

pub(super) async fn claim(
    State(state): State<HostState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !headers.contains_key(header::ORIGIN) {
        return (
            StatusCode::FORBIDDEN,
            "coordinator invite claim requires the configured browser origin",
        )
            .into_response();
    }
    upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve_browser(socket, state))
}

/// One claimed connection; logically closed exactly once on drop.
struct Claimed {
    sessions: SharedSessions,
    handle: RegistrationHandle,
    connection_id: Uuid,
}

impl Claimed {
    /// The registration's current lease deadline, or `None` once it is gone.
    fn deadline(&self) -> Option<Instant> {
        let sessions = self.sessions.lock().ok()?;
        sessions
            .lease_deadline(&self.handle)
            .filter(|deadline| *deadline > Instant::now())
    }
}

impl Drop for Claimed {
    fn drop(&mut self) {
        // A registration that is already gone has invalidated its invites.
        let _ = call(&self.sessions, |sessions, now, wall| {
            sessions.close_connection(&self.handle, self.connection_id, now, wall)
        });
    }
}

async fn send_json(socket: &mut WebSocket, value: serde_json::Value) -> bool {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .is_ok()
}

/// Waits up to `CLAIM_TIMEOUT` for the single claim message.
async fn read_claim(
    socket: &mut WebSocket,
    stopping: &mut watch::Receiver<bool>,
) -> Option<Zeroizing<[u8; 32]>> {
    let deadline = tokio::time::Instant::now() + CLAIM_TIMEOUT;
    loop {
        let message = tokio::select! {
            message = socket.recv() => message,
            _ = tokio::time::sleep_until(deadline) => return None,
            _ = stopping.wait_for(|stopped| *stopped) => return None,
        };
        match message {
            Some(Ok(Message::Text(text))) => {
                let BrowserMessage::ClaimInvite { secret } =
                    serde_json::from_str(text.as_str()).ok()?;
                return Some(secret);
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return None,
        }
    }
}

async fn serve_browser(mut socket: WebSocket, state: HostState) {
    let HostState {
        sessions,
        mut stopping,
        alive,
        retired,
    } = state;
    // Subscribe before claiming so a registration closed meanwhile is seen.
    let mut retired = retired.subscribe();
    if let Some(secret) = read_claim(&mut socket, &mut stopping).await {
        let connection_id = Uuid::new_v4();
        let claim_sessions = sessions.clone();
        let claimed = tokio::task::spawn_blocking(move || {
            call(&claim_sessions, |sessions, now, wall| {
                sessions.claim_invite_by_secret(&*secret, connection_id, now, wall)
            })
            .map(|(handle, _)| {
                Arc::new(Claimed {
                    sessions: claim_sessions,
                    handle,
                    connection_id,
                })
            })
        })
        .await;
        match claimed {
            Ok(Ok(claimed)) => {
                if send_json(&mut socket, serde_json::json!({"type": "claimed"})).await {
                    hold(&mut socket, &claimed, &mut stopping, &mut retired).await;
                }
                let _ = tokio::task::spawn_blocking(move || drop(claimed)).await;
            }
            _ => {
                let _ = send_json(&mut socket, serde_json::json!({"type": "claim_rejected"})).await;
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
    drop(alive);
}

/// Keeps a claimed socket open while its registration lives.
async fn hold(
    socket: &mut WebSocket,
    claimed: &Arc<Claimed>,
    stopping: &mut watch::Receiver<bool>,
    retired: &mut watch::Receiver<()>,
) {
    loop {
        // The sessions lock may wait on a store checkpoint; never block here.
        let lookup = Arc::clone(claimed);
        let Ok(Some(deadline)) = tokio::task::spawn_blocking(move || lookup.deadline()).await
        else {
            return;
        };
        tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return,
            },
            changed = retired.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            _ = tokio::time::sleep_until(deadline.into()) => {}
            _ = stopping.wait_for(|stopped| *stopped) => return,
        }
    }
}
