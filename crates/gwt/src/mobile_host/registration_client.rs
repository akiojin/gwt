//! Host side of the coordinator registration WebSocket (Issue #6476).
//!
//! One Host actor drives one registration: it selects registered mode on the
//! PC authority (fresh Host epoch), authenticates the coordinator through TLS
//! with the caller's trust roots and the credential's fixed origin, sends the
//! credential only as the `Authorization` bearer of `/host/v1/register`, then
//! starts the lease from the coordinator's generation and keeps renewing it.
//! Every exit — explicit stop, EOF, rejection or protocol/lease failure —
//! disables the authority (new epoch); a later registration needs an explicit
//! PC enable again. Name resolution, root-store selection and the PC control
//! wiring belong to the caller, which hands over a connected TCP stream.
//!
//! While enabled, the caller may register pairing invites over the same socket
//! one at a time: each [`InviteRequest`] becomes a `register_invite` for the
//! current generation and is answered through its reply channel. A request the
//! registration ends before answering loses its reply (never an inferred
//! success). The hex secret is built in a wiped buffer; the copy handed to the
//! WebSocket frame cannot be wiped (as for the bearer). Dropping every invite
//! sender only ends invite input, not the registration.
//!
//! [`register_invite`] issues an invite on the PC authority's ledger and
//! registers it. The coordinator announces browser connections that claimed
//! this registration's invites (naming the invite by
//! [`InviteRequest::invite_id`]); the client binds each claim to the ledger and
//! reports it, or closes a connection whose invite this Host did not issue (or
//! that expired or was already claimed) without reporting the claim. A closed
//! connection's unapproved pairing is dropped before its close is reported.
//! Connections may also arrive
//! that reconnect an approved device (naming the unverified device ID): the
//! client binds each to that device on the ledger, so only it may be
//! challenged there, and reports it, or closes a connection naming a device
//! this Host does not hold approved and unexpired. With the signals each
//! connection relays and its close, these become [`ConnectionEvent`]s. Each
//! signal must follow the Host wire ([`super::signal`]) on its claimed or
//! reconnected connection, where only a claimed connection may pair and only
//! a reconnected one may prove its device key: a signal on any other
//! connection, or the first
//! that breaks the wire, closes that connection unreported, and later signals
//! of a rejected connection are dropped.
//! A caller that stopped receiving is ignored, but a full event channel ends
//! the registration (`EventBacklog`) rather than lose a connection's state.
//! Signals may use only half the channel: a signal beyond that is dropped and
//! its connection closed through the coordinator (`close_connection`), so one
//! browser cannot end the registration for the others.
//!
//! The caller answers a claimed or reconnected connection with typed
//! [`OutboundSignal`]s, sent one at a time while enabled on the Host wire of
//! that connection ([`super::signal`]). A signal for a connection without wire
//! state (never admitted, or closed) or already rejected is dropped; the first
//! that breaks the wire closes its connection. A signal whose frame exceeds
//! the coordinator's message bound would end the whole registration there, so
//! it is dropped and only its connection closed. Dropping every signal sender
//! only ends signal input.
//!
//! The coordinator ends a registration whose socket exceeds the Host message
//! rate it announces in `registered`. Every message the client sends spends
//! from its own bucket at that rate, holding half the burst (the other half
//! absorbs arrival jitter). Renewals, invites and closes are bounded by the
//! protocol and never wait, so a lease renewal is not delayed; signals wait
//! for a token, which backpressures the caller's bounded channel.

use super::credential::LoadedHostCredential;
use super::signal::{BrowserSignal, HostSignal, SignalError, SignalSequence};
use crate::mobile_connection::{AuthorizationError, IssuedInvite, MobileAuthorization};
use base64::Engine as _;
use futures_util::{SinkExt as _, StreamExt as _};
use rustls::pki_types::ServerName;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock, RwLockWriteGuard},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot, watch},
};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, http::HeaderValue, protocol::WebSocketConfig, Message,
};
use uuid::Uuid;
use zeroize::Zeroizing;

