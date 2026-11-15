//! Host-only registration WebSocket (Issue #6476).
//!
//! A PC Host authenticates with exactly one `Authorization: Bearer` header whose
//! token is base64url (no padding) JSON naming its credential identity, one
//! Project, its trusted Host epoch and the 32-byte secret in hex. A request with
//! any Origin is a browser and is refused; browsers claim invites on the
//! separate `browser_route`. The registration exists only while its socket is served: every
//! exit path, including a failed upgrade, logically closes it once.
//!
//! The coordinator answers `{"type":"registered","generation","ttl_secs"}` and
//! echoes each `{"type":"renew","generation","nonce"}` as `renewed` with the
//! remaining lease seconds. The Host's request-start/nonce lease remains its own
//! contract; this route never extends a lease beyond the domain deadline and
//! closes the socket when that deadline passes, or when the server stops.
//!
//! `{"type":"register_invite","generation","nonce","secret","ttl_secs"}` adds a
//! Host-generated invite (64 hex, never escaped) to the registration's scope
//! and answers `invite_registered`, or `invite_rejected` with a `reason` for a
//! rejection the Host can recover from by generating another invite. Any other
//! failure loses the registration and closes the socket. On the accepted path
//! the secret is decoded from the borrowed frame text straight into a wiped
//! buffer; an escaped secret is refused (after serde has made its own
//! unescaped copies), and neither the frame buffer nor stack copies left by
//! moves are erased. Device authority is out of scope. Closed invites stay as replay fences until their TTL; they count
//! toward the global 64-record bound and a per-Host 16-record bound, so one
//! Host's fences cannot exhaust other Hosts' invites (`host_capacity`).
//!
//! When a browser claims one of its invites, the Host is sent
//! `{"type":"connection_claimed","generation","connection_id","invite"}` (the
//! 64-hex SHA-256 digest of the claimed invite's secret), each opaque
//! browser signal as `{"type":"connection_signal",...,"payload"}`, and
//! `{"type":"connection_closed",...}` once that claimed connection is logically
//! closed (claimed, signals, then closed). An approved device's reconnect
//! (`browser_route`) is announced as
//! `{"type":"connection_reconnect","generation","connection_id","device_id"}`
//! and then relayed and closed the same way. Only the owning registration
//! receives them. The Host answers with
//! `{"type":"signal","generation","connection_id","payload"}`, relayed only to
//! that connection while it is one of the registration's live claims; any
//! other connection ID is ignored, and a browser that falls behind is closed.
//! `{"type":"close_connection","generation","connection_id"}` closes one of
//! the registration's own claimed connections (others are ignored).
//!
//! Every Host data message spends one token of the registration's configured
//! Host rate (burst, refilled per second); a message beyond it is not served,
//! and the Host is sent `{"type":"rate_limited","generation"}` before the
//! registration is logically closed. Nothing is queued or retried.
//!
//! Each call reads the real wall clock; a backwards step makes the domain gate
//! terminal (fail closed) until the operator restarts the coordinator.

