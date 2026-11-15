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
//! An approved device reconnects on `BROWSER_RECONNECT_PATH` instead, with the
//! single message `{"type":"request_reconnect","host_id","project","device_id"}`.
//! It is routed only to the one live registration of that Host and Project, and
//! the Host is sent `connection_reconnect` with the named device ID; the
//! browser gets `reconnect_routed`, or `reconnect_rejected` without a reason.
//! Reaching the Host is not device approval: the Host verifies the device.
//!
//! A claimed (or reconnected) browser may then only send `{"type":"signal","payload"}` with an
//! opaque string; it is relayed to its Host as `connection_signal`, waiting for
//! room in the Host's backlog (one pending signal per connection). A signal
//! whose relayed frame would exceed the message bound, or any other data
//! frame, closes the socket. Host signals for this connection arrive as
//! `{"type":"signal","payload"}`. Every data message after the claim spends
//! one token of the connection's configured device rate (burst, refilled per
//! second); a message beyond it is not relayed, and the browser is sent
//! `{"type":"rate_limited"}` and closed. Nothing is queued or retried.
//!
//! A claimed socket ends when the server stops, when its Host registration is
//! logically closed or its lease passes. Every exit after a claim logically
//! closes that connection once and announces `connection_closed` to the Host;
//! the invite stays a replay fence until its TTL.

use super::host_route::{
    announce_open, call, BrowserConnections, HostEvent, HostState, SharedSessions, SignalBudget,
    BROWSER_BACKLOG,
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
use gwt_core::repo_hash::ProjectKey;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;
use zeroize::Zeroizing;

pub(super) const BROWSER_CLAIM_PATH: &str = "/browser/v1/claim";
pub(super) const BROWSER_RECONNECT_PATH: &str = "/browser/v1/reconnect";
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
    RequestReconnect {
        host_id: Uuid,
        project: String,
        device_id: Uuid,
    },
}

/// Which single opening message a browser path accepts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    Claim,
    Reconnect,
}

impl Entry {
    fn replies(self) -> (&'static str, &'static str) {
        match self {
            Self::Claim => ("claimed", "claim_rejected"),
            Self::Reconnect => ("reconnect_routed", "reconnect_rejected"),
        }
    }
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
    open(state, headers, upgrade, Entry::Claim)
}

pub(super) async fn reconnect(
    State(state): State<HostState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    open(state, headers, upgrade, Entry::Reconnect)
}

fn open(state: HostState, headers: HeaderMap, upgrade: WebSocketUpgrade, entry: Entry) -> Response {
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
        .on_upgrade(move |socket| serve_browser(socket, state, entry))
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

/// Waits up to `CLAIM_TIMEOUT` for the single opening message of `entry`.
async fn read_opening(
    socket: &mut WebSocket,
    stopping: &mut watch::Receiver<bool>,
    entry: Entry,
) -> Option<BrowserMessage> {
    let deadline = tokio::time::Instant::now() + CLAIM_TIMEOUT;
    loop {
        let message = tokio::select! {
            message = socket.recv() => message,
            _ = tokio::time::sleep_until(deadline) => return None,
            _ = stopping.wait_for(|stopped| *stopped) => return None,
        };
        match message {
            Some(Ok(Message::Text(text))) => {
                let message = serde_json::from_str(text.as_str()).ok()?;
                let expected = match message {
                    BrowserMessage::ClaimInvite { .. } => Entry::Claim,
                    BrowserMessage::RequestReconnect { .. } => Entry::Reconnect,
                };
                return (expected == entry).then_some(message);
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return None,
        }
    }
}

/// Admits the opening message: the registration it routes to and the event
/// that opens the connection for that Host. `None` for any refusal.
fn admit(
    sessions: &SharedSessions,
    message: BrowserMessage,
    connection_id: Uuid,
) -> Option<(RegistrationHandle, HostEvent)> {
    match message {
        BrowserMessage::ClaimInvite { secret } => {
            let invite: [u8; 32] = Sha256::digest(secret.as_slice()).into();
            let (handle, _) = call(sessions, |sessions, now, wall| {
                sessions.claim_invite_by_secret(&*secret, connection_id, now, wall)
            })
            .ok()?;
            let opened = HostEvent::Claimed {
                connection_id,
                invite,
            };
            Some((handle, opened))
        }
        BrowserMessage::RequestReconnect {
            host_id,
            project,
            device_id,
        } => {
            let project = ProjectKey::parse(&project).ok()?;
            let handle = call(sessions, |sessions, now, wall| {
                sessions.request_reconnect(host_id, &project, connection_id, now, wall)
            })
            .ok()?;
            let opened = HostEvent::Reconnect {
                connection_id,
                device_id,
            };
            Some((handle, opened))
        }
    }
}

async fn serve_browser(mut socket: WebSocket, state: HostState, entry: Entry) {
    let HostState {
        sessions,
        mut stopping,
        alive,
        retired,
        events,
        connections,
        signal_rate,
        host_rate: _,
    } = state;
    // Subscribe before claiming so a registration closed meanwhile is seen.
    let mut retired = retired.subscribe();
    let (accepted, rejected) = entry.replies();
    if let Some(message) = read_opening(&mut socket, &mut stopping, entry).await {
        let connection_id = Uuid::new_v4();
        let claim_sessions = sessions.clone();
        let (outbound, mut inbound) = mpsc::channel(BROWSER_BACKLOG);
        let claimed = tokio::task::spawn_blocking(move || {
            let (handle, opened) = admit(&claim_sessions, message, connection_id)?;
            let generation = handle.generation();
            // Routed before the Host learns of it, so its first signal lands.
            let routed = connections
                .lock()
                .map(|mut table| table.insert(connection_id, (generation, outbound)))
                .is_ok();
            let announced = routed
                .then(|| announce_open(&events, generation, opened))
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
                if send_json(&mut socket, serde_json::json!({"type": accepted})).await {
                    let generation = claimed.handle.generation();
                    hold(
                        &mut socket,
                        &claimed,
                        generation,
                        SignalBudget::new(signal_rate, Instant::now()),
                        &mut inbound,
                        &mut stopping,
                        &mut retired,
                    )
                    .await;
                }
                let _ = tokio::task::spawn_blocking(move || drop(claimed)).await;
            }
            _ => {
                let _ = send_json(&mut socket, serde_json::json!({"type": rejected})).await;
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
    mut budget: SignalBudget,
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
            RateLimited,
        }
        let next = tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(Message::Text(_))) if !budget.take(Instant::now()) => Next::RateLimited,
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
            Next::RateLimited => {
                let _ = send_json(socket, serde_json::json!({"type": "rate_limited"})).await;
                return;
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
