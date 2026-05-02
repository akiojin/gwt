//! `gwt -> gwtd` IPC client (SPEC-2077 Phase 2 prerequisite).
//!
//! This module is the front-door client used by `gwt` and the in-process
//! hook dispatcher to talk to a running `gwtd` daemon over the local
//! Unix domain socket.
//!
//! Wire format mirrors [`super::server::handle_connection`]:
//!
//! 1. Send one newline-delimited [`IpcHandshakeRequest`] line.
//! 2. Read one newline-delimited [`IpcHandshakeResponse`] line.
//! 3. Validate using [`validate_handshake`]; bail if the daemon
//!    rejected the handshake or reported a protocol mismatch.
//! 4. After handshake, [`DaemonClient::send_frame`] writes a JSON line
//!    and [`DaemonClient::read_ack`] reads the daemon's ack line.
//!
//! Phase H1〜H4 will graft hook-envelope routing onto the post-handshake
//! frame loop. For Phase 2 the client only needs to *prove* end-to-end
//! IPC works so future phases can route real payloads through it.

#![cfg(unix)]

use gwt_core::daemon::{
    validate_handshake, DaemonEndpoint, IpcHandshakeRequest, IpcHandshakeResponse,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{
        unix::{OwnedReadHalf, OwnedWriteHalf},
        UnixStream,
    },
};

/// Connected, post-handshake daemon client.
///
/// `DaemonClient` owns the split read/write halves of the underlying
/// [`UnixStream`]; dropping the value closes the connection.
pub struct DaemonClient {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl DaemonClient {
    /// Connect to the daemon at `endpoint.bind` and complete the
    /// handshake. The returned client is ready for [`Self::send_frame`].
    pub async fn connect(endpoint: &DaemonEndpoint) -> Result<Self, String> {
        let stream = UnixStream::connect(&endpoint.bind)
            .await
            .map_err(|err| format!("daemon connect failed ({}): {err}", endpoint.bind))?;
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);

        let request = IpcHandshakeRequest {
            protocol_version: endpoint.protocol_version,
            auth_token: endpoint.auth_token.clone(),
            scope: endpoint.scope.clone(),
        };
        write_json_line(&mut write_half, &request).await?;

        let response: IpcHandshakeResponse = read_json_line(&mut reader)
            .await?
            .ok_or_else(|| "daemon closed connection during handshake".to_string())?;

        validate_handshake(endpoint, &request, &response)
            .map_err(|err| format!("daemon handshake validation failed: {err}"))?;

        Ok(Self {
            reader,
            writer: write_half,
        })
    }

    /// Send one newline-delimited JSON frame to the daemon.
    pub async fn send_frame<T: serde::Serialize>(&mut self, value: &T) -> Result<(), String> {
        write_json_line(&mut self.writer, value).await
    }

    /// Read the daemon's ack frame as a generic JSON value. Phase H
    /// callers will replace this with typed responses keyed off
    /// [`gwt_core::daemon::HookEnvelope`] result types.
    pub async fn read_ack(&mut self) -> Result<serde_json::Value, String> {
        read_json_line(&mut self.reader)
            .await?
            .ok_or_else(|| "daemon closed connection while awaiting ack".to_string())
    }

    /// Convenience: read a typed JSON frame from the daemon.
    pub async fn read_frame<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, String> {
        read_json_line(&mut self.reader)
            .await?
            .ok_or_else(|| "daemon closed connection while awaiting frame".to_string())
    }
}

async fn write_json_line<T, W>(writer: &mut W, value: &T) -> Result<(), String>
where
    T: serde::Serialize,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut payload =
        serde_json::to_vec(value).map_err(|err| format!("frame serialize failed: {err}"))?;
    payload.push(b'\n');
    writer
        .write_all(&payload)
        .await
        .map_err(|err| format!("frame write failed: {err}"))?;
    Ok(())
}

async fn read_json_line<T>(reader: &mut BufReader<OwnedReadHalf>) -> Result<Option<T>, String>
where
    T: serde::de::DeserializeOwned,
{
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .await
        .map_err(|err| format!("frame read failed: {err}"))?;
    if n == 0 {
        return Ok(None);
    }
    let trimmed = line.trim_end();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value =
        serde_json::from_str(trimmed).map_err(|err| format!("frame parse failed: {err}"))?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gwt_core::daemon::{DaemonEndpoint, RuntimeScope, RuntimeTarget, DAEMON_PROTOCOL_VERSION};
    use tempfile::TempDir;

    use super::DaemonClient;
    use crate::cli::daemon::server;

    fn sample_scope(temp: &TempDir) -> RuntimeScope {
        RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            temp.path().to_path_buf(),
            RuntimeTarget::Host,
        )
        .expect("scope")
    }

    fn sample_endpoint(
        scope: RuntimeScope,
        socket_path: &std::path::Path,
        token: &str,
    ) -> DaemonEndpoint {
        DaemonEndpoint::new(
            scope,
            std::process::id(),
            socket_path.to_string_lossy().to_string(),
            token.to_string(),
            "test-daemon".to_string(),
        )
    }

    async fn wait_for_socket(path: &std::path::Path) {
        for _ in 0..50 {
            if path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("daemon socket never appeared at {}", path.display());
    }

    #[tokio::test]
    async fn client_connects_to_daemon_and_round_trips_a_frame() {
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let endpoint = sample_endpoint(scope.clone(), &socket_path, "client-secret");

        let server_endpoint = endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let server_handle = tokio::spawn(async move {
            server::run_server(server_endpoint, server_socket, server_endpoint_path).await
        });

        wait_for_socket(&socket_path).await;

        let mut client = DaemonClient::connect(&endpoint)
            .await
            .expect("client connects");

        client
            .send_frame(&serde_json::json!({ "hook": "runtime-state" }))
            .await
            .expect("send frame");

        let ack = client.read_ack().await.expect("read ack");
        assert_eq!(ack, serde_json::json!({ "ack": true }));

        drop(client);
        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn client_handshake_fails_when_auth_token_mismatches() {
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let server_endpoint = sample_endpoint(scope.clone(), &socket_path, "expected");

        let server_endpoint_clone = server_endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let server_handle = tokio::spawn(async move {
            server::run_server(server_endpoint_clone, server_socket, server_endpoint_path).await
        });

        wait_for_socket(&socket_path).await;

        // Client uses a different auth_token than the daemon expects.
        let mut bad_endpoint = server_endpoint.clone();
        bad_endpoint.auth_token = "wrong-token".to_string();

        let result = DaemonClient::connect(&bad_endpoint).await;
        assert!(result.is_err(), "client connect should fail");
        let message = result.err().unwrap();
        assert!(
            message.contains("handshake")
                || message.contains("token")
                || message.contains("rejected"),
            "unexpected error message: {message}"
        );

        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn client_handshake_fails_when_protocol_version_mismatches() {
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let server_endpoint = sample_endpoint(scope.clone(), &socket_path, "tok");

        let server_endpoint_clone = server_endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let server_handle = tokio::spawn(async move {
            server::run_server(server_endpoint_clone, server_socket, server_endpoint_path).await
        });

        wait_for_socket(&socket_path).await;

        let mut bad_endpoint = server_endpoint.clone();
        bad_endpoint.protocol_version = DAEMON_PROTOCOL_VERSION + 99;

        let result = DaemonClient::connect(&bad_endpoint).await;
        assert!(result.is_err(), "client connect should fail");

        server_handle.abort();
        let _ = server_handle.await;
    }
}