use super::{
    invite::InviteError,
    registration::{CoordinatorSessions, RegistrationError, RegistrationHandle},
    CredentialError, CredentialIdentity,
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use gwt_core::repo_hash::ProjectKey;
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;
use zeroize::Zeroizing;

pub(super) const HOST_REGISTER_PATH: &str = "/host/v1/register";
/// The same bound as HTTP request bodies (the mobile policy's maximum SDP).
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_TOKEN_CHARS: usize = 1024;
const UNAUTHORIZED: &str =
    "coordinator host registration rejected; import a valid Host credential and retry";

/// Shared by every connection service so the private store guard lives until
/// the last accepted connection closes.
pub(super) type SharedSessions = Arc<Mutex<CoordinatorSessions>>;

/// Bound of undelivered connection events per Host: opened and closed for
/// each of the per-Host 16 invite records and 8 reconnects.
const EVENT_BACKLOG: usize = 48;

/// A claimed browser connection's lifecycle and signals, announced to its Host.
#[derive(Debug)]
pub(super) enum HostEvent {
    /// A claimed connection and the SHA-256 digest of the claimed invite's
    /// secret (the registry's key), so the Host can tell which invite it was.
    Claimed {
        connection_id: Uuid,
        invite: [u8; 32],
    },
    /// An approved device's reconnect, by the device ID the browser named;
    /// the Host verifies the device itself.
    Reconnect {
        connection_id: Uuid,
        device_id: Uuid,
    },
    /// A complete `connection_signal` frame, already checked against the
    /// message bound by the browser route.
    Signal(String),
    Closed(Uuid),
}

/// Event senders of live registrations, by registration generation.
pub(super) type HostEvents = Arc<Mutex<HashMap<Uuid, mpsc::Sender<HostEvent>>>>;

/// Bound of undelivered Host signal frames per claimed browser connection.
pub(super) const BROWSER_BACKLOG: usize = 32;

/// Live claimed browser connections, by connection ID: the owning registration
/// generation and the sender of frames for that browser.
pub(super) type BrowserConnections = Arc<Mutex<HashMap<Uuid, (Uuid, mpsc::Sender<String>)>>>;

/// Relays a Host signal to one of its own live claimed connections. A
/// connection that is gone or belongs to another registration is ignored (it
/// may have just closed); a browser that fell behind is dropped from the table,
/// which closes it. `false` only when the table is unusable.
fn relay_to_browser(
    connections: &BrowserConnections,
    generation: Uuid,
    connection_id: Uuid,
    payload: String,
) -> bool {
    let Ok(mut table) = connections.lock() else {
        return false;
    };
    let Some((_, sender)) = table
        .get(&connection_id)
        .filter(|(owner, _)| *owner == generation)
    else {
        return true;
    };
    let frame = serde_json::json!({"type": "signal", "payload": payload}).to_string();
    if sender.try_send(frame).is_err() {
        table.remove(&connection_id);
    }
    true
}

/// Closes one of the registration's own live claimed connections by dropping
/// its route (the browser then closes); others are ignored. `false` only when
/// the table is unusable.
fn close_browser(connections: &BrowserConnections, generation: Uuid, connection_id: Uuid) -> bool {
    let Ok(mut table) = connections.lock() else {
        return false;
    };
    if table
        .get(&connection_id)
        .is_some_and(|(owner, _)| *owner == generation)
    {
        table.remove(&connection_id);
    }
    true
}

/// Queues the `opened` event (a claim or reconnect) for the live registration
/// `generation` and returns the Host's event sender (for signals) with the slot reserved for
/// its `connection_closed`, so the close can never be lost to a full backlog.
/// `None` when the registration is gone or its backlog cannot hold both
/// events (the caller fails closed).
pub(super) fn announce_open(
    events: &HostEvents,
    generation: Uuid,
    opened: HostEvent,
) -> Option<(mpsc::Sender<HostEvent>, mpsc::OwnedPermit<HostEvent>)> {
    let sender = events.lock().ok()?.get(&generation)?.clone();
    let closed = sender.clone().try_reserve_owned().ok()?;
    sender.try_send(opened).ok()?;
    Some((sender, closed))
}

/// Route state. `stopping` ends every Host socket when the server stops; each
/// socket holds an `alive` sender until its registration is logically closed,
/// so the server can wait for all of them before releasing the registry.
/// `retired` is notified after each registration's logical close, so claimed
/// browser sockets of that registration can end. `events` routes claimed
/// connection events to the owning Host socket, and `connections` routes Host
/// signals to one claimed browser.
#[derive(Clone)]
pub(super) struct HostState {
    pub(super) sessions: SharedSessions,
    pub(super) stopping: watch::Receiver<bool>,
    pub(super) alive: mpsc::Sender<()>,
    pub(super) retired: Arc<watch::Sender<()>>,
    pub(super) events: HostEvents,
    pub(super) connections: BrowserConnections,
    /// The configured per-device message rate of each claimed browser.
    pub(super) signal_rate: SignalRate,
    /// The configured message rate of each Host registration socket.
    pub(super) host_rate: SignalRate,
    /// Each source's budget for browser claim/reconnect upgrade requests.
    pub(super) opens: Arc<super::browser_route::OpenBudgets>,
}

/// A token-bucket policy: `burst` messages at once, refilled at
/// `per_second`. Both are positive (validated with the config).
#[derive(Clone, Copy, Debug)]
pub(super) struct SignalRate {
    pub(super) per_second: u16,
    pub(super) burst: u16,
}

/// One socket's token bucket for its data messages (a claimed browser's or a
/// Host registration's).
pub(super) struct SignalBudget {
    rate: SignalRate,
    tokens: f64,
    refilled: Instant,
}

impl SignalBudget {
    pub(super) fn new(rate: SignalRate, now: Instant) -> Self {
        Self {
            rate,
            tokens: f64::from(rate.burst),
            refilled: now,
        }
    }

    /// Whether the bucket has refilled to its burst by `now`.
    pub(super) fn is_full(&self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.refilled).as_secs_f64();
        self.tokens + elapsed * f64::from(self.rate.per_second) >= f64::from(self.rate.burst)
    }

    /// Spends one token at `now`; `false` when none is left.
    pub(super) fn take(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.refilled).as_secs_f64();
        self.tokens = (self.tokens + elapsed * f64::from(self.rate.per_second))
            .min(f64::from(self.rate.burst));
        self.refilled = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostToken {
    schema_version: u32,
    host_id: Uuid,
    credential_id: Uuid,
    generation: Uuid,
    project: String,
    host_epoch: Uuid,
    #[serde(deserialize_with = "super::borrowed_hex_secret")]
    secret: Zeroizing<[u8; 32]>,
}

struct HostCredentials {
    identity: CredentialIdentity,
    project: ProjectKey,
    host_epoch: Uuid,
    secret: Zeroizing<[u8; 32]>,
}

fn decode_token(token: &str) -> Option<Zeroizing<Vec<u8>>> {
    if token.len() > MAX_TOKEN_CHARS {
        return None;
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token)
        .ok()
        .map(Zeroizing::new)
}

fn parse_authorization(headers: &HeaderMap) -> Option<HostCredentials> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let bytes = decode_token(value.to_str().ok()?.strip_prefix("Bearer ")?)?;
    let token: HostToken = serde_json::from_slice(&bytes).ok()?;
    if token.schema_version != 1 {
        return None;
    }
    Some(HostCredentials {
        identity: CredentialIdentity {
            host_id: token.host_id,
            credential_id: token.credential_id,
            generation: token.generation,
        },
        project: ProjectKey::parse(&token.project).ok()?,
        host_epoch: token.host_epoch,
        secret: token.secret,
    })
}

