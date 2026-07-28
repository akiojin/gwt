use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::{atomic::AtomicU64, Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

use axum::{
    extract::{
        connect_info::ConnectInfo,
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, Request, State,
    },
    http::{
        header::{AUTHORIZATION, HOST, ORIGIN, USER_AGENT},
        HeaderMap, StatusCode,
    },
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use gwt::{
    AgentWorkTerminalizationRequest, AgentWorkspaceUpdateError, AgentWorkspaceUpdateErrorCode,
    AgentWorkspaceUpdateRequest, BackendEvent, FrontendEvent, HookForwardTarget, RuntimeHookEvent,
};
use gwt_terminal::PtyHandle;
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, net::TcpListener, runtime::Runtime, sync::oneshot};
use uuid::Uuid;

use crate::{
    embedded_web, AppEventProxy, AttachmentUploadStore, DispatchTarget, OutboundEvent,
    UploadedAttachment, UserEvent,
};

type PtyWriterRegistry = Arc<RwLock<HashMap<String, Arc<PtyHandle>>>>;

/// SPEC-2359 W-17 (FR-394/FR-395): per-client outbound queue limits.
///
/// `LOSSY_HIGH_WATER` caps droppable stream traffic (terminal output and
/// other `Streamed` / `EphemeralStatus` kinds); past it those entries are
/// dropped instead of disconnecting the client. `LOSSLESS_HARD_CAP` is the
/// disconnect of last resort for a client that stopped draining entirely.
/// `DRAIN_LOW_WATER` is the drain level at which panes whose output was
/// dropped get scheduled for snapshot self-repair (FR-396).
const LOSSY_HIGH_WATER: usize = 256;
const DRAIN_LOW_WATER: usize = 32;
const LOSSLESS_HARD_CAP: usize = 8192;
pub(super) const AGENT_STALE_BINDING_CLOSE: ClientCloseFrame = ClientCloseFrame {
    code: 1008,
    reason: "execution binding is no longer current",
};
pub(super) const AGENT_AUTHORITY_UNAVAILABLE_CLOSE: ClientCloseFrame = ClientCloseFrame {
    code: 1011,
    reason: "execution authority is unavailable",
};
/// Upper bound on the in-memory access log ring buffer. The canonical sink
/// for production is `tracing::info!(target: "gwt_access", ...)` which writes
/// to `~/.gwt/logs/<date>/`; this in-memory ring exists only so tests (and an
/// eventual operator-visible Live tab) can sample the most recent entries
/// without parsing log files. Older entries are evicted FIFO once the ring
/// reaches the cap. SPEC-1942 US-14 follow-up review: previous unbounded Vec
/// would grow without limit in long-running browser-server sessions.
const ACCESS_LOG_RING_CAPACITY: usize = 1024;

/// One captured HTTP / WebSocket access event. Emitted both as
/// `tracing::info!(target: "gwt_access", ...)` (or `debug!` for `/healthz`)
/// and into an in-memory [`AccessLogSink`] for test inspection.
///
/// SPEC-1942 FR-098: visibility for LAN-bound browser-server mode — operators need to see
/// where access comes from when running with `--bind` on a LAN-reachable
/// address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessLogRecord {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub peer: Option<String>,
    pub user_agent: Option<String>,
    pub elapsed_ms: u64,
}

/// In-memory ring of access log entries. Cloning yields a handle to the same
/// underlying buffer (Arc-wrapped) so the embedded server, middleware and
/// tests observe the same recordings. The ring is capped at
/// [`ACCESS_LOG_RING_CAPACITY`] entries; older records are evicted FIFO so
/// memory stays bounded under long-running browser-server sessions.
#[derive(Clone, Default)]
pub struct AccessLogSink {
    inner: Arc<Mutex<std::collections::VecDeque<AccessLogRecord>>>,
}

#[derive(Clone)]
struct AccessLogPolicy {
    sink: AccessLogSink,
    record_user_agent: bool,
}

impl AccessLogPolicy {
    fn browser(sink: AccessLogSink) -> Self {
        Self {
            sink,
            record_user_agent: true,
        }
    }

    fn agent(sink: AccessLogSink) -> Self {
        Self {
            sink,
            record_user_agent: false,
        }
    }
}

impl AccessLogSink {
    pub(crate) fn record(&self, rec: AccessLogRecord) {
        if let Ok(mut guard) = self.inner.lock() {
            if guard.len() == ACCESS_LOG_RING_CAPACITY {
                guard.pop_front();
            }
            guard.push_back(rec);
        }
    }

    /// Returns a snapshot copy of every recorded entry so callers do not have
    /// to hold the underlying mutex.
    #[cfg(test)]
    pub fn snapshot(&self) -> Vec<AccessLogRecord> {
        self.inner
            .lock()
            .map(|guard| guard.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// How one [`BackendEvent`] kind behaves when a client's outbound queue is
/// under pressure. Derived from `BACKEND_EVENT_POLICIES` (`protocol.rs`),
/// which is the single source of truth for the delivery contract
/// (SPEC-2359 W-17 FR-394).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueClass {
    /// Droppable stream (terminal output, ephemeral statuses). Dropped past
    /// `LOSSY_HIGH_WATER`; pane-scoped drops self-repair via snapshot.
    Lossy,
    /// Only the latest payload matters; replaces the queued entry in place.
    IdempotentLatest,
    /// Latest snapshot per (kind, pane) replaces the queued one — lossless,
    /// but a replay burst never stacks stale snapshots.
    SnapshotLatest,
    /// Must reach the client. Never dropped; the hard cap disconnects the
    /// client instead (last resort).
    Lossless,
}

fn queue_class_for_kind(kind: &str) -> QueueClass {
    use gwt::protocol::BackendEventDeliveryClass as Delivery;
    match gwt::protocol::backend_event_policy(kind) {
        Some(policy) => match policy.delivery {
            Delivery::Streamed | Delivery::EphemeralStatus | Delivery::BestEffortDaemon => {
                QueueClass::Lossy
            }
            Delivery::IdempotentLatest => QueueClass::IdempotentLatest,
            Delivery::Snapshot => QueueClass::SnapshotLatest,
            Delivery::Error => QueueClass::Lossless,
        },
        // Kinds missing from the policy table must never be silently
        // droppable — fail toward guaranteed delivery.
        None => QueueClass::Lossless,
    }
}

/// One backend event serialized once and shared across every client queue.
struct PreparedOutbound {
    payload: String,
    kind: &'static str,
    pane_id: Option<String>,
    class: QueueClass,
}

fn prepare_outbound(event: &gwt::BackendEvent) -> PreparedOutbound {
    let kind = event.event_kind();
    let pane_id = match event {
        gwt::BackendEvent::TerminalOutput { id, .. }
        | gwt::BackendEvent::TerminalSnapshot { id, .. } => Some(id.clone()),
        _ => None,
    };
    PreparedOutbound {
        payload: serde_json::to_string(event).expect("backend event json"),
        kind,
        pane_id,
        class: queue_class_for_kind(kind),
    }
}

struct QueuedOutbound {
    payload: String,
    kind: &'static str,
    pane_id: Option<String>,
}

#[derive(Default)]
struct ClientQueueState {
    entries: std::collections::VecDeque<QueuedOutbound>,
    dirty_panes: std::collections::HashSet<String>,
    dropped_lossy: u64,
    dead: bool,
    close_frame: Option<ClientCloseFrame>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ClientCloseFrame {
    pub(super) code: u16,
    pub(super) reason: &'static str,
}

/// One step handed to the per-client drain loop in [`client_session`].
pub(super) enum DrainStep {
    Message {
        payload: String,
        /// Panes whose streamed output was dropped while the queue was
        /// saturated; the session loop must request snapshot re-sends for
        /// them (SPEC-2359 W-17 FR-396).
        repair_panes: Vec<String>,
    },
    Closed(Option<ClientCloseFrame>),
}

/// SPEC-2359 W-17 (FR-394/FR-395): per-client outbound queue that enforces
/// the `BACKEND_EVENT_POLICIES` delivery contract. Replaces the former
/// bounded mpsc channel whose overflow disconnected the client — under an
/// agent-startup output flood that evicted the very client that initiated
/// the launch and lost its lossless replies.
#[derive(Default)]
pub(super) struct ClientQueue {
    state: Mutex<ClientQueueState>,
    notify: tokio::sync::Notify,
}

impl ClientQueue {
    /// Enqueue one prepared event. Returns `true` when the client crossed
    /// the lossless hard cap and must be unregistered by the caller.
    fn enqueue(&self, message: &PreparedOutbound) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.dead {
            return true;
        }
        // Snapshot-class kinds without an extracted pane identity (file
        // trees, resume acks, release notes) must not replace each other by
        // kind alone — different windows would clobber one another. They get
        // lossless append semantics instead.
        let effective_class = match message.class {
            QueueClass::SnapshotLatest if message.pane_id.is_none() => QueueClass::Lossless,
            other => other,
        };
        match effective_class {
            QueueClass::IdempotentLatest => {
                if let Some(entry) = state
                    .entries
                    .iter_mut()
                    .find(|entry| entry.kind == message.kind)
                {
                    entry.payload = message.payload.clone();
                } else {
                    state.entries.push_back(Self::queued(message));
                }
            }
            QueueClass::SnapshotLatest => {
                if let Some(entry) = state
                    .entries
                    .iter_mut()
                    .find(|entry| entry.kind == message.kind && entry.pane_id == message.pane_id)
                {
                    entry.payload = message.payload.clone();
                } else {
                    state.entries.push_back(Self::queued(message));
                }
            }
            QueueClass::Lossy => {
                if state.entries.len() >= LOSSY_HIGH_WATER {
                    state.dropped_lossy += 1;
                    if let Some(pane) = &message.pane_id {
                        state.dirty_panes.insert(pane.clone());
                    }
                    return false;
                }
                state.entries.push_back(Self::queued(message));
            }
            QueueClass::Lossless => {
                if state.entries.len() >= LOSSLESS_HARD_CAP {
                    state.dead = true;
                    drop(state);
                    self.notify.notify_one();
                    return true;
                }
                state.entries.push_back(Self::queued(message));
            }
        }
        drop(state);
        self.notify.notify_one();
        false
    }

    fn queued(message: &PreparedOutbound) -> QueuedOutbound {
        QueuedOutbound {
            payload: message.payload.clone(),
            kind: message.kind,
            pane_id: message.pane_id.clone(),
        }
    }

    /// Pop the next message without waiting. `None` means the queue is
    /// currently empty (but alive).
    pub(super) fn try_next(&self) -> Option<DrainStep> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.dead {
            return Some(DrainStep::Closed(state.close_frame));
        }
        let entry = state.entries.pop_front()?;
        let repair_panes = if state.entries.len() < DRAIN_LOW_WATER && !state.dirty_panes.is_empty()
        {
            state.dirty_panes.drain().collect()
        } else {
            Vec::new()
        };
        Some(DrainStep::Message {
            payload: entry.payload,
            repair_panes,
        })
    }

    /// Await the next drain step. Cancel-safe: a popped message is returned
    /// synchronously, never lost across an await point.
    pub(super) async fn next(&self) -> DrainStep {
        loop {
            if let Some(step) = self.try_next() {
                return step;
            }
            // `notify_one` stores a permit when no waiter is registered, so
            // an enqueue racing this gap completes the await immediately.
            self.notify.notified().await;
        }
    }

    fn close(&self) {
        self.close_with_frame(None);
    }

    fn close_with_frame(&self, close_frame: Option<ClientCloseFrame>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.dead = true;
        state.close_frame = close_frame;
        drop(state);
        self.notify.notify_one();
    }

    fn health_stats(&self) -> ClientHubHealthStats {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ClientHubHealthStats {
            client_count: 0,
            queued_entries: state.entries.len(),
            dirty_panes: state.dirty_panes.len(),
            dropped_lossy: state.dropped_lossy,
            dead_clients: usize::from(state.dead),
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .len()
    }

    #[cfg(test)]
    fn is_dead(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .dead
    }

    #[cfg(test)]
    fn dropped_lossy(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .dropped_lossy
    }

    /// Test-only convenience mirroring the old mpsc `try_recv`: pop the next
    /// queued payload, ignoring repair bookkeeping.
    #[cfg(test)]
    pub(crate) fn try_recv(&self) -> Option<String> {
        match self.try_next()? {
            DrainStep::Message { payload, .. } => Some(payload),
            DrainStep::Closed(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientHubHealthStats {
    pub client_count: usize,
    pub queued_entries: usize,
    pub dirty_panes: usize,
    pub dropped_lossy: u64,
    pub dead_clients: usize,
}

#[derive(Clone, Default)]
pub struct ClientHub {
    clients: Arc<Mutex<HashMap<String, ClientRegistration>>>,
}

#[derive(Clone)]
struct ClientRegistration {
    queue: Arc<ClientQueue>,
    receives_broadcasts: bool,
}

impl ClientHub {
    pub(super) fn register(&self, client_id: String) -> Arc<ClientQueue> {
        self.register_with_broadcasts(client_id, true)
    }

    fn register_pane(&self, client_id: String) -> Arc<ClientQueue> {
        self.register_with_broadcasts(client_id, false)
    }

    fn register_with_broadcasts(
        &self,
        client_id: String,
        receives_broadcasts: bool,
    ) -> Arc<ClientQueue> {
        let queue = Arc::new(ClientQueue::default());
        self.clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                client_id,
                ClientRegistration {
                    queue: queue.clone(),
                    receives_broadcasts,
                },
            );
        queue
    }

    pub(super) fn unregister(&self, client_id: &str) {
        self.unregister_with_close_frame(client_id, None);
    }

    pub(super) fn unregister_with_close_frame(
        &self,
        client_id: &str,
        close_frame: Option<ClientCloseFrame>,
    ) {
        let removed = self
            .clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(client_id);
        if let Some(registration) = removed {
            registration.queue.close_with_frame(close_frame);
        }
    }

    /// SPEC-2970 FR-007: whether any GUI client is currently connected. The
    /// usage poller skips work entirely when no one is watching.
    pub fn has_clients(&self) -> bool {
        !self
            .clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    /// SPEC-3107: lightweight queue pressure snapshot for runtime health.
    /// The registry lock is held only long enough to clone queue handles; each
    /// queue is sampled under its own mutex.
    pub fn health_stats(&self) -> ClientHubHealthStats {
        let snapshot: Vec<Arc<ClientQueue>> = {
            let clients = self
                .clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            clients
                .values()
                .map(|registration| registration.queue.clone())
                .collect()
        };

        let mut stats = ClientHubHealthStats {
            client_count: snapshot.len(),
            ..ClientHubHealthStats::default()
        };
        for queue in snapshot {
            let queue_stats = queue.health_stats();
            stats.queued_entries += queue_stats.queued_entries;
            stats.dirty_panes += queue_stats.dirty_panes;
            stats.dropped_lossy += queue_stats.dropped_lossy;
            stats.dead_clients += queue_stats.dead_clients;
        }
        stats
    }

    pub(super) fn dispatch(&self, events: Vec<OutboundEvent>) {
        // Snapshot queue handles under a short-lived lock so serialization
        // and per-client enqueue work happen outside the registry mutex. This
        // keeps register/unregister responsive even when the broadcast batch
        // is large or one client is slow to drain its queue.
        let snapshot: Vec<(String, Arc<ClientQueue>, bool)> = {
            let clients = self
                .clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            clients
                .iter()
                .map(|(id, registration)| {
                    (
                        id.clone(),
                        registration.queue.clone(),
                        registration.receives_broadcasts,
                    )
                })
                .collect()
        };

        let mut dead_clients: Vec<String> = Vec::new();
        for outbound in events {
            let prepared = prepare_outbound(&outbound.event);
            match outbound.target {
                DispatchTarget::Broadcast => {
                    for (client_id, queue, receives_broadcasts) in &snapshot {
                        if !receives_broadcasts {
                            continue;
                        }
                        if queue.enqueue(&prepared) {
                            dead_clients.push(client_id.clone());
                        }
                    }
                }
                DispatchTarget::Client(client_id) => {
                    if let Some((_, queue, _)) = snapshot.iter().find(|(id, _, _)| id == &client_id)
                    {
                        if queue.enqueue(&prepared) {
                            dead_clients.push(client_id);
                        }
                    }
                }
            }
        }

        if !dead_clients.is_empty() {
            dead_clients.sort();
            dead_clients.dedup();
            // SPEC-2359 W-17 (FR-395): queue pressure alone no longer
            // disconnects a client — only the lossless hard cap does, as the
            // last resort for a client that stopped draining entirely.
            tracing::warn!(
                target: "gwt::client_hub",
                lossless_hard_cap = LOSSLESS_HARD_CAP,
                dead_client_count = dead_clients.len(),
                dead_clients = ?dead_clients,
                "disconnecting websocket clients stuck past the lossless hard cap; reconnect will replay latest state"
            );
            let mut clients = self
                .clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for client_id in dead_clients {
                if let Some(registration) = clients.remove(&client_id) {
                    registration.queue.close();
                }
            }
        }
    }
}

#[derive(Clone)]
struct ServerState {
    proxy: AppEventProxy,
    clients: ClientHub,
    agent_capabilities: AgentCapabilityRegistry,
    host_instance_id: String,
    attachment_upload_token: String,
    attachment_uploads: AttachmentUploadStore,
    pty_writers: PtyWriterRegistry,
    // Held only so the in-process sink stays alive for the lifetime of the
    // server. Read directly through [`EmbeddedServer::access_log`] in tests.
    #[allow(dead_code)]
    access_log: AccessLogSink,
}

pub struct EmbeddedServer {
    url: String,
    bound_addr: SocketAddr,
    agent_capability_issuer: AgentCapabilityIssuer,
    shutdown_tx: Option<oneshot::Sender<()>>,
    agent_shutdown_tx: Option<oneshot::Sender<()>>,
    // Same rationale as `ServerState::access_log`: tests read it via the
    // `access_log()` accessor; production code (main bootstrap) does not yet
    // surface the sink to the UI.
    #[allow(dead_code)]
    access_log: AccessLogSink,
}

/// Server-side identity authenticated by an opaque agent capability.
///
/// Neither field is accepted as routing authority from an agent request: the
/// registry derives this principal when the capability is issued and keeps it
/// process-local for the lifetime of the embedded server.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AgentSessionPrincipal {
    canonical_project_root: PathBuf,
    session_id: String,
    execution_authority: AgentExecutionAuthority,
}

#[derive(Clone, PartialEq, Eq)]
enum AgentExecutionAuthority {
    Inspection,
    // Prepared issuance is consumed by the continuation coordinator in the
    // next W-24 slice; this slice establishes its observation-only boundary.
    #[allow(dead_code)]
    Prepared(Box<gwt_agent::SessionExecutionBinding>),
    Active(Box<gwt_agent::SessionExecutionBinding>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentExecutionAuthorityKind {
    Inspection,
    Prepared,
    Active,
}

/// One authenticated capability generation carried from the agent listener
/// to the tao event loop. Its custom `Debug` implementation prevents the
/// bearer or principal from leaking through `UserEvent` diagnostics.
#[derive(Clone)]
pub(crate) struct AgentCapabilityGrant {
    token: String,
    principal: AgentSessionPrincipal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentDurableAuthority {
    ObservationOnly,
    Current,
    Stale,
    Unavailable,
}

impl AgentCapabilityGrant {
    fn new(token: String, principal: AgentSessionPrincipal) -> Self {
        Self { token, principal }
    }

    pub(crate) fn principal(&self) -> &AgentSessionPrincipal {
        &self.principal
    }
}

impl std::fmt::Debug for AgentCapabilityGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentCapabilityGrant(<redacted>)")
    }
}

/// Narrow internal protocol accepted from a capability-authenticated agent.
/// Project and Session authority remain attached to the server-side
/// [`AgentSessionPrincipal`]; no path or Session claim is copied from the
/// untrusted WebSocket payload.
#[derive(Clone)]
pub(crate) enum AgentFrontendRequest {
    Ready,
    CloseWindow {
        id: String,
        request_id: Option<String>,
        responder: Option<AgentSelfCloseResponder>,
    },
    SendInput {
        text: String,
    },
}

impl std::fmt::Debug for AgentFrontendRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => formatter.write_str("AgentFrontendRequest::Ready"),
            Self::CloseWindow { .. } => {
                formatter.write_str("AgentFrontendRequest::CloseWindow(<redacted>)")
            }
            Self::SendInput { .. } => {
                formatter.write_str("AgentFrontendRequest::SendInput(<redacted>)")
            }
        }
    }
}

impl AgentFrontendRequest {
    pub(crate) fn mutates_host_state(&self) -> bool {
        matches!(self, Self::CloseWindow { .. } | Self::SendInput { .. })
    }

    pub(crate) fn requires_producing_authority(&self) -> bool {
        matches!(self, Self::SendInput { .. })
    }
}

impl AgentSessionPrincipal {
    fn new(project_root: &Path, session_id: &str) -> Result<Self, String> {
        Self::new_with_authority(
            project_root,
            session_id,
            AgentExecutionAuthority::Inspection,
        )
    }

    #[allow(dead_code)]
    fn new_prepared(
        project_root: &Path,
        session_id: &str,
        execution_binding: gwt_agent::SessionExecutionBinding,
    ) -> Result<Self, String> {
        Self::new_with_authority(
            project_root,
            session_id,
            AgentExecutionAuthority::Prepared(Box::new(execution_binding)),
        )
    }

    fn new_bound(
        project_root: &Path,
        session_id: &str,
        execution_binding: gwt_agent::SessionExecutionBinding,
    ) -> Result<Self, String> {
        Self::new_with_authority(
            project_root,
            session_id,
            AgentExecutionAuthority::Active(Box::new(execution_binding)),
        )
    }

    fn new_with_authority(
        project_root: &Path,
        session_id: &str,
        execution_authority: AgentExecutionAuthority,
    ) -> Result<Self, String> {
        if session_id.trim() != session_id
            || gwt_agent::validate_session_id_path_component(session_id).is_err()
        {
            return Err("agent capability session id must be non-empty and canonical".to_string());
        }
        if execution_authority.binding().is_some_and(|binding| {
            binding.schema_version != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
                || binding.session_id != session_id
                || binding.repo_hash.trim().is_empty()
                || !matches!(binding.owner_kind.as_str(), "spec" | "issue")
                || binding.identity.generation_id.trim().is_empty()
                || binding.identity.binding_id.trim().is_empty()
                || binding.identity.ledger_head_hash.trim().is_empty()
                || binding.capability_generation == 0
        }) {
            return Err(
                "agent capability execution binding must be canonical and match the Session"
                    .to_string(),
            );
        }

        let canonical_project_root = dunce::canonicalize(project_root)
            .map(|path| gwt_core::paths::normalize_windows_child_process_path(&path))
            .map_err(|_| "agent capability project scope must be an existing canonical root")?;

        Ok(Self {
            canonical_project_root,
            session_id: session_id.to_string(),
            execution_authority,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(project_root: &Path, session_id: &str) -> Result<Self, String> {
        Self::new(project_root, session_id)
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn canonical_project_root(&self) -> &Path {
        &self.canonical_project_root
    }

    #[allow(dead_code)]
    pub(crate) fn execution_binding(&self) -> Option<&gwt_agent::SessionExecutionBinding> {
        self.execution_authority.binding()
    }

    pub(crate) fn active_execution_binding(&self) -> Option<&gwt_agent::SessionExecutionBinding> {
        match &self.execution_authority {
            AgentExecutionAuthority::Active(binding) => Some(binding),
            AgentExecutionAuthority::Inspection | AgentExecutionAuthority::Prepared(_) => None,
        }
    }

    pub(crate) fn prepared_execution_binding(&self) -> Option<&gwt_agent::SessionExecutionBinding> {
        match &self.execution_authority {
            AgentExecutionAuthority::Prepared(binding) => Some(binding),
            AgentExecutionAuthority::Inspection | AgentExecutionAuthority::Active(_) => None,
        }
    }

    pub(crate) fn authorizes_producing_mutation(&self) -> bool {
        self.active_execution_binding().is_some()
    }

    pub(crate) fn execution_authority_kind(&self) -> AgentExecutionAuthorityKind {
        match &self.execution_authority {
            AgentExecutionAuthority::Inspection => AgentExecutionAuthorityKind::Inspection,
            AgentExecutionAuthority::Prepared(_) => AgentExecutionAuthorityKind::Prepared,
            AgentExecutionAuthority::Active(_) => AgentExecutionAuthorityKind::Active,
        }
    }

    /// Kept as the narrow project-observation check for the forthcoming
    /// workspace-update route; hook-live only needs the canonical root value.
    #[allow(dead_code)]
    pub(crate) fn authorizes_project_root(&self, project_root: &Path) -> bool {
        dunce::canonicalize(project_root)
            .map(|path| gwt_core::paths::normalize_windows_child_process_path(&path))
            .is_ok_and(|candidate| candidate == self.canonical_project_root)
    }
}

impl std::fmt::Debug for AgentSessionPrincipal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentSessionPrincipal")
            .field("canonical_project_root", &"<redacted>")
            .field("session_id", &"<redacted>")
            .field(
                "execution_authority",
                &match self.execution_authority_kind() {
                    AgentExecutionAuthorityKind::Inspection => "inspection",
                    AgentExecutionAuthorityKind::Prepared => "prepared",
                    AgentExecutionAuthorityKind::Active => "active",
                },
            )
            .finish()
    }
}

impl AgentExecutionAuthority {
    fn binding(&self) -> Option<&gwt_agent::SessionExecutionBinding> {
        match self {
            Self::Inspection => None,
            Self::Prepared(binding) | Self::Active(binding) => Some(binding),
        }
    }
}

fn durable_agent_execution_authority(principal: &AgentSessionPrincipal) -> AgentDurableAuthority {
    let Some(binding) = principal.active_execution_binding() else {
        return AgentDurableAuthority::ObservationOnly;
    };
    let session_path =
        gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", principal.session_id()));
    let session = match gwt_agent::Session::load(&session_path) {
        Ok(session) => session,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return AgentDurableAuthority::Stale;
        }
        Err(_) => return AgentDurableAuthority::Unavailable,
    };
    if session.execution_binding.as_ref() != Some(binding) {
        return AgentDurableAuthority::Stale;
    }
    let owner_kind = match binding.owner_kind.as_str() {
        "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => return AgentDurableAuthority::Stale,
    };
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: binding.owner_number,
    };
    match gwt::cli::execution_state::current_active_execution_binding_matches(
        &session.worktree_path,
        owner,
        principal.session_id(),
        &binding.identity,
    ) {
        Ok(true) => {}
        Ok(false) => return AgentDurableAuthority::Stale,
        Err(_) => return AgentDurableAuthority::Unavailable,
    }

    let probe_id = Uuid::new_v4().to_string();
    let request = gwt::AgentExecutionBindingProbeRequest {
        schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
        operation_id: format!("pane-dispatch-{probe_id}"),
        nonce: format!("pane-nonce-{probe_id}"),
    };
    match gwt::probe_authenticated_execution_binding(
        principal.canonical_project_root(),
        principal.session_id(),
        binding,
        &format!("pane-host-{probe_id}"),
        request,
    ) {
        Ok(_) => AgentDurableAuthority::Current,
        Err(error) if error.code == AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch => {
            AgentDurableAuthority::Stale
        }
        Err(_) => AgentDurableAuthority::Unavailable,
    }
}

async fn durable_agent_execution_authority_async(
    principal: AgentSessionPrincipal,
) -> AgentDurableAuthority {
    if !principal.authorizes_producing_mutation() {
        return AgentDurableAuthority::ObservationOnly;
    }
    tokio::task::spawn_blocking(move || durable_agent_execution_authority(&principal))
        .await
        .unwrap_or(AgentDurableAuthority::Unavailable)
}

async fn durable_agent_execution_authority_with_lease_async(
    principal: AgentSessionPrincipal,
) -> AgentDurableAuthority {
    let Some(binding) = principal.active_execution_binding().cloned() else {
        return AgentDurableAuthority::ObservationOnly;
    };
    tokio::task::spawn_blocking(move || {
        let authority = durable_agent_execution_authority(&principal);
        if authority != AgentDurableAuthority::Current {
            return authority;
        }
        match gwt::cli::execution_state::with_current_active_execution_binding_lease(
            &gwt_core::paths::gwt_sessions_dir(),
            &binding,
            || (),
        ) {
            Ok(Some(())) => AgentDurableAuthority::Current,
            Ok(None) => AgentDurableAuthority::Stale,
            Err(_) => AgentDurableAuthority::Unavailable,
        }
    })
    .await
    .unwrap_or(AgentDurableAuthority::Unavailable)
}

#[derive(Default)]
struct AgentCapabilityRegistryState {
    principals_by_token: HashMap<String, AgentSessionPrincipal>,
    token_by_project_session: HashMap<(PathBuf, String), String>,
    closing_by_ticket: HashMap<String, ClosingAgentCapability>,
    closing_ticket_by_project_session: HashMap<(PathBuf, String), String>,
}

struct ClosingAgentCapability {
    token: String,
    principal: AgentSessionPrincipal,
    revoked: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AgentSelfCloseCapabilityTicket {
    id: String,
}

impl std::fmt::Debug for AgentSelfCloseCapabilityTicket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentSelfCloseCapabilityTicket(<redacted>)")
    }
}

impl AgentSelfCloseCapabilityTicket {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
}

/// One direct, origin-socket response channel for a correlated agent
/// self-close. It is deliberately absent from the shared [`ClientHub`], so an
/// acceptance can neither be broadcast nor replayed to another connection.
#[derive(Clone)]
pub(crate) struct AgentSelfCloseResponder {
    sender: Arc<Mutex<Option<oneshot::Sender<AgentSelfCloseDirectAcceptance>>>>,
}

impl std::fmt::Debug for AgentSelfCloseResponder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentSelfCloseResponder(<redacted>)")
    }
}