const REGISTER_PATH: &str = "/host/v1/register";
/// The coordinator's message bound.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// Bounds the TLS/WebSocket handshake and the wait for `registered`.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Renewal requests keep their own ten-second cadence; this only bounds how
/// late the lease poll observes it.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum RegistrationClientError {
    #[error(transparent)]
    Authority(#[from] AuthorizationError),
    #[error("the coordinator origin cannot name a TLS server; correct the Host configuration")]
    InvalidOrigin,
    #[error("trusted Unix time is unavailable; repair the clock and explicitly enable again")]
    Clock,
    #[error("the coordinator TLS/WebSocket connection failed; check the network and coordinator certificate, then explicitly enable again")]
    Connect,
    #[error("the coordinator refused the Host registration (HTTP {0}); repair the credential or coordinator, then explicitly enable again")]
    Rejected(u16),
    #[error("the coordinator sent an invalid registration message; explicitly enable again after inspecting the coordinator")]
    Protocol,
    #[error("the coordinator registration connection closed; explicitly enable again")]
    Closed,
    #[error("the Host fell behind on coordinator connection events; explicitly enable again")]
    EventBacklog,
    #[error("the coordinator closed the Host registration for exceeding its message rate; explicitly enable again")]
    RateLimited,
}

/// A browser connection that claimed one of this registration's invites, and
/// the opaque signals it relays to this Host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// `invite` is the claimed invite's [`InviteRequest::invite_id`].
    Claimed {
        connection_id: Uuid,
        invite: [u8; 32],
    },
    /// An approved device's reconnect, by the device ID its browser named and
    /// the ledger bound to the connection. Not proof of the device: the caller
    /// must verify its key possession (`challenge_peer_bound`).
    Reconnect {
        connection_id: Uuid,
        device_id: Uuid,
    },
    /// A signal that followed the Host wire on this connection.
    Signal {
        connection_id: Uuid,
        signal: BrowserSignal,
    },
    Closed {
        connection_id: Uuid,
    },
}

/// A signal for one claimed or reconnected connection of this registration.
pub struct OutboundSignal {
    pub connection_id: Uuid,
    pub signal: HostSignal,
}

/// A pairing invite for the coordinator to admit under this registration.
pub struct InviteRequest {
    pub secret: Zeroizing<[u8; 32]>,
    pub ttl_secs: u64,
    pub reply: oneshot::Sender<InviteOutcome>,
}

