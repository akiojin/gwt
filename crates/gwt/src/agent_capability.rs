//! Process-local agent capability authority, shared by runtime owners and their transport.
use crate::{
    AgentWorkspaceUpdateError, AgentWorkspaceUpdateErrorCode, BackendEvent, HookForwardTarget,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use uuid::Uuid;

/// Budget for accepting a privileged PM send before its physical write begins.
pub const AGENT_PM_SEND_ACCEPTANCE_DEADLINE: Duration = Duration::from_secs(5);

/// Server-side identity authenticated by an opaque agent capability.
///
/// Neither field is accepted as routing authority from an agent request: the
/// registry derives this principal when the capability is issued and keeps it
/// process-local for the lifetime of the embedded server.
#[derive(Clone, PartialEq, Eq)]
pub struct AgentSessionPrincipal {
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
pub enum AgentExecutionAuthorityKind {
    Inspection,
    Prepared,
    Active,
}

/// One authenticated capability generation carried from the agent listener
/// to the runtime dispatcher. Its custom `Debug` implementation prevents the
/// bearer or principal from leaking through event diagnostics.
#[derive(Clone)]
pub struct AgentCapabilityGrant {
    token: String,
    principal: AgentSessionPrincipal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentDurableAuthority {
    ObservationOnly,
    Current,
    Stale,
    Unavailable,
}

impl AgentCapabilityGrant {
    pub fn new(token: String, principal: AgentSessionPrincipal) -> Self {
        Self { token, principal }
    }

    pub fn principal(&self) -> &AgentSessionPrincipal {
        &self.principal
    }

    pub fn matches_token(&self, token: &str) -> bool {
        self.token == token
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
pub enum AgentFrontendRequest {
    Ready,
    ListWindows,
    CloseWindow {
        id: String,
        request_id: Option<String>,
        responder: Option<AgentSelfCloseResponder>,
    },
    RecoverRestoredWindow {
        id: String,
        session_id: String,
        child_pid: u32,
        child_started_at: u64,
    },
    SendInput {
        text: String,
    },
    PmSendInput {
        operation_id: String,
        window_id: String,
        text: String,
        responder: Option<AgentPmSendResponder>,
    },
    IssueMonitorScanNow {
        expected_project_scope: String,
    },
}

impl std::fmt::Debug for AgentFrontendRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => formatter.write_str("AgentFrontendRequest::Ready"),
            Self::ListWindows => formatter.write_str("AgentFrontendRequest::ListWindows"),
            Self::CloseWindow { .. } => {
                formatter.write_str("AgentFrontendRequest::CloseWindow(<redacted>)")
            }
            Self::RecoverRestoredWindow { .. } => {
                formatter.write_str("AgentFrontendRequest::RecoverRestoredWindow(<redacted>)")
            }
            Self::SendInput { .. } => {
                formatter.write_str("AgentFrontendRequest::SendInput(<redacted>)")
            }
            Self::PmSendInput { .. } => {
                formatter.write_str("AgentFrontendRequest::PmSendInput(<redacted>)")
            }
            Self::IssueMonitorScanNow { .. } => {
                formatter.write_str("AgentFrontendRequest::IssueMonitorScanNow")
            }
        }
    }
}

impl AgentFrontendRequest {
    pub fn mutates_host_state(&self) -> bool {
        matches!(
            self,
            Self::CloseWindow { .. }
                | Self::RecoverRestoredWindow { .. }
                | Self::SendInput { .. }
                | Self::PmSendInput { .. }
                | Self::IssueMonitorScanNow { .. }
        )
    }

    pub fn requires_producing_authority(&self) -> bool {
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

    #[cfg(any(test, feature = "test-gh-guard"))]
    pub fn for_test(project_root: &Path, session_id: &str) -> Result<Self, String> {
        Self::new(project_root, session_id)
    }

    #[cfg(any(test, feature = "test-gh-guard"))]
    pub fn for_test_bound(
        project_root: &Path,
        session_id: &str,
        binding: gwt_agent::SessionExecutionBinding,
    ) -> Result<Self, String> {
        Self::new_bound(project_root, session_id, binding)
    }

    #[cfg(any(test, feature = "test-gh-guard"))]
    pub fn for_test_prepared(
        project_root: &Path,
        session_id: &str,
        binding: gwt_agent::SessionExecutionBinding,
    ) -> Result<Self, String> {
        Self::new_prepared(project_root, session_id, binding)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn canonical_project_root(&self) -> &Path {
        &self.canonical_project_root
    }

    pub fn execution_binding(&self) -> Option<&gwt_agent::SessionExecutionBinding> {
        self.execution_authority.binding()
    }

    pub fn active_execution_binding(&self) -> Option<&gwt_agent::SessionExecutionBinding> {
        match &self.execution_authority {
            AgentExecutionAuthority::Active(binding) => Some(binding),
            AgentExecutionAuthority::Inspection | AgentExecutionAuthority::Prepared(_) => None,
        }
    }

    pub fn prepared_execution_binding(&self) -> Option<&gwt_agent::SessionExecutionBinding> {
        match &self.execution_authority {
            AgentExecutionAuthority::Prepared(binding) => Some(binding),
            AgentExecutionAuthority::Inspection | AgentExecutionAuthority::Active(_) => None,
        }
    }

    pub fn authorizes_producing_mutation(&self) -> bool {
        self.active_execution_binding().is_some()
    }

    pub fn execution_authority_kind(&self) -> AgentExecutionAuthorityKind {
        match &self.execution_authority {
            AgentExecutionAuthority::Inspection => AgentExecutionAuthorityKind::Inspection,
            AgentExecutionAuthority::Prepared(_) => AgentExecutionAuthorityKind::Prepared,
            AgentExecutionAuthority::Active(_) => AgentExecutionAuthorityKind::Active,
        }
    }

    /// Kept as the narrow project-observation check for the forthcoming
    /// workspace-update route; hook-live only needs the canonical root value.
    #[allow(dead_code)]
    pub fn authorizes_project_root(&self, project_root: &Path) -> bool {
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
    if session.execution_binding.as_ref() != Some(binding)
        || session.repo_hash.as_deref() != Some(binding.repo_hash.as_str())
        || session.linked_issue_number != Some(binding.owner_number)
    {
        return AgentDurableAuthority::Stale;
    }
    let session_project_root = session
        .project_state_root
        .as_deref()
        .filter(|root| !root.as_os_str().is_empty())
        .unwrap_or(&session.worktree_path);
    let session_project_root = match dunce::canonicalize(session_project_root) {
        Ok(path) => gwt_core::paths::normalize_windows_child_process_path(&path),
        Err(_) => return AgentDurableAuthority::Unavailable,
    };
    if session_project_root != principal.canonical_project_root {
        return AgentDurableAuthority::Stale;
    }
    let owner_kind = match binding.owner_kind.as_str() {
        "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => return AgentDurableAuthority::Stale,
    };
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: binding.owner_number,
    };
    match crate::cli::execution_state::current_active_execution_binding_matches(
        &session.worktree_path,
        owner,
        principal.session_id(),
        &binding.identity,
    ) {
        Ok(true) => {}
        Ok(false) => return AgentDurableAuthority::Stale,
        Err(_) => return AgentDurableAuthority::Unavailable,
    }
    AgentDurableAuthority::Current
}

pub async fn durable_agent_execution_authority_async(
    principal: AgentSessionPrincipal,
) -> AgentDurableAuthority {
    if !principal.authorizes_producing_mutation() {
        return AgentDurableAuthority::ObservationOnly;
    }
    tokio::task::spawn_blocking(move || durable_agent_execution_authority(&principal))
        .await
        .unwrap_or(AgentDurableAuthority::Unavailable)
}

pub async fn durable_agent_execution_authority_with_lease_async(
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
        match crate::cli::execution_state::with_current_active_execution_binding_lease(
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
    manual_handoff_reservations: HashMap<String, ManualExecutionHandoffState>,
}

struct ManualExecutionHandoffState {
    binding: gwt_agent::SessionExecutionBinding,
    suspended: Option<SuspendedManualExecutionCapability>,
    restore_suspended_on_rollback: bool,
}

struct SuspendedManualExecutionCapability {
    token: String,
    principal: AgentSessionPrincipal,
    principal_key: (PathBuf, String),
}

struct ClosingAgentCapability {
    token: String,
    principal: AgentSessionPrincipal,
    revoked: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AgentSelfCloseCapabilityTicket {
    id: String,
}

impl std::fmt::Debug for AgentSelfCloseCapabilityTicket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentSelfCloseCapabilityTicket(<redacted>)")
    }
}

impl AgentSelfCloseCapabilityTicket {
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub fn for_test(id: String) -> Self {
        Self { id }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Opaque process-local fence held while the coordinator settles one exact
/// manual-launch predecessor. It contains no durable or bearer authority.
#[derive(Clone, PartialEq, Eq)]
pub struct ManualExecutionHandoffReservation {
    id: String,
    /// A committed stop fence may be claimed by the immediate successor
    /// preparation. Failed replay must retain that fence instead of reopening
    /// the predecessor capability issuance path.
    inherited_committed_fence: bool,
}

impl std::fmt::Debug for ManualExecutionHandoffReservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManualExecutionHandoffReservation(<redacted>)")
    }
}

/// One direct, origin-socket response channel for a correlated agent
/// self-close. It is deliberately absent from the transport client hub, so an
/// acceptance can neither be broadcast nor replayed to another connection.
#[derive(Clone)]
pub struct AgentSelfCloseResponder {
    sender: Arc<Mutex<Option<oneshot::Sender<AgentSelfCloseDirectAcceptance>>>>,
}

/// Direct origin-socket response channel for one privileged PM send.
/// Delivery results never enter the ambient browser/client event stream.
#[derive(Clone)]
pub struct AgentPmSendResponder {
    sender: Arc<Mutex<Option<oneshot::Sender<BackendEvent>>>>,
    mutation_state: Arc<AtomicU8>,
    deadline: Instant,
}

pub struct AgentPmSendCancellation {
    mutation_state: Arc<AtomicU8>,
}

const AGENT_PM_MUTATION_PENDING: u8 = 0;
const AGENT_PM_MUTATION_COMMITTED: u8 = 1;
const AGENT_PM_MUTATION_CANCELLED: u8 = 2;

impl AgentPmSendCancellation {
    /// Cancel a mutation that has not crossed its physical-I/O commit point.
    /// Returns `true` when input was already committed, so callers must report
    /// an ambiguous outcome rather than a safe refusal.
    pub fn cancel(&self) -> bool {
        match self.mutation_state.compare_exchange(
            AGENT_PM_MUTATION_PENDING,
            AGENT_PM_MUTATION_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => false,
            Err(state) => state == AGENT_PM_MUTATION_COMMITTED,
        }
    }
}

impl Drop for AgentPmSendCancellation {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

impl std::fmt::Debug for AgentPmSendResponder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentPmSendResponder(<redacted>)")
    }
}

impl AgentPmSendResponder {
    pub fn channel() -> (
        Self,
        oneshot::Receiver<BackendEvent>,
        AgentPmSendCancellation,
    ) {
        Self::channel_with_acceptance_window(AGENT_PM_SEND_ACCEPTANCE_DEADLINE)
    }

    /// The same channel with an explicit acceptance window. Tests that drive
    /// the acknowledgement wait to its end use this so they cost their own
    /// budget rather than the production one.
    pub fn channel_with_acceptance_window(
        acceptance_window: Duration,
    ) -> (
        Self,
        oneshot::Receiver<BackendEvent>,
        AgentPmSendCancellation,
    ) {
        let (sender, receiver) = oneshot::channel();
        let mutation_state = Arc::new(AtomicU8::new(AGENT_PM_MUTATION_PENDING));
        let responder = Self {
            sender: Arc::new(Mutex::new(Some(sender))),
            mutation_state: Arc::clone(&mutation_state),
            deadline: Instant::now() + acceptance_window,
        };
        (
            responder,
            receiver,
            AgentPmSendCancellation { mutation_state },
        )
    }

    pub fn mutation_is_current(&self) -> bool {
        self.mutation_state.load(Ordering::Acquire) != AGENT_PM_MUTATION_CANCELLED
            && Instant::now() < self.deadline
    }

    pub fn try_commit_mutation(&self) -> bool {
        if Instant::now() >= self.deadline {
            return false;
        }
        match self.mutation_state.compare_exchange(
            AGENT_PM_MUTATION_PENDING,
            AGENT_PM_MUTATION_COMMITTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => true,
            Err(state) => state == AGENT_PM_MUTATION_COMMITTED,
        }
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    #[allow(clippy::result_large_err)]
    pub fn send(&self, event: BackendEvent) -> Result<(), BackendEvent> {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(sender) = sender else {
            return Err(event);
        };
        sender.send(event)
    }
}

impl std::fmt::Debug for AgentSelfCloseResponder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentSelfCloseResponder(<redacted>)")
    }
}

impl AgentSelfCloseResponder {
    pub fn channel() -> (Self, oneshot::Receiver<AgentSelfCloseDirectAcceptance>) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                sender: Arc::new(Mutex::new(Some(sender))),
            },
            receiver,
        )
    }

    pub fn send(
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
pub struct AgentSelfCloseDirectAcceptance {
    request_id: String,
    window_id: String,
    ticket: Option<AgentSelfCloseCapabilityTicket>,
    completion: Arc<dyn Fn(AgentSelfCloseCapabilityTicket) + Send + Sync>,
}

impl std::fmt::Debug for AgentSelfCloseDirectAcceptance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentSelfCloseDirectAcceptance(<redacted>)")
    }
}

