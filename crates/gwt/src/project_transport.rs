//! HTTP and WebSocket transport shared by project runtime owners.
//! Browser assets and GUI dispatch remain in the embedding application.
use crate::agent_capability::{
    durable_agent_execution_authority_async, durable_agent_execution_authority_with_lease_async,
    AgentCapabilityGrant, AgentCapabilityRegistry, AgentDurableAuthority, AgentFrontendRequest,
    AgentPmSendResponder, AgentSelfCloseDirectAcceptance, AgentSelfCloseResponder,
    AgentSessionPrincipal, AGENT_PM_SEND_ACCEPTANCE_DEADLINE,
};
use crate::{
    AgentBuildAbortTerminalizationRequest, AgentWorkMaterializationProbeRequest,
    AgentWorkTerminalizationRequest, AgentWorkspaceUpdateError, AgentWorkspaceUpdateErrorCode,
    AgentWorkspaceUpdateRequest, BackendEvent, FrontendEvent, RuntimeHookEvent,
};
use axum::{
    extract::{
        connect_info::ConnectInfo,
        ws::{Message, WebSocket, WebSocketUpgrade},
        Request, State,
    },
    http::{
        header::{AUTHORIZATION, USER_AGENT},
        HeaderMap, StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{atomic::AtomicU64, Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientScope {
    Hub,
    Project(gwt_core::repo_hash::ProjectKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchTarget {
    All,
    // Explicit Hub recipients are part of the routing contract; producers migrate separately.
    #[allow(dead_code)]
    Hub,
    Project(gwt_core::repo_hash::ProjectKey),
    Client(String),
}

#[derive(Debug, Clone)]
pub enum KnowledgeWireMetadata {
    SemanticRetry(crate::KnowledgeSemanticRetry),
    NonSemanticError,
}

#[derive(Debug, Clone)]
pub struct OutboundEvent {
    pub target: DispatchTarget,
    pub event: BackendEvent,
    /// SPEC #1939 FR-407 / SPEC #3170 FR-098: private wire-only metadata for
    /// semantic retry directives and explicitly non-semantic search errors.
    /// Keeping it outside the public `BackendEvent` preserves the baseline
    /// Rust construction/destructuring shape.
    pub knowledge_wire_metadata: Option<KnowledgeWireMetadata>,
    /// Issue #4095: pane stream position for `terminal_output` (the chunk's
    /// own position) and `terminal_snapshot` (the position the snapshot was
    /// serialized at). Never serialized; the client queue uses it to skip
    /// streamed chunks that a queued snapshot already contains.
    pub terminal_stream_seq: Option<u64>,
    /// Error provenance only; never serialized or inferred from dispatch targets.
    pub error_origin: Option<crate::error_report::ErrorOrigin>,
}

impl OutboundEvent {
    pub fn project(project_key: gwt_core::repo_hash::ProjectKey, event: BackendEvent) -> Self {
        Self {
            target: DispatchTarget::Project(project_key),
            event,
            knowledge_wire_metadata: None,
            terminal_stream_seq: None,
            error_origin: None,
        }
    }

    pub fn hub(event: BackendEvent) -> Self {
        let mut outbound = Self::reply("", event);
        outbound.target = DispatchTarget::Hub;
        outbound
    }

    pub fn broadcast(event: BackendEvent) -> Self {
        // Process diagnostics, account state, host settings, and the updater
        // are the complete global inventory. System settings live in the
        // user's global config; autostart describes the user's OS registration.
        // Their request handlers still reply only to the requesting client:
        // permission to broadcast does not turn a request reply into a broadcast.
        // Project and Hub payloads need an owner.
        assert!(
            matches!(
                &event,
                BackendEvent::ProcessLine { .. }
                    | BackendEvent::LogEntryAppended { .. }
                    | BackendEvent::RuntimeHealth { .. }
                    | BackendEvent::ProviderUsage { .. }
                    | BackendEvent::BoardAuthStatus { .. }
                    | BackendEvent::SystemSettings { .. }
                    | BackendEvent::SystemSettingsUpdated { .. }
                    | BackendEvent::SystemSettingsError { .. }
                    | BackendEvent::AutostartStatus { .. }
                    | BackendEvent::AutostartError { .. }
                    | BackendEvent::UpdateState(_)
                    | BackendEvent::UpdateProgress { .. }
                    | BackendEvent::UpdateReady { .. }
                    | BackendEvent::UpdateAutoApply { .. }
                    | BackendEvent::UpdateApplyPendingPersisted { .. }
                    | BackendEvent::UpdateApplyError { .. }
            ),
            "project-owned events require an explicit dispatch scope"
        );
        Self {
            target: DispatchTarget::All,
            event,
            knowledge_wire_metadata: None,
            terminal_stream_seq: None,
            error_origin: None,
        }
    }

    /// Host update notifications share the toast wire shape, but never carry
    /// an Issue owner. Keep this exception separate from project broadcasts:
    /// allowing IssueMonitorToast in broadcast would also admit project toasts.
    /// Callers supply only host update text; the constructor fixes the owner to None.
    pub fn global_update_notice(level: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            target: DispatchTarget::All,
            event: BackendEvent::IssueMonitorToast {
                notification_transition: None,
                level: level.into(),
                message: message.into(),
                issue_number: None,
            },
            knowledge_wire_metadata: None,
            terminal_stream_seq: None,
            error_origin: Some(crate::error_report::ErrorOrigin::Host),
        }
    }

    pub fn with_error_project_root(mut self, root: &Path) -> Self {
        self.error_origin = Some(crate::error_report::ErrorOrigin::Project(
            root.display().to_string(),
        ));
        self
    }

    pub fn reply(client_id: impl Into<String>, event: BackendEvent) -> Self {
        Self {
            target: DispatchTarget::Client(client_id.into()),
            event,
            knowledge_wire_metadata: None,
            terminal_stream_seq: None,
            error_origin: None,
        }
    }

    pub fn reply_with_knowledge_semantic_retry(
        client_id: impl Into<String>,
        event: BackendEvent,
        semantic_retry: Option<crate::KnowledgeSemanticRetry>,
    ) -> Self {
        assert!(
            matches!(event, BackendEvent::KnowledgeSearchResults { .. }),
            "knowledge semantic retry metadata requires KnowledgeSearchResults"
        );
        Self {
            target: DispatchTarget::Client(client_id.into()),
            event,
            knowledge_wire_metadata: semantic_retry.map(KnowledgeWireMetadata::SemanticRetry),
            terminal_stream_seq: None,
            error_origin: None,
        }
    }

    pub fn reply_with_nonsemantic_knowledge_error(
        client_id: impl Into<String>,
        event: BackendEvent,
    ) -> Self {
        assert!(
            matches!(
                event,
                BackendEvent::KnowledgeError {
                    request_id: Some(_),
                    query: Some(_),
                    ..
                }
            ),
            "non-semantic knowledge error metadata requires a correlated KnowledgeError"
        );
        Self {
            target: DispatchTarget::Client(client_id.into()),
            event,
            knowledge_wire_metadata: Some(KnowledgeWireMetadata::NonSemanticError),
            terminal_stream_seq: None,
            error_origin: None,
        }
    }

    /// Attach the pane stream position of a `terminal_output` /
    /// `terminal_snapshot` event (Issue #4095). Private to the process; the
    /// client queue reads it, the wire never carries it.
    pub fn with_terminal_stream_seq(mut self, seq: Option<u64>) -> Self {
        self.terminal_stream_seq = seq;
        self
    }
}

/// AgentFrontend delivery must enqueue only: it runs under the registry fence.
#[derive(Clone)]
pub struct TransportState {
    clients: ClientHub,
    agent_capabilities: AgentCapabilityRegistry,
    host_instance_id: String,
    sink: Arc<dyn Fn(TransportEvent) + Send + Sync>,
}
impl TransportState {
    pub fn new(
        clients: ClientHub,
        agent_capabilities: AgentCapabilityRegistry,
        host_instance_id: String,
        sink: Arc<dyn Fn(TransportEvent) + Send + Sync>,
    ) -> Self {
        Self {
            clients,
            agent_capabilities,
            host_instance_id,
            sink,
        }
    }
    fn send(&self, event: TransportEvent) {
        (self.sink)(event);
    }
}
pub enum TransportEvent {
    FreshExecutionReadyResend {
        grant: AgentCapabilityGrant,
        request: crate::AgentExecutionContinuationRequest,
        reply: std::sync::mpsc::Sender<
            Result<
                Option<crate::AgentExecutionContinuationReceipt>,
                crate::AgentWorkspaceUpdateError,
            >,
        >,
    },
    RuntimeHook(RuntimeHookEvent),
    WorkspaceProjectionChanged {
        project_root: PathBuf,
    },
    AgentFrontend {
        client_id: String,
        grant: AgentCapabilityGrant,
        request: AgentFrontendRequest,
    },
    ClientPaneSnapshotRepair {
        client_id: String,
        pane_ids: Vec<String>,
    },
    BrowserFrontend {
        client_id: String,
        input_seq: Arc<AtomicU64>,
        event: FrontendEvent,
        received_at: Instant,
    },
}

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
pub const AGENT_STALE_BINDING_CLOSE: ClientCloseFrame = ClientCloseFrame {
    code: 1008,
    reason: "execution binding is no longer current",
};
pub const AGENT_AUTHORITY_UNAVAILABLE_CLOSE: ClientCloseFrame = ClientCloseFrame {
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
const AGENT_PM_TERMINAL_SEND_DEADLINE: Duration = Duration::from_secs(1);
const AGENT_PM_TARGET_REFUSAL: &str =
    "pm.message.send refused: target is not an authorized live agent pane";

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
/// `ACCESS_LOG_RING_CAPACITY` entries; older records are evicted FIFO so
/// memory stays bounded under long-running browser-server sessions.
#[derive(Clone, Default)]
pub struct AccessLogSink {
    inner: Arc<Mutex<std::collections::VecDeque<AccessLogRecord>>>,
}

#[derive(Clone)]
pub struct AccessLogPolicy {
    sink: AccessLogSink,
    record_user_agent: bool,
}

impl AccessLogPolicy {
    pub fn browser(sink: AccessLogSink) -> Self {
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
    #[cfg(any(test, feature = "test-gh-guard"))]
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
    use crate::protocol::BackendEventDeliveryClass as Delivery;
    match crate::protocol::backend_event_policy(kind) {
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
pub struct PreparedOutbound {
    payload: Arc<str>,
    kind: &'static str,
    coalesce_key: Option<String>,
    repair_pane_id: Option<String>,
    class: QueueClass,
    /// Terminal pane a `terminal_output` / `terminal_snapshot` belongs to
    /// (Issue #4095), paired with `stream_seq`.
    terminal_pane: Option<String>,
    /// Pane stream position: the chunk's own position for `terminal_output`,
    /// the serialization position for `terminal_snapshot`. `None` for events
    /// produced outside the PTY reader (launch mirror, daemon replay).
    stream_seq: Option<u64>,
}

const KNOWLEDGE_SEMANTIC_RETRY_INITIAL_DELAY_MS: u64 = 5_000;

fn prepare_outbound(event: &crate::BackendEvent) -> PreparedOutbound {
    let kind = event.event_kind();
    let (coalesce_key, repair_pane_id, terminal_pane) = match event {
        crate::BackendEvent::TerminalOutput { id, .. } => {
            (None, Some(id.clone()), Some(id.clone()))
        }
        crate::BackendEvent::TerminalPreview { id, .. } => (Some(id.clone()), None, None),
        crate::BackendEvent::TerminalSnapshot { id, .. } => {
            (Some(id.clone()), None, Some(id.clone()))
        }
        crate::BackendEvent::AttachmentProgress { operation_id, .. } => {
            (Some(operation_id.clone()), None, None)
        }
        _ => (None, None, None),
    };
    PreparedOutbound {
        payload: Arc::from(serde_json::to_string(event).expect("backend event json")),
        kind,
        coalesce_key,
        repair_pane_id,
        class: queue_class_for_kind(kind),
        terminal_pane,
        stream_seq: None,
    }
}

/// Serialize private Knowledge wire metadata without changing the public
/// `BackendEvent` construction/destructuring shape.
pub fn prepare_outbound_event(outbound: &OutboundEvent) -> PreparedOutbound {
    crate::error_report::record_backend_event_with_origin(
        &outbound.event,
        outbound.error_origin.as_ref(),
    );
    let mut prepared = prepare_outbound(&outbound.event);
    prepared.stream_seq = outbound.terminal_stream_seq;
    let Some(metadata) = outbound.knowledge_wire_metadata.as_ref() else {
        return prepared;
    };
    let mut payload = serde_json::to_value(&outbound.event).expect("backend event value");
    let object = payload
        .as_object_mut()
        .expect("internally tagged backend event must serialize as an object");
    match metadata {
        KnowledgeWireMetadata::SemanticRetry(semantic_retry) => {
            if !matches!(
                outbound.event,
                crate::BackendEvent::KnowledgeSearchResults { .. }
            ) || !semantic_retry.retryable
                || !matches!(
                    semantic_retry.error_code.as_str(),
                    "INDEX_NOT_READY" | "SEARCH_UNAVAILABLE"
                )
                || semantic_retry.retry_after_ms != KNOWLEDGE_SEMANTIC_RETRY_INITIAL_DELAY_MS
            {
                return prepared;
            }
            object.insert(
                "semantic_retry".to_string(),
                serde_json::to_value(semantic_retry).expect("knowledge semantic retry value"),
            );
        }
        KnowledgeWireMetadata::NonSemanticError => {
            if !matches!(
                outbound.event,
                crate::BackendEvent::KnowledgeError {
                    request_id: Some(_),
                    query: Some(_),
                    ..
                }
            ) {
                return prepared;
            }
            object.insert(
                "error_domain".to_string(),
                serde_json::Value::String("non_semantic".to_string()),
            );
        }
    }
    prepared.payload = Arc::from(serde_json::to_string(&payload).expect("backend event json"));
    prepared
}

struct QueuedOutbound {
    payload: Arc<str>,
    kind: &'static str,
    coalesce_key: Option<String>,
    terminal_pane: Option<String>,
    stream_seq: Option<u64>,
}

#[derive(Default)]
struct ClientQueueState {
    entries: std::collections::VecDeque<QueuedOutbound>,
    dirty_panes: std::collections::HashSet<String>,
    /// Issue #4095: highest pane stream position a queued or delivered
    /// `terminal_snapshot` was serialized at, per pane. A `terminal_output`
    /// at or below it is already part of that snapshot and must not follow it.
    snapshot_stream_seq: HashMap<String, u64>,
    /// Issue #4206: panes whose streamed output has a hole in it. A chunk the
    /// client never received carried the cursor moves the following chunks
    /// position themselves against, so the surviving suffix is unusable until
    /// a snapshot re-establishes the screen. Cleared by that snapshot.
    torn_panes: std::collections::HashSet<String>,
    dropped_lossy: u64,
    dead: bool,
    close_frame: Option<ClientCloseFrame>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientCloseFrame {
    pub code: u16,
    pub reason: &'static str,
}

/// One step handed to the per-client drain loop in [`client_session`].
pub enum DrainStep {
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
pub struct ClientQueue {
    state: Mutex<ClientQueueState>,
    notify: tokio::sync::Notify,
}

impl ClientQueue {
    /// Enqueue one prepared event. Returns `true` when the client crossed
    /// the lossless hard cap and must be unregistered by the caller.
    pub fn enqueue(&self, message: &PreparedOutbound) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.dead {
            return true;
        }
        if Self::superseded_by_snapshot(&state, message) {
            return false;
        }
        if Self::withheld_from_torn_pane(&mut state, message) {
            return false;
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
                Self::heal_torn_pane(&mut state, message);
                Self::record_snapshot_position(&mut state, message);
                if let Some(entry) = state.entries.iter_mut().find(|entry| {
                    entry.kind == message.kind && entry.coalesce_key == message.coalesce_key
                }) {
                    entry.payload = message.payload.clone();
                    entry.stream_seq = message.stream_seq;
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
                        // Issue #4206: the hole starts here. Everything the
                        // pane streams from now on is positioned against a
                        // screen the client will never have.
                        state.torn_panes.insert(pane.clone());
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
            terminal_pane: message.terminal_pane.clone(),
            stream_seq: message.stream_seq,
        }
    }

    /// Issue #4095: a streamed chunk whose pane stream position is at or
    /// below a snapshot this queue already holds is reproduced by that
    /// snapshot; delivering it afterwards would re-apply its cursor moves.
    fn superseded_by_snapshot(state: &ClientQueueState, message: &PreparedOutbound) -> bool {
        if message.kind != "terminal_output" {
            return false;
        }
        let (Some(pane), Some(seq)) = (&message.terminal_pane, message.stream_seq) else {
            return false;
        };
        state
            .snapshot_stream_seq
            .get(pane)
            .is_some_and(|snapshot_seq| seq <= *snapshot_seq)
    }

    /// Issue #4206: hold back a torn pane's stream. Once a chunk is dropped
    /// under queue pressure the client's screen and the pane's byte stream
    /// have diverged, and every later chunk positions its text against the
    /// state the missing one produced — writing it paints unrelated output
    /// across whatever occupies those cells instead. The pane is re-marked
    /// dirty so the drain loop keeps asking for the snapshot that heals it;
    /// without that a pane whose first repair failed would never recover.
    fn withheld_from_torn_pane(state: &mut ClientQueueState, message: &PreparedOutbound) -> bool {
        let Some(pane) = &message.repair_pane_id else {
            return false;
        };
        if !state.torn_panes.contains(pane) {
            return false;
        }
        state.dropped_lossy += 1;
        state.dirty_panes.insert(pane.clone());
        true
    }

    /// Issue #4206: a snapshot replaces the client's screen wholesale, so it
    /// closes the pane's hole regardless of whether the stream position that
    /// produced it is known.
    fn heal_torn_pane(state: &mut ClientQueueState, message: &PreparedOutbound) {
        if message.kind != "terminal_snapshot" {
            return;
        }
        let Some(pane) = &message.terminal_pane else {
            return;
        };
        state.torn_panes.remove(pane);
        state.dirty_panes.remove(pane);
    }

    /// Issue #4095: remember the snapshot's stream position and drop every
    /// queued chunk of the same pane it already contains — including chunks
    /// queued after an older snapshot whose slot this one is about to reuse.
    fn record_snapshot_position(state: &mut ClientQueueState, message: &PreparedOutbound) {
        if message.kind != "terminal_snapshot" {
            return;
        }
        let (Some(pane), Some(seq)) = (&message.terminal_pane, message.stream_seq) else {
            return;
        };
        let position = state.snapshot_stream_seq.entry(pane.clone()).or_insert(0);
        *position = (*position).max(seq);
        let position = *position;
        state.entries.retain(|entry| {
            !(entry.kind == "terminal_output"
                && entry.terminal_pane.as_deref() == Some(pane.as_str())
                && entry
                    .stream_seq
                    .is_some_and(|chunk_seq| chunk_seq <= position))
        });
    }

    /// Pop the next message without waiting. `None` means the queue is
    /// currently empty (but alive).
    pub fn try_next(&self) -> Option<DrainStep> {
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
            payload: entry.payload.to_string(),
            repair_panes,
        })
    }

    /// Await the next drain step. Cancel-safe: a popped message is returned
    /// synchronously, never lost across an await point.
    pub async fn next(&self) -> DrainStep {
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
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub fn try_recv(&self) -> Option<String> {
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

#[cfg(test)]
type ClientHubDispatchHook = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Default)]
pub struct ClientHub {
    clients: Arc<Mutex<HashMap<String, ClientRegistration>>>,
    #[cfg(test)]
    before_dispatch_enqueue: Arc<Mutex<Option<ClientHubDispatchHook>>>,
}

#[derive(Clone)]
struct ClientRegistration {
    queue: Arc<ClientQueue>,
    receives_broadcasts: bool,
    scope: ClientScope,
}

fn target_selects(
    target: &DispatchTarget,
    client_id: &str,
    receives_broadcasts: bool,
    scope: &ClientScope,
) -> bool {
    match target {
        DispatchTarget::All => receives_broadcasts,
        DispatchTarget::Hub => receives_broadcasts && *scope == ClientScope::Hub,
        DispatchTarget::Project(key) => {
            receives_broadcasts
                && matches!(scope, ClientScope::Project(client_key) if client_key == key)
        }
        DispatchTarget::Client(id) => id == client_id,
    }
}

impl ClientHub {
    #[cfg(test)]
    fn set_before_dispatch_enqueue_hook(&self, hook: ClientHubDispatchHook) {
        *self
            .before_dispatch_enqueue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
    }

    #[cfg(any(test, feature = "test-gh-guard"))]
    pub fn register(&self, client_id: String) -> Arc<ClientQueue> {
        self.register_scoped(client_id, ClientScope::Hub)
    }

    pub fn register_scoped(&self, client_id: String, scope: ClientScope) -> Arc<ClientQueue> {
        self.register_with_broadcasts(client_id, true, scope)
    }

    pub fn scope(&self, client_id: &str) -> Option<ClientScope> {
        self.clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(client_id)
            .map(|registration| registration.scope.clone())
    }

    fn register_pane(&self, client_id: String) -> Arc<ClientQueue> {
        self.register_with_broadcasts(client_id, false, ClientScope::Hub)
    }

    fn register_with_broadcasts(
        &self,
        client_id: String,
        receives_broadcasts: bool,
        scope: ClientScope,
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
                    scope,
                },
            );
        queue
    }

    pub fn unregister(&self, client_id: &str) {
        self.unregister_with_close_frame(client_id, None);
    }

    pub fn unregister_with_close_frame(
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

    pub fn dispatch(&self, events: Vec<OutboundEvent>) {
        // Snapshot queue handles under a short-lived lock so serialization
        // and per-client enqueue work happen outside the registry mutex. This
        // keeps register/unregister responsive even when the broadcast batch
        // is large or one client is slow to drain its queue.
        let snapshot: Vec<(String, Arc<ClientQueue>, bool, ClientScope)> = {
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
                        registration.scope.clone(),
                    )
                })
                .collect()
        };

        // The test barrier intentionally sits after the registry snapshot
        // guard is dropped and before serialization or per-client enqueue.
        // This makes the lock boundary observable without relying on a
        // scheduler-sensitive latency assertion.
        #[cfg(test)]
        if let Some(hook) = self
            .before_dispatch_enqueue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            hook();
        }

        let mut dead_clients: Vec<String> = Vec::new();
        for outbound in events {
            let prepared = prepare_outbound_event(&outbound);
            for (client_id, queue, receives_broadcasts, scope) in &snapshot {
                if target_selects(&outbound.target, client_id, *receives_broadcasts, scope)
                    && queue.enqueue(&prepared)
                {
                    dead_clients.push(client_id.clone());
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
                target: "crate::client_hub",
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

    /// Issue #3777: enqueue a background-serialized Active Work snapshot
    /// without reserializing its large Work/event graph on the tao thread.
    pub fn dispatch_prepared_active_work(&self, payload: Arc<str>, target: DispatchTarget) {
        let snapshot: Vec<(String, Arc<ClientQueue>, bool, ClientScope)> = {
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
                        registration.scope.clone(),
                    )
                })
                .collect()
        };
        let kind = "active_work_projection";
        let prepared = PreparedOutbound {
            payload,
            kind,
            coalesce_key: None,
            repair_pane_id: None,
            class: queue_class_for_kind(kind),
            // Not a PTY event: it belongs to no terminal pane and carries no
            // position in a pane's output stream (Issue #4095).
            terminal_pane: None,
            stream_seq: None,
        };
        let mut dead_clients = Vec::new();
        for (client_id, queue, receives_broadcasts, scope) in snapshot {
            let selected = target_selects(&target, &client_id, receives_broadcasts, &scope);
            if selected && queue.enqueue(&prepared) {
                dead_clients.push(client_id);
            }
        }
        if !dead_clients.is_empty() {
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

pub fn agent_router(state: TransportState, access_log: AccessLogSink) -> Router {
    Router::new()
        .route("/internal/hook-live", post(hook_live_handler))
        .route("/internal/pane-ws", get(agent_pane_websocket_handler))
        .route("/internal/host-contract", post(host_contract_handler))
        .route(
            "/internal/execution-binding-probe",
            post(execution_binding_probe_handler),
        )
        .route(
            "/internal/execution-continuation",
            post(execution_continuation_handler),
        )
        .route(
            "/internal/execution-adoption",
            post(execution_adoption_handler),
        )
        .route("/internal/workspace-update", post(workspace_update_handler))
        .route(
            "/internal/work-materialization-probe",
            post(work_materialization_probe_handler),
        )
        .route(
            "/internal/work-terminalization",
            post(work_terminalization_handler),
        )
        .route(
            "/internal/build-abort-terminalization",
            post(build_abort_terminalization_handler),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            AccessLogPolicy::agent(access_log),
            access_log_middleware,
        ))
}

pub fn agent_bridge_bind_ip() -> IpAddr {
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
pub async fn access_log_middleware(
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

async fn agent_pane_websocket_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<TransportState>,
) -> Response {
    let Some(grant) = agent_capability_grant(&headers, &state) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    // Issue #3667: no durable-authority check at connect. The transport
    // carries observation for every authenticated capability — including a
    // settled session whose durable binding is no longer current, which must
    // not hold less authority than a session that never had a binding.
    // Producing mutation is re-checked per request by the in-session durable
    // fence below and again at runtime dispatch.
    ws.on_upgrade(move |socket| agent_pane_client_session(socket, state, grant))
}

async fn hook_live_handler(
    headers: HeaderMap,
    State(state): State<TransportState>,
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
    state.send(TransportEvent::RuntimeHook(event));
    StatusCode::NO_CONTENT
}

async fn workspace_update_handler(
    headers: HeaderMap,
    State(state): State<TransportState>,
    Json(request): Json<AgentWorkspaceUpdateRequest>,
) -> Response {
    let Some(principal) = agent_capability_principal(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::InvalidRequest,
                "agent capability is missing or invalid".to_string(),
            ),
        );
    };

    let Some(execution_binding) = principal.active_execution_binding().cloned() else {
        return execution_binding_error_response(
            "workspace_update_requires_active_execution_authority",
        );
    };
    let project_root = principal.canonical_project_root().to_path_buf();
    let session_id = principal.session_id().to_string();
    let mutation_project_root = project_root.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::apply_bound_authenticated_workspace_update(
            &mutation_project_root,
            &session_id,
            &execution_binding,
            request,
        )
    })
    .await;

    match result {
        Ok(Ok(receipt)) => {
            state.send(TransportEvent::WorkspaceProjectionChanged { project_root });
            Json(receipt).into_response()
        }
        Ok(Err(error)) => {
            let status = workspace_update_error_status(error.code);
            workspace_update_error_response(status, error)
        }
        Err(_) => workspace_update_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host workspace mutation task failed before a response was produced".to_string(),
            ),
        ),
    }
}

async fn work_terminalization_handler(
    headers: HeaderMap,
    State(state): State<TransportState>,
    Json(request): Json<AgentWorkTerminalizationRequest>,
) -> Response {
    let Some(principal) = agent_capability_principal(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::InvalidRequest,
                "agent capability is missing or invalid".to_string(),
            ),
        );
    };

    let Some(execution_binding) = principal.active_execution_binding().cloned() else {
        return execution_binding_error_response(
            "work_terminalization_requires_active_execution_authority",
        );
    };
    let project_root = principal.canonical_project_root().to_path_buf();
    let session_id = principal.session_id().to_string();
    let mutation_project_root = project_root.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::apply_bound_authenticated_work_terminalization(
            &mutation_project_root,
            &session_id,
            &execution_binding,
            request,
        )
    })
    .await;

    match result {
        Ok(Ok(receipt)) => {
            state.send(TransportEvent::WorkspaceProjectionChanged { project_root });
            Json(receipt).into_response()
        }
        Ok(Err(error)) => {
            let status = workspace_update_error_status(error.code);
            workspace_update_error_response(status, error)
        }
        Err(_) => workspace_update_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host Work terminalization task failed before a response was produced".to_string(),
            ),
        ),
    }
}

