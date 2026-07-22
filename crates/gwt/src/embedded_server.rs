use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::{atomic::AtomicU64, Arc, Mutex, RwLock},
    time::Instant,
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
    AgentWorkspaceUpdateRequest, FrontendEvent, HookForwardTarget, RuntimeHookEvent,
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
///
/// `coalesce_key` and `repair_pane_id` are deliberately separate identities.
/// `coalesce_key` collapses successive snapshots of the same logical target to
/// the latest value (terminal pane for `terminal_snapshot`, `operation_id` for
/// `attachment_progress`). `repair_pane_id` names the terminal pane whose
/// dropped streamed output must self-heal via a snapshot re-send. A single
/// event participates in at most one role, so attachment progress can coalesce
/// by operation without being mistaken for a terminal pane needing repair
/// (Issue #3315).
struct PreparedOutbound {
    payload: String,
    kind: &'static str,
    coalesce_key: Option<String>,
    repair_pane_id: Option<String>,
    class: QueueClass,
}

fn prepare_outbound(event: &gwt::BackendEvent) -> PreparedOutbound {
    let kind = event.event_kind();
    let (coalesce_key, repair_pane_id) = match event {
        gwt::BackendEvent::TerminalOutput { id, .. } => (None, Some(id.clone())),
        gwt::BackendEvent::TerminalSnapshot { id, .. } => (Some(id.clone()), None),
        gwt::BackendEvent::AttachmentProgress { operation_id, .. } => {
            (Some(operation_id.clone()), None)
        }
        _ => (None, None),
    };
    PreparedOutbound {
        payload: serde_json::to_string(event).expect("backend event json"),
        kind,
        coalesce_key,
        repair_pane_id,
        class: queue_class_for_kind(kind),
    }
}

struct QueuedOutbound {
    payload: String,
    kind: &'static str,
    coalesce_key: Option<String>,
}