impl AgentSelfCloseResponder {
    pub(crate) fn channel() -> (Self, oneshot::Receiver<AgentSelfCloseDirectAcceptance>) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                sender: Arc::new(Mutex::new(Some(sender))),
            },
            receiver,
        )
    }

    pub(crate) fn send(
        &self,
        acceptance: AgentSelfCloseDirectAcceptance,
    ) -> Result<(), AgentSelfCloseDirectAcceptance> {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(sender) = sender else {
            return Err(acceptance);
        };
        sender.send(acceptance)
    }
}

/// Accepted self-close state owned by the origin WebSocket task.
///
/// Once this value reaches the direct-response channel, every exit path must
/// commit the captured close. Keeping the finalizer in `Drop` covers socket
/// failure, timeout, disconnect, and async task cancellation without exposing
/// the internal capability ticket on the wire.
pub(crate) struct AgentSelfCloseDirectAcceptance {
    request_id: String,
    window_id: String,
    ticket: Option<AgentSelfCloseCapabilityTicket>,
    proxy: AppEventProxy,
}

impl std::fmt::Debug for AgentSelfCloseDirectAcceptance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentSelfCloseDirectAcceptance(<redacted>)")
    }
}

impl AgentSelfCloseDirectAcceptance {
    pub(crate) fn new(
        request_id: String,
        window_id: String,
        ticket: AgentSelfCloseCapabilityTicket,
        proxy: AppEventProxy,
    ) -> Self {
        Self {
            request_id,
            window_id,
            ticket: Some(ticket),
            proxy,
        }
    }

    fn wire_payload(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&BackendEvent::PaneCloseAccepted {
            request_id: self.request_id.clone(),
            window_id: self.window_id.clone(),
        })
    }

    pub(crate) fn disarm(mut self) -> AgentSelfCloseCapabilityTicket {
        self.ticket
            .take()
            .expect("self-close acceptance ticket is armed")
    }
}

impl Drop for AgentSelfCloseDirectAcceptance {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            self.proxy.send(UserEvent::CommitAgentSelfClose { ticket });
        }
    }
}

/// Process-local map from opaque bearer capabilities to immutable Session
/// principals. A capability never persists to disk and its bearer is the only
/// identity material that crosses into an agent process or container.
#[derive(Clone, Default)]
struct AgentCapabilityRegistry {
    inner: Arc<RwLock<AgentCapabilityRegistryState>>,
}

impl AgentCapabilityRegistry {
    fn preflight_issue(&self, project_root: &Path, session_id: &str) -> Result<(), String> {
        let principal = AgentSessionPrincipal::new(project_root, session_id)?;
        let principal_key = (
            principal.canonical_project_root().to_path_buf(),
            principal.session_id().to_string(),
        );
        let state = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .closing_ticket_by_project_session
            .contains_key(&principal_key)
        {
            return Err("agent capability is closing; retry after pane teardown".to_string());
        }
        Ok(())
    }

    fn issue(&self, project_root: &Path, session_id: &str) -> Result<String, String> {
        let principal = AgentSessionPrincipal::new(project_root, session_id)?;
        self.issue_principal(principal)
    }

    fn issue_bound(
        &self,
        project_root: &Path,
        session_id: &str,
        execution_binding: gwt_agent::SessionExecutionBinding,
    ) -> Result<String, String> {
        let principal =
            AgentSessionPrincipal::new_bound(project_root, session_id, execution_binding)?;
        self.issue_principal(principal)
    }

    #[allow(dead_code)]
    fn issue_prepared(
        &self,
        project_root: &Path,
        session_id: &str,
        execution_binding: gwt_agent::SessionExecutionBinding,
    ) -> Result<String, String> {
        let principal =
            AgentSessionPrincipal::new_prepared(project_root, session_id, execution_binding)?;
        self.issue_principal(principal)
    }

    fn issue_principal(&self, principal: AgentSessionPrincipal) -> Result<String, String> {
        let principal_key = (
            principal.canonical_project_root().to_path_buf(),
            principal.session_id().to_string(),
        );

        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .closing_ticket_by_project_session
            .contains_key(&principal_key)
        {
            return Err("agent capability is closing; retry after pane teardown".to_string());
        }
        let token = loop {
            let candidate = format!("gwt_agent_{}{}", Uuid::new_v4(), Uuid::new_v4());
            let collides_with_closing = state
                .closing_by_ticket
                .values()
                .any(|closing| constant_time_token_eq(&candidate, &closing.token));
            if !state.principals_by_token.contains_key(&candidate) && !collides_with_closing {
                break candidate;
            }
        };

        // Rotation of a project + Session pair happens while one write lock is
        // held, so no observer can authenticate both the stale and new bearer.
        if let Some(previous) = state
            .token_by_project_session
            .insert(principal_key, token.clone())
        {
            state.principals_by_token.remove(&previous);
        }
        state.principals_by_token.insert(token.clone(), principal);
        Ok(token)
    }