async fn build_abort_terminalization_handler(
    headers: HeaderMap,
    State(state): State<TransportState>,
    Json(request): Json<AgentBuildAbortTerminalizationRequest>,
) -> Response {
    let Some(principal) = agent_capability_principal(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::InvalidRequest,
                "agent capability is missing or invalid".to_string(),
            ),
        );
    };

    let Some(execution_binding) = principal.active_execution_binding().cloned() else {
        return execution_binding_error_response(
            "build_abort_terminalization_requires_bound_execution_authority",
        );
    };
    let project_root = principal.canonical_project_root().to_path_buf();
    let session_id = principal.session_id().to_string();
    let mutation_project_root = project_root.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::apply_bound_authenticated_blocked_build_abort_terminalization(
            &mutation_project_root,
            &session_id,
            &execution_binding,
            request,
        )
    })
    .await;

    match result {
        Ok(Ok(receipt)) => {
            state.send(TransportEvent::WorkspaceProjectionChanged { project_root });
            Json(receipt).into_response()
        }
        Ok(Err(error)) => {
            let status = workspace_update_error_status(error.code);
            workspace_update_error_response(status, error)
        }
        Err(_) => workspace_update_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host build abort mutation task failed before a response was produced".to_string(),
            ),
        ),
    }
}

async fn work_materialization_probe_handler(
    headers: HeaderMap,
    State(state): State<TransportState>,
    Json(request): Json<AgentWorkMaterializationProbeRequest>,
) -> Response {
    let Some(principal) = agent_capability_principal(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::InvalidRequest,
                "agent capability is missing or invalid",
            ),
        );
    };

    let Some(execution_binding) = principal.active_execution_binding().cloned() else {
        return execution_binding_error_response(
            "work_materialization_probe_requires_active_execution_authority",
        );
    };
    let project_root = principal.canonical_project_root().to_path_buf();
    let session_id = principal.session_id().to_string();
    let result = tokio::task::spawn_blocking(move || {
        crate::probe_bound_authenticated_work_materialization(
            &project_root,
            &session_id,
            &execution_binding,
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
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host Work materialization probe failed before a response was produced",
            ),
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

/// Answer the Host contract preflight (SPEC #3248 FR-242, Issue #4546).
///
/// Deliberately the one authenticated route that does *not* demand execution
/// authority. The preflight runs before a generation, a binding or a
/// capability exists — requiring one here would make the contract unprovable
/// exactly when it matters, and the launch would materialize first and
/// discover the mismatch afterwards, which is the failure this route removes.
///
/// It touches no store and spawns no work: the answer is assembled from
/// compile-time constants and the presented principal, which is what makes the
/// zero-side-effect guarantee checkable rather than merely asserted.
async fn host_contract_handler(
    headers: HeaderMap,
    State(state): State<TransportState>,
    Json(request): Json<crate::AgentHostContractRequest>,
) -> Response {
    let Some(principal) = agent_capability_principal(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::InvalidRequest,
                "agent capability is missing or invalid".to_string(),
            ),
        );
    };
    let capability_generation = principal
        .execution_binding()
        .map_or(0, |binding| binding.capability_generation);
    match crate::describe_authenticated_host_contract(
        &request,
        principal.session_id(),
        &state.host_instance_id,
        capability_generation,
    ) {
        Ok(receipt) => Json(receipt).into_response(),
        Err(error) => {
            let status = workspace_update_error_status(error.code);
            workspace_update_error_response(status, error)
        }
    }
}