impl AgentSelfCloseDirectAcceptance {
    pub fn new(
        request_id: String,
        window_id: String,
        ticket: AgentSelfCloseCapabilityTicket,
        completion: Arc<dyn Fn(AgentSelfCloseCapabilityTicket) + Send + Sync>,
    ) -> Self {
        Self {
            request_id,
            window_id,
            ticket: Some(ticket),
            completion,
        }
    }

    pub fn wire_payload(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&BackendEvent::PaneCloseAccepted {
            request_id: self.request_id.clone(),
            window_id: self.window_id.clone(),
        })
    }

    pub fn disarm(mut self) -> AgentSelfCloseCapabilityTicket {
        self.ticket
            .take()
            .expect("self-close acceptance ticket is armed")
    }
}

impl Drop for AgentSelfCloseDirectAcceptance {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            (self.completion)(ticket);
        }
    }
}

/// Process-local map from opaque bearer capabilities to immutable Session
/// principals. A capability never persists to disk and its bearer is the only
/// identity material that crosses into an agent process or container.
#[derive(Clone, Default)]
pub struct AgentCapabilityRegistry {
    inner: Arc<RwLock<AgentCapabilityRegistryState>>,
}

struct AgentAdoptionPublisher<'a> {
    registry: &'a AgentCapabilityRegistry,
    grant: &'a AgentCapabilityGrant,
    guard: Option<std::sync::RwLockWriteGuard<'a, AgentCapabilityRegistryState>>,
    published: Option<gwt_agent::SessionExecutionBinding>,
}

