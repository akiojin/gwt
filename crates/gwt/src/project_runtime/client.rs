//! Private Rust GUI client; the daemon token never enters a browser projection.
use super::events::{OwnerCommand, OwnerReply, OwnerRequest};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

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
                // A failed redirected connection would not prove that the
                // original command was unsent. Private commands never redirect.
                .redirect(reqwest::redirect::Policy::none())
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
    route: Arc<tokio::sync::Mutex<ClientRoute>>,
    gwt_home: PathBuf,
    project_key: String,
    transport: &'static CommandTransport,
    identity: Option<crate::daemon_subscriber::ProjectOwnerIdentity>,
}

struct ClientRoute {
    endpoint: gwt_core::daemon::DaemonEndpoint,
    unavailable: Option<String>,
    recovery_pending: bool,
}

const INTERRUPTED_ENDPOINT_RECOVERY: &str =
    "Project execution owner endpoint re-resolution was interrupted before command delivery";

struct CommandFailure {
    message: String,
    connect: bool,
}

impl ProjectRuntimeClient {
    pub fn new(endpoint: &gwt_core::daemon::DaemonEndpoint) -> Result<Self, String> {
        endpoint
            .project_transport_url
            .as_ref()
            .ok_or("daemon has no project transport")?;
        Ok(Self {
            route: Arc::new(tokio::sync::Mutex::new(ClientRoute {
                endpoint: endpoint.clone(),
                unavailable: None,
                recovery_pending: false,
            })),
            gwt_home: gwt_core::paths::gwt_home(),
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
        let endpoint = Self::resolve_endpoint(project_root, gwt_home)?;
        let mut client = Self::new(&endpoint)?;
        client.gwt_home = gwt_home.to_path_buf();
        client.identity =
            Some(crate::daemon_subscriber::inspect_project_owner(&client, &endpoint).await?);
        Ok(client)
    }

    fn resolve_endpoint(
        project_root: &std::path::Path,
        gwt_home: &std::path::Path,
    ) -> Result<gwt_core::daemon::DaemonEndpoint, String> {
        use gwt_core::daemon::{DaemonBootstrapAction, RuntimeScope, RuntimeTarget};
        let scope = RuntimeScope::from_project_root(project_root, RuntimeTarget::Host)
            .map_err(|error| error.to_string())?;
        // One project owner can serve several worktrees. Prefer its authority
        // endpoint before the caller's exact descriptor, as the subscriber does.
        let action = match crate::daemon_publisher::resolve_live_project_authority(
            gwt_home,
            &scope,
            &crate::process::is_process_alive,
        )? {
            Some(endpoint)
                if endpoint.daemon_version.trim().trim_start_matches('v')
                    == env!("CARGO_PKG_VERSION").trim().trim_start_matches('v') =>
            {
                DaemonBootstrapAction::Reuse(endpoint)
            }
            Some(endpoint) => DaemonBootstrapAction::RetireStaleVersion { endpoint },
            None => gwt_core::daemon::resolve_bootstrap_action_for_version(
                gwt_home,
                &scope,
                gwt_core::daemon::DAEMON_PROTOCOL_VERSION,
                env!("CARGO_PKG_VERSION"),
                crate::process::is_process_alive,
            )
            .map_err(|error| error.to_string())?,
        };
        let DaemonBootstrapAction::Reuse(endpoint) = action else {
            return Err("Project execution owner is not ready".into());
        };
        Ok(endpoint)
    }

    pub fn identity(&self) -> Option<&crate::daemon_subscriber::ProjectOwnerIdentity> {
        self.identity.as_ref()
    }

    /// Used from the GUI's existing background tasks. The established
    /// supervisor remains the only child/ensure registry. Keep the returned
    /// event stream alive throughout the operation to retain GUI residency.
    pub async fn connect_for_gui(
        project_root: std::path::PathBuf,
        supervisor: std::sync::Arc<crate::daemon_supervisor::DaemonSupervisor>,
    ) -> Result<
        (
            Self,
            impl futures_util::Stream<Item = Result<super::events::OwnerEvent, String>>,
        ),
        String,
    > {
        let gwt_home = gwt_core::paths::gwt_home();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let error = match Self::from_current_project(&project_root, &gwt_home).await {
                Ok(client) => match client.gui_events().await {
                    Ok(presence) => return Ok((client, presence)),
                    Err(error) => error,
                },
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
    ) -> Result<
        impl futures_util::Stream<Item = Result<super::events::OwnerEvent, String>>
            + futures_util::Sink<
                tokio_tungstenite::tungstenite::Message,
                Error = tokio_tungstenite::tungstenite::Error,
            >,
        String,
    > {
        self.events_with_gui_presence(true).await
    }
    async fn events_with_gui_presence(
        &self,
        gui_presence: bool,
    ) -> Result<
        impl futures_util::Stream<Item = Result<super::events::OwnerEvent, String>>
            + futures_util::Sink<
                tokio_tungstenite::tungstenite::Message,
                Error = tokio_tungstenite::tungstenite::Error,
            >,
        String,
    > {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let endpoint = self.route.lock().await.endpoint.clone();
        let url = format!(
            "{}/private/project/events",
            endpoint
                .project_transport_url
                .as_deref()
                .ok_or("daemon has no project transport")?
                .replacen("http://", "ws://", 1)
        );
        let mut request = url
            .into_client_request()
            .map_err(|error| error.to_string())?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", endpoint.auth_token)
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
        // Recovery shares the original observation budget, including the
        // two-second private input frames. It never extends their deadline.
        let outcome =
            tokio::time::timeout(timeout, self.command_with_recovery(command, timeout)).await;
        match outcome {
            Ok(result) => result,
            Err(_) => {
                // Do not wait on another caller beyond this frame's budget.
                if let Ok(mut route) = self.route.try_lock() {
                    if route.recovery_pending {
                        return Err(self.record_unavailable_route(
                            &mut route,
                            INTERRUPTED_ENDPOINT_RECOVERY.into(),
                        ));
                    }
                }
                Err("project command timed out".into())
            }
        }
    }

    async fn command_with_recovery(
        &self,
        command: OwnerCommand,
        timeout: std::time::Duration,
    ) -> OwnerReply {
        let payload = serde_json::to_vec(&OwnerRequest {
            project_key: self.project_key.clone(),
            command,
        })
        .map_err(|error| error.to_string())?;
        let endpoint = {
            let mut route = self.route.lock().await;
            if let Some(error) = &route.unavailable {
                return Err(error.clone());
            }
            if route.recovery_pending {
                return Err(
                    self.record_unavailable_route(&mut route, INTERRUPTED_ENDPOINT_RECOVERY.into())
                );
            }
            route.endpoint.clone()
        };
        match self.send_command(&endpoint, payload.clone(), timeout).await {
            Ok(reply) => return Ok(reply),
            Err(error) if error.connect && self.identity.is_some() => {}
            Err(error) => return Err(error.message),
        }

        // Only a connection failure proves this command was never sent. Keep
        // one recovery in flight across clones; ambiguous outcomes never retry.
        let mut route = self.route.lock().await;
        if let Some(error) = &route.unavailable {
            return Err(error.clone());
        }
        if route.recovery_pending {
            return Err(
                self.record_unavailable_route(&mut route, INTERRUPTED_ENDPOINT_RECOVERY.into())
            );
        }
        if route.endpoint == endpoint {
            // A live recovery owns this mutex. A later lock holder observing
            // this flag proves that recovery was cancelled before any resend.
            route.recovery_pending = true;
            let replacement = async {
                let next = Self::resolve_endpoint(&endpoint.scope.project_root, &self.gwt_home)?;
                let expected = self.identity.as_ref().expect("authenticated recovery");
                if next.scope.repo_hash != self.project_key || next.pid != expected.pid {
                    return Err("Project execution owner identity changed; reattach its inventory before sending commands".into());
                }
                if next.project_transport_url == endpoint.project_transport_url {
                    return Err("Project execution owner endpoint is still unavailable after re-resolution".into());
                }
                next.project_transport_url.as_ref().ok_or("daemon has no project transport")?;
                let health = serde_json::to_vec(&OwnerRequest {
                    project_key: self.project_key.clone(),
                    command: OwnerCommand::Health,
                }).map_err(|error| error.to_string())?;
                let health = self.send_command(&next, health, std::time::Duration::from_secs(5))
                    .await.map_err(|error| error.message)?;
                let actual = crate::daemon_subscriber::validate_project_owner_health(health, &next)?;
                if &actual != expected {
                    return Err("Project execution owner identity changed; reattach its inventory before sending commands".into());
                }
                Ok(next)
            }.await;
            route.recovery_pending = false;
            match replacement {
                Ok(endpoint) => route.endpoint = endpoint,
                Err(error) => return Err(self.record_unavailable_route(&mut route, error)),
            }
        }
        let endpoint = route.endpoint.clone();
        drop(route);
        match self.send_command(&endpoint, payload, timeout).await {
            Ok(reply) => Ok(reply),
            Err(error) if error.connect => {
                let mut route = self.route.lock().await;
                if route.endpoint != endpoint {
                    return Err(error.message);
                }
                Err(self.record_unavailable_route(&mut route, error.message))
            }
            Err(error) => Err(error.message),
        }
    }

    fn record_unavailable_route(&self, route: &mut ClientRoute, cause: String) -> String {
        route.recovery_pending = false;
        if let Some(error) = &route.unavailable {
            return error.clone();
        }
        use gwt_core::error_ledger::{ErrorKind, ErrorRecord, ErrorTarget};
        let diagnostic = format!("Project execution owner endpoint recovery needs_human: {cause}");
        let mut record = ErrorRecord::new(
            ErrorKind::DaemonFault,
            &diagnostic,
            ErrorTarget {
                project_root: Some(
                    route
                        .endpoint
                        .scope
                        .project_root
                        .to_string_lossy()
                        .into_owned(),
                ),
                ..Default::default()
            },
        )
        .with_context(std::collections::BTreeMap::from([
            ("operation".into(), "project_owner_endpoint".into()),
            ("recovery".into(), "needs_human".into()),
        ]));
        let identity = self.identity.as_ref().expect("authenticated recovery");
        record.id = format!(
            "project-owner-endpoint-{}-{}-{}-{}",
            self.project_key, identity.pid, identity.host_started_at, identity.host_instance_id
        );
        if let Err(error) = gwt_core::error_ledger::record_once(record) {
            tracing::warn!(%error, "could not record project owner endpoint recovery");
        }
        route.unavailable = Some(diagnostic.clone());
        diagnostic
    }

    async fn send_command(
        &self,
        endpoint: &gwt_core::daemon::DaemonEndpoint,
        payload: Vec<u8>,
        timeout: std::time::Duration,
    ) -> Result<serde_json::Value, CommandFailure> {
        let request = self
            .transport
            .http
            .post(format!(
                "{}/private/project/command",
                endpoint
                    .project_transport_url
                    .as_deref()
                    .expect("validated transport")
            ))
            .bearer_auth(&endpoint.auth_token)
            .timeout(timeout)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload);
        // Both the connection driver and response body belong to the process
        // runtime, so ending any short-lived GUI caller cannot break the pool.
        // JoinSet aborts the task when the caller drops this request future.
        let mut pending = tokio::task::JoinSet::new();
        pending.spawn_on(
            async move {
                let response = request.send().await.map_err(|error| CommandFailure {
                    connect: error.is_connect(),
                    message: error.to_string(),
                })?;
                let status = response.status();
                let reply: serde_json::Value =
                    response.json().await.map_err(|error| CommandFailure {
                        connect: false,
                        message: error.to_string(),
                    })?;
                if status.is_success() {
                    Ok(reply)
                } else {
                    Err(CommandFailure {
                        connect: false,
                        message: reply["error"]
                            .as_str()
                            .unwrap_or("project command refused")
                            .into(),
                    })
                }
            },
            self.transport.runtime.handle(),
        );
        pending
            .join_next()
            .await
            .expect("private HTTP request task was registered")
            .map_err(|error| CommandFailure {
                connect: false,
                message: error.to_string(),
            })?
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

    async fn recovery_endpoint(
        root: &std::path::Path,
        host_instance_id: &str,
        health_started: Option<Arc<tokio::sync::Notify>>,
    ) -> (
        DaemonEndpoint,
        tokio::task::JoinHandle<()>,
        Arc<Mutex<Vec<&'static str>>>,
    ) {
        #[derive(Clone)]
        struct Fixture {
            project_key: String,
            host_instance_id: String,
            requests: Arc<Mutex<Vec<&'static str>>>,
            health_started: Option<Arc<tokio::sync::Notify>>,
        }
        async fn command(
            State(fixture): State<Fixture>,
            headers: HeaderMap,
            Json(request): Json<OwnerRequest>,
        ) -> Json<serde_json::Value> {
            assert_eq!(headers["authorization"], "Bearer recovery-fixture");
            assert_eq!(request.project_key, fixture.project_key);
            if matches!(request.command, OwnerCommand::Health) {
                fixture.requests.lock().unwrap().push("health");
                if let Some(started) = fixture.health_started {
                    started.notify_one();
                    std::future::pending::<()>().await;
                }
                Json(serde_json::json!({
                    "project_key": fixture.project_key,
                    "pid": std::process::id(),
                    "host_started_at": crate::process::host_process_start_time(std::process::id()).unwrap(),
                    "host_instance_id": fixture.host_instance_id,
                }))
            } else {
                fixture.requests.lock().unwrap().push("command");
                Json(serde_json::json!({"accepted": true}))
            }
        }
        let scope = RuntimeScope::from_project_root(root, RuntimeTarget::Host).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let fixture = Fixture {
            project_key: scope.repo_hash.clone(),
            host_instance_id: host_instance_id.into(),
            requests: requests.clone(),
            health_started,
        };
        let router = Router::new()
            .route("/private/project/command", post(command))
            .with_state(fixture);
        let served = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut endpoint = DaemonEndpoint::new(
            scope,
            std::process::id(),
            address.to_string(),
            "recovery-fixture".into(),
            env!("CARGO_PKG_VERSION").into(),
        );
        endpoint.project_transport_url = Some(format!("http://{address}"));
        (endpoint, served, requests)
    }

    fn client_with_dead_endpoint(endpoint: &DaemonEndpoint) -> ProjectRuntimeClient {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut stale = endpoint.clone();
        stale.project_transport_url = Some(format!("http://{}", listener.local_addr().unwrap()));
        drop(listener);
        let mut client = ProjectRuntimeClient::new(&stale).unwrap();
        client.identity = Some(crate::daemon_subscriber::ProjectOwnerIdentity {
            pid: std::process::id(),
            host_started_at: crate::process::host_process_start_time(std::process::id()).unwrap(),
            host_instance_id: "same-owner".into(),
        });
        client
    }

    #[test]
    fn dead_endpoint_is_reresolved_once_and_shared_with_client_clones() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (endpoint, served, requests) =
                recovery_endpoint(temp.path(), "same-owner", None).await;
            let client = client_with_dead_endpoint(&endpoint);
            let cloned = client.clone();
            gwt_core::daemon::persist_endpoint(
                &endpoint.scope.endpoint_path(&gwt_core::paths::gwt_home()),
                &endpoint,
            )
            .unwrap();
            for caller in [client, cloned] {
                assert_eq!(
                    caller
                        .command(OwnerCommand::ProviderUsageSnapshot { accounts: vec![] })
                        .await
                        .unwrap()["accepted"],
                    true
                );
            }
            assert_eq!(
                *requests.lock().unwrap(),
                ["health", "command", "command"],
                "clones share the authenticated route and deliver once each"
            );
            served.abort();
            assert!(served.await.unwrap_err().is_cancelled());
        });
    }