/// Synchronous domain call; the store checkpoint fsyncs at most once per
/// trusted wall second, so async callers run it on a blocking thread. The
/// monotonic instant is read under the lock.
pub(super) fn call<T>(
    sessions: &SharedSessions,
    f: impl FnOnce(&mut CoordinatorSessions, Instant, u64) -> Result<T, RegistrationError>,
) -> Result<T, RegistrationError> {
    let mut sessions = sessions.lock().map_err(|_| RegistrationError::Unusable)?;
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RegistrationError::ClockRollback)?
        .as_secs();
    f(&mut sessions, Instant::now(), wall)
}

/// Owns one live registration and logically closes it exactly once on drop.
struct Registration {
    sessions: SharedSessions,
    handle: RegistrationHandle,
    retired: Arc<watch::Sender<()>>,
    events: HostEvents,
    connections: BrowserConnections,
}

impl Registration {
    fn renew(&self) -> Result<Instant, RegistrationError> {
        call(&self.sessions, |sessions, now, wall| {
            sessions.renew(&self.handle, now, wall)?;
            sessions
                .lease_deadline(&self.handle)
                .ok_or(RegistrationError::DeadRegistration)
        })
    }

    fn register_invite(&self, secret: &[u8; 32], ttl_secs: u64) -> Result<(), RegistrationError> {
        call(&self.sessions, |sessions, now, wall| {
            sessions.register_invite(&self.handle, secret, ttl_secs, now, wall)
        })
    }
}