    fn promote_prepared(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<(), String> {
        self.promote_to_active(token, expected_binding, false)
    }

    fn promote_inspection(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<(), String> {
        self.promote_to_active(token, expected_binding, true)
    }

    fn promote_to_active(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
        allow_inspection: bool,
    ) -> Result<(), String> {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let issued_token = state
            .principals_by_token
            .keys()
            .find(|candidate| constant_time_token_eq(token, candidate))
            .cloned()
            .ok_or_else(|| "agent capability is missing or no longer current".to_string())?;
        let principal = state
            .principals_by_token
            .get(&issued_token)
            .cloned()
            .ok_or_else(|| "agent capability is missing or no longer current".to_string())?;
        let principal_key = (
            principal.canonical_project_root().to_path_buf(),
            principal.session_id().to_string(),
        );
        if !state
            .token_by_project_session
            .get(&principal_key)
            .is_some_and(|current| constant_time_token_eq(&issued_token, current))
        {
            return Err("agent capability is missing or no longer current".to_string());
        }
        match &principal.execution_authority {
            AgentExecutionAuthority::Inspection if allow_inspection => {
                let mut promoted = principal;
                promoted.execution_authority =
                    AgentExecutionAuthority::Active(Box::new(expected_binding.clone()));
                state.principals_by_token.insert(issued_token, promoted);
                Ok(())
            }
            AgentExecutionAuthority::Prepared(binding) if binding.as_ref() == expected_binding => {
                let mut promoted = principal;
                promoted.execution_authority =
                    AgentExecutionAuthority::Active(Box::new(expected_binding.clone()));
                state.principals_by_token.insert(issued_token, promoted);
                Ok(())
            }
            AgentExecutionAuthority::Active(binding) if binding.as_ref() == expected_binding => {
                Ok(())
            }
            AgentExecutionAuthority::Inspection
            | AgentExecutionAuthority::Prepared(_)
            | AgentExecutionAuthority::Active(_) => Err(
                "agent capability execution authority cannot be promoted to the requested binding"
                    .to_string(),
            ),
        }
    }

    fn refresh_grant(&self, grant: &AgentCapabilityGrant) -> Option<AgentCapabilityGrant> {
        let state = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (issued_token, principal) =
            state
                .principals_by_token
                .iter()
                .find_map(|(candidate, principal)| {
                    constant_time_token_eq(&grant.token, candidate)
                        .then_some((candidate, principal))
                })?;
        if principal.canonical_project_root() != grant.principal.canonical_project_root()
            || principal.session_id() != grant.principal.session_id()
        {
            return None;
        }
        let principal_key = (
            principal.canonical_project_root().to_path_buf(),
            principal.session_id().to_string(),
        );
        if !state
            .token_by_project_session
            .get(&principal_key)
            .is_some_and(|current| constant_time_token_eq(issued_token, current))
        {
            return None;
        }
        Some(AgentCapabilityGrant::new(
            issued_token.clone(),
            principal.clone(),
        ))
    }

    fn begin_self_close_if_current(
        &self,
        grant: &AgentCapabilityGrant,
    ) -> Option<AgentSelfCloseCapabilityTicket> {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !Self::grant_is_current_in_state(&state, grant) {
            return None;
        }
        let issued_token = state
            .principals_by_token
            .keys()
            .find(|candidate| constant_time_token_eq(&grant.token, candidate))
            .cloned()?;
        let principal = state.principals_by_token.remove(&issued_token)?;
        let principal_key = (
            principal.canonical_project_root().to_path_buf(),
            principal.session_id().to_string(),
        );
        if state
            .token_by_project_session
            .get(&principal_key)
            .is_some_and(|current| constant_time_token_eq(&issued_token, current))
        {
            state.token_by_project_session.remove(&principal_key);
        }
        let ticket = loop {
            let candidate = format!("gwt_close_{}{}", Uuid::new_v4(), Uuid::new_v4());
            if !state.closing_by_ticket.contains_key(&candidate) {
                break AgentSelfCloseCapabilityTicket { id: candidate };
            }
        };
        state
            .closing_ticket_by_project_session
            .insert(principal_key, ticket.id.clone());
        state.closing_by_ticket.insert(
            ticket.id.clone(),
            ClosingAgentCapability {
                token: issued_token,
                principal,
                revoked: false,
            },
        );
        Some(ticket)
    }

    fn rollback_self_close(&self, ticket: &AgentSelfCloseCapabilityTicket) -> bool {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(closing) = state.closing_by_ticket.remove(ticket.id()) else {
            return false;
        };
        let principal_key = (
            closing.principal.canonical_project_root().to_path_buf(),
            closing.principal.session_id().to_string(),
        );
        if state
            .closing_ticket_by_project_session
            .get(&principal_key)
            .is_some_and(|current| current == ticket.id())
        {
            state
                .closing_ticket_by_project_session
                .remove(&principal_key);
        }
        if closing.revoked
            || state.token_by_project_session.contains_key(&principal_key)
            || state.principals_by_token.contains_key(&closing.token)
        {
            return false;
        }
        state
            .token_by_project_session
            .insert(principal_key, closing.token.clone());
        state
            .principals_by_token
            .insert(closing.token, closing.principal);
        true
    }

    fn finish_self_close(&self, ticket: &AgentSelfCloseCapabilityTicket) -> bool {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(closing) = state.closing_by_ticket.remove(ticket.id()) else {
            return false;
        };
        let principal_key = (
            closing.principal.canonical_project_root().to_path_buf(),
            closing.principal.session_id().to_string(),
        );
        if state
            .closing_ticket_by_project_session
            .get(&principal_key)
            .is_some_and(|current| current == ticket.id())
        {
            state
                .closing_ticket_by_project_session
                .remove(&principal_key);
        }
        true
    }

    fn authenticate(&self, token: &str) -> Option<AgentSessionPrincipal> {
        let state = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut authenticated = None;
        for (candidate, principal) in &state.principals_by_token {
            if constant_time_token_eq(token, candidate) {
                authenticated = Some(principal.clone());
            }
        }
        authenticated
    }

    /// Run one non-blocking dispatch only while `token` is still the current
    /// grant for `expected_principal`.
    ///
    /// The registry read lock stays held through the callback. Rotation or
    /// revocation therefore linearizes either before this check (zero
    /// dispatch) or after the already-authorized enqueue.
    fn dispatch_if_current(&self, grant: &AgentCapabilityGrant, dispatch: impl FnOnce()) -> bool {
        self.with_current_grant(grant, dispatch).is_some()
    }

    fn with_current_grant<T>(
        &self,
        grant: &AgentCapabilityGrant,
        dispatch: impl FnOnce() -> T,
    ) -> Option<T> {
        let state = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !Self::grant_is_current_in_state(&state, grant) {
            return None;
        }

        Some(dispatch())
    }

    fn grant_is_current(&self, grant: &AgentCapabilityGrant) -> bool {
        let state = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::grant_is_current_in_state(&state, grant)
    }

    fn grant_is_current_in_state(
        state: &AgentCapabilityRegistryState,
        grant: &AgentCapabilityGrant,
    ) -> bool {
        let authenticated = state
            .principals_by_token
            .iter()
            .find_map(|(candidate, principal)| {
                constant_time_token_eq(&grant.token, candidate).then_some(principal)
            });
        if authenticated != Some(&grant.principal) {
            return false;
        }
        let principal_key = (
            grant.principal.canonical_project_root().to_path_buf(),
            grant.principal.session_id().to_string(),
        );
        state
            .token_by_project_session
            .get(&principal_key)
            .is_some_and(|current| constant_time_token_eq(&grant.token, current))
    }

    /// Revoke one issue-time opaque token without consulting the filesystem.
    ///
    /// The project+Session reverse index is removed only when it still points
    /// at this exact token, so cleanup for an older launch cannot revoke a
    /// rotated replacement.
    fn revoke_token(&self, token: &str) -> bool {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(issued_token) = state
            .principals_by_token
            .keys()
            .find(|candidate| constant_time_token_eq(token, candidate))
            .cloned()
        else {
            let closing = state
                .closing_by_ticket
                .values_mut()
                .find(|closing| constant_time_token_eq(token, &closing.token));
            let Some(closing) = closing else {
                return false;
            };
            let newly_revoked = !closing.revoked;
            closing.revoked = true;
            return newly_revoked;
        };
        let Some(principal) = state.principals_by_token.remove(&issued_token) else {
            return false;
        };
        let principal_key = (
            principal.canonical_project_root().to_path_buf(),
            principal.session_id().to_string(),
        );
        if state
            .token_by_project_session
            .get(&principal_key)
            .is_some_and(|current| constant_time_token_eq(&issued_token, current))
        {
            state.token_by_project_session.remove(&principal_key);
        }
        true
    }

    fn session_count(&self) -> usize {
        let state = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.token_by_project_session.len() + state.closing_ticket_by_project_session.len()
    }
}

/// In-process authority used by launch orchestration to mint one capability
/// for a canonical project + Session pair.
#[derive(Clone)]
pub(crate) struct AgentCapabilityIssuer {
    hook_forward_url: String,
    pane_websocket_url: String,
    agent_pane_websocket_url: String,
    registry: AgentCapabilityRegistry,
}

impl AgentCapabilityIssuer {
    fn new(
        hook_forward_url: String,
        pane_websocket_url: String,
        agent_pane_websocket_url: String,
        registry: AgentCapabilityRegistry,
    ) -> Self {
        Self {
            hook_forward_url,
            pane_websocket_url,
            agent_pane_websocket_url,
            registry,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        hook_forward_url: &str,
        pane_websocket_url: &str,
        agent_pane_websocket_url: &str,
    ) -> Self {
        Self::new(
            hook_forward_url.to_string(),
            pane_websocket_url.to_string(),
            agent_pane_websocket_url.to_string(),
            AgentCapabilityRegistry::default(),
        )
    }

    pub(crate) fn issue(
        &self,
        project_root: &Path,
        session_id: &str,
    ) -> Result<HookForwardTarget, String> {
        Ok(HookForwardTarget {
            url: self.hook_forward_url.clone(),
            token: self.registry.issue(project_root, session_id)?,
        })
    }

    pub(crate) fn preflight_issue(
        &self,
        project_root: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        self.registry.preflight_issue(project_root, session_id)
    }

    pub(crate) fn issue_bound(
        &self,
        project_root: &Path,
        session_id: &str,
        execution_binding: gwt_agent::SessionExecutionBinding,
    ) -> Result<HookForwardTarget, String> {
        Ok(HookForwardTarget {
            url: self.hook_forward_url.clone(),
            token: self
                .registry
                .issue_bound(project_root, session_id, execution_binding)?,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn issue_prepared(
        &self,
        project_root: &Path,
        session_id: &str,
        execution_binding: gwt_agent::SessionExecutionBinding,
    ) -> Result<HookForwardTarget, String> {
        Ok(HookForwardTarget {
            url: self.hook_forward_url.clone(),
            token: self
                .registry
                .issue_prepared(project_root, session_id, execution_binding)?,
        })
    }

    pub(crate) fn promote_prepared(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<(), String> {
        self.registry.promote_prepared(token, expected_binding)
    }

    pub(crate) fn promote_inspection(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<(), String> {
        self.registry.promote_inspection(token, expected_binding)
    }

    pub(crate) fn prepared_token_is_current(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> bool {
        let Some(principal) = self.registry.authenticate(token) else {
            return false;
        };
        let grant = AgentCapabilityGrant::new(token.to_string(), principal);
        self.registry.grant_is_current(&grant)
            && grant.principal().prepared_execution_binding() == Some(expected_binding)
    }

    pub(crate) fn active_token_is_current(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> bool {
        let Some(principal) = self.registry.authenticate(token) else {
            return false;
        };
        let grant = AgentCapabilityGrant::new(token.to_string(), principal);
        self.registry.grant_is_current(&grant)
            && grant.principal().active_execution_binding() == Some(expected_binding)
    }

    pub(crate) fn revoke_token(&self, token: &str) -> bool {
        self.registry.revoke_token(token)
    }

    pub(crate) fn grant_is_current(&self, grant: &AgentCapabilityGrant) -> bool {
        self.registry.grant_is_current(grant)
    }

    pub(crate) fn durable_authority(&self, grant: &AgentCapabilityGrant) -> AgentDurableAuthority {
        durable_agent_execution_authority(grant.principal())
    }

    pub(crate) fn with_current_grant<T>(
        &self,
        grant: &AgentCapabilityGrant,
        dispatch: impl FnOnce() -> T,
    ) -> Option<T> {
        self.registry.with_current_grant(grant, dispatch)
    }

    pub(crate) fn begin_self_close_if_current(
        &self,
        grant: &AgentCapabilityGrant,
    ) -> Option<AgentSelfCloseCapabilityTicket> {
        self.registry.begin_self_close_if_current(grant)
    }

    pub(crate) fn rollback_self_close(&self, ticket: &AgentSelfCloseCapabilityTicket) -> bool {
        self.registry.rollback_self_close(ticket)
    }

    pub(crate) fn finish_self_close(&self, ticket: &AgentSelfCloseCapabilityTicket) -> bool {
        self.registry.finish_self_close(ticket)
    }

    #[cfg(test)]
    pub(crate) fn authenticates_token(&self, token: &str) -> bool {
        self.registry.authenticate(token).is_some()
    }

    #[cfg(test)]
    pub(crate) fn grant_for_test(&self, token: &str) -> Option<AgentCapabilityGrant> {
        self.registry
            .authenticate(token)
            .map(|principal| AgentCapabilityGrant::new(token.to_string(), principal))
    }

    pub(crate) fn pane_websocket_url(&self) -> &str {
        &self.pane_websocket_url
    }

    pub(crate) fn hook_forward_url(&self) -> &str {
        &self.hook_forward_url
    }

    pub(crate) fn agent_pane_websocket_url(&self) -> &str {
        &self.agent_pane_websocket_url
    }
}

impl std::fmt::Debug for AgentCapabilityIssuer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentCapabilityIssuer")
            .field("hook_forward_url", &self.hook_forward_url)
            .field("pane_websocket_url", &self.pane_websocket_url)
            .field("agent_pane_websocket_url", &self.agent_pane_websocket_url)
            .field("registered_sessions", &self.registry.session_count())
            .finish()
    }
}

impl EmbeddedServer {
    /// Loopback (`127.0.0.1`) on an ephemeral port — the original GUI default.
    /// Kept as a thin shim so non-headless callers do not have to know about
    /// the bind/port surface introduced for SPEC-1942 US-14.
    #[cfg(test)]
    pub(super) fn start(
        runtime: &Runtime,
        proxy: AppEventProxy,
        clients: ClientHub,
        pty_writers: PtyWriterRegistry,
        attachment_uploads: AttachmentUploadStore,
    ) -> std::io::Result<Self> {
        Self::start_with_bind(
            runtime,
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            0,
            // 0 disables the dedicated fixed-port OAuth listener so parallel
            // tests never contend on a shared loopback port.
            0,
            proxy,
            clients,
            pty_writers,
            attachment_uploads,
        )
    }

    /// SPEC-1942 FR-095 / FR-098: bind the embedded server to a caller-chosen
    /// IP / port and install the access-log middleware. Used by the current
    /// browser-server route for both loopback defaults and operator-chosen
    /// `--bind` / `--port`.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_with_bind(
        runtime: &Runtime,
        bind: IpAddr,
        port: u16,
        oauth_redirect_port: u16,
        proxy: AppEventProxy,
        clients: ClientHub,
        pty_writers: PtyWriterRegistry,
        attachment_uploads: AttachmentUploadStore,
    ) -> std::io::Result<Self> {
        let listener = runtime.block_on(TcpListener::bind(SocketAddr::new(bind, port)))?;
        let listener = listener.into_std()?;
        Self::start_with_listener(
            runtime,
            listener,
            oauth_redirect_port,
            proxy,
            clients,
            pty_writers,
            attachment_uploads,
        )
    }

    /// Start serving from a listener that was bound and committed by the
    /// stable-port startup transaction.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_with_listener(
        runtime: &Runtime,
        listener: std::net::TcpListener,
        oauth_redirect_port: u16,
        proxy: AppEventProxy,
        clients: ClientHub,
        pty_writers: PtyWriterRegistry,
        attachment_uploads: AttachmentUploadStore,
    ) -> std::io::Result<Self> {
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        if addr.port() == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "embedded server listener reported bound port 0",
            ));
        }
        let listener = {
            let _runtime_guard = runtime.enter();
            TcpListener::from_std(listener)?
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let agent_listener = runtime.block_on(TcpListener::bind(SocketAddr::new(
            agent_bridge_bind_ip(),
            0,
        )))?;
        let agent_addr = agent_listener.local_addr()?;
        let (agent_shutdown_tx, agent_shutdown_rx) = oneshot::channel();
        let agent_capabilities = AgentCapabilityRegistry::default();
        let agent_capability_issuer = AgentCapabilityIssuer::new(
            format!("http://127.0.0.1:{}/internal/hook-live", agent_addr.port()),
            format!(
                "ws://{}:{}/ws",
                display_host(local_browser_client_ip(addr.ip())),
                addr.port()
            ),
            format!("ws://127.0.0.1:{}/internal/pane-ws", agent_addr.port()),
            agent_capabilities.clone(),
        );
        let attachment_upload_token = Uuid::new_v4().to_string();
        let host_instance_id = Uuid::new_v4().to_string();
        let access_log = AccessLogSink::default();
        let server_state = ServerState {
            proxy,
            clients,
            agent_capabilities,
            host_instance_id,
            attachment_upload_token,
            attachment_uploads,
            pty_writers,
            access_log: access_log.clone(),
        };

        // Agent-originated HTTP traffic is isolated from the browser surface.
        // This router is deliberately capability-only; future agent routes can
        // be added here and reuse the same authenticated principal boundary.
        let agent_app = agent_router(server_state.clone(), access_log.clone());

        // SPEC-3016: every embedded frontend asset route (entrypoints, root
        // JS modules, vendor JS/CSS, stylesheets, fonts) is registered from
        // the embedded_web manifest tables.
        let app = route_root_js_modules(route_static_assets(Router::new()))
            .route("/healthz", get(health_handler))
            // SPEC-2963 Phase 5: OAuth redirect target for remote Board provider
            // sign-in. Completes the flow against the process-global session store.
            .route("/oauth/callback", get(oauth_callback_handler))
            .route(
                "/internal/attachment-upload-token",
                get(attachment_upload_token_handler),
            )
            .route(
                "/internal/attachments/upload",
                post(attachment_upload_handler),
            )
            .route("/ws", get(websocket_handler))
            .with_state(server_state)
            .layer(middleware::from_fn_with_state(
                AccessLogPolicy::browser(access_log.clone()),
                access_log_middleware,
            ));

        // SPEC-2963 FR-005: dedicated fixed-port loopback OAuth callback
        // listener. The OAuth redirect_uri must be a stable, pre-registered URL
        // (`http://127.0.0.1:<oauth_redirect_port>/oauth/callback`), but the
        // main server uses an ephemeral / operator-chosen port. Bind the fixed
        // loopback port and serve the same router so `/oauth/callback` is
        // reachable there. Skipped when disabled (`0`, e.g. tests) or when the
        // main server already listens on that port (no double-bind).
        let oauth_listener = if oauth_redirect_port != 0 && oauth_redirect_port != addr.port() {
            match runtime.block_on(TcpListener::bind((
                IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                oauth_redirect_port,
            ))) {
                Ok(listener) => Some(listener),
                Err(error) => {
                    eprintln!(
                        "gwt: OAuth callback port {oauth_redirect_port} is unavailable \
                         ({error}); remote Board sign-in may fail until it is freed or \
                         changed in Settings."
                    );
                    None
                }
            }
        } else {
            None
        };

        if let Some(oauth_listener) = oauth_listener {
            let oauth_app = app.clone();
            runtime.spawn(async move {
                if let Err(error) = axum::serve(
                    oauth_listener,
                    oauth_app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                {
                    eprintln!("embedded OAuth callback server error: {error}");
                }
            });
        }

        runtime.spawn(async move {
            let server = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            if let Err(error) = server.await {
                eprintln!("embedded server error: {error}");
            }
        });

        runtime.spawn(async move {
            let server = axum::serve(
                agent_listener,
                agent_app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = agent_shutdown_rx.await;
            });
            if let Err(error) = server.await {
                eprintln!("embedded agent bridge error: {error}");
            }
        });

        Ok(Self {
            url: format!("http://{}:{}/", display_host(addr.ip()), addr.port()),
            bound_addr: addr,
            agent_capability_issuer,
            shutdown_tx: Some(shutdown_tx),
            agent_shutdown_tx: Some(agent_shutdown_tx),
            access_log,
        })
    }

    /// Returns the in-memory sink that captures every access log record.
    /// Used by tests and (eventually) by an operator-visible Live tab.
    #[cfg(test)]
    pub(super) fn access_log(&self) -> &AccessLogSink {
        &self.access_log
    }

    pub(super) fn url(&self) -> &str {
        &self.url
    }

    pub(super) fn bound_port(&self) -> NonZeroU16 {
        NonZeroU16::new(self.bound_addr.port())
            .expect("EmbeddedServer validates its bound port before construction")
    }

    pub(super) fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(tx) = self.agent_shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    pub(crate) fn agent_capability_issuer(&self) -> AgentCapabilityIssuer {
        self.agent_capability_issuer.clone()
    }

    #[cfg(test)]
    pub(super) fn hook_forward_target(&self) -> HookForwardTarget {
        let project_root = std::env::current_dir().expect("embedded-server test project root");
        self.agent_capability_issuer
            .issue(&project_root, "session-1")
            .expect("canonical embedded-server test session")
    }
}

fn agent_router(state: ServerState, access_log: AccessLogSink) -> Router {
    Router::new()
        .route("/internal/hook-live", post(hook_live_handler))
        .route("/internal/pane-ws", get(agent_pane_websocket_handler))
        .route(
            "/internal/execution-binding-probe",
            post(execution_binding_probe_handler),
        )
        .route("/internal/workspace-update", post(workspace_update_handler))
        .route(
            "/internal/work-terminalization",
            post(work_terminalization_handler),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            AccessLogPolicy::agent(access_log),
            access_log_middleware,
        ))
}

fn route_root_js_modules(mut router: Router<ServerState>) -> Router<ServerState> {
    for asset in embedded_web::root_js_module_assets() {
        let asset = *asset;
        router = router.route(
            asset.path,
            get(move || async move { embedded_web::root_js_module_response(asset) }),
        );
    }
    router
}

/// Registers one GET route per [`embedded_web::StaticAsset`] manifest entry
/// (SPEC-3016: the manifest is the routing source of truth).
fn route_static_assets(mut router: Router<ServerState>) -> Router<ServerState> {
    for asset in embedded_web::static_assets() {
        router = router.route(
            asset.route,
            get(move || async move { embedded_web::static_asset_response(asset) }),
        );
    }
    router
}

pub async fn health_handler() -> &'static str {
    "ok"
}

/// Query parameters on the OAuth redirect (SPEC-2963 Phase 5).
#[derive(Debug, Deserialize)]
struct OAuthCallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

fn oauth_result_page(title: &str, message: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body style=\"font-family:system-ui,sans-serif;padding:2.5rem;max-width:34rem;margin:auto\">\
         <h2>{title}</h2><p>{message}</p>\
         <p style=\"color:#666\">You can close this tab and return to gwt.</p></body></html>"
    ))
}

/// OAuth redirect handler: completes the remote Board provider sign-in against
/// the process-global session store. On success it broadcasts a refreshed
/// [`gwt::BackendEvent::BoardAuthStatus`] to every connected client so the
/// settings UI flips to "Signed in" without a manual Refresh (SPEC-2963
/// FR-012). The token exchange itself is self-contained (global session +
/// token store); only the broadcast needs the shared [`ServerState`].
async fn oauth_callback_handler(
    State(state): State<ServerState>,
    Query(params): Query<OAuthCallbackQuery>,
) -> Html<String> {
    if let Some(error) = params.error.as_deref().filter(|value| !value.is_empty()) {
        return oauth_result_page("Sign-in failed", error);
    }
    let (Some(code), Some(oauth_state)) = (params.code, params.state) else {
        return oauth_result_page("Sign-in failed", "Missing authorization code or state.");
    };
    // The token exchange is blocking (reqwest); run it off the async worker.
    let outcome = tokio::task::spawn_blocking(move || {
        let poster = gwt::board_remote::http::ReqwestHttpClient::new();
        gwt::board_remote::oauth_session::complete_callback(
            gwt::board_remote::signin::sessions(),
            &code,
            &oauth_state,
            &poster,
            &gwt::board_remote::token_store::default_dir(),
            chrono::Utc::now(),
        )
    })
    .await;
    match outcome {
        Ok(Ok(provider_key)) => {
            // Push the refreshed auth/config view to all connected gwt clients
            // so the Settings panel reflects the new sign-in immediately.
            state.clients.dispatch(vec![OutboundEvent::broadcast(
                gwt::system_settings::board_auth_status_event(Some(format!(
                    "Signed in to {provider_key}."
                ))),
            )]);
            oauth_result_page(
                "Signed in",
                &format!("Connected the {provider_key} Board provider."),
            )
        }
        Ok(Err(reason)) => oauth_result_page("Sign-in failed", &reason),
        Err(_) => oauth_result_page("Sign-in failed", "Internal error completing sign-in."),
    }
}

#[derive(Debug, Serialize)]
struct AttachmentUploadTokenResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
struct AttachmentUploadQuery {
    filename: Option<String>,
    mime_type: Option<String>,
    size: Option<u64>,
}

#[derive(Debug, Serialize)]
struct AttachmentUploadResponse {
    upload_id: String,
    filename: String,
    mime_type: Option<String>,
    size: u64,
}

async fn attachment_upload_token_handler(State(state): State<ServerState>) -> impl IntoResponse {
    Json(AttachmentUploadTokenResponse {
        token: state.attachment_upload_token,
    })
}

async fn attachment_upload_handler(
    headers: HeaderMap,
    Query(query): Query<AttachmentUploadQuery>,
    State(state): State<ServerState>,
    request: Request,
) -> Response {
    if !websocket_origin_authorized(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let authorized = headers
        .get("x-gwt-upload-token")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|token| token == state.attachment_upload_token);
    if !authorized {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let filename = query
        .filename
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("file")
        .to_string();
    let mime_type = query
        .mime_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let (upload_id, path) = state.attachment_uploads.allocate_path();

    if let Some(parent) = path.parent() {
        if let Err(error) = tokio::fs::create_dir_all(parent).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create upload directory: {error}"),
            )
                .into_response();
        }
    }

    let mut file = match tokio::fs::File::create(&path).await {
        Ok(file) => file,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create upload file: {error}"),
            )
                .into_response();
        }
    };
    let mut total_size = 0_u64;
    let mut stream = request.into_body().into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tokio::fs::remove_file(&path).await;
                return (
                    StatusCode::BAD_REQUEST,
                    format!("failed to read upload: {error}"),
                )
                    .into_response();
            }
        };
        total_size += chunk.len() as u64;
        if let Err(error) = file.write_all(&chunk).await {
            let _ = tokio::fs::remove_file(&path).await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to write upload: {error}"),
            )
                .into_response();
        }
    }
    if let Err(error) = file.flush().await {
        let _ = tokio::fs::remove_file(&path).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to flush upload: {error}"),
        )
            .into_response();
    }
    if let Some(declared) = query.size {
        if declared != total_size {
            let _ = tokio::fs::remove_file(&path).await;
            return (
                StatusCode::BAD_REQUEST,
                format!("upload size mismatch: declared {declared}, received {total_size}"),
            )
                .into_response();
        }
    }

    if let Err(error) = state.attachment_uploads.insert(
        upload_id.clone(),
        UploadedAttachment {
            path,
            filename: filename.clone(),
            mime_type: mime_type.clone(),
            size: total_size,
        },
    ) {
        return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
    }

    Json(AttachmentUploadResponse {
        upload_id,
        filename,
        mime_type,
        size: total_size,
    })
    .into_response()
}

/// Format an [`IpAddr`] for embedding in a URL: IPv6 addresses are wrapped in
/// `[...]` per RFC 3986, IPv4 / hostnames are emitted verbatim.
fn display_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

fn local_browser_client_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        ip => ip,
    }
}

fn agent_bridge_bind_ip() -> IpAddr {
    // Docker Desktop and Podman Machine proxy their host aliases to host
    // loopback. Native Linux host-gateway aliases target a bridge interface,
    // so this wildcard bind is intentional and applies only to the
    // capability-only router protected by an opaque two-UUID bearer; browser
    // routes stay on the independently configured listener.
    if cfg!(target_os = "linux") {
        IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    }
}

/// SPEC-1942 FR-098: access log middleware. Captures every HTTP request (and
/// the start of every WebSocket upgrade — the upgrade returns a `101 Switching
/// Protocols` response which is exactly what we record) into both
/// `tracing::info!(target: "gwt_access", ...)` and an in-memory sink for tests.
///
/// `/healthz` is demoted to `tracing::debug!` so periodic health probes do not
/// dominate the stderr stream when the operator wants to spot real LAN access.
/// Successful `/internal/hook-live` posts are internal hook-forwarding traffic
/// and are omitted entirely; failures remain visible for diagnosis.
async fn access_log_middleware(
    State(policy): State<AccessLogPolicy>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let user_agent = policy.record_user_agent.then(|| {
        request
            .headers()
            .get(USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    });
    let user_agent = user_agent.flatten();

    let started = Instant::now();
    let response = next.run(request).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let status = response.status().as_u16();

    let record = AccessLogRecord {
        method,
        path,
        status,
        peer: Some(peer.to_string()),
        user_agent,
        elapsed_ms,
    };

    if should_drop_access_log_record(&record) {
        return response;
    }

    if record.path == "/healthz" {
        tracing::debug!(
            target: "gwt_access",
            method = %record.method,
            path = %record.path,
            status,
            peer = %peer,
            user_agent = ?record.user_agent,
            elapsed_ms,
            "healthz probe"
        );
    } else {
        tracing::info!(
            target: "gwt_access",
            method = %record.method,
            path = %record.path,
            status,
            peer = %peer,
            user_agent = ?record.user_agent,
            elapsed_ms,
            "embedded server access"
        );
    }
    policy.sink.record(record);

    response
}

fn should_drop_access_log_record(record: &AccessLogRecord) -> bool {
    record.method == "POST" && record.path == "/internal/hook-live" && record.status == 204
}

async fn websocket_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
) -> impl IntoResponse {
    if !websocket_origin_authorized(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| client_session(socket, state))
}

async fn agent_pane_websocket_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
) -> Response {
    let Some(grant) = agent_capability_grant(&headers, &state) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match durable_agent_execution_authority_async(grant.principal().clone()).await {
        AgentDurableAuthority::ObservationOnly | AgentDurableAuthority::Current => {}
        AgentDurableAuthority::Stale => return StatusCode::CONFLICT.into_response(),
        AgentDurableAuthority::Unavailable => {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    ws.on_upgrade(move |socket| agent_pane_client_session(socket, state, grant))
}

async fn hook_live_handler(
    headers: HeaderMap,
    State(state): State<ServerState>,
    Json(mut event): Json<RuntimeHookEvent>,
) -> StatusCode {
    let Some(principal) = agent_capability_principal(&headers, &state) else {
        return StatusCode::UNAUTHORIZED;
    };
    if event.gwt_session_id.as_deref() != Some(principal.session_id()) {
        tracing::warn!(
            target: "gwt_security",
            "hook-live session claim did not match the authenticated agent capability"
        );
        return StatusCode::UNAUTHORIZED;
    }

    // The payload is observational data, not routing authority. Docker agents
    // may report an in-container cwd, so dispatch uses the server-side scope.
    event.gwt_session_id = Some(principal.session_id().to_string());
    event.project_root = Some(
        principal
            .canonical_project_root()
            .to_string_lossy()
            .into_owned(),
    );
    state.proxy.send(UserEvent::RuntimeHook(event));
    StatusCode::NO_CONTENT
}

async fn workspace_update_handler(
    headers: HeaderMap,
    State(state): State<ServerState>,
    Json(request): Json<AgentWorkspaceUpdateRequest>,
) -> Response {
    let Some(principal) = agent_capability_principal(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError {
                code: AgentWorkspaceUpdateErrorCode::InvalidRequest,
                message: "agent capability is missing or invalid".to_string(),
            },
        );
    };

    let Some(execution_binding) = principal.active_execution_binding().cloned() else {
        return execution_binding_error_response();
    };
    let project_root = principal.canonical_project_root().to_path_buf();
    let session_id = principal.session_id().to_string();
    let mutation_project_root = project_root.clone();
    let result = tokio::task::spawn_blocking(move || {
        gwt::apply_bound_authenticated_workspace_update(
            &mutation_project_root,
            &session_id,
            &execution_binding,
            request,
        )
    })
    .await;

    match result {
        Ok(Ok(receipt)) => {
            state
                .proxy
                .send(UserEvent::WorkspaceProjectionChanged { project_root });
            Json(receipt).into_response()
        }
        Ok(Err(error)) => {
            let status = workspace_update_error_status(error.code);
            workspace_update_error_response(status, error)
        }
        Err(_) => workspace_update_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            AgentWorkspaceUpdateError {
                code: AgentWorkspaceUpdateErrorCode::Internal,
                message: "Host workspace mutation task failed before a response was produced"
                    .to_string(),
            },
        ),
    }
}