#[derive(Default)]
struct ClientQueueState {
    entries: std::collections::VecDeque<QueuedOutbound>,
    dirty_panes: std::collections::HashSet<String>,
    dropped_lossy: u64,
    dead: bool,
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
    Closed,
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
        // Snapshot-class kinds without a coalesce key (file trees, resume acks,
        // release notes) must not replace each other by kind alone — different
        // windows would clobber one another. They get lossless append semantics
        // instead.
        let effective_class = match message.class {
            QueueClass::SnapshotLatest if message.coalesce_key.is_none() => QueueClass::Lossless,
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
                if let Some(entry) = state.entries.iter_mut().find(|entry| {
                    entry.kind == message.kind && entry.coalesce_key == message.coalesce_key
                }) {
                    entry.payload = message.payload.clone();
                } else {
                    if state.entries.len() >= LOSSLESS_HARD_CAP {
                        state.dead = true;
                        drop(state);
                        self.notify.notify_one();
                        return true;
                    }
                    state.entries.push_back(Self::queued(message));
                }
            }
            QueueClass::Lossy => {
                if state.entries.len() >= LOSSY_HIGH_WATER {
                    state.dropped_lossy += 1;
                    if let Some(pane) = &message.repair_pane_id {
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
            coalesce_key: message.coalesce_key.clone(),
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
            return Some(DrainStep::Closed);
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
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.dead = true;
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
            DrainStep::Closed => None,
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
    clients: Arc<Mutex<HashMap<String, Arc<ClientQueue>>>>,
}

impl ClientHub {
    pub(super) fn register(&self, client_id: String) -> Arc<ClientQueue> {
        let queue = Arc::new(ClientQueue::default());
        self.clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(client_id, queue.clone());
        queue
    }

    pub(super) fn unregister(&self, client_id: &str) {
        let removed = self
            .clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(client_id);
        if let Some(queue) = removed {
            queue.close();
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
            clients.values().cloned().collect()
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
        let snapshot: Vec<(String, Arc<ClientQueue>)> = {
            let clients = self
                .clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            clients
                .iter()
                .map(|(id, queue)| (id.clone(), queue.clone()))
                .collect()
        };

        let mut dead_clients: Vec<String> = Vec::new();
        for outbound in events {
            let prepared = prepare_outbound(&outbound.event);
            match outbound.target {
                DispatchTarget::Broadcast => {
                    for (client_id, queue) in &snapshot {
                        if queue.enqueue(&prepared) {
                            dead_clients.push(client_id.clone());
                        }
                    }
                }
                DispatchTarget::Client(client_id) => {
                    if let Some((_, queue)) = snapshot.iter().find(|(id, _)| id == &client_id) {
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
                if let Some(queue) = clients.remove(&client_id) {
                    queue.close();
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
}

impl AgentSessionPrincipal {
    fn new(project_root: &Path, session_id: &str) -> Result<Self, String> {
        if session_id.trim() != session_id
            || gwt_agent::validate_session_id_path_component(session_id).is_err()
        {
            return Err("agent capability session id must be non-empty and canonical".to_string());
        }

        let canonical_project_root = dunce::canonicalize(project_root)
            .map(|path| gwt_core::paths::normalize_windows_child_process_path(&path))
            .map_err(|_| "agent capability project scope must be an existing canonical root")?;

        Ok(Self {
            canonical_project_root,
            session_id: session_id.to_string(),
        })
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn canonical_project_root(&self) -> &Path {
        &self.canonical_project_root
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
            .finish()
    }
}

#[derive(Default)]
struct AgentCapabilityRegistryState {
    principals_by_token: HashMap<String, AgentSessionPrincipal>,
    token_by_project_session: HashMap<(PathBuf, String), String>,
}

/// Process-local map from opaque bearer capabilities to immutable Session
/// principals. A capability never persists to disk and its bearer is the only
/// identity material that crosses into an agent process or container.
#[derive(Clone, Default)]
struct AgentCapabilityRegistry {
    inner: Arc<RwLock<AgentCapabilityRegistryState>>,
}

impl AgentCapabilityRegistry {
    fn issue(&self, project_root: &Path, session_id: &str) -> Result<String, String> {
        let principal = AgentSessionPrincipal::new(project_root, session_id)?;
        let principal_key = (
            principal.canonical_project_root().to_path_buf(),
            principal.session_id().to_string(),
        );

        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let token = loop {
            let candidate = format!("gwt_agent_{}{}", Uuid::new_v4(), Uuid::new_v4());
            if !state.principals_by_token.contains_key(&candidate) {
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

    fn revoke(&self, project_root: &Path, session_id: &str) -> Result<bool, String> {
        let principal = AgentSessionPrincipal::new(project_root, session_id)?;
        let principal_key = (
            principal.canonical_project_root().to_path_buf(),
            principal.session_id().to_string(),
        );
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(token) = state.token_by_project_session.remove(&principal_key) else {
            return Ok(false);
        };
        state.principals_by_token.remove(&token);
        Ok(true)
    }

    fn session_count(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .token_by_project_session
            .len()
    }
}

/// In-process authority used by launch orchestration to mint one capability
/// for a canonical project + Session pair.
#[derive(Clone)]
pub(crate) struct AgentCapabilityIssuer {
    hook_forward_url: String,
    pane_websocket_url: String,
    registry: AgentCapabilityRegistry,
}

impl AgentCapabilityIssuer {
    fn new(
        hook_forward_url: String,
        pane_websocket_url: String,
        registry: AgentCapabilityRegistry,
    ) -> Self {
        Self {
            hook_forward_url,
            pane_websocket_url,
            registry,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(hook_forward_url: &str, pane_websocket_url: &str) -> Self {
        Self::new(
            hook_forward_url.to_string(),
            pane_websocket_url.to_string(),
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

    pub(crate) fn revoke(&self, project_root: &Path, session_id: &str) -> Result<bool, String> {
        self.registry.revoke(project_root, session_id)
    }

    pub(crate) fn pane_websocket_url(&self) -> &str {
        &self.pane_websocket_url
    }
}

impl std::fmt::Debug for AgentCapabilityIssuer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentCapabilityIssuer")
            .field("hook_forward_url", &self.hook_forward_url)
            .field("pane_websocket_url", &self.pane_websocket_url)
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
            agent_capabilities.clone(),
        );
        let attachment_upload_token = Uuid::new_v4().to_string();
        let access_log = AccessLogSink::default();
        let server_state = ServerState {
            proxy,
            clients,
            agent_capabilities,
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

    let project_root = principal.canonical_project_root().to_path_buf();
    let session_id = principal.session_id().to_string();
    let mutation_project_root = project_root.clone();
    let result = tokio::task::spawn_blocking(move || {
        gwt::apply_authenticated_workspace_update(&mutation_project_root, &session_id, request)
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

    let project_root = principal.canonical_project_root().to_path_buf();
    let session_id = principal.session_id().to_string();
    let mutation_project_root = project_root.clone();
    let result = tokio::task::spawn_blocking(move || {
        gwt::apply_authenticated_work_terminalization(&mutation_project_root, &session_id, request)
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
        | AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired
        | AgentWorkspaceUpdateErrorCode::ProvenanceMismatch
        | AgentWorkspaceUpdateErrorCode::IdentityConflict
        | AgentWorkspaceUpdateErrorCode::TransactionConflict => StatusCode::CONFLICT,
        AgentWorkspaceUpdateErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn workspace_update_error_response(
    status: StatusCode,
    error: AgentWorkspaceUpdateError,
) -> Response {
    (status, Json(error)).into_response()
}

async fn client_session(socket: WebSocket, state: ServerState) {
    let client_id = Uuid::new_v4().to_string();
    let outbound = state.clients.register(client_id.clone());
    let (mut sender, mut receiver) = socket.split();

    let input_seq = Arc::new(AtomicU64::new(0));

    loop {
        tokio::select! {
            step = outbound.next() => {
                match step {
                    DrainStep::Message { payload, repair_panes } => {
                        if sender.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
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
                    DrainStep::Closed => break,
                }
            }
            maybe_message = receiver.next() => {
                match maybe_message {
                    Some(Ok(Message::Text(text))) => {
                        let text_len = text.len();
                        match serde_json::from_str::<FrontendEvent>(text.as_ref()) {
                            Ok(event) => {
                                handle_frontend_message(
                                    &state,
                                    &client_id,
                                    &input_seq,
                                    text_len,
                                    event,
                                );
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
    text_len: usize,
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
    let data_len = data.len();
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "ws_recv",
        client_id = %client_id,
        seq,
        window_id = %id,
        data_len,
        text_len,
        "terminal_input received over WebSocket"
    );

    let pty_handle = match state.pty_writers.read() {
        Ok(guard) => guard.get(&id).cloned(),
        Err(error) => {
            tracing::warn!(
                target: "gwt_input_trace",
                stage = "fast_path_lock_poisoned",
                client_id = %client_id,
                seq,
                window_id = %id,
                error = %error,
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
                    data_len,
                    write_us = write_started.elapsed().as_micros() as u64,
                    "terminal_input written to PTY via WS fast-path"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    target: "gwt_input_trace",
                    stage = "fast_path_write_err",
                    client_id = %client_id,
                    seq,
                    window_id = %id,
                    data_len,
                    error = %error,
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
            data_len,
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
        data_len,
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
        sync::{atomic::AtomicU64, Arc, Mutex, RwLock},
    };

    use axum::http::{
        header::{AUTHORIZATION, HOST, ORIGIN},
        HeaderMap,
    };
    use gwt::{
        AttachmentProgressPhase, BackendEvent, FrontendEvent, RuntimeHookEvent,
        RuntimeHookEventKind,
    };
    use reqwest::StatusCode as HttpStatusCode;
    use tokio::runtime::Runtime;

    use crate::{AppEventProxy, AttachmentUploadStore, OutboundEvent, UserEvent};

    use super::{
        agent_bridge_bind_ip, bearer_token, handle_frontend_message, prepare_outbound,
        queue_class_for_kind, websocket_origin_authorized, AgentCapabilityIssuer,
        AgentCapabilityRegistry, AgentSessionPrincipal, ClientHub, ClientQueue, DrainStep,
        EmbeddedServer, QueueClass, ServerState, DRAIN_LOW_WATER, LOSSLESS_HARD_CAP,
        LOSSY_HIGH_WATER,
    };

    fn sample_server_state() -> (ServerState, Arc<Mutex<Vec<UserEvent>>>) {
        let (proxy, events) = AppEventProxy::stub();
        (
            ServerState {
                proxy,
                clients: ClientHub::default(),
                agent_capabilities: AgentCapabilityRegistry::default(),
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
    fn agent_capability_registry_revoke_retires_only_requested_pair() {
        let project_a = tempfile::tempdir().expect("project A tempdir");
        let project_b = tempfile::tempdir().expect("project B tempdir");
        let registry = AgentCapabilityRegistry::default();
        let token_a = registry
            .issue(project_a.path(), "session-1")
            .expect("project A capability");
        let token_b = registry
            .issue(project_b.path(), "session-1")
            .expect("project B capability");

        assert!(registry
            .revoke(project_a.path(), "session-1")
            .expect("revoke project A"));
        assert!(registry.authenticate(&token_a).is_none());
        assert!(registry.authenticate(&token_b).is_some());
        assert!(!registry
            .revoke(project_a.path(), "session-1")
            .expect("repeat revoke"));
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
    fn agent_bridge_bind_policy_widens_only_for_native_linux_container_access() {
        let expected = if cfg!(target_os = "linux") {
            IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        };

        assert_eq!(agent_bridge_bind_ip(), expected);
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
        assert_eq!(error["code"], "provenance_mismatch");
        assert!(error["message"]
            .as_str()
            .is_some_and(|message| message.contains("Session claim")));
        assert!(!error.to_string().contains(&current.token));
        assert!(events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());

        server.shutdown();
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
        assert_eq!(error["code"], "provenance_mismatch");
        assert!(error["message"]
            .as_str()
            .is_some_and(|message| message.contains("Session claim")));
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
            32,
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
            48,
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

    fn attachment_progress(
        pane: &str,
        operation_id: &str,
        phase: AttachmentProgressPhase,
    ) -> BackendEvent {
        BackendEvent::AttachmentProgress {
            id: pane.to_string(),
            operation_id: operation_id.to_string(),
            phase,
            file_index: Some(0),
            file_count: 1,
            filename: Some("notes.txt".to_string()),
            bytes_done: Some(16),
            bytes_total: Some(16),
            message: None,
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
                DrainStep::Closed => break,
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
            matches!(queue.try_next(), Some(DrainStep::Closed)),
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
        // Issue #3315: attachment progress is a lossless snapshot, not lossy.
        assert_eq!(
            queue_class_for_kind("attachment_progress"),
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

    // Issue #3315: under a terminal-output flood that saturates the lossy
    // queue, a full attachment operation must still coalesce to a single
    // latest-state entry and preserve the terminal `Attached` phase. The old
    // EphemeralStatus/Lossy class dropped these past the high-water mark, which
    // left the frontend surface stuck at `Queued · 100%`.
    #[test]
    fn client_queue_coalesces_attachment_progress_and_preserves_terminal_state_under_lossy_flood() {
        let queue = ClientQueue::default();

        for index in 0..(LOSSY_HIGH_WATER + 20) {
            queue.enqueue(&prepare_outbound(&terminal_output(
                "tab-1::agent-1",
                &format!("chunk-{index}"),
            )));
        }
        assert_eq!(
            queue.len(),
            LOSSY_HIGH_WATER,
            "lossy flood saturates the queue at the high-water mark"
        );

        for phase in [
            AttachmentProgressPhase::Queued,
            AttachmentProgressPhase::Staging,
            AttachmentProgressPhase::Injecting,
            AttachmentProgressPhase::Attached,
        ] {
            queue.enqueue(&prepare_outbound(&attachment_progress(
                "tab-1::agent-1",
                "op-1",
                phase,
            )));
        }

        assert!(
            !queue.is_dead(),
            "attachment progress must never disconnect the client"
        );

        let (payloads, _) = drain_all(&queue);
        let attachment: Vec<&String> = payloads
            .iter()
            .filter(|payload| payload.contains("\"kind\":\"attachment_progress\""))
            .collect();
        assert_eq!(
            attachment.len(),
            1,
            "one operation coalesces to a single queued entry regardless of flood"
        );
        assert!(
            attachment[0].contains("\"phase\":\"attached\""),
            "the terminal Attached state survives the lossy flood"
        );
        assert!(attachment[0].contains("\"operation_id\":\"op-1\""));
    }

    // Issue #3315: coalescing is keyed by operation_id — different attachment
    // operations in the same pane must never clobber one another, and a
    // terminal `Failed` is as durable as `Attached`.
    #[test]
    fn client_queue_keeps_distinct_attachment_operations_independent() {
        let queue = ClientQueue::default();

        queue.enqueue(&prepare_outbound(&attachment_progress(
            "tab-1::agent-1",
            "op-a",
            AttachmentProgressPhase::Queued,
        )));
        queue.enqueue(&prepare_outbound(&attachment_progress(
            "tab-1::agent-1",
            "op-b",
            AttachmentProgressPhase::Staging,
        )));
        queue.enqueue(&prepare_outbound(&attachment_progress(
            "tab-1::agent-1",
            "op-a",
            AttachmentProgressPhase::Attached,
        )));
        queue.enqueue(&prepare_outbound(&attachment_progress(
            "tab-1::agent-1",
            "op-b",
            AttachmentProgressPhase::Failed,
        )));

        let (payloads, _) = drain_all(&queue);
        let attachment: Vec<&String> = payloads
            .iter()
            .filter(|payload| payload.contains("\"kind\":\"attachment_progress\""))
            .collect();
        assert_eq!(
            attachment.len(),
            2,
            "two distinct operations keep two independent entries"
        );
        assert!(
            attachment
                .iter()
                .any(|payload| payload.contains("\"operation_id\":\"op-a\"")
                    && payload.contains("\"phase\":\"attached\"")),
            "op-a keeps only its latest (Attached) state"
        );
        assert!(
            attachment
                .iter()
                .any(|payload| payload.contains("\"operation_id\":\"op-b\"")
                    && payload.contains("\"phase\":\"failed\"")),
            "op-b keeps its terminal Failed state independently"
        );
    }

    // Issue #3315: a runaway progress stream for one operation must not grow
    // the queue toward the lossless hard cap or disconnect the client — the
    // latest state replaces the queued one in place.
    #[test]
    fn client_queue_does_not_disconnect_under_attachment_progress_flood() {
        let queue = ClientQueue::default();

        for index in 0..(LOSSLESS_HARD_CAP + 100) {
            let phase = if index % 2 == 0 {
                AttachmentProgressPhase::Staging
            } else {
                AttachmentProgressPhase::Injecting
            };
            let dead = queue.enqueue(&prepare_outbound(&attachment_progress(
                "tab-1::agent-1",
                "op-flood",
                phase,
            )));
            assert!(!dead, "coalesced snapshot flood never reaches the hard cap");
        }

        assert!(!queue.is_dead());
        assert_eq!(
            queue.len(),
            1,
            "same-operation progress coalesces to a single queued entry"
        );
    }

    // Issue #3315 / SPEC-2359 FR-563: coalescing only bounds repeated
    // snapshots for the same operation. A stuck client can still receive
    // many distinct operations, so adding a new coalesce key must retain the
    // same hard-cap disconnect contract as any other lossless event.
    #[test]
    fn client_queue_disconnects_when_distinct_attachment_operations_exceed_hard_cap() {
        let queue = ClientQueue::default();

        for index in 0..LOSSLESS_HARD_CAP {
            let dead = queue.enqueue(&prepare_outbound(&attachment_progress(
                "tab-1::agent-1",
                &format!("op-{index}"),
                AttachmentProgressPhase::Staging,
            )));
            assert!(!dead, "client stays alive until the hard cap");
        }
        assert_eq!(queue.len(), LOSSLESS_HARD_CAP);
        assert!(!queue.is_dead());

        let dead = queue.enqueue(&prepare_outbound(&attachment_progress(
            "tab-1::agent-1",
            "op-0",
            AttachmentProgressPhase::Attached,
        )));
        assert!(
            !dead,
            "an existing operation can still reach its terminal state at the hard cap"
        );
        assert_eq!(queue.len(), LOSSLESS_HARD_CAP);

        let dead = queue.enqueue(&prepare_outbound(&attachment_progress(
            "tab-1::agent-1",
            "op-overflow",
            AttachmentProgressPhase::Attached,
        )));
        assert!(dead, "a distinct operation beyond the hard cap is rejected");
        assert!(queue.is_dead());
        assert!(
            matches!(queue.try_next(), Some(DrainStep::Closed)),
            "hard-capped snapshot queue reports Closed to the drain loop"
        );
    }

    // SPEC-2359 SC-399 names both terminal phases. Failed must remain
    // lossless under the same lossy terminal-output flood as Attached.
    #[test]
    fn client_queue_preserves_failed_attachment_state_under_lossy_flood() {
        let queue = ClientQueue::default();

        for index in 0..(LOSSY_HIGH_WATER + 20) {
            queue.enqueue(&prepare_outbound(&terminal_output(
                "tab-1::agent-1",
                &format!("chunk-{index}"),
            )));
        }
        queue.enqueue(&prepare_outbound(&attachment_progress(
            "tab-1::agent-1",
            "op-failed",
            AttachmentProgressPhase::Queued,
        )));
        queue.enqueue(&prepare_outbound(&attachment_progress(
            "tab-1::agent-1",
            "op-failed",
            AttachmentProgressPhase::Failed,
        )));

        let (payloads, _) = drain_all(&queue);
        let attachment: Vec<&String> = payloads
            .iter()
            .filter(|payload| payload.contains("\"kind\":\"attachment_progress\""))
            .collect();
        assert_eq!(attachment.len(), 1, "one operation keeps one latest entry");
        assert!(attachment[0].contains("\"phase\":\"failed\""));
        assert!(!queue.is_dead());
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

        let records = server.access_log().snapshot();
        for path in [
            "/internal/hook-live",
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
