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

use super::credential::LoadedHostCredential;
use crate::mobile_connection::{AuthorizationError, MobileAuthorization};
use base64::Engine as _;
use futures_util::{SinkExt as _, StreamExt as _};
use rustls::pki_types::ServerName;
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, RwLock, RwLockWriteGuard},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{net::TcpStream, sync::watch};
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
    },
    Renewed {
        generation: Uuid,
        nonce: Uuid,
        ttl_secs: u64,
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
pub async fn run_registration(
    authority: Arc<RwLock<MobileAuthorization>>,
    credential: &LoadedHostCredential,
    tls: Arc<rustls::ClientConfig>,
    tcp: TcpStream,
    stop: watch::Receiver<bool>,
) -> Result<(), RegistrationClientError> {
    let result = register_and_renew(&authority, credential, tls, tcp, stop).await;
    authority
        .write()
        .unwrap_or_else(|error| error.into_inner())
        .disable();
    result
}

async fn register_and_renew(
    authority: &RwLock<MobileAuthorization>,
    credential: &LoadedHostCredential,
    tls: Arc<rustls::ClientConfig>,
    tcp: TcpStream,
    mut stop: watch::Receiver<bool>,
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
            CoordinatorMessage::Registered { generation, .. } => Ok((socket, generation)),
            CoordinatorMessage::Renewed { .. } => Err(RegistrationClientError::Protocol),
        }
    });
    let (mut socket, generation) = tokio::select! {
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
    send_renew(&mut socket, request.generation(), request.nonce()).await?;
    let mut poll = tokio::time::interval(POLL_INTERVAL);
    loop {
        tokio::select! {
            _ = stopped(&mut stop) => {
                let _ = socket.close(None).await;
                return Ok(());
            }
            _ = poll.tick() => {
                let request = write(authority)?.poll_registration(Instant::now(), wall()?)?;
                if let Some(request) = request {
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
                _ => return Err(RegistrationClientError::Protocol),
            },
        }
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