async fn execution_binding_probe_handler(
    headers: HeaderMap,
    State(state): State<TransportState>,
    Json(request): Json<crate::AgentExecutionBindingProbeRequest>,
) -> Response {
    let Some(principal) = agent_capability_principal(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::InvalidRequest,
                "agent capability is missing or invalid".to_string(),
            ),
        );
    };
    // This route authorizes agent-initiated producing mutation. Prepared
    // authority is observation-only until the coordinator commits and
    // promotes the bearer, so this probe must never hand it a successful
    // receipt.
    let Some(execution_binding) = principal.active_execution_binding().cloned() else {
        return execution_binding_error_response(
            "execution_binding_probe_requires_active_execution_authority",
        );
    };
    let project_root = principal.canonical_project_root().to_path_buf();
    let session_id = principal.session_id().to_string();
    let host_instance_id = state.host_instance_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::probe_authenticated_execution_binding(
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
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host execution binding probe failed before a response was produced".to_string(),
            ),
        ),
    }
}

async fn execution_adoption_handler(
    headers: HeaderMap,
    State(state): State<TransportState>,
    Json(request): Json<crate::AgentExecutionAdoptionRequest>,
) -> Response {
    let Some(grant) = agent_capability_grant(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::InvalidRequest,
                "agent capability is missing or invalid",
            ),
        );
    };
    let project_root = grant.principal().canonical_project_root().to_path_buf();
    let capabilities = state.agent_capabilities.clone();
    match tokio::task::spawn_blocking(move || capabilities.adopt_execution(&grant, request)).await {
        Ok(Ok(receipt)) => {
            state.send(TransportEvent::WorkspaceProjectionChanged { project_root });
            Json(receipt).into_response()
        }
        Ok(Err(error)) => {
            workspace_update_error_response(workspace_update_error_status(error.code), error)
        }
        Err(_) => workspace_update_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host adoption failed; inspect execution.status before retrying",
            ),
        ),
    }
}

