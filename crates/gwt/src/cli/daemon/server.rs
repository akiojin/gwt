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
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use gwt_core::daemon::{
    persist_endpoint, validate_handshake, ClientFrame, DaemonEndpoint, DaemonFrame, DaemonStatus,
    IpcHandshakeRequest, IpcHandshakeResponse, RuntimeScope, DAEMON_PROTOCOL_VERSION,
};
use gwt_github::{client::ApiError, SpecOpsError};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    runtime::Builder,
    signal::unix::{signal, SignalKind},
    sync::{mpsc, Notify},
};

use super::broadcast::BroadcastHub;

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

    let hub = BroadcastHub::new();
    let result = runtime.block_on(run_server(
        endpoint,
        socket_path.clone(),
        endpoint_path.clone(),
        hub,
    ));

    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&endpoint_path);

    result
}

pub(crate) async fn run_server(
    endpoint: DaemonEndpoint,
    socket_path: PathBuf,
    endpoint_path: PathBuf,
    hub: BroadcastHub,
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
    let started_at = Instant::now();
    let connections = Arc::new(AtomicUsize::new(0));
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
                        let hub = hub.clone();
                        let connections = Arc::clone(&connections);
                        tokio::spawn(async move {
                            let guard = ConnectionGuard::new(connections);
                            if let Err(err) =
                                handle_connection(stream, endpoint, hub, started_at, &guard).await
                            {
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

/// RAII counter for live IPC connections. The constructor increments
/// the shared counter; `Drop` decrements it. This guarantees the
/// counter stays accurate even on panic or abnormal task abort.
struct ConnectionGuard {
    counter: Arc<AtomicUsize>,
}

impl ConnectionGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self { counter }
    }

    fn snapshot(&self) -> usize {
        self.counter.load(Ordering::SeqCst)
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
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
    hub: BroadcastHub,
    started_at: Instant,
    connection_guard: &ConnectionGuard,
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

    // After handshake, all writes flow through `out_tx` so the reader loop
    // and any broadcast forwarders can send concurrently without sharing
    // `write_half` directly.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<DaemonFrame>();
    let writer = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            if let Err(err) = write_json_line(&mut write_half, &frame).await {
                tracing::warn!(target: "gwtd::daemon", error = %err, "writer task failed");
                break;
            }
        }
    });

    let mut line = String::new();
    loop {
        line.clear();
        let n = match reader.read_line(&mut line).await {
            Ok(n) => n,
            Err(err) => {
                tracing::warn!(target: "gwtd::daemon", error = %err, "read frame failed");
                break;
            }
        };
        if n == 0 {
            break; // peer closed
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<ClientFrame>(trimmed) {
            Ok(ClientFrame::Hook(envelope)) => {
                // Phase H1〜H4 will route hook envelopes into real handlers;
                // for now we just ack so the client side knows the daemon
                // received the frame.
                tracing::debug!(
                    target: "gwtd::daemon",
                    hook = %envelope.hook_name,
                    "received hook envelope"
                );
                if out_tx.send(DaemonFrame::Ack).is_err() {
                    break;
                }
            }
            Ok(ClientFrame::Subscribe { channels }) => {
                for channel in channels {
                    let mut rx = hub.subscribe(&channel);
                    let out_tx = out_tx.clone();
                    let channel_for_log = channel.clone();
                    tokio::spawn(async move {
                        loop {
                            match rx.recv().await {
                                Ok(frame) => {
                                    if out_tx.send(frame).is_err() {
                                        break;
                                    }
                                }
                                Err(err) => {
                                    tracing::debug!(
                                        target: "gwtd::daemon",
                                        channel = %channel_for_log,
                                        error = %err,
                                        "broadcast receiver closed"
                                    );
                                    break;
                                }
                            }
                        }
                    });
                }
                if out_tx.send(DaemonFrame::Ack).is_err() {
                    break;
                }
            }
            Ok(ClientFrame::Status) => {
                let snapshot = DaemonStatus {
                    protocol_version: endpoint.protocol_version,
                    daemon_version: endpoint.daemon_version.clone(),
                    uptime_seconds: started_at.elapsed().as_secs(),
                    broadcast_channels: hub.channel_count(),
                    connections: connection_guard.snapshot(),
                };
                if out_tx.send(DaemonFrame::Status(snapshot)).is_err() {
                    break;
                }
            }
            Ok(ClientFrame::Publish { channel, payload }) => {
                // Enqueue the Ack into our `out_tx` *before* the
                // broadcast fan-out so a client that is both
                // subscribed and publishing on the same connection
                // never observes its own broadcast Event arrive
                // before the Ack for the Publish that triggered it.
                // Without this ordering the spawned per-channel
                // forwarder task can race the Publish reader and
                // push `DaemonFrame::Event` into `out_tx` first,
                // desynchronizing any caller doing a simple
                // `send_frame(Publish) -> read_frame::<Ack>` flow.
                if out_tx.send(DaemonFrame::Ack).is_err() {
                    break;
                }
                let queued = hub.publish(
                    &channel,
                    DaemonFrame::Event {
                        channel: channel.clone(),
                        payload,
                    },
                );
                tracing::debug!(
                    target: "gwtd::daemon",
                    %channel,
                    queued,
                    "publish frame fanned out"
                );
            }
            Err(err) => {
                tracing::warn!(target: "gwtd::daemon", frame = %trimmed, error = %err, "rejected unrecognized frame");
                if out_tx
                    .send(DaemonFrame::Error {
                        message: format!("frame parse failed: {err}"),
                    })
                    .is_err()
                {
                    break;
                }
            }
        }
    }

    drop(out_tx);
    let _ = writer.await;
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
        ClientFrame, DaemonEndpoint, HookEnvelope, IpcHandshakeRequest, RuntimeScope,
        RuntimeTarget, DAEMON_PROTOCOL_VERSION,
    };
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::UnixStream,
    };

    use super::{build_handshake_response, run_server, BroadcastHub};

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
        let server_hub = BroadcastHub::new();
        let server_handle = tokio::spawn(async move {
            run_server(endpoint, server_socket, server_endpoint_path, server_hub).await
        });

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

        // Send a typed `ClientFrame::Hook` and expect a `DaemonFrame::Ack`.
        let request_scope = sample_scope(&temp);
        let frame = ClientFrame::Hook(HookEnvelope {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            scope: request_scope,
            hook_name: "runtime-state".to_string(),
            session_id: None,
            cwd: temp.path().to_path_buf(),
            payload: serde_json::json!({}),
        });
        let mut frame_bytes = serde_json::to_vec(&frame).expect("serialize frame");
        frame_bytes.push(b'\n');
        write_half
            .write_all(&frame_bytes)
            .await
            .expect("write frame");
        let mut ack = String::new();
        reader.read_line(&mut ack).await.expect("read ack");
        assert!(
            ack.contains("\"type\":\"ack\""),
            "expected typed ack frame, got: {ack}"
        );

        // Send a malformed line and expect a typed Error frame back.
        write_half
            .write_all(b"not-json\n")
            .await
            .expect("write malformed frame");
        let mut error_line = String::new();
        reader
            .read_line(&mut error_line)
            .await
            .expect("read error frame");
        assert!(
            error_line.contains("\"type\":\"error\""),
            "expected typed error frame, got: {error_line}"
        );

        // Closing the client should let the per-connection task finish.
        drop(write_half);
        drop(reader);

        // Cancel the server (simulating SIGINT) by aborting.
        server_handle.abort();
        let _ = server_handle.await;
    }
}
