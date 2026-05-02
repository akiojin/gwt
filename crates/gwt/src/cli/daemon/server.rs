//! Tokio-based Unix-socket IPC server for the runtime daemon (SPEC-2077
//! Phase 1 runtime layer).
//!
//! Foreground entry: caller blocks inside [`serve_blocking`] until the
//! daemon receives `SIGINT` / `SIGTERM`, at which point the listener is
//! dropped, the socket file is removed, and the persisted endpoint file
//! is unlinked. Per-connection workers handle:
//!
//! 1. Read one newline-delimited [`IpcHandshakeRequest`] JSON line.
//! 2. Validate against the in-memory endpoint with
//!    [`validate_handshake`].
//! 3. Write the matching [`IpcHandshakeResponse`] line.
//! 4. While the connection stays open, accept newline-delimited JSON
//!    payloads (today: log + ack; later phases route hook envelopes).
//!
//! Hook envelope routing is intentionally out of scope for this PR — the
//! purpose here is to stand up the daemon process and make `gwt -> gwtd`
//! IPC end-to-end provable. Phase H1〜H4 will graft handler logic onto
//! the per-connection loop.

#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use gwt_core::daemon::{
    persist_endpoint, validate_handshake, DaemonEndpoint, IpcHandshakeRequest,
    IpcHandshakeResponse, RuntimeScope, DAEMON_PROTOCOL_VERSION,
};
use gwt_github::{client::ApiError, SpecOpsError};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    runtime::Builder,
    signal::unix::{signal, SignalKind},
    sync::Notify,
};

const ACCEPT_BACKOFF_MS: u64 = 50;

pub(super) fn serve_blocking(
    scope: RuntimeScope,
    endpoint_path: PathBuf,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let socket_path = derive_socket_path(&endpoint_path);
    if let Err(err) = ensure_socket_parent(&socket_path) {
        return Err(config_error(format!(
            "failed to prepare daemon socket directory: {err}"
        )));
    }
    cleanup_stale_socket(&socket_path);

    let auth_token = uuid::Uuid::new_v4().to_string();
    let endpoint = DaemonEndpoint::new(
        scope,
        std::process::id(),
        socket_path.to_string_lossy().to_string(),
        auth_token,
        env!("CARGO_PKG_VERSION").to_string(),
    );

    persist_endpoint(&endpoint_path, &endpoint)
        .map_err(|err| config_error(format!("failed to persist daemon endpoint: {err}")))?;

    out.push_str(&format!(
        "gwtd daemon start: bind={socket}\n",
        socket = socket_path.display()
    ));
    out.push_str(&format!(
        "gwtd daemon start: pid={pid} version={version}\n",
        pid = endpoint.pid,
        version = endpoint.daemon_version
    ));

    let runtime = Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .map_err(|err| config_error(format!("tokio runtime build failed: {err}")))?;

    let result = runtime.block_on(run_server(
        endpoint,
        socket_path.clone(),
        endpoint_path.clone(),
    ));

    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&endpoint_path);

    result
}

pub(super) async fn run_server(
    endpoint: DaemonEndpoint,
    socket_path: PathBuf,
    endpoint_path: PathBuf,
) -> Result<i32, SpecOpsError> {
    let listener = UnixListener::bind(&socket_path).map_err(|err| {
        config_error(format!(
            "failed to bind daemon socket {}: {err}",
            socket_path.display()
        ))
    })?;

    let shutdown = Arc::new(Notify::new());
    spawn_signal_watcher(Arc::clone(&shutdown));

    let endpoint = Arc::new(endpoint);
    let _endpoint_path = endpoint_path; // retained for symmetry with future watch flows
    loop {
        tokio::select! {
            biased;
            _ = shutdown.notified() => {
                tracing::info!("gwtd daemon: shutdown signal received");
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _addr)) => {
                        let endpoint = Arc::clone(&endpoint);
                        tokio::spawn(async move {
                            if let Err(err) = handle_connection(stream, endpoint).await {
                                tracing::warn!("gwtd daemon: connection error: {err}");
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!("gwtd daemon: accept failed: {err}");
                        tokio::time::sleep(Duration::from_millis(ACCEPT_BACKOFF_MS)).await;
                    }
                }
            }
        }
    }

    Ok(0)
}

