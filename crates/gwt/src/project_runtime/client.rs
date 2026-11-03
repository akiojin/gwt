//! Private Rust GUI client; the daemon token never enters a browser projection.
use super::events::{OwnerCommand, OwnerReply, OwnerRequest};
use std::sync::OnceLock;

struct CommandTransport {
    http: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}

fn command_transport() -> Result<&'static CommandTransport, String> {
    static TRANSPORT: OnceLock<Result<CommandTransport, String>> = OnceLock::new();
    TRANSPORT
        .get_or_init(|| {
            // Build the client first so a client initialization failure never
            // drops a Tokio runtime inside an async caller.
            let http = reqwest::Client::builder()
                .pool_max_idle_per_host(8)
                .build()
                .map_err(|error| error.to_string())?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("gwt-private-http")
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            Ok(CommandTransport { http, runtime })
        })
        .as_ref()
        .map_err(|error| error.clone())
}

#[derive(Clone)]
pub struct ProjectRuntimeClient {
    url: String,
    token: String,
    project_key: String,
    transport: &'static CommandTransport,
    identity: Option<crate::daemon_subscriber::ProjectOwnerIdentity>,
}

impl ProjectRuntimeClient {
    pub fn new(endpoint: &gwt_core::daemon::DaemonEndpoint) -> Result<Self, String> {
        Ok(Self {
            url: endpoint
                .project_transport_url
                .clone()
                .ok_or("daemon has no project transport")?,
            token: endpoint.auth_token.clone(),
            project_key: endpoint.scope.repo_hash.clone(),
            transport: command_transport()?,
            identity: None,
        })
    }

    /// Resolve and authenticate this project's current physical owner. The
    /// existing health validator pins PID, birth and instance to discovery.
    pub async fn from_current_project(
        project_root: &std::path::Path,
        gwt_home: &std::path::Path,
    ) -> Result<Self, String> {
        use gwt_core::daemon::{DaemonBootstrapAction, RuntimeScope, RuntimeTarget};
        let scope = RuntimeScope::from_project_root(project_root, RuntimeTarget::Host)
            .map_err(|error| error.to_string())?;
        let action = gwt_core::daemon::resolve_bootstrap_action_for_version(
            gwt_home,
            &scope,
            gwt_core::daemon::DAEMON_PROTOCOL_VERSION,
            env!("CARGO_PKG_VERSION"),
            crate::process::is_process_alive,
        )
        .map_err(|error| error.to_string())?;
        let DaemonBootstrapAction::Reuse(endpoint) = action else {
            return Err("Project execution owner is not ready".into());
        };
        let mut client = Self::new(&endpoint)?;
        client.identity =
            Some(crate::daemon_subscriber::inspect_project_owner(&client, &endpoint).await?);
        Ok(client)
    }

    pub fn identity(&self) -> Option<&crate::daemon_subscriber::ProjectOwnerIdentity> {
        self.identity.as_ref()
    }