async fn work_terminalization_handler(
    headers: HeaderMap,
    State(state): State<ServerState>,
    Json(request): Json<AgentWorkTerminalizationRequest>,
) -> Response {
    let Some(principal) = agent_capability_principal(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError {
                code: AgentWorkspaceUpdateErrorCode::InvalidRequest,
                message: "agent capability is missing or invalid".to_string(),
            },
        );
    };

    let Some(execution_binding) = principal.active_execution_binding().cloned() else {
        return execution_binding_error_response();
    };
    let project_root = principal.canonical_project_root().to_path_buf();
    let session_id = principal.session_id().to_string();
    let mutation_project_root = project_root.clone();
    let result = tokio::task::spawn_blocking(move || {
        gwt::apply_bound_authenticated_work_terminalization(
            &mutation_project_root,
            &session_id,
            &execution_binding,
            request,
        )
    })
    .await;

    match result {
        Ok(Ok(receipt)) => {
            state
                .proxy
                .send(UserEvent::WorkspaceProjectionChanged { project_root });
            Json(receipt).into_response()
        }
        Ok(Err(error)) => {
            let status = workspace_update_error_status(error.code);
            workspace_update_error_response(status, error)
        }
        Err(_) => workspace_update_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            AgentWorkspaceUpdateError {
                code: AgentWorkspaceUpdateErrorCode::Internal,
                message: "Host Work terminalization task failed before a response was produced"
                    .to_string(),
            },
        ),
    }
}

fn workspace_update_error_status(code: AgentWorkspaceUpdateErrorCode) -> StatusCode {
    match code {
        AgentWorkspaceUpdateErrorCode::InvalidRequest => StatusCode::BAD_REQUEST,
        AgentWorkspaceUpdateErrorCode::RelaunchRequired
        | AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch
        | AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired
        | AgentWorkspaceUpdateErrorCode::ProvenanceMismatch
        | AgentWorkspaceUpdateErrorCode::IdentityConflict
        | AgentWorkspaceUpdateErrorCode::TransactionConflict => StatusCode::CONFLICT,
        AgentWorkspaceUpdateErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn execution_binding_probe_handler(
    headers: HeaderMap,
    State(state): State<ServerState>,
    Json(request): Json<gwt::AgentExecutionBindingProbeRequest>,
) -> Response {
    let Some(principal) = agent_capability_principal(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError {
                code: AgentWorkspaceUpdateErrorCode::InvalidRequest,
                message: "agent capability is missing or invalid".to_string(),
            },
        );
    };
    // This route authorizes agent-initiated producing mutation. Prepared
    // authority is observation-only until the coordinator commits and
    // promotes the bearer, so it must never receive a successful receipt
    // through the same endpoint used by PreToolUse.
    let Some(execution_binding) = principal.active_execution_binding().cloned() else {
        return execution_binding_error_response();
    };
    let project_root = principal.canonical_project_root().to_path_buf();
    let session_id = principal.session_id().to_string();
    let host_instance_id = state.host_instance_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        gwt::probe_authenticated_execution_binding(
            &project_root,
            &session_id,
            &execution_binding,
            &host_instance_id,
            request,
        )
    })
    .await;

    match result {
        Ok(Ok(receipt)) => Json(receipt).into_response(),
        Ok(Err(error)) => {
            let status = workspace_update_error_status(error.code);
            workspace_update_error_response(status, error)
        }
        Err(_) => workspace_update_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            AgentWorkspaceUpdateError {
                code: AgentWorkspaceUpdateErrorCode::Internal,
                message: "Host execution binding probe failed before a response was produced"
                    .to_string(),
            },
        ),
    }
}

fn execution_binding_error_response() -> Response {
    workspace_update_error_response(
        StatusCode::CONFLICT,
        AgentWorkspaceUpdateError {
            code: AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch,
            message:
                "Execution binding is missing, stale, or no longer current; relaunch the Session before retrying"
                    .to_string(),
        },
    )
}

fn workspace_update_error_response(
    status: StatusCode,
    error: AgentWorkspaceUpdateError,
) -> Response {
    (status, Json(error)).into_response()
}

struct AgentPaneSessionScope {
    grant: AgentCapabilityGrant,
    allowed_window_ids: HashSet<String>,
}

impl AgentPaneSessionScope {
    fn new(grant: AgentCapabilityGrant) -> Self {
        Self {
            grant,
            allowed_window_ids: HashSet::new(),
        }
    }

    fn filter_inbound(&self, event: FrontendEvent) -> Option<AgentFrontendRequest> {
        match event {
            FrontendEvent::FrontendReady => Some(AgentFrontendRequest::Ready),
            FrontendEvent::CloseWindow { id, request_id }
                if self.allowed_window_ids.contains(&id)
                    && (self.grant.principal().authorizes_producing_mutation()
                        || request_id.is_some()) =>
            {
                Some(AgentFrontendRequest::CloseWindow {
                    id,
                    request_id,
                    responder: None,
                })
            }
            FrontendEvent::PaneSendInput { session_id, text }
                if session_id == self.grant.principal().session_id()
                    && self.grant.principal().authorizes_producing_mutation() =>
            {
                Some(AgentFrontendRequest::SendInput { text })
            }
            _ => None,
        }
    }

    fn filter_outbound(&mut self, payload: String) -> Option<String> {
        let mut value: serde_json::Value = serde_json::from_str(&payload).ok()?;
        match value.get("kind").and_then(serde_json::Value::as_str)? {
            "workspace_state" => {
                self.filter_workspace_state(&mut value)?;
                serde_json::to_string(&value).ok()
            }
            "terminal_snapshot" => value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| self.allowed_window_ids.contains(id))
                .then_some(payload),
            "pane_send_result" if self.grant.principal().authorizes_producing_mutation() => value
                .get("window_id")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|id| self.allowed_window_ids.contains(id))
                .then_some(payload),
            _ => None,
        }
    }

    fn filter_workspace_state(&mut self, value: &mut serde_json::Value) -> Option<()> {
        let workspace = value.get_mut("workspace")?.as_object_mut()?;
        let active_tab_id = workspace
            .get("active_tab_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let tabs = workspace.get_mut("tabs")?.as_array_mut()?;
        tabs.retain(|tab| self.authorizes_tab(tab));

        self.allowed_window_ids.clear();
        for tab in tabs.iter() {
            let windows = tab
                .get("workspace")
                .and_then(|workspace| workspace.get("windows"))
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten();
            for window in windows {
                if let Some(id) = window.get("id").and_then(serde_json::Value::as_str) {
                    self.allowed_window_ids.insert(id.to_string());
                }
            }
        }

        let first_tab_id = tabs
            .first()
            .and_then(|tab| tab.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let active_is_allowed = active_tab_id.is_some_and(|active| {
            tabs.iter().any(|tab| {
                tab.get("id").and_then(serde_json::Value::as_str) == Some(active.as_str())
            })
        });
        if !active_is_allowed {
            workspace.insert(
                "active_tab_id".to_string(),
                first_tab_id
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null),
            );
        }
        workspace.insert(
            "recent_projects".to_string(),
            serde_json::Value::Array(Vec::new()),
        );
        Some(())
    }

    fn authorizes_tab(&self, tab: &serde_json::Value) -> bool {
        let Some(project_root) = tab.get("project_root").and_then(serde_json::Value::as_str) else {
            return false;
        };
        dunce::canonicalize(project_root)
            .map(|path| gwt_core::paths::normalize_windows_child_process_path(&path))
            .is_ok_and(|path| path == self.grant.principal().canonical_project_root())
    }

    fn filter_repair_panes(&self, repair_panes: Vec<String>) -> Vec<String> {
        repair_panes
            .into_iter()
            .filter(|id| self.allowed_window_ids.contains(id))
            .collect()
    }
}

enum ClientSessionScope {
    Browser,
    Agent(AgentPaneSessionScope),
}

enum ScopedFrontendRequest {
    Browser(FrontendEvent),
    Agent {
        grant: AgentCapabilityGrant,
        request: AgentFrontendRequest,
    },
}

impl ClientSessionScope {
    fn refresh_agent_grant(&mut self, registry: &AgentCapabilityRegistry) -> bool {
        match self {
            Self::Browser => true,
            Self::Agent(scope) => {
                let Some(grant) = registry.refresh_grant(&scope.grant) else {
                    return false;
                };
                scope.grant = grant;
                true
            }
        }
    }

    fn filter_inbound(&self, event: FrontendEvent) -> Option<ScopedFrontendRequest> {
        match self {
            Self::Browser => Some(ScopedFrontendRequest::Browser(event)),
            Self::Agent(scope) => {
                scope
                    .filter_inbound(event)
                    .map(|request| ScopedFrontendRequest::Agent {
                        grant: scope.grant.clone(),
                        request,
                    })
            }
        }
    }

    fn filter_outbound(&mut self, payload: String) -> Option<String> {
        match self {
            Self::Browser => Some(payload),
            Self::Agent(scope) => scope.filter_outbound(payload),
        }
    }

    fn filter_repair_panes(&self, repair_panes: Vec<String>) -> Vec<String> {
        match self {
            Self::Browser => repair_panes,
            Self::Agent(scope) => scope.filter_repair_panes(repair_panes),
        }
    }
}

async fn client_session(socket: WebSocket, state: ServerState) {
    client_session_with_scope(socket, state, ClientSessionScope::Browser).await;
}

async fn agent_pane_client_session(
    socket: WebSocket,
    state: ServerState,
    grant: AgentCapabilityGrant,
) {
    client_session_with_scope(
        socket,
        state,
        ClientSessionScope::Agent(AgentPaneSessionScope::new(grant)),
    )
    .await;
}

async fn send_agent_self_close_acceptance<S>(
    sender: &mut S,
    acceptance: AgentSelfCloseDirectAcceptance,
    deadline_after: Duration,
) where
    S: futures_util::Sink<Message> + Unpin,
{
    match acceptance.wire_payload() {
        Ok(payload) => {
            let _ =
                tokio::time::timeout(deadline_after, sender.send(Message::Text(payload.into())))
                    .await;
        }
        Err(error) => {
            tracing::error!(
                error = %error,
                "failed to serialize direct pane close acceptance"
            );
        }
    }
    // The accepted handoff finalizes after the bounded send attempt on every
    // result. If this future is cancelled while awaiting the sink, Rust drops
    // the owned acceptance and runs the same finalizer.
    drop(acceptance);
}

async fn send_agent_fence_close<S>(sender: &mut S, close_frame: ClientCloseFrame)
where
    S: futures_util::Sink<Message> + Unpin,
{
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        sender.send(Message::Close(Some(axum::extract::ws::CloseFrame {
            code: close_frame.code,
            reason: close_frame.reason.into(),
        }))),
    )
    .await;
}

async fn client_session_with_scope(
    socket: WebSocket,
    state: ServerState,
    mut scope: ClientSessionScope,
) {
    let client_id = Uuid::new_v4().to_string();
    let outbound = match &scope {
        ClientSessionScope::Browser => state.clients.register(client_id.clone()),
        ClientSessionScope::Agent(_) => state.clients.register_pane(client_id.clone()),
    };
    let (mut sender, mut receiver) = socket.split();

    let input_seq = Arc::new(AtomicU64::new(0));

    loop {
        tokio::select! {
            step = outbound.next() => {
                match step {
                    DrainStep::Message { payload, repair_panes } => {
                        if !scope.refresh_agent_grant(&state.agent_capabilities) {
                            send_agent_fence_close(
                                &mut sender,
                                AGENT_STALE_BINDING_CLOSE,
                            )
                            .await;
                            break;
                        }
                        let Some(payload) = scope.filter_outbound(payload) else {
                            continue;
                        };
                        if sender.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                        let repair_panes = scope.filter_repair_panes(repair_panes);
                        if !repair_panes.is_empty() {
                            // SPEC-2359 W-17 (FR-396): streamed output for
                            // these panes was dropped under queue pressure —
                            // ask the event loop for fresh snapshots so the
                            // display self-heals.
                            state.proxy.send(UserEvent::ClientPaneSnapshotRepair {
                                client_id: client_id.clone(),
                                pane_ids: repair_panes,
                            });
                        }
                    }
                    DrainStep::Closed(close_frame) => {
                        if let Some(close_frame) = close_frame {
                            let _ = sender
                                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                                    code: close_frame.code,
                                    reason: close_frame.reason.into(),
                                })))
                                .await;
                        }
                        break;
                    }
                }
            }
            maybe_message = receiver.next() => {
                match maybe_message {
                    Some(Ok(Message::Text(text))) => {
                        if !scope.refresh_agent_grant(&state.agent_capabilities) {
                            send_agent_fence_close(
                                &mut sender,
                                AGENT_STALE_BINDING_CLOSE,
                            )
                            .await;
                            break;
                        }
                        match serde_json::from_str::<FrontendEvent>(text.as_ref()) {
                            Ok(event) => {
                                match scope.filter_inbound(event) {
                                    Some(ScopedFrontendRequest::Browser(event)) => {
                                        handle_frontend_message(
                                            &state,
                                            &client_id,
                                            &input_seq,
                                            event,
                                        );
                                    }
                                    Some(ScopedFrontendRequest::Agent {
                                        grant,
                                        mut request,
                                    }) => {
                                        if grant.principal().authorizes_producing_mutation()
                                            && request.mutates_host_state()
                                        {
                                            let durable_authority =
                                                if request.requires_producing_authority() {
                                                    durable_agent_execution_authority_with_lease_async(
                                                        grant.principal().clone(),
                                                    )
                                                    .await
                                                } else {
                                                    durable_agent_execution_authority_async(
                                                        grant.principal().clone(),
                                                    )
                                                    .await
                                                };
                                            match durable_authority {
                                                AgentDurableAuthority::Current => {}
                                                AgentDurableAuthority::Stale => {
                                                    tracing::warn!(
                                                        target: "gwt_security",
                                                        "agent pane WebSocket execution binding is no longer current"
                                                    );
                                                    send_agent_fence_close(
                                                        &mut sender,
                                                        AGENT_STALE_BINDING_CLOSE,
                                                    )
                                                    .await;
                                                    break;
                                                }
                                                AgentDurableAuthority::Unavailable => {
                                                    tracing::warn!(
                                                        target: "gwt_security",
                                                        "agent pane WebSocket execution authority is unavailable"
                                                    );
                                                    send_agent_fence_close(
                                                        &mut sender,
                                                        AGENT_AUTHORITY_UNAVAILABLE_CLOSE,
                                                    )
                                                    .await;
                                                    break;
                                                }
                                                AgentDurableAuthority::ObservationOnly => {
                                                    send_agent_fence_close(
                                                        &mut sender,
                                                        AGENT_STALE_BINDING_CLOSE,
                                                    )
                                                    .await;
                                                    break;
                                                }
                                            }
                                        }
                                        let direct_acceptance = match &mut request {
                                            AgentFrontendRequest::CloseWindow {
                                                request_id: Some(_),
                                                responder,
                                                ..
                                            } => {
                                                let (direct_responder, acceptance) =
                                                    AgentSelfCloseResponder::channel();
                                                *responder = Some(direct_responder);
                                                Some(acceptance)
                                            }
                                            _ => None,
                                        };
                                        let event_grant = grant.clone();
                                        let dispatched = state.agent_capabilities.dispatch_if_current(
                                            &grant,
                                            || {
                                                state.proxy.send(UserEvent::AgentFrontend {
                                                    client_id: client_id.clone(),
                                                    grant: event_grant,
                                                    request,
                                                });
                                            },
                                        );
                                        if !dispatched {
                                            tracing::warn!(
                                                target: "gwt_security",
                                                "agent pane WebSocket capability rotated or revoked before dispatch"
                                            );
                                            send_agent_fence_close(
                                                &mut sender,
                                                AGENT_STALE_BINDING_CLOSE,
                                            )
                                            .await;
                                            break;
                                        }
                                        if let Some(mut direct_acceptance) = direct_acceptance {
                                            // Correlated self-close is a two-phase exchange. The
                                            // tao thread first atomically accepts or rejects the
                                            // current capability generation. Only an accepted
                                            // request gets a direct response on this origin
                                            // socket; generic ClientHub traffic is never used.
                                            let deadline =
                                                tokio::time::Instant::now() + Duration::from_secs(2);
                                            let accepted = loop {
                                                tokio::select! {
                                                    result = &mut direct_acceptance => {
                                                        break result.ok();
                                                    }
                                                    incoming = receiver.next() => {
                                                        match incoming {
                                                            Some(Ok(Message::Close(_)))
                                                            | Some(Err(_))
                                                            | None => break None,
                                                            Some(Ok(_)) => {}
                                                        }
                                                    }
                                                    _ = tokio::time::sleep_until(deadline) => {
                                                        break None;
                                                    }
                                                }
                                            };
                                            let Some(acceptance) = accepted else {
                                                break;
                                            };
                                            send_agent_self_close_acceptance(
                                                &mut sender,
                                                acceptance,
                                                Duration::from_secs(2),
                                            )
                                            .await;
                                            break;
                                        }
                                    }
                                    None => {
                                        tracing::warn!(
                                            target: "gwt_security",
                                            "agent pane WebSocket rejected an out-of-scope frontend event"
                                        );
                                    }
                                }
                            }
                            Err(error) => {
                                eprintln!("invalid frontend message: {error}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        eprintln!("websocket error: {error}");
                        break;
                    }
                }
            }
        }
    }

    state.clients.unregister(&client_id);
}

fn handle_frontend_message(
    state: &ServerState,
    client_id: &str,
    input_seq: &AtomicU64,
    event: FrontendEvent,
) {
    let (id, data) = match event {
        FrontendEvent::TerminalInput { id, data } => (id, data),
        other => {
            state.proxy.send(UserEvent::Frontend {
                client_id: client_id.to_string(),
                event: other,
            });
            return;
        }
    };

    let seq = input_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "ws_recv",
        client_id = %client_id,
        seq,
        window_id = %id,
        "terminal_input received over WebSocket"
    );

    let pty_handle = match state.pty_writers.read() {
        Ok(guard) => guard.get(&id).cloned(),
        Err(_error) => {
            tracing::warn!(
                target: "gwt_input_trace",
                stage = "fast_path_lock_poisoned",
                client_id = %client_id,
                seq,
                window_id = %id,
                "pty_writers read lock poisoned; falling back to event loop"
            );
            None
        }
    };

    if let Some(pty) = pty_handle {
        let write_started = Instant::now();
        match pty.write_input(data.as_bytes()) {
            Ok(()) => {
                tracing::debug!(
                    target: "gwt_input_trace",
                    stage = "fast_path_write",
                    client_id = %client_id,
                    seq,
                    window_id = %id,
                    write_us = write_started.elapsed().as_micros() as u64,
                    "terminal_input written to PTY via WS fast-path"
                );
                return;
            }
            Err(_error) => {
                tracing::warn!(
                    target: "gwt_input_trace",
                    stage = "fast_path_write_err",
                    client_id = %client_id,
                    seq,
                    window_id = %id,
                    "fast-path PTY write failed; forwarding to event loop for error handling"
                );
            }
        }
    } else {
        tracing::debug!(
            target: "gwt_input_trace",
            stage = "fast_path_miss",
            client_id = %client_id,
            seq,
            window_id = %id,
            "pty_writers registry miss; falling back to event loop"
        );
    }

    state.proxy.send(UserEvent::Frontend {
        client_id: client_id.to_string(),
        event: FrontendEvent::TerminalInput {
            id: id.clone(),
            data,
        },
    });
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "ws_dispatch",
        client_id = %client_id,
        seq,
        window_id = %id,
        ok = true,
        "terminal_input forwarded to event loop proxy (fallback)"
    );
}

#[cfg(test)]
pub fn hook_forward_authorized(headers: &HeaderMap, expected_token: &str) -> bool {
    bearer_token(headers).is_some_and(|token| constant_time_token_eq(token, expected_token))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty())
}

fn constant_time_token_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn agent_capability_principal(
    headers: &HeaderMap,
    state: &ServerState,
) -> Option<AgentSessionPrincipal> {
    state
        .agent_capabilities
        .authenticate(bearer_token(headers)?)
}

fn agent_capability_grant(
    headers: &HeaderMap,
    state: &ServerState,
) -> Option<AgentCapabilityGrant> {
    let token = bearer_token(headers)?.to_string();
    let principal = state.agent_capabilities.authenticate(&token)?;
    Some(AgentCapabilityGrant::new(token, principal))
}

pub fn websocket_origin_authorized(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN) else {
        return true;
    };
    let Some(host) = headers.get(HOST) else {
        return false;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Ok(host) = host.to_str() else {
        return false;
    };

    let origin = origin.trim_end_matches('/');
    origin == format!("http://{host}") || origin == format!("https://{host}")
}

