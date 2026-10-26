//! Exact handles for fixtures exercising the production project owner.

use std::sync::{atomic::AtomicBool, Arc, Mutex};

/// Captured only by the owner's mailbox after an exact pane/incarnation match.
/// These are the production resources, never a second GUI runtime or registry.
#[doc(hidden)]
pub struct OwnerRuntimeProbe {
    pub id: String,
    pub incarnation: u64,
    pub output_bytes: u64,
    pub pane: Arc<Mutex<gwt_terminal::Pane>>,
    pub pty: Arc<gwt_terminal::pty::PtyHandle>,
    pub current: Arc<AtomicBool>,
    pub policy: super::runtime_events::RuntimePolicyState,
    pub status: crate::WindowProcessStatus,
    pub detail: Option<String>,
    pub capacity_resume: Option<OwnerCapacityResumeProbe>,
    pub monitor_startup_at: Option<std::time::Instant>,
    pub readiness_watch: Option<super::launch_readiness::ContinueWorkReadinessWatch>,
    pub pending_fresh_execution: Option<super::launch_payload::PendingFreshExecutionLaunch>,
    pub readiness_inflight: bool,
    pub readiness_waiters: usize,
    pub writer: Option<Arc<super::agent_dispatch::PtyWriterEntry>>,
    pub issuer: crate::agent_capability::AgentCapabilityIssuer,
    pub grant: Option<crate::agent_capability::AgentCapabilityGrant>,
    pub(crate) capability_token: Option<String>,
    pub session: Option<gwt_agent::Session>,
    pub active_session: Option<super::launch_payload::ActiveAgentSession>,
}

#[doc(hidden)]
pub struct OwnerCapacityResumeProbe {
    pub due: Option<std::time::Instant>,
    pub attempted: bool,
}

impl OwnerRuntimeProbe {
    /// Build the real capability socket request without minting or translating
    /// a grant in the GUI fixture. The captured bearer stays out of Debug/logs.
    pub fn agent_pane_request(
        &self,
    ) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, String> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let token = self
            .capability_token
            .as_ref()
            .ok_or("the captured runtime has no capability")?;
        let mut request = self
            .issuer
            .agent_pane_websocket_url()
            .into_client_request()
            .map_err(|error| error.to_string())?;
        request.headers_mut().insert(
            tokio_tungstenite::tungstenite::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().map_err(
                |error: tokio_tungstenite::tungstenite::http::header::InvalidHeaderValue| {
                    error.to_string()
                },
            )?,
        );
        Ok(request)
    }
}

/// A fixture owning the existing production actor and both authenticated listeners.
/// Its executor keeps HTTP completions alive while a synchronous GUI fixture drains.
#[doc(hidden)]
pub struct ProjectOwnerHarness {
    executor: Option<tokio::runtime::Runtime>,
    transport: Option<OwnerHarnessTransport>,
    owner: super::owner::OwnerHandle,
    client: super::client::ProjectRuntimeClient,
    endpoint: gwt_core::daemon::DaemonEndpoint,
    endpoint_path: std::path::PathBuf,
}

enum OwnerHarnessTransport {
    Private(super::transport::ProjectTransportTask),
    Daemon(Box<crate::cli::daemon::server::OwnerDaemonFixture>),
}

impl OwnerHarnessTransport {
    async fn shutdown(self) {
        match self {
            Self::Private(transport) => transport.shutdown().await,
            Self::Daemon(daemon) => daemon
                .shutdown()
                .await
                .expect("canonical fixture daemon drain"),
        }
    }
}

#[doc(hidden)]
pub struct OwnerMonitorScanPauseGuard {
    gate: Arc<crate::cli::daemon::server::OwnerMonitorScanGate>,
    baseline: usize,
}

impl OwnerMonitorScanPauseGuard {
    pub async fn wait(&self) {
        self.gate.wait(self.baseline).await;
    }
    pub fn scans_started(&self) -> usize {
        self.gate.count().saturating_sub(self.baseline)
    }
    pub fn release(&self) {
        self.gate.release();
    }
}
impl Drop for OwnerMonitorScanPauseGuard {
    fn drop(&mut self) {
        self.gate.release();
    }
}

