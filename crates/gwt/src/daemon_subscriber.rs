//! Reusable daemon broadcast subscriber thread for SPEC-2077 Phase H1+.
//!
//! `DaemonSubscriber` owns a background OS thread that:
//!
//! 1. Connects to the project's `gwtd` daemon via [`DaemonClient`].
//! 2. Sends a [`ClientFrame::Subscribe`] for the requested channels.
//! 3. Reads [`DaemonFrame::Event`] frames in a loop and forwards each
//!    `(channel, payload)` pair to the supplied callback.
//! 4. On disconnect or error, sleeps with exponential backoff (capped
//!    at 5 s) and reconnects until [`DaemonSubscriber::stop`] is called
//!    or the subscriber is dropped.
//!
//! Phase H1 GREEN handler migration uses this primitive to bridge
//! `DaemonFrame::Event { channel: "board" }` into a
//! `UserEvent::BoardProjectionChanged` send on gwt's tao event loop.
//! Future phases (H2-H4) will reuse the same primitive for runtime
//! status, hook events, and launch lifecycle channels.

#![cfg(unix)]

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use gwt_core::daemon::{ClientFrame, DaemonEndpoint, DaemonFrame};
use tokio::sync::Notify;

use crate::cli::daemon::client::DaemonClient;

const RETRY_BACKOFF_MIN: Duration = Duration::from_millis(100);
const RETRY_BACKOFF_MAX: Duration = Duration::from_secs(5);

/// Closure that produces the *current* daemon endpoint for each
/// connect attempt. Returning `Err` puts the subscriber into
/// retry-with-backoff mode (e.g. when no daemon is registered yet),
/// while a successful resolve provides a fresh `DaemonEndpoint` —
/// crucially, picking up any new `auth_token` / `bind` after a daemon
/// restart.
pub type EndpointResolver = dyn Fn() -> Result<DaemonEndpoint, String> + Send + Sync + 'static;

/// Owns the subscriber thread. Drop or call [`Self::stop`] to wind it
/// down cleanly.
pub struct DaemonSubscriber {
    stop_flag: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl DaemonSubscriber {
    /// Spawn a subscriber thread for a fixed `endpoint`. Convenience
    /// shorthand for [`Self::spawn_with_resolver`] when the caller
    /// guarantees the endpoint never changes (mostly tests with an
    /// in-process daemon).
    pub fn spawn<F>(endpoint: DaemonEndpoint, channels: Vec<String>, on_event: F) -> Self
    where
        F: Fn(String, serde_json::Value) + Send + Sync + 'static,
    {
        Self::spawn_with_resolver(move || Ok(endpoint.clone()), channels, on_event)
    }

    /// Spawn a subscriber thread that re-resolves its `DaemonEndpoint`
    /// on every connect attempt. Each received
    /// `DaemonFrame::Event { channel, payload }` invokes
    /// `on_event(channel, payload)`. The callback runs on the
    /// subscriber thread; keep it short and forward to your own queue
    /// (e.g. via a `Sender` clone) if more work is needed.
    ///
    /// Re-resolving is what keeps subscribers healthy across daemon
    /// restarts. When `gwtd daemon start` is killed and respawned, the
    /// new daemon picks a fresh `auth_token`, so a subscriber that
    /// caches the old endpoint would loop forever on handshake
    /// rejection. Calling `resolver()` per session gives the caller a
    /// chance to re-read the persisted endpoint file before each
    /// reconnect.
    pub fn spawn_with_resolver<R, F>(resolver: R, channels: Vec<String>, on_event: F) -> Self
    where
        R: Fn() -> Result<DaemonEndpoint, String> + Send + Sync + 'static,
        F: Fn(String, serde_json::Value) + Send + Sync + 'static,
    {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(Notify::new());
        let stop_flag_inner = Arc::clone(&stop_flag);
        let shutdown_inner = Arc::clone(&shutdown);
        let callback = Arc::new(on_event);
        let resolver: Arc<EndpointResolver> = Arc::new(resolver);
        let join_handle = thread::Builder::new()
            .name("gwt-daemon-subscriber".to_string())
            .spawn(move || {
                run_loop(
                    resolver,
                    channels,
                    callback,
                    stop_flag_inner,
                    shutdown_inner,
                )
            })
            .expect("daemon subscriber thread spawn");
        Self {
            stop_flag,
            shutdown,
            join_handle: Some(join_handle),
        }
    }

    /// Signal the subscriber thread to stop and wait for it to exit.
    pub fn stop(mut self) {
        self.signal_stop();
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }

    fn signal_stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.shutdown.notify_waiters();
    }
}

