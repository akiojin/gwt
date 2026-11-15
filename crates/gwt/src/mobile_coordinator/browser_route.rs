//! Browser invite-claim WebSocket (Issue #6476).
//!
//! A browser must send exactly the configured Origin (a missing Origin is
//! refused here; a different one is refused by the HTTP boundary). Its first
//! and only message, within `CLAIM_TIMEOUT`, is
//! `{"type":"claim_invite","secret"}` with the 64-hex invite secret (never
//! escaped). The browser names no Host or Project: the coordinator routes by
//! the invite's saved scope and claims it once for a connection ID it
//! generates. It answers `claimed` only after queueing `connection_claimed`
//! for the owning Host registration and reserving room for the matching
//! `connection_closed`; otherwise (including a Host whose event backlog is
//! full) it answers `claim_rejected` without a reason and closes. A claim is
//! not device approval or read/input authority.
//!
//! A claimed browser may then only send `{"type":"signal","payload"}` with an
//! opaque string; it is relayed to its Host as `connection_signal`, waiting for
//! room in the Host's backlog (one pending signal per connection). A signal
//! whose relayed frame would exceed the message bound, or any other data
//! frame, closes the socket. Host signals for this connection arrive as
//! `{"type":"signal","payload"}`.
//!
//! A claimed socket ends when the server stops, when its Host registration is
//! logically closed or its lease passes. Every exit after a claim logically
//! closes that connection once and announces `connection_closed` to the Host;
//! the invite stays a replay fence until its TTL.

use super::host_route::{
    announce_claim, call, BrowserConnections, HostEvent, HostState, SharedSessions, BROWSER_BACKLOG,
};
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
use tokio::sync::{mpsc, watch};
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

/// The only message of a claimed browser.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ClaimedMessage {
    Signal { payload: String },
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

/// One claimed connection; logically closed exactly once on drop, and its
/// Host told of the close through the slot reserved when it was told of the
/// claim.
struct Claimed {
    sessions: SharedSessions,
    handle: RegistrationHandle,
    connection_id: Uuid,
    connections: BrowserConnections,
    host: mpsc::Sender<HostEvent>,
    closed: Option<mpsc::OwnedPermit<HostEvent>>,
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
        if let Ok(mut table) = self.connections.lock() {
            table.remove(&self.connection_id);
        }
        // A registration that is already gone has invalidated its invites.
        let _ = call(&self.sessions, |sessions, now, wall| {
            sessions.close_connection(&self.handle, self.connection_id, now, wall)
        });
        if let Some(closed) = self.closed.take() {
            // A Host socket that already ended discards it.
            closed.send(HostEvent::Closed(self.connection_id));
        }
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
        events,
        connections,
    } = state;
    // Subscribe before claiming so a registration closed meanwhile is seen.
    let mut retired = retired.subscribe();
    if let Some(secret) = read_claim(&mut socket, &mut stopping).await {
        let connection_id = Uuid::new_v4();
        let claim_sessions = sessions.clone();
        let (outbound, mut inbound) = mpsc::channel(BROWSER_BACKLOG);
        let claimed = tokio::task::spawn_blocking(move || {
            let (handle, _) = call(&claim_sessions, |sessions, now, wall| {
                sessions.claim_invite_by_secret(&*secret, connection_id, now, wall)
            })
            .ok()?;
            let generation = handle.generation();
            // Routed before the Host learns of it, so its first signal lands.
            let routed = connections
                .lock()
                .map(|mut table| table.insert(connection_id, (generation, outbound)))
                .is_ok();
            let announced = routed
                .then(|| announce_claim(&events, generation, connection_id))
                .flatten();
            // A closed sender stands in when unannounced; the claim is then
            // dropped (closed and unrouted) here, on the blocking thread.
            let (host, closed) = match announced {
                Some((host, closed)) => (host, Some(closed)),
                None => (mpsc::channel(1).0, None),
            };
            let claimed = Claimed {
                sessions: claim_sessions,
                handle,
                connection_id,
                connections,
                host,
                closed,
            };
            claimed.closed.is_some().then(|| Arc::new(claimed))
        })
        .await;
        match claimed {
            Ok(Some(claimed)) => {
                if send_json(&mut socket, serde_json::json!({"type": "claimed"})).await {
                    let generation = claimed.handle.generation();
                    hold(
                        &mut socket,
                        &claimed,
                        generation,
                        &mut inbound,
                        &mut stopping,
                        &mut retired,
                    )
                    .await;
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

/// The `connection_signal` frame for the Host, or `None` for any message other
/// than a signal whose relayed frame fits the message bound.
fn host_signal(text: &str, generation: Uuid, connection_id: Uuid) -> Option<String> {
    let ClaimedMessage::Signal { payload } = serde_json::from_str(text).ok()?;
    let frame = serde_json::json!({
        "type": "connection_signal", "generation": generation,
        "connection_id": connection_id, "payload": payload,
    })
    .to_string();
    (frame.len() <= MAX_MESSAGE_BYTES).then_some(frame)
}

/// Keeps a claimed socket open while its registration lives, relaying signals
/// both ways.
async fn hold(
    socket: &mut WebSocket,
    claimed: &Arc<Claimed>,
    generation: Uuid,
    inbound: &mut mpsc::Receiver<String>,
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
        enum Next {
            ToHost(String),
            ToBrowser(String),
        }
        let next = tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(Message::Text(text))) => {
                    match host_signal(text.as_str(), generation, claimed.connection_id) {
                        Some(frame) => Next::ToHost(frame),
                        None => return,
                    }
                }
                _ => return,
            },
            frame = inbound.recv() => match frame {
                Some(frame) => Next::ToBrowser(frame),
                None => return,
            },
            changed = retired.changed() => {
                if changed.is_err() {
                    return;
                }
                continue;
            }
            _ = tokio::time::sleep_until(deadline.into()) => continue,
            _ = stopping.wait_for(|stopped| *stopped) => return,
        };
        let signal = match next {
            Next::ToHost(signal) => signal,
            Next::ToBrowser(frame) => {
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    return;
                }
                continue;
            }
        };
        // Waits for room in the Host backlog: one pending signal per browser.
        tokio::select! {
            sent = claimed.host.send(HostEvent::Signal(signal)) => {
                if sent.is_err() {
                    return;
                }
            }
            _ = stopping.wait_for(|stopped| *stopped) => return,
        }
    }
}