#[doc(hidden)]
pub struct OwnerAgentClient {
    id: String,
    queue: Arc<crate::project_transport::ClientQueue>,
    owner: super::owner::OwnerHandle,
}

#[doc(hidden)]
pub struct OwnerAgentReply {
    pub events: Vec<serde_json::Value>,
    pub close: Option<crate::project_transport::ClientCloseFrame>,
}

impl OwnerAgentClient {
    pub fn enqueue(
        &self,
        grant: crate::agent_capability::AgentCapabilityGrant,
        request: crate::agent_capability::AgentFrontendRequest,
    ) {
        self.owner
            .transport_event(crate::project_transport::TransportEvent::AgentFrontend {
                client_id: self.id.clone(),
                grant,
                request,
            });
    }

    pub async fn request(
        &self,
        grant: crate::agent_capability::AgentCapabilityGrant,
        request: crate::agent_capability::AgentFrontendRequest,
    ) -> Result<OwnerAgentReply, String> {
        self.enqueue(grant, request);
        self.owner.test_agent_dispatch_barrier(false).await?;
        self.drain()
    }

    pub fn drain(&self) -> Result<OwnerAgentReply, String> {
        let mut reply = OwnerAgentReply {
            events: Vec::new(),
            close: None,
        };
        while let Some(step) = self.queue.try_next() {
            match step {
                crate::project_transport::DrainStep::Message { payload, .. } => {
                    reply
                        .events
                        .push(serde_json::from_str(&payload).map_err(|error| error.to_string())?);
                }
                crate::project_transport::DrainStep::Closed(reason) => {
                    reply.close = reason;
                    break;
                }
            }
        }
        Ok(reply)
    }
}

impl ProjectOwnerHarness {
    pub async fn agent_client(&self, id: &str) -> Result<OwnerAgentClient, String> {
        Ok(OwnerAgentClient {
            id: id.into(),
            queue: self.register_agent_client(id).await?,
            owner: self.owner.clone(),
        })
    }
    pub async fn set_issue_client_factory(
        &self,
        factory: super::launch::RuntimeIssueClientFactory,
    ) -> Result<(), String> {
        self.owner.test_set_issue_client_factory(factory).await
    }
    pub async fn register_agent_client(
        &self,
        id: &str,
    ) -> Result<Arc<crate::project_transport::ClientQueue>, String> {
        self.owner.test_register_agent_client(id.into()).await
    }

    /// Wait for canonical actor/dispatch completion, rather than sleeping for
    /// a guessed delay after enqueueing an authenticated transport request.
    pub async fn agent_dispatch_barrier(&self) -> Result<(), String> {
        self.owner.test_agent_dispatch_barrier(false).await
    }
    /// Observe the existing close ACK/finalizer completion after the origin
    /// acceptance is dropped or its exact captured ticket is replayed.
    pub async fn settled_self_closes(&self) -> Result<(), String> {
        self.owner.test_agent_dispatch_barrier(true).await
    }

    pub fn commit_self_close(
        &self,
        ticket: crate::agent_capability::AgentSelfCloseCapabilityTicket,
    ) -> Result<(), String> {
        self.owner.test_commit_self_close(ticket)
    }

    pub async fn pane_event(
        &self,
        event: crate::pane_runtime::PaneRuntimeEvent,
    ) -> Result<(), String> {
        self.owner.test_pane_event(event)?;
        let reply = self
            .owner
            .request(super::events::OwnerRequest {
                project_key: self.endpoint.scope.repo_hash.clone(),
                command: super::events::OwnerCommand::Health,
            })
            .await;
        reply.map(|_| ())
    }

    pub async fn start(root: &std::path::Path) -> Result<Self, String> {
        Self::start_mode(root, false).await
    }