async fn execution_continuation_handler(
    headers: HeaderMap,
    State(state): State<TransportState>,
    Json(request): Json<crate::AgentExecutionContinuationRequest>,
) -> Response {
    let Some(grant) = agent_capability_grant(&headers, &state) else {
        return workspace_update_error_response(
            StatusCode::UNAUTHORIZED,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::InvalidRequest,
                "agent capability is missing or invalid".to_string(),
            ),
        );
    };
    let project_root = grant.principal().canonical_project_root().to_path_buf();
    let session_id = grant.principal().session_id().to_string();
    if let Err(error) = request.validate() {
        return workspace_update_error_response(workspace_update_error_status(error.code), error);
    }
    if grant.principal().execution_binding().is_some() {
        let (reply, response) = std::sync::mpsc::channel();
        state.send(TransportEvent::FreshExecutionReadyResend {
            grant: grant.clone(),
            request: request.clone(),
            reply,
        });
        match tokio::task::spawn_blocking(move || response.recv_timeout(Duration::from_secs(25)))
            .await
        {
            Ok(Ok(Ok(Some(receipt)))) => return Json(receipt).into_response(),
            Ok(Ok(Ok(None))) => {}
            Ok(Ok(Err(error))) => {
                return workspace_update_error_response(
                    workspace_update_error_status(error.code),
                    error,
                )
            }
            _ => {
                return workspace_update_error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    AgentWorkspaceUpdateError::new(
                        AgentWorkspaceUpdateErrorCode::Internal,
                        "Host readiness coordinator is unavailable; retry execution.continue",
                    ),
                )
            }
        }
    }
    let mutation_project_root = project_root.clone();
    let operation = tokio::task::spawn_blocking(move || {
        crate::continue_authenticated_execution(&mutation_project_root, &session_id, request)
    })
    .await;
    let (receipt, binding) = match operation {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            return workspace_update_error_response(
                workspace_update_error_status(error.code),
                error,
            );
        }
        Err(_) => {
            return workspace_update_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                AgentWorkspaceUpdateError::new(
                    AgentWorkspaceUpdateErrorCode::Internal,
                    "Host continuation task failed before a response was produced".to_string(),
                ),
            );
        }
    };
    if state
        .agent_capabilities
        .promote_continuation(&grant, &binding)
        .is_err()
        || !state
            .agent_capabilities
            .refresh_grant(&grant)
            .is_some_and(|current| current.principal().active_execution_binding() == Some(&binding))
    {
        return workspace_update_error_response(
            StatusCode::CONFLICT,
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::TransactionConflict,
                "agent capability changed before continuation authority could be published"
                    .to_string(),
            ),
        );
    }
    state.send(TransportEvent::WorkspaceProjectionChanged { project_root });
    Json(receipt).into_response()
}

fn execution_binding_error_response(diagnostic_reason: &'static str) -> Response {
    tracing::warn!(
        reason = diagnostic_reason,
        "Host-managed operation rejected an execution binding"
    );
    // Issue #4443 AC-2: `authority_mismatch` is the refusal agents got stuck
    // on, and "relaunch the Session" is not something an agent can do. Name the
    // diagnosis operation it can run, whose `available_recoveries` resolves to
    // the exact next operation for this record.
    let mut error = AgentWorkspaceUpdateError::new(
        AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch,
        "Execution binding is missing, stale, or no longer current; run JSON operation `execution.status` and follow its `available_recoveries`, or relaunch the Session",
    );
    error.diagnostic_reason = Some(diagnostic_reason.into());
    workspace_update_error_response(StatusCode::CONFLICT, error)
}

#[derive(Serialize)]
struct AgentWorkspaceUpdateErrorResponse {
    code: AgentWorkspaceUpdateErrorCode,
    reason: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic_reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mismatched_fields: Vec<String>,
    /// Issue #4443 AC-2: the route out, as canonical operation names the agent
    /// bridge is allowed to surface.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    recovery_operations: Vec<String>,
}

fn workspace_update_error_response(
    status: StatusCode,
    error: AgentWorkspaceUpdateError,
) -> Response {
    let reason = match error.code {
        AgentWorkspaceUpdateErrorCode::InvalidRequest => "invalid_request",
        AgentWorkspaceUpdateErrorCode::RelaunchRequired => "relaunch_required",
        AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch => "authority_mismatch",
        AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired => "workspace_ensure_required",
        AgentWorkspaceUpdateErrorCode::ProvenanceMismatch => "provenance_mismatch",
        AgentWorkspaceUpdateErrorCode::IdentityConflict => "identity_conflict",
        AgentWorkspaceUpdateErrorCode::TransactionConflict => "transaction_conflict",
        AgentWorkspaceUpdateErrorCode::Internal => "internal",
    };
    (
        status,
        Json(AgentWorkspaceUpdateErrorResponse {
            code: error.code,
            reason,
            message: error.message,
            diagnostic_reason: error.diagnostic_reason,
            mismatched_fields: error.mismatched_fields,
            recovery_operations: error.recovery_operations,
        }),
    )
        .into_response()
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
            FrontendEvent::ListWindows => Some(AgentFrontendRequest::ListWindows),
            // Issue #3629 AC-9: closing a scoped peer pane is a window
            // lifecycle operation, not a producing Work mutation — the PM
            // holds only an observation grant and pane.close is its everyday
            // recovery tool. Project scoping (`allowed_window_ids`) still
            // applies, and an uncorrelated self-close is refused by the
            // runtime dispatch.
            FrontendEvent::CloseWindow { id, request_id }
                if self.allowed_window_ids.contains(&id) =>
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
            FrontendEvent::RecoverRestoredWindow {
                id,
                session_id,
                child_pid,
                child_started_at,
            } if self.allowed_window_ids.contains(&id) => {
                Some(AgentFrontendRequest::RecoverRestoredWindow {
                    id,
                    session_id,
                    child_pid,
                    child_started_at,
                })
            }
            FrontendEvent::PmPaneSendInput {
                operation_id,
                window_id,
                text,
            } if Uuid::parse_str(&operation_id)
                .is_ok_and(|parsed| parsed.hyphenated().to_string() == operation_id) =>
            {
                Some(AgentFrontendRequest::PmSendInput {
                    operation_id,
                    window_id,
                    text,
                    responder: None,
                })
            }
            FrontendEvent::AgentIssueMonitorScanNow {
                expected_project_scope,
            } => Some(AgentFrontendRequest::IssueMonitorScanNow {
                expected_project_scope,
            }),
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
            "pane_sync_complete" => {
                for field in [
                    "empty_window_ids",
                    "busy_window_ids",
                    "unavailable_window_ids",
                    "failed_window_ids",
                ] {
                    value.get_mut(field)?.as_array_mut()?.retain(|id| {
                        id.as_str()
                            .is_some_and(|id| self.allowed_window_ids.contains(id))
                    });
                }
                serde_json::to_string(&value).ok()
            }
            "pane_send_result" if self.grant.principal().authorizes_producing_mutation() => value
                .get("window_id")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|id| self.allowed_window_ids.contains(id))
                .then_some(payload),
            "issue_monitor_scan_request_result" => Some(payload),
            // Issue #3629 AC-12: the close reply is already client-scoped by
            // its dispatch target; passing it through lets the requester hear
            // the outcome even after the window left the projection.
            "pane_close_result" => Some(payload),
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
    Browser(ClientScope),
    Agent(AgentPaneSessionScope),
}

enum ScopedFrontendRequest {
    Browser(FrontendEvent),
    Agent {
        grant: AgentCapabilityGrant,
        request: AgentFrontendRequest,
    },
    AgentPmRefusal {
        operation_id: String,
        window_id: String,
    },
}