impl Drop for DaemonSubscriber {
    fn drop(&mut self) {
        self.signal_stop();
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_loop(
    resolver: Arc<EndpointResolver>,
    channels: Vec<String>,
    on_event: Arc<dyn Fn(String, serde_json::Value) + Send + Sync>,
    stop_flag: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            tracing::warn!(error = %err, "daemon subscriber: tokio runtime build failed");
            return;
        }
    };

    let mut backoff = RETRY_BACKOFF_MIN;
    while !stop_flag.load(Ordering::SeqCst) {
        let endpoint = match resolver() {
            Ok(ep) => ep,
            Err(err) => {
                tracing::debug!(error = %err, "daemon subscriber: endpoint resolve failed, retrying");
                sleep_with_stop(&stop_flag, &shutdown, backoff);
                backoff = (backoff * 2).min(RETRY_BACKOFF_MAX);
                continue;
            }
        };
        let channels_for_session = channels.clone();
        let callback_for_session = Arc::clone(&on_event);
        let shutdown_for_session = Arc::clone(&shutdown);
        let stop_flag_for_session = Arc::clone(&stop_flag);
        let session = runtime.block_on(async move {
            run_session(
                endpoint,
                channels_for_session,
                callback_for_session,
                shutdown_for_session,
                stop_flag_for_session,
            )
            .await
        });

        if stop_flag.load(Ordering::SeqCst) {
            break;
        }

        if let Err(err) = session {
            tracing::debug!(error = %err, "daemon subscriber: session ended, retrying");
        }

        sleep_with_stop(&stop_flag, &shutdown, backoff);
        backoff = (backoff * 2).min(RETRY_BACKOFF_MAX);
    }
}

async fn run_session(
    endpoint: DaemonEndpoint,
    channels: Vec<String>,
    on_event: Arc<dyn Fn(String, serde_json::Value) + Send + Sync>,
    shutdown: Arc<Notify>,
    stop_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut client = DaemonClient::connect(&endpoint)
        .await
        .map_err(|err| format!("connect failed: {err}"))?;
    client
        .send_frame(&ClientFrame::Subscribe { channels })
        .await
        .map_err(|err| format!("subscribe send failed: {err}"))?;
    // The first frame after Subscribe is the daemon's Ack; drain it so
    // the subsequent loop only sees Event frames.
    let _ack: DaemonFrame = client
        .read_frame()
        .await
        .map_err(|err| format!("subscribe ack failed: {err}"))?;

    loop {
        // Re-check the flag at the top of each iteration so a
        // `notify_waiters()` that fires while we're processing the
        // previous frame is not lost (Notify has no permit semantics
        // for `notify_waiters`).
        if stop_flag.load(Ordering::SeqCst) {
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = shutdown.notified() => {
                return Ok(());
            }
            frame_result = client.read_frame::<DaemonFrame>() => {
                let frame = frame_result
                    .map_err(|err| format!("event read failed: {err}"))?;
                match frame {
                    DaemonFrame::Event { channel, payload } => {
                        on_event(channel, payload);
                    }
                    DaemonFrame::Ack | DaemonFrame::Status(_) => {
                        // ignore stray non-event frames; daemon may emit
                        // them for unrelated control flow.
                    }
                    DaemonFrame::Error { message } => {
                        tracing::warn!(
                            error = %message,
                            "daemon subscriber: error frame"
                        );
                    }
                }
            }
        }
    }
}