impl InviteRequest {
    /// The SHA-256 digest of the secret, by which a later
    /// [`ConnectionEvent::Claimed`] names this invite. Not a secret.
    pub fn invite_id(&self) -> [u8; 32] {
        Sha256::digest(self.secret.as_slice()).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteOutcome {
    Registered,
    Rejected(InviteRejection),
}

/// Rejections the Host recovers from by generating another invite (or waiting
/// for a pending one to expire); the registration stays live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteRejection {
    InvalidTtl,
    AlreadyRegistered,
    HostCapacity,
    Capacity,
}

#[derive(Debug, thiserror::Error)]
pub enum InviteRegistrationError {
    #[error(transparent)]
    Authority(#[from] AuthorizationError),
    #[error("trusted Unix time is unavailable; repair the clock before issuing an invite")]
    Clock,
    #[error("the coordinator rejected the invite ({0:?}); issue another invite")]
    Rejected(InviteRejection),
    #[error("the coordinator registration ended before the invite was registered")]
    Closed,
}

/// Issue a pairing invite on the enabled PC authority and register it over
/// the live registration behind `invites`. Only an invite issued here can be
/// claimed: the client binds each claim the coordinator reports to the Host's
/// invite ledger and closes any other. A rejected invite stays in the ledger,
/// unclaimable, until it expires.
pub async fn register_invite(
    authority: &RwLock<MobileAuthorization>,
    invites: &mpsc::Sender<InviteRequest>,
) -> Result<IssuedInvite, InviteRegistrationError> {
    let now = wall().map_err(|_| InviteRegistrationError::Clock)?;
    let issued = authority
        .write()
        .map_err(|_| AuthorizationError::Unavailable)?
        .issue_invite(now)?;
    let (reply, outcome) = oneshot::channel();
    let request = InviteRequest {
        secret: issued.secret.clone(),
        ttl_secs: issued.expires_at.saturating_sub(now),
        reply,
    };
    invites
        .send(request)
        .await
        .map_err(|_| InviteRegistrationError::Closed)?;
    match outcome.await {
        Ok(InviteOutcome::Registered) => Ok(issued),
        Ok(InviteOutcome::Rejected(reason)) => Err(InviteRegistrationError::Rejected(reason)),
        Err(_) => Err(InviteRegistrationError::Closed),
    }
}

#[derive(Serialize)]
struct Token<'a> {
    schema_version: u32,
    host_id: Uuid,
    credential_id: Uuid,
    generation: Uuid,
    project: &'a str,
    host_epoch: Uuid,
    secret: &'a str,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum CoordinatorMessage {
    Registered {
        generation: Uuid,
        #[allow(dead_code)]
        ttl_secs: u64,
        messages_per_second: u16,
        burst: u16,
    },
    Renewed {
        generation: Uuid,
        nonce: Uuid,
        ttl_secs: u64,
    },
    InviteRegistered {
        generation: Uuid,
        nonce: Uuid,
    },
    InviteRejected {
        generation: Uuid,
        nonce: Uuid,
        reason: InviteRejection,
    },
    ConnectionClaimed {
        generation: Uuid,
        connection_id: Uuid,
        /// 64 hex; anything else is a protocol violation.
        invite: String,
    },
    ConnectionReconnect {
        generation: Uuid,
        connection_id: Uuid,
        device_id: Uuid,
    },
    ConnectionSignal {
        generation: Uuid,
        connection_id: Uuid,
        payload: String,
    },
    ConnectionClosed {
        generation: Uuid,
        connection_id: Uuid,
    },
    RateLimited {
        generation: Uuid,
    },
}

/// `Bearer <base64url JSON>` built in pre-sized, wiped buffers. The copy the
/// HTTP request keeps cannot be wiped (as on the coordinator side).
fn bearer(credential: &LoadedHostCredential, host_epoch: Uuid) -> Zeroizing<String> {
    let mut secret = Zeroizing::new(String::with_capacity(64));
    for byte in credential.secret_bytes() {
        let _ = std::fmt::Write::write_fmt(&mut *secret, format_args!("{byte:02x}"));
    }
    let identity = credential.identity();
    let token = Token {
        schema_version: 1,
        host_id: identity.host_id,
        credential_id: identity.credential_id,
        generation: identity.generation,
        project: credential.project().as_str(),
        host_epoch,
        secret: &secret,
    };
    let mut json = Zeroizing::new(Vec::with_capacity(512));
    // Writing into a Vec cannot fail; the capacity covers the fixed fields.
    let _ = serde_json::to_writer(&mut *json, &token);
    let mut value = Zeroizing::new(String::with_capacity(8 + json.len() * 4 / 3 + 4));
    value.push_str("Bearer ");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode_string(&*json, &mut value);
    value
}

fn wall() -> Result<u64, RegistrationClientError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|_| RegistrationClientError::Clock)
}

fn write(
    authority: &RwLock<MobileAuthorization>,
) -> Result<RwLockWriteGuard<'_, MobileAuthorization>, RegistrationClientError> {
    authority
        .write()
        .map_err(|_| AuthorizationError::Unavailable.into())
}