fn spawn_signal_watcher(shutdown: Arc<Notify>) {
    let term = shutdown.clone();
    tokio::spawn(async move {
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(sig) => sig,
            Err(err) => {
                tracing::warn!("gwtd daemon: failed to install SIGTERM handler: {err}");
                return;
            }
        };
        let mut sigint = match signal(SignalKind::interrupt()) {
            Ok(sig) => sig,
            Err(err) => {
                tracing::warn!("gwtd daemon: failed to install SIGINT handler: {err}");
                return;
            }
        };
        tokio::select! {
            _ = sigterm.recv() => {}
            _ = sigint.recv() => {}
        }
        term.notify_waiters();
    });
}

async fn handle_connection(
    stream: UnixStream,
    endpoint: Arc<DaemonEndpoint>,
) -> Result<(), String> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let request = read_handshake(&mut reader).await?;
    let response = build_handshake_response(&endpoint, &request);
    write_json_line(&mut write_half, &response).await?;

    let validation = validate_handshake(&endpoint, &request, &response);
    if validation.is_err() {
        return Ok(()); // we already told the client; drop the connection.
    }

    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|err| format!("read frame failed: {err}"))?;
        if n == 0 {
            break; // peer closed
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        // Phase 1: surface the frame to logs; Phase H1 will route into
        // hook handlers / runtime-state aggregator.
        tracing::debug!(target: "gwtd::daemon", frame = %trimmed, "received frame");
        write_half
            .write_all(b"{\"ack\":true}\n")
            .await
            .map_err(|err| format!("ack write failed: {err}"))?;
    }

    Ok(())
}

async fn read_handshake(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
) -> Result<IpcHandshakeRequest, String> {
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .await
        .map_err(|err| format!("handshake read failed: {err}"))?;
    if n == 0 {
        return Err("client closed before handshake".to_string());
    }
    serde_json::from_str(line.trim_end()).map_err(|err| format!("handshake parse failed: {err}"))
}

fn build_handshake_response(
    endpoint: &DaemonEndpoint,
    request: &IpcHandshakeRequest,
) -> IpcHandshakeResponse {
    let mut response = IpcHandshakeResponse {
        protocol_version: DAEMON_PROTOCOL_VERSION,
        daemon_version: endpoint.daemon_version.clone(),
        accepted: true,
        rejection_reason: None,
    };
    if request.protocol_version != endpoint.protocol_version {
        response.accepted = false;
        response.rejection_reason = Some("protocol version mismatch".to_string());
        return response;
    }
    if request.auth_token != endpoint.auth_token {
        response.accepted = false;
        response.rejection_reason = Some("auth token mismatch".to_string());
        return response;
    }
    if request.scope != endpoint.scope {
        response.accepted = false;
        response.rejection_reason = Some("scope mismatch".to_string());
        return response;
    }
    response
}

async fn write_json_line<T, W>(writer: &mut W, value: &T) -> Result<(), String>
where
    T: serde::Serialize,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut payload =
        serde_json::to_vec(value).map_err(|err| format!("serialize failed: {err}"))?;
    payload.push(b'\n');
    writer
        .write_all(&payload)
        .await
        .map_err(|err| format!("write failed: {err}"))?;
    Ok(())
}

fn derive_socket_path(endpoint_path: &Path) -> PathBuf {
    endpoint_path.with_extension("sock")
}