fn sleep_with_stop(stop_flag: &AtomicBool, _shutdown: &Notify, total: Duration) {
    let tick = Duration::from_millis(20);
    let mut elapsed = Duration::ZERO;
    while elapsed < total {
        if stop_flag.load(Ordering::SeqCst) {
            return;
        }
        let step = tick.min(total - elapsed);
        thread::sleep(step);
        elapsed += step;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use gwt_core::daemon::{DaemonEndpoint, DaemonFrame, RuntimeScope, RuntimeTarget};
    use serde_json::json;
    use tempfile::TempDir;

    use super::DaemonSubscriber;
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscriber_forwards_events_through_callback() {
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let endpoint = sample_endpoint(scope.clone(), &socket_path, "subscriber-secret");

        let server_endpoint = endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let hub = BroadcastHub::new();
        let publisher = hub.clone();
        let server_handle = tokio::spawn(async move {
            server::run_server(server_endpoint, server_socket, server_endpoint_path, hub).await
        });

        wait_for_socket(&socket_path).await;

        let received: Arc<Mutex<Vec<(String, serde_json::Value)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let received_for_cb = Arc::clone(&received);
        let subscriber = DaemonSubscriber::spawn(
            endpoint,
            vec!["board".to_string()],
            move |channel, payload| {
                received_for_cb.lock().unwrap().push((channel, payload));
            },
        );

        // Publish must be retried briefly: the per-connection forwarder
        // task on the daemon side races with the test's first publish.
        let event = DaemonFrame::Event {
            channel: "board".to_string(),
            payload: json!({"entries": 5}),
        };
        for _ in 0..50 {
            if publisher.publish("board", event.clone()) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Wait up to 1 s for the callback to record the event.
        let mut delivered = false;
        for _ in 0..100 {
            if !received.lock().unwrap().is_empty() {
                delivered = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(delivered, "expected callback to receive at least one event");

        let captured = received.lock().unwrap().clone();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, "board");
        assert_eq!(captured[0].1, json!({"entries": 5}));

        subscriber.stop();
        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscriber_stop_unblocks_thread_even_without_events() {
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let endpoint = sample_endpoint(scope.clone(), &socket_path, "stop-secret");

        let server_endpoint = endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let hub = BroadcastHub::new();
        let server_handle = tokio::spawn(async move {
            server::run_server(server_endpoint, server_socket, server_endpoint_path, hub).await
        });

        wait_for_socket(&socket_path).await;

        let subscriber =
            DaemonSubscriber::spawn(endpoint, vec!["board".to_string()], |_channel, _payload| {});

        // Drop without sending any events; stop() must complete promptly.
        subscriber.stop();

        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscriber_with_resolver_picks_up_endpoint_after_initial_failure() {
        // Reflects the production race: a project tab opens before
        // `gwtd daemon start` is invoked, so the first resolver call
        // returns Err. Once the daemon comes up the resolver returns
        // the live endpoint and the subscriber starts receiving
        // events without needing a re-spawn.

        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let endpoint = sample_endpoint(scope.clone(), &socket_path, "resolver-secret");

        // The resolver returns Err the first 3 times, then Ok. This
        // simulates the daemon coming up after the subscriber is
        // already retrying.
        let calls = Arc::new(Mutex::new(0usize));
        let endpoint_for_resolver = endpoint.clone();
        let calls_for_resolver = Arc::clone(&calls);
        let resolver = move || -> Result<DaemonEndpoint, String> {
            let mut guard = calls_for_resolver.lock().unwrap();
            *guard += 1;
            if *guard <= 3 {
                Err("daemon not running".to_string())
            } else {
                Ok(endpoint_for_resolver.clone())
            }
        };

        let received: Arc<Mutex<Vec<(String, serde_json::Value)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let received_for_cb = Arc::clone(&received);
        let subscriber = DaemonSubscriber::spawn_with_resolver(
            resolver,
            vec!["board".to_string()],
            move |channel, payload| {
                received_for_cb.lock().unwrap().push((channel, payload));
            },
        );

        // Now bring the daemon up. The resolver is on a backoff loop;
        // wait long enough for it to call past the threshold and then
        // start the server before the next backoff window.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let server_endpoint = endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let hub = BroadcastHub::new();
        let publisher = hub.clone();
        let server_handle = tokio::spawn(async move {
            server::run_server(server_endpoint, server_socket, server_endpoint_path, hub).await
        });

        // Wait for the daemon socket and then publish.
        wait_for_socket(&socket_path).await;
        let event = DaemonFrame::Event {
            channel: "board".to_string(),
            payload: json!({"entries": 11}),
        };
        for _ in 0..200 {
            if publisher.publish("board", event.clone()) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // Wait up to 5 s for the callback to record the event.
        let mut delivered = false;
        for _ in 0..500 {
            if !received.lock().unwrap().is_empty() {
                delivered = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            delivered,
            "expected resolver-based subscriber to receive event after daemon came up"
        );
        assert!(
            *calls.lock().unwrap() >= 4,
            "expected resolver to retry at least 3 times before succeeding"
        );

        subscriber.stop();
        server_handle.abort();
        let _ = server_handle.await;
    }
}