/// Fallback for paths that cannot await (a failed upgrade); normal socket exits
/// drop the registration on a blocking thread.
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut events) = self.events.lock() {
            events.remove(&self.handle.generation());
        }
        let _ = call(&self.sessions, |sessions, now, wall| {
            sessions.close(&self.handle, now, wall)
        });
        self.retired.send_modify(|_| {});
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum HostMessage {
    Renew {
        generation: Uuid,
        nonce: Uuid,
    },
    RegisterInvite {
        generation: Uuid,
        nonce: Uuid,
        #[serde(deserialize_with = "super::borrowed_hex_secret")]
        secret: Zeroizing<[u8; 32]>,
        ttl_secs: u64,
    },
    Signal {
        generation: Uuid,
        connection_id: Uuid,
        payload: String,
    },
    CloseConnection {
        generation: Uuid,
        connection_id: Uuid,
    },
}

/// Invite rejections that leave the registration intact; the Host recovers by
/// generating a fresh invite or waiting for a pending one to expire.
fn recoverable_invite_rejection(error: &RegistrationError) -> Option<&'static str> {
    match error {
        RegistrationError::Invite(InviteError::InvalidTtl) => Some("invalid_ttl"),
        RegistrationError::Invite(InviteError::AlreadyRegistered) => Some("already_registered"),
        RegistrationError::Invite(InviteError::HostCapacity) => Some("host_capacity"),
        RegistrationError::Invite(InviteError::Capacity) => Some("capacity"),
        _ => None,
    }
}

/// Remaining lease seconds, rounded up so a fresh lease reports 30.
fn ttl_secs(deadline: Instant) -> u64 {
    let left = deadline.saturating_duration_since(Instant::now());
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

pub(super) async fn register(
    State(state): State<HostState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if headers.contains_key(header::ORIGIN) {
        return (
            StatusCode::FORBIDDEN,
            "coordinator host registration refuses browser origins; use the Host client",
        )
            .into_response();
    }
    let Some(credentials) = parse_authorization(&headers) else {
        return (StatusCode::UNAUTHORIZED, UNAUTHORIZED).into_response();
    };
    // The header bytes cannot be wiped; at least do not keep them alive.
    drop(headers);
    let sessions = Arc::clone(&state.sessions);
    let registered = tokio::task::spawn_blocking(move || {
        call(&sessions, |sessions, now, wall| {
            let handle = sessions.register(
                &credentials.identity,
                &credentials.project,
                &*credentials.secret,
                credentials.host_epoch,
                now,
                wall,
            )?;
            match sessions.lease_deadline(&handle) {
                Some(deadline) => Ok((handle, deadline)),
                None => {
                    let _ = sessions.close(&handle, now, wall);
                    Err(RegistrationError::DeadRegistration)
                }
            }
        })
    })
    .await
    .unwrap_or(Err(RegistrationError::Unusable));
    let (handle, deadline) = match registered {
        Ok(registered) => registered,
        Err(
            RegistrationError::InvalidEpoch
            | RegistrationError::Credential(CredentialError::Unauthorized | CredentialError::Expired),
        ) => return (StatusCode::UNAUTHORIZED, UNAUTHORIZED).into_response(),
        Err(RegistrationError::Capacity) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "coordinator host registration capacity reached; retry later",
            )
                .into_response()
        }
        Err(_) => return unavailable(),
    };
    let generation = handle.generation();
    let (sender, events) = mpsc::channel(EVENT_BACKLOG);
    let registration = Registration {
        sessions: state.sessions,
        handle,
        retired: state.retired,
        events: state.events,
        connections: state.connections,
    };
    // A poisoned route table drops (closes) the registration; the guard is
    // released before that drop takes the lock again.
    let routed = registration
        .events
        .lock()
        .map(|mut routes| {
            routes.insert(generation, sender);
        })
        .is_ok();
    if !routed {
        return unavailable();
    }
    let (stopping, alive) = (state.stopping, state.alive);
    let budget = SignalBudget::new(state.host_rate, Instant::now());
    upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| {
            serve_host(
                socket,
                registration,
                generation,
                deadline,
                budget,
                stopping,
                alive,
                events,
            )
        })
}

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "coordinator host registration unavailable; the operator must repair and restart the coordinator",
    )
        .into_response()
}