fn ensure_socket_parent(socket_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn cleanup_stale_socket(socket_path: &Path) {
    if socket_path.exists() {
        let _ = fs::remove_file(socket_path);
    }
}

fn config_error(message: impl Into<String>) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(message.into()))
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use gwt_core::daemon::{
        DaemonEndpoint, IpcHandshakeRequest, RuntimeScope, RuntimeTarget, DAEMON_PROTOCOL_VERSION,
    };
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::UnixStream,
    };

    use super::{build_handshake_response, run_server};

    fn sample_endpoint(scope: RuntimeScope, socket_path: &Path, token: &str) -> DaemonEndpoint {
        DaemonEndpoint::new(
            scope,
            std::process::id(),
            socket_path.to_string_lossy().to_string(),
            token.to_string(),
            "test-daemon".to_string(),
        )
    }

    fn sample_scope(temp: &TempDir) -> RuntimeScope {
        RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            temp.path().to_path_buf(),
            RuntimeTarget::Host,
        )
        .expect("scope")
    }

    #[test]
    fn build_handshake_response_rejects_protocol_version_mismatch() {
        let temp = TempDir::new().unwrap();
        let scope = sample_scope(&temp);
        let endpoint = sample_endpoint(scope.clone(), &temp.path().join("daemon.sock"), "tok");
        let request = IpcHandshakeRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION + 99,
            auth_token: "tok".to_string(),
            scope,
        };
        let response = super::build_handshake_response(&endpoint, &request);
        assert!(!response.accepted);
        assert_eq!(
            response.rejection_reason.as_deref(),
            Some("protocol version mismatch")
        );
    }

    #[test]
    fn build_handshake_response_rejects_auth_token_mismatch() {
        let temp = TempDir::new().unwrap();
        let scope = sample_scope(&temp);
        let endpoint = sample_endpoint(scope.clone(), &temp.path().join("daemon.sock"), "tok");
        let request = IpcHandshakeRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            auth_token: "wrong".to_string(),
            scope,
        };
        let response = build_handshake_response(&endpoint, &request);
        assert!(!response.accepted);
        assert_eq!(
            response.rejection_reason.as_deref(),
            Some("auth token mismatch")
        );
    }

    #[test]
    fn build_handshake_response_accepts_matching_request() {
        let temp = TempDir::new().unwrap();
        let scope = sample_scope(&temp);
        let endpoint = sample_endpoint(scope.clone(), &temp.path().join("daemon.sock"), "tok");
        let request = IpcHandshakeRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            auth_token: "tok".to_string(),
            scope,
        };
        let response = build_handshake_response(&endpoint, &request);
        assert!(response.accepted);
        assert!(response.rejection_reason.is_none());
    }

    #[tokio::test]
    async fn run_server_accepts_handshake_and_acknowledges_frames() {
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let endpoint = sample_endpoint(scope.clone(), &socket_path, "secret");

        // Pre-create the socket file by binding inside run_server. We need
        // run_server to bind, then a client connects, exchanges handshake,
        // and sends one frame.
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let server_handle =
            tokio::spawn(
                async move { run_server(endpoint, server_socket, server_endpoint_path).await },
            );

        // wait until the socket is bound
        let mut attempts = 0;
        while !socket_path.exists() && attempts < 50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            attempts += 1;
        }
        assert!(socket_path.exists(), "socket bound");

        let stream = UnixStream::connect(&socket_path).await.expect("connect");
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);

        let request = IpcHandshakeRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            auth_token: "secret".to_string(),
            scope,
        };
        let payload = serde_json::to_vec(&request).expect("serialize");
        write_half.write_all(&payload).await.expect("write request");
        write_half.write_all(b"\n").await.expect("write newline");

        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .await
            .expect("read response");
        assert!(response_line.contains("\"accepted\":true"));

        // Send a sample frame and expect the ack response.
        write_half
            .write_all(b"{\"hook\":\"runtime-state\"}\n")
            .await
            .expect("write frame");
        let mut ack = String::new();
        reader.read_line(&mut ack).await.expect("read ack");
        assert!(ack.contains("\"ack\":true"));

        // Closing the client should let the per-connection task finish.
        drop(write_half);
        drop(reader);

        // Cancel the server (simulating SIGINT) by aborting.
        server_handle.abort();
        let _ = server_handle.await;
    }
}
