//! Host-only registration WebSocket (Issue #6476).
//!
//! A PC Host authenticates with exactly one `Authorization: Bearer` header whose
//! token is base64url (no padding) JSON naming its credential identity, one
//! Project, its trusted Host epoch and the 32-byte secret in hex. A request with
//! any Origin is a browser and is refused; browser signaling is a separate,
//! later route. The registration exists only while its socket is served: every
//! exit path, including a failed upgrade, logically closes it once.
//!
//! The coordinator answers `{"type":"registered","generation","ttl_secs"}` and
//! echoes each `{"type":"renew","generation","nonce"}` as `renewed` with the
//! remaining lease seconds. The Host's request-start/nonce lease remains its own
//! contract; this route never extends a lease beyond the domain deadline and
//! closes the socket when that deadline passes, or when the server stops.
//! Invites, browser signaling and device authority are out of scope.
//!
//! Each call reads the real wall clock; a backwards step makes the domain gate
//! terminal (fail closed) until the operator restarts the coordinator.

use super::{
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
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

pub(super) const HOST_REGISTER_PATH: &str = "/host/v1/register";
/// The same bound as HTTP request bodies (the mobile policy's maximum SDP).
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_TOKEN_CHARS: usize = 1024;
const UNAUTHORIZED: &str =
    "coordinator host registration rejected; import a valid Host credential and retry";

/// Shared by every connection service so the private store guard lives until
/// the last accepted connection closes.
pub(super) type SharedSessions = Arc<Mutex<CoordinatorSessions>>;

/// Route state. `stopping` ends every Host socket when the server stops; each
/// socket holds an `alive` sender until its registration is logically closed,
/// so the server can wait for all of them before releasing the registry.
#[derive(Clone)]
pub(super) struct HostState {
    pub(super) sessions: SharedSessions,
    pub(super) stopping: watch::Receiver<bool>,
    pub(super) alive: mpsc::Sender<()>,
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
    secret: String,
}

struct HostCredentials {
    identity: CredentialIdentity,
    project: ProjectKey,
    host_epoch: Uuid,
    secret: [u8; 32],
}

fn parse_authorization(headers: &HeaderMap) -> Option<HostCredentials> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let token = value.to_str().ok()?.strip_prefix("Bearer ")?;
    if token.len() > MAX_TOKEN_CHARS {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token)
        .ok()?;
    let token: HostToken = serde_json::from_slice(&bytes).ok()?;
    if token.schema_version != 1 {
        return None;
    }
    let mut secret = [0u8; 32];
    hex::decode_to_slice(&token.secret, &mut secret).ok()?;
    Some(HostCredentials {
        identity: CredentialIdentity {
            host_id: token.host_id,
            credential_id: token.credential_id,
            generation: token.generation,
        },
        project: ProjectKey::parse(&token.project).ok()?,
        host_epoch: token.host_epoch,
        secret,
    })
}

/// Synchronous domain call; the store checkpoint fsyncs at most once per
/// trusted wall second, so async callers run it on a blocking thread. The
/// monotonic instant is read under the lock.
fn call<T>(
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
}

/// Fallback for paths that cannot await (a failed upgrade); normal socket exits
/// drop the registration on a blocking thread.
impl Drop for Registration {
    fn drop(&mut self) {
        let _ = call(&self.sessions, |sessions, now, wall| {
            sessions.close(&self.handle, now, wall)
        });
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum HostMessage {
    Renew { generation: Uuid, nonce: Uuid },
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
    let sessions = Arc::clone(&state.sessions);
    let registered = tokio::task::spawn_blocking(move || {
        call(&sessions, |sessions, now, wall| {
            let handle = sessions.register(
                &credentials.identity,
                &credentials.project,
                &credentials.secret,
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
    let registration = Registration {
        sessions: state.sessions,
        handle,
    };
    let (stopping, alive) = (state.stopping, state.alive);
    upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| {
            serve_host(socket, registration, generation, deadline, stopping, alive)
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

async fn serve_host(
    mut socket: WebSocket,
    mut registration: Registration,
    generation: Uuid,
    mut deadline: Instant,
    mut stopping: watch::Receiver<bool>,
    alive: mpsc::Sender<()>,
) {
    let registered = serde_json::json!({
        "type": "registered", "generation": generation, "ttl_secs": ttl_secs(deadline),
    });
    if send_json(&mut socket, registered).await {
        loop {
            let message = tokio::select! {
                message = socket.recv() => message,
                _ = tokio::time::sleep_until(deadline.into()) => break,
                _ = stopping.wait_for(|stopped| *stopped) => break,
            };
            let text = match message {
                Some(Ok(Message::Text(text))) => text,
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => break,
            };
            let Ok(HostMessage::Renew {
                generation: renewed,
                nonce,
            }) = serde_json::from_str(text.as_str())
            else {
                break;
            };
            if renewed != generation {
                break;
            }
            let renewal = tokio::task::spawn_blocking(move || {
                let result = registration.renew();
                (registration, result)
            })
            .await;
            // A panicked renewal already dropped (closed) the registration.
            let Ok((returned, result)) = renewal else {
                let _ = socket.send(Message::Close(None)).await;
                return;
            };
            registration = returned;
            let Ok(next) = result else {
                break;
            };
            deadline = next;
            let renewed = serde_json::json!({
                "type": "renewed", "generation": generation, "nonce": nonce,
                "ttl_secs": ttl_secs(deadline),
            });
            if !send_json(&mut socket, renewed).await {
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
        assert_eq!(parsed.secret, [0xab; 32]);
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
}
