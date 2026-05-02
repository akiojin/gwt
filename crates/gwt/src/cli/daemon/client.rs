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

    use gwt_core::daemon::{
        ClientFrame, DaemonEndpoint, DaemonFrame, HookEnvelope, RuntimeScope, RuntimeTarget,
        DAEMON_PROTOCOL_VERSION,
    };
    use tempfile::TempDir;

    use super::DaemonClient;
    use crate::cli::daemon::{broadcast::BroadcastHub, server};

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
        let server_hub = BroadcastHub::new();
        let server_handle = tokio::spawn(async move {
            server::run_server(
                server_endpoint,
                server_socket,
                server_endpoint_path,
                server_hub,
            )
            .await
        });

        wait_for_socket(&socket_path).await;

        let mut client = DaemonClient::connect(&endpoint)
            .await
            .expect("client connects");

        let hook_frame = ClientFrame::Hook(HookEnvelope {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            scope: scope.clone(),
            hook_name: "runtime-state".to_string(),
            session_id: None,
            cwd: temp.path().to_path_buf(),
            payload: serde_json::json!({}),
        });
        client.send_frame(&hook_frame).await.expect("send frame");

        let ack: DaemonFrame = client.read_frame().await.expect("read ack");
        assert_eq!(ack, DaemonFrame::Ack);

        // Send a Subscribe frame and confirm it also acks.
        let subscribe = ClientFrame::Subscribe {
            channels: vec!["board".to_string()],
        };
        client.send_frame(&subscribe).await.expect("send subscribe");
        let ack2: DaemonFrame = client.read_frame().await.expect("read second ack");
        assert_eq!(ack2, DaemonFrame::Ack);

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
        let server_hub = BroadcastHub::new();
        let server_handle = tokio::spawn(async move {
            server::run_server(
                server_endpoint_clone,
                server_socket,
                server_endpoint_path,
                server_hub,
            )
            .await
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
        let server_hub = BroadcastHub::new();
        let server_handle = tokio::spawn(async move {
            server::run_server(
                server_endpoint_clone,
                server_socket,
                server_endpoint_path,
                server_hub,
            )
            .await
        });

        wait_for_socket(&socket_path).await;

        let mut bad_endpoint = server_endpoint.clone();
        bad_endpoint.protocol_version = DAEMON_PROTOCOL_VERSION + 99;

        let result = DaemonClient::connect(&bad_endpoint).await;
        assert!(result.is_err(), "client connect should fail");

        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn client_status_request_returns_daemon_snapshot() {
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let endpoint = sample_endpoint(scope.clone(), &socket_path, "status-secret");

        let server_endpoint = endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let server_hub = BroadcastHub::new();
        // Pre-create one channel so the snapshot has a known nonzero
        // value to assert against.
        let _initial_rx = server_hub.subscribe("warmup");
        let server_handle = tokio::spawn(async move {
            server::run_server(
                server_endpoint,
                server_socket,
                server_endpoint_path,
                server_hub,
            )
            .await
        });

        wait_for_socket(&socket_path).await;

        let mut client = DaemonClient::connect(&endpoint)
            .await
            .expect("client connects");

        client
            .send_frame(&ClientFrame::Status)
            .await
            .expect("send status");
        let frame: DaemonFrame = client.read_frame().await.expect("read status");
        match frame {
            DaemonFrame::Status(status) => {
                assert_eq!(status.protocol_version, DAEMON_PROTOCOL_VERSION);
                assert_eq!(status.daemon_version, "test-daemon");
                // uptime_seconds may be 0 for very fast tests; just check
                // that the field exists in the response by reading it.
                let _uptime = status.uptime_seconds;
                assert!(
                    status.broadcast_channels >= 1,
                    "expected at least the warmup channel, got {}",
                    status.broadcast_channels
                );
                // The asking client itself counts as one connection;
                // the daemon should always report >= 1 from inside the
                // ClientFrame::Status arm.
                assert!(
                    status.connections >= 1,
                    "expected at least one tracked connection, got {}",
                    status.connections
                );
            }
            other => panic!("expected Status frame, got: {other:?}"),
        }

        drop(client);
        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn publish_frame_fans_out_to_subscriber_through_daemon() {
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let endpoint = sample_endpoint(scope.clone(), &socket_path, "publish-secret");

        let server_endpoint = endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let server_hub = BroadcastHub::new();
        let server_handle = tokio::spawn(async move {
            server::run_server(
                server_endpoint,
                server_socket,
                server_endpoint_path,
                server_hub,
            )
            .await
        });

        wait_for_socket(&socket_path).await;

        // Client A: subscribes to "board" and waits for events.
        let mut subscriber = DaemonClient::connect(&endpoint).await.expect("subscriber");
        subscriber
            .send_frame(&ClientFrame::Subscribe {
                channels: vec!["board".to_string()],
            })
            .await
            .expect("subscribe send");
        let sub_ack: DaemonFrame = subscriber.read_frame().await.expect("subscribe ack");
        assert_eq!(sub_ack, DaemonFrame::Ack);

        // Client B: publishes a payload via ClientFrame::Publish.
        let mut publisher = DaemonClient::connect(&endpoint).await.expect("publisher");
        let payload = serde_json::json!({"entries": 9});
        publisher
            .send_frame(&ClientFrame::Publish {
                channel: "board".to_string(),
                payload: payload.clone(),
            })
            .await
            .expect("publish send");
        let pub_ack: DaemonFrame = publisher.read_frame().await.expect("publish ack");
        assert_eq!(pub_ack, DaemonFrame::Ack);

        // Subscriber must observe the matching Event frame.
        let event: DaemonFrame = tokio::time::timeout(
            Duration::from_millis(500),
            subscriber.read_frame::<DaemonFrame>(),
        )
        .await
        .expect("subscriber read timeout")
        .expect("subscriber read");
        assert_eq!(
            event,
            DaemonFrame::Event {
                channel: "board".to_string(),
                payload,
            }
        );

        drop(subscriber);
        drop(publisher);
        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn publish_fans_out_to_multiple_subscribers_on_same_channel() {
        // Locks in the multi-subscriber fan-out invariant: a single
        // `ClientFrame::Publish` reaches every connection that has
        // subscribed to the same channel. Phase H2-H4 will rely on
        // this — runtime status / hook events / launch lifecycle all
        // assume "broadcast to N gwt instances" is the default.
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let endpoint = sample_endpoint(scope.clone(), &socket_path, "fan-out-secret");

        let server_endpoint = endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let server_hub = BroadcastHub::new();
        let server_handle = tokio::spawn(async move {
            server::run_server(
                server_endpoint,
                server_socket,
                server_endpoint_path,
                server_hub,
            )
            .await
        });

        wait_for_socket(&socket_path).await;

        // Three independent subscribers on the same "board" channel.
        let mut subscribers = Vec::with_capacity(3);
        for _ in 0..3 {
            let mut client = DaemonClient::connect(&endpoint)
                .await
                .expect("subscriber connects");
            client
                .send_frame(&ClientFrame::Subscribe {
                    channels: vec!["board".to_string()],
                })
                .await
                .expect("subscribe send");
            let ack: DaemonFrame = client.read_frame().await.expect("subscribe ack");
            assert_eq!(ack, DaemonFrame::Ack);
            subscribers.push(client);
        }

        // Single publisher, single Publish frame.
        let mut publisher = DaemonClient::connect(&endpoint)
            .await
            .expect("publisher connects");
        let payload = serde_json::json!({"entries": 11});
        publisher
            .send_frame(&ClientFrame::Publish {
                channel: "board".to_string(),
                payload: payload.clone(),
            })
            .await
            .expect("publish send");
        let pub_ack: DaemonFrame = publisher.read_frame().await.expect("publish ack");
        assert_eq!(pub_ack, DaemonFrame::Ack);

        // Every subscriber must observe the same Event payload.
        let expected = DaemonFrame::Event {
            channel: "board".to_string(),
            payload,
        };
        for (idx, mut client) in subscribers.into_iter().enumerate() {
            let event: DaemonFrame = tokio::time::timeout(
                Duration::from_millis(500),
                client.read_frame::<DaemonFrame>(),
            )
            .await
            .unwrap_or_else(|_| panic!("subscriber {idx} timeout"))
            .unwrap_or_else(|err| panic!("subscriber {idx} read: {err}"));
            assert_eq!(
                event, expected,
                "subscriber {idx} got unexpected frame: {event:?}"
            );
            drop(client);
        }

        drop(publisher);
        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn subscribed_client_receives_published_broadcast_events() {
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let endpoint = sample_endpoint(scope.clone(), &socket_path, "broadcast-secret");

        let server_endpoint = endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        // Pass a hub clone the test keeps so we can publish into it.
        let server_hub = BroadcastHub::new();
        let publisher = server_hub.clone();
        let server_handle = tokio::spawn(async move {
            server::run_server(
                server_endpoint,
                server_socket,
                server_endpoint_path,
                server_hub,
            )
            .await
        });

        wait_for_socket(&socket_path).await;

        let mut client = DaemonClient::connect(&endpoint)
            .await
            .expect("client connects");

        // Subscribe to the "board" channel and wait for the ack so we
        // know the per-channel forwarder has been spawned before we
        // publish.
        let subscribe = ClientFrame::Subscribe {
            channels: vec!["board".to_string()],
        };
        client.send_frame(&subscribe).await.expect("send subscribe");
        let subscribe_ack: DaemonFrame = client.read_frame().await.expect("subscribe ack");
        assert_eq!(subscribe_ack, DaemonFrame::Ack);

        // Publish a board event into the hub from outside the daemon
        // (Phase H1 GREEN will make a real handler call this).
        let event = DaemonFrame::Event {
            channel: "board".to_string(),
            payload: serde_json::json!({"entries": 7}),
        };
        // Retry briefly: the per-channel forwarder may need a moment to
        // register on the broadcast::Sender after the Subscribe ack.
        let mut delivered = 0;
        for _ in 0..20 {
            delivered = publisher.publish("board", event.clone());
            if delivered > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            delivered > 0,
            "expected hub to have at least one subscriber"
        );

        let received: DaemonFrame = tokio::time::timeout(
            Duration::from_millis(500),
            client.read_frame::<DaemonFrame>(),
        )
        .await
        .expect("event timeout")
        .expect("read event");
        assert_eq!(received, event);

        drop(client);
        server_handle.abort();
        let _ = server_handle.await;
    }
}