/// Register this Host and keep its lease until `stop` turns true or its
/// sender is dropped (`Ok`, observed during the handshake too) or the
/// registration fails (`Err`). The authority is disabled on return either way.
/// `tcp` must already be connected to the credential's coordinator origin.
/// `invites` is read only while enabled and one request at a time; `signals`
/// only while enabled; claimed connections are reported on `events`.
#[allow(clippy::too_many_arguments)]
pub async fn run_registration(
    authority: Arc<RwLock<MobileAuthorization>>,
    credential: &LoadedHostCredential,
    tls: Arc<rustls::ClientConfig>,
    tcp: TcpStream,
    stop: watch::Receiver<bool>,
    invites: mpsc::Receiver<InviteRequest>,
    signals: mpsc::Receiver<OutboundSignal>,
    events: mpsc::Sender<ConnectionEvent>,
) -> Result<(), RegistrationClientError> {
    let result = register_and_renew(
        &authority, credential, tls, tcp, stop, invites, signals, events,
    )
    .await;
    authority
        .write()
        .unwrap_or_else(|error| error.into_inner())
        .disable();
    result
}

#[allow(clippy::too_many_arguments)]
async fn register_and_renew(
    authority: &RwLock<MobileAuthorization>,
    credential: &LoadedHostCredential,
    tls: Arc<rustls::ClientConfig>,
    tcp: TcpStream,
    mut stop: watch::Receiver<bool>,
    mut invites: mpsc::Receiver<InviteRequest>,
    mut signals: mpsc::Receiver<OutboundSignal>,
    events: mpsc::Sender<ConnectionEvent>,
) -> Result<(), RegistrationClientError> {
    let host_epoch = write(authority)?.prepare_registration(credential)?;
    let origin = reqwest::Url::parse(credential.binding().coordinator_origin())
        .map_err(|_| RegistrationClientError::InvalidOrigin)?;
    let host = origin
        .host_str()
        .ok_or(RegistrationClientError::InvalidOrigin)?;
    let host = host
        .strip_prefix('[')
        .and_then(|ip| ip.strip_suffix(']'))
        .unwrap_or(host);
    let server_name = match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ServerName::from(ip),
        Err(_) => ServerName::try_from(host.to_owned())
            .map_err(|_| RegistrationClientError::InvalidOrigin)?,
    };
    let authority_part = origin
        .as_str()
        .strip_prefix("https://")
        .and_then(|rest| rest.strip_suffix('/'))
        .ok_or(RegistrationClientError::InvalidOrigin)?;
    let mut request = format!("wss://{authority_part}{REGISTER_PATH}")
        .into_client_request()
        .map_err(|_| RegistrationClientError::InvalidOrigin)?;
    let mut value = HeaderValue::from_str(&bearer(credential, host_epoch))
        .map_err(|_| RegistrationClientError::Protocol)?;
    value.set_sensitive(true);
    request.headers_mut().insert("authorization", value);

    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let handshake = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let stream = tokio_rustls::TlsConnector::from(tls)
            .connect(server_name, tcp)
            .await
            .map_err(|_| RegistrationClientError::Connect)?;
        let (mut socket, _) =
            tokio_tungstenite::client_async_with_config(request, stream, Some(config))
                .await
                .map_err(|error| match error {
                    tokio_tungstenite::tungstenite::Error::Http(response) => {
                        RegistrationClientError::Rejected(response.status().as_u16())
                    }
                    _ => RegistrationClientError::Connect,
                })?;
        match next_message(&mut socket).await? {
            CoordinatorMessage::Registered {
                generation,
                messages_per_second,
                burst,
                ..
            } if messages_per_second > 0 && burst > 0 => Ok((
                socket,
                generation,
                SendBudget::new(messages_per_second, burst, Instant::now()),
            )),
            _ => Err(RegistrationClientError::Protocol),
        }
    });
    let (mut socket, generation, mut budget) = tokio::select! {
        _ = stopped(&mut stop) => return Ok(()),
        result = handshake => result.map_err(|_| RegistrationClientError::Connect)??,
    };

    let request = write(authority)?.enable_registered(
        credential,
        host_epoch,
        generation,
        Instant::now(),
        wall()?,
    )?;
    budget.spend(Instant::now());
    send_renew(&mut socket, request.generation(), request.nonce()).await?;
    let mut poll = tokio::time::interval(POLL_INTERVAL);
    let mut invites_open = true;
    let mut signals_open = true;
    // The one unanswered invite: its nonce and the caller's reply.
    let mut pending: Option<(Uuid, oneshot::Sender<InviteOutcome>)> = None;
    // The signal state of each claimed or reconnected connection until its close.
    let mut sequences: HashMap<Uuid, SignalSequence> = HashMap::new();
    loop {
        tokio::select! {
            _ = stopped(&mut stop) => {
                let _ = socket.close(None).await;
                return Ok(());
            }
            request = invites.recv(), if invites_open && pending.is_none() => {
                let Some(request) = request else {
                    invites_open = false;
                    continue;
                };
                let nonce = Uuid::new_v4();
                budget.spend(Instant::now());
                send_invite(&mut socket, generation, nonce, &request).await?;
                pending = Some((nonce, request.reply));
            }
            signal = signals.recv(), if signals_open && budget.ready(Instant::now()) => {
                let Some(signal) = signal else {
                    signals_open = false;
                    continue;
                };
                let Some(sequence) = sequences.get_mut(&signal.connection_id) else {
                    continue;
                };
                match sequence.send(&signal.signal) {
                    Ok(payload) => {
                        budget.spend(Instant::now());
                        if !send_signal(&mut socket, generation, signal.connection_id, payload)
                            .await?
                        {
                            sequence.reject();
                        }
                    }
                    // Already closed: drop what was in flight.
                    Err(SignalError::Rejected) => {}
                    Err(_) => {
                        budget.spend(Instant::now());
                        send_close(&mut socket, generation, signal.connection_id).await?;
                    }
                }
            }
            // Wakes the loop when the next signal token is due.
            _ = tokio::time::sleep_until(budget.next_token().into()), if signals_open
                && !budget.ready(Instant::now()) => {}
            _ = poll.tick() => {
                let request = write(authority)?.poll_registration(Instant::now(), wall()?)?;
                if let Some(request) = request {
                    budget.spend(Instant::now());
                    send_renew(&mut socket, request.generation(), request.nonce()).await?;
                }
            }
            message = next_message(&mut socket) => match message? {
                CoordinatorMessage::Renewed { generation: renewed, nonce, ttl_secs }
                    if renewed == generation =>
                {
                    match write(authority)?
                        .accept_registration(renewed, nonce, ttl_secs, Instant::now(), wall()?)
                    {
                        // A late duplicate cannot disable the current request.
                        Ok(()) | Err(AuthorizationError::Registration(
                            super::LeaseError::StaleRenewal,
                        )) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                CoordinatorMessage::InviteRegistered { generation: scoped, nonce }
                    if scoped == generation =>
                {
                    answer(&mut pending, nonce, InviteOutcome::Registered)?;
                }
                CoordinatorMessage::InviteRejected { generation: scoped, nonce, reason }
                    if scoped == generation =>
                {
                    answer(&mut pending, nonce, InviteOutcome::Rejected(reason))?;
                }
                CoordinatorMessage::ConnectionClaimed { generation: scoped, connection_id, invite }
                    if scoped == generation =>
                {
                    let mut digest = [0; 32];
                    hex::decode_to_slice(&invite, &mut digest)
                        .map_err(|_| RegistrationClientError::Protocol)?;
                    // Only an invite this Host issued binds to the connection.
                    let claimed = write(authority)?.claim_invite(digest, connection_id, wall()?);
                    match claimed {
                        Ok(()) => {
                            // Only a claimed invite's connection may pair.
                            sequences.insert(connection_id, SignalSequence::claimed());
                            report(
                                &events,
                                ConnectionEvent::Claimed { connection_id, invite: digest },
                            )?
                        }
                        Err(AuthorizationError::UnknownRequest
                            | AuthorizationError::Expired
                            | AuthorizationError::WrongPeer) => {
                            budget.spend(Instant::now());
                            send_close(&mut socket, generation, connection_id).await?;
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                CoordinatorMessage::ConnectionReconnect { generation: scoped, connection_id, device_id }
                    if scoped == generation =>
                {
                    // Only a device this Host approved binds to the connection.
                    // A reconnect may arrive before the first renewal enables
                    // the Host (`Disabled`): it closes only that connection.
                    let bound = write(authority)?.claim_reconnect(
                        &device_id.to_string(),
                        connection_id,
                        wall()?,
                    );
                    match bound {
                        Ok(()) => {
                            // Only a reconnected device's connection proves its key.
                            sequences.insert(connection_id, SignalSequence::reconnected());
                            report(
                                &events,
                                ConnectionEvent::Reconnect { connection_id, device_id },
                            )?
                        }
                        Err(AuthorizationError::Disabled
                            | AuthorizationError::UnknownDevice
                            | AuthorizationError::Revoked
                            | AuthorizationError::Expired
                            | AuthorizationError::WrongPeer
                            | AuthorizationError::Capacity) => {
                            budget.spend(Instant::now());
                            send_close(&mut socket, generation, connection_id).await?;
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                CoordinatorMessage::ConnectionSignal { generation: scoped, connection_id, payload }
                    if scoped == generation =>
                {
                    let accepted = match sequences.get_mut(&connection_id) {
                        Some(sequence) => {
                            let accepted = sequence.accept(&payload);
                            // Half the channel stays free for claimed/closed
                            // events: a browser outpacing the caller loses only
                            // its own connection.
                            if accepted.is_ok() && events.capacity() <= events.max_capacity() / 2 {
                                sequence.reject();
                                Err(SignalError::Invalid)
                            } else {
                                accepted
                            }
                        }
                        None => Err(SignalError::Invalid),
                    };
                    match accepted {
                        Ok(signal) => {
                            report(&events, ConnectionEvent::Signal { connection_id, signal })?
                        }
                        // Already closed: drop what was in flight.
                        Err(SignalError::Rejected) => {}
                        Err(_) => {
                            budget.spend(Instant::now());
                            send_close(&mut socket, generation, connection_id).await?;
                        }
                    }
                }
                CoordinatorMessage::ConnectionClosed { generation: scoped, connection_id }
                    if scoped == generation =>
                {
                    write(authority)?.close_pairing_connection(connection_id);
                    sequences.remove(&connection_id);
                    report(&events, ConnectionEvent::Closed { connection_id })?;
                }
                CoordinatorMessage::RateLimited { generation: scoped } if scoped == generation => {
                    return Err(RegistrationClientError::RateLimited);
                }
                _ => return Err(RegistrationClientError::Protocol),
            },
        }
    }
}

/// The client's own token bucket for the messages it sends: refilled at the
/// announced rate and holding half the announced burst (at least one). Its
/// balance may go negative after messages that never wait.
struct SendBudget {
    per_second: f64,
    capacity: f64,
    tokens: f64,
    refilled: Instant,
}

impl SendBudget {
    fn new(per_second: u16, burst: u16, now: Instant) -> Self {
        let capacity = f64::from((burst / 2).max(1));
        Self {
            per_second: f64::from(per_second),
            capacity,
            tokens: capacity,
            refilled: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.refilled).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_second).min(self.capacity);
        self.refilled = now;
    }

    /// Whether a signal may be sent at `now`.
    fn ready(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= 1.0
    }

    fn spend(&mut self, now: Instant) {
        self.refill(now);
        self.tokens -= 1.0;
    }

    /// When the balance reaches one token.
    fn next_token(&self) -> Instant {
        let missing = (1.0 - self.tokens).max(0.0);
        self.refilled + Duration::from_secs_f64(missing / self.per_second)
    }
}

/// Deliver the answer for the pending invite; an unrequested or mismatched
/// answer is a protocol violation. A caller that stopped waiting is ignored.
fn answer(
    pending: &mut Option<(Uuid, oneshot::Sender<InviteOutcome>)>,
    nonce: Uuid,
    outcome: InviteOutcome,
) -> Result<(), RegistrationClientError> {
    match pending.take() {
        Some((expected, reply)) if expected == nonce => {
            let _ = reply.send(outcome);
            Ok(())
        }
        _ => Err(RegistrationClientError::Protocol),
    }
}

/// Hand a connection event to the caller without stalling renewals.
fn report(
    events: &mpsc::Sender<ConnectionEvent>,
    event: ConnectionEvent,
) -> Result<(), RegistrationClientError> {
    match events.try_send(event) {
        Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => Ok(()),
        Err(mpsc::error::TrySendError::Full(_)) => Err(RegistrationClientError::EventBacklog),
    }
}

/// A dropped stop sender also stops: the controller is gone, so fail closed.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|stopped| *stopped).await;
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>;

async fn send_renew(
    socket: &mut Socket,
    generation: Uuid,
    nonce: Uuid,
) -> Result<(), RegistrationClientError> {
    let text = serde_json::json!({"type": "renew", "generation": generation, "nonce": nonce});
    socket
        .send(Message::text(text.to_string()))
        .await
        .map_err(|_| RegistrationClientError::Closed)
}

/// Asks the coordinator to close one claimed connection of this registration.
async fn send_close(
    socket: &mut Socket,
    generation: Uuid,
    connection_id: Uuid,
) -> Result<(), RegistrationClientError> {
    let text = serde_json::json!({
        "type": "close_connection", "generation": generation, "connection_id": connection_id,
    });
    socket
        .send(Message::text(text.to_string()))
        .await
        .map_err(|_| RegistrationClientError::Closed)
}

/// Relays a signal to one claimed connection, or closes that connection when
/// the frame would exceed the coordinator's message bound (`false`).
async fn send_signal(
    socket: &mut Socket,
    generation: Uuid,
    connection_id: Uuid,
    payload: String,
) -> Result<bool, RegistrationClientError> {
    let text = serde_json::json!({
        "type": "signal", "generation": generation,
        "connection_id": connection_id, "payload": payload,
    })
    .to_string();
    if text.len() > MAX_MESSAGE_BYTES {
        send_close(socket, generation, connection_id).await?;
        return Ok(false);
    }
    socket
        .send(Message::text(text))
        .await
        .map_err(|_| RegistrationClientError::Closed)?;
    Ok(true)
}

/// `register_invite` JSON assembled in a wiped buffer around the hex secret.
async fn send_invite(
    socket: &mut Socket,
    generation: Uuid,
    nonce: Uuid,
    request: &InviteRequest,
) -> Result<(), RegistrationClientError> {
    let mut text = Zeroizing::new(String::with_capacity(256));
    let _ = std::fmt::Write::write_fmt(
        &mut *text,
        format_args!(
            r#"{{"type":"register_invite","generation":"{generation}","nonce":"{nonce}","secret":""#
        ),
    );
    for byte in request.secret.iter() {
        let _ = std::fmt::Write::write_fmt(&mut *text, format_args!("{byte:02x}"));
    }
    let _ = std::fmt::Write::write_fmt(
        &mut *text,
        format_args!(r#"","ttl_secs":{}}}"#, request.ttl_secs),
    );
    socket
        .send(Message::text(text.as_str()))
        .await
        .map_err(|_| RegistrationClientError::Closed)
}

async fn next_message(socket: &mut Socket) -> Result<CoordinatorMessage, RegistrationClientError> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(text.as_str())
                    .map_err(|_| RegistrationClientError::Protocol)
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(Message::Binary(_) | Message::Frame(_))) => {
                return Err(RegistrationClientError::Protocol)
            }
            Some(Ok(Message::Close(_)) | Err(_)) | None => {
                return Err(RegistrationClientError::Closed)
            }
        }
    }
}