    #[test]
    fn endpoint_recovery_refuses_a_replacement_owner_without_resending_commands() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (endpoint, served, requests) =
                recovery_endpoint(temp.path(), "replacement-owner", None).await;
            let client = client_with_dead_endpoint(&endpoint);
            gwt_core::daemon::persist_endpoint(
                &endpoint.scope.endpoint_path(&gwt_core::paths::gwt_home()),
                &endpoint,
            )
            .unwrap();
            for caller in [client.clone(), client] {
                let error = caller
                    .command(OwnerCommand::ProviderUsageSnapshot { accounts: vec![] })
                    .await
                    .unwrap_err();
                assert!(error.contains("needs_human"), "{error}");
                assert!(error.contains("identity"), "{error}");
            }
            assert_eq!(
                *requests.lock().unwrap(),
                ["health"],
                "one recovery probe; no captured command reaches another owner"
            );
            let records = gwt_core::error_ledger::list_since(None).unwrap();
            assert_eq!(
                records
                    .iter()
                    .filter(|record| record.context.get("operation").map(String::as_str)
                        == Some("project_owner_endpoint"))
                    .count(),
                1,
                "one durable needs_human diagnosis across clones"
            );
            served.abort();
            assert!(served.await.unwrap_err().is_cancelled());
        });
    }

    #[test]
    fn cancelled_endpoint_recovery_cannot_retry_the_dead_route_from_a_clone() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let health_started = Arc::new(tokio::sync::Notify::new());
            let (endpoint, served, requests) = recovery_endpoint(temp.path(), "same-owner", Some(health_started.clone())).await;
            let client = client_with_dead_endpoint(&endpoint);
            let cloned = client.clone();
            gwt_core::daemon::persist_endpoint(
                &endpoint.scope.endpoint_path(&gwt_core::paths::gwt_home()), &endpoint,
            ).unwrap();
            let mut command = Box::pin(client.command(OwnerCommand::ProviderUsageSnapshot { accounts: vec![] }));
            tokio::select! {
                _ = health_started.notified() => {},
                result = &mut command => panic!("recovery finished before the held Health response: {result:?}"),
            }
            // The outer observation deadline drops this same future. Observe
            // arrival before cancellation instead of assuming wall-clock order.
            drop(command);
            for _ in 0..2 {
                let error = cloned.command(OwnerCommand::ProviderUsageSnapshot { accounts: vec![] }).await.unwrap_err();
                assert!(error.contains("needs_human") && error.contains("interrupted"), "{error}");
            }
            assert_eq!(*requests.lock().unwrap(), ["health"], "cancelled recovery must not restart or deliver a command");
            let records = gwt_core::error_ledger::list_since(None).unwrap();
            assert_eq!(records.iter().filter(|record| record.context.get("operation").map(String::as_str) == Some("project_owner_endpoint")).count(), 1);
            served.abort();
            assert!(served.await.unwrap_err().is_cancelled());
        });
    }

    #[test]
    fn sent_commands_are_not_replayed_after_http_body_or_timeout_errors() {
        use axum::{http::StatusCode, response::IntoResponse};
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for failure in ["http", "body", "timeout"] {
                let requests = Arc::new(AtomicUsize::new(0));
                let state = requests.clone();
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let router = Router::new().route(
                    "/private/project/command",
                    post(move || {
                        state.fetch_add(1, Ordering::SeqCst);
                        async move {
                            match failure {
                                "http" => (
                                    StatusCode::CONFLICT,
                                    Json(serde_json::json!({"error":"fixture refused"})),
                                )
                                    .into_response(),
                                "body" => "not JSON".into_response(),
                                _ => std::future::pending::<axum::response::Response>().await,
                            }
                        }
                    }),
                );
                let served =
                    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
                let mut endpoint = DaemonEndpoint::new(
                    RuntimeScope::from_project_root(temp.path(), RuntimeTarget::Host).unwrap(),
                    std::process::id(),
                    address.to_string(),
                    "recovery-fixture".into(),
                    env!("CARGO_PKG_VERSION").into(),
                );
                endpoint.project_transport_url = Some(format!("http://{address}"));
                let mut client = ProjectRuntimeClient::new(&endpoint).unwrap();
                client.identity = Some(crate::daemon_subscriber::ProjectOwnerIdentity {
                    pid: std::process::id(),
                    host_started_at: crate::process::host_process_start_time(std::process::id())
                        .unwrap(),
                    host_instance_id: "same-owner".into(),
                });
                gwt_core::daemon::persist_endpoint(
                    &endpoint.scope.endpoint_path(&gwt_core::paths::gwt_home()),
                    &endpoint,
                )
                .unwrap();
                assert!(
                    client
                        .command_with_timeout(
                            OwnerCommand::ProviderUsageSnapshot { accounts: vec![] },
                            if failure == "timeout" {
                                std::time::Duration::from_millis(100)
                            } else {
                                gwt_core::deadline_budget::HANG_GUARD
                            },
                        )
                        .await
                        .is_err(),
                    "{failure}"
                );
                let received = requests.load(Ordering::SeqCst);
                if failure == "timeout" {
                    // A saturated runner may expire before dispatch. Neither
                    // zero nor one observed request permits a second delivery.
                    assert!(received <= 1, "timeout: {received} deliveries");
                } else {
                    assert_eq!(
                        received, 1,
                        "{failure}: outcome is not permission to resend"
                    );
                }
                served.abort();
                assert!(served.await.unwrap_err().is_cancelled());
            }
        });
    }

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