async fn send_json(socket: &mut WebSocket, value: serde_json::Value) -> bool {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .is_ok()
}

/// Runs a synchronous domain call on a blocking thread; `None` if it panicked.
async fn blocking<T: Send + 'static>(
    registration: Registration,
    f: impl FnOnce(&Registration) -> T + Send + 'static,
) -> Option<(Registration, T)> {
    tokio::task::spawn_blocking(move || {
        let result = f(&registration);
        (registration, result)
    })
    .await
    .ok()
}

#[allow(clippy::too_many_arguments)]
async fn serve_host(
    mut socket: WebSocket,
    mut registration: Registration,
    generation: Uuid,
    mut deadline: Instant,
    mut budget: SignalBudget,
    mut stopping: watch::Receiver<bool>,
    alive: mpsc::Sender<()>,
    mut events: mpsc::Receiver<HostEvent>,
) {
    let registered = serde_json::json!({
        "type": "registered", "generation": generation, "ttl_secs": ttl_secs(deadline),
    });
    if send_json(&mut socket, registered).await {
        loop {
            enum Input {
                Socket(Option<Result<Message, axum::Error>>),
                Event(HostEvent),
            }
            let input = tokio::select! {
                message = socket.recv() => Input::Socket(message),
                event = events.recv() => match event {
                    Some(event) => Input::Event(event),
                    None => break,
                },
                _ = tokio::time::sleep_until(deadline.into()) => break,
                _ = stopping.wait_for(|stopped| *stopped) => break,
            };
            let message = match input {
                Input::Socket(message) => message,
                Input::Event(event) => {
                    let event = match event {
                        HostEvent::Claimed {
                            connection_id,
                            invite,
                        } => serde_json::json!({
                            "type": "connection_claimed", "generation": generation,
                            "connection_id": connection_id, "invite": hex::encode(invite),
                        }),
                        HostEvent::Reconnect {
                            connection_id,
                            device_id,
                        } => serde_json::json!({
                            "type": "connection_reconnect", "generation": generation,
                            "connection_id": connection_id, "device_id": device_id,
                        }),
                        HostEvent::Closed(connection_id) => serde_json::json!({
                            "type": "connection_closed", "generation": generation,
                            "connection_id": connection_id,
                        }),
                        HostEvent::Signal(frame) => {
                            if socket.send(Message::Text(frame.into())).await.is_err() {
                                break;
                            }
                            continue;
                        }
                    };
                    if !send_json(&mut socket, event).await {
                        break;
                    }
                    continue;
                }
            };
            let text = match message {
                Some(Ok(Message::Text(_))) if !budget.take(Instant::now()) => {
                    let limited = serde_json::json!({
                        "type": "rate_limited", "generation": generation,
                    });
                    let _ = send_json(&mut socket, limited).await;
                    break;
                }
                Some(Ok(Message::Text(text))) => text,
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => break,
            };
            let Ok(message) = serde_json::from_str::<HostMessage>(text.as_str()) else {
                break;
            };
            let reply = match message {
                HostMessage::Renew {
                    generation: renewed,
                    nonce,
                } if renewed == generation => {
                    // A panicked call already dropped the registration, which
                    // closes it unless the panic poisoned the sessions lock.
                    let Some((returned, result)) = blocking(registration, |r| r.renew()).await
                    else {
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    };
                    registration = returned;
                    let Ok(next) = result else {
                        break;
                    };
                    deadline = next;
                    serde_json::json!({
                        "type": "renewed", "generation": generation, "nonce": nonce,
                        "ttl_secs": ttl_secs(deadline),
                    })
                }
                HostMessage::RegisterInvite {
                    generation: scoped,
                    nonce,
                    secret,
                    ttl_secs: invite_ttl,
                } if scoped == generation => {
                    let invite = move |r: &Registration| r.register_invite(&secret, invite_ttl);
                    let Some((returned, result)) = blocking(registration, invite).await else {
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    };
                    registration = returned;
                    match result.as_ref().map_err(recoverable_invite_rejection) {
                        Ok(()) => serde_json::json!({
                            "type": "invite_registered", "generation": generation, "nonce": nonce,
                        }),
                        Err(Some(reason)) => serde_json::json!({
                            "type": "invite_rejected", "generation": generation, "nonce": nonce,
                            "reason": reason,
                        }),
                        Err(None) => break,
                    }
                }
                HostMessage::Signal {
                    generation: scoped,
                    connection_id,
                    payload,
                } if scoped == generation => {
                    if !relay_to_browser(
                        &registration.connections,
                        generation,
                        connection_id,
                        payload,
                    ) {
                        break;
                    }
                    continue;
                }
                HostMessage::CloseConnection {
                    generation: scoped,
                    connection_id,
                } if scoped == generation => {
                    if !close_browser(&registration.connections, generation, connection_id) {
                        break;
                    }
                    continue;
                }
                _ => break,
            };
            if !send_json(&mut socket, reply).await {
                break;
            }
        }
    }
    // Logical close precedes the best-effort socket close and `alive` release.
    let _ = tokio::task::spawn_blocking(move || drop(registration)).await;
    let _ = socket.send(Message::Close(None)).await;
    drop(alive);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(header::AUTHORIZATION, value.parse().unwrap());
        }
        headers
    }

    fn token(value: serde_json::Value) -> String {
        format!(
            "Bearer {}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&value).unwrap())
        )
    }

    fn valid() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1, "host_id": Uuid::new_v4(), "credential_id": Uuid::new_v4(),
            "generation": Uuid::new_v4(), "project": "0123456789abcdef",
            "host_epoch": Uuid::new_v4(), "secret": "ab".repeat(32),
        })
    }

    #[test]
    fn bearer_tokens_are_parsed_strictly() {
        let parsed = parse_authorization(&headers(&[&token(valid())])).unwrap();
        assert_eq!(*parsed.secret, [0xab; 32]);
        assert_eq!(parsed.project.as_str(), "0123456789abcdef");
        for (field, value) in [
            ("schema_version", serde_json::json!(2)),
            ("secret", serde_json::json!("ab".repeat(31))),
            ("secret", serde_json::json!("zz".repeat(32))),
            ("project", serde_json::json!("../escape")),
            ("extra", serde_json::json!(true)),
        ] {
            let mut value_json = valid();
            value_json[field] = value;
            assert!(parse_authorization(&headers(&[&token(value_json)])).is_none());
        }
        let mut missing = valid();
        missing.as_object_mut().unwrap().remove("host_epoch");
        assert!(parse_authorization(&headers(&[&token(missing)])).is_none());
        let padded = format!("{}=", token(valid()));
        assert!(parse_authorization(&headers(&[&padded])).is_none());
        assert!(parse_authorization(&headers(&[])).is_none());
    }

    fn zeroizes_on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}

    #[test]
    fn owned_secret_holders_are_zeroize_on_drop_and_escaped_secrets_are_refused() {
        let token = token(valid());
        let bearer = token.strip_prefix("Bearer ").unwrap();
        zeroizes_on_drop(&decode_token(bearer).unwrap());
        let parsed = parse_authorization(&headers(&[&token])).unwrap();
        zeroizes_on_drop(&parsed.secret);
        // An escaped secret cannot be borrowed and would need an owned copy.
        let escaped = serde_json::to_string(&valid()).unwrap().replacen(
            "\"secret\":\"abab",
            "\"secret\":\"\\u0061bab",
            1,
        );
        let escaped = format!(
            "Bearer {}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(escaped)
        );
        assert!(parse_authorization(&headers(&[&escaped])).is_none());
    }

    #[test]
    fn signal_budget_allows_the_burst_then_refills_at_the_rate_up_to_the_burst() {
        let start = Instant::now();
        let mut budget = SignalBudget::new(
            SignalRate {
                per_second: 1,
                burst: 2,
            },
            start,
        );
        assert!(budget.take(start));
        assert!(budget.take(start));
        assert!(!budget.take(start));
        let later = start + Duration::from_secs(1);
        assert!(budget.take(later));
        assert!(!budget.take(later));
        // A long idle period refills only up to the burst.
        let idle = later + Duration::from_secs(60);
        assert!(budget.take(idle));
        assert!(budget.take(idle));
        assert!(!budget.take(idle));
    }
}