    pub async fn start_with_monitor(root: &std::path::Path) -> Result<Self, String> {
        Self::start_mode(root, true).await
    }

    async fn start_mode(root: &std::path::Path, with_monitor: bool) -> Result<Self, String> {
        let root = root.to_path_buf();
        let fixture_root = root.clone();
        let home = gwt_config::home::user_home().ok_or("fixture user home unavailable")?;
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let mut fixture = tokio::task::spawn_blocking(move || {
            let _dispatch = tracing::dispatcher::set_default(&dispatch);
            let _home = gwt_core::test_support::ScopedGwtHome::set(&home);
            Self::bind(&fixture_root, with_monitor)
        })
        .await
        .map_err(|error| error.to_string())??;
        fixture.client = super::client::ProjectRuntimeClient::from_current_project(
            &root,
            &gwt_core::paths::gwt_home(),
        )
        .await?;
        Ok(fixture)
    }

    fn bind(root: &std::path::Path, with_monitor: bool) -> Result<Self, String> {
        let scope = gwt_core::daemon::RuntimeScope::from_project_root(
            root,
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .map_err(|error| error.to_string())?;
        let endpoint_path = scope.endpoint_path(&gwt_core::paths::gwt_home());
        if endpoint_path.exists() {
            return Err("fixture project already has an owner endpoint".into());
        }
        let fixture_home = gwt_config::home::user_home().ok_or("fixture user home unavailable")?;
        let executor = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .on_thread_start(move || {
                // The dedicated fixture executor owns these threads until
                // shutdown; preserve its captured home across async callbacks.
                // Initialize the override's TLS dependencies before the TLS
                // slot owning its guard, so that guard drops before them.
                let home = gwt_core::test_support::ScopedGwtHome::set(&fixture_home);
                FIXTURE_HOME.with(|guard| *guard.borrow_mut() = Some(home));
            })
            .on_thread_stop(|| {
                FIXTURE_HOME.with(|guard| {
                    guard.borrow_mut().take();
                });
            })
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let mut endpoint = gwt_core::daemon::DaemonEndpoint::new(
            scope,
            std::process::id(),
            String::new(),
            uuid::Uuid::new_v4().to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        let (owner, transport) = if with_monitor {
            let _entered = executor.enter();
            let daemon =
                crate::cli::daemon::server::OwnerDaemonFixture::bind(root, &endpoint_path)?;
            endpoint = daemon.endpoint.clone();
            (
                daemon.owner.clone(),
                OwnerHarnessTransport::Daemon(Box::new(daemon)),
            )
        } else {
            let _entered = executor.enter();
            let bound = super::transport::BoundProjectTransport::bind(&endpoint)
                .map_err(|error| error.to_string())?;
            endpoint.bind = bound.private_url.trim_start_matches("http://").to_string();
            endpoint.project_transport_url = Some(bound.private_url.clone());
            let owner = bound.owner.clone();
            (
                owner,
                OwnerHarnessTransport::Private(bound.serve().map_err(|error| error.to_string())?),
            )
        };
        let client = super::client::ProjectRuntimeClient::new(&endpoint)?;
        let fixture = Self {
            executor: Some(executor),
            transport: Some(transport),
            owner,
            client,
            endpoint,
            endpoint_path,
        };
        if let Some(parent) = fixture.endpoint_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if !with_monitor {
            std::fs::write(
                &fixture.endpoint_path,
                serde_json::to_vec(&fixture.endpoint).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(fixture)
    }

    pub fn pane_id(&self, raw_id: &str) -> String {
        format!("{}::{raw_id}", self.endpoint.scope.repo_hash)
    }

    pub fn client(&self) -> super::client::ProjectRuntimeClient {
        self.client.clone()
    }

    pub fn transport_event(&self, event: crate::project_transport::TransportEvent) {
        self.owner.transport_event(event);
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<super::events::OwnerEvent> {
        self.owner.subscribe()
    }

    pub async fn command(&self, command: super::events::OwnerCommand) -> super::events::OwnerReply {
        self.client.command(command).await
    }

    pub async fn agent_switch_source_exited(
        &self,
        proof: crate::issue_monitor::IssueMonitorRuntimeStopRequest,
        pty: Arc<gwt_terminal::PtyHandle>,
    ) -> Result<(), String> {
        self.owner.test_agent_switch_source_exited(proof, pty)?;
        self.command(super::events::OwnerCommand::Health)
            .await
            .map(|_| ())
    }

    pub async fn probe(&self, id: &str, incarnation: u64) -> Result<OwnerRuntimeProbe, String> {
        self.owner.test_probe_runtime(id, incarnation).await
    }

    pub async fn observe_runtime_policy(
        &self,
        id: &str,
        incarnation: u64,
        wall: chrono::DateTime<chrono::Utc>,
        now: std::time::Instant,
    ) -> Result<(), String> {
        self.owner
            .test_observe_runtime_policy(id, incarnation, wall, now)
            .await
    }

    pub async fn tick_runtime_policy(&self, now: std::time::Instant) -> Result<(), String> {
        self.owner.test_tick_runtime_policy(now).await
    }

    pub async fn check_readiness_deadline(
        &self,
        id: &str,
        incarnation: u64,
        watch: super::launch_readiness::ContinueWorkReadinessWatch,
    ) -> Result<(), String> {
        self.owner
            .test_check_readiness_deadline(id, incarnation, watch)
            .await
    }

    pub async fn check_monitor_deadlines(&self, now: std::time::Instant) -> Result<(), String> {
        self.owner.test_check_monitor_deadlines(now).await
    }

    pub async fn expire_monitor_windowless_launches_at(&self, now: String) -> Result<(), String> {
        let Some(OwnerHarnessTransport::Daemon(daemon)) = self.transport.as_ref() else {
            return Err("canonical Monitor driver is required".into());
        };
        *daemon.scan.expire_at.lock().unwrap() = Some(now);
        let root = self.endpoint.scope.project_root.clone();
        let home = gwt_core::test_support::gwt_home_override();
        tokio::task::spawn_blocking(move || {
            let _home = home
                .as_ref()
                .map(gwt_core::test_support::ScopedGwtHome::set);
            crate::daemon_publisher::publish_durable_issue_monitor_control(
                &root,
                crate::runtime_daemon_events::issue_monitor_payload(
                    "control",
                    serde_json::json!({"scan_now":{}}),
                    std::process::id(),
                ),
            )
            .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())??;
        daemon.scan.wait_idle().await;
        Ok(())
    }

    pub fn monitor_snapshot(&self) -> Result<Option<crate::IssueMonitorWindowSnapshot>, String> {
        match self.transport.as_ref() {
            Some(OwnerHarnessTransport::Daemon(daemon)) => {
                Ok(daemon.monitor_snapshot.borrow().clone())
            }
            _ => Err("the canonical Monitor daemon fixture is required".into()),
        }
    }

    pub fn pause_next_monitor_scan(&self) -> Result<OwnerMonitorScanPauseGuard, String> {
        let Some(OwnerHarnessTransport::Daemon(daemon)) = self.transport.as_ref() else {
            return Err("fixture has no canonical Monitor driver".into());
        };
        let gate = daemon.scan.clone();
        let baseline = gate.pause();
        Ok(OwnerMonitorScanPauseGuard { gate, baseline })
    }

    pub async fn wait_monitor_idle(&self) -> Result<(), String> {
        let Some(OwnerHarnessTransport::Daemon(daemon)) = self.transport.as_ref() else {
            return Err("fixture has no canonical Monitor driver".into());
        };
        daemon.scan.wait_idle().await;
        Ok(())
    }

    pub async fn shutdown(mut self) {
        if let Some(transport) = self.transport.take() {
            transport.shutdown().await;
        }
        if let Some(executor) = self.executor.take() {
            executor.shutdown_background();
        }
        self.remove_exact_endpoint();
    }

    fn remove_exact_endpoint(&self) {
        let current = std::fs::read(&self.endpoint_path).ok().and_then(|bytes| {
            serde_json::from_slice::<gwt_core::daemon::DaemonEndpoint>(&bytes).ok()
        });
        if current.is_some_and(|endpoint| {
            endpoint.auth_token == self.endpoint.auth_token
                && endpoint.pid == self.endpoint.pid
                && endpoint.scope == self.endpoint.scope
        }) {
            let _ = std::fs::remove_file(&self.endpoint_path);
        }
    }
}

thread_local! {
    static FIXTURE_HOME: std::cell::RefCell<Option<gwt_core::test_support::ScopedGwtHome>> = const { std::cell::RefCell::new(None) };
}

impl Drop for ProjectOwnerHarness {
    fn drop(&mut self) {
        if let (Some(transport), Some(executor)) = (self.transport.take(), self.executor.take()) {
            // The captured fixture owns these exact resources. Joining outside
            // any caller's Tokio executor also keeps synchronous tests usable.
            let cleanup = std::thread::spawn(move || executor.block_on(transport.shutdown()));
            let _ = cleanup.join();
        }
        self.remove_exact_endpoint();
    }
}

/// Exact evidence captured by the real launch worker before mailbox publication.
#[derive(Clone)]
#[doc(hidden)]
pub struct OwnerLaunchEvidence {
    pub id: String,
    pub incarnation: u64,
    pub operation_id: String,
    pub session_id: Option<String>,
    pub current: Arc<AtomicBool>,
    pub session: Option<gwt_agent::Session>,
    pub pty: Option<Arc<gwt_terminal::pty::PtyHandle>>,
    pub issuer: crate::agent_capability::AgentCapabilityIssuer,
    pub grant: Option<crate::agent_capability::AgentCapabilityGrant>,
    pub pending_continue_work: Option<super::launch_payload::PendingContinueWork>,
    pub pending_fresh_execution: Option<super::launch_payload::PendingFreshExecutionLaunch>,
}

impl OwnerRuntimeProbe {
    pub fn current_capability_grant(
        &self,
    ) -> Option<crate::agent_capability::AgentCapabilityGrant> {
        self.capability_token
            .as_deref()
            .and_then(|token| self.issuer.grant_for_test(token))
    }

    /// Run the real CLI forwarding path with this runtime's captured bearer.
    /// Callers hold the process environment test lock for the whole fixture.
    pub fn with_agent_environment<T>(&self, operation: impl FnOnce() -> T) -> T {
        let session = self.session.as_ref().expect("captured agent Session");
        let token = self
            .capability_token
            .as_ref()
            .expect("captured agent bearer");
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &session.id);
        let _url = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
            self.issuer.hook_forward_url(),
        );
        let _token =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, token);
        let _runtime = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            gwt_agent::runtime_state_path(&gwt_core::paths::gwt_sessions_dir(), &session.id),
        );
        operation()
    }

    pub async fn workspace_update(
        &self,
        request: &crate::AgentWorkspaceUpdateRequest,
    ) -> Result<(reqwest::StatusCode, serde_json::Value), String> {
        let token = self
            .capability_token
            .as_ref()
            .ok_or("captured runtime has no capability")?;
        let url = reqwest::Url::parse(self.issuer.hook_forward_url())
            .map_err(|error| error.to_string())?
            .join("/internal/workspace-update")
            .map_err(|error| error.to_string())?;
        let response = reqwest::Client::new()
            .post(url)
            .bearer_auth(token)
            .json(request)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let status = response.status();
        let body = response.json().await.map_err(|error| error.to_string())?;
        Ok((status, body))
    }

    pub fn has_capability_bearer(
        &self,
        grant: &crate::agent_capability::AgentCapabilityGrant,
    ) -> bool {
        self.capability_token
            .as_deref()
            .is_some_and(|token| grant.matches_token(token))
    }

    /// Submit an observation through the actual authenticated hook listener.
    pub async fn runtime_hook(
        &self,
        event: &crate::RuntimeHookEvent,
    ) -> Result<reqwest::StatusCode, String> {
        let token = self
            .capability_token
            .as_ref()
            .ok_or("captured runtime has no capability")?;
        let response = reqwest::Client::new()
            .post(self.issuer.hook_forward_url())
            .bearer_auth(token)
            .json(event)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        Ok(response.status())
    }

    pub async fn execution_continue(
        &self,
        request: &crate::AgentExecutionContinuationRequest,
    ) -> Result<(reqwest::StatusCode, serde_json::Value), String> {
        let token = self
            .capability_token
            .as_ref()
            .ok_or("captured runtime has no capability")?;
        let url = reqwest::Url::parse(self.issuer.hook_forward_url())
            .map_err(|error| error.to_string())?
            .join("/internal/execution-continuation")
            .map_err(|error| error.to_string())?;
        let response = reqwest::Client::new()
            .post(url)
            .bearer_auth(token)
            .json(request)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let status = response.status();
        let body = response.json().await.map_err(|error| error.to_string())?;
        Ok((status, body))
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum OwnerLaunchStage {
    MaterializedBeforeInstall,
    #[default]
    PreparedBeforeMailbox,
}

#[doc(hidden)]
#[derive(Default)]
pub struct OwnerLaunchCheckpoint {
    pub(crate) stage: OwnerLaunchStage,
    state: Mutex<(Option<OwnerLaunchEvidence>, bool)>,
    arrived: tokio::sync::Notify,
    released: std::sync::Condvar,
}

impl OwnerLaunchCheckpoint {
    pub(crate) fn pause(&self, evidence: OwnerLaunchEvidence) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.0 = Some(evidence);
        self.arrived.notify_waiters();
        while !state.1 {
            state = self
                .released
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    pub async fn wait(&self) -> OwnerLaunchEvidence {
        loop {
            let arrived = self.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            if let Some(evidence) = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0
                .clone()
            {
                return evidence;
            }
            arrived.await;
        }
    }

    pub fn release(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .1 = true;
        self.released.notify_all();
    }
}

/// Fixture cleanup always releases its captured worker before draining the owner.
#[doc(hidden)]
pub struct OwnerLaunchPauseGuard(Arc<OwnerLaunchCheckpoint>);
impl OwnerLaunchPauseGuard {
    pub async fn wait(&self) -> OwnerLaunchEvidence {
        self.0.wait().await
    }
    pub fn release(&self) {
        self.0.release();
    }
}
impl Drop for OwnerLaunchPauseGuard {
    fn drop(&mut self) {
        self.0.release();
    }
}

impl ProjectOwnerHarness {
    /// Exercise a legacy durable Prepared receipt through the real launch and
    /// readiness finalizer. This in-process seam adds no transport authority.
    pub async fn launch_prepared_continuation(
        &self,
        config: Box<gwt_agent::LaunchConfig>,
        pending: Box<super::launch_payload::PendingContinueWork>,
    ) -> Result<serde_json::Value, String> {
        self.owner
            .test_launch_prepared_continuation(config, pending)
            .await
    }

    pub async fn disconnect_preparing_mailbox(&self) -> Result<(), String> {
        self.owner.test_disconnect_preparing_mailbox().await
    }

    pub async fn pause_next_agent_launch(&self) -> Result<OwnerLaunchPauseGuard, String> {
        self.pause_next_agent_launch_at(OwnerLaunchStage::PreparedBeforeMailbox)
            .await
    }

    pub async fn pause_next_agent_launch_at(
        &self,
        stage: OwnerLaunchStage,
    ) -> Result<OwnerLaunchPauseGuard, String> {
        let checkpoint = Arc::new(OwnerLaunchCheckpoint {
            stage,
            ..Default::default()
        });
        self.owner
            .test_pause_next_agent_launch(checkpoint.clone())
            .await?;
        Ok(OwnerLaunchPauseGuard(checkpoint))
    }
}

/// Fixture-selected gate executable; production resolution is unchanged.
#[doc(hidden)]
#[derive(Clone)]
pub struct OwnerTestGate {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
}

impl ProjectOwnerHarness {
    pub async fn set_pty_start_gate(
        &self,
        program: std::path::PathBuf,
        args: Vec<String>,
    ) -> Result<(), String> {
        self.owner
            .test_set_pty_start_gate(OwnerTestGate { program, args })
            .await
    }
}

/// Shared entry for existing libtest-hosted real PTY start gates.
#[doc(hidden)]
pub fn run_pty_start_gate_helper() {
    if std::env::var_os("GWT_INTERNAL_PTY_GATE_ENDPOINT").is_none() {
        return;
    }
    let outcome = gwt_terminal::pty::run_start_gate_from_env();
    // Issue #4628: a released helper execs its target, so it only returns
    // here on an aborted or failed gate — exactly when the owner SIGKILLs the
    // group without waiting (#3705). Exiting through libtest would run the
    // coverage runtime's exit handler, and a kill in the middle of that write
    // leaves a truncated raw profile that fails the whole coverage merge.
    #[cfg(unix)]
    {
        if let Err(error) = &outcome {
            eprintln!("test PTY start gate failed: {error}");
        }
        // SAFETY: `_exit` only ends this helper process; skipping exit
        // handlers is the point.
        unsafe { libc::_exit(if matches!(outcome, Ok(0)) { 0 } else { 1 }) }
    }
    #[cfg(not(unix))]
    {
        let exit_code = outcome.expect("run test PTY start gate");
        assert_eq!(exit_code, 0, "released PTY target failed");
    }
}

#[cfg(test)]
#[test]
fn pty_start_gate_helper() {
    run_pty_start_gate_helper();
}

/// One-shot observer at the existing capacity worker's pre-revalidation boundary.
#[doc(hidden)]
#[derive(Default)]
pub struct OwnerPolicyRetryCheckpoint {
    state: Mutex<(bool, bool, bool)>,
    changed: tokio::sync::Notify,
    released: std::sync::Condvar,
}
impl OwnerPolicyRetryCheckpoint {
    pub(crate) fn pause(self: &Arc<Self>) -> OwnerPolicyRetryWorker {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_waiters();
        while !state.1 {
            state = self.released.wait(state).unwrap();
        }
        OwnerPolicyRetryWorker(self.clone())
    }
    async fn observe(&self, finished: bool) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let state = *self.state.lock().unwrap();
            if if finished { state.2 } else { state.0 } {
                return;
            }
            changed.await;
        }
    }
    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.released.notify_all();
    }
}
pub(crate) struct OwnerPolicyRetryWorker(Arc<OwnerPolicyRetryCheckpoint>);
impl Drop for OwnerPolicyRetryWorker {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().2 = true;
        self.0.changed.notify_waiters();
    }
}
#[doc(hidden)]
pub struct OwnerPolicyRetryPauseGuard(Arc<OwnerPolicyRetryCheckpoint>);
impl OwnerPolicyRetryPauseGuard {
    pub async fn wait(&self) {
        self.0.observe(false).await;
    }
    pub async fn finished(&self) {
        self.0.observe(true).await;
    }
    pub fn release(&self) {
        self.0.release();
    }
}
impl Drop for OwnerPolicyRetryPauseGuard {
    fn drop(&mut self) {
        self.0.release();
    }
}
impl ProjectOwnerHarness {
    pub async fn pause_next_capacity_retry(
        &self,
        id: &str,
        incarnation: u64,
    ) -> Result<OwnerPolicyRetryPauseGuard, String> {
        let checkpoint = Arc::new(OwnerPolicyRetryCheckpoint::default());
        self.owner
            .test_pause_capacity_retry(id.into(), incarnation, checkpoint.clone())
            .await?;
        Ok(OwnerPolicyRetryPauseGuard(checkpoint))
    }
}