impl crate::cli::execution_state::ExecutionAdoptionPublisher for AgentAdoptionPublisher<'_> {
    fn acquire(&mut self) -> std::io::Result<()> {
        // The durable coordinator invokes this only after owner and Session
        // leases. Never hold the registry while waiting for those leases.
        let guard = self
            .registry
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !AgentCapabilityRegistry::grant_is_current_in_state(&guard, self.grant)
            || guard
                .manual_handoff_reservations
                .values()
                .any(|reservation| {
                    self.grant.principal().active_execution_binding() == Some(&reservation.binding)
                })
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "agent capability changed or is reserved before adoption; no authority was updated",
            ));
        }
        self.guard = Some(guard);
        Ok(())
    }

    fn publish(&mut self, binding: gwt_agent::SessionExecutionBinding) {
        // Issue #4443 AC-10: publication without a held guard means the
        // transaction never acquired this grant. Dropping the publication is a
        // refusal the caller reports; panicking here reached the agent as an
        // opaque `500 code=internal` from the adoption handler instead.
        let Some(mut guard) = self.guard.take() else {
            return;
        };
        let mut principal = self.grant.principal().clone();
        principal.execution_authority = AgentExecutionAuthority::Active(Box::new(binding.clone()));
        guard
            .principals_by_token
            .insert(self.grant.token.clone(), principal);
        self.published = Some(binding);
    }
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

    pub fn issue(&self, project_root: &Path, session_id: &str) -> Result<String, String> {
        let principal = AgentSessionPrincipal::new(project_root, session_id)?;
        self.issue_principal(principal)
    }

    pub fn issue_bound(
        &self,
        project_root: &Path,
        session_id: &str,
        execution_binding: gwt_agent::SessionExecutionBinding,
    ) -> Result<String, String> {
        let principal =
            AgentSessionPrincipal::new_bound(project_root, session_id, execution_binding)?;
        self.issue_principal(principal)
    }

    pub fn issue_prepared(
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
        if state.manual_handoff_reservations.values().any(|reserved| {
            reserved
                .suspended
                .as_ref()
                .is_some_and(|suspended| suspended.principal_key == principal_key)
                || principal
                    .active_execution_binding()
                    .is_some_and(|binding| reserved.binding == *binding)
        }) {
            return Err("agent capability is reserved for manual execution handoff".to_string());
        }
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

    fn reserve_manual_execution_handoff(
        &self,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<ManualExecutionHandoffReservation, String> {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .principals_by_token
            .values()
            .any(|principal| principal.active_execution_binding() == Some(expected_binding))
        {
            return Err(
                "manual execution handoff refuses an active predecessor capability".to_string(),
            );
        }
        if let Some((id, reserved)) = state
            .manual_handoff_reservations
            .iter()
            .find(|(_, reserved)| reserved.binding == *expected_binding)
        {
            if reserved.suspended.is_some() {
                return Err("manual execution handoff is already reserved".to_string());
            }
            return Ok(ManualExecutionHandoffReservation {
                id: id.clone(),
                inherited_committed_fence: true,
            });
        }
        let id = loop {
            let candidate = format!("gwt_manual_handoff_{}", Uuid::new_v4());
            if !state.manual_handoff_reservations.contains_key(&candidate) {
                break candidate;
            }
        };
        state.manual_handoff_reservations.insert(
            id.clone(),
            ManualExecutionHandoffState {
                binding: expected_binding.clone(),
                suspended: None,
                restore_suspended_on_rollback: false,
            },
        );
        Ok(ManualExecutionHandoffReservation {
            id,
            inherited_committed_fence: false,
        })
    }

    fn begin_manual_execution_handoff(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<ManualExecutionHandoffReservation, String> {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .manual_handoff_reservations
            .values()
            .any(|reserved| reserved.binding == *expected_binding)
        {
            return Err("manual execution handoff is already reserved".to_string());
        }
        let issued_token = state
            .principals_by_token
            .keys()
            .find(|candidate| constant_time_token_eq(token, candidate))
            .cloned()
            .ok_or_else(|| "exact holder capability is missing or no longer current".to_string())?;
        let principal = state
            .principals_by_token
            .get(&issued_token)
            .cloned()
            .ok_or_else(|| "exact holder capability is missing or no longer current".to_string())?;
        if principal.active_execution_binding() != Some(expected_binding) {
            return Err("exact holder capability binding changed".to_string());
        }
        let principal_key = (
            principal.canonical_project_root().to_path_buf(),
            principal.session_id().to_string(),
        );
        if !state
            .token_by_project_session
            .get(&principal_key)
            .is_some_and(|current| constant_time_token_eq(&issued_token, current))
        {
            return Err("exact holder capability is missing or no longer current".to_string());
        }
        state.principals_by_token.remove(&issued_token);
        state.token_by_project_session.remove(&principal_key);
        let id = loop {
            let candidate = format!("gwt_manual_handoff_{}", Uuid::new_v4());
            if !state.manual_handoff_reservations.contains_key(&candidate) {
                break candidate;
            }
        };
        state.manual_handoff_reservations.insert(
            id.clone(),
            ManualExecutionHandoffState {
                binding: expected_binding.clone(),
                suspended: Some(SuspendedManualExecutionCapability {
                    token: issued_token,
                    principal,
                    principal_key,
                }),
                restore_suspended_on_rollback: true,
            },
        );
        Ok(ManualExecutionHandoffReservation {
            id,
            inherited_committed_fence: false,
        })
    }

    fn active_execution_binding_for_token(
        &self,
        token: &str,
    ) -> Option<gwt_agent::SessionExecutionBinding> {
        self.authenticate(token)?
            .active_execution_binding()
            .cloned()
    }

    fn self_close_active_execution_binding(
        &self,
        ticket: &AgentSelfCloseCapabilityTicket,
    ) -> Option<gwt_agent::SessionExecutionBinding> {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closing_by_ticket
            .get(ticket.id())
            .filter(|closing| !closing.revoked)
            .and_then(|closing| closing.principal.active_execution_binding().cloned())
    }

    /// Transfer one already-accepted correlated self-close into the same
    /// exact-generation handoff used by a manual close. This is deliberately
    /// one registry transaction: the direct ACK has already moved the bearer
    /// out of `principals_by_token`, so trying to begin a token handoff would
    /// always fail and leave the durable Session running.
    fn begin_self_close_manual_execution_handoff(
        &self,
        ticket: &AgentSelfCloseCapabilityTicket,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<ManualExecutionHandoffReservation, String> {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .manual_handoff_reservations
            .values()
            .any(|reserved| reserved.binding == *expected_binding)
        {
            return Err("manual execution handoff is already reserved".to_string());
        }
        let closing = state
            .closing_by_ticket
            .get(ticket.id())
            .ok_or_else(|| "self-close capability is missing or no longer current".to_string())?;
        if closing.revoked {
            return Err("self-close capability was revoked".to_string());
        }
        if closing.principal.active_execution_binding() != Some(expected_binding) {
            return Err("self-close capability binding changed".to_string());
        }

        let closing = state
            .closing_by_ticket
            .remove(ticket.id())
            .expect("validated self-close ticket remains present");
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
        let id = loop {
            let candidate = format!("gwt_manual_handoff_{}", Uuid::new_v4());
            if !state.manual_handoff_reservations.contains_key(&candidate) {
                break candidate;
            }
        };
        state.manual_handoff_reservations.insert(
            id.clone(),
            ManualExecutionHandoffState {
                binding: expected_binding.clone(),
                suspended: Some(SuspendedManualExecutionCapability {
                    token: closing.token,
                    principal: closing.principal,
                    principal_key,
                }),
                // The direct self-close ACK is the bearer revocation commit
                // point. Later PTY, persistence, or scheduling failures may
                // release this fence, but must never authenticate the closed
                // origin socket again.
                restore_suspended_on_rollback: false,
            },
        );
        Ok(ManualExecutionHandoffReservation {
            id,
            inherited_committed_fence: false,
        })
    }

    fn release_manual_execution_handoff(
        &self,
        reservation: &ManualExecutionHandoffReservation,
    ) -> bool {
        self.inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .manual_handoff_reservations
            .remove(&reservation.id)
            .is_some()
    }

    fn rollback_manual_execution_handoff(
        &self,
        reservation: &ManualExecutionHandoffReservation,
    ) -> bool {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if reservation.inherited_committed_fence {
            return state
                .manual_handoff_reservations
                .get(&reservation.id)
                .is_some_and(|handoff| handoff.suspended.is_none());
        }
        let Some(mut handoff) = state.manual_handoff_reservations.remove(&reservation.id) else {
            return false;
        };
        let Some(suspended) = handoff.suspended.take() else {
            return true;
        };
        if !handoff.restore_suspended_on_rollback {
            return true;
        }
        if state.principals_by_token.contains_key(&suspended.token)
            || state
                .token_by_project_session
                .contains_key(&suspended.principal_key)
        {
            handoff.suspended = Some(suspended);
            state
                .manual_handoff_reservations
                .insert(reservation.id.clone(), handoff);
            return false;
        }
        state
            .token_by_project_session
            .insert(suspended.principal_key, suspended.token.clone());
        state
            .principals_by_token
            .insert(suspended.token, suspended.principal);
        true
    }

    fn commit_manual_execution_handoff(
        &self,
        reservation: &ManualExecutionHandoffReservation,
    ) -> bool {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(handoff) = state.manual_handoff_reservations.get_mut(&reservation.id) else {
            return false;
        };
        handoff.suspended = None;
        true
    }

    pub fn promote_prepared(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<(), String> {
        self.promote_to_active(token, expected_binding, false, false)
    }

    fn promote_inspection(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<(), String> {
        self.promote_to_active(token, expected_binding, true, false)
    }

    pub fn promote_continuation(
        &self,
        grant: &AgentCapabilityGrant,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<(), String> {
        self.promote_to_active(&grant.token, expected_binding, true, true)
    }

    pub fn adopt_execution(
        &self,
        grant: &AgentCapabilityGrant,
        request: crate::AgentExecutionAdoptionRequest,
    ) -> Result<crate::AgentExecutionAdoptionReceipt, AgentWorkspaceUpdateError> {
        let principal = grant.principal();
        let binding = principal.active_execution_binding().ok_or_else(|| AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch,
            "execution.adopt requires a bound Host capability; use execution.continue for an unbound Session",
        ))?;
        let mut publisher = AgentAdoptionPublisher {
            registry: self,
            grant,
            guard: None,
            published: None,
        };
        crate::adopt_authenticated_execution(
            principal.canonical_project_root(),
            principal.session_id(),
            binding,
            request,
            &mut publisher,
        )?;
        // Issue #4443 AC-10: a success that published no binding leaves the
        // capability registry behind the durable record, so there is no receipt
        // to return. Report it as the conflict it is — the `.expect()` that
        // stood here panicked inside `spawn_blocking` and the handler's
        // `Err(_)` arm answered `500 code=internal`.
        let published = publisher.published.ok_or_else(|| {
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::TransactionConflict,
                "Host adoption settled the durable record but published no capability binding; run JSON operation `execution.status` and follow its `available_recoveries` before retrying",
            )
        })?;
        Ok(crate::AgentExecutionAdoptionReceipt {
            schema_version: 1,
            execution_binding: published,
        })
    }

    fn promote_to_active(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
        allow_inspection: bool,
        allow_active_replacement: bool,
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
        if state
            .manual_handoff_reservations
            .values()
            .any(|reserved| reserved.binding == *expected_binding)
        {
            return Err("agent capability is reserved for manual execution handoff".to_string());
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
            AgentExecutionAuthority::Active(_) if allow_active_replacement => {
                let mut promoted = principal;
                promoted.execution_authority =
                    AgentExecutionAuthority::Active(Box::new(expected_binding.clone()));
                state.principals_by_token.insert(issued_token, promoted);
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

    pub fn refresh_grant(&self, grant: &AgentCapabilityGrant) -> Option<AgentCapabilityGrant> {
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

    pub fn begin_self_close_if_current(
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

    pub fn rollback_self_close(&self, ticket: &AgentSelfCloseCapabilityTicket) -> bool {
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

    pub fn finish_self_close(&self, ticket: &AgentSelfCloseCapabilityTicket) -> bool {
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

    pub fn authenticate(&self, token: &str) -> Option<AgentSessionPrincipal> {
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
    pub fn dispatch_if_current(
        &self,
        grant: &AgentCapabilityGrant,
        dispatch: impl FnOnce(),
    ) -> bool {
        self.with_current_grant(grant, dispatch).is_some()
    }

    pub fn with_current_grant<T>(
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

    /// Commit one mutation while this exact grant remains current. The
    /// registry lock covers only the caller's non-blocking commit CAS and is
    /// released before any PTY I/O.
    pub fn commit_mutation_if_current(
        &self,
        grant: &AgentCapabilityGrant,
        commit: impl FnOnce() -> bool,
    ) -> bool {
        self.with_current_grant(grant, commit).unwrap_or(false)
    }

    pub fn grant_is_current(&self, grant: &AgentCapabilityGrant) -> bool {
        let state = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::grant_is_current_in_state(&state, grant)
    }

    /// Accept an operation from the exact current grant and return its
    /// authenticated principal without carrying the registry lock into the
    /// operation itself.
    ///
    /// This is the linearization boundary for an operation that may mutate a
    /// different capability while it runs. Rotation before this snapshot is
    /// rejected; rotation after it does not cancel the accepted operation.
    pub fn accept_current_grant(
        &self,
        grant: &AgentCapabilityGrant,
    ) -> Option<AgentSessionPrincipal> {
        let state = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !Self::grant_is_current_in_state(&state, grant) {
            return None;
        }

        Some(grant.principal().clone())
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
pub struct AgentCapabilityIssuer {
    hook_forward_url: String,
    pane_websocket_url: String,
    agent_pane_websocket_url: String,
    registry: AgentCapabilityRegistry,
}

impl AgentCapabilityIssuer {
    pub fn new(
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

    #[cfg(any(test, feature = "test-gh-guard"))]
    pub fn for_test(
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

    pub fn issue(
        &self,
        project_root: &Path,
        session_id: &str,
    ) -> Result<HookForwardTarget, String> {
        Ok(HookForwardTarget {
            url: self.hook_forward_url.clone(),
            token: self.registry.issue(project_root, session_id)?,
        })
    }

    pub fn preflight_issue(&self, project_root: &Path, session_id: &str) -> Result<(), String> {
        self.registry.preflight_issue(project_root, session_id)
    }

    pub fn issue_bound(
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

    pub fn issue_prepared(
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

    pub fn promote_prepared(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<(), String> {
        self.registry.promote_prepared(token, expected_binding)
    }

    pub fn promote_inspection(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<(), String> {
        self.registry.promote_inspection(token, expected_binding)
    }

    pub fn prepared_token_is_current(
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

    pub fn active_token_is_current(
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

    pub fn reserve_manual_execution_handoff(
        &self,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<ManualExecutionHandoffReservation, String> {
        self.registry
            .reserve_manual_execution_handoff(expected_binding)
    }

    pub fn begin_manual_execution_handoff(
        &self,
        token: &str,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<ManualExecutionHandoffReservation, String> {
        self.registry
            .begin_manual_execution_handoff(token, expected_binding)
    }

    pub fn active_execution_binding_for_token(
        &self,
        token: &str,
    ) -> Option<gwt_agent::SessionExecutionBinding> {
        self.registry.active_execution_binding_for_token(token)
    }

    pub fn self_close_active_execution_binding(
        &self,
        ticket: &AgentSelfCloseCapabilityTicket,
    ) -> Option<gwt_agent::SessionExecutionBinding> {
        self.registry.self_close_active_execution_binding(ticket)
    }

    pub fn begin_self_close_manual_execution_handoff(
        &self,
        ticket: &AgentSelfCloseCapabilityTicket,
        expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> Result<ManualExecutionHandoffReservation, String> {
        self.registry
            .begin_self_close_manual_execution_handoff(ticket, expected_binding)
    }

    pub fn release_manual_execution_handoff(
        &self,
        reservation: &ManualExecutionHandoffReservation,
    ) -> bool {
        self.registry.release_manual_execution_handoff(reservation)
    }

    pub fn rollback_manual_execution_handoff(
        &self,
        reservation: &ManualExecutionHandoffReservation,
    ) -> bool {
        self.registry.rollback_manual_execution_handoff(reservation)
    }

    pub fn commit_manual_execution_handoff(
        &self,
        reservation: &ManualExecutionHandoffReservation,
    ) -> bool {
        self.registry.commit_manual_execution_handoff(reservation)
    }

    pub fn revoke_token(&self, token: &str) -> bool {
        self.registry.revoke_token(token)
    }

    pub fn grant_is_current(&self, grant: &AgentCapabilityGrant) -> bool {
        self.registry.grant_is_current(grant)
    }

    pub fn accept_current_grant(
        &self,
        grant: &AgentCapabilityGrant,
    ) -> Option<AgentSessionPrincipal> {
        self.registry.accept_current_grant(grant)
    }

    /// Linearize one operation commit against capability rotation/revocation
    /// without carrying the global registry lock into physical I/O.
    pub fn commit_mutation_if_current(
        &self,
        grant: &AgentCapabilityGrant,
        commit: impl FnOnce() -> bool,
    ) -> bool {
        self.registry.commit_mutation_if_current(grant, commit)
    }

    pub fn durable_authority(&self, grant: &AgentCapabilityGrant) -> AgentDurableAuthority {
        durable_agent_execution_authority(grant.principal())
    }

    pub fn with_current_grant<T>(
        &self,
        grant: &AgentCapabilityGrant,
        dispatch: impl FnOnce() -> T,
    ) -> Option<T> {
        self.registry.with_current_grant(grant, dispatch)
    }

    pub fn begin_self_close_if_current(
        &self,
        grant: &AgentCapabilityGrant,
    ) -> Option<AgentSelfCloseCapabilityTicket> {
        self.registry.begin_self_close_if_current(grant)
    }

    pub fn rollback_self_close(&self, ticket: &AgentSelfCloseCapabilityTicket) -> bool {
        self.registry.rollback_self_close(ticket)
    }

    pub fn finish_self_close(&self, ticket: &AgentSelfCloseCapabilityTicket) -> bool {
        self.registry.finish_self_close(ticket)
    }

    #[cfg(any(test, feature = "test-gh-guard"))]
    pub fn authenticates_token(&self, token: &str) -> bool {
        self.registry.authenticate(token).is_some()
    }

    #[cfg(any(test, feature = "test-gh-guard"))]
    pub fn grant_for_test(&self, token: &str) -> Option<AgentCapabilityGrant> {
        self.registry
            .authenticate(token)
            .map(|principal| AgentCapabilityGrant::new(token.to_string(), principal))
    }

    pub fn pane_websocket_url(&self) -> &str {
        &self.pane_websocket_url
    }

    pub fn hook_forward_url(&self) -> &str {
        &self.hook_forward_url
    }

    pub fn agent_pane_websocket_url(&self) -> &str {
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

pub fn constant_time_token_eq(left: &str, right: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_session_principal_canonicalizes_project_and_redacts_debug() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project_state_root.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
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
    fn manual_handoff_reservation_refuses_an_existing_active_predecessor() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45155/internal/hook-live",
            "ws://127.0.0.1:46255/ws",
            "ws://127.0.0.1:45155/internal/pane-ws",
        );
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-reserved-active".to_string(),
            repo_hash: "repo-reserved-active".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3547,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-reserved-active".to_string(),
                binding_id: "binding-reserved-active".to_string(),
                ledger_head_hash: "head-reserved-active".to_string(),
            },
            capability_generation: 4,
        };
        issuer
            .issue_bound(project.path(), &binding.session_id, binding.clone())
            .expect("active predecessor capability");

        let error = issuer
            .reserve_manual_execution_handoff(&binding)
            .expect_err("an active predecessor handshake must refuse reservation");

        assert_eq!(
            error,
            "manual execution handoff refuses an active predecessor capability"
        );
        assert!(!format!("{error:?}").contains(&binding.identity.binding_id));
    }

    #[test]
    fn manual_handoff_reservation_blocks_matching_issue_and_promotion_only() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45155/internal/hook-live",
            "ws://127.0.0.1:46255/ws",
            "ws://127.0.0.1:45155/internal/pane-ws",
        );
        let predecessor = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-reserved-predecessor".to_string(),
            repo_hash: "repo-reserved".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3547,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-reserved-predecessor".to_string(),
                binding_id: "binding-reserved-predecessor".to_string(),
                ledger_head_hash: "head-reserved-predecessor".to_string(),
            },
            capability_generation: 7,
        };
        let prepared = issuer
            .issue_prepared(project.path(), &predecessor.session_id, predecessor.clone())
            .expect("Prepared predecessor capability");
        let reservation = issuer
            .reserve_manual_execution_handoff(&predecessor)
            .expect("reserve exact predecessor handoff");

        let issue_error = issuer
            .issue_bound(project.path(), &predecessor.session_id, predecessor.clone())
            .expect_err("matching Active issue must be fenced");
        assert_eq!(
            issue_error,
            "agent capability is reserved for manual execution handoff"
        );
        let promotion_error = issuer
            .promote_prepared(&prepared.token, &predecessor)
            .expect_err("matching promotion must be fenced");
        assert_eq!(
            promotion_error,
            "agent capability is reserved for manual execution handoff"
        );

        let successor = gwt_agent::SessionExecutionBinding {
            session_id: "session-reserved-successor".to_string(),
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-reserved-successor".to_string(),
                binding_id: "binding-reserved-successor".to_string(),
                ledger_head_hash: "head-reserved-successor".to_string(),
            },
            capability_generation: 1,
            ..predecessor.clone()
        };
        let successor_session_id = successor.session_id.clone();
        issuer
            .issue_prepared(project.path(), &successor_session_id, successor)
            .expect("a distinct Prepared successor is not fenced");

        assert!(issuer.release_manual_execution_handoff(&reservation));
        issuer
            .promote_prepared(&prepared.token, &predecessor)
            .expect("exact promotion is allowed after release");
        issuer
            .issue_bound(project.path(), &predecessor.session_id, predecessor.clone())
            .expect("exact Active issue is allowed after release");
        assert!(!issuer.release_manual_execution_handoff(&reservation));
    }

    #[test]
    fn manual_handoff_begin_suspends_exact_active_capability_and_can_rollback_or_commit() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45155/internal/hook-live",
            "ws://127.0.0.1:46255/ws",
            "ws://127.0.0.1:45155/internal/pane-ws",
        );
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-stop-handoff".to_string(),
            repo_hash: "repo-stop-handoff".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3547,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-stop-handoff".to_string(),
                binding_id: "binding-stop-handoff".to_string(),
                ledger_head_hash: "head-stop-handoff".to_string(),
            },
            capability_generation: 9,
        };
        let active = issuer
            .issue_bound(project.path(), &binding.session_id, binding.clone())
            .expect("issue exact active holder");

        let reservation = issuer
            .begin_manual_execution_handoff(&active.token, &binding)
            .expect("suspend exact active holder");
        assert!(!issuer.active_token_is_current(&active.token, &binding));
        assert!(issuer
            .issue_bound(project.path(), &binding.session_id, binding.clone())
            .is_err());
        assert!(issuer.rollback_manual_execution_handoff(&reservation));
        assert!(issuer.active_token_is_current(&active.token, &binding));

        let reservation = issuer
            .begin_manual_execution_handoff(&active.token, &binding)
            .expect("suspend exact active holder again");
        assert!(issuer.commit_manual_execution_handoff(&reservation));
        assert!(!issuer.active_token_is_current(&active.token, &binding));
        assert!(issuer
            .issue_bound(project.path(), &binding.session_id, binding.clone())
            .is_err());
        assert!(issuer.release_manual_execution_handoff(&reservation));
    }

    #[test]
    fn accepted_self_close_handoff_rollback_releases_fence_without_restoring_bearer() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45155/internal/hook-live",
            "ws://127.0.0.1:46255/ws",
            "ws://127.0.0.1:45155/internal/pane-ws",
        );
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-self-close-handoff".to_string(),
            repo_hash: "repo-self-close-handoff".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3783,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-self-close-handoff".to_string(),
                binding_id: "binding-self-close-handoff".to_string(),
                ledger_head_hash: "head-self-close-handoff".to_string(),
            },
            capability_generation: 1,
        };
        let active = issuer
            .issue_bound(project.path(), &binding.session_id, binding.clone())
            .expect("issue exact active holder");
        let grant = issuer
            .grant_for_test(&active.token)
            .expect("current capability grant");
        let ticket = issuer
            .begin_self_close_if_current(&grant)
            .expect("accept correlated self-close");
        let reservation = issuer
            .begin_self_close_manual_execution_handoff(&ticket, &binding)
            .expect("transfer accepted self-close into exact handoff");

        assert!(!issuer.authenticates_token(&active.token));
        assert!(
            issuer
                .issue_bound(project.path(), &binding.session_id, binding.clone())
                .is_err(),
            "the in-flight finalizer fence must block replacement capability issuance"
        );

        assert!(issuer.rollback_manual_execution_handoff(&reservation));
        assert!(
            !issuer.authenticates_token(&active.token),
            "an accepted self-close bearer must stay revoked when finalization fails"
        );
        assert!(!issuer.grant_is_current(&grant));
        let replacement = issuer
            .issue_bound(project.path(), &binding.session_id, binding.clone())
            .expect("rollback releases only the finalizer fence");
        assert_ne!(replacement.token, active.token);
        assert!(issuer.active_token_is_current(&replacement.token, &binding));
        assert!(!issuer.rollback_manual_execution_handoff(&reservation));
    }

    #[test]
    fn agent_capability_registry_promotes_legacy_inspection_without_rotating_bearer() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
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
    fn continuation_promotion_can_replace_only_the_current_bearers_active_binding() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let registry = AgentCapabilityRegistry::default();
        let predecessor = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-continuation".to_string(),
            repo_hash: "repo-continuation".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3393,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-predecessor".to_string(),
                binding_id: "binding-predecessor".to_string(),
                ledger_head_hash: "head-predecessor".to_string(),
            },
            capability_generation: 1,
        };
        let token = registry
            .issue_bound(project.path(), "session-continuation", predecessor.clone())
            .expect("Active capability");
        let stale = AgentCapabilityGrant::new(
            token.clone(),
            registry.authenticate(&token).expect("authenticate bearer"),
        );
        let mut successor = predecessor;
        successor.identity.generation_id = "generation-successor".to_string();
        successor.identity.binding_id = "binding-successor".to_string();
        successor.identity.ledger_head_hash = "head-successor".to_string();
        successor.capability_generation = 2;

        assert!(
            registry.promote_inspection(&token, &successor).is_err(),
            "ordinary inspection promotion cannot replace Active authority"
        );
        registry
            .promote_continuation(&stale, &successor)
            .expect("validated continuation may replace Active authority");
        let current = registry
            .refresh_grant(&stale)
            .expect("same current bearer refreshes after continuation");
        assert_eq!(
            current.principal().active_execution_binding(),
            Some(&successor)
        );

        let rotated = registry
            .issue_bound(project.path(), "session-continuation", successor.clone())
            .expect("rotate bearer");
        assert_ne!(rotated, token);
        assert!(registry.promote_continuation(&stale, &successor).is_err());
    }

    #[test]
    fn agent_capability_issue_preflight_is_non_issuing_and_rejects_closing_principal() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project_a.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project_a.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
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
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(root.path());
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
    fn agent_capability_issuer_debug_never_contains_secret_or_principal() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
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
    fn self_close_completion_sink_runs_without_gui_event_loop() {
        let completed = Arc::new(Mutex::new(Vec::new()));
        let sink = completed.clone();
        let acceptance = AgentSelfCloseDirectAcceptance::new(
            "request".to_string(),
            "pane".to_string(),
            AgentSelfCloseCapabilityTicket {
                id: "ticket".to_string(),
            },
            Arc::new(move |ticket: AgentSelfCloseCapabilityTicket| {
                sink.lock().unwrap().push(ticket.id().to_string());
            }),
        );
        drop(acceptance);
        assert_eq!(*completed.lock().unwrap(), vec!["ticket"]);
    }
}