#[cfg(test)]
pub fn broadcast_runtime_hook_event(clients: &ClientHub, event: RuntimeHookEvent) {
    clients.dispatch(vec![OutboundEvent::broadcast(
        gwt::BackendEvent::RuntimeHookEvent { event },
    )]);
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        net::IpAddr,
        pin::Pin,
        sync::{atomic::AtomicU64, Arc, Mutex, RwLock},
        task::{Context, Poll},
        time::Duration,
    };

    use axum::extract::ws::Message as AxumMessage;
    use axum::http::{
        header::{AUTHORIZATION, HOST, ORIGIN},
        HeaderMap, StatusCode,
    };
    use futures_util::{Sink, SinkExt, StreamExt};
    use gwt::{BackendEvent, FrontendEvent, RuntimeHookEvent, RuntimeHookEventKind};
    use gwt_core::test_support::ScopedEnvVar;
    use reqwest::StatusCode as HttpStatusCode;
    use tokio::runtime::Runtime;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{
            client::IntoClientRequest, Error as WebSocketError, Message as WebSocketMessage,
        },
    };

    use crate::{AppEventProxy, AttachmentUploadStore, OutboundEvent, UserEvent};

    use super::{
        agent_bridge_bind_ip, bearer_token, handle_frontend_message, prepare_outbound,
        queue_class_for_kind, send_agent_self_close_acceptance, websocket_origin_authorized,
        AgentCapabilityGrant, AgentCapabilityIssuer, AgentCapabilityRegistry, AgentFrontendRequest,
        AgentPaneSessionScope, AgentSelfCloseDirectAcceptance, AgentSessionPrincipal, ClientHub,
        ClientQueue, ClientSessionScope, DrainStep, EmbeddedServer, HookForwardTarget,
        PreparedOutbound, QueueClass, ScopedFrontendRequest, ServerState, DRAIN_LOW_WATER,
        LOSSLESS_HARD_CAP, LOSSY_HIGH_WATER,
    };

    struct FailingMessageSink;

    impl Sink<AxumMessage> for FailingMessageSink {
        type Error = &'static str;

        fn poll_ready(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Err("socket closed"))
        }

        fn start_send(self: Pin<&mut Self>, _item: AxumMessage) -> Result<(), Self::Error> {
            Err("socket closed")
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    struct PendingMessageSink;

    impl Sink<AxumMessage> for PendingMessageSink {
        type Error = &'static str;

        fn poll_ready(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }

        fn start_send(self: Pin<&mut Self>, _item: AxumMessage) -> Result<(), Self::Error> {
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
    }

    fn sample_server_state() -> (ServerState, Arc<Mutex<Vec<UserEvent>>>) {
        let (proxy, events) = AppEventProxy::stub();
        (
            ServerState {
                proxy,
                clients: ClientHub::default(),
                agent_capabilities: AgentCapabilityRegistry::default(),
                host_instance_id: "test-host-instance".to_string(),
                attachment_upload_token: "upload-token".to_string(),
                attachment_uploads: AttachmentUploadStore::in_system_temp(),
                pty_writers: Arc::new(RwLock::new(HashMap::new())),
                access_log: super::AccessLogSink::default(),
            },
            events,
        )
    }

    fn sample_runtime_hook_event() -> RuntimeHookEvent {
        RuntimeHookEvent {
            kind: RuntimeHookEventKind::RuntimeState,
            source_event: Some("PreToolUse".to_string()),
            gwt_session_id: Some("session-1".to_string()),
            continuation_readiness_nonce: None,
            agent_session_id: Some("agent-1".to_string()),
            project_root: Some("E:/gwt/test-repo".to_string()),
            branch: Some("feature/runtime".to_string()),
            status: Some("Running".to_string()),
            tool_name: Some("Bash".to_string()),
            message: None,
            occurred_at: "2026-04-21T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn agent_session_principal_canonicalizes_project_and_redacts_debug() {
        let project = tempfile::tempdir().expect("project tempdir");
        let aliased_project = project.path().join("child").join("..");
        std::fs::create_dir_all(project.path().join("child")).expect("project child");

        let principal = AgentSessionPrincipal::new(&aliased_project, "session-secret")
            .expect("canonical principal");
        let canonical_project = dunce::canonicalize(project.path()).expect("canonical project");

        assert_eq!(principal.canonical_project_root(), canonical_project);
        assert_eq!(principal.session_id(), "session-secret");
        assert!(principal.authorizes_project_root(project.path()));
        assert!(AgentSessionPrincipal::new(project.path(), "").is_err());
        assert!(AgentSessionPrincipal::new(project.path(), " session-secret").is_err());
        let unsafe_session_error = AgentSessionPrincipal::new(project.path(), "../session-secret")
            .expect_err("unsafe Session id must be rejected");
        assert!(!unsafe_session_error.contains("session-secret"));
        assert!(AgentSessionPrincipal::new(project.path(), "session/foreign").is_err());

        let debug = format!("{principal:?}");
        assert!(!debug.contains("session-secret"));
        assert!(!debug.contains(&canonical_project.display().to_string()));
    }

    #[test]
    fn agent_session_principal_preserves_exact_project_state_scope() {
        let project_state_root = tempfile::tempdir().expect("Project State root");
        let child_bare = project_state_root.path().join("project.git");
        let request = gwt_core::process::ProcessPlanRequest::new("git")
            .args(["init", "--bare"])
            .arg(&child_bare);
        let output = gwt_core::process::resolved_command(request)
            .expect("resolve git")
            .output()
            .expect("initialize child bare repository");
        assert!(
            output.status.success(),
            "git init --bare failed: {output:?}"
        );

        let principal = AgentSessionPrincipal::new(project_state_root.path(), "session-1")
            .expect("Project State-scoped principal");
        let canonical_project_state_root =
            dunce::canonicalize(project_state_root.path()).expect("canonical Project State root");
        let canonical_bare = dunce::canonicalize(&child_bare).expect("canonical bare repository");

        assert_eq!(
            principal.canonical_project_root(),
            canonical_project_state_root,
            "capability scope must match the exact root persisted in the Session ledger"
        );
        assert_ne!(principal.canonical_project_root(), canonical_bare);
        assert!(principal.authorizes_project_root(project_state_root.path()));
        assert!(!principal.authorizes_project_root(&child_bare));
    }

    #[test]
    fn agent_session_principal_separates_inspection_from_exact_execution_authority() {
        let project = tempfile::tempdir().expect("project tempdir");
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-current".to_string(),
            repo_hash: "repo-current".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 2359,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-current".to_string(),
                binding_id: "binding-current".to_string(),
                ledger_head_hash: "head-current".to_string(),
            },
            capability_generation: 4,
        };

        let inspection = AgentSessionPrincipal::new(project.path(), "session-current")
            .expect("inspection principal");
        assert!(inspection.execution_binding().is_none());
        assert!(!inspection.authorizes_producing_mutation());
        assert_eq!(
            inspection.execution_authority_kind(),
            super::AgentExecutionAuthorityKind::Inspection
        );

        let prepared =
            AgentSessionPrincipal::new_prepared(project.path(), "session-current", binding.clone())
                .expect("prepared observation principal");
        assert_eq!(prepared.execution_binding(), Some(&binding));
        assert!(!prepared.authorizes_producing_mutation());
        assert_eq!(
            prepared.execution_authority_kind(),
            super::AgentExecutionAuthorityKind::Prepared
        );

        let current =
            AgentSessionPrincipal::new_bound(project.path(), "session-current", binding.clone())
                .expect("current producing principal");
        assert_eq!(current.execution_binding(), Some(&binding));
        assert!(current.authorizes_producing_mutation());
        assert_eq!(
            current.execution_authority_kind(),
            super::AgentExecutionAuthorityKind::Active
        );

        let error =
            AgentSessionPrincipal::new_bound(project.path(), "foreign-session", binding.clone())
                .expect_err("binding cannot select a different Session principal");
        assert!(!error.contains("binding-current"));
        assert!(!error.contains("generation-current"));
        for principal in [inspection, prepared, current] {
            let debug = format!("{principal:?}");
            assert!(!debug.contains("binding-current"));
            assert!(!debug.contains("generation-current"));
        }
    }

    #[test]
    fn agent_capability_registry_rotates_same_project_session_atomically() {
        let project = tempfile::tempdir().expect("project tempdir");
        let registry = AgentCapabilityRegistry::default();

        let stale = registry
            .issue(project.path(), "session-1")
            .expect("first capability");
        let current = registry
            .issue(project.path(), "session-1")
            .expect("rotated capability");

        assert_ne!(stale, current);
        assert!(registry.authenticate(&stale).is_none());
        let principal = registry
            .authenticate(&current)
            .expect("current capability remains valid");
        assert_eq!(principal.session_id(), "session-1");
        assert!(principal.authorizes_project_root(project.path()));
        assert_eq!(registry.session_count(), 1);
    }

    #[test]
    fn agent_capability_registry_promotes_prepared_authority_without_rotating_bearer() {
        let project = tempfile::tempdir().expect("project tempdir");
        let registry = AgentCapabilityRegistry::default();
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-promote".to_string(),
            repo_hash: "repo-promote".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 2359,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-promote".to_string(),
                binding_id: "binding-promote".to_string(),
                ledger_head_hash: "head-promote".to_string(),
            },
            capability_generation: 2,
        };
        let token = registry
            .issue_prepared(project.path(), "session-promote", binding.clone())
            .expect("Prepared capability");
        let prepared_grant = super::AgentCapabilityGrant::new(
            token.clone(),
            registry
                .authenticate(&token)
                .expect("authenticate Prepared capability"),
        );
        assert_eq!(
            prepared_grant.principal().execution_authority_kind(),
            super::AgentExecutionAuthorityKind::Prepared
        );

        registry
            .promote_prepared(&token, &binding)
            .expect("promote exact Prepared authority");
        registry
            .promote_prepared(&token, &binding)
            .expect("promotion readback is idempotent");
        let refreshed = registry
            .refresh_grant(&prepared_grant)
            .expect("same bearer refreshes to Active principal");
        assert_eq!(refreshed.token, token);
        assert_eq!(
            refreshed.principal().execution_authority_kind(),
            super::AgentExecutionAuthorityKind::Active
        );
        assert!(refreshed.principal().authorizes_producing_mutation());
        assert!(
            !registry.grant_is_current(&prepared_grant),
            "a queued pre-promotion snapshot must not dispatch as Active"
        );
        assert!(registry.grant_is_current(&refreshed));

        let mut mismatched = binding;
        mismatched.identity.ledger_head_hash.push_str("-mismatch");
        assert!(
            registry.promote_prepared(&token, &mismatched).is_err(),
            "promotion cannot retarget a bearer to another execution identity"
        );
    }

    #[test]
    fn agent_capability_registry_promotes_legacy_inspection_without_rotating_bearer() {
        let project = tempfile::tempdir().expect("project tempdir");
        let registry = AgentCapabilityRegistry::default();
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-legacy".to_string(),
            repo_hash: "repo-legacy".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 2359,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-legacy".to_string(),
                binding_id: "binding-legacy".to_string(),
                ledger_head_hash: "head-legacy".to_string(),
            },
            capability_generation: 1,
        };
        let token = registry
            .issue(project.path(), "session-legacy")
            .expect("legacy inspection capability");
        let inspection = super::AgentCapabilityGrant::new(
            token.clone(),
            registry
                .authenticate(&token)
                .expect("authenticate inspection capability"),
        );

        registry
            .promote_inspection(&token, &binding)
            .expect("promote exact legacy authority");
        let refreshed = registry
            .refresh_grant(&inspection)
            .expect("same bearer refreshes to Active");
        assert_eq!(refreshed.token, token);
        assert_eq!(
            refreshed.principal().execution_authority_kind(),
            super::AgentExecutionAuthorityKind::Active
        );
        assert!(!registry.grant_is_current(&inspection));
        assert!(registry.grant_is_current(&refreshed));

        let mut mismatched = binding;
        mismatched.identity.ledger_head_hash.push_str("-mismatch");
        assert!(registry.promote_inspection(&token, &mismatched).is_err());
    }

    #[test]
    fn connected_agent_scope_refreshes_same_bearer_after_prepared_promotion() {
        let project = tempfile::tempdir().expect("project tempdir");
        let registry = AgentCapabilityRegistry::default();
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-connected-promotion".to_string(),
            repo_hash: "repo-connected-promotion".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 2359,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-connected-promotion".to_string(),
                binding_id: "binding-connected-promotion".to_string(),
                ledger_head_hash: "head-connected-promotion".to_string(),
            },
            capability_generation: 2,
        };
        let token = registry
            .issue_prepared(
                project.path(),
                "session-connected-promotion",
                binding.clone(),
            )
            .expect("Prepared capability");
        let grant = AgentCapabilityGrant::new(
            token.clone(),
            registry
                .authenticate(&token)
                .expect("authenticate Prepared capability"),
        );
        let mut scope = ClientSessionScope::Agent(AgentPaneSessionScope::new(grant));
        assert!(scope
            .filter_inbound(FrontendEvent::PaneSendInput {
                session_id: "session-connected-promotion".to_string(),
                text: "before-promotion".to_string(),
            })
            .is_none());

        registry
            .promote_prepared(&token, &binding)
            .expect("promote exact Prepared capability");
        assert!(
            scope.refresh_agent_grant(&registry),
            "an already-connected socket must refresh the same bearer"
        );
        assert!(matches!(
            scope.filter_inbound(FrontendEvent::PaneSendInput {
                session_id: "session-connected-promotion".to_string(),
                text: "after-promotion".to_string(),
            }),
            Some(ScopedFrontendRequest::Agent {
                request: AgentFrontendRequest::SendInput { text },
                ..
            }) if text == "after-promotion"
        ));
    }

    #[test]
    fn agent_capability_issue_preflight_is_non_issuing_and_rejects_closing_principal() {
        let project = tempfile::tempdir().expect("project tempdir");
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:1/hook",
            "ws://127.0.0.1:1/pane",
            "ws://127.0.0.1:1/agent-pane",
        );

        issuer
            .preflight_issue(project.path(), "session-preflight")
            .expect("preflight accepts one canonical project + Session");
        assert_eq!(
            issuer.registry.session_count(),
            0,
            "preflight must not mint or reserve a capability"
        );

        let missing_root = project.path().join("missing-project-root");
        let unsafe_error = issuer
            .preflight_issue(&missing_root, "../session-secret")
            .expect_err("preflight rejects non-canonical identity inputs");
        assert_eq!(
            unsafe_error,
            "agent capability session id must be non-empty and canonical"
        );
        assert!(!unsafe_error.contains("session-secret"));
        assert_eq!(issuer.registry.session_count(), 0);

        let target = issuer
            .issue(project.path(), "session-preflight")
            .expect("issue capability");
        let grant = issuer
            .grant_for_test(&target.token)
            .expect("current capability grant");
        let ticket = issuer
            .begin_self_close_if_current(&grant)
            .expect("begin closing current capability");

        let closing_error = issuer
            .preflight_issue(project.path(), "session-preflight")
            .expect_err("preflight rejects a principal whose pane is closing");
        assert_eq!(
            closing_error,
            "agent capability is closing; retry after pane teardown"
        );
        assert!(!closing_error.contains("session-preflight"));
        assert_eq!(
            issuer.registry.session_count(),
            1,
            "closing state remains the only registered Session"
        );
        assert!(issuer.finish_self_close(&ticket));
    }

    #[test]
    fn agent_capability_registry_keeps_same_session_separate_across_projects() {
        let project_a = tempfile::tempdir().expect("project A tempdir");
        let project_b = tempfile::tempdir().expect("project B tempdir");
        let registry = AgentCapabilityRegistry::default();

        let token_a = registry
            .issue(project_a.path(), "shared-session")
            .expect("project A capability");
        let token_b = registry
            .issue(project_b.path(), "shared-session")
            .expect("project B capability");

        assert_ne!(token_a, token_b);
        let principal_a = registry
            .authenticate(&token_a)
            .expect("project A principal");
        let principal_b = registry
            .authenticate(&token_b)
            .expect("project B principal");
        assert!(principal_a.authorizes_project_root(project_a.path()));
        assert!(!principal_a.authorizes_project_root(project_b.path()));
        assert!(principal_b.authorizes_project_root(project_b.path()));
        assert!(!principal_b.authorizes_project_root(project_a.path()));
        assert_eq!(registry.session_count(), 2);
    }

    #[test]
    fn agent_capability_registry_exact_token_revoke_preserves_rotated_and_foreign_grants() {
        let project_a = tempfile::tempdir().expect("project A tempdir");
        let project_b = tempfile::tempdir().expect("project B tempdir");
        let registry = AgentCapabilityRegistry::default();
        let stale_a = registry
            .issue(project_a.path(), "session-1")
            .expect("stale project A capability");
        let current_a = registry
            .issue(project_a.path(), "session-1")
            .expect("current project A capability");
        let token_b = registry
            .issue(project_b.path(), "session-1")
            .expect("project B capability");

        assert!(
            !registry.revoke_token(&stale_a),
            "revoking a rotated token must not remove the replacement grant"
        );
        assert!(registry.authenticate(&current_a).is_some());
        assert!(registry.authenticate(&token_b).is_some());
        assert!(registry.revoke_token(&current_a));
        assert!(registry.authenticate(&current_a).is_none());
        assert!(registry.authenticate(&token_b).is_some());
        assert!(!registry.revoke_token(&current_a));
        assert_eq!(registry.session_count(), 1);
    }

    #[test]
    fn agent_capability_registry_revoke_survives_project_deletion() {
        let project = tempfile::tempdir().expect("project tempdir");
        let registry = AgentCapabilityRegistry::default();
        let token = registry
            .issue(project.path(), "session-1")
            .expect("project capability");

        project.close().expect("delete project after issue");

        assert!(registry.revoke_token(&token));
        assert!(registry.authenticate(&token).is_none());
        assert_eq!(registry.session_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn agent_capability_registry_revoke_survives_project_permission_loss() {
        use std::os::unix::fs::PermissionsExt;

        let project = tempfile::tempdir().expect("project tempdir");
        let registry = AgentCapabilityRegistry::default();
        let token = registry
            .issue(project.path(), "session-permission-loss")
            .expect("project capability");
        let original_permissions = std::fs::metadata(project.path())
            .expect("project metadata")
            .permissions();
        let mut inaccessible_permissions = original_permissions.clone();
        inaccessible_permissions.set_mode(0o0);
        std::fs::set_permissions(project.path(), inaccessible_permissions)
            .expect("remove project permissions");

        let revoked = registry.revoke_token(&token);

        std::fs::set_permissions(project.path(), original_permissions)
            .expect("restore project permissions");
        assert!(revoked);
        assert!(registry.authenticate(&token).is_none());
        assert_eq!(registry.session_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn agent_capability_registry_exact_revoke_ignores_symlink_retargeting() {
        let root = tempfile::tempdir().expect("root tempdir");
        let project_a = root.path().join("project-a");
        let project_b = root.path().join("project-b");
        let alias = root.path().join("project-link");
        std::fs::create_dir(&project_a).expect("project A");
        std::fs::create_dir(&project_b).expect("project B");
        std::os::unix::fs::symlink(&project_a, &alias).expect("alias project A");

        let registry = AgentCapabilityRegistry::default();
        let token_a = registry
            .issue(&alias, "session-1")
            .expect("project A capability");
        std::fs::remove_file(&alias).expect("remove project A alias");
        std::os::unix::fs::symlink(&project_b, &alias).expect("retarget alias to project B");
        let token_b = registry
            .issue(&alias, "session-1")
            .expect("project B capability");

        assert!(registry.revoke_token(&token_a));
        assert!(registry.authenticate(&token_a).is_none());
        assert!(
            registry.authenticate(&token_b).is_some(),
            "retargeting a symlink must not make stale cleanup revoke the new principal"
        );
        assert_eq!(registry.session_count(), 1);
    }

    #[test]
    fn bearer_token_parser_rejects_missing_empty_and_non_bearer_values() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer_token(&headers), None);

        headers.insert(AUTHORIZATION, "Bearer ".parse().expect("empty bearer"));
        assert_eq!(bearer_token(&headers), None);

        headers.insert(
            AUTHORIZATION,
            "bearer capability".parse().expect("lowercase bearer"),
        );
        assert_eq!(bearer_token(&headers), None);

        headers.insert(
            AUTHORIZATION,
            "Basic capability".parse().expect("basic authorization"),
        );
        assert_eq!(bearer_token(&headers), None);

        headers.insert(
            AUTHORIZATION,
            "Bearer capability".parse().expect("bearer authorization"),
        );
        assert_eq!(bearer_token(&headers), Some("capability"));
    }

    #[test]
    fn agent_capability_issuer_debug_never_contains_secret_or_principal() {
        let project = tempfile::tempdir().expect("project tempdir");
        let registry = AgentCapabilityRegistry::default();
        let issuer = AgentCapabilityIssuer::new(
            "http://127.0.0.1:43123/internal/hook-live".to_string(),
            "ws://127.0.0.1:43124/ws".to_string(),
            "ws://127.0.0.1:43123/internal/pane-ws".to_string(),
            registry,
        );
        let target = issuer
            .issue(project.path(), "session-secret")
            .expect("issued target");

        let debug = format!("{issuer:?}");
        assert!(!debug.contains(&target.token));
        assert!(!debug.contains("session-secret"));
        assert!(!debug.contains(&project.path().display().to_string()));
    }

    #[test]
    fn agent_pane_scope_filters_project_output_and_frontend_authority() {
        let project = tempfile::tempdir().expect("project tempdir");
        let foreign = tempfile::tempdir().expect("foreign tempdir");
        let principal =
            AgentSessionPrincipal::new(project.path(), "session-1").expect("agent principal");
        let mut scope = super::AgentPaneSessionScope::new(super::AgentCapabilityGrant::new(
            "test-capability".to_string(),
            principal,
        ));
        let workspace = serde_json::json!({
            "kind": "workspace_state",
            "workspace": {
                "app_version": "test",
                "active_tab_id": "tab-foreign",
                "recent_projects": [{ "path": foreign.path() }],
                "tabs": [
                    {
                        "id": "tab-owned",
                        "project_root": project.path(),
                        "workspace": { "windows": [{ "id": "tab-owned::agent-1" }] }
                    },
                    {
                        "id": "tab-foreign",
                        "project_root": foreign.path(),
                        "workspace": { "windows": [{ "id": "tab-foreign::agent-2" }] }
                    }
                ]
            }
        });

        let filtered = scope
            .filter_outbound(workspace.to_string())
            .expect("owned workspace projection");
        let filtered: serde_json::Value =
            serde_json::from_str(&filtered).expect("filtered workspace JSON");
        let tabs = filtered["workspace"]["tabs"]
            .as_array()
            .expect("workspace tabs");
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0]["id"], "tab-owned");
        assert_eq!(filtered["workspace"]["active_tab_id"], "tab-owned");
        assert_eq!(
            filtered["workspace"]["recent_projects"],
            serde_json::json!([])
        );

        assert!(scope
            .filter_outbound(
                serde_json::json!({
                    "kind": "terminal_snapshot",
                    "id": "tab-owned::agent-1",
                    "data_base64": ""
                })
                .to_string()
            )
            .is_some());
        assert!(scope
            .filter_outbound(
                serde_json::json!({
                    "kind": "terminal_snapshot",
                    "id": "tab-foreign::agent-2",
                    "data_base64": ""
                })
                .to_string()
            )
            .is_none());
        assert!(
            scope
                .filter_inbound(FrontendEvent::CloseWindow {
                    id: "tab-owned::agent-1".to_string(),
                    request_id: None,
                })
                .is_none(),
            "Inspection principal must not mutate peer pane lifecycle"
        );
        assert!(scope
            .filter_inbound(FrontendEvent::CloseWindow {
                id: "tab-owned::agent-1".to_string(),
                request_id: Some("72fc3cd4-ad49-43e3-bf3d-d791357643a3".to_string()),
            })
            .is_some());
        assert!(scope
            .filter_inbound(FrontendEvent::CloseWindow {
                id: "tab-foreign::agent-2".to_string(),
                request_id: None,
            })
            .is_none());
        assert!(
            scope
                .filter_inbound(FrontendEvent::PaneSendInput {
                    session_id: "session-1".to_string(),
                    text: "hello".to_string(),
                })
                .is_none(),
            "Inspection principal must not dispatch producing terminal input"
        );
        assert!(scope
            .filter_inbound(FrontendEvent::PaneSendInput {
                session_id: "foreign-claim".to_string(),
                text: "hello".to_string(),
            })
            .is_none());
        assert!(scope
            .filter_inbound(FrontendEvent::TerminalInput {
                id: "tab-owned::agent-1".to_string(),
                data: "not-authorized-on-agent-route".to_string(),
            })
            .is_none());

        let bound_principal = AgentSessionPrincipal::new_bound(
            project.path(),
            "session-1",
            gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: "session-1".to_string(),
                repo_hash: "repo-current".to_string(),
                owner_kind: "issue".to_string(),
                owner_number: 2359,
                identity: gwt_agent::ExecutionBindingIdentity {
                    generation_id: "generation-current".to_string(),
                    binding_id: "binding-current".to_string(),
                    ledger_head_hash: "head-current".to_string(),
                },
                capability_generation: 1,
            },
        )
        .expect("bound agent principal");
        let bound_scope = super::AgentPaneSessionScope::new(super::AgentCapabilityGrant::new(
            "bound-capability".to_string(),
            bound_principal.clone(),
        ));
        assert!(matches!(
            bound_scope.filter_inbound(FrontendEvent::PaneSendInput {
                session_id: "session-1".to_string(),
                text: "hello".to_string(),
            }),
            Some(super::AgentFrontendRequest::SendInput { text }) if text == "hello"
        ));

        let prepared_principal = AgentSessionPrincipal::new_prepared(
            project.path(),
            "session-1",
            bound_principal
                .execution_binding()
                .expect("bound execution binding")
                .clone(),
        )
        .expect("prepared observation principal");
        let mut prepared_scope = super::AgentPaneSessionScope::new(
            super::AgentCapabilityGrant::new("prepared-capability".to_string(), prepared_principal),
        );
        assert!(prepared_scope
            .filter_outbound(workspace.to_string())
            .is_some());
        assert!(
            prepared_scope
                .filter_inbound(FrontendEvent::PaneSendInput {
                    session_id: "session-1".to_string(),
                    text: "must-not-dispatch".to_string(),
                })
                .is_none(),
            "Prepared principal must remain observation-only"
        );
        assert!(
            prepared_scope
                .filter_inbound(FrontendEvent::CloseWindow {
                    id: "tab-owned::agent-1".to_string(),
                    request_id: None,
                })
                .is_none(),
            "Prepared principal must not mutate peer pane lifecycle"
        );
        assert!(prepared_scope
            .filter_outbound(
                serde_json::json!({
                    "kind": "pane_send_result",
                    "ok": true,
                    "window_id": "tab-owned::agent-1",
                    "error": null
                })
                .to_string()
            )
            .is_none());

        let refreshed_workspace = serde_json::json!({
            "kind": "workspace_state",
            "workspace": {
                "active_tab_id": "tab-owned",
                "recent_projects": [],
                "tabs": [{
                    "id": "tab-owned",
                    "project_root": project.path(),
                    "workspace": { "windows": [{ "id": "tab-owned::agent-3" }] }
                }]
            }
        });
        scope
            .filter_outbound(refreshed_workspace.to_string())
            .expect("refreshed owned workspace projection");
        assert!(scope
            .filter_inbound(FrontendEvent::CloseWindow {
                id: "tab-owned::agent-1".to_string(),
                request_id: None,
            })
            .is_none());
        assert!(scope
            .filter_inbound(FrontendEvent::CloseWindow {
                id: "tab-owned::agent-3".to_string(),
                request_id: Some("17e16410-0b91-4382-83f0-625d2a81ee89".to_string()),
            })
            .is_some());
        assert_eq!(
            scope.filter_repair_panes(vec![
                "tab-owned::agent-1".to_string(),
                "tab-owned::agent-3".to_string(),
                "tab-foreign::agent-2".to_string(),
            ]),
            vec!["tab-owned::agent-3".to_string()]
        );
    }

    #[test]
    fn browser_client_scope_preserves_existing_unrestricted_websocket_contract() {
        let mut scope = super::ClientSessionScope::Browser;
        assert!(matches!(
            scope.filter_inbound(FrontendEvent::TerminalInput {
                id: "any-project::terminal-1".to_string(),
                data: "input".to_string(),
            }),
            Some(super::ScopedFrontendRequest::Browser(FrontendEvent::TerminalInput { id, data }))
                if id == "any-project::terminal-1" && data == "input"
        ));

        let payload = serde_json::json!({
            "kind": "workspace_state",
            "workspace": {
                "recent_projects": [{ "path": "/another/project" }],
                "tabs": [{ "id": "another-project" }]
            }
        })
        .to_string();
        assert_eq!(scope.filter_outbound(payload.clone()), Some(payload));
        assert_eq!(
            scope.filter_repair_panes(vec!["any-project::terminal-1".to_string()]),
            vec!["any-project::terminal-1".to_string()]
        );
    }

    #[test]
    fn agent_pane_client_registration_never_enqueues_global_broadcasts() {
        let clients = ClientHub::default();
        let browser = clients.register("browser".to_string());
        let pane = clients.register_pane("pane".to_string());

        clients.dispatch(vec![OutboundEvent::broadcast(terminal_snapshot(
            "foreign-tab::agent-1",
            "foreign snapshot",
        ))]);

        assert!(browser.try_recv().is_some());
        assert!(pane.try_recv().is_none());

        clients.dispatch(vec![OutboundEvent::reply(
            "pane",
            terminal_snapshot("scoped-tab::agent-1", "scoped snapshot"),
        )]);
        assert!(pane.try_recv().is_some());
    }

    fn direct_acceptance_for_test(
        proxy: AppEventProxy,
        ticket_id: &str,
    ) -> AgentSelfCloseDirectAcceptance {
        AgentSelfCloseDirectAcceptance::new(
            "e544de42-fd9f-49a7-9ba2-b8b16ca1572a".to_string(),
            "tab-owned::agent-1".to_string(),
            super::AgentSelfCloseCapabilityTicket {
                id: ticket_id.to_string(),
            },
            proxy,
        )
    }

    fn recorded_self_close_commit_ids(events: &Arc<Mutex<Vec<UserEvent>>>) -> Vec<String> {
        events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|event| match event {
                UserEvent::CommitAgentSelfClose { ticket } => Some(ticket.id().to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn accepted_self_close_send_error_still_finalizes_exactly_once() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut sink = FailingMessageSink;

        runtime.block_on(send_agent_self_close_acceptance(
            &mut sink,
            direct_acceptance_for_test(proxy, "send-error-ticket"),
            Duration::from_secs(1),
        ));

        assert_eq!(
            recorded_self_close_commit_ids(&events),
            vec!["send-error-ticket"]
        );
    }

    #[test]
    fn accepted_self_close_send_timeout_still_finalizes_exactly_once() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut sink = PendingMessageSink;

        runtime.block_on(send_agent_self_close_acceptance(
            &mut sink,
            direct_acceptance_for_test(proxy, "send-timeout-ticket"),
            Duration::from_millis(10),
        ));

        assert_eq!(
            recorded_self_close_commit_ids(&events),
            vec!["send-timeout-ticket"]
        );
    }

    #[test]
    fn accepted_self_close_task_cancellation_still_finalizes_exactly_once() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();

        runtime.block_on(async {
            let acceptance = direct_acceptance_for_test(proxy, "cancelled-task-ticket");
            let task = tokio::spawn(async move {
                let mut sink = PendingMessageSink;
                send_agent_self_close_acceptance(&mut sink, acceptance, Duration::from_secs(60))
                    .await;
            });
            tokio::task::yield_now().await;
            task.abort();
            assert!(task
                .await
                .expect_err("task must be cancelled")
                .is_cancelled());
        });

        assert_eq!(
            recorded_self_close_commit_ids(&events),
            vec!["cancelled-task-ticket"]
        );
    }

    #[test]
    fn correlated_agent_self_close_acceptance_uses_only_the_origin_socket() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let finalizer_proxy = proxy.clone();
        let clients = ClientHub::default();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients.clone(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let issuer = server.agent_capability_issuer();
        let target = issuer
            .issue(project.path(), "session-1")
            .expect("current target");
        let pane_url = issuer.agent_pane_websocket_url().to_string();
        let request_id = "e544de42-fd9f-49a7-9ba2-b8b16ca1572a";
        let window_id = "tab-owned::agent-1";

        let ticket = runtime.block_on(async {
            let mut request = pane_url
                .as_str()
                .into_client_request()
                .expect("agent pane WebSocket request");
            request.headers_mut().insert(
                AUTHORIZATION,
                format!("Bearer {}", target.token)
                    .parse()
                    .expect("bearer header value"),
            );
            let (mut socket, _) = connect_async(request).await.expect("agent pane WebSocket");

            let pane_queue = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let queue = clients
                        .clients
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .values()
                        .find(|registration| !registration.receives_broadcasts)
                        .map(|registration| registration.queue.clone());
                    if let Some(queue) = queue {
                        break queue;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("pane client registration");
            assert!(!pane_queue.enqueue(&PreparedOutbound {
                payload: serde_json::json!({
                    "kind": "workspace_state",
                    "workspace": {
                        "active_tab_id": "tab-owned",
                        "recent_projects": [],
                        "tabs": [{
                            "id": "tab-owned",
                            "project_root": project.path(),
                            "workspace": { "windows": [{ "id": window_id }] }
                        }]
                    }
                })
                .to_string(),
                kind: "workspace_state",
                pane_id: None,
                class: QueueClass::IdempotentLatest,
            }));
            let workspace = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("scoped workspace response")
                .expect("workspace frame")
                .expect("valid workspace frame");
            assert!(matches!(workspace, WebSocketMessage::Text(_)));

            socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "close_window",
                        "id": window_id,
                        "request_id": request_id,
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send correlated close");

            let (grant, responder) = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let dispatched = {
                        let mut recorded = events
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let position = recorded
                            .iter()
                            .position(|event| matches!(event, UserEvent::AgentFrontend { .. }));
                        position.map(|position| recorded.remove(position))
                    };
                    if let Some(UserEvent::AgentFrontend {
                        grant,
                        request:
                            AgentFrontendRequest::CloseWindow {
                                id,
                                request_id: Some(correlation),
                                responder: Some(responder),
                            },
                        ..
                    }) = dispatched
                    {
                        assert_eq!(id, window_id);
                        assert_eq!(correlation, request_id);
                        break (grant, responder);
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("agent close dispatch");
            let ticket = issuer
                .begin_self_close_if_current(&grant)
                .expect("accept current self-close generation");
            responder
                .send(AgentSelfCloseDirectAcceptance::new(
                    request_id.to_string(),
                    window_id.to_string(),
                    ticket,
                    finalizer_proxy.clone(),
                ))
                .expect("origin response task is waiting");

            let response = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("direct close acceptance")
                .expect("acceptance frame")
                .expect("valid acceptance frame");
            let WebSocketMessage::Text(response) = response else {
                panic!("acceptance must be text");
            };
            let response: serde_json::Value =
                serde_json::from_str(response.as_ref()).expect("acceptance JSON");
            assert_eq!(response["kind"], "pane_close_accepted");
            assert_eq!(response["request_id"], request_id);
            assert_eq!(response["window_id"], window_id);
            assert_eq!(
                pane_queue.len(),
                0,
                "the direct acceptance must not pass through ClientHub"
            );

            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let ticket = {
                        let mut recorded = events
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let position = recorded.iter().position(|event| {
                            matches!(event, UserEvent::CommitAgentSelfClose { .. })
                        });
                        position.map(|position| match recorded.remove(position) {
                            UserEvent::CommitAgentSelfClose { ticket } => ticket,
                            _ => unreachable!("matched self-close finalizer"),
                        })
                    };
                    if let Some(ticket) = ticket {
                        break ticket;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("accepted response attempt must schedule finalization")
        });
        assert!(issuer.finish_self_close(&ticket));
        server.shutdown();
    }

    #[test]
    fn correlated_agent_self_close_is_rejected_before_enqueue_after_rotation() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients.clone(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let issuer = server.agent_capability_issuer();
        let original = issuer
            .issue(project.path(), "session-1")
            .expect("original target");
        let pane_url = issuer.agent_pane_websocket_url().to_string();

        runtime.block_on(async {
            let mut request = pane_url
                .as_str()
                .into_client_request()
                .expect("agent pane WebSocket request");
            request.headers_mut().insert(
                AUTHORIZATION,
                format!("Bearer {}", original.token)
                    .parse()
                    .expect("bearer header value"),
            );
            let (mut socket, _) = connect_async(request).await.expect("agent pane WebSocket");
            let pane_queue = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let queue = clients
                        .clients
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .values()
                        .find(|registration| !registration.receives_broadcasts)
                        .map(|registration| registration.queue.clone());
                    if let Some(queue) = queue {
                        break queue;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("pane client registration");
            assert!(!pane_queue.enqueue(&PreparedOutbound {
                payload: serde_json::json!({
                    "kind": "workspace_state",
                    "workspace": {
                        "active_tab_id": "tab-owned",
                        "recent_projects": [],
                        "tabs": [{
                            "id": "tab-owned",
                            "project_root": project.path(),
                            "workspace": {
                                "windows": [{ "id": "tab-owned::agent-1" }]
                            }
                        }]
                    }
                })
                .to_string(),
                kind: "workspace_state",
                pane_id: None,
                class: QueueClass::IdempotentLatest,
            }));
            let _workspace = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("scoped workspace response")
                .expect("workspace frame")
                .expect("valid workspace frame");

            issuer
                .issue(project.path(), "session-1")
                .expect("rotate capability");
            socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "close_window",
                        "id": "tab-owned::agent-1",
                        "request_id": "52185ac8-3d18-470f-bfc3-73fa5eac2ff5",
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send close after rotation");
            let _ = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("rotated correlated socket must close");
        });

        assert!(
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "a rotated correlated close must not enqueue AgentFrontend"
        );
        server.shutdown();
    }

    #[test]
    fn agent_bridge_bind_policy_widens_only_for_native_linux_container_access() {
        let expected = if cfg!(target_os = "linux") {
            IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        };

        assert_eq!(agent_bridge_bind_ip(), expected);
    }

    #[test]
    fn agent_pane_websocket_route_requires_its_capability_and_keeps_browser_ws_open() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let issuer = server.agent_capability_issuer();
        let target = issuer
            .issue(project.path(), "session-1")
            .expect("current target");
        let foreign_token = AgentCapabilityRegistry::default()
            .issue(project.path(), "session-1")
            .expect("foreign-registry capability");
        let agent_pane_url = issuer.agent_pane_websocket_url().to_string();
        let browser_pane_url = issuer.pane_websocket_url().to_string();

        runtime.block_on(async {
            for (case, token) in [("missing", None), ("foreign", Some(foreign_token.as_str()))] {
                let mut request = agent_pane_url
                    .as_str()
                    .into_client_request()
                    .expect("agent pane WebSocket request");
                if let Some(token) = token {
                    request.headers_mut().insert(
                        AUTHORIZATION,
                        format!("Bearer {token}")
                            .parse()
                            .expect("bearer header value"),
                    );
                }

                match connect_async(request).await {
                    Err(WebSocketError::Http(response)) => assert_eq!(
                        response.status().as_u16(),
                        StatusCode::UNAUTHORIZED.as_u16(),
                        "{case} capability must be rejected during the handshake"
                    ),
                    Err(error) => panic!("{case} handshake returned the wrong error: {error}"),
                    Ok((socket, _)) => {
                        drop(socket);
                        panic!("{case} capability unexpectedly upgraded")
                    }
                }
            }

            let mut authorized_request = agent_pane_url
                .as_str()
                .into_client_request()
                .expect("authorized agent pane WebSocket request");
            authorized_request.headers_mut().insert(
                AUTHORIZATION,
                format!("Bearer {}", target.token)
                    .parse()
                    .expect("authorized bearer header value"),
            );
            let (mut authorized_socket, response) = connect_async(authorized_request)
                .await
                .expect("authorized agent pane WebSocket upgrade");
            assert_eq!(
                response.status().as_u16(),
                StatusCode::SWITCHING_PROTOCOLS.as_u16()
            );
            authorized_socket
                .close(None)
                .await
                .expect("close authorized agent pane WebSocket");

            let (mut browser_socket, response) = connect_async(browser_pane_url.as_str())
                .await
                .expect("browser WebSocket remains token-free");
            assert_eq!(
                response.status().as_u16(),
                StatusCode::SWITCHING_PROTOCOLS.as_u16()
            );
            browser_socket
                .close(None)
                .await
                .expect("close browser WebSocket");
        });

        let records = server.access_log().snapshot();
        assert!(records.iter().any(|record| {
            record.path == "/internal/pane-ws" && record.status == StatusCode::UNAUTHORIZED.as_u16()
        }));
        assert!(records.iter().any(|record| {
            record.path == "/internal/pane-ws"
                && record.status == StatusCode::SWITCHING_PROTOCOLS.as_u16()
        }));
        assert!(records.iter().any(|record| {
            record.path == "/ws" && record.status == StatusCode::SWITCHING_PROTOCOLS.as_u16()
        }));

        server.shutdown();
    }

    #[test]
    fn connected_agent_pane_socket_stops_dispatching_after_rotation_and_revoke() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let issuer = server.agent_capability_issuer();
        let original = issuer
            .issue(project.path(), "session-1")
            .expect("original capability");
        let pane_url = issuer.agent_pane_websocket_url().to_string();
        let ready = r#"{"kind":"frontend_ready"}"#.to_string();

        runtime.block_on(async {
            let connect = |token: &str| {
                let mut request = pane_url
                    .as_str()
                    .into_client_request()
                    .expect("agent pane WebSocket request");
                request.headers_mut().insert(
                    AUTHORIZATION,
                    format!("Bearer {token}")
                        .parse()
                        .expect("bearer header value"),
                );
                request
            };

            let (mut original_socket, _) = connect_async(connect(&original.token))
                .await
                .expect("original agent pane WebSocket");
            original_socket
                .send(WebSocketMessage::Text(ready.clone().into()))
                .await
                .expect("send ready on original socket");
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if !events
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_empty()
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("original ready dispatch");
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();

            let current = issuer
                .issue(project.path(), "session-1")
                .expect("rotated capability");
            original_socket
                .send(WebSocketMessage::Text(ready.clone().into()))
                .await
                .expect("send ready after rotation");
            let _ = tokio::time::timeout(Duration::from_secs(1), original_socket.next())
                .await
                .expect("rotated socket must be closed by the server");
            assert!(
                events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_empty(),
                "a rotated socket must not enqueue an AgentFrontend event"
            );

            let (mut current_socket, _) = connect_async(connect(&current.token))
                .await
                .expect("current agent pane WebSocket");
            current_socket
                .send(WebSocketMessage::Text(ready.clone().into()))
                .await
                .expect("send ready on current socket");
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if !events
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_empty()
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("current ready dispatch");
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();

            assert!(issuer.revoke_token(&current.token));
            current_socket
                .send(WebSocketMessage::Text(ready.into()))
                .await
                .expect("send ready after revoke");
            let _ = tokio::time::timeout(Duration::from_secs(1), current_socket.next())
                .await
                .expect("revoked socket must be closed by the server");
            assert!(
                events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_empty(),
                "a revoked socket must not enqueue an AgentFrontend event"
            );
        });

        server.shutdown();
    }

    #[test]
    fn accepted_self_close_makes_grant_non_current_until_ticket_finishes() {
        let project = tempfile::tempdir().expect("project tempdir");
        let issuer = super::AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:1/internal/hook-live",
            "ws://127.0.0.1:1/ws",
            "ws://127.0.0.1:2/internal/pane-ws",
        );
        let original = issuer
            .issue(project.path(), "session-1")
            .expect("original capability");
        let grant = issuer
            .grant_for_test(&original.token)
            .expect("current grant");

        let ticket = issuer
            .begin_self_close_if_current(&grant)
            .expect("begin self-close");
        assert!(!issuer.grant_is_current(&grant));
        assert!(!issuer.authenticates_token(&original.token));
        assert!(
            issuer.issue(project.path(), "session-1").is_err(),
            "the same principal cannot reissue while its close ticket is pending"
        );

        assert!(issuer.rollback_self_close(&ticket));
        assert!(issuer.grant_is_current(&grant));
        assert!(issuer.authenticates_token(&original.token));

        let ticket = issuer
            .begin_self_close_if_current(&grant)
            .expect("begin accepted self-close");
        assert!(issuer.revoke_token(&original.token));
        assert!(
            !issuer.rollback_self_close(&ticket),
            "an independently revoked closing grant must never become active again"
        );
        assert!(!issuer.grant_is_current(&grant));

        let replacement = issuer
            .issue(project.path(), "session-1")
            .expect("reissue after revoked ticket clears");
        let replacement_grant = issuer
            .grant_for_test(&replacement.token)
            .expect("replacement grant");
        let ticket = issuer
            .begin_self_close_if_current(&replacement_grant)
            .expect("begin replacement self-close");
        assert!(issuer.finish_self_close(&ticket));
        assert!(
            !issuer.finish_self_close(&ticket),
            "ticket replay must be a no-op"
        );
        assert!(issuer.issue(project.path(), "session-1").is_ok());
    }

    #[test]
    fn agent_bridge_uses_capability_only_listener_and_rejects_stale_or_foreign_tokens() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let foreign_project = tempfile::tempdir().expect("foreign project tempdir");
        let issuer = server.agent_capability_issuer();
        let pane_websocket_url = issuer.pane_websocket_url().to_string();
        let stale = issuer
            .issue(project.path(), "session-1")
            .expect("stale target");
        let current = issuer
            .issue(project.path(), "session-1")
            .expect("current target");
        let foreign = issuer
            .issue(foreign_project.path(), "session-2")
            .expect("foreign target");
        let client = reqwest::blocking::Client::new();

        assert_ne!(
            reqwest::Url::parse(server.url())
                .expect("browser URL")
                .port_or_known_default(),
            reqwest::Url::parse(&current.url)
                .expect("agent URL")
                .port_or_known_default(),
        );
        assert_eq!(
            reqwest::Url::parse(&pane_websocket_url)
                .expect("pane WebSocket URL")
                .port_or_known_default(),
            reqwest::Url::parse(server.url())
                .expect("browser URL")
                .port_or_known_default(),
        );
        assert_ne!(
            reqwest::Url::parse(&pane_websocket_url)
                .expect("pane WebSocket URL")
                .port_or_known_default(),
            reqwest::Url::parse(&current.url)
                .expect("agent URL")
                .port_or_known_default(),
        );
        assert_eq!(
            reqwest::Url::parse(&current.url)
                .expect("agent URL")
                .host_str(),
            Some("127.0.0.1")
        );

        let agent_health = client
            .get(
                reqwest::Url::parse(&current.url)
                    .expect("agent URL")
                    .join("/healthz")
                    .expect("agent health URL"),
            )
            .send()
            .expect("agent health request");
        assert_eq!(agent_health.status(), HttpStatusCode::NOT_FOUND);

        let browser_hook = client
            .post(format!("{}internal/hook-live", server.url()))
            .json(&sample_runtime_hook_event())
            .send()
            .expect("browser hook request");
        assert_eq!(browser_hook.status(), HttpStatusCode::NOT_FOUND);

        let stale_response = client
            .post(&stale.url)
            .bearer_auth(&stale.token)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("stale hook request");
        assert_eq!(stale_response.status(), HttpStatusCode::UNAUTHORIZED);

        let foreign_response = client
            .post(&foreign.url)
            .bearer_auth(&foreign.token)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("foreign hook request");
        assert_eq!(foreign_response.status(), HttpStatusCode::UNAUTHORIZED);

        let accepted = client
            .post(&current.url)
            .bearer_auth(&current.token)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("current hook request");
        assert_eq!(accepted.status(), HttpStatusCode::NO_CONTENT);

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let [UserEvent::RuntimeHook(recorded_event)] = recorded.as_slice() else {
            panic!("only the current matching capability should dispatch: {recorded:?}");
        };
        let canonical_project = dunce::canonicalize(project.path())
            .expect("canonical project")
            .to_string_lossy()
            .into_owned();
        assert_eq!(recorded_event.gwt_session_id.as_deref(), Some("session-1"));
        assert_eq!(
            recorded_event.project_root.as_deref(),
            Some(canonical_project.as_str())
        );

        drop(recorded);
        server.shutdown();
    }

    #[test]
    fn workspace_update_route_authenticates_before_host_mutation_service() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let foreign_project = tempfile::tempdir().expect("foreign project tempdir");
        let issuer = server.agent_capability_issuer();
        let stale = issuer
            .issue(project.path(), "session-1")
            .expect("stale target");
        let current = issuer
            .issue(project.path(), "session-1")
            .expect("current target");
        let foreign = AgentCapabilityIssuer::new(
            current.url.clone(),
            issuer.pane_websocket_url().to_string(),
            issuer.agent_pane_websocket_url().to_string(),
            AgentCapabilityRegistry::default(),
        )
        .issue(foreign_project.path(), "session-1")
        .expect("foreign-registry target");
        let mut workspace_update_url = reqwest::Url::parse(&current.url).expect("agent hook URL");
        workspace_update_url.set_path("/internal/workspace-update");
        let request = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "different-session",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            },
            "intent": {}
        });
        let client = reqwest::blocking::Client::new();

        let browser_response = client
            .post(format!("{}internal/workspace-update", server.url()))
            .json(&request)
            .send()
            .expect("browser workspace-update request");
        assert_eq!(browser_response.status(), HttpStatusCode::NOT_FOUND);

        for (case, token) in [
            ("missing", None),
            ("stale", Some(stale.token.as_str())),
            ("foreign", Some(foreign.token.as_str())),
        ] {
            let mut request_builder = client.post(workspace_update_url.clone()).json(&request);
            if let Some(token) = token {
                request_builder = request_builder.bearer_auth(token);
            }
            let response = request_builder
                .send()
                .unwrap_or_else(|error| panic!("{case} workspace-update request: {error}"));
            assert_eq!(
                response.status(),
                HttpStatusCode::UNAUTHORIZED,
                "{case} bearer must be rejected before Host mutation"
            );
            let body = response.text().expect("unauthorized response body");
            assert!(!body.contains(&stale.token));
            assert!(!body.contains(&foreign.token));
        }

        let current_response = client
            .post(workspace_update_url)
            .bearer_auth(&current.token)
            .json(&request)
            .send()
            .expect("current workspace-update request");
        assert_eq!(current_response.status(), HttpStatusCode::CONFLICT);
        let error: serde_json::Value = current_response
            .json()
            .expect("Host mutation service error body");
        assert_eq!(error["code"], "execution_binding_mismatch");
        assert!(error["message"]
            .as_str()
            .is_some_and(|message| message.contains("Execution binding")));
        assert!(!error.to_string().contains(&current.token));
        assert!(events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());

        server.shutdown();
    }

    #[test]
    fn execution_binding_probe_route_rejects_inspection_principal_without_mutation() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let target = server
            .agent_capability_issuer()
            .issue(project.path(), "session-inspection")
            .expect("inspection target");
        let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
        url.set_path("/internal/execution-binding-probe");
        let request = serde_json::json!({
            "schema_version": gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            "operation_id": "operation-inspection",
            "nonce": "nonce-inspection"
        });
        let client = reqwest::blocking::Client::new();

        let response = client
            .post(url)
            .bearer_auth(&target.token)
            .json(&request)
            .send()
            .expect("inspection binding probe");

        assert_eq!(response.status(), HttpStatusCode::CONFLICT);
        let error: serde_json::Value = response.json().expect("binding probe error");
        assert_eq!(error["code"], "execution_binding_mismatch");
        assert!(events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());
        server.shutdown();
    }

    #[test]
    fn execution_binding_probe_route_rejects_prepared_authority_until_activation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repository");
        for args in [
            vec!["init", "-q"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/prepared-probe.git",
            ],
        ] {
            let output = gwt_core::process::hidden_command("git")
                .args(&args)
                .current_dir(&repo)
                .output()
                .expect("run fixture git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let repo = dunce::canonicalize(repo).expect("canonical repository");
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 2359,
        };
        let completed_at = chrono::Utc::now();
        gwt::cli::execution_state::save(
            &repo,
            &gwt::cli::execution_state::ExecutionControlRecord {
                owner_kind: owner.kind,
                owner_number: owner.number,
                primary_session_id: "session-predecessor".to_string(),
                entrypoint: "gwt-execute".to_string(),
                bundled_required_owners: Vec::new(),
                status: gwt::cli::execution_state::ExecutionControlStatus::Completed,
                blocked_reason: None,
                missing_verification: None,
                launched_at: completed_at,
                settled_at: Some(completed_at),
                transfers: Vec::new(),
                recoveries: Vec::new(),
                content_hash: String::new(),
            },
        )
        .expect("save completed predecessor");
        gwt::cli::execution_state::ensure_generation_ledger(
            &repo,
            owner,
            gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
        )
        .expect("import completed predecessor");
        let continuation_session_id = "session-prepared-probe";
        let request = gwt::cli::execution_state::SuccessorRequest {
            operation_id: "operation-prepared-probe".to_string(),
            principal_id: "host-prepared-probe".to_string(),
            work_id: Some("work-prepared-probe".to_string()),
            source: "continue-work".to_string(),
            session_binding_id: "binding-prepared-probe".to_string(),
            initial_session_id: continuation_session_id.to_string(),
            entrypoint: "resume".to_string(),
            requested_at: chrono::Utc::now(),
        };
        gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
            .expect("prepare successor");
        let planned_identity =
            gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
                .expect("derive Prepared binding");
        let mut session =
            gwt_agent::Session::new(&repo, "work/prepared-probe", gwt_agent::AgentId::Codex);
        session.id = continuation_session_id.to_string();
        session.project_state_root = Some(repo.clone());
        session.linked_issue_number = Some(owner.number);
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: session.repo_hash.clone().expect("repository hash"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: planned_identity.clone(),
            capability_generation: 1,
        };
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind Prepared Session");
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("persist Prepared Session");

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let target = server
            .agent_capability_issuer()
            .issue_prepared(&repo, continuation_session_id, binding.clone())
            .expect("Prepared Host capability");
        let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
        url.set_path("/internal/execution-binding-probe");
        let response = reqwest::blocking::Client::new()
            .post(url)
            .bearer_auth(&target.token)
            .json(&serde_json::json!({
                "schema_version": gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                "operation_id": "operation-prepared-probe",
                "nonce": "nonce-prepared-probe"
            }))
            .send()
            .expect("Prepared binding probe");

        assert_eq!(
            response.status(),
            HttpStatusCode::CONFLICT,
            "the agent-facing mutation probe must require Active authority",
        );
        assert!(
            gwt::cli::execution_state::prepared_execution_binding_matches(
                &repo,
                owner,
                continuation_session_id,
                &binding.identity,
            )
            .expect("Prepared authority remains pending")
        );
        assert_eq!(
            gwt::cli::execution_state::load_generation_ledger(&repo, owner)
                .expect("read generation ledger")
                .expect("generation ledger")
                .current_effective_status(),
            Some(gwt::cli::execution_state::ExecutionControlStatus::Completed),
            "an HTTP probe must not activate the successor"
        );
        assert!(
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "the probe is side-effect-free at the runtime dispatch boundary"
        );
        server.shutdown();
    }

    #[test]
    fn execution_binding_probe_fences_an_older_host_with_the_durable_capability_epoch() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = tempfile::tempdir().expect("fixture root");
        let repo = fixture.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repository fixture");
        for args in [
            vec!["init"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test User"],
            vec!["checkout", "-b", "work/execution-binding-probe"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/execution-binding-probe.git",
            ],
            vec!["commit", "--allow-empty", "-m", "initial"],
        ] {
            let output =
                gwt_core::process::run_git_logged(&args, Some(&repo)).expect("run fixture git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let repo = dunce::canonicalize(repo).expect("canonical repository fixture");
        let mut session = gwt_agent::Session::new(
            &repo,
            "work/execution-binding-probe",
            gwt_agent::AgentId::Codex,
        );
        session.id = "session-two-host".to_string();
        session.project_state_root = Some(repo.clone());
        session.linked_issue_number = Some(2359);
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save durable Session");
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 2359,
        };
        gwt::cli::execution_state::materialize_at_launch(
            &repo,
            owner.kind,
            owner.number,
            &session.id,
            "gwt-execute",
            false,
        )
        .expect("materialize execution projection");
        gwt::cli::execution_state::ensure_generation_ledger(
            &repo,
            owner,
            gwt::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize owner ledger");
        let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
            .expect("read current binding")
            .expect("active generation binding");
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: session
                .repo_hash
                .clone()
                .expect("Session repository identity"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation: 1,
        };
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind Session to active generation");
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("persist initial execution binding");

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy_a, events_a) = AppEventProxy::stub();
        let mut server_a = EmbeddedServer::start(
            &runtime,
            proxy_a,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("first Host");
        let target_a = server_a
            .agent_capability_issuer()
            .issue_bound(&repo, &session.id, binding)
            .expect("first Host binding");
        let pane_url_a = server_a
            .agent_capability_issuer()
            .agent_pane_websocket_url()
            .to_string();
        let mut pane_request_a = pane_url_a
            .as_str()
            .into_client_request()
            .expect("old Host pane request");
        pane_request_a.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", target_a.token)
                .parse()
                .expect("old Host bearer"),
        );
        let (mut old_host_socket, _) = runtime
            .block_on(connect_async(pane_request_a))
            .expect("old Host socket is current before rotation");

        let (proxy_b, events_b) = AppEventProxy::stub();
        let mut server_b = EmbeddedServer::start(
            &runtime,
            proxy_b,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("second Host");
        let rotated = gwt_agent::rotate_session_execution_capability(
            &gwt_core::paths::gwt_sessions_dir(),
            &session.id,
        )
        .expect("rotate durable Host epoch");
        let target_b = server_b
            .agent_capability_issuer()
            .issue_bound(&repo, &session.id, rotated.clone())
            .expect("second Host binding");

        runtime.block_on(async {
            old_host_socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "pane_send_input",
                        "session_id": &session.id,
                        "text": "must-not-dispatch"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send input through old Host socket");
            let close = tokio::time::timeout(Duration::from_secs(2), old_host_socket.next())
                .await
                .expect("old Host socket must be fenced")
                .expect("old Host close frame")
                .expect("valid old Host close frame");
            let WebSocketMessage::Close(Some(close)) = close else {
                panic!("old Host socket must receive an explicit policy close");
            };
            assert_eq!(u16::from(close.code), 1008);
            assert_eq!(close.reason, "execution binding is no longer current");
            assert!(!close.reason.contains(&target_a.token));
            assert!(!close.reason.contains(&rotated.identity.binding_id));
            assert!(!close.reason.contains(repo.to_string_lossy().as_ref()));
        });
        assert!(
            events_a
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "old Host input must be rejected before AgentFrontend dispatch"
        );

        let mut stale_handshake = pane_url_a
            .as_str()
            .into_client_request()
            .expect("stale Host pane request");
        stale_handshake.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", target_a.token)
                .parse()
                .expect("stale Host bearer"),
        );
        match runtime.block_on(connect_async(stale_handshake)) {
            Err(WebSocketError::Http(response)) => {
                assert_eq!(response.status(), HttpStatusCode::CONFLICT);
            }
            Ok(_) => panic!("durably stale Host capability must not upgrade"),
            Err(error) => panic!("unexpected stale Host handshake error: {error}"),
        }

        let pane_url_b = server_b
            .agent_capability_issuer()
            .agent_pane_websocket_url()
            .to_string();
        let mut pane_request_b = pane_url_b
            .as_str()
            .into_client_request()
            .expect("current Host pane request");
        pane_request_b.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", target_b.token)
                .parse()
                .expect("current Host bearer"),
        );
        runtime.block_on(async {
            let (mut current_host_socket, _) = connect_async(pane_request_b)
                .await
                .expect("current Host socket upgrades");
            current_host_socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "pane_send_input",
                        "session_id": &session.id,
                        "text": "current-dispatch"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send input through current Host socket");
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if events_b
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .iter()
                        .any(|event| matches!(event, UserEvent::AgentFrontend { .. }))
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("current Host input reaches runtime dispatch queue");
            current_host_socket
                .close(None)
                .await
                .expect("close current Host socket");
        });

        let request = serde_json::json!({
            "schema_version": gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            "operation_id": "operation-two-host",
            "nonce": "nonce-two-host"
        });
        let client = reqwest::blocking::Client::new();
        let probe = |target: &HookForwardTarget| {
            let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
            url.set_path("/internal/execution-binding-probe");
            client
                .post(url)
                .bearer_auth(&target.token)
                .json(&request)
                .send()
                .expect("binding probe request")
        };

        let stale = probe(&target_a);
        assert_eq!(stale.status(), HttpStatusCode::CONFLICT);
        let current = probe(&target_b);
        assert_eq!(current.status(), HttpStatusCode::OK);
        let receipt: gwt::AgentExecutionBindingProbeReceipt =
            current.json().expect("current Host receipt");
        assert_eq!(receipt.execution_binding, rotated.identity);
        assert_eq!(receipt.capability_generation, rotated.capability_generation);
        assert!(!receipt.host_instance_id.trim().is_empty());

        let dispatched_before_corruption = events_b
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        let mut corrupt_session_request = pane_url_b
            .as_str()
            .into_client_request()
            .expect("corrupt Session pane request");
        corrupt_session_request.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", target_b.token)
                .parse()
                .expect("current Host bearer"),
        );
        runtime.block_on(async {
            let (mut current_host_socket, _) = connect_async(corrupt_session_request)
                .await
                .expect("current Host socket upgrades before Session corruption");
            std::fs::write(
                gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id)),
                "{",
            )
            .expect("corrupt durable Session fixture");
            current_host_socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "pane_send_input",
                        "session_id": &session.id,
                        "text": "must-not-dispatch-when-authority-is-unavailable"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send input after Session corruption");
            let close = tokio::time::timeout(Duration::from_secs(2), current_host_socket.next())
                .await
                .expect("current Host socket must fail closed")
                .expect("current Host close frame")
                .expect("valid current Host close frame");
            let WebSocketMessage::Close(Some(close)) = close else {
                panic!("unknown durable authority must receive an explicit internal-error close");
            };
            assert_eq!(u16::from(close.code), 1011);
            assert_eq!(close.reason, "execution authority is unavailable");
            assert!(!close.reason.contains(&target_b.token));
            assert!(!close.reason.contains(&session.id));
            assert!(!close.reason.contains(repo.to_string_lossy().as_ref()));
        });
        assert_eq!(
            events_b
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            dispatched_before_corruption,
            "corrupt durable authority must be rejected before AgentFrontend dispatch"
        );

        server_a.shutdown();
        server_b.shutdown();
    }

    #[test]
    fn work_terminalization_route_authenticates_before_host_mutation_service() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let target = server
            .agent_capability_issuer()
            .issue(project.path(), "session-1")
            .expect("terminalization target");
        let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
        url.set_path("/internal/work-terminalization");
        let request = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "different-session",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            },
            "terminal_kind": "done"
        });
        let client = reqwest::blocking::Client::new();

        let browser_response = client
            .post(format!("{}internal/work-terminalization", server.url()))
            .json(&request)
            .send()
            .expect("browser terminalization request");
        assert_eq!(browser_response.status(), HttpStatusCode::NOT_FOUND);

        let unauthorized = client
            .post(url.clone())
            .json(&request)
            .send()
            .expect("unauthorized terminalization request");
        assert_eq!(unauthorized.status(), HttpStatusCode::UNAUTHORIZED);

        let authenticated = client
            .post(url)
            .bearer_auth(&target.token)
            .json(&request)
            .send()
            .expect("authenticated terminalization request");
        assert_eq!(authenticated.status(), HttpStatusCode::CONFLICT);
        let error: serde_json::Value = authenticated
            .json()
            .expect("terminalization service error body");
        assert_eq!(error["code"], "execution_binding_mismatch");
        assert!(error["message"]
            .as_str()
            .is_some_and(|message| message.contains("Execution binding")));
        assert!(events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());

        server.shutdown();
    }

    #[test]
    fn handle_frontend_message_forwards_non_terminal_events_to_proxy() {
        let (state, events) = sample_server_state();

        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(0),
            FrontendEvent::FrontendReady,
        );

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(matches!(
            recorded.as_slice(),
            [UserEvent::Frontend { client_id, event: FrontendEvent::FrontendReady }]
                if client_id == "client-1"
        ));
    }

    #[test]
    fn handle_frontend_message_falls_back_to_proxy_when_pty_writer_is_missing() {
        let (state, events) = sample_server_state();

        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(0),
            FrontendEvent::TerminalInput {
                id: "tab-1::shell-1".to_string(),
                data: "ls\n".to_string(),
            },
        );

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(matches!(
            recorded.as_slice(),
            [UserEvent::Frontend { client_id, event: FrontendEvent::TerminalInput { id, data } }]
                if client_id == "client-1"
                    && id == "tab-1::shell-1"
                    && data == "ls\n"
        ));
    }

    fn terminal_output(pane: &str, data: &str) -> BackendEvent {
        BackendEvent::TerminalOutput {
            id: pane.to_string(),
            data_base64: data.to_string(),
        }
    }

    fn terminal_snapshot(pane: &str, data: &str) -> BackendEvent {
        BackendEvent::TerminalSnapshot {
            id: pane.to_string(),
            data_base64: data.to_string(),
        }
    }

    fn lossless_error(message: &str) -> BackendEvent {
        BackendEvent::ReleaseNotesError {
            id: "release-notes-1".to_string(),
            message: message.to_string(),
        }
    }

    fn index_status(message: &str) -> BackendEvent {
        BackendEvent::ProjectIndexStatus {
            project_root: "/tmp/project".to_string(),
            status: gwt::ProjectIndexStatusView::new(
                gwt::ProjectIndexStatusState::Skipped,
                message,
            ),
        }
    }

    fn drain_all(queue: &ClientQueue) -> (Vec<String>, Vec<String>) {
        let mut payloads = Vec::new();
        let mut repairs = Vec::new();
        while let Some(step) = queue.try_next() {
            match step {
                DrainStep::Message {
                    payload,
                    repair_panes,
                } => {
                    payloads.push(payload);
                    repairs.extend(repair_panes);
                }
                DrainStep::Closed(_) => break,
            }
        }
        (payloads, repairs)
    }

    // SPEC-2359 W-17 (FR-394/FR-395): queue pressure must never disconnect a
    // client for lossy traffic — only drop the lossy entries themselves.
    #[test]
    fn client_queue_drops_lossy_at_high_water_without_disconnect() {
        let queue = ClientQueue::default();

        for index in 0..(LOSSY_HIGH_WATER + 50) {
            queue.enqueue(&prepare_outbound(&terminal_output(
                "tab-1::agent-1",
                &format!("chunk-{index}"),
            )));
        }

        assert!(!queue.is_dead(), "lossy flood must not kill the client");
        assert_eq!(queue.len(), LOSSY_HIGH_WATER, "queue capped at high water");
        assert_eq!(queue.dropped_lossy(), 50, "overflow entries are dropped");
    }

    #[test]
    fn client_hub_health_stats_summarizes_queue_pressure() {
        let hub = ClientHub::default();
        let queue_a = hub.register("client-a".to_string());
        let queue_b = hub.register("client-b".to_string());

        for index in 0..(LOSSY_HIGH_WATER + 3) {
            queue_a.enqueue(&prepare_outbound(&terminal_output(
                "tab-1::agent-1",
                &format!("chunk-{index}"),
            )));
        }
        queue_b.enqueue(&prepare_outbound(&lossless_error("must arrive")));

        let stats = hub.health_stats();
        assert_eq!(stats.client_count, 2);
        assert_eq!(stats.queued_entries, LOSSY_HIGH_WATER + 1);
        assert_eq!(stats.dirty_panes, 1);
        assert_eq!(stats.dropped_lossy, 3);
        assert_eq!(stats.dead_clients, 0);
    }

    // SPEC-2359 W-17 (FR-395): lossless events must survive any lossy flood.
    #[test]
    fn client_queue_keeps_lossless_under_lossy_flood() {
        let queue = ClientQueue::default();

        for index in 0..(LOSSY_HIGH_WATER * 2) {
            queue.enqueue(&prepare_outbound(&terminal_output(
                "tab-1::agent-1",
                &format!("flood-{index}"),
            )));
        }
        for index in 0..5 {
            queue.enqueue(&prepare_outbound(&lossless_error(&format!(
                "must-arrive-{index}"
            ))));
        }
        for index in 0..LOSSY_HIGH_WATER {
            queue.enqueue(&prepare_outbound(&terminal_output(
                "tab-1::agent-1",
                &format!("flood-tail-{index}"),
            )));
        }

        let (payloads, _) = drain_all(&queue);
        for index in 0..5 {
            let marker = format!("must-arrive-{index}");
            assert!(
                payloads.iter().any(|payload| payload.contains(&marker)),
                "lossless payload {marker} must be delivered"
            );
        }
        assert!(!queue.is_dead());
    }

    // SPEC-2359 W-17 (FR-394): IdempotentLatest kinds keep one entry holding
    // the latest payload (server-side LatestWins).
    #[test]
    fn client_queue_replaces_idempotent_latest_in_place() {
        let queue = ClientQueue::default();

        queue.enqueue(&prepare_outbound(&index_status("first")));
        queue.enqueue(&prepare_outbound(&lossless_error("between")));
        queue.enqueue(&prepare_outbound(&index_status("latest")));

        let (payloads, _) = drain_all(&queue);
        let index_payloads: Vec<&String> = payloads
            .iter()
            .filter(|payload| payload.contains("\"kind\":\"project_index_status\""))
            .collect();
        assert_eq!(index_payloads.len(), 1, "only one queued entry per kind");
        assert!(
            index_payloads[0].contains("latest"),
            "queued entry must carry the latest payload"
        );
        assert!(
            payloads[0].contains("project_index_status"),
            "replacement keeps the original queue position"
        );
    }

    // SPEC-2359 W-17 (FR-396/FR-397): snapshots dedupe per pane so a replay
    // burst cannot accumulate stale snapshots, while staying lossless.
    #[test]
    fn client_queue_replaces_snapshot_per_pane() {
        let queue = ClientQueue::default();

        queue.enqueue(&prepare_outbound(&terminal_snapshot("pane-a", "a-v1")));
        queue.enqueue(&prepare_outbound(&terminal_snapshot("pane-b", "b-v1")));
        queue.enqueue(&prepare_outbound(&terminal_snapshot("pane-a", "a-v2")));

        let (payloads, _) = drain_all(&queue);
        assert_eq!(payloads.len(), 2, "one snapshot per pane");
        assert!(
            payloads.iter().any(|payload| payload.contains("a-v2")),
            "pane-a keeps only the newest snapshot"
        );
        assert!(
            !payloads.iter().any(|payload| payload.contains("a-v1")),
            "stale pane-a snapshot is superseded"
        );
        assert!(payloads.iter().any(|payload| payload.contains("b-v1")));
    }

    // SPEC-2359 W-17 (FR-395): disconnect is the last resort, reached only via
    // the lossless hard cap (a truly stuck client).
    #[test]
    fn client_queue_goes_dead_only_at_lossless_hard_cap() {
        let queue = ClientQueue::default();

        for index in 0..LOSSLESS_HARD_CAP {
            let dead = queue.enqueue(&prepare_outbound(&lossless_error(&format!("fill-{index}"))));
            assert!(!dead, "client stays alive until the hard cap");
        }
        assert!(!queue.is_dead());

        let dead = queue.enqueue(&prepare_outbound(&lossless_error("overflow")));
        assert!(dead, "hard cap overflow marks the client dead");
        assert!(queue.is_dead());
        assert!(
            matches!(queue.try_next(), Some(DrainStep::Closed(_))),
            "dead queue reports Closed to the drain loop"
        );
    }

    // SPEC-2359 W-17 (FR-396): dropped pane output self-heals via a snapshot
    // repair request once the queue drains below the low-water mark.
    #[test]
    fn client_queue_surfaces_repair_panes_after_drain_below_low_water() {
        let queue = ClientQueue::default();

        for index in 0..(LOSSY_HIGH_WATER + 10) {
            queue.enqueue(&prepare_outbound(&terminal_output(
                "tab-1::agent-7",
                &format!("chunk-{index}"),
            )));
        }

        let (payloads, repairs) = drain_all(&queue);
        assert_eq!(payloads.len(), LOSSY_HIGH_WATER);
        assert_eq!(
            repairs,
            vec!["tab-1::agent-7".to_string()],
            "dropped pane is reported exactly once for snapshot repair"
        );
        assert!(
            queue.len() < DRAIN_LOW_WATER,
            "repair fires only below the low-water mark"
        );
    }

    // SPEC-2359 W-17 (FR-394): kinds missing from BACKEND_EVENT_POLICIES are
    // treated as lossless so new events can never be silently dropped.
    #[test]
    fn queue_class_falls_back_to_lossless_for_unknown_kind() {
        assert_eq!(
            queue_class_for_kind("definitely_not_a_kind"),
            QueueClass::Lossless
        );
        assert_eq!(queue_class_for_kind("terminal_output"), QueueClass::Lossy);
        assert_eq!(
            queue_class_for_kind("project_index_status"),
            QueueClass::IdempotentLatest
        );
        assert_eq!(
            queue_class_for_kind("terminal_snapshot"),
            QueueClass::SnapshotLatest
        );
        assert_eq!(
            queue_class_for_kind("release_notes_error"),
            QueueClass::Lossless
        );
    }

    // SPEC-2359 W-17 (FR-394): Snapshot-class kinds without an extracted pane
    // id (file trees, release notes, resume acks) must append — replacing by
    // kind alone would let unrelated windows clobber each other's payloads.
    #[test]
    fn client_queue_appends_snapshot_kinds_without_pane_id() {
        let queue = ClientQueue::default();

        let payload_for = |id: &str| BackendEvent::ReleaseNotesPayload {
            id: id.to_string(),
            entries: Vec::new(),
            focus_version: None,
            current_version: "1.0.0".to_string(),
        };
        queue.enqueue(&prepare_outbound(&payload_for("window-1")));
        queue.enqueue(&prepare_outbound(&payload_for("window-2")));

        let (payloads, _) = drain_all(&queue);
        assert_eq!(payloads.len(), 2, "distinct windows must both be delivered");
        assert!(payloads.iter().any(|payload| payload.contains("window-1")));
        assert!(payloads.iter().any(|payload| payload.contains("window-2")));
    }

    // SPEC-2359 W-17 (FR-395/SC-263): the dispatch path keeps clients
    // registered under a terminal output flood — the requesting client must
    // still receive lossless replies afterwards.
    #[test]
    fn client_hub_keeps_client_registered_under_terminal_output_flood() {
        let hub = ClientHub::default();
        let queue = hub.register("busy-client".to_string());

        for index in 0..(LOSSY_HIGH_WATER * 4) {
            hub.dispatch(vec![OutboundEvent::broadcast(terminal_output(
                "tab-1::agent-1",
                &format!("chunk-{index}"),
            ))]);
        }

        {
            let clients = hub
                .clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert!(
                clients.contains_key("busy-client"),
                "lossy flood must not evict the client"
            );
        }

        hub.dispatch(vec![OutboundEvent::broadcast(lossless_error(
            "after-flood",
        ))]);
        let (payloads, _) = drain_all(&queue);
        assert!(
            payloads
                .iter()
                .any(|payload| payload.contains("after-flood")),
            "lossless reply still reaches the client after the flood"
        );
    }

    // SPEC-2359 W-17 (FR-395): only the lossless hard cap unregisters a
    // client (replacement for the old capacity-64 eviction behavior).
    #[test]
    fn client_hub_unregisters_client_only_at_lossless_hard_cap() {
        let hub = ClientHub::default();
        let _queue = hub.register("stuck-client".to_string());

        let events: Vec<OutboundEvent> = (0..=LOSSLESS_HARD_CAP)
            .map(|index| OutboundEvent::broadcast(lossless_error(&format!("fill-{index}"))))
            .collect();
        hub.dispatch(events);

        let clients = hub
            .clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            !clients.contains_key("stuck-client"),
            "hard-capped client is unregistered as the last resort"
        );
    }

    #[test]
    fn client_hub_dispatch_releases_lock_before_serializing_and_sending() {
        let hub = ClientHub::default();
        let _receivers: Vec<_> = (0..200)
            .map(|i| hub.register(format!("client-{i}")))
            .collect();

        let events: Vec<OutboundEvent> = (0..1000)
            .map(|i| {
                OutboundEvent::broadcast(BackendEvent::ProjectOpenError {
                    message: format!("event-{i}"),
                })
            })
            .collect();

        let dispatch_hub = hub.clone();
        let started_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started_flag_for_thread = started_flag.clone();
        let dispatch_handle = std::thread::spawn(move || {
            started_flag_for_thread.store(true, std::sync::atomic::Ordering::Release);
            dispatch_hub.dispatch(events);
        });

        while !started_flag.load(std::sync::atomic::Ordering::Acquire) {
            std::thread::yield_now();
        }
        std::thread::sleep(std::time::Duration::from_micros(200));

        let register_start = std::time::Instant::now();
        let _intruder_rx = hub.register("intruder".to_string());
        let register_elapsed = register_start.elapsed();

        dispatch_handle.join().expect("dispatch thread joins");

        assert!(
            register_elapsed < std::time::Duration::from_millis(20),
            "register must not wait for dispatch's serialize+send loop; waited {register_elapsed:?}"
        );
    }

    #[test]
    fn websocket_origin_authorized_requires_same_host_when_origin_is_present() {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "127.0.0.1:3000".parse().expect("host header"));
        assert!(websocket_origin_authorized(&headers));

        headers.insert(ORIGIN, "http://127.0.0.1:3000".parse().expect("origin"));
        assert!(websocket_origin_authorized(&headers));

        headers.insert(ORIGIN, "https://127.0.0.1:3000".parse().expect("origin"));
        assert!(websocket_origin_authorized(&headers));

        headers.insert(ORIGIN, "http://evil.example:3000".parse().expect("origin"));
        assert!(!websocket_origin_authorized(&headers));
    }

    #[test]
    fn embedded_server_exposes_health_and_authenticated_hook_live_routes() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let hook = server.hook_forward_target();
        let client = reqwest::blocking::Client::new();

        assert_ne!(hook.url, format!("{}internal/hook-live", server.url()));

        let health = client
            .get(format!("{}healthz", server.url()))
            .send()
            .expect("health request");
        assert_eq!(health.status(), HttpStatusCode::OK);
        assert_eq!(health.text().expect("health body"), "ok");

        let app_js = client
            .get(format!("{}app.js", server.url()))
            .send()
            .expect("app.js request");
        assert_eq!(app_js.status(), HttpStatusCode::OK);
        let content_type = app_js
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .expect("app.js content type");
        assert_eq!(content_type, "text/javascript; charset=utf-8");
        assert!(
            app_js
                .text()
                .expect("app.js body")
                .contains("function websocketUrl()"),
            "expected embedded server to serve the shared frontend bundle script",
        );

        let xterm_js = client
            .get(format!("{}assets/xterm/xterm.mjs", server.url()))
            .send()
            .expect("xterm module request");
        assert_eq!(xterm_js.status(), HttpStatusCode::OK);
        assert_eq!(
            xterm_js
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/javascript; charset=utf-8")
        );
        assert!(
            xterm_js
                .text()
                .expect("xterm module body")
                .contains("Terminal"),
            "expected embedded server to serve pinned xterm module asset",
        );

        let xterm_fit_js = client
            .get(format!("{}assets/xterm/addon-fit.mjs", server.url()))
            .send()
            .expect("xterm fit module request");
        assert_eq!(xterm_fit_js.status(), HttpStatusCode::OK);
        assert_eq!(
            xterm_fit_js
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/javascript; charset=utf-8")
        );
        assert!(
            xterm_fit_js
                .text()
                .expect("xterm fit module body")
                .contains("FitAddon"),
            "expected embedded server to serve pinned xterm fit addon asset",
        );

        let xterm_css = client
            .get(format!("{}assets/xterm/xterm.css", server.url()))
            .send()
            .expect("xterm css request");
        assert_eq!(xterm_css.status(), HttpStatusCode::OK);
        assert_eq!(
            xterm_css
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/css; charset=utf-8")
        );
        assert!(
            xterm_css.text().expect("xterm css body").contains(".xterm"),
            "expected embedded server to serve pinned xterm stylesheet asset",
        );

        let theme_toggle_js = client
            .get(format!("{}theme-toggle.js", server.url()))
            .send()
            .expect("theme toggle module request");
        assert_eq!(theme_toggle_js.status(), HttpStatusCode::OK);
        assert_eq!(
            theme_toggle_js
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/javascript; charset=utf-8")
        );
        assert!(
            theme_toggle_js
                .text()
                .expect("theme toggle module body")
                .contains("wireThemeToggle"),
            "expected embedded server to serve the segmented theme toggle module",
        );

        let event = sample_runtime_hook_event();

        let unauthorized = client
            .post(&hook.url)
            .json(&event)
            .send()
            .expect("unauthorized hook request");
        assert_eq!(unauthorized.status(), HttpStatusCode::UNAUTHORIZED);

        let wrong_token = client
            .post(&hook.url)
            .bearer_auth("wrong-token")
            .json(&event)
            .send()
            .expect("wrong token hook request");
        assert_eq!(wrong_token.status(), HttpStatusCode::UNAUTHORIZED);

        let accepted = client
            .post(&hook.url)
            .bearer_auth(&hook.token)
            .json(&event)
            .send()
            .expect("authorized hook request");
        assert_eq!(accepted.status(), HttpStatusCode::NO_CONTENT);

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(recorded.iter().any(|user_event| {
            matches!(
                user_event,
                UserEvent::RuntimeHook(recorded_event)
                    if recorded_event.kind == RuntimeHookEventKind::RuntimeState
                        && recorded_event.source_event.as_deref() == Some("PreToolUse")
                        && recorded_event.agent_session_id.as_deref() == Some("agent-1")
            )
        }));

        server.shutdown();
    }

    #[test]
    fn successful_hook_live_requests_do_not_fill_access_log_ring() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("server");

        let hook = server.hook_forward_target();
        let client = reqwest::blocking::Client::new();
        let accepted = client
            .post(&hook.url)
            .bearer_auth(&hook.token)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("authorized hook request");
        assert_eq!(accepted.status(), HttpStatusCode::NO_CONTENT);

        let records = server.access_log().snapshot();
        assert!(
            records
                .iter()
                .all(|record| record.path != "/internal/hook-live"),
            "successful internal hook-live traffic must not evict operator-relevant access records"
        );

        server.shutdown();
    }

    #[test]
    fn unsuccessful_hook_live_requests_remain_in_access_log_ring() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("server");

        let hook = server.hook_forward_target();
        let client = reqwest::blocking::Client::new();
        let unauthorized = client
            .post(&hook.url)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("unauthorized hook request");
        assert_eq!(unauthorized.status(), HttpStatusCode::UNAUTHORIZED);

        let records = server.access_log().snapshot();
        let hook_record = records
            .iter()
            .find(|record| record.path == "/internal/hook-live")
            .expect("failed hook-live access should remain visible");
        assert_eq!(hook_record.method, "POST");
        assert_eq!(hook_record.status, 401);

        server.shutdown();
    }

    #[test]
    fn failed_agent_routes_never_record_client_metadata_that_can_repeat_capability_secrets() {
        const TOKEN_SENTINEL: &str = "agent-capability-secret-sentinel";

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("server");
        let hook = server.hook_forward_target();
        let mut workspace_update_url = reqwest::Url::parse(&hook.url).expect("agent hook URL");
        workspace_update_url.set_path("/internal/workspace-update");
        let mut work_terminalization_url = reqwest::Url::parse(&hook.url).expect("agent hook URL");
        work_terminalization_url.set_path("/internal/work-terminalization");
        let mut execution_binding_probe_url =
            reqwest::Url::parse(&hook.url).expect("agent hook URL");
        execution_binding_probe_url.set_path("/internal/execution-binding-probe");
        let workspace_request = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "session-1",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            },
            "intent": {}
        });
        let terminalization_request = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "session-1",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            },
            "terminal_kind": "done"
        });
        let binding_probe_request = serde_json::json!({
            "schema_version": gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            "operation_id": "operation-access-log",
            "nonce": "nonce-access-log"
        });
        let client = reqwest::blocking::Client::new();

        let hook_response = client
            .post(&hook.url)
            .header(reqwest::header::USER_AGENT, TOKEN_SENTINEL)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("unauthorized hook request");
        assert_eq!(hook_response.status(), HttpStatusCode::UNAUTHORIZED);

        let workspace_response = client
            .post(workspace_update_url)
            .header(reqwest::header::USER_AGENT, TOKEN_SENTINEL)
            .json(&workspace_request)
            .send()
            .expect("unauthorized workspace-update request");
        assert_eq!(workspace_response.status(), HttpStatusCode::UNAUTHORIZED);

        let terminalization_response = client
            .post(work_terminalization_url)
            .header(reqwest::header::USER_AGENT, TOKEN_SENTINEL)
            .json(&terminalization_request)
            .send()
            .expect("unauthorized Work terminalization request");
        assert_eq!(
            terminalization_response.status(),
            HttpStatusCode::UNAUTHORIZED
        );

        let binding_probe_response = client
            .post(execution_binding_probe_url)
            .header(reqwest::header::USER_AGENT, TOKEN_SENTINEL)
            .json(&binding_probe_request)
            .send()
            .expect("unauthorized execution binding probe request");
        assert_eq!(
            binding_probe_response.status(),
            HttpStatusCode::UNAUTHORIZED
        );

        let records = server.access_log().snapshot();
        for path in [
            "/internal/hook-live",
            "/internal/execution-binding-probe",
            "/internal/workspace-update",
            "/internal/work-terminalization",
        ] {
            let record = records
                .iter()
                .find(|record| record.path == path)
                .unwrap_or_else(|| panic!("failed {path} access should remain visible"));
            assert_eq!(record.status, 401);
            assert_eq!(
                record.user_agent, None,
                "agent access records must not retain caller-controlled metadata"
            );
        }
        assert!(
            !format!("{records:?}").contains(TOKEN_SENTINEL),
            "agent access records must stay capability-secret-free"
        );

        server.shutdown();
    }

    #[test]
    fn embedded_server_streams_attachment_uploads_into_upload_store() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let upload_store = AttachmentUploadStore::in_system_temp();
        let mut server =
            EmbeddedServer::start(&runtime, proxy, clients, pty_writers, upload_store.clone())
                .expect("embedded server");
        let client = reqwest::blocking::Client::new();
        let token_response: serde_json::Value = client
            .get(format!("{}internal/attachment-upload-token", server.url()))
            .send()
            .expect("token request")
            .json()
            .expect("token json");
        let token = token_response
            .get("token")
            .and_then(|value| value.as_str())
            .expect("token field")
            .to_string();

        let upload_response: serde_json::Value = client
            .post(format!(
                "{}internal/attachments/upload?filename=Large%20File.bin&mime_type=application%2Foctet-stream&size=12",
                server.url()
            ))
            .header("x-gwt-upload-token", token)
            .body("upload-bytes")
            .send()
            .expect("upload request")
            .json()
            .expect("upload json");
        let upload_id = upload_response
            .get("upload_id")
            .and_then(|value| value.as_str())
            .expect("upload id");

        let uploaded = upload_store
            .take(upload_id)
            .expect("take upload")
            .expect("uploaded file registered");
        assert_eq!(uploaded.filename, "Large File.bin");
        assert_eq!(
            uploaded.mime_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(uploaded.size, 12);
        assert_eq!(
            std::fs::read(uploaded.path).expect("read uploaded temp"),
            b"upload-bytes"
        );

        server.shutdown();
    }

    #[test]
    fn embedded_server_preserves_unicode_attachment_upload_filename() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let upload_store = AttachmentUploadStore::in_system_temp();
        let mut server =
            EmbeddedServer::start(&runtime, proxy, clients, pty_writers, upload_store.clone())
                .expect("embedded server");
        let client = reqwest::blocking::Client::new();
        let token_response: serde_json::Value = client
            .get(format!("{}internal/attachment-upload-token", server.url()))
            .send()
            .expect("token request")
            .json()
            .expect("token json");
        let token = token_response
            .get("token")
            .and_then(|value| value.as_str())
            .expect("token field")
            .to_string();

        let upload_response: serde_json::Value = client
            .post(format!(
                "{}internal/attachments/upload?filename=%E8%B3%87%E6%96%99%20%E6%97%A5%E6%9C%AC%E8%AA%9E.txt&mime_type=text%2Fplain&size=7",
                server.url()
            ))
            .header("x-gwt-upload-token", token)
            .body("nihongo")
            .send()
            .expect("unicode filename upload request")
            .json()
            .expect("unicode filename upload json");
        assert_eq!(
            upload_response
                .get("filename")
                .and_then(|value| value.as_str()),
            Some("資料 日本語.txt")
        );
        let upload_id = upload_response
            .get("upload_id")
            .and_then(|value| value.as_str())
            .expect("upload id");

        let uploaded = upload_store
            .take(upload_id)
            .expect("take upload")
            .expect("uploaded file registered");
        assert_eq!(uploaded.filename, "資料 日本語.txt");
        assert_eq!(uploaded.size, 7);
        assert_eq!(
            std::fs::read(uploaded.path).expect("read uploaded temp"),
            b"nihongo"
        );

        server.shutdown();
    }

    // ---------------------------------------------------------------
    // SPEC-1942 US-14: bind / port surface + access log middleware
    // ---------------------------------------------------------------

    #[test]
    fn embedded_server_start_with_bind_accepts_loopback_and_emits_loopback_url() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start_with_bind(
            &runtime,
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            0,
            0, // no dedicated OAuth listener in tests
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("loopback bind succeeds");

        assert!(
            server.url().starts_with("http://127.0.0.1:"),
            "loopback bind must surface 127.0.0.1 url, got {}",
            server.url(),
        );
        assert_ne!(server.bound_port().get(), 0);
        assert!(server.url().contains(&format!(":{}/", server.bound_port())));
        server.shutdown();
    }

    #[test]
    fn embedded_server_start_with_bind_accepts_unspecified_v4_and_surfaces_actual_ip() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start_with_bind(
            &runtime,
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            0,
            0, // no dedicated OAuth listener in tests
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("0.0.0.0 bind succeeds");

        assert!(
            server.url().starts_with("http://0.0.0.0:"),
            "0.0.0.0 bind must surface 0.0.0.0 url, got {}",
            server.url(),
        );
        assert!(
            server
                .agent_capability_issuer()
                .pane_websocket_url()
                .starts_with("ws://127.0.0.1:"),
            "pane clients must receive a connectable loopback URL for a wildcard browser bind"
        );
        server.shutdown();
    }

    /// SPEC #2920 Phase 4 partial — end-to-end coverage that mirrors how
    /// `main.rs` wires the GUI route after the `--bind`/`--port` restore:
    /// argv tokens → `parse_tray_argv` → `TrayArgs` → `start_with_bind` →
    /// served URL. The full main bootstrap blocks on the per-worktree
    /// project-index runtime, so we cannot exercise it inline, but this
    /// composes the pieces that actually deliver VPN-reachable bind.
    #[test]
    fn parsed_tray_argv_drives_embedded_server_bind_end_to_end() {
        let argv: Vec<String> = [
            "gwt",
            "--bind",
            "0.0.0.0",
            "--port",
            "0",
            "--no-tray",
            "--no-open",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let tray_args =
            gwt::cli::tray::parse_tray_argv(&argv).expect("argv with --bind / --port parses");
        assert_eq!(
            tray_args.bind,
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        );
        assert_eq!(tray_args.port, Some(0));

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start_with_bind(
            &runtime,
            tray_args.bind,
            tray_args.port.unwrap_or(0),
            0, // no dedicated OAuth listener in tests
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("start_with_bind succeeds for parsed TrayArgs");

        let url = server.url().to_string();
        assert!(
            url.starts_with("http://0.0.0.0:"),
            "parsed `--bind 0.0.0.0` must surface a 0.0.0.0 URL, got {url}",
        );
        server.shutdown();
    }

    #[test]
    fn access_log_layer_records_http_request_with_method_path_status_and_peer() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("server");

        let url = server.url().to_string();
        let client = reqwest::blocking::Client::new();
        let response = client
            .get(format!("{url}app.js"))
            .header(reqwest::header::USER_AGENT, "build-spec-test/1.0")
            .send()
            .expect("app.js request");
        assert_eq!(response.status(), HttpStatusCode::OK);

        let records = server.access_log().snapshot();
        let app_js = records
            .iter()
            .find(|r| r.path == "/app.js")
            .expect("/app.js entry must be recorded by access log middleware");
        assert_eq!(app_js.method, "GET");
        assert_eq!(app_js.status, 200);
        assert_eq!(
            app_js.user_agent.as_deref(),
            Some("build-spec-test/1.0"),
            "user agent must be carried into the record"
        );
        let peer = app_js.peer.as_deref().expect("peer addr captured");
        assert!(
            peer.starts_with("127.0.0.1:"),
            "peer must be the loopback client, got {peer}"
        );

        server.shutdown();
    }

    #[test]
    fn access_log_layer_still_records_healthz_and_distinguishes_path() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("server");

        let url = server.url().to_string();
        let client = reqwest::blocking::Client::new();
        let response = client
            .get(format!("{url}healthz"))
            .send()
            .expect("healthz request");
        assert_eq!(response.status(), HttpStatusCode::OK);

        // The sink still captures /healthz so an in-process operator panel
        // can render it, but the tracing layer demotes it to debug — this
        // distinction is asserted at the path level: /healthz is recorded
        // but lives separately from real LAN access records.
        let records = server.access_log().snapshot();
        let healthz = records
            .iter()
            .find(|r| r.path == "/healthz")
            .expect("healthz still appears in the in-memory sink");
        assert_eq!(healthz.method, "GET");
        assert_eq!(healthz.status, 200);

        server.shutdown();
    }
}