impl ClientSessionScope {
    fn refresh_agent_grant(&mut self, registry: &AgentCapabilityRegistry) -> bool {
        match self {
            Self::Browser(_) => true,
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
            Self::Browser(_) => Some(ScopedFrontendRequest::Browser(event)),
            Self::Agent(scope) => {
                if let FrontendEvent::PmPaneSendInput {
                    operation_id,
                    window_id,
                    ..
                } = &event
                {
                    let canonical_operation_id = Uuid::parse_str(operation_id)
                        .is_ok_and(|parsed| parsed.hyphenated().to_string() == *operation_id);
                    if !canonical_operation_id {
                        return Some(ScopedFrontendRequest::AgentPmRefusal {
                            operation_id: operation_id.clone(),
                            window_id: window_id.clone(),
                        });
                    }
                }
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
            Self::Browser(_) => Some(payload),
            Self::Agent(scope) => scope.filter_outbound(payload),
        }
    }

    fn filter_repair_panes(&self, repair_panes: Vec<String>) -> Vec<String> {
        match self {
            Self::Browser(_) => repair_panes,
            Self::Agent(scope) => scope.filter_repair_panes(repair_panes),
        }
    }
}

pub async fn client_session(socket: WebSocket, state: TransportState, scope: ClientScope) {
    client_session_with_scope(socket, state, ClientSessionScope::Browser(scope)).await;
}

async fn agent_pane_client_session(
    socket: WebSocket,
    state: TransportState,
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

async fn send_agent_pm_terminal_result<S>(
    sender: &mut S,
    event: BackendEvent,
    deadline_after: Duration,
) where
    S: futures_util::Sink<Message> + Unpin,
{
    match serde_json::to_string(&event) {
        Ok(payload) => {
            let _ =
                tokio::time::timeout(deadline_after, sender.send(Message::Text(payload.into())))
                    .await;
        }
        Err(error) => {
            tracing::error!(%error, "failed to serialize direct PM message result");
        }
    }
}

fn correlate_agent_pm_terminal_result(
    operation_id: String,
    requested_window_id: String,
    event: BackendEvent,
) -> BackendEvent {
    match event {
        BackendEvent::PmMessageSendResult {
            operation_id: result_operation_id,
            status,
            window_id,
            reason,
        } if result_operation_id == operation_id
            && window_id
                .as_deref()
                .is_none_or(|window_id| window_id == requested_window_id) =>
        {
            BackendEvent::PmMessageSendResult {
                operation_id,
                status,
                window_id: Some(requested_window_id),
                reason,
            }
        }
        _ => BackendEvent::PmMessageSendResult {
            operation_id,
            status: "failed".to_string(),
            window_id: Some(requested_window_id),
            reason: Some("PM delivery returned a mismatched terminal result".to_string()),
        },
    }
}

fn opaque_agent_pm_target_refusal(operation_id: String, window_id: String) -> BackendEvent {
    BackendEvent::PmMessageSendResult {
        operation_id,
        status: "failed".to_string(),
        window_id: Some(window_id),
        reason: Some(AGENT_PM_TARGET_REFUSAL.to_string()),
    }
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
    state: TransportState,
    mut scope: ClientSessionScope,
) {
    let client_id = Uuid::new_v4().to_string();
    let outbound = match &scope {
        ClientSessionScope::Browser(scope) => state
            .clients
            .register_scoped(client_id.clone(), scope.clone()),
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
                            state.send(TransportEvent::ClientPaneSnapshotRepair {
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
                        let received_at = Instant::now();
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
                                        state.send(TransportEvent::BrowserFrontend {
                                            client_id: client_id.clone(), input_seq: input_seq.clone(), event, received_at,
                                        });
                                    }
                                    Some(ScopedFrontendRequest::AgentPmRefusal {
                                        operation_id,
                                        window_id,
                                    }) => {
                                        send_agent_pm_terminal_result(
                                            &mut sender,
                                            opaque_agent_pm_target_refusal(operation_id, window_id),
                                            AGENT_PM_TERMINAL_SEND_DEADLINE,
                                        )
                                        .await;
                                        break;
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
                                        let mut direct_pm_result = None;
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
                                            AgentFrontendRequest::PmSendInput {
                                                operation_id,
                                                window_id,
                                                responder,
                                                ..
                                            } => {
                                                let (direct_responder, result, cancellation) =
                                                    AgentPmSendResponder::channel();
                                                *responder = Some(direct_responder.clone());
                                                direct_pm_result = Some((
                                                    result,
                                                    cancellation,
                                                    operation_id.clone(),
                                                    window_id.clone(),
                                                ));
                                                None
                                            }
                                            _ => None,
                                        };
                                        let event_grant = grant.clone();
                                        let dispatched = state.agent_capabilities.dispatch_if_current(
                                            &grant,
                                            || {
                                                state.send(TransportEvent::AgentFrontend {
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
                                            if let Some((
                                                _direct_result,
                                                cancellation,
                                                operation_id,
                                                window_id,
                                            )) = direct_pm_result.take()
                                            {
                                                let _ = cancellation.cancel();
                                                send_agent_pm_terminal_result(
                                                    &mut sender,
                                                    BackendEvent::PmMessageSendResult {
                                                        operation_id,
                                                        status: "failed".to_string(),
                                                        window_id: Some(window_id),
                                                        reason: Some(
                                                            "PM capability changed before dispatch"
                                                                .to_string(),
                                                        ),
                                                    },
                                                    AGENT_PM_TERMINAL_SEND_DEADLINE,
                                                )
                                                .await;
                                            } else {
                                                send_agent_fence_close(
                                                    &mut sender,
                                                    AGENT_STALE_BINDING_CLOSE,
                                                )
                                                .await;
                                            }
                                            break;
                                        }
                                        if let Some((
                                            mut direct_result,
                                            cancellation,
                                            operation_id,
                                            window_id,
                                        )) = direct_pm_result
                                        {
                                            let deadline = tokio::time::Instant::now()
                                                + AGENT_PM_SEND_ACCEPTANCE_DEADLINE;
                                            let mut connected = true;
                                            let mut timed_out = false;
                                            let result = loop {
                                                tokio::select! {
                                                    result = &mut direct_result => break result.ok(),
                                                    incoming = receiver.next() => {
                                                        match incoming {
                                                            Some(Ok(Message::Close(_)))
                                                            | Some(Err(_))
                                                            | None => {
                                                                connected = false;
                                                                break None;
                                                            }
                                                            Some(Ok(_)) => {}
                                                        }
                                                    }
                                                    _ = tokio::time::sleep_until(deadline) => {
                                                        timed_out = true;
                                                        break None;
                                                    }
                                                }
                                            };
                                            let event = if let Some(result) = result {
                                                correlate_agent_pm_terminal_result(
                                                    operation_id,
                                                    window_id,
                                                    result,
                                                )
                                            } else {
                                                let mutation_committed = cancellation.cancel();
                                                BackendEvent::PmMessageSendResult {
                                                    operation_id,
                                                    // Issue #3608 (AC-2/AC-3): a committed input
                                                    // whose worker never answered is unverified,
                                                    // not failed, and nothing here observed the
                                                    // body sitting unsubmitted — only the durable
                                                    // receipt knows how it ended.
                                                    status: if mutation_committed {
                                                        "unverified".to_string()
                                                    } else {
                                                        "failed".to_string()
                                                    },
                                                    window_id: Some(window_id),
                                                    reason: Some(if mutation_committed {
                                                        "PM delivery returned no terminal result before the server acceptance deadline; the input was committed to the pane and the durable receipt records the final outcome — do not retry with a new operation"
                                                            .to_string()
                                                    } else if timed_out {
                                                        "PM delivery exceeded the server acceptance deadline"
                                                            .to_string()
                                                    } else {
                                                        "PM delivery origin disconnected before a terminal result"
                                                            .to_string()
                                                    }),
                                                }
                                            };
                                            if connected {
                                                send_agent_pm_terminal_result(
                                                    &mut sender,
                                                    event,
                                                    AGENT_PM_TERMINAL_SEND_DEADLINE,
                                                )
                                                .await;
                                            }
                                            break;
                                        }
                                        if let Some(mut direct_acceptance) = direct_acceptance {
                                            // Correlated self-close is a two-phase exchange. The
                                            // runtime owner first atomically accepts or rejects the
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

#[cfg(any(test, feature = "test-gh-guard"))]
pub fn hook_forward_authorized(headers: &HeaderMap, expected_token: &str) -> bool {
    bearer_token(headers)
        .is_some_and(|token| crate::agent_capability::constant_time_token_eq(token, expected_token))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty())
}

fn agent_capability_principal(
    headers: &HeaderMap,
    state: &TransportState,
) -> Option<AgentSessionPrincipal> {
    state
        .agent_capabilities
        .authenticate(bearer_token(headers)?)
}

fn agent_capability_grant(
    headers: &HeaderMap,
    state: &TransportState,
) -> Option<AgentCapabilityGrant> {
    let token = bearer_token(headers)?.to_string();
    let principal = state.agent_capabilities.authenticate(&token)?;
    Some(AgentCapabilityGrant::new(token, principal))
}

#[cfg(any(test, feature = "test-gh-guard"))]
impl ClientHub {
    pub fn register_pane_for_test(&self, id: String) -> Arc<ClientQueue> {
        self.register_pane(id)
    }
    pub fn first_agent_queue_for_test(&self) -> Option<Arc<ClientQueue>> {
        self.clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .find(|registration| !registration.receives_broadcasts)
            .map(|registration| registration.queue.clone())
    }
    pub fn scopes_for_test(&self) -> Vec<ClientScope> {
        self.clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .map(|registration| registration.scope.clone())
            .collect()
    }
}
#[cfg(any(test, feature = "test-gh-guard"))]
impl ClientQueue {
    pub fn len_for_test(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .len()
    }
    pub fn enqueue_workspace_for_test(&self, payload: Arc<str>) -> bool {
        self.enqueue(&PreparedOutbound {
            payload,
            kind: "workspace_state",
            coalesce_key: None,
            repair_pane_id: None,
            class: QueueClass::IdempotentLatest,
            terminal_pane: None,
            stream_seq: None,
        })
    }
}
#[cfg(any(test, feature = "test-gh-guard"))]
pub async fn send_agent_self_close_acceptance_for_test<S>(
    sender: &mut S,
    acceptance: AgentSelfCloseDirectAcceptance,
    deadline: Duration,
) where
    S: futures_util::Sink<Message> + Unpin,
{
    send_agent_self_close_acceptance(sender, acceptance, deadline).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttachmentProgressPhase, KnowledgeKind, KnowledgeSemanticRetry};
    use gwt_core::repo_hash::ProjectKey;
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as WireMessage};
    fn project_a() -> ProjectKey {
        ProjectKey::parse("0123456789abcdef").unwrap()
    }
    fn transport_all(event: BackendEvent) -> OutboundEvent {
        OutboundEvent {
            target: DispatchTarget::All,
            event,
            knowledge_wire_metadata: None,
            terminal_stream_seq: None,
            error_origin: None,
        }
    }
    #[tokio::test]
    async fn authenticated_http_and_websocket_dispatch_without_gui() {
        let project = tempfile::tempdir().expect("project");
        let registry = crate::agent_capability::AgentCapabilityRegistry::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let issuer = crate::agent_capability::AgentCapabilityIssuer::new(
            format!("http://{addr}/internal/hook-live"),
            format!("ws://{addr}/ws"),
            format!("ws://{addr}/internal/pane-ws"),
            registry.clone(),
        );
        let grant = issuer.issue(project.path(), "transport-session").unwrap();
        let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
        let state = TransportState::new(
            ClientHub::default(),
            registry,
            "test-host".into(),
            std::sync::Arc::new(move |event| {
                let _ = events.send(event);
            }),
        );
        let app = agent_router(state, AccessLogSink::default());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let response = reqwest::Client::new()
            .post(format!("http://{addr}/internal/hook-live"))
            .bearer_auth(&grant.token)
            .json(&serde_json::json!({
                "kind": "runtime_state", "gwt_session_id": "transport-session",
                "occurred_at": "2026-09-28T00:00:00Z"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(5), received.recv())
                .await
                .unwrap(),
            Some(TransportEvent::RuntimeHook(_))
        ));
        let mut request = format!("ws://{addr}/internal/pane-ws")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", grant.token).parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket
            .send(WireMessage::Text(r#"{"kind":"frontend_ready"}"#.into()))
            .await
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(5), received.recv())
                .await
                .unwrap(),
            Some(TransportEvent::AgentFrontend {
                request: crate::agent_capability::AgentFrontendRequest::Ready,
                ..
            })
        ));
        socket.close(None).await.unwrap();
        server.abort();
    }
    #[test]
    fn connected_agent_scope_refreshes_same_bearer_after_prepared_promotion() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
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

    /// Issue #3629 AC-9/AC-12: an authenticated observation grant (the PM has
    /// no Active execution binding) must be able to request close of a peer
    /// pane inside its own project scope, and the close reply kind must pass
    /// the outbound filter so the caller hears the outcome.
    #[test]
    fn agent_pane_scope_limits_recovery_to_project_windows() {
        let project = tempfile::tempdir().expect("project");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let principal = AgentSessionPrincipal::for_test(project.path(), "pm-session")
            .expect("observation principal");
        let mut scope = AgentPaneSessionScope::new(AgentCapabilityGrant::new(
            "test-capability".to_string(),
            principal,
        ));
        scope.allowed_window_ids.insert("owned-window".to_string());
        let request = |id: &str| {
            serde_json::from_value::<FrontendEvent>(serde_json::json!({
                "kind": "recover_restored_window", "id": id, "session_id": "restored-session",
                "child_pid": 123, "child_started_at": 456
            }))
            .expect("recovery request")
        };

        assert!(scope
            .filter_inbound(request("owned-window"))
            .is_some_and(
                |request| request.mutates_host_state() && !request.requires_producing_authority()
            ));
        assert!(scope.filter_inbound(request("foreign-window")).is_none());
    }

    #[test]
    fn agent_pane_scope_allows_observation_grant_close_and_passes_close_result() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let foreign = tempfile::tempdir().expect("foreign tempdir");
        let principal =
            AgentSessionPrincipal::for_test(project.path(), "session-pm").expect("agent principal");
        assert!(
            !principal.authorizes_producing_mutation(),
            "test precondition: the PM-shaped principal is observation-only"
        );
        let mut scope = super::AgentPaneSessionScope::new(super::AgentCapabilityGrant::new(
            "test-capability".to_string(),
            principal,
        ));
        let workspace = serde_json::json!({
            "kind": "workspace_state",
            "workspace": {
                "app_version": "test",
                "active_tab_id": "tab-owned",
                "recent_projects": [],
                "tabs": [
                    {
                        "id": "tab-owned",
                        "project_root": project.path(),
                        "workspace": { "windows": [{
                            "id": "tab-owned::agent-1",
                            "preset": "codex",
                            "status": "running",
                            "session_id": "target-session"
                        }] }
                    },
                    {
                        "id": "tab-foreign",
                        "project_root": foreign.path(),
                        "workspace": { "windows": [{ "id": "tab-foreign::agent-2" }] }
                    }
                ]
            }
        });
        scope
            .filter_outbound(workspace.to_string())
            .expect("workspace projection populates allowed ids");

        assert!(
            matches!(
                scope.filter_inbound(FrontendEvent::CloseWindow {
                    id: "tab-owned::agent-1".to_string(),
                    request_id: None,
                }),
                Some(super::AgentFrontendRequest::CloseWindow { .. })
            ),
            "an observation grant must be able to request a scoped peer close (Issue #3629 AC-9)"
        );
        assert!(
            scope
                .filter_inbound(FrontendEvent::CloseWindow {
                    id: "tab-foreign::agent-2".to_string(),
                    request_id: None,
                })
                .is_none(),
            "a foreign-project window stays out of reach"
        );
        assert!(
            scope
                .filter_outbound(
                    serde_json::json!({
                        "kind": "pane_close_result",
                        "ok": true,
                        "window_id": "tab-owned::agent-1",
                        "reason": null
                    })
                    .to_string()
                )
                .is_some(),
            "the close reply must reach the requesting client (Issue #3629 AC-12)"
        );
    }

    #[test]
    fn agent_pane_scope_filters_project_output_and_frontend_authority() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let foreign = tempfile::tempdir().expect("foreign tempdir");
        let principal =
            AgentSessionPrincipal::for_test(project.path(), "session-1").expect("agent principal");
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
                        "workspace": { "windows": [{
                            "id": "tab-owned::agent-1",
                            "preset": "codex",
                            "status": "idle",
                            "session_id": "target-session"
                        }] }
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
        assert!(matches!(
            scope.filter_inbound(FrontendEvent::ListWindows),
            Some(super::AgentFrontendRequest::ListWindows)
        ));

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
        let completion = scope
            .filter_outbound(
                serde_json::json!({
                    "kind": "pane_sync_complete",
                    "empty_window_ids": ["tab-owned::agent-1", "tab-foreign::agent-2"],
                    "busy_window_ids": ["tab-foreign::agent-2"],
                    "unavailable_window_ids": ["tab-owned::agent-1"],
                    "failed_window_ids": ["tab-foreign::agent-2"]
                })
                .to_string(),
            )
            .expect("pane completion reaches its origin client");
        let completion: serde_json::Value =
            serde_json::from_str(&completion).expect("filtered pane completion");
        assert_eq!(
            completion["empty_window_ids"],
            serde_json::json!(["tab-owned::agent-1"])
        );
        assert_eq!(completion["busy_window_ids"], serde_json::json!([]));
        assert_eq!(
            completion["unavailable_window_ids"],
            serde_json::json!(["tab-owned::agent-1"])
        );
        assert_eq!(completion["failed_window_ids"], serde_json::json!([]));
        assert!(
            scope
                .filter_inbound(FrontendEvent::CloseWindow {
                    id: "tab-owned::agent-1".to_string(),
                    request_id: None,
                })
                .is_some(),
            "Issue #3629 AC-9: an inspection principal may request a scoped peer close; \
             self-close correlation is enforced by the runtime dispatch"
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
        assert!(
            scope
                .filter_inbound(FrontendEvent::PmPaneSendInput {
                    operation_id: "72fc3cd4-ad49-43e3-bf3d-d791357643a3".to_string(),
                    window_id: "tab-owned::agent-1".to_string(),
                    text: "report status\r".to_string(),
                })
                .is_some(),
            "an authenticated principal must route PM delivery to the runtime gate instead of silently dropping it"
        );
        assert!(
            scope
                .filter_inbound(FrontendEvent::PmPaneSendInput {
                    operation_id: "72fc3cd4-ad49-43e3-bf3d-d791357643a4".to_string(),
                    window_id: "tab-foreign::agent-2".to_string(),
                    text: "must not cross projects\r".to_string(),
                })
                .is_some(),
            "canonical PM replays must reach the runtime's durable project gate even when the target is absent from the current projection"
        );
        assert!(matches!(
            scope.filter_inbound(FrontendEvent::AgentIssueMonitorScanNow {
                expected_project_scope: "scope-123".to_string(),
            }),
            Some(AgentFrontendRequest::IssueMonitorScanNow {
                expected_project_scope,
            }) if expected_project_scope == "scope-123"
        ));
        assert!(scope
            .filter_outbound(
                serde_json::json!({
                    "kind": "issue_monitor_scan_request_result",
                    "accepted": false,
                    "reason": "scan_already_in_flight"
                })
                .to_string()
            )
            .is_some());
        assert!(scope
            .filter_inbound(FrontendEvent::TerminalInput {
                id: "tab-owned::agent-1".to_string(),
                data: "not-authorized-on-agent-route".to_string(),
            })
            .is_none());

        let bound_principal = AgentSessionPrincipal::for_test_bound(
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
        // Issue #3552 AC-4: unblocking peer *close* (Issue #3629 AC-9) must not
        // leak into peer *injection*. An ordinary agent — even a fully bound,
        // producing one — keeps the SPEC-3050 FR-002 self-only contract here;
        // the only route into another pane's keyboard stays the registered-PM
        // path of SPEC-3431 FR-111.
        assert!(
            bound_scope
                .filter_inbound(FrontendEvent::PaneSendInput {
                    session_id: "session-peer".to_string(),
                    text: "hello".to_string(),
                })
                .is_none(),
            "a producing ordinary agent still cannot inject into a peer Session's pane"
        );

        let prepared_principal = AgentSessionPrincipal::for_test_prepared(
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
                .is_some(),
            "Issue #3629 AC-9: a scoped peer close is a window lifecycle \
             operation available to every authenticated grant"
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
        let mut scope = super::ClientSessionScope::Browser(ClientScope::Hub);
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

        clients.dispatch(vec![transport_all(terminal_snapshot(
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
            status: Box::new(crate::ProjectIndexStatusView::new(
                crate::ProjectIndexStatusState::Skipped,
                message,
            )),
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
                DrainStep::Closed(_) => break,
            }
        }
        (payloads, repairs)
    }

    fn knowledge_search_results() -> BackendEvent {
        BackendEvent::KnowledgeSearchResults {
            id: "tab-1::issue-1".to_string(),
            knowledge_kind: KnowledgeKind::Issue,
            query: "silent recovery".to_string(),
            request_id: 7,
            entries: Vec::new(),
            selected_number: None,
            empty_message: None,
            refresh_enabled: true,
        }
    }

    fn semantic_retry_directive() -> KnowledgeSemanticRetry {
        KnowledgeSemanticRetry {
            error_code: "SEARCH_UNAVAILABLE".to_string(),
            retryable: true,
            retry_after_ms: KNOWLEDGE_SEMANTIC_RETRY_INITIAL_DELAY_MS,
        }
    }

    #[test]
    fn prepared_active_work_enqueue_reuses_the_background_payload_allocation() {
        let queue = ClientQueue::default();
        let payload: Arc<str> = Arc::from("x".repeat(4 * 1024 * 1024));
        let prepared = PreparedOutbound {
            payload: payload.clone(),
            kind: "active_work_projection",
            coalesce_key: None,
            repair_pane_id: None,
            class: QueueClass::IdempotentLatest,
            terminal_pane: None,
            stream_seq: None,
        };

        assert!(!queue.enqueue(&prepared));

        let state = queue
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let queued = state.entries.front().expect("queued Active Work payload");
        assert!(
            Arc::ptr_eq(&queued.payload, &payload),
            "tao-side enqueue must retain the background Arc instead of cloning 4 MB"
        );
    }

    #[test]
    fn client_hub_injects_exact_semantic_retry_metadata_on_knowledge_search_wire() {
        let hub = ClientHub::default();
        let queue = hub.register("knowledge-client".to_string());
        hub.dispatch(vec![OutboundEvent::reply_with_knowledge_semantic_retry(
            "knowledge-client",
            knowledge_search_results(),
            Some(semantic_retry_directive()),
        )]);

        let (payloads, repairs) = drain_all(&queue);
        assert!(repairs.is_empty());
        assert_eq!(payloads.len(), 1);
        let value: serde_json::Value =
            serde_json::from_str(&payloads[0]).expect("knowledge search wire payload");
        let directive = value
            .get("semantic_retry")
            .expect("typed retry directive on outbound wire");
        assert_eq!(
            directive
                .get("error_code")
                .and_then(serde_json::Value::as_str),
            Some("SEARCH_UNAVAILABLE")
        );
        assert_eq!(
            directive
                .get("retryable")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            directive
                .get("retry_after_ms")
                .and_then(serde_json::Value::as_u64),
            Some(KNOWLEDGE_SEMANTIC_RETRY_INITIAL_DELAY_MS)
        );
        assert_eq!(
            directive.as_object().map(serde_json::Map::len),
            Some(3),
            "wire directive must contain no raw diagnostic: {directive}"
        );
    }

    #[test]
    fn client_hub_omits_absent_semantic_retry_metadata() {
        let hub = ClientHub::default();
        let queue = hub.register("knowledge-client".to_string());
        hub.dispatch(vec![OutboundEvent::reply_with_knowledge_semantic_retry(
            "knowledge-client",
            knowledge_search_results(),
            None,
        )]);

        let (payloads, _) = drain_all(&queue);
        assert_eq!(payloads.len(), 1);
        let value: serde_json::Value =
            serde_json::from_str(&payloads[0]).expect("knowledge search wire payload");
        assert!(
            value.get("semantic_retry").is_none(),
            "None metadata must be absent from the wire: {value}"
        );
    }

    #[test]
    fn outbound_marks_correlated_nonsemantic_knowledge_errors_on_the_private_wire() {
        let outbound = OutboundEvent::reply_with_nonsemantic_knowledge_error(
            "knowledge-client",
            BackendEvent::KnowledgeError {
                id: "tab-1::issue-1".to_string(),
                knowledge_kind: KnowledgeKind::Issue,
                request_id: Some(7),
                query: Some("silent recovery".to_string()),
                message: "failed to read issue cache".to_string(),
            },
        );

        let prepared = prepare_outbound_event(&outbound);
        let value: serde_json::Value =
            serde_json::from_str(&prepared.payload).expect("knowledge error wire payload");
        assert_eq!(
            value
                .get("error_domain")
                .and_then(serde_json::Value::as_str),
            Some("non_semantic")
        );
        assert!(
            value.get("semantic_retry").is_none(),
            "non-semantic errors must never carry retry metadata: {value}"
        );
    }

    #[test]
    fn outbound_wire_accepts_both_allowlisted_semantic_retry_codes() {
        for error_code in ["INDEX_NOT_READY", "SEARCH_UNAVAILABLE"] {
            let outbound = OutboundEvent::reply_with_knowledge_semantic_retry(
                "knowledge-client",
                knowledge_search_results(),
                Some(KnowledgeSemanticRetry {
                    error_code: error_code.to_string(),
                    retryable: true,
                    retry_after_ms: KNOWLEDGE_SEMANTIC_RETRY_INITIAL_DELAY_MS,
                }),
            );
            let prepared = prepare_outbound_event(&outbound);
            let value: serde_json::Value =
                serde_json::from_str(&prepared.payload).expect("knowledge search wire payload");
            assert_eq!(
                value
                    .get("semantic_retry")
                    .and_then(|directive| directive.get("error_code"))
                    .and_then(serde_json::Value::as_str),
                Some(error_code)
            );
        }
    }

    #[test]
    fn outbound_wire_omits_invalid_semantic_retry_directives() {
        let cases = [
            (
                "unknown code",
                KnowledgeSemanticRetry {
                    error_code: "FUTURE_CODE".to_string(),
                    retryable: true,
                    retry_after_ms: KNOWLEDGE_SEMANTIC_RETRY_INITIAL_DELAY_MS,
                },
            ),
            (
                "non-retryable flag",
                KnowledgeSemanticRetry {
                    error_code: "INDEX_NOT_READY".to_string(),
                    retryable: false,
                    retry_after_ms: KNOWLEDGE_SEMANTIC_RETRY_INITIAL_DELAY_MS,
                },
            ),
            (
                "zero delay",
                KnowledgeSemanticRetry {
                    error_code: "SEARCH_UNAVAILABLE".to_string(),
                    retryable: true,
                    retry_after_ms: 0,
                },
            ),
            (
                "unexpected delay",
                KnowledgeSemanticRetry {
                    error_code: "SEARCH_UNAVAILABLE".to_string(),
                    retryable: true,
                    retry_after_ms: 30_000,
                },
            ),
        ];

        for (case, directive) in cases {
            let outbound = OutboundEvent::reply_with_knowledge_semantic_retry(
                "knowledge-client",
                knowledge_search_results(),
                Some(directive),
            );
            let prepared = prepare_outbound_event(&outbound);
            let value: serde_json::Value =
                serde_json::from_str(&prepared.payload).expect("knowledge search wire payload");
            assert!(
                value.get("semantic_retry").is_none(),
                "{case} must not cross the wire: {value}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "requires KnowledgeSearchResults")]
    fn outbound_rejects_semantic_retry_metadata_for_non_knowledge_events() {
        let _ = OutboundEvent::reply_with_knowledge_semantic_retry(
            "knowledge-client",
            BackendEvent::KnowledgeError {
                id: "tab-1::issue-1".to_string(),
                knowledge_kind: KnowledgeKind::Issue,
                request_id: Some(7),
                query: Some("silent recovery".to_string()),
                message: "visible non-semantic failure".to_string(),
            },
            Some(semantic_retry_directive()),
        );
    }

    #[test]
    fn outbound_error_origin_stays_in_ledger_not_wire() {
        use gwt_core::{
            error_ledger::{self, ErrorScope},
            test_support::ScopedGwtHome,
        };
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let event = BackendEvent::IssueMonitorToast {
            notification_transition: None,
            level: "error".into(),
            message: "project launch error".into(),
            issue_number: Some(4735),
        };
        let outbound = OutboundEvent::reply("client", event.clone())
            .with_error_project_root(std::path::Path::new("/project-a"));
        let prepared = prepare_outbound_event(&outbound);
        assert_eq!(
            prepared.payload.as_ref(),
            serde_json::to_string(&event).unwrap()
        );
        prepare_outbound_event(&OutboundEvent::global_update_notice(
            "error",
            "host update error",
        ));
        prepare_outbound_event(&OutboundEvent::reply(
            "client",
            BackendEvent::IssueMonitorLaunchFailed {
                issue_number: 4735,
                message: "unknown launch error".into(),
            },
        ));
        let rows = error_ledger::list_since(None).unwrap();
        assert_eq!(rows.len(), 3);
        let project = rows
            .iter()
            .find(|row| row.message == "project launch error")
            .unwrap();
        assert_eq!(project.scope, ErrorScope::Project);
        assert_eq!(project.target.project_root.as_deref(), Some("/project-a"));
        assert_eq!(
            rows.iter()
                .find(|row| row.message == "host update error")
                .unwrap()
                .scope,
            ErrorScope::Host
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.message == "unknown launch error")
                .unwrap()
                .scope,
            ErrorScope::Unknown
        );
    }

    #[test]
    fn prepare_outbound_ignores_invalid_private_metadata_defensively() {
        let outbound = OutboundEvent {
            target: DispatchTarget::Client("knowledge-client".to_string()),
            event: BackendEvent::KnowledgeError {
                id: "tab-1::issue-1".to_string(),
                knowledge_kind: KnowledgeKind::Issue,
                request_id: Some(7),
                query: Some("silent recovery".to_string()),
                message: "visible non-semantic failure".to_string(),
            },
            knowledge_wire_metadata: Some(KnowledgeWireMetadata::SemanticRetry(
                semantic_retry_directive(),
            )),
            terminal_stream_seq: None,
            error_origin: None,
        };
        let prepared = prepare_outbound_event(&outbound);
        let value: serde_json::Value =
            serde_json::from_str(&prepared.payload).expect("non-knowledge wire payload");
        assert!(
            value.get("semantic_retry").is_none(),
            "defensive serializer must ignore invalid metadata: {value}"
        );
        assert!(
            value.get("error_domain").is_none(),
            "legacy/untyped errors must not gain a non-semantic marker: {value}"
        );
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

    #[test]
    fn terminal_preview_keeps_latest_for_each_of_three_panes() {
        let policy = crate::protocol::backend_event_policy("terminal_preview")
            .expect("preview event policy");
        assert!(
            !policy.coalesces_on_frontend(),
            "three panes must not coalesce by kind"
        );
        let queue = ClientQueue::default();
        for (id, text) in [
            ("a", "old"),
            ("b", "second"),
            ("c", "third"),
            ("a", "latest"),
        ] {
            let prepared = prepare_outbound(&BackendEvent::TerminalPreview {
                id: id.into(),
                text: text.into(),
            });
            assert!(prepared.terminal_pane.is_none());
            queue.enqueue(&prepared);
        }
        let (payloads, _) = drain_all(&queue);
        assert_eq!(payloads.len(), 3);
        assert!(payloads.iter().any(|value| value.contains("latest")));
        assert!(!payloads.iter().any(|value| value.contains("old")));
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

    fn terminal_output_at(pane: &str, data: &str, seq: u64) -> PreparedOutbound {
        prepare_outbound_event(
            &transport_all(terminal_output(pane, data)).with_terminal_stream_seq(Some(seq)),
        )
    }

    fn terminal_snapshot_at(pane: &str, data: &str, seq: u64) -> PreparedOutbound {
        prepare_outbound_event(
            &OutboundEvent::reply("client-1", terminal_snapshot(pane, data))
                .with_terminal_stream_seq(Some(seq)),
        )
    }

    fn drained_terminal_events(queue: &ClientQueue) -> Vec<String> {
        let (payloads, _) = drain_all(queue);
        payloads
            .iter()
            .filter_map(|payload| {
                let value: serde_json::Value = serde_json::from_str(payload).ok()?;
                let kind = value.get("kind")?.as_str()?;
                if !kind.starts_with("terminal_") {
                    return None;
                }
                let data = value.get("data_base64")?.as_str()?;
                Some(format!("{kind}:{data}"))
            })
            .collect()
    }

    // Issue #4095: a repair / reconnect snapshot is serialized on the event
    // loop from a pane the reader thread may already have advanced past the
    // last dispatched chunk. Chunks the snapshot already contains must not
    // follow it to the client, or their cursor-up / erase-line redraws land on
    // a screen that already moved.
    #[test]
    fn client_queue_never_replays_output_a_queued_snapshot_already_contains() {
        let queue = ClientQueue::default();
        let pane = "tab-1::agent-7";
        queue.enqueue(&terminal_output_at(pane, "chunk-1", 1));
        queue.enqueue(&terminal_output_at(pane, "chunk-2", 2));
        // Event loop: snapshot taken while the reader had parsed chunks 3-4.
        queue.enqueue(&terminal_snapshot_at(pane, "snapshot-4", 4));
        queue.enqueue(&terminal_output_at(pane, "chunk-3", 3));
        queue.enqueue(&terminal_output_at(pane, "chunk-4", 4));
        queue.enqueue(&terminal_output_at(pane, "chunk-5", 5));
        // Another pane and an un-sequenced legacy chunk are untouched.
        queue.enqueue(&terminal_output_at("tab-1::agent-8", "other-1", 1));
        queue.enqueue(&prepare_outbound(&terminal_output(pane, "unsequenced")));

        assert_eq!(
            drained_terminal_events(&queue),
            vec![
                "terminal_snapshot:snapshot-4".to_string(),
                "terminal_output:chunk-5".to_string(),
                "terminal_output:other-1".to_string(),
                "terminal_output:unsequenced".to_string(),
            ]
        );
        assert_eq!(
            queue.dropped_lossy(),
            0,
            "superseded chunks are not queue-pressure drops"
        );
    }

    // Issue #4095: SnapshotLatest keeps the older queue slot when a newer
    // snapshot replaces it, so chunks queued between the two would otherwise
    // be delivered after a snapshot that already contains them.
    #[test]
    fn client_queue_coalesced_snapshot_purges_output_it_already_contains() {
        let queue = ClientQueue::default();
        let pane = "tab-1::agent-7";
        queue.enqueue(&terminal_snapshot_at(pane, "snapshot-1", 1));
        queue.enqueue(&terminal_output_at(pane, "chunk-2", 2));
        queue.enqueue(&terminal_output_at("tab-1::agent-8", "other-2", 2));
        queue.enqueue(&terminal_snapshot_at(pane, "snapshot-2", 2));
        queue.enqueue(&terminal_output_at(pane, "chunk-3", 3));

        assert_eq!(
            drained_terminal_events(&queue),
            vec![
                "terminal_snapshot:snapshot-2".to_string(),
                "terminal_output:other-2".to_string(),
                "terminal_output:chunk-3".to_string(),
            ]
        );
    }

    // Issue #4206 AC-1 / AC-2: a dropped chunk tears the pane's byte stream.
    // Every later chunk of that pane positions its text relative to the screen
    // the dropped one produced, so delivering the surviving suffix paints at
    // offsets that never existed — two legitimate outputs of the same stream
    // crossing inside one line. The pane must stay silent from the drop until
    // a snapshot re-baselines the client screen.
    #[test]
    fn client_queue_withholds_torn_pane_output_until_a_snapshot_rebaselines_it() {
        let queue = ClientQueue::default();
        let torn = "tab-1::agent-7";
        let healthy = "tab-1::agent-8";

        // Saturate with unrelated lossless traffic so the pane's own delivery
        // is the only terminal_* content under test.
        for index in 0..LOSSY_HIGH_WATER {
            queue.enqueue(&prepare_outbound(&lossless_error(&format!("fill-{index}"))));
        }
        queue.enqueue(&terminal_output_at(torn, "dropped", 1));
        // Room again: the queue would accept this pane's next chunk, and
        // before Issue #4206 it did.
        let _ = queue.try_next();
        queue.enqueue(&terminal_output_at(torn, "post-hole", 2));
        queue.enqueue(&terminal_output_at(healthy, "unaffected", 2));

        assert_eq!(
            drained_terminal_events(&queue),
            vec!["terminal_output:unaffected".to_string()],
            "a torn pane stays silent until repaired; other panes are untouched"
        );

        queue.enqueue(&terminal_snapshot_at(torn, "repair", 3));
        queue.enqueue(&terminal_output_at(torn, "rebaselined", 4));

        assert_eq!(
            drained_terminal_events(&queue),
            vec![
                "terminal_snapshot:repair".to_string(),
                "terminal_output:rebaselined".to_string(),
            ],
            "the repair snapshot re-baselines the screen and the stream resumes"
        );
    }

    // Issue #4206 AC-1: withholding must not strand a pane. While torn, the
    // pane stays scheduled for repair so the drain loop keeps asking the event
    // loop for a snapshot instead of leaving the display frozen forever.
    #[test]
    fn client_queue_keeps_requesting_repair_while_a_pane_stays_torn() {
        let queue = ClientQueue::default();
        let pane = "tab-1::agent-7";

        for index in 0..LOSSY_HIGH_WATER {
            queue.enqueue(&prepare_outbound(&lossless_error(&format!("fill-{index}"))));
        }
        queue.enqueue(&terminal_output_at(pane, "dropped", 1));
        let (_, first_repairs) = drain_all(&queue);
        assert_eq!(first_repairs, vec![pane.to_string()]);

        // The repair snapshot never materialized (contended pane lock, empty
        // screen). The next withheld chunk must re-arm the request.
        queue.enqueue(&terminal_output_at(pane, "still-torn", 2));
        queue.enqueue(&prepare_outbound(&lossless_error("carrier")));
        let (_, second_repairs) = drain_all(&queue);
        assert_eq!(
            second_repairs,
            vec![pane.to_string()],
            "a still-torn pane is re-scheduled for repair, never left blank"
        );
    }

    // Issue #4206 AC-3: the reported corruption needed a saturated host — the
    // WebView stopped draining under CPU pressure while writers kept
    // producing. Reproduce that shape (concurrent writers against a drain that
    // runs behind them) and assert the property that actually protects the
    // screen: what a client receives for a pane is always an unbroken prefix
    // of what that pane produced. A gap is the corruption.
    #[test]
    fn client_queue_delivers_gap_free_pane_prefixes_under_concurrent_saturation() {
        use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

        const WRITERS: u64 = 4;
        const CHUNKS_PER_WRITER: u64 = 4_000;

        let queue = Arc::new(ClientQueue::default());
        let writers_done = Arc::new(AtomicBool::new(false));

        let writers = (0..WRITERS)
            .map(|writer| {
                let queue = Arc::clone(&queue);
                std::thread::spawn(move || {
                    let pane = format!("tab-1::agent-{writer}");
                    for index in 0..CHUNKS_PER_WRITER {
                        queue.enqueue(&terminal_output_at(&pane, &format!("{index}"), index + 1));
                    }
                })
            })
            .collect::<Vec<_>>();

        // A drain that keeps falling behind: the queue oscillates around the
        // high-water mark, so panes tear and the queue then has room again.
        let drainer = {
            let queue = Arc::clone(&queue);
            let writers_done = Arc::clone(&writers_done);
            std::thread::spawn(move || {
                let mut delivered: HashMap<String, Vec<u64>> = HashMap::new();
                loop {
                    let Some(DrainStep::Message { payload, .. }) = queue.try_next() else {
                        if writers_done.load(AtomicOrdering::Relaxed) {
                            break;
                        }
                        std::thread::yield_now();
                        continue;
                    };
                    let value: serde_json::Value =
                        serde_json::from_str(&payload).expect("outbound payload json");
                    if value.get("kind").and_then(serde_json::Value::as_str)
                        != Some("terminal_output")
                    {
                        continue;
                    }
                    let pane = value["id"].as_str().expect("pane id").to_string();
                    let index = value["data_base64"]
                        .as_str()
                        .expect("chunk label")
                        .parse::<u64>()
                        .expect("chunk index");
                    delivered.entry(pane).or_default().push(index);
                    std::thread::yield_now();
                }
                delivered
            })
        };

        for writer in writers {
            writer.join().expect("writer thread");
        }
        writers_done.store(true, AtomicOrdering::Relaxed);
        let delivered = drainer.join().expect("drain thread");

        assert!(
            queue.dropped_lossy() > 0,
            "the test must actually saturate the queue"
        );
        for (pane, indices) in &delivered {
            let expected = (0..indices.len() as u64).collect::<Vec<_>>();
            assert_eq!(
                indices, &expected,
                "{pane} received a torn stream: a gap means later chunks painted \
                 against a screen state the client never saw"
            );
        }
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

    /// Issue #3755 AC-2: pane hydration finishes with one lossless receipt.
    /// Queue pressure may drop terminal output, but it must preserve both the
    /// latest snapshot and the completion ordering observed by pane.read.
    #[test]
    fn client_queue_preserves_pane_snapshot_before_completion_under_pressure() {
        let queue = ClientQueue::default();
        let pane_id = "tab-1::agent-7";
        for index in 0..(LOSSY_HIGH_WATER + 10) {
            queue.enqueue(&prepare_outbound(&terminal_output(
                pane_id,
                &format!("chunk-{index}"),
            )));
        }

        assert!(
            !queue.enqueue(&prepare_outbound(&BackendEvent::TerminalSnapshot {
                id: pane_id.to_string(),
                data_base64: "c25hcHNob3QK".to_string(),
            }))
        );
        assert!(
            !queue.enqueue(&prepare_outbound(&BackendEvent::PaneSyncComplete {
                empty_window_ids: Vec::new(),
                busy_window_ids: Vec::new(),
                unavailable_window_ids: Vec::new(),
                failed_window_ids: Vec::new(),
            }))
        );

        let (payloads, _) = drain_all(&queue);
        let kinds = payloads
            .iter()
            .map(|payload| {
                serde_json::from_str::<serde_json::Value>(payload)
                    .expect("queued backend event json")
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .expect("queued backend event kind")
                    .to_string()
            })
            .collect::<Vec<_>>();
        let snapshot_index = kinds
            .iter()
            .position(|kind| kind == "terminal_snapshot")
            .expect("terminal snapshot survives pressure");
        let completion_index = kinds
            .iter()
            .position(|kind| kind == "pane_sync_complete")
            .expect("pane completion survives pressure");
        assert!(snapshot_index < completion_index);
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| kind.as_str() == "pane_sync_complete")
                .count(),
            1
        );
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
            matches!(queue.try_next(), Some(DrainStep::Closed(_))),
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
            hub.dispatch(vec![transport_all(terminal_output(
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

        hub.dispatch(vec![transport_all(lossless_error("after-flood"))]);
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
            .map(|index| transport_all(lossless_error(&format!("fill-{index}"))))
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
        let _receiver = hub.register("busy-client".to_string());
        let (dispatch_paused_tx, dispatch_paused_rx) = std::sync::mpsc::sync_channel(1);
        let (release_dispatch_tx, release_dispatch_rx) = std::sync::mpsc::sync_channel(1);
        let release_dispatch_rx = Arc::new(Mutex::new(release_dispatch_rx));
        hub.set_before_dispatch_enqueue_hook(Arc::new(move || {
            dispatch_paused_tx
                .send(())
                .expect("report dispatch enqueue phase");
            release_dispatch_rx
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .recv()
                .expect("release dispatch enqueue phase");
        }));
        let dispatch_hub = hub.clone();
        let dispatch_handle = std::thread::spawn(move || {
            dispatch_hub.dispatch(vec![transport_all(BackendEvent::ProjectOpenError {
                message: "blocked enqueue".to_string(),
                request_id: None,
            })]);
        });

        let dispatch_paused = dispatch_paused_rx.recv_timeout(Duration::from_secs(5));
        if dispatch_paused.is_err() {
            let _ = release_dispatch_tx.send(());
        }
        dispatch_paused.expect("dispatch should pause after releasing the client registry lock");

        let register_hub = hub.clone();
        let (register_done_tx, register_done_rx) = std::sync::mpsc::sync_channel(1);
        let register_handle = std::thread::spawn(move || {
            let queue = register_hub.register("intruder".to_string());
            register_done_tx
                .send(queue)
                .expect("report concurrent registration");
        });

        let registered = register_done_rx.recv_timeout(Duration::from_secs(5));
        let _ = release_dispatch_tx.send(());
        let _intruder_rx =
            registered.expect("register must complete while dispatch enqueue work is paused");
        register_handle.join().expect("register thread joins");
        dispatch_handle.join().expect("dispatch thread joins");
    }

    #[test]
    fn client_hub_filters_both_dispatch_paths_by_connection_scope() {
        let hub = ClientHub::default();
        let a = hub.register_scoped("a".into(), ClientScope::Project(project_a()));
        let b = hub.register_scoped(
            "b".into(),
            ClientScope::Project(ProjectKey::parse("fedcba9876543210").unwrap()),
        );
        let home = hub.register("home".into());
        let agent = hub.register_pane("agent".into());
        let queues = [&a, &b, &home, &agent];
        for (target, expected) in [
            (DispatchTarget::Project(project_a()), [1, 0, 0, 0]),
            (DispatchTarget::Hub, [0, 0, 1, 0]),
            (DispatchTarget::All, [1, 1, 1, 0]),
            (DispatchTarget::Client("agent".into()), [0, 0, 0, 1]),
        ] {
            let outbound = match &target {
                DispatchTarget::Project(key) => {
                    OutboundEvent::project(key.clone(), lossless_error("scope"))
                }
                _ => OutboundEvent {
                    target: target.clone(),
                    event: lossless_error("scope"),
                    knowledge_wire_metadata: None,
                    terminal_stream_seq: None,
                    error_origin: None,
                },
            };
            hub.dispatch(vec![outbound]);
            for (queue, count) in queues.iter().zip(expected) {
                assert_eq!(drain_all(queue).0.len(), count);
            }
            hub.dispatch_prepared_active_work(Arc::from("{}"), target);
            for (queue, count) in queues.iter().zip(expected) {
                assert_eq!(drain_all(queue).0.len(), count);
            }
        }
        assert_eq!(hub.scope("a"), Some(ClientScope::Project(project_a())));
        hub.unregister("a");
        assert_eq!(hub.scope("a"), None);
        hub.register("a".into());
        assert_eq!(hub.scope("a"), Some(ClientScope::Hub));
    }
}