    /// Used from the GUI's existing background tasks. The established
    /// supervisor remains the only child/ensure registry.
    pub async fn connect_for_gui(
        project_root: std::path::PathBuf,
        supervisor: std::sync::Arc<crate::daemon_supervisor::DaemonSupervisor>,
    ) -> Result<Self, String> {
        let gwt_home = gwt_core::paths::gwt_home();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let error = match Self::from_current_project(&project_root, &gwt_home).await {
                Ok(client) => return Ok(client),
                Err(error) => error,
            };
            if tokio::time::Instant::now() >= deadline {
                return Err(error);
            }
            let ensure = supervisor.clone();
            let root = project_root.clone();
            tokio::task::spawn_blocking(move || ensure.ensure_running(&root))
                .await
                .map_err(|error| error.to_string())??;
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    pub async fn events(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<super::events::OwnerEvent, String>>, String>
    {
        self.events_with_gui_presence(false).await
    }
    /// An authenticated Rust GUI connection owns exactly one residency lease.
    pub async fn gui_events(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<super::events::OwnerEvent, String>>, String>
    {
        self.events_with_gui_presence(true).await
    }
    async fn events_with_gui_presence(
        &self,
        gui_presence: bool,
    ) -> Result<impl futures_util::Stream<Item = Result<super::events::OwnerEvent, String>>, String>
    {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let url = format!(
            "{}/private/project/events",
            self.url.replacen("http://", "ws://", 1)
        );
        let mut request = url
            .into_client_request()
            .map_err(|error| error.to_string())?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", self.token)
                .parse()
                .map_err(|error: axum::http::header::InvalidHeaderValue| error.to_string())?,
        );
        if gui_presence {
            request.headers_mut().insert(
                "x-gwt-gui-presence",
                "1".parse().expect("static presence header"),
            );
        }
        #[cfg(windows)]
        let (stream, _) = {
            use tokio_tungstenite::tungstenite::error::{Error, UrlError};

            let host = request
                .uri()
                .host()
                .ok_or_else(|| Error::Url(UrlError::NoHostName).to_string())?;
            // The private owner publishes plain HTTP. Resolve the same host
            // and try every address in order, as connect_async does, but create
            // the socket with NO_HANDLE_INHERIT before any concurrent spawn.
            // Otherwise a later project's daemon can retain this GUI lease.
            let port = match request.uri().scheme_str() {
                Some("ws") => request.uri().port_u16().unwrap_or(80),
                _ => return Err(Error::Url(UrlError::UnsupportedUrlScheme).to_string()),
            };
            let addresses = tokio::net::lookup_host(format!("{host}:{port}"))
                .await
                .map_err(|error| error.to_string())?;
            let mut connected = None;
            let mut last_error = None;
            for address in addresses {
                let socket = if address.is_ipv4() {
                    tokio::net::TcpSocket::new_v4()
                } else {
                    tokio::net::TcpSocket::new_v6()
                };
                let result = match socket {
                    Ok(socket) => socket.connect(address).await,
                    Err(error) => Err(error),
                };
                match result {
                    Ok(socket) => {
                        connected = Some(socket);
                        break;
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            let socket = connected.ok_or_else(|| {
                last_error
                    .unwrap_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "could not resolve to any address",
                        )
                    })
                    .to_string()
            })?;
            tokio_tungstenite::client_async(request, socket)
                .await
                .map_err(|error| error.to_string())?
        };
        #[cfg(not(windows))]
        let (stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|error| error.to_string())?;
        Ok(stream.filter_map(|message| async move {
            match message {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    Some(serde_json::from_str(&text).map_err(|error| error.to_string()))
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                    Some(Err("project owner event connection closed".into()))
                }
                Err(error) => Some(Err(error.to_string())),
                _ => None,
            }
        }))
    }
    pub async fn command(&self, command: OwnerCommand) -> OwnerReply {
        self.command_with_timeout(command, std::time::Duration::from_secs(120))
            .await
    }

    /// Private transaction frames have bounded observation calls. A timeout
    /// never cancels a frame and is never evidence that physical input stopped.
    #[doc(hidden)]
    pub async fn mobile_input_command(&self, command: OwnerCommand) -> OwnerReply {
        if !matches!(
            command,
            OwnerCommand::PrepareMobileInput { .. }
                | OwnerCommand::CommitMobileInput { .. }
                | OwnerCommand::CancelMobileInput { .. }
                | OwnerCommand::MobileInputOutcome { .. }
                | OwnerCommand::SettleMobileInput { .. }
        ) {
            return Err("not a private prepared input frame".into());
        }
        self.command_with_timeout(command, std::time::Duration::from_secs(2))
            .await
    }

    async fn command_with_timeout(
        &self,
        command: OwnerCommand,
        timeout: std::time::Duration,
    ) -> OwnerReply {
        let request = self
            .transport
            .http
            .post(format!("{}/private/project/command", self.url))
            .bearer_auth(&self.token)
            .timeout(timeout)
            .json(&OwnerRequest {
                project_key: self.project_key.clone(),
                command,
            });
        // Both the connection driver and response body belong to the process
        // runtime, so ending any short-lived GUI caller cannot break the pool.
        // JoinSet aborts the task when the caller drops this request future.
        let mut pending = tokio::task::JoinSet::new();
        pending.spawn_on(
            async move {
                let response = request.send().await.map_err(|error| error.to_string())?;
                let status = response.status();
                let reply: serde_json::Value =
                    response.json().await.map_err(|error| error.to_string())?;
                if status.is_success() {
                    Ok(reply)
                } else {
                    Err(reply["error"]
                        .as_str()
                        .unwrap_or("project command refused")
                        .into())
                }
            },
            self.transport.runtime.handle(),
        );
        pending
            .join_next()
            .await
            .expect("private HTTP request task was registered")
            .map_err(|error| error.to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{ConnectInfo, State},
        http::HeaderMap,
        routing::post,
        Json, Router,
    };
    use gwt_core::{
        daemon::{DaemonEndpoint, RuntimeScope, RuntimeTarget},
        test_support::recv_event,
    };
    use std::{
        collections::HashSet,
        net::SocketAddr,
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc, Mutex,
        },
    };

    #[test]
    fn request_survives_another_callers_runtime_shutdown() {
        struct Fixture {
            requests: AtomicUsize,
            connections: Mutex<HashSet<SocketAddr>>,
            received: mpsc::Sender<()>,
            release: tokio::sync::Notify,
        }
        async fn command(
            State(fixture): State<Arc<Fixture>>,
            ConnectInfo(peer): ConnectInfo<SocketAddr>,
            headers: HeaderMap,
            Json(request): Json<OwnerRequest>,
        ) -> Json<serde_json::Value> {
            assert_eq!(headers["authorization"], "Bearer inert-fixture");
            assert!(matches!(request.command, OwnerCommand::Health));
            fixture.connections.lock().unwrap().insert(peer);
            if fixture.requests.fetch_add(1, Ordering::SeqCst) == 1 {
                fixture.received.send(()).unwrap();
                fixture.release.notified().await;
            }
            Json(serde_json::json!({"ok": true}))
        }
        let server = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let (received, arrivals) = mpsc::channel();
        let fixture = Arc::new(Fixture {
            requests: AtomicUsize::new(0),
            connections: Mutex::new(HashSet::new()),
            received,
            release: tokio::sync::Notify::new(),
        });
        let state = fixture.clone();
        let (address, served) = server.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = Router::new()
                .route("/private/project/command", post(command))
                .with_state(state);
            let served = tokio::spawn(async move {
                axum::serve(
                    listener,
                    router.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .unwrap();
            });
            (address, served)
        });
        let scope = RuntimeScope::new(
            "fixture",
            "fixture",
            std::env::current_dir().unwrap(),
            RuntimeTarget::Host,
        )
        .unwrap();
        let mut endpoint = DaemonEndpoint::new(
            scope,
            std::process::id(),
            "fixture".into(),
            "inert-fixture".into(),
            "fixture".into(),
        );
        endpoint.project_transport_url = Some(format!("http://{address}"));
        let client = ProjectRuntimeClient::new(&endpoint).unwrap();
        let (finish_first, first_finished) = tokio::sync::oneshot::channel();
        let (first_ready, ready) = mpsc::channel();
        let initial = client.clone();
        let first = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                initial.command(OwnerCommand::Health).await.unwrap();
                first_ready.send(()).unwrap();
                let _ = first_finished.await;
            });
            drop(runtime);
        });
        recv_event(&ready, "first private command completed");
        let next = client.clone();
        let second = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(next.command(OwnerCommand::Health))
        });
        recv_event(&arrivals, "second private command reached the server");
        finish_first.send(()).unwrap();
        first.join().unwrap();
        fixture.release.notify_one();
        let result = second.join().unwrap();
        // New wrappers and their clones must share the same process pool,
        // including after each caller's runtime has been fully dropped.
        for _ in 0..16 {
            let fresh = ProjectRuntimeClient::new(&endpoint).unwrap();
            for caller in [fresh.clone(), fresh] {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(caller.command(OwnerCommand::Health))
                    .unwrap();
            }
        }
        drop(client);
        // A process-wide pool outlives this fixture; do not wait for its idle
        // timeout during graceful server shutdown.
        served.abort();
        assert!(server.block_on(served).unwrap_err().is_cancelled());
        drop(server);
        assert!(
            result.is_ok(),
            "another caller's runtime teardown aborted the private command: {result:?}"
        );
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 34);
        let connections = fixture.connections.lock().unwrap().len();
        println!("private HTTP: 34 requests, {connections} TCP connections");
        assert_eq!(connections, 1, "private HTTP connections were not reused");
    }
}
