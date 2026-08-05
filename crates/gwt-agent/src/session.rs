//! Agent session persistence: save/load sessions as TOML files.

use std::{
    fs::{self, File, OpenOptions},
    io,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    launch::{normalize_launch_args, LaunchConfig},
    types::{
        AgentId, AgentStatus, DockerLifecycleIntent, LaunchRuntimeTarget, SessionMode,
        WindowsShellKind, WorkflowBypass,
    },
};

/// Idle duration (in seconds) after which a session is considered stopped.
const IDLE_TIMEOUT_SECS: i64 = 60;
const CODEX_PLACEHOLDER_SESSION_ID: &str = "agent-session";
const SESSION_LEASE_WAIT: Duration = Duration::from_secs(2);
const SESSION_LEASE_POLL: Duration = Duration::from_millis(25);

std::thread_local! {
    static SESSION_LEASE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

struct SessionLeaseThreadGuard;

impl SessionLeaseThreadGuard {
    fn enter() -> io::Result<Self> {
        SESSION_LEASE_DEPTH.with(|depth| {
            if depth.get() != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "nested Session lease is refused; retry after the current Session operation",
                ));
            }
            depth.set(1);
            Ok(Self)
        })
    }
}

impl Drop for SessionLeaseThreadGuard {
    fn drop(&mut self) {
        SESSION_LEASE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Return whether this thread already holds a Session persistence lease.
///
/// Owner+Session transactions use this to reject the reverse Session→owner
/// acquisition order before taking the owner lease.
#[doc(hidden)]
pub fn current_thread_holds_session_lease() -> bool {
    SESSION_LEASE_DEPTH.with(|depth| depth.get() != 0)
}

/// Environment variable injected into agent PTYs so hooks can identify the
/// backing gwt session.
pub const GWT_SESSION_ID_ENV: &str = "GWT_SESSION_ID";
/// One-time challenge injected only for a Prepared Continue work launch.
///
/// The candidate returns it with its authenticated SessionStart event. The
/// coordinating Host consumes it before committing generation/Work state.
pub const GWT_CONTINUE_WORK_READY_NONCE_ENV: &str = "GWT_CONTINUE_WORK_READY_TOKEN";
/// Environment variable injected into agent PTYs so hooks can write the
/// matching runtime sidecar without discovering gwt paths on their own.
pub const GWT_SESSION_RUNTIME_PATH_ENV: &str = "GWT_SESSION_RUNTIME_PATH";
/// Immutable producing capability epoch assigned to one launched agent
/// process. Runtime hooks and Host shutdown paths use it as a compare-and-swap
/// fence; they must never recover a newer value from mutable Session state.
pub const GWT_EXECUTION_CAPABILITY_GENERATION_ENV: &str = "GWT_EXECUTION_CAPABILITY_GENERATION";
/// Environment variable injected into agent PTYs so skills can locate the
/// gwt binary for calling gwtd CLI (GitHub operations, etc.).
pub const GWT_BIN_PATH_ENV: &str = "GWT_BIN_PATH";
/// Loopback endpoint used by daemon-owned hook live events.
pub const GWT_HOOK_FORWARD_URL_ENV: &str = "GWT_HOOK_FORWARD_URL";
/// Bearer token paired with [`GWT_HOOK_FORWARD_URL_ENV`] and authenticated
/// agent-listener pane WebSockets.
pub const GWT_HOOK_FORWARD_TOKEN_ENV: &str = "GWT_HOOK_FORWARD_TOKEN";
/// Explicit WebSocket endpoint used by `gwtd pane.*` operations.
///
/// Managed Host and container launches use the capability-authenticated agent
/// listener together with [`GWT_HOOK_FORWARD_TOKEN_ENV`]. Host keeps the
/// listener's loopback URL while containers use their runtime host alias. The
/// endpoint stays separate from [`GWT_HOOK_FORWARD_URL_ENV`] so clients never
/// derive one route from the other.
pub const GWT_PANE_WS_URL_ENV: &str = "GWT_PANE_WS_URL";

/// One agent-tool conversation session observed for a gwt session (a Work, in
/// the Workspace → Work → Session model). Claude Code / Codex can split a
/// single launch into multiple conversation UUIDs (`/clear`, context-limit,
/// resume fork); each distinct UUID is appended here forward-only by
/// [`persist_agent_session_id`] instead of overwriting `agent_session_id`, so
/// the projection can render the full Session list under a Work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSessionHistoryEntry {
    pub agent_session_id: String,
    pub started_at: DateTime<Utc>,
}

/// Durable launch-time binding for a Docker-backed Session.
///
/// The runtime worktree path is the exact container cwd passed to
/// `docker compose exec -w`. The Project State scope hash identifies the
/// canonical host-side `~/.gwt/projects/<scope>/project-state` directory
/// without requiring that host path to be visible inside the container.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DockerRuntimeBinding {
    pub runtime_worktree_path: PathBuf,
    pub project_state_scope_hash: String,
}

/// Non-secret identity of one owner-scoped Execution generation.
///
/// The owner ledger remains authoritative. This value is only a durable
/// Session projection used to reject stale panes and predecessor evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionBindingIdentity {
    pub generation_id: String,
    pub binding_id: String,
    pub ledger_head_hash: String,
}

/// Durable, non-secret projection from a gwt Session to its producing
/// Execution generation.
///
/// Bearer capabilities, provider conversation ids, Host routes, and readiness
/// nonces must remain process-local and are intentionally absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionExecutionBinding {
    pub schema_version: u32,
    pub session_id: String,
    pub repo_hash: String,
    pub owner_kind: String,
    pub owner_number: u64,
    pub identity: ExecutionBindingIdentity,
    pub capability_generation: u64,
}

impl SessionExecutionBinding {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;
}

/// Exact, non-secret identity of one durable producing Session incarnation.
///
/// This allowlist is the single canonical field set used by destructive
/// Session cleanup CAS operations. Mutable runtime state, bearer material,
/// provider conversation ids, process ids, and activity timestamps are
/// intentionally excluded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionExecutionIdentity {
    pub session_id: String,
    pub worktree_path: PathBuf,
    pub project_state_root: Option<PathBuf>,
    pub repo_hash: Option<String>,
    pub branch: String,
    pub agent_id: AgentId,
    pub linked_issue_number: Option<u64>,
    pub execution_binding: SessionExecutionBinding,
}

impl SessionExecutionIdentity {
    /// Capture the canonical identity fields against an explicit binding.
    ///
    /// Launch preparation uses this before the binding is persisted, while
    /// recovery uses [`Self::from_session`] after loading the durable Session.
    pub fn for_binding(
        session: &Session,
        binding: &SessionExecutionBinding,
    ) -> Result<Self, String> {
        validate_session_execution_binding(session, binding)?;
        Ok(Self {
            session_id: session.id.clone(),
            worktree_path: session.worktree_path.clone(),
            project_state_root: session.project_state_root.clone(),
            repo_hash: session.repo_hash.clone(),
            branch: session.branch.clone(),
            agent_id: session.agent_id.clone(),
            linked_issue_number: session.linked_issue_number,
            execution_binding: binding.clone(),
        })
    }

    pub fn from_session(session: &Session) -> Result<Option<Self>, String> {
        session
            .execution_binding
            .as_ref()
            .map(|binding| Self::for_binding(session, binding))
            .transpose()
    }
}

/// Exact classification of one durable Session path without following a
/// directory entry before deciding whether it exists.
///
/// A dangling symlink is [`SessionPathState::Error`], not
/// [`SessionPathState::Missing`]: `symlink_metadata` proves that an entry is
/// present, and any subsequent load failure must retain that evidence.
#[derive(Debug)]
pub enum SessionPathState {
    Missing,
    Present(Box<Session>),
    Error(io::Error),
}

/// Inspect one Session path while preserving present-but-unreadable entries.
#[must_use]
pub fn inspect_session_path(path: &Path) -> SessionPathState {
    match fs::symlink_metadata(path) {
        Ok(_) => match Session::load(path) {
            Ok(session) => SessionPathState::Present(Box::new(session)),
            Err(error) => SessionPathState::Error(error),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => SessionPathState::Missing,
        Err(error) => SessionPathState::Error(error),
    }
}

/// Represents a single agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub worktree_path: PathBuf,
    /// Canonical Project State root for Workspace / Agent projection data.
    ///
    /// `worktree_path` is the process cwd, but gwt-managed worktrees may share
    /// one Workspace Home Project State. Agent title updates must write to that
    /// canonical root so GUI panes and `workspace.update` observe the same
    /// projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_state_root: Option<PathBuf>,
    #[serde(default)]
    pub repo_hash: Option<String>,
    pub branch: String,
    pub agent_id: AgentId,
    pub agent_session_id: Option<String>,
    /// Forward-only history of agent-tool conversation sessions (the Session
    /// level of Workspace → Work → Session). Appended by
    /// [`persist_agent_session_id`] the first time a new `agent_session_id` is
    /// observed. Empty for sessions persisted before this field existed.
    #[serde(default)]
    pub session_history: Vec<AgentSessionHistoryEntry>,
    pub status: AgentStatus,
    pub tool_version: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_level: Option<String>,
    #[serde(default)]
    pub session_mode: SessionMode,
    #[serde(default)]
    pub skip_permissions: bool,
    #[serde(default)]
    pub fast_mode: bool,
    /// Legacy Codex-only compatibility field. Deserialization still accepts
    /// this key so older session TOML restores retain Fast mode intent.
    #[serde(default)]
    pub codex_fast_mode: bool,
    #[serde(default)]
    pub runtime_target: LaunchRuntimeTarget,
    #[serde(default)]
    pub docker_service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_runtime_binding: Option<DockerRuntimeBinding>,
    /// Optional producing authority projection. Legacy and unbound Resume
    /// Sessions keep this absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_binding: Option<SessionExecutionBinding>,
    #[serde(default)]
    pub docker_lifecycle_intent: DockerLifecycleIntent,
    #[serde(default)]
    pub linked_issue_number: Option<u64>,
    #[serde(default)]
    pub workflow_bypass: Option<WorkflowBypass>,
    /// When the bypass was armed. Consumers treat a bypass without a fresh
    /// timestamp as expired so a forgotten disarm cannot outlive its release.
    #[serde(default)]
    pub workflow_bypass_armed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub launch_command: String,
    #[serde(default)]
    pub launch_args: Vec<String>,
    /// GUI window lifecycle flag used by startup restore. Conversation
    /// history alone must not reopen a window after the user closed it.
    #[serde(default)]
    pub restore_window_on_startup: bool,
    /// Active backend override id, if any (SPEC-1921 FR-102).
    /// `None` means the agent launched against its default upstream
    /// (no env override). Set only for built-in agents that support
    /// Backend Override (Claude Code / Codex in the 2026-05-18 amendment).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_id: Option<String>,
    #[serde(default)]
    pub windows_shell: Option<WindowsShellKind>,
    /// Schema version of this persisted session. SPEC-1921 Phase 53 / FR-066:
    /// bumped by `Session::migrate_legacy_launch_args` so migrations are
    /// idempotent. Legacy TOML files without this field deserialize as `0`.
    #[serde(default)]
    pub schema_version: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_hook_event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_hook_event_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_stop_at: Option<DateTime<Utc>>,
    pub display_name: String,
}

/// Lightweight runtime state updated by hook events while the PTY is alive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingDiscussionResume {
    pub proposal_label: String,
    pub proposal_title: String,
    #[serde(default)]
    pub next_question: Option<String>,
}

/// Lightweight runtime state updated by hook events while the PTY is alive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRuntimeState {
    pub status: AgentStatus,
    pub updated_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    /// Producing capability epoch that authored this sidecar. Legacy hook
    /// sidecars omit it; launch-authored sidecars use it to fence stale
    /// relaunch rollback writes from a newer durable Session incarnation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_generation: Option<u64>,
    #[serde(default)]
    pub source_event: Option<String>,
    #[serde(default)]
    pub pending_discussion: Option<PendingDiscussionResume>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CapabilityClaimFailureReceipt {
    schema_version: u32,
    session_id: String,
    execution_binding: SessionExecutionBinding,
    #[serde(default)]
    host_pid: u32,
    #[serde(default)]
    phase: CapabilityClaimReceiptPhase,
    recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum CapabilityClaimReceiptPhase {
    Preparing,
    #[default]
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityClaimReceiptState {
    Absent,
    Preparing { host_pid: u32 },
    Failed,
}

#[cfg(test)]
type CapabilityClaimPostSessionWriteHook = Box<dyn FnOnce() -> io::Result<()> + 'static>;

#[cfg(test)]
thread_local! {
    static CAPABILITY_CLAIM_POST_SESSION_WRITE_HOOK:
        std::cell::RefCell<Option<CapabilityClaimPostSessionWriteHook>> =
        const { std::cell::RefCell::new(None) };
    static ATOMIC_STATE_POST_REPLACE_FAILURE: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    static PAIRED_RUNTIME_BEFORE_SESSION_WRITE_FAILURE: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    static CAPABILITY_RECEIPT_POST_UNLINK_FAILURE: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn set_atomic_state_post_replace_failure_for_test() {
    ATOMIC_STATE_POST_REPLACE_FAILURE.with(|slot| assert!(!slot.replace(true)));
}

#[cfg(test)]
fn fail_atomic_state_post_replace_if_requested() -> io::Result<()> {
    ATOMIC_STATE_POST_REPLACE_FAILURE.with(|slot| {
        if slot.replace(false) {
            Err(io::Error::other("injected post-replace sync failure"))
        } else {
            Ok(())
        }
    })
}

#[cfg(not(test))]
fn fail_atomic_state_post_replace_if_requested() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn set_paired_runtime_before_session_write_failure_for_test() {
    PAIRED_RUNTIME_BEFORE_SESSION_WRITE_FAILURE.with(|slot| assert!(!slot.replace(true)));
}

#[cfg(test)]
fn fail_paired_runtime_before_session_write_if_requested() -> io::Result<()> {
    PAIRED_RUNTIME_BEFORE_SESSION_WRITE_FAILURE.with(|slot| {
        if slot.replace(false) {
            Err(io::Error::other(
                "injected Session write failure after runtime proof",
            ))
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
fn set_capability_receipt_post_unlink_failure_for_test() {
    CAPABILITY_RECEIPT_POST_UNLINK_FAILURE.with(|slot| assert!(!slot.replace(true)));
}

#[cfg(test)]
fn fail_capability_receipt_post_unlink_if_requested() -> io::Result<()> {
    CAPABILITY_RECEIPT_POST_UNLINK_FAILURE.with(|slot| {
        if slot.replace(false) {
            Err(io::Error::other("injected receipt directory sync failure"))
        } else {
            Ok(())
        }
    })
}

#[cfg(not(test))]
fn fail_capability_receipt_post_unlink_if_requested() -> io::Result<()> {
    Ok(())
}

#[cfg(not(test))]
fn fail_paired_runtime_before_session_write_if_requested() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn set_capability_claim_post_session_write_hook_for_test(
    hook: CapabilityClaimPostSessionWriteHook,
) {
    CAPABILITY_CLAIM_POST_SESSION_WRITE_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(test)]
fn invoke_capability_claim_post_session_write_hook() -> io::Result<()> {
    CAPABILITY_CLAIM_POST_SESSION_WRITE_HOOK
        .with(|slot| slot.borrow_mut().take().map_or(Ok(()), |hook| hook()))
}

#[cfg(not(test))]
fn invoke_capability_claim_post_session_write_hook() -> io::Result<()> {
    Ok(())
}

fn capability_claim_failure_path(sessions_dir: &Path, session_id: &str) -> PathBuf {
    sessions_dir
        .join("capability-claim-failures")
        .join(format!("{session_id}.json"))
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic state path must have a parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    let prefix = format!(
        ".{}.tmp-",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state")
    );
    let mut temporary = tempfile::Builder::new()
        .prefix(&prefix)
        .tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    fail_atomic_state_post_replace_if_requested()?;
    sync_parent_dir(parent)
}

fn load_capability_claim_receipt(
    sessions_dir: &Path,
    binding: &SessionExecutionBinding,
) -> io::Result<Option<CapabilityClaimFailureReceipt>> {
    let path = capability_claim_failure_path(sessions_dir, &binding.session_id);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let receipt: CapabilityClaimFailureReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok((receipt.schema_version == 1
        && receipt.session_id == binding.session_id
        && receipt.execution_binding == *binding)
        .then_some(receipt))
}

fn persist_capability_claim_receipt(
    sessions_dir: &Path,
    binding: &SessionExecutionBinding,
    phase: CapabilityClaimReceiptPhase,
) -> io::Result<()> {
    let receipt = CapabilityClaimFailureReceipt {
        schema_version: 1,
        session_id: binding.session_id.clone(),
        execution_binding: binding.clone(),
        host_pid: std::process::id(),
        phase,
        recorded_at: Utc::now(),
    };
    let mut bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    bytes.push(b'\n');
    let path = capability_claim_failure_path(sessions_dir, &binding.session_id);
    match write_atomic_bytes(&path, &bytes) {
        Ok(()) => Ok(()),
        Err(error) => match load_capability_claim_receipt(sessions_dir, binding) {
            Ok(Some(durable)) if durable == receipt => Ok(()),
            Ok(Some(_) | None) | Err(_) => Err(error),
        },
    }
}

fn claimed_capability_failure_error(
    sessions_dir: &Path,
    binding: &SessionExecutionBinding,
    context: &str,
    error: io::Error,
) -> io::Error {
    match persist_capability_claim_receipt(
        sessions_dir,
        binding,
        CapabilityClaimReceiptPhase::Failed,
    ) {
        Ok(()) => io::Error::new(
            error.kind(),
            format!("{context}; durable pre-issuance failure receipt recorded: {error}"),
        ),
        Err(receipt_error) => io::Error::new(
            error.kind(),
            format!(
                "{context}; pre-issuance failure receipt requires reconciliation: {error}; {receipt_error}"
            ),
        ),
    }
}

fn clear_capability_claim_failure(
    sessions_dir: &Path,
    binding: &SessionExecutionBinding,
) -> io::Result<()> {
    let path = capability_claim_failure_path(sessions_dir, &binding.session_id);
    if load_capability_claim_receipt(sessions_dir, binding)?.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "capability claim failure receipt no longer matches the claimed epoch",
        ));
    }
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(_) if !path.exists() => return Ok(()),
        Err(error) => return Err(error),
    }
    fail_capability_receipt_post_unlink_if_requested()?;
    sync_parent_dir(path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "capability claim failure path has no parent directory",
        )
    })?)
}

fn save_runtime_state_with_exact_readback(
    runtime_state: &SessionRuntimeState,
    runtime_path: &Path,
) -> io::Result<()> {
    match runtime_state.save(runtime_path) {
        Ok(()) => Ok(()),
        Err(error) => match SessionRuntimeState::load(runtime_path) {
            Ok(durable) if durable == *runtime_state => Ok(()),
            Ok(_) | Err(_) => Err(error),
        },
    }
}

pub fn capability_claim_receipt_state(
    sessions_dir: &Path,
    binding: &SessionExecutionBinding,
) -> io::Result<CapabilityClaimReceiptState> {
    validate_session_id_path_component(&binding.session_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    Ok(
        match load_capability_claim_receipt(sessions_dir, binding)? {
            None => CapabilityClaimReceiptState::Absent,
            Some(receipt) => match receipt.phase {
                CapabilityClaimReceiptPhase::Preparing => CapabilityClaimReceiptState::Preparing {
                    host_pid: receipt.host_pid,
                },
                CapabilityClaimReceiptPhase::Failed => CapabilityClaimReceiptState::Failed,
            },
        },
    )
}

/// Whether a durable receipt proves that one exact claimed epoch failed
/// before its capability operation ran. The exact durable binding is the
/// authority; the Session may still be Running when failure occurred directly
/// after its claim commit.
pub fn capability_claim_failure_matches(
    sessions_dir: &Path,
    binding: &SessionExecutionBinding,
) -> io::Result<bool> {
    Ok(matches!(
        capability_claim_receipt_state(sessions_dir, binding)?,
        CapabilityClaimReceiptState::Failed
    ))
}

impl Session {
    /// Current persisted session schema version. SPEC-1921 Phase 53 / FR-066.
    /// Bump when adding a new migration in `migrate_legacy_launch_args` and
    /// ensure the new migration is idempotent relative to this value.
    pub const CURRENT_SCHEMA_VERSION: u32 = 3;

    /// Create a new session with a generated UUID.
    pub fn new(
        worktree_path: impl Into<PathBuf>,
        branch: impl Into<String>,
        agent_id: AgentId,
    ) -> Self {
        let worktree_path = worktree_path.into();
        let now = Utc::now();
        let display_name = agent_id.display_name().to_string();
        let repo_hash = gwt_core::repo_hash::detect_repo_hash(&worktree_path)
            .map(|hash| hash.as_str().to_string());
        Self {
            id: Uuid::new_v4().to_string(),
            worktree_path,
            project_state_root: None,
            repo_hash,
            branch: branch.into(),
            agent_id,
            agent_session_id: None,
            session_history: Vec::new(),
            status: AgentStatus::Unknown,
            tool_version: None,
            model: None,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            skip_permissions: false,
            fast_mode: false,
            codex_fast_mode: false,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_runtime_binding: None,
            execution_binding: None,
            docker_lifecycle_intent: DockerLifecycleIntent::Connect,
            linked_issue_number: None,
            workflow_bypass: None,
            workflow_bypass_armed_at: None,
            launch_command: String::new(),
            launch_args: Vec::new(),
            restore_window_on_startup: false,
            backend_id: None,
            windows_shell: None,
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            created_at: now,
            updated_at: now,
            last_activity_at: now,
            last_hook_event: None,
            last_hook_event_at: None,
            last_completed_stop_at: None,
            display_name,
        }
    }

    /// Create a persisted session snapshot from a prepared launch config.
    ///
    /// The launch command/args are captured from the prepared Host command.
    /// Docker still persists the logical agent command before `compose exec`
    /// is applied.
    pub fn from_launch_config(
        worktree_path: impl Into<PathBuf>,
        branch: impl Into<String>,
        config: &LaunchConfig,
    ) -> Self {
        let mut session = Self::new(worktree_path, branch, config.agent_id.clone());
        session.display_name = config.display_name.clone();
        session.tool_version = config.tool_version.clone();
        session.model = config.model.clone();
        session.reasoning_level = config.reasoning_level.clone();
        session.session_mode = config.session_mode;
        session.skip_permissions = config.skip_permissions;
        session.fast_mode = config.fast_mode;
        session.codex_fast_mode = config.codex_fast_mode;
        session.runtime_target = config.runtime_target;
        session.docker_service = config.docker_service.clone();
        session.docker_lifecycle_intent = config.docker_lifecycle_intent;
        session.linked_issue_number = config.linked_issue_number;
        session.launch_command = config.command.clone();
        session.launch_args = config.args.clone();
        session.windows_shell = config.windows_shell;
        session.update_status(AgentStatus::Running);
        session
    }

    /// Bind this Docker Session to the exact runtime cwd used by
    /// `docker compose exec -w` and to the canonical host Project State
    /// scope selected at launch.
    pub fn bind_docker_runtime(
        &mut self,
        runtime_worktree_path: impl Into<PathBuf>,
        project_state_root: &Path,
    ) -> Result<(), String> {
        let runtime_worktree_path = runtime_worktree_path.into();
        validate_docker_runtime_worktree_path(&runtime_worktree_path)?;
        self.docker_runtime_binding = Some(DockerRuntimeBinding {
            runtime_worktree_path,
            project_state_scope_hash: gwt_core::paths::project_scope_hash(project_state_root)
                .as_str()
                .to_string(),
        });
        Ok(())
    }

    /// Set or clear the Session's non-secret Execution generation projection.
    ///
    /// Validation binds every identity component back to the durable Session
    /// before persistence. The owner ledger, not this projection, authorizes
    /// mutations.
    pub fn set_execution_binding(
        &mut self,
        binding: Option<SessionExecutionBinding>,
    ) -> Result<(), String> {
        if let Some(binding) = binding.as_ref() {
            validate_session_execution_binding(self, binding)?;
        }
        if self.execution_binding == binding {
            return Ok(());
        }
        if let (Some(current), Some(next)) = (self.execution_binding.as_ref(), binding.as_ref()) {
            if next.capability_generation < current.capability_generation {
                return Err(format!(
                    "execution binding capability generation downgrade is forbidden: current {}, requested {}",
                    current.capability_generation, next.capability_generation
                ));
            }
            if next.capability_generation == current.capability_generation {
                return Err(
                    "changed execution authority must advance capability generation".to_string(),
                );
            }
        }
        self.execution_binding = binding;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Update the session status and touch timestamps.
    pub fn update_status(&mut self, status: AgentStatus) {
        self.status = status;
        let now = Utc::now();
        self.updated_at = now;
        if matches!(
            status,
            AgentStatus::Running | AgentStatus::Idle | AgentStatus::WaitingInput
        ) {
            self.last_activity_at = now;
        }
    }

    /// Persist that a managed runtime hook was observed for this session.
    pub fn record_hook_event(&mut self, event: &str) {
        let now = Utc::now();
        self.last_hook_event = Some(event.to_string());
        self.last_hook_event_at = Some(now);
        self.updated_at = now;
        if let Some(status) = hook_event_status(event) {
            self.update_status(status);
        }
    }

    /// Persist that the latest Stop hook was allowed to complete.
    pub fn record_completed_stop(&mut self) {
        let now = Utc::now();
        self.last_completed_stop_at = Some(now);
        self.updated_at = now;
        if self.last_hook_event.as_deref() != Some("Stop") {
            self.last_hook_event = Some("Stop".to_string());
            self.last_hook_event_at = Some(now);
        }
        self.update_status(AgentStatus::Idle);
    }

    /// Whether the latest hook lifecycle indicates the session did not reach a
    /// completed Stop boundary.
    pub fn should_mark_interrupted_from_lifecycle(&self) -> bool {
        if self.status == AgentStatus::Stopped {
            return false;
        }
        let Some(last_hook_event_at) = self.last_hook_event_at else {
            return false;
        };
        if self.last_hook_event.as_deref() != Some("Stop") {
            return true;
        }
        self.last_completed_stop_at
            .is_none_or(|completed_at| completed_at < last_hook_event_at)
    }

    pub fn interrupted_recovery_candidate(&self) -> bool {
        self.status == AgentStatus::Interrupted && self.worktree_path.exists()
    }

    pub fn exact_auto_resume_candidate(&self) -> bool {
        matches!(
            self.status,
            AgentStatus::Running
                | AgentStatus::Idle
                | AgentStatus::WaitingInput
                | AgentStatus::Interrupted
        ) && self.has_lifecycle_recovery_evidence()
            && self.worktree_path.exists()
            && self.has_exact_resume_session_id()
    }

    fn has_lifecycle_recovery_evidence(&self) -> bool {
        self.last_hook_event_at.is_some() || self.last_completed_stop_at.is_some()
    }

    pub fn exact_resume_session_id(&self) -> Option<&str> {
        self.agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|value| {
                !(matches!(self.agent_id, AgentId::Codex) && *value == CODEX_PLACEHOLDER_SESSION_ID)
            })
    }

    fn has_exact_resume_session_id(&self) -> bool {
        self.exact_resume_session_id().is_some()
    }

    /// True when `id` is a conversation handle gwt can hand the agent CLI as a
    /// `--resume` target — non-empty and not the Codex placeholder. Used to gate
    /// per-Session Resume: a Session row whose conversation is not resumable
    /// shows no Resume control (history-only) instead of a button that silently
    /// fails. Generic per-Session inspection only rejects structurally unusable
    /// ids and leaves provider validation to the CLI. Producing Continue work
    /// performs its separate, read-only provider-store preflight before it
    /// prepares a successor generation, so a missing or foreign conversation
    /// can fall back without leaving a partial generation.
    pub fn is_resumable_conversation(&self, id: &str) -> bool {
        let id = id.trim();
        !(id.is_empty()
            || (matches!(self.agent_id, AgentId::Codex) && id == CODEX_PLACEHOLDER_SESSION_ID))
    }

    /// Resolve the agent-side resume handle for a Workspace → Work → Session
    /// resume. When `requested` names a specific Session (a conversation UUID
    /// from [`Session::session_history`]) that conversation is resumed;
    /// otherwise it falls back to the latest captured handle
    /// ([`Session::exact_resume_session_id`], the plain Work resume). Blank or
    /// Codex-placeholder requests are ignored so they fall back to the latest
    /// handle instead of trying to resume an unusable id.
    pub fn resume_session_id_for(&self, requested: Option<&str>) -> Option<String> {
        if let Some(requested) = requested.filter(|value| self.is_resumable_conversation(value)) {
            return Some(requested.trim().to_string());
        }
        self.exact_resume_session_id().map(str::to_string)
    }

    /// Check if the session should be marked as stopped due to idle timeout.
    pub fn should_mark_stopped(&self) -> bool {
        if self.status == AgentStatus::Stopped {
            return false;
        }
        let elapsed = Utc::now()
            .signed_duration_since(self.last_activity_at)
            .num_seconds();
        elapsed >= IDLE_TIMEOUT_SECS
    }

    /// Save the session to a TOML file under the given directory.
    /// File is written to `<dir>/<session_id>.toml`.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        validate_session_id_path_component(&self.id)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let content = serialize_session_toml(self)?;
        with_session_lock(dir, &self.id, || {
            write_session_toml_atomic(&session_file_path(dir, &self.id), &content)
        })
    }

    /// Create this Session only while its durable path is genuinely absent.
    ///
    /// A present or unreadable same-id entry is never replaced. Prepared
    /// execution launches use this as their materialization commit point.
    pub fn save_if_absent(&self, dir: &Path) -> std::io::Result<bool> {
        validate_session_id_path_component(&self.id)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let content = serialize_session_toml(self)?;
        with_session_lock(dir, &self.id, || {
            let path = session_file_path(dir, &self.id);
            match inspect_session_path(&path) {
                SessionPathState::Missing => {
                    write_session_toml_atomic(&path, &content)?;
                    Ok(true)
                }
                SessionPathState::Present(_) => Ok(false),
                SessionPathState::Error(error) => Err(error),
            }
        })
    }

    /// Persist mutable Session fields only while the durable producing
    /// incarnation still has the exact expected non-secret identity.
    ///
    /// Missing, unbound, invalid, or replaced entries fail closed without
    /// recreating or rewriting the path.
    pub fn save_if_execution_identity_matches(
        &self,
        dir: &Path,
        expected: &SessionExecutionIdentity,
    ) -> std::io::Result<bool> {
        validate_session_id_path_component(&self.id)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        if self.id != expected.session_id
            || expected.execution_binding.session_id != expected.session_id
            || SessionExecutionIdentity::from_session(self)
                .ok()
                .flatten()
                .as_ref()
                != Some(expected)
        {
            return Ok(false);
        }
        let content = serialize_session_toml(self)?;
        with_session_lock(dir, &self.id, || {
            let path = session_file_path(dir, &self.id);
            let durable = match inspect_session_path(&path) {
                SessionPathState::Present(session) => session,
                SessionPathState::Missing => return Ok(false),
                SessionPathState::Error(error) => return Err(error),
            };
            if SessionExecutionIdentity::from_session(&durable)
                .ok()
                .flatten()
                .as_ref()
                != Some(expected)
            {
                return Ok(false);
            }
            write_session_toml_atomic(&path, &content)?;
            Ok(true)
        })
    }

    /// Persist this Session only when the durable value still equals the
    /// caller's complete pre-mutation snapshot.
    ///
    /// This is the migration CAS for legacy unbound Sessions, which cannot
    /// yet carry a [`SessionExecutionIdentity`]. Comparing every canonical
    /// field prevents a stale clone from overwriting concurrent runtime or
    /// ownership updates while adding the first execution binding.
    pub fn save_if_unchanged(&self, dir: &Path, expected: &Session) -> std::io::Result<bool> {
        validate_session_id_path_component(&self.id)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        if self.id != expected.id {
            return Ok(false);
        }
        let content = serialize_session_toml(self)?;
        let expected_content = serialize_session_toml(expected)?;
        with_session_lock(dir, &self.id, || {
            let path = session_file_path(dir, &self.id);
            let durable = match inspect_session_path(&path) {
                SessionPathState::Present(session) => session,
                SessionPathState::Missing => return Ok(false),
                SessionPathState::Error(error) => return Err(error),
            };
            if serialize_session_toml(&durable)? != expected_content {
                return Ok(false);
            }
            write_session_toml_atomic(&path, &content)?;
            Ok(true)
        })
    }

    /// Create this Session when absent, or replace one exact pre-mutation
    /// snapshot. Any same-id value with different durable contents is
    /// retained byte-identically.
    pub fn save_if_absent_or_unchanged(
        &self,
        dir: &Path,
        expected: &Session,
    ) -> std::io::Result<bool> {
        validate_session_id_path_component(&self.id)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        if self.id != expected.id {
            return Ok(false);
        }
        let content = serialize_session_toml(self)?;
        let expected_content = serialize_session_toml(expected)?;
        with_session_lock(dir, &self.id, || {
            let path = session_file_path(dir, &self.id);
            match inspect_session_path(&path) {
                SessionPathState::Missing => {}
                SessionPathState::Present(session)
                    if serialize_session_toml(&session)? == expected_content => {}
                SessionPathState::Present(_) => return Ok(false),
                SessionPathState::Error(error) => return Err(error),
            }
            write_session_toml_atomic(&path, &content)?;
            Ok(true)
        })
    }

    /// Deserialize a session from a TOML file verbatim. SPEC-1921 FR-066:
    /// `load` must not silently rewrite `launch_args`. Callers that need
    /// legacy migration applied should use [`Session::load_and_migrate`].
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut session: Self = toml::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        session.normalize_fast_mode_fields();
        Ok(session)
    }

    /// Load a session and apply any pending legacy migrations. Production
    /// call sites (runtime hooks, daemon, wizard Quick Start, board view)
    /// should prefer this over [`Session::load`] so legacy TOML files get
    /// their default `launch_args` filled in. SPEC-1921 FR-066.
    pub fn load_and_migrate(path: &Path) -> std::io::Result<Self> {
        let mut session = Self::load(path)?;
        session.migrate_legacy_launch_args();
        Ok(session)
    }

    /// Idempotent migration helper for pre-Phase-53 session TOML files.
    /// Walks the `schema_version` forward to
    /// [`Session::CURRENT_SCHEMA_VERSION`], injecting any missing canonical
    /// launch args (such as Codex's `--no-alt-screen`) along the way.
    pub fn migrate_legacy_launch_args(&mut self) {
        if self.schema_version < 1 {
            // Schema 0 -> 1: apply canonical default args at the correct
            // runner prefix position so legacy sessions written before
            // SPEC-1921 FR-064 pick up agent-neutral defaults (Issue #2091).
            normalize_launch_args(&self.agent_id, &self.launch_command, &mut self.launch_args);
            self.schema_version = 1;
        }

        if self.schema_version < 2 {
            scrub_legacy_codex_hooks_enablement(&self.agent_id, &mut self.launch_args);
            self.schema_version = 2;
        }

        if self.schema_version < 3 {
            if self.worktree_path.exists() {
                self.status = AgentStatus::Interrupted;
            }
            self.schema_version = 3;
        }
    }

    fn normalize_fast_mode_fields(&mut self) {
        if self.codex_fast_mode {
            self.fast_mode = true;
        }
    }

    pub fn fast_mode_enabled(&self) -> bool {
        self.fast_mode || self.codex_fast_mode
    }
}

fn validate_session_execution_binding(
    session: &Session,
    binding: &SessionExecutionBinding,
) -> Result<(), String> {
    if binding.schema_version != SessionExecutionBinding::CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported execution binding schema version: {}",
            binding.schema_version
        ));
    }
    if binding.session_id != session.id {
        return Err("execution binding session does not match the durable Session".to_string());
    }
    if session.repo_hash.as_deref() != Some(binding.repo_hash.as_str()) {
        return Err(
            "execution binding repository does not match the durable Session repository"
                .to_string(),
        );
    }
    if !matches!(binding.owner_kind.as_str(), "spec" | "issue") {
        return Err("execution binding owner kind must be `spec` or `issue`".to_string());
    }
    if session.linked_issue_number != Some(binding.owner_number) {
        return Err("execution binding owner does not match the linked Session owner".to_string());
    }
    if binding.identity.generation_id.trim().is_empty() {
        return Err("execution binding generation id must be non-empty".to_string());
    }
    if binding.identity.binding_id.trim().is_empty() {
        return Err("execution binding id must be non-empty".to_string());
    }
    if binding.identity.ledger_head_hash.trim().is_empty() {
        return Err("execution binding ledger head hash must be non-empty".to_string());
    }
    if binding.capability_generation == 0 {
        return Err("execution binding capability generation must be positive".to_string());
    }
    Ok(())
}

/// Validate the exact container cwd stored for a Docker-backed Session.
///
/// Container paths are POSIX paths on every host platform, including Windows.
/// Validate their string form instead of relying on host-native `Path`
/// semantics.
pub fn validate_docker_runtime_worktree_path(path: &Path) -> Result<(), String> {
    let Some(path) = path.to_str() else {
        return Err("Docker runtime worktree must be an absolute POSIX path".to_string());
    };
    let contains_unsafe_component = path
        .split('/')
        .skip(1)
        .any(|component| component.is_empty() || matches!(component, "." | ".."));
    if !path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || contains_unsafe_component
    {
        return Err(format!(
            "Docker runtime worktree must be an absolute POSIX path without traversal components: {path:?}"
        ));
    }
    Ok(())
}

/// Validate a Session id before using it as one filesystem path component.
///
/// Existing opaque ids remain valid; this only rejects values that are empty,
/// special directory entries, separators, NUL-containing, or absolute/drive
/// paths on either POSIX or Windows.
pub fn validate_session_id_path_component(session_id: &str) -> Result<(), String> {
    let bytes = session_id.as_bytes();
    let is_windows_drive_path =
        bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if session_id.is_empty()
        || matches!(session_id, "." | "..")
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id.contains('\0')
        || is_windows_drive_path
    {
        return Err(format!(
            "Session id must be a safe path component: {session_id:?}"
        ));
    }
    Ok(())
}

fn session_file_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.toml"))
}

fn session_lock_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!(".{session_id}.lock"))
}

fn serialize_session_toml(session: &Session) -> io::Result<String> {
    toml::to_string_pretty(session)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

fn with_session_lock<T, F>(dir: &Path, session_id: &str, action: F) -> io::Result<T>
where
    F: FnOnce() -> io::Result<T>,
{
    let _thread_guard = SessionLeaseThreadGuard::enter()?;
    fs::create_dir_all(dir)?;
    let lock_path = session_lock_path(dir, session_id);
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    lock_file.lock_exclusive()?;
    let result = action();
    match lock_file.unlock() {
        Ok(()) => result,
        Err(unlock_error) => match result {
            Ok(_) => Err(unlock_error),
            Err(action_error) => Err(action_error),
        },
    }
}

/// Hold the per-Session lease while classifying its durable path and running
/// one operation without releasing the lease after a `Missing` observation.
///
/// Callers combining this with an owner ledger lease must acquire the owner
/// lease first and must not persist the same Session from `operation`.
pub fn with_session_path_lease<T, F>(
    sessions_dir: &Path,
    session_id: &str,
    operation: F,
) -> io::Result<T>
where
    F: FnOnce(SessionPathState) -> io::Result<T>,
{
    with_session_path_lease_wait(sessions_dir, session_id, SESSION_LEASE_WAIT, operation)
}

/// [`with_session_path_lease`] with an explicit wait bound for retry-aware
/// callers and deterministic contention tests.
pub fn with_session_path_lease_wait<T, F>(
    sessions_dir: &Path,
    session_id: &str,
    wait: Duration,
    operation: F,
) -> io::Result<T>
where
    F: FnOnce(SessionPathState) -> io::Result<T>,
{
    let _thread_guard = SessionLeaseThreadGuard::enter()?;
    validate_session_id_path_component(session_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    fs::create_dir_all(sessions_dir)?;
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(session_lock_path(sessions_dir, session_id))?;
    let deadline = Instant::now() + wait;
    let is_contended = |error: &io::Error| {
        error.kind() == io::ErrorKind::WouldBlock
            || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
    };
    loop {
        match lock_file.try_lock_exclusive() {
            Ok(()) => break,
            Err(error) if is_contended(&error) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "Session lease is held by another gwt operation; retry after it settles",
                    ));
                }
                std::thread::sleep(SESSION_LEASE_POLL.min(deadline.saturating_duration_since(now)));
            }
            Err(error) => return Err(error),
        }
    }
    let result = operation(inspect_session_path(&session_file_path(
        sessions_dir,
        session_id,
    )));
    match lock_file.unlock() {
        Ok(()) => result,
        Err(unlock_error) => match result {
            Ok(_) => Err(unlock_error),
            Err(operation_error) => Err(operation_error),
        },
    }
}

/// Hold the per-Session lease while reading one exact durable Session and
/// running an already-authorized operation.
///
/// This is an exclusive, bounded lease because capability rotation and
/// Session persistence use the same lock file. Callers combining this with an
/// owner ledger lease must acquire the owner lease first and must not persist
/// the same Session from `operation`.
pub fn with_session_lease<T, F>(
    sessions_dir: &Path,
    session_id: &str,
    operation: F,
) -> io::Result<T>
where
    F: FnOnce(&Session) -> io::Result<T>,
{
    with_session_lease_wait(sessions_dir, session_id, SESSION_LEASE_WAIT, operation)
}

/// [`with_session_lease`] with an explicit wait bound for retry-aware callers
/// and deterministic contention tests.
pub fn with_session_lease_wait<T, F>(
    sessions_dir: &Path,
    session_id: &str,
    wait: Duration,
    operation: F,
) -> io::Result<T>
where
    F: FnOnce(&Session) -> io::Result<T>,
{
    with_session_path_lease_wait(sessions_dir, session_id, wait, |state| match state {
        SessionPathState::Present(session) => operation(&session),
        SessionPathState::Missing => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Session not found: {session_id}"),
        )),
        SessionPathState::Error(error) => Err(error),
    })
}

fn write_session_toml_atomic(path: &Path, content: &str) -> io::Result<()> {
    write_session_toml_atomic_with_replace(path, content, |temporary, destination| {
        fs::rename(temporary, destination)
    })
}

fn write_session_toml_atomic_with_replace<F>(
    path: &Path,
    content: &str,
    replace: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "session TOML path must have a parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("session.toml");
    let tmp_path = parent.join(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));

    let write_result = (|| -> io::Result<()> {
        let mut tmp = File::create(&tmp_path)?;
        tmp.write_all(content.as_bytes())?;
        tmp.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }

    if let Err(error) = replace(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }

    sync_parent_dir(parent)
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> io::Result<()> {
    Ok(())
}

/// Load, mutate, and persist one session under a per-session file lock.
///
/// Production read-modify-write paths should use this helper instead of
/// loading a `Session` and later calling [`Session::save`], otherwise
/// concurrent hook/startup updates can still overwrite each other.
pub fn update_session<F>(sessions_dir: &Path, session_id: &str, mutate: F) -> io::Result<Session>
where
    F: FnOnce(&mut Session) -> io::Result<()>,
{
    validate_session_id_path_component(session_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    with_session_lock(sessions_dir, session_id, || {
        let path = session_file_path(sessions_dir, session_id);
        let mut session = Session::load_and_migrate(&path)?;
        mutate(&mut session)?;
        let content = serialize_session_toml(&session)?;
        write_session_toml_atomic(&path, &content)?;
        Ok(session)
    })
}

/// Load and mutate one session under a per-session file lock, persisting it
/// only when migration or mutation changes its normalized value.
///
/// Comparing canonical values before and after mutation keeps comments,
/// formatting, and file identity intact for semantic no-ops while preserving
/// required schema migrations.
pub fn update_session_if_changed<F>(
    sessions_dir: &Path,
    session_id: &str,
    mutate: F,
) -> io::Result<Session>
where
    F: FnOnce(&mut Session) -> io::Result<()>,
{
    validate_session_id_path_component(session_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    with_session_lock(sessions_dir, session_id, || {
        let path = session_file_path(sessions_dir, session_id);
        let mut session = Session::load(&path)?;
        let before = serialize_session_toml(&session)?;
        session.migrate_legacy_launch_args();
        mutate(&mut session)?;
        let after = serialize_session_toml(&session)?;
        if after != before {
            write_session_toml_atomic(&path, &after)?;
        }
        Ok(session)
    })
}

/// Remove one uncommitted Session only when its durable execution binding
/// still matches the exact Prepared candidate selected by the caller.
///
/// Runtime sidecars are deleted before the Session TOML (the commit marker),
/// so an interrupted cleanup remains retryable and can never delete a
/// replacement Session that reused the same public id with different
/// authority.
pub fn remove_session_if_execution_identity_matches(
    sessions_dir: &Path,
    session_id: &str,
    expected: &SessionExecutionIdentity,
) -> io::Result<bool> {
    remove_session_if_execution_identity_matches_with(sessions_dir, session_id, expected, || Ok(()))
}

/// Run the caller's cleanup commit and remove its candidate Session while the
/// same per-session lock protects the exact binding and Agent identity.
///
/// This closes the validator-to-cleanup race: a concurrent [`Session::save`]
/// cannot replace the Session after validation but before the durable cleanup
/// callback and unlink.
pub fn remove_session_if_execution_identity_matches_with<F>(
    sessions_dir: &Path,
    session_id: &str,
    expected: &SessionExecutionIdentity,
    before_remove: F,
) -> io::Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    remove_session_if_execution_identity_matches_or_missing_inner(
        sessions_dir,
        session_id,
        expected,
        false,
        before_remove,
    )
}

/// Commit cleanup when the candidate is either still the exact Session or
/// was never materialized. Any existing non-matching Session fails closed.
pub fn remove_session_if_execution_identity_matches_or_missing_with<F>(
    sessions_dir: &Path,
    session_id: &str,
    expected: &SessionExecutionIdentity,
    before_remove: F,
) -> io::Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    remove_session_if_execution_identity_matches_or_missing_inner(
        sessions_dir,
        session_id,
        expected,
        true,
        before_remove,
    )
}

fn remove_session_if_execution_identity_matches_or_missing_inner<F>(
    sessions_dir: &Path,
    session_id: &str,
    expected: &SessionExecutionIdentity,
    commit_if_missing: bool,
    before_remove: F,
) -> io::Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    validate_session_id_path_component(session_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if expected.session_id != session_id
        || expected.execution_binding.session_id != expected.session_id
    {
        return Ok(false);
    }
    with_session_lock(sessions_dir, session_id, || {
        let path = session_file_path(sessions_dir, session_id);
        let session = match inspect_session_path(&path) {
            SessionPathState::Present(session) => session,
            SessionPathState::Missing => {
                if commit_if_missing {
                    before_remove()?;
                    return Ok(true);
                }
                return Ok(false);
            }
            SessionPathState::Error(error) => return Err(error),
        };
        let actual_identity = match SessionExecutionIdentity::from_session(&session) {
            Ok(Some(identity)) => identity,
            Ok(None) | Err(_) => return Ok(false),
        };
        if &actual_identity != expected {
            return Ok(false);
        }
        before_remove()?;

        let runtime_root = sessions_dir.join("runtime");
        match fs::symlink_metadata(&runtime_root) {
            Ok(_) => {
                for entry in fs::read_dir(&runtime_root)? {
                    let entry = entry?;
                    if !entry.file_type()?.is_dir()
                        || entry
                            .file_name()
                            .to_str()
                            .and_then(|value| value.parse::<u32>().ok())
                            .is_none()
                    {
                        continue;
                    }
                    let runtime_path = entry.path().join(format!("{session_id}.json"));
                    match fs::remove_file(runtime_path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::remove_file(&path)?;
        #[cfg(unix)]
        File::open(
            path.parent()
                .ok_or_else(|| io::Error::other("Session path has no parent directory"))?,
        )?
        .sync_all()?;
        Ok(true)
    })
}

fn scrub_legacy_codex_hooks_enablement(agent_id: &AgentId, args: &mut Vec<String>) {
    if !matches!(agent_id, AgentId::Codex) {
        return;
    }

    let mut cleaned = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        if let Some(next) = args.get(index + 1) {
            if should_strip_codex_hooks_enablement(&args[index], next) {
                index += 2;
                continue;
            }
        }
        cleaned.push(args[index].clone());
        index += 1;
    }

    *args = cleaned;
}

fn should_strip_codex_hooks_enablement(flag: &str, value: &str) -> bool {
    (flag == "--enable" && value == "codex_hooks")
        || (flag == "-c" && normalize_config_override(value) == "features.codex_hooks=true")
}

fn normalize_config_override(value: &str) -> String {
    value.chars().filter(|c| !c.is_whitespace()).collect()
}

impl SessionRuntimeState {
    /// Create a new runtime state snapshot for the given status.
    pub fn new(status: AgentStatus) -> Self {
        let now = Utc::now();
        Self {
            status,
            updated_at: now,
            last_activity_at: now,
            capability_generation: None,
            source_event: None,
            pending_discussion: None,
        }
    }

    /// Create a runtime snapshot fenced to one producing capability epoch.
    pub fn new_for_execution_capability(status: AgentStatus, capability_generation: u64) -> Self {
        Self {
            capability_generation: Some(capability_generation),
            ..Self::new(status)
        }
    }

    /// Create a runtime state snapshot from a supported hook event.
    pub fn from_hook_event(event: &str) -> Option<Self> {
        let status = hook_event_status(event)?;
        Some(Self {
            source_event: Some(event.to_string()),
            ..Self::new(status)
        })
    }

    /// Save the runtime state to a JSON sidecar file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let mut bytes = content.into_bytes();
        bytes.push(b'\n');
        write_atomic_bytes(path, &bytes)
    }

    /// Load the runtime state from a JSON sidecar file.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
}

/// Return the JSON sidecar path for a session runtime state record.
pub fn runtime_state_path(sessions_dir: &Path, session_id: &str) -> PathBuf {
    runtime_state_path_for_pid(sessions_dir, std::process::id(), session_id)
}

/// Return the runtime namespace directory for a specific gwt process id.
pub fn runtime_state_dir_for_pid(sessions_dir: &Path, pid: u32) -> PathBuf {
    sessions_dir.join("runtime").join(pid.to_string())
}

/// Return the JSON sidecar path for a session runtime state record scoped to a
/// specific gwt process id.
pub fn runtime_state_path_for_pid(sessions_dir: &Path, pid: u32, session_id: &str) -> PathBuf {
    runtime_state_dir_for_pid(sessions_dir, pid).join(format!("{session_id}.json"))
}

/// Recover the sessions directory from a runtime sidecar path like
/// `~/.gwt/sessions/runtime/<pid>/<session>.json`.
pub fn sessions_dir_from_runtime_path(runtime_path: &Path) -> Option<PathBuf> {
    runtime_path
        .parent()?
        .parent()?
        .parent()
        .map(std::path::Path::to_path_buf)
}

/// Reset the runtime namespace for the current gwt process.
pub fn reset_runtime_state_dir(sessions_dir: &Path) -> std::io::Result<()> {
    reset_runtime_state_dir_for_pid(sessions_dir, std::process::id())
}

/// Reset the runtime namespace for the provided gwt process id without
/// touching sibling PID namespaces.
pub fn reset_runtime_state_dir_for_pid(sessions_dir: &Path, pid: u32) -> std::io::Result<()> {
    let runtime_dir = runtime_state_dir_for_pid(sessions_dir, pid);
    if runtime_dir.exists() {
        std::fs::remove_dir_all(&runtime_dir)?;
    }
    std::fs::create_dir_all(&runtime_dir)
}

/// Persist a final session status into both the TOML metadata and the runtime
/// sidecar so future renders do not keep stale active states around.
pub fn persist_session_status(
    sessions_dir: &Path,
    session_id: &str,
    status: AgentStatus,
) -> std::io::Result<()> {
    update_session(sessions_dir, session_id, |session| {
        session.update_status(status);
        Ok(())
    })?;
    SessionRuntimeState::new(status).save(&runtime_state_path(sessions_dir, session_id))
}

fn session_capability_generation_matches(
    session: &Session,
    expected_capability_generation: Option<u64>,
) -> bool {
    let durable_capability_generation = session
        .execution_binding
        .as_ref()
        .map(|binding| binding.capability_generation);
    match (
        expected_capability_generation,
        durable_capability_generation,
    ) {
        (Some(expected), Some(durable)) => expected != 0 && expected == durable,
        // Old launchers did not export an epoch. Keep their unbound and first
        // capability generation writable, but never let them mutate a rotated
        // Session whose current generation they cannot prove.
        (None, None | Some(1)) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

fn update_session_if_capability_generation_matches<F>(
    sessions_dir: &Path,
    session_id: &str,
    expected_capability_generation: Option<u64>,
    mutate: F,
) -> io::Result<bool>
where
    F: FnOnce(&mut Session) -> io::Result<()>,
{
    validate_session_id_path_component(session_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    with_session_lock(sessions_dir, session_id, || {
        let path = session_file_path(sessions_dir, session_id);
        let mut session = Session::load_and_migrate(&path)?;
        if !session_capability_generation_matches(&session, expected_capability_generation) {
            return Ok(false);
        }
        mutate(&mut session)?;
        write_session_toml_atomic(&path, &serialize_session_toml(&session)?)?;
        Ok(true)
    })
}

fn persist_runtime_state_if_capability_generation_matches<F>(
    sessions_dir: &Path,
    session_id: &str,
    expected_capability_generation: Option<u64>,
    runtime_path: &Path,
    mut runtime_state: SessionRuntimeState,
    mutate: F,
) -> io::Result<bool>
where
    F: FnOnce(&mut Session) -> io::Result<()>,
{
    validate_session_id_path_component(session_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let expected_file_name = format!("{session_id}.json");
    if sessions_dir_from_runtime_path(runtime_path).as_deref() != Some(sessions_dir)
        || runtime_path.file_name().and_then(|name| name.to_str())
            != Some(expected_file_name.as_str())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime state path does not match its Session namespace",
        ));
    }
    runtime_state.capability_generation = expected_capability_generation;
    with_session_lock(sessions_dir, session_id, || {
        let path = session_file_path(sessions_dir, session_id);
        let mut session = Session::load_and_migrate(&path)?;
        if !session_capability_generation_matches(&session, expected_capability_generation) {
            return Ok(false);
        }
        // Publish the epoch-stamped sidecar before the Session commit marker.
        // If atomic replacement reached disk but parent fsync reported an
        // error, exact readback proves the intended state is nevertheless
        // durable enough to continue. A genuinely failed sidecar leaves the
        // Session byte-identical.
        save_runtime_state_with_exact_readback(&runtime_state, runtime_path)?;
        mutate(&mut session)?;
        fail_paired_runtime_before_session_write_if_requested()?;
        write_session_toml_atomic(&path, &serialize_session_toml(&session)?)?;
        Ok(true)
    })
}

/// Persist a Host terminal status only when the immutable launch epoch still
/// identifies the durable Session. The sidecar carries the same epoch so
/// liveness recovery can distinguish a current stop from a late old window.
pub fn persist_session_status_if_capability_generation_matches(
    sessions_dir: &Path,
    session_id: &str,
    expected_capability_generation: Option<u64>,
    status: AgentStatus,
) -> io::Result<bool> {
    persist_session_status_at_runtime_path_if_capability_generation_matches(
        sessions_dir,
        session_id,
        expected_capability_generation,
        &runtime_state_path(sessions_dir, session_id),
        status,
        None,
        None,
    )
}

/// Variant for managed hook subprocesses whose immutable runtime path points
/// at the owning Host PID namespace rather than the hook process PID.
pub fn persist_session_status_at_runtime_path_if_capability_generation_matches(
    sessions_dir: &Path,
    session_id: &str,
    expected_capability_generation: Option<u64>,
    runtime_path: &Path,
    status: AgentStatus,
    source_event: Option<&str>,
    pending_discussion: Option<PendingDiscussionResume>,
) -> io::Result<bool> {
    let mut runtime_state = SessionRuntimeState::new(status);
    runtime_state.source_event = source_event.map(str::to_string);
    runtime_state.pending_discussion = pending_discussion;
    persist_runtime_state_if_capability_generation_matches(
        sessions_dir,
        session_id,
        expected_capability_generation,
        runtime_path,
        runtime_state,
        |session| {
            session.update_status(status);
            Ok(())
        },
    )
}

/// Persist one managed hook lifecycle event and its runtime sidecar under the
/// same expected-epoch Session lease.
pub fn persist_session_hook_state_if_capability_generation_matches(
    sessions_dir: &Path,
    session_id: &str,
    expected_capability_generation: Option<u64>,
    runtime_path: &Path,
    runtime_state: SessionRuntimeState,
    event: &str,
    agent_session_id: Option<&str>,
) -> io::Result<bool> {
    persist_runtime_state_if_capability_generation_matches(
        sessions_dir,
        session_id,
        expected_capability_generation,
        runtime_path,
        runtime_state,
        |session| {
            if let Some(agent_session_id) = agent_session_id {
                persist_agent_session_id_in_memory(session, agent_session_id);
            }
            session.record_hook_event(event);
            Ok(())
        },
    )
}

/// Record the clean Stop boundary only for the process that owns the durable
/// capability epoch.
pub fn persist_session_completed_stop_if_capability_generation_matches(
    sessions_dir: &Path,
    session_id: &str,
    expected_capability_generation: Option<u64>,
) -> io::Result<bool> {
    update_session_if_capability_generation_matches(
        sessions_dir,
        session_id,
        expected_capability_generation,
        |session| {
            session.record_completed_stop();
            Ok(())
        },
    )
}

pub fn persist_session_hook_event(
    sessions_dir: &Path,
    session_id: &str,
    event: &str,
) -> std::io::Result<()> {
    update_session(sessions_dir, session_id, |session| {
        session.record_hook_event(event);
        Ok(())
    })
    .map(|_| ())
}

pub fn persist_session_completed_stop(
    sessions_dir: &Path,
    session_id: &str,
) -> std::io::Result<()> {
    update_session(sessions_dir, session_id, |session| {
        session.record_completed_stop();
        Ok(())
    })
    .map(|_| ())
}

/// Persist the backing agent session id into the session TOML so quick-start
/// flows can resume a concrete prior conversation instead of falling back to
/// a tool-global "last session" lookup.
pub fn persist_agent_session_id(
    sessions_dir: &Path,
    session_id: &str,
    agent_session_id: &str,
) -> std::io::Result<()> {
    if agent_session_id.trim().is_empty() {
        return Ok(());
    }

    update_session(sessions_dir, session_id, |session| {
        persist_agent_session_id_in_memory(session, agent_session_id);
        Ok(())
    })
    .map(|_| ())
}

fn persist_agent_session_id_in_memory(session: &mut Session, agent_session_id: &str) {
    let agent_session_id = agent_session_id.trim();
    if agent_session_id.is_empty() || session.agent_session_id.as_deref() == Some(agent_session_id)
    {
        return;
    }
    // Forward-only Session history: record each distinct conversation UUID the
    // first time we see it, before promoting it to the latest. Splits already
    // arrive via the SessionStart hook, so appending here is enough to
    // reconstruct the full Session list under a Work.
    if !session
        .session_history
        .iter()
        .any(|entry| entry.agent_session_id == agent_session_id)
    {
        session.session_history.push(AgentSessionHistoryEntry {
            agent_session_id: agent_session_id.to_string(),
            started_at: Utc::now(),
        });
    }
    session.agent_session_id = Some(agent_session_id.to_string());
}

/// Persist or clear a Session's Execution generation projection under the
/// same per-Session lock used by hook and conversation metadata updates.
pub fn persist_session_execution_binding(
    sessions_dir: &Path,
    session_id: &str,
    binding: Option<SessionExecutionBinding>,
) -> std::io::Result<()> {
    update_session(sessions_dir, session_id, move |session| {
        session
            .set_execution_binding(binding)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
    })
    .map(|_| ())
}

/// Atomically advance the process-local Host capability epoch projected into
/// one producing Session.
///
/// Every Host issuance gets a distinct durable epoch under the per-Session
/// lock. Unbound/legacy Sessions have no producing binding and therefore
/// cannot be promoted implicitly by capability issuance.
pub fn rotate_session_execution_capability(
    sessions_dir: &Path,
    session_id: &str,
) -> std::io::Result<SessionExecutionBinding> {
    let updated = update_session(sessions_dir, session_id, |session| {
        let mut binding = session.execution_binding.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Session has no producing execution binding",
            )
        })?;
        binding.capability_generation =
            binding
                .capability_generation
                .checked_add(1)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "execution binding capability generation overflow",
                    )
                })?;
        session
            .set_execution_binding(Some(binding))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
    })?;
    updated.execution_binding.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "capability rotation lost the producing execution binding",
        )
    })
}

/// Claim one exact Running Session capability epoch, publish the matching
/// runtime sidecar, and run an already-authorized operation before releasing
/// the per-Session lease.
///
/// The expected binding is a single-use compare-and-swap token: concurrent
/// relaunches that present the same epoch have exactly one winner. Callers
/// combining this with generation authority must acquire the owner lease
/// before entering this function.
pub fn claim_running_session_execution_capability_if_unchanged_with<T>(
    sessions_dir: &Path,
    expected: &SessionExecutionBinding,
    runtime_path: &Path,
    operation: impl FnOnce(&SessionExecutionBinding) -> T,
) -> io::Result<Option<(SessionExecutionBinding, T)>> {
    validate_session_id_path_component(&expected.session_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if runtime_path != runtime_state_path(sessions_dir, &expected.session_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capability claim runtime path does not match its Session namespace",
        ));
    }
    with_session_path_lease(sessions_dir, &expected.session_id, |state| {
        let mut session = match state {
            SessionPathState::Present(session) => session,
            SessionPathState::Missing => return Ok(None),
            SessionPathState::Error(error) => return Err(error),
        };
        if session.status != AgentStatus::Running
            || session.execution_binding.as_ref() != Some(expected)
        {
            return Ok(None);
        }
        if !matches!(
            capability_claim_receipt_state(sessions_dir, expected)?,
            CapabilityClaimReceiptState::Absent
        ) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the current capability epoch has an unreconciled pre-issuance receipt",
            ));
        }
        let mut claimed = expected.clone();
        claimed.capability_generation =
            claimed
                .capability_generation
                .checked_add(1)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "execution binding capability generation overflow",
                    )
                })?;
        // Write-ahead proof closes the crash window between exposing the
        // claimed epoch and publishing its runtime capability. It is removed
        // only after both durable records agree and immediately before the
        // already-authorized operation runs.
        persist_capability_claim_receipt(
            sessions_dir,
            &claimed,
            CapabilityClaimReceiptPhase::Preparing,
        )?;
        session
            .set_execution_binding(Some(claimed.clone()))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let session_path = session_file_path(sessions_dir, &session.id);
        let claimed_content = serialize_session_toml(&session)?;
        if let Err(claim_error) = write_session_toml_atomic(&session_path, &claimed_content) {
            let exact_claim_visible = matches!(
                inspect_session_path(&session_path),
                SessionPathState::Present(durable)
                    if serialize_session_toml(&durable)? == claimed_content
            );
            if exact_claim_visible {
                return Err(claimed_capability_failure_error(
                    sessions_dir,
                    &claimed,
                    "capability claim persistence had an unknown outcome and the exact claimed Session is visible",
                    claim_error,
                ));
            }
            return match clear_capability_claim_failure(sessions_dir, &claimed) {
                Ok(()) => Err(claim_error),
                Err(cleanup_error) => Err(io::Error::new(
                    claim_error.kind(),
                    format!(
                        "capability claim persistence failed and its unmatched write-ahead receipt could not be removed: {claim_error}; {cleanup_error}"
                    ),
                )),
            };
        }
        if let Err(boundary_error) = invoke_capability_claim_post_session_write_hook() {
            return Err(claimed_capability_failure_error(
                sessions_dir,
                &claimed,
                "capability claim stopped after Session persistence",
                boundary_error,
            ));
        }
        let runtime_state = SessionRuntimeState::new_for_execution_capability(
            AgentStatus::Running,
            claimed.capability_generation,
        );
        if let Err(runtime_error) =
            save_runtime_state_with_exact_readback(&runtime_state, runtime_path)
        {
            return Err(claimed_capability_failure_error(
                sessions_dir,
                &claimed,
                "failed to publish claimed capability runtime state",
                runtime_error,
            ));
        }
        if let Err(cleanup_error) = clear_capability_claim_failure(sessions_dir, &claimed) {
            return Err(claimed_capability_failure_error(
                sessions_dir,
                &claimed,
                "claimed capability is durable but its Preparing receipt could not be cleared, so capability issuance was aborted",
                cleanup_error,
            ));
        }
        let result = operation(&claimed);
        Ok(Some((claimed, result)))
    })
}

/// Persist whether the GUI should recreate this session's agent window during
/// startup. This is intentionally separate from agent status/conversation
/// persistence so manual close can opt out without deleting history.
pub fn persist_session_restore_window_on_startup(
    sessions_dir: &Path,
    session_id: &str,
    restore: bool,
) -> std::io::Result<()> {
    update_session(sessions_dir, session_id, |session| {
        if session.restore_window_on_startup == restore {
            return Ok(());
        }
        session.restore_window_on_startup = restore;
        session.updated_at = Utc::now();
        Ok(())
    })
    .map(|_| ())
}

fn hook_event_status(event: &str) -> Option<AgentStatus> {
    match event {
        "SessionStart" | "Stop" => Some(AgentStatus::Idle),
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => Some(AgentStatus::Running),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_has_uuid_id() {
        let session = Session::new("/tmp/wt", "feature/test", AgentId::ClaudeCode);
        assert!(!session.id.is_empty());
        // Verify it's a valid UUID
        assert!(Uuid::parse_str(&session.id).is_ok());
    }

    #[test]
    fn new_session_defaults() {
        let session = Session::new("/tmp/wt", "main", AgentId::Codex);
        assert_eq!(session.status, AgentStatus::Unknown);
        assert_eq!(session.branch, "main");
        assert_eq!(session.agent_id, AgentId::Codex);
        assert_eq!(session.display_name, "Codex");
        assert!(session.agent_session_id.is_none());
        assert!(session.project_state_root.is_none());
        assert!(session.tool_version.is_none());
        assert!(session.model.is_none());
        assert!(session.reasoning_level.is_none());
        assert!(!session.skip_permissions);
        assert!(!session.fast_mode);
        assert!(!session.codex_fast_mode);
        assert_eq!(session.runtime_target, LaunchRuntimeTarget::Host);
        assert!(session.docker_service.is_none());
        assert!(session.docker_runtime_binding.is_none());
        assert_eq!(
            session.docker_lifecycle_intent,
            DockerLifecycleIntent::Connect
        );
        assert!(session.workflow_bypass.is_none());
        assert!(!session.restore_window_on_startup);
        // SPEC-1921 FR-102: new sessions default to no backend override.
        assert!(session.backend_id.is_none());
    }

    #[test]
    fn docker_runtime_binding_roundtrips_without_schema_bump() {
        let session = Session::new("/host/worktree", "work/docker-binding", AgentId::Codex);
        let schema_version = session.schema_version;
        let mut persisted = toml::to_string(&session).expect("serialize Session");
        persisted.push_str(
            "\n[docker_runtime_binding]\nruntime_worktree_path = \"/workspace/repo\"\nproject_state_scope_hash = \"0123456789abcdef\"\n",
        );

        let loaded: Session = toml::from_str(&persisted).expect("deserialize Docker binding");
        let loaded_binding = loaded
            .docker_runtime_binding
            .as_ref()
            .expect("deserialize Docker runtime binding");
        assert_eq!(
            loaded_binding.runtime_worktree_path,
            PathBuf::from("/workspace/repo")
        );
        assert_eq!(loaded_binding.project_state_scope_hash, "0123456789abcdef");
        let roundtrip = toml::to_string(&loaded).expect("reserialize Session");
        let value: toml::Value = toml::from_str(&roundtrip).expect("parse roundtrip TOML");
        let binding = value
            .get("docker_runtime_binding")
            .and_then(toml::Value::as_table)
            .expect("Docker runtime binding must remain persisted");

        assert_eq!(
            binding
                .get("runtime_worktree_path")
                .and_then(toml::Value::as_str),
            Some("/workspace/repo")
        );
        assert_eq!(
            binding
                .get("project_state_scope_hash")
                .and_then(toml::Value::as_str),
            Some("0123456789abcdef")
        );
        assert_eq!(loaded.schema_version, schema_version);
    }

    #[test]
    fn legacy_and_host_sessions_omit_docker_runtime_binding() {
        let legacy = r#"
id = "1d3d2d2d-3333-4444-5555-777777777778"
worktree_path = "/tmp/wt"
branch = "main"
agent_id = { type = "Codex" }
status = "WaitingInput"
created_at = "2026-05-18T00:00:00Z"
updated_at = "2026-05-18T00:00:00Z"
last_activity_at = "2026-05-18T00:00:00Z"
display_name = "Codex"
"#;
        let loaded: Session = toml::from_str(legacy).expect("deserialize legacy Session");
        assert!(loaded.docker_runtime_binding.is_none());
        let roundtrip = toml::to_string(&loaded).expect("serialize legacy Session");
        let host = toml::to_string(&Session::new("/tmp/wt", "main", AgentId::Codex))
            .expect("serialize Host Session");

        assert!(!roundtrip.contains("docker_runtime_binding"));
        assert!(!host.contains("docker_runtime_binding"));
    }

    fn test_session_execution_binding(session: &Session) -> SessionExecutionBinding {
        SessionExecutionBinding {
            schema_version: 1,
            session_id: session.id.clone(),
            repo_hash: session
                .repo_hash
                .clone()
                .expect("test Session must carry a repo hash"),
            owner_kind: "spec".to_string(),
            owner_number: session
                .linked_issue_number
                .expect("test Session must carry an owner"),
            identity: ExecutionBindingIdentity {
                generation_id: "generation-2".to_string(),
                binding_id: "binding-2".to_string(),
                ledger_head_hash: "ledger-head-2".to_string(),
            },
            capability_generation: 2,
        }
    }

    fn session_with_execution_owner() -> Session {
        let mut session = Session::new("/host/worktree", "work/issue-2359", AgentId::Codex);
        session.repo_hash = Some("repo-hash-2359".to_string());
        session.linked_issue_number = Some(2359);
        session
    }

    #[test]
    fn save_if_absent_retains_existing_same_id_session_byte_identically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let candidate = session_with_execution_owner();
        let mut replacement = candidate.clone();
        replacement.agent_id = AgentId::Custom("replacement".to_string());
        replacement.save(dir.path()).expect("save replacement");
        let path = dir.path().join(format!("{}.toml", candidate.id));
        let before = fs::read(&path).expect("read replacement bytes");

        assert!(
            !candidate
                .save_if_absent(dir.path())
                .expect("classify present Session"),
            "a present same-id Session must reject create-if-absent"
        );
        assert_eq!(fs::read(path).expect("read retained replacement"), before);
    }

    #[test]
    fn save_if_execution_identity_matches_updates_exact_and_rejects_replacement() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut candidate = session_with_execution_owner();
        let binding = test_session_execution_binding(&candidate);
        candidate
            .set_execution_binding(Some(binding))
            .expect("bind candidate");
        candidate.save(dir.path()).expect("save exact candidate");
        let identity = SessionExecutionIdentity::from_session(&candidate)
            .expect("derive identity")
            .expect("bound identity");

        candidate.display_name = "Updated display".to_string();
        assert!(candidate
            .save_if_execution_identity_matches(dir.path(), &identity)
            .expect("save exact mutable update"));
        assert_eq!(
            Session::load(&dir.path().join(format!("{}.toml", candidate.id)))
                .expect("reload exact update")
                .display_name,
            "Updated display"
        );

        let mut replacement = candidate.clone();
        replacement.agent_id = AgentId::Custom("replacement".to_string());
        replacement.save(dir.path()).expect("save replacement");
        let path = dir.path().join(format!("{}.toml", candidate.id));
        let before = fs::read(&path).expect("read replacement bytes");
        candidate.display_name = "Must not persist".to_string();

        assert!(!candidate
            .save_if_execution_identity_matches(dir.path(), &identity)
            .expect("reject replacement"));
        assert_eq!(fs::read(path).expect("read retained replacement"), before);
    }

    #[test]
    fn save_if_unchanged_migrates_exact_unbound_session_and_rejects_stale_clone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = session_with_execution_owner();
        original.save(dir.path()).expect("save unbound Session");

        let mut migrated = original.clone();
        migrated
            .set_execution_binding(Some(test_session_execution_binding(&migrated)))
            .expect("bind migrated Session");
        assert!(migrated
            .save_if_unchanged(dir.path(), &original)
            .expect("migrate exact Session"));

        original.save(dir.path()).expect("restore unbound Session");
        let mut replacement = original.clone();
        replacement.display_name = "Concurrent replacement".to_string();
        replacement.save(dir.path()).expect("save replacement");
        let path = dir.path().join(format!("{}.toml", original.id));
        let before = fs::read(&path).expect("read replacement bytes");

        assert!(!migrated
            .save_if_unchanged(dir.path(), &original)
            .expect("reject stale clone"));
        assert_eq!(fs::read(path).expect("read retained replacement"), before);
    }

    #[test]
    fn save_if_absent_or_unchanged_creates_or_migrates_without_replacing_same_id_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = session_with_execution_owner();
        let mut migrated = original.clone();
        migrated
            .set_execution_binding(Some(test_session_execution_binding(&migrated)))
            .expect("bind migrated Session");

        assert!(migrated
            .save_if_absent_or_unchanged(dir.path(), &original)
            .expect("create missing Session"));
        fs::remove_file(dir.path().join(format!("{}.toml", original.id)))
            .expect("remove created Session");
        original
            .save(dir.path())
            .expect("save exact unbound Session");
        assert!(migrated
            .save_if_absent_or_unchanged(dir.path(), &original)
            .expect("migrate exact Session"));

        let mut replacement = original.clone();
        replacement.agent_id = AgentId::Custom("replacement".to_string());
        replacement.save(dir.path()).expect("save replacement");
        let path = dir.path().join(format!("{}.toml", original.id));
        let before = fs::read(&path).expect("read replacement bytes");
        assert!(!migrated
            .save_if_absent_or_unchanged(dir.path(), &original)
            .expect("reject replacement"));
        assert_eq!(fs::read(path).expect("read retained replacement"), before);
    }

    #[test]
    fn session_execution_binding_new_and_legacy_sessions_default_to_none() {
        let session = Session::new("/host/worktree", "work/legacy", AgentId::Codex);
        assert!(session.execution_binding.is_none());

        let legacy = r#"
id = "1d3d2d2d-3333-4444-5555-777777777779"
worktree_path = "/tmp/wt"
branch = "work/legacy"
agent_id = { type = "Codex" }
status = "WaitingInput"
created_at = "2026-07-24T00:00:00Z"
updated_at = "2026-07-24T00:00:00Z"
last_activity_at = "2026-07-24T00:00:00Z"
display_name = "Codex"
"#;
        let loaded: Session = toml::from_str(legacy).expect("deserialize legacy Session");
        assert!(
            loaded.execution_binding.is_none(),
            "legacy and unbound Resume sessions must remain non-producing"
        );
        assert!(
            !toml::to_string(&loaded)
                .expect("serialize legacy Session")
                .contains("execution_binding"),
            "optional projection must not be synthesized during a legacy read"
        );
    }

    #[test]
    fn session_execution_binding_roundtrips_without_schema_bump_and_is_secret_free() {
        let mut session = session_with_execution_owner();
        let session_schema_version = session.schema_version;
        let binding = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind exact Session/owner/repository identity");

        let serialized = toml::to_string_pretty(&session).expect("serialize bound Session");
        let value: toml::Value = toml::from_str(&serialized).expect("parse Session TOML");
        let table = value
            .get("execution_binding")
            .and_then(toml::Value::as_table)
            .expect("execution binding table");
        let mut top_level_keys = table.keys().map(String::as_str).collect::<Vec<_>>();
        top_level_keys.sort_unstable();
        assert_eq!(
            top_level_keys,
            vec![
                "capability_generation",
                "identity",
                "owner_kind",
                "owner_number",
                "repo_hash",
                "schema_version",
                "session_id",
            ],
            "durable binding must contain only the exact non-secret allowlist"
        );
        let identity = table
            .get("identity")
            .and_then(toml::Value::as_table)
            .expect("execution identity table");
        let mut identity_keys = identity.keys().map(String::as_str).collect::<Vec<_>>();
        identity_keys.sort_unstable();
        assert_eq!(
            identity_keys,
            vec!["binding_id", "generation_id", "ledger_head_hash"]
        );
        for forbidden in [
            "bearer",
            "forward_token",
            "conversation",
            "host_url",
            "socket",
            "nonce",
        ] {
            assert!(
                !serialized.to_ascii_lowercase().contains(forbidden),
                "binding projection leaked forbidden field {forbidden}: {serialized}"
            );
        }

        let loaded: Session = toml::from_str(&serialized).expect("roundtrip bound Session");
        assert_eq!(loaded.execution_binding.as_ref(), Some(&binding));
        assert_eq!(loaded.schema_version, session_schema_version);
    }

    #[test]
    fn prepared_session_cleanup_requires_exact_binding_and_removes_runtime_sidecars() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = session_with_execution_owner();
        let binding = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("canonical Session identity")
            .expect("bound Session identity");
        session.save(dir.path()).expect("save bound Session");
        let current_runtime = runtime_state_path(dir.path(), &session.id);
        let foreign_runtime = runtime_state_path_for_pid(dir.path(), 424_242, &session.id);
        SessionRuntimeState::new(AgentStatus::Running)
            .save(&current_runtime)
            .expect("save current runtime");
        SessionRuntimeState::new(AgentStatus::Running)
            .save(&foreign_runtime)
            .expect("save foreign runtime");

        let mut mismatched = identity.clone();
        mismatched.execution_binding.identity.binding_id = "foreign-binding".to_string();
        assert!(
            !remove_session_if_execution_identity_matches(dir.path(), &session.id, &mismatched,)
                .expect("mismatched cleanup"),
            "a stale or foreign cleanup must be a zero-write refusal",
        );
        assert!(dir.path().join(format!("{}.toml", session.id)).exists());
        assert!(current_runtime.exists());
        assert!(foreign_runtime.exists());

        assert!(
            remove_session_if_execution_identity_matches(dir.path(), &session.id, &identity,)
                .expect("exact cleanup"),
            "the exact uncommitted Session must be removed",
        );
        assert!(!dir.path().join(format!("{}.toml", session.id)).exists());
        assert!(!current_runtime.exists());
        assert!(!foreign_runtime.exists());
        assert!(
            !remove_session_if_execution_identity_matches(dir.path(), &session.id, &identity,)
                .expect("idempotent missing cleanup"),
            "repeat cleanup must be an idempotent no-op",
        );
    }

    #[test]
    fn prepared_session_cleanup_retains_same_binding_with_different_agent_identity() {
        for (case_name, expected_agent, replacement_agent) in [
            (
                "codex-to-custom",
                AgentId::Codex,
                AgentId::Custom("review-bot".to_string()),
            ),
            (
                "custom-to-custom",
                AgentId::Custom("review-bot".to_string()),
                AgentId::Custom("release-bot".to_string()),
            ),
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let mut session = session_with_execution_owner();
            session.agent_id = expected_agent.clone();
            let binding = test_session_execution_binding(&session);
            session
                .set_execution_binding(Some(binding.clone()))
                .expect("bind Session");
            let identity = SessionExecutionIdentity::from_session(&session)
                .expect("canonical Session identity")
                .expect("bound Session identity");
            session.save(dir.path()).expect("save bound Session");

            let mut replacement = session.clone();
            replacement.agent_id = replacement_agent;
            replacement
                .save(dir.path())
                .expect("replace Session behind the validator");
            let replacement_path = dir.path().join(format!("{}.toml", session.id));
            let replacement_before = fs::read(&replacement_path).expect("read replacement before");

            assert!(
                !remove_session_if_execution_identity_matches(dir.path(), &session.id, &identity,)
                    .expect("agent-mismatched cleanup"),
                "{case_name}: same binding must not authenticate a different Agent",
            );
            assert_eq!(
                fs::read(&replacement_path).expect("read retained replacement"),
                replacement_before,
                "{case_name}: replacement must be retained byte-identically",
            );

            session
                .save(dir.path())
                .expect("restore exact Agent Session");
            assert!(
                remove_session_if_execution_identity_matches(dir.path(), &session.id, &identity,)
                    .expect("exact Agent cleanup"),
                "{case_name}: matching Agent and binding must be removed",
            );
        }
    }

    #[test]
    fn prepared_session_cleanup_requires_every_stable_non_secret_session_anchor() {
        let mut original = session_with_execution_owner();
        original.project_state_root = Some(PathBuf::from("/host/project-state"));
        let binding = test_session_execution_binding(&original);
        original
            .set_execution_binding(Some(binding.clone()))
            .expect("bind Session");
        let identity = SessionExecutionIdentity::from_session(&original)
            .expect("canonical Session identity")
            .expect("bound Session identity");

        for (case_name, replacement) in [
            ("worktree-path", {
                let mut replacement = original.clone();
                replacement.worktree_path = PathBuf::from("/host/replacement-worktree");
                replacement
            }),
            ("project-state-root", {
                let mut replacement = original.clone();
                replacement.project_state_root = Some(PathBuf::from("/host/foreign-state"));
                replacement
            }),
            ("branch", {
                let mut replacement = original.clone();
                replacement.branch = "work/foreign".to_string();
                replacement
            }),
            ("repository-hash", {
                let mut replacement = original.clone();
                replacement.repo_hash = Some("foreign-repository".to_string());
                replacement
            }),
            ("linked-owner", {
                let mut replacement = original.clone();
                replacement.linked_issue_number = Some(9999);
                replacement
            }),
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            original.save(dir.path()).expect("save exact Session");
            replacement
                .save(dir.path())
                .expect("replace Session behind validator");
            let replacement_path = dir.path().join(format!("{}.toml", original.id));
            let replacement_before = fs::read(&replacement_path).expect("read replacement");

            assert!(
                !remove_session_if_execution_identity_matches(dir.path(), &original.id, &identity,)
                    .expect("anchor-mismatched cleanup"),
                "{case_name}: a stable Session anchor mismatch must fail closed",
            );
            assert_eq!(
                fs::read(&replacement_path).expect("read retained replacement"),
                replacement_before,
                "{case_name}: replacement must remain byte-identical",
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn prepared_session_cleanup_retains_session_when_runtime_root_is_dangling() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = session_with_execution_owner();
        let binding = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("canonical Session identity")
            .expect("bound Session identity");
        session.save(dir.path()).expect("save Session");
        let session_path = dir.path().join(format!("{}.toml", session.id));
        let session_before = fs::read(&session_path).expect("read Session before cleanup");
        symlink(
            dir.path().join("missing-runtime-root"),
            dir.path().join("runtime"),
        )
        .expect("create dangling runtime root");

        assert!(
            remove_session_if_execution_identity_matches(dir.path(), &session.id, &identity,)
                .is_err(),
            "a dangling runtime entry must be an I/O failure, never an empty runtime root",
        );
        assert_eq!(
            fs::read(&session_path).expect("read retained Session"),
            session_before,
        );
    }

    #[cfg(unix)]
    #[test]
    fn prepared_session_cleanup_does_not_commit_for_dangling_main_session_path() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = session_with_execution_owner();
        let binding = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(binding))
            .expect("bind Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("canonical Session identity")
            .expect("bound Session identity");
        session.save(dir.path()).expect("save Session");
        let session_path = dir.path().join(format!("{}.toml", session.id));
        fs::remove_file(&session_path).expect("remove materialized Session");
        let missing_target = dir.path().join("missing-session-target");
        symlink(&missing_target, &session_path).expect("create dangling Session entry");
        let callback_ran = std::cell::Cell::new(false);

        let error = remove_session_if_execution_identity_matches_or_missing_with(
            dir.path(),
            &session.id,
            &identity,
            || {
                callback_ran.set(true);
                Ok(())
            },
        )
        .expect_err("a dangling Session entry must remain an unreadable present candidate");

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(
            !callback_ran.get(),
            "owner/Work cleanup must not commit for an unreadable Session entry",
        );
        assert!(fs::symlink_metadata(&session_path)
            .expect("dangling Session entry must remain")
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_link(&session_path).unwrap(), missing_target);

        fs::remove_file(&session_path).expect("remove dangling Session entry");
        assert!(
            remove_session_if_execution_identity_matches_or_missing_with(
                dir.path(),
                &session.id,
                &identity,
                || {
                    callback_ran.set(true);
                    Ok(())
                },
            )
            .expect("truly missing candidate cleanup"),
            "a genuinely absent Session keeps the materialization-never-happened path",
        );
        assert!(callback_ran.get());
    }

    #[test]
    fn session_path_lease_reports_true_missing_without_releasing_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let session_id = "missing-path-lease";
        let observed =
            with_session_path_lease_wait(dir.path(), session_id, Duration::from_secs(1), |state| {
                Ok(matches!(state, SessionPathState::Missing))
            })
            .expect("inspect a genuinely missing Session under its lease");

        assert!(observed);
    }

    #[test]
    fn prepared_session_cleanup_runs_commit_only_for_exact_identity_and_retains_on_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = session_with_execution_owner();
        let binding = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("canonical Session identity")
            .expect("bound Session identity");
        session.save(dir.path()).expect("save Session");
        let path = dir.path().join(format!("{}.toml", session.id));
        let before = fs::read(&path).expect("read Session before");
        let callback_ran = std::cell::Cell::new(false);

        let result = remove_session_if_execution_identity_matches_with(
            dir.path(),
            &session.id,
            &identity,
            || {
                callback_ran.set(true);
                Err(io::Error::other("simulated cleanup commit failure"))
            },
        );

        assert!(result.is_err());
        assert!(callback_ran.get());
        assert_eq!(fs::read(&path).expect("read retained Session"), before);

        let mut foreign_identity = identity.clone();
        foreign_identity.agent_id = AgentId::Custom("review-bot".to_string());
        callback_ran.set(false);
        assert!(!remove_session_if_execution_identity_matches_with(
            dir.path(),
            &session.id,
            &foreign_identity,
            || {
                callback_ran.set(true);
                Ok(())
            },
        )
        .expect("foreign Agent refusal"));
        assert!(!callback_ran.get());
        assert_eq!(fs::read(&path).expect("read retained Session"), before);

        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = remove_session_if_execution_identity_matches_with(
                dir.path(),
                &session.id,
                &identity,
                || -> io::Result<()> { panic!("simulated cleanup callback panic") },
            );
        }));
        assert!(panic_result.is_err());
        assert_eq!(
            fs::read(&path).expect("read Session retained after callback panic"),
            before,
        );
        assert!(
            remove_session_if_execution_identity_matches(dir.path(), &session.id, &identity,)
                .expect("cleanup after callback panic"),
            "callback unwind must release the Session lock without deleting the candidate",
        );
        callback_ran.set(false);
        assert!(
            remove_session_if_execution_identity_matches_or_missing_with(
                dir.path(),
                &session.id,
                &identity,
                || {
                    callback_ran.set(true);
                    Ok(())
                },
            )
            .expect("commit cleanup for never-materialized candidate"),
        );
        assert!(callback_ran.get());
    }

    #[test]
    fn persist_session_execution_binding_is_atomic_idempotent_and_preserves_history() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut session = session_with_execution_owner();
        session.agent_session_id = Some("conversation-current".to_string());
        session.session_history.push(AgentSessionHistoryEntry {
            agent_session_id: "conversation-current".to_string(),
            started_at: Utc::now(),
        });
        session.record_hook_event("UserPromptSubmit");
        let expected_history = session.session_history.clone();
        let expected_hook = session.last_hook_event.clone();
        let binding = test_session_execution_binding(&session);
        let session_id = session.id.clone();
        session.save(dir.path()).expect("save Session");

        persist_session_execution_binding(dir.path(), &session_id, Some(binding.clone()))
            .expect("persist execution binding");
        let path = dir.path().join(format!("{session_id}.toml"));
        let first = fs::read_to_string(&path).expect("read first binding write");
        persist_session_execution_binding(dir.path(), &session_id, Some(binding.clone()))
            .expect("duplicate binding write is idempotent");
        let second = fs::read_to_string(&path).expect("read duplicate binding write");
        assert_eq!(
            first, second,
            "exact duplicate must not mutate Session TOML"
        );

        let loaded = Session::load(&path).expect("reload bound Session");
        assert_eq!(loaded.execution_binding.as_ref(), Some(&binding));
        assert_eq!(loaded.session_history, expected_history);
        assert_eq!(loaded.last_hook_event, expected_hook);
        assert!(
            fs::read_dir(dir.path())
                .expect("read sessions dir")
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().contains(".tmp-")),
            "atomic persistence must not leave temporary Session files"
        );

        persist_session_execution_binding(dir.path(), &session_id, None)
            .expect("clear producing binding for an unbound Resume");
        assert!(Session::load(&path)
            .expect("reload cleared Session")
            .execution_binding
            .is_none());
    }

    #[test]
    fn failed_atomic_session_replace_preserves_existing_record_and_cleans_temporary_file() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let path = dir.path().join("candidate.toml");
        fs::write(&path, "old durable Session record").expect("seed existing Session record");

        let error = write_session_toml_atomic_with_replace(
            &path,
            "new Session record",
            |temporary, destination| {
                assert!(
                    temporary.exists(),
                    "temporary record must be durable before replace"
                );
                assert_eq!(
                    fs::read_to_string(destination).expect("read existing destination"),
                    "old durable Session record",
                    "replacement must never pre-delete the sole recovery record",
                );
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "simulated atomic replace failure",
                ))
            },
        )
        .expect_err("simulated replace failure must surface");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            fs::read_to_string(&path).expect("read preserved destination"),
            "old durable Session record",
        );
        assert!(
            fs::read_dir(dir.path())
                .expect("read sessions dir")
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().contains(".tmp-")),
            "failed replacement must clean the uncommitted temporary record",
        );
    }

    #[test]
    fn persist_session_execution_binding_rejects_capability_generation_replay_or_downgrade() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let session = session_with_execution_owner();
        let session_id = session.id.clone();
        let current = test_session_execution_binding(&session);
        session.save(dir.path()).expect("save Session");
        persist_session_execution_binding(dir.path(), &session_id, Some(current.clone()))
            .expect("persist current capability generation");
        let path = dir.path().join(format!("{session_id}.toml"));
        let bound_bytes = fs::read_to_string(&path).expect("read current binding");

        let mut replayed_epoch = current.clone();
        replayed_epoch.identity.ledger_head_hash = "different-ledger-head".to_string();
        let replay_error =
            persist_session_execution_binding(dir.path(), &session_id, Some(replayed_epoch))
                .expect_err("changed authority must advance capability generation");
        assert_eq!(replay_error.kind(), io::ErrorKind::InvalidInput);
        assert!(replay_error.to_string().contains("advance"));

        let mut downgraded = current;
        downgraded.capability_generation -= 1;
        let downgrade_error =
            persist_session_execution_binding(dir.path(), &session_id, Some(downgraded))
                .expect_err("capability generation downgrade must fail closed");
        assert_eq!(downgrade_error.kind(), io::ErrorKind::InvalidInput);
        assert!(downgrade_error.to_string().contains("downgrade"));
        assert_eq!(
            fs::read_to_string(&path).expect("read unchanged Session"),
            bound_bytes,
            "epoch replay/downgrade rejection must preserve Session bytes"
        );
    }

    #[test]
    fn rotate_session_execution_capability_is_atomic_monotonic_and_requires_binding() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let session = session_with_execution_owner();
        let session_id = session.id.clone();
        let initial = test_session_execution_binding(&session);
        session.save(dir.path()).expect("save Session");
        persist_session_execution_binding(dir.path(), &session_id, Some(initial.clone()))
            .expect("persist initial binding");

        let first = rotate_session_execution_capability(dir.path(), &session_id)
            .expect("rotate first Host capability");
        let second = rotate_session_execution_capability(dir.path(), &session_id)
            .expect("rotate second Host capability");
        assert_eq!(first.identity, initial.identity);
        assert_eq!(second.identity, initial.identity);
        assert_eq!(
            first.capability_generation,
            initial.capability_generation + 1
        );
        assert_eq!(
            second.capability_generation,
            first.capability_generation + 1
        );

        let mut workers = Vec::new();
        for _ in 0..4 {
            let sessions_dir = dir.path().to_path_buf();
            let session_id = session_id.clone();
            workers.push(std::thread::spawn(move || {
                rotate_session_execution_capability(&sessions_dir, &session_id)
                    .expect("concurrent capability rotation")
                    .capability_generation
            }));
        }
        let mut generations = workers
            .into_iter()
            .map(|worker| worker.join().expect("join capability rotation"))
            .collect::<Vec<_>>();
        generations.sort_unstable();
        assert_eq!(
            generations,
            vec![
                second.capability_generation + 1,
                second.capability_generation + 2,
                second.capability_generation + 3,
                second.capability_generation + 4,
            ],
            "the per-Session lock must serialize Host capability epochs"
        );

        let unbound = session_with_execution_owner();
        let unbound_id = unbound.id.clone();
        unbound.save(dir.path()).expect("save unbound Session");
        let unbound_path = dir.path().join(format!("{unbound_id}.toml"));
        let before = fs::read_to_string(&unbound_path).expect("read unbound Session");
        let error = rotate_session_execution_capability(dir.path(), &unbound_id)
            .expect_err("an unbound Session cannot receive producing capability");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            fs::read_to_string(&unbound_path).expect("read unchanged unbound Session"),
            before
        );
    }

    #[test]
    fn session_lease_holds_epoch_rotation_until_authorized_operation_finishes() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let session = session_with_execution_owner();
        let session_id = session.id.clone();
        let initial = test_session_execution_binding(&session);
        session.save(dir.path()).expect("save Session");
        persist_session_execution_binding(dir.path(), &session_id, Some(initial.clone()))
            .expect("persist initial binding");

        let (lease_acquired_tx, lease_acquired_rx) = std::sync::mpsc::sync_channel(1);
        let (release_lease_tx, release_lease_rx) = std::sync::mpsc::sync_channel(1);
        let sessions_dir = dir.path().to_path_buf();
        let leased_session_id = session_id.clone();
        let leased_binding = initial.clone();
        let lease_worker = std::thread::spawn(move || {
            with_session_lease_wait(
                &sessions_dir,
                &leased_session_id,
                std::time::Duration::from_secs(1),
                |locked| {
                    assert_eq!(
                        locked.execution_binding.as_ref(),
                        Some(&leased_binding),
                        "the leased read must observe the exact producing epoch"
                    );
                    lease_acquired_tx.send(()).expect("signal Session lease");
                    release_lease_rx.recv().expect("release Session lease");
                    Ok(())
                },
            )
            .expect("leased operation")
        });
        lease_acquired_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("Session lease acquired");

        let (rotation_tx, rotation_rx) = std::sync::mpsc::sync_channel(1);
        let sessions_dir = dir.path().to_path_buf();
        let rotated_session_id = session_id.clone();
        let rotation_worker = std::thread::spawn(move || {
            let rotated = rotate_session_execution_capability(&sessions_dir, &rotated_session_id);
            rotation_tx.send(rotated).expect("report Host rotation");
        });
        assert!(
            rotation_rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "Host epoch rotation must wait while an authorized operation holds the Session lease"
        );

        release_lease_tx.send(()).expect("release Session lease");
        lease_worker.join().expect("join leased operation");
        let rotated = rotation_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("Host rotation resumes")
            .expect("rotate Host epoch");
        rotation_worker.join().expect("join Host rotation");
        assert_eq!(
            rotated.capability_generation,
            initial.capability_generation + 1
        );
    }

    #[test]
    fn session_lease_contention_returns_retryable_timeout_then_succeeds() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let session = session_with_execution_owner();
        let session_id = session.id.clone();
        session.save(dir.path()).expect("save Session");

        let (lease_acquired_tx, lease_acquired_rx) = std::sync::mpsc::sync_channel(1);
        let (release_lease_tx, release_lease_rx) = std::sync::mpsc::sync_channel(1);
        let sessions_dir = dir.path().to_path_buf();
        let leased_session_id = session_id.clone();
        let lease_worker = std::thread::spawn(move || {
            with_session_lease_wait(
                &sessions_dir,
                &leased_session_id,
                std::time::Duration::from_secs(1),
                |_| {
                    lease_acquired_tx.send(()).expect("signal Session lease");
                    release_lease_rx.recv().expect("release Session lease");
                    Ok(())
                },
            )
            .expect("hold Session lease")
        });
        lease_acquired_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("Session lease acquired");

        let timeout = with_session_lease_wait(
            dir.path(),
            &session_id,
            std::time::Duration::from_millis(25),
            |_| Ok(()),
        )
        .expect_err("contended Session lease must return a bounded retry error");
        assert_eq!(timeout.kind(), io::ErrorKind::WouldBlock);
        assert!(timeout.to_string().contains("retry"));
        assert!(!timeout.to_string().contains(&session_id));

        release_lease_tx.send(()).expect("release Session lease");
        lease_worker.join().expect("join Session lease holder");
        with_session_lease_wait(
            dir.path(),
            &session_id,
            std::time::Duration::from_millis(100),
            |_| Ok(()),
        )
        .expect("retry succeeds after the concurrent lease is released");
    }

    #[test]
    fn persist_session_execution_binding_rejects_session_repo_owner_and_identity_mismatch() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let session = session_with_execution_owner();
        let session_id = session.id.clone();
        session.save(dir.path()).expect("save Session");
        let path = dir.path().join(format!("{session_id}.toml"));
        let original = fs::read_to_string(&path).expect("read original Session");

        let mut cases = Vec::new();
        let mut wrong_session = test_session_execution_binding(&session);
        wrong_session.session_id = "different-session".to_string();
        cases.push(("session", wrong_session));
        let mut wrong_repo = test_session_execution_binding(&session);
        wrong_repo.repo_hash = "different-repo".to_string();
        cases.push(("repository", wrong_repo));
        let mut wrong_owner = test_session_execution_binding(&session);
        wrong_owner.owner_number = 3248;
        cases.push(("owner", wrong_owner));
        let mut wrong_kind = test_session_execution_binding(&session);
        wrong_kind.owner_kind = "workspace".to_string();
        cases.push(("owner kind", wrong_kind));
        let mut missing_generation = test_session_execution_binding(&session);
        missing_generation.identity.generation_id = " ".to_string();
        cases.push(("generation", missing_generation));
        let mut missing_binding = test_session_execution_binding(&session);
        missing_binding.identity.binding_id.clear();
        cases.push(("binding", missing_binding));
        let mut missing_head = test_session_execution_binding(&session);
        missing_head.identity.ledger_head_hash.clear();
        cases.push(("ledger head", missing_head));
        let mut zero_capability = test_session_execution_binding(&session);
        zero_capability.capability_generation = 0;
        cases.push(("capability", zero_capability));

        for (expected, invalid) in cases {
            let error = persist_session_execution_binding(dir.path(), &session_id, Some(invalid))
                .expect_err("mismatched binding must fail closed");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(
                error.to_string().contains(expected),
                "expected {expected:?} diagnostic, got {error}"
            );
            assert_eq!(
                fs::read_to_string(&path).expect("read unchanged Session"),
                original,
                "rejected binding must preserve Session bytes"
            );
        }
    }

    #[test]
    fn docker_runtime_binding_uses_host_project_state_scope_and_persists() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work/repo");
        std::fs::create_dir_all(&worktree).expect("create worktree");
        let init = gwt_core::process::hidden_command("git")
            .args(["init", "-q", "-b", "work/docker-binding"])
            .current_dir(&worktree)
            .status()
            .expect("git init");
        assert!(init.success());
        let remote = gwt_core::process::hidden_command("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/docker-binding.git",
            ])
            .current_dir(&worktree)
            .status()
            .expect("git remote add");
        assert!(remote.success());

        let mut session = Session::new(&worktree, "work/docker-binding", AgentId::Codex);
        session.runtime_target = LaunchRuntimeTarget::Docker;
        session
            .bind_docker_runtime("/workspace/repo", &project_root)
            .expect("bind absolute POSIX Docker worktree");
        let binding = session
            .docker_runtime_binding
            .as_ref()
            .expect("Docker runtime binding");
        assert_eq!(
            binding.runtime_worktree_path,
            PathBuf::from("/workspace/repo")
        );
        assert_eq!(
            binding.project_state_scope_hash,
            gwt_core::paths::project_scope_hash(&project_root)
                .as_str()
                .to_string()
        );
        assert_ne!(
            Some(binding.project_state_scope_hash.as_str()),
            session.repo_hash.as_deref(),
            "workspace-home Project State scope must remain distinct from the repository hash"
        );

        let sessions_dir = temp.path().join("sessions");
        session.save(&sessions_dir).expect("save Session");
        let loaded = Session::load(&sessions_dir.join(format!("{}.toml", session.id)))
            .expect("reload Session");
        assert_eq!(
            loaded.docker_runtime_binding,
            session.docker_runtime_binding
        );
    }

    #[test]
    fn docker_runtime_binding_rejects_non_absolute_or_non_posix_worktree_paths() {
        let project_root = Path::new("/host/workspace-home");
        let mut session = Session::new("/host/worktree", "work/demo", AgentId::Codex);

        for path in [
            "workspace/repo",
            r"C:\workspace\repo",
            r"\workspace\repo",
            "/workspace/../repo",
            "/workspace//repo",
            "/workspace/repo/",
            "/",
            "/workspace\\repo",
        ] {
            let error = session
                .bind_docker_runtime(path, project_root)
                .expect_err("invalid Docker runtime worktree must fail closed");
            assert!(
                error.contains("absolute POSIX"),
                "unexpected error for {path:?}: {error}"
            );
            assert!(session.docker_runtime_binding.is_none());
        }
    }

    #[test]
    fn session_id_path_component_validation_is_platform_neutral() {
        for session_id in [
            "591d5d2a-9226-4584-a475-15952c49b37d",
            "legacy.session_opaque-id@v1",
            "opaque id retained for compatibility",
        ] {
            validate_session_id_path_component(session_id)
                .unwrap_or_else(|error| panic!("safe legacy id {session_id:?}: {error}"));
        }

        for session_id in [
            "",
            ".",
            "..",
            "nested/session",
            r"nested\session",
            "nul\0session",
            "/absolute/session",
            r"C:\absolute\session",
            "C:drive-relative-session",
        ] {
            assert!(
                validate_session_id_path_component(session_id).is_err(),
                "unsafe Session id must be rejected: {session_id:?}"
            );
        }
    }

    #[test]
    fn legacy_session_toml_without_restore_window_flag_defaults_to_false() {
        let legacy = r#"
id = "1d3d2d2d-3333-4444-5555-777777777777"
worktree_path = "/tmp/wt"
branch = "main"
agent_id = { type = "Codex" }
agent_session_id = "abc"
status = "WaitingInput"
launch_command = "codex"
launch_args = []
created_at = "2026-05-18T00:00:00Z"
updated_at = "2026-05-18T00:00:00Z"
last_activity_at = "2026-05-18T00:00:00Z"
display_name = "Codex"
"#;
        let session: Session = toml::from_str(legacy).expect("deserialize legacy");

        assert!(!session.restore_window_on_startup);
    }

    #[test]
    fn restore_window_on_startup_round_trips() {
        let mut session = Session::new("/tmp/wt", "main", AgentId::Codex);
        session.restore_window_on_startup = true;

        let serialized = toml::to_string(&session).expect("serialize");
        assert!(serialized.contains("restore_window_on_startup = true"));
        let parsed: Session = toml::from_str(&serialized).expect("deserialize");
        assert!(parsed.restore_window_on_startup);
    }

    #[test]
    fn project_state_root_round_trips() {
        let mut session = Session::new("/tmp/wt", "main", AgentId::Codex);
        session.project_state_root = Some(PathBuf::from("/tmp/workspace-home"));

        let serialized = toml::to_string(&session).expect("serialize");
        assert!(serialized.contains("project_state_root = \"/tmp/workspace-home\""));
        let parsed: Session = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(
            parsed.project_state_root.as_deref(),
            Some(Path::new("/tmp/workspace-home"))
        );
    }

    #[test]
    fn legacy_session_toml_without_project_state_root_defaults_to_none() {
        let legacy = r#"
id = "1d3d2d2d-3333-4444-5555-999999999999"
worktree_path = "/tmp/wt"
branch = "main"
agent_id = { type = "Codex" }
agent_session_id = "abc"
status = "WaitingInput"
launch_command = "codex"
launch_args = []
created_at = "2026-06-01T00:00:00Z"
updated_at = "2026-06-01T00:00:00Z"
last_activity_at = "2026-06-01T00:00:00Z"
display_name = "Codex"
"#;
        let session: Session = toml::from_str(legacy).expect("deserialize legacy");

        assert!(session.project_state_root.is_none());
    }

    #[test]
    fn legacy_session_toml_without_backend_id_deserializes_with_none() {
        // FR-102 backwards compatibility: sessions saved before the
        // 2026-05-18 amendment carry no `backend_id` field.
        let legacy = r#"
id = "1d3d2d2d-3333-4444-5555-666666666666"
worktree_path = "/tmp/wt"
branch = "main"
agent_id = { type = "ClaudeCode" }
agent_session_id = "abc"
status = "Unknown"
launch_command = ""
launch_args = []
created_at = "2026-05-18T00:00:00Z"
updated_at = "2026-05-18T00:00:00Z"
last_activity_at = "2026-05-18T00:00:00Z"
display_name = "Claude Code"
"#;
        let session: Session = toml::from_str(legacy).expect("deserialize legacy");
        assert!(session.backend_id.is_none());
    }

    #[test]
    fn session_with_backend_id_round_trips() {
        let mut session = Session::new("/tmp/wt", "main", AgentId::ClaudeCode);
        session.backend_id = Some("lmstudio".to_string());
        let serialized = toml::to_string(&session).expect("serialize");
        // FR-102: when present, persists under the canonical `backend_id` key.
        assert!(serialized.contains("backend_id = \"lmstudio\""));
        let parsed: Session = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(parsed.backend_id.as_deref(), Some("lmstudio"));
    }

    #[test]
    fn session_with_no_backend_id_omits_field_on_serialize() {
        let session = Session::new("/tmp/wt", "main", AgentId::ClaudeCode);
        let serialized = toml::to_string(&session).expect("serialize");
        // skip_serializing_if keeps the field out of clean session files.
        assert!(!serialized.contains("backend_id"));
    }

    #[test]
    fn update_status_touches_timestamps() {
        let mut session = Session::new("/tmp/wt", "main", AgentId::ClaudeCode);
        let before = session.updated_at;
        // Small sleep not needed; just verify the method works
        session.update_status(AgentStatus::Running);
        assert_eq!(session.status, AgentStatus::Running);
        assert!(session.updated_at >= before);
    }

    #[test]
    fn should_mark_stopped_returns_false_when_already_stopped() {
        let mut session = Session::new("/tmp/wt", "main", AgentId::ClaudeCode);
        session.status = AgentStatus::Stopped;
        assert!(!session.should_mark_stopped());
    }

    #[test]
    fn should_mark_stopped_recent_activity() {
        let session = Session::new("/tmp/wt", "main", AgentId::ClaudeCode);
        // Just created, so last_activity_at is now
        assert!(!session.should_mark_stopped());
    }

    #[test]
    fn should_mark_stopped_old_activity() {
        let mut session = Session::new("/tmp/wt", "main", AgentId::ClaudeCode);
        session.last_activity_at = Utc::now() - chrono::Duration::seconds(120);
        session.status = AgentStatus::Running;
        assert!(session.should_mark_stopped());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new("/tmp/wt", "feature/x", AgentId::Gemini);
        session.model = Some("gemini-3-flash-preview".into());
        session.tool_version = Some("0.1.0".into());
        session.agent_session_id = Some("agent-abc".into());
        session.reasoning_level = Some("high".into());
        session.skip_permissions = true;
        session.codex_fast_mode = true;
        session.runtime_target = LaunchRuntimeTarget::Docker;
        session.docker_service = Some("web".into());
        session.docker_lifecycle_intent = DockerLifecycleIntent::Restart;
        session.workflow_bypass = Some(WorkflowBypass::Release);
        session.launch_command = "codex".into();
        session.launch_args = vec![
            "--no-alt-screen".into(),
            "--model=gpt-5.4".into(),
            "resume".into(),
            "--last".into(),
        ];

        session.save(dir.path()).unwrap();

        let path = dir.path().join(format!("{}.toml", session.id));
        assert!(path.exists());

        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.branch, "feature/x");
        assert_eq!(loaded.agent_id, AgentId::Gemini);
        assert_eq!(loaded.model, Some("gemini-3-flash-preview".into()));
        assert_eq!(loaded.tool_version, Some("0.1.0".into()));
        assert_eq!(loaded.agent_session_id, Some("agent-abc".into()));
        assert_eq!(loaded.reasoning_level, Some("high".into()));
        assert!(loaded.skip_permissions);
        assert!(loaded.codex_fast_mode);
        assert_eq!(loaded.runtime_target, LaunchRuntimeTarget::Docker);
        assert_eq!(loaded.docker_service, Some("web".into()));
        assert_eq!(
            loaded.docker_lifecycle_intent,
            DockerLifecycleIntent::Restart
        );
        assert_eq!(loaded.launch_command, "codex");
        assert_eq!(
            loaded.launch_args,
            vec![
                "--no-alt-screen".to_string(),
                "--model=gpt-5.4".to_string(),
                "resume".to_string(),
                "--last".to_string()
            ]
        );
        assert_eq!(loaded.workflow_bypass, Some(WorkflowBypass::Release));
        assert_eq!(loaded.display_name, "Gemini CLI (legacy)");
    }

    #[test]
    fn load_legacy_codex_fast_mode_populates_fast_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-fast-mode.toml");
        let session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        let mut legacy = toml::map::Map::new();
        legacy.insert("id".into(), toml::Value::String(session.id.clone()));
        legacy.insert(
            "worktree_path".into(),
            toml::Value::String(session.worktree_path.display().to_string()),
        );
        legacy.insert("branch".into(), toml::Value::String(session.branch.clone()));
        legacy.insert(
            "agent_id".into(),
            toml::Value::try_from(&session.agent_id).unwrap(),
        );
        legacy.insert(
            "status".into(),
            toml::Value::try_from(session.status).unwrap(),
        );
        legacy.insert("codex_fast_mode".into(), toml::Value::Boolean(true));
        legacy.insert(
            "created_at".into(),
            toml::Value::String(session.created_at.to_rfc3339()),
        );
        legacy.insert(
            "updated_at".into(),
            toml::Value::String(session.updated_at.to_rfc3339()),
        );
        legacy.insert(
            "last_activity_at".into(),
            toml::Value::String(session.last_activity_at.to_rfc3339()),
        );
        legacy.insert(
            "display_name".into(),
            toml::Value::String(session.display_name.clone()),
        );
        std::fs::write(&path, toml::to_string(&legacy).unwrap()).unwrap();

        let loaded = Session::load(&path).unwrap();

        assert!(loaded.fast_mode);
        assert!(loaded.codex_fast_mode);
        assert!(loaded.fast_mode_enabled());
    }

    #[test]
    fn load_legacy_toml_without_runtime_fields_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.toml");
        let session = Session::new("/tmp/wt", "feature/x", AgentId::Gemini);
        let mut legacy = toml::map::Map::new();
        legacy.insert("id".into(), toml::Value::String(session.id.clone()));
        legacy.insert(
            "worktree_path".into(),
            toml::Value::String(session.worktree_path.display().to_string()),
        );
        legacy.insert("branch".into(), toml::Value::String(session.branch.clone()));
        legacy.insert(
            "agent_id".into(),
            toml::Value::try_from(session.agent_id.clone()).unwrap(),
        );
        legacy.insert(
            "agent_session_id".into(),
            toml::Value::String("agent-legacy".into()),
        );
        legacy.insert(
            "status".into(),
            toml::Value::try_from(session.status).unwrap(),
        );
        legacy.insert("tool_version".into(), toml::Value::String("1.2.3".into()));
        legacy.insert("model".into(), toml::Value::String("gemini-pro".into()));
        legacy.insert("reasoning_level".into(), toml::Value::String("high".into()));
        legacy.insert("skip_permissions".into(), toml::Value::Boolean(true));
        legacy.insert("codex_fast_mode".into(), toml::Value::Boolean(false));
        legacy.insert(
            "created_at".into(),
            toml::Value::try_from(session.created_at).unwrap(),
        );
        legacy.insert(
            "updated_at".into(),
            toml::Value::try_from(session.updated_at).unwrap(),
        );
        legacy.insert(
            "last_activity_at".into(),
            toml::Value::try_from(session.last_activity_at).unwrap(),
        );
        legacy.insert(
            "display_name".into(),
            toml::Value::String(session.display_name),
        );

        std::fs::write(&path, toml::to_string(&legacy).unwrap()).unwrap();

        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.runtime_target, LaunchRuntimeTarget::Host);
        assert!(loaded.docker_service.is_none());
        assert_eq!(
            loaded.docker_lifecycle_intent,
            DockerLifecycleIntent::Connect
        );
        assert!(loaded.launch_command.is_empty());
        assert!(loaded.launch_args.is_empty());
        assert!(loaded.workflow_bypass.is_none());
    }

    #[test]
    fn persist_agent_session_id_updates_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        let session_id = session.id.clone();
        session.save(dir.path()).unwrap();

        persist_agent_session_id(dir.path(), &session_id, "agent-123").unwrap();

        let loaded = Session::load(&dir.path().join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("agent-123"));
    }

    // SPEC-2359 Workspace → Work → Session: Claude Code / Codex can split one
    // launch (Work) into multiple conversation UUIDs. `persist_agent_session_id`
    // must keep every distinct UUID as forward-only Session history instead of
    // overwriting, so the projection can render the full Session list.
    #[test]
    fn persist_agent_session_id_appends_session_history_forward_only() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        let session_id = session.id.clone();
        session.save(dir.path()).unwrap();

        // First conversation UUID.
        persist_agent_session_id(dir.path(), &session_id, "agent-1").unwrap();
        // Duplicate of the current latest — must not add a second history entry.
        persist_agent_session_id(dir.path(), &session_id, "agent-1").unwrap();
        // Split: a new conversation UUID arrives (/clear, context limit, fork).
        persist_agent_session_id(dir.path(), &session_id, "agent-2").unwrap();

        let loaded = Session::load(&dir.path().join(format!("{session_id}.toml"))).unwrap();
        // Latest stays the most recent conversation (resume target).
        assert_eq!(loaded.agent_session_id.as_deref(), Some("agent-2"));
        // History keeps each distinct conversation in arrival order.
        let history: Vec<&str> = loaded
            .session_history
            .iter()
            .map(|entry| entry.agent_session_id.as_str())
            .collect();
        assert_eq!(history, vec!["agent-1", "agent-2"]);
        assert!(loaded.session_history[0].started_at <= loaded.session_history[1].started_at);
    }

    // SPEC-2359 Workspace → Work → Session: a Session row (one conversation
    // UUID) can be resumed directly. `resume_session_id_for` resumes the
    // requested conversation when given one, and otherwise falls back to the
    // latest captured handle (the plain Work resume).
    #[test]
    fn resume_session_id_for_prefers_requested_conversation() {
        let mut session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        session.agent_session_id = Some("conv-latest".to_string());

        // A specific (historical) Session is requested → resume that exact
        // conversation, not the latest one.
        assert_eq!(
            session.resume_session_id_for(Some("conv-older")),
            Some("conv-older".to_string()),
        );
        // No request → fall back to the latest captured conversation handle.
        assert_eq!(
            session.resume_session_id_for(None),
            Some("conv-latest".to_string()),
        );
        // Blank / placeholder requests are ignored and fall back to latest.
        assert_eq!(
            session.resume_session_id_for(Some("   ")),
            Some("conv-latest".to_string()),
        );
        assert_eq!(
            session.resume_session_id_for(Some(CODEX_PLACEHOLDER_SESSION_ID)),
            Some("conv-latest".to_string()),
        );
    }

    // SPEC-2359: per-Session Resume must hide the Resume control for a
    // conversation that cannot be resumed (empty handle / Codex placeholder)
    // rather than showing a button that silently fails.
    #[test]
    fn is_resumable_conversation_rejects_blank_and_codex_placeholder() {
        let codex = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        assert!(codex.is_resumable_conversation("95862acd-a761-4fd0"));
        assert!(!codex.is_resumable_conversation(""));
        assert!(!codex.is_resumable_conversation("   "));
        assert!(!codex.is_resumable_conversation(CODEX_PLACEHOLDER_SESSION_ID));

        // The placeholder is Codex-specific; for Claude Code it is a normal id.
        let claude = Session::new("/tmp/wt", "feature/x", AgentId::ClaudeCode);
        assert!(claude.is_resumable_conversation(CODEX_PLACEHOLDER_SESSION_ID));
        assert!(!claude.is_resumable_conversation(" "));
    }

    #[test]
    fn persist_session_restore_window_on_startup_updates_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        let session_id = session.id.clone();
        session.save(dir.path()).unwrap();

        persist_session_restore_window_on_startup(dir.path(), &session_id, true).unwrap();

        let loaded = Session::load(&dir.path().join(format!("{session_id}.toml"))).unwrap();
        assert!(loaded.restore_window_on_startup);

        persist_session_restore_window_on_startup(dir.path(), &session_id, false).unwrap();

        let loaded = Session::load(&dir.path().join(format!("{session_id}.toml"))).unwrap();
        assert!(!loaded.restore_window_on_startup);
    }

    #[test]
    fn concurrent_session_metadata_updates_preserve_history_and_parseable_toml() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        let session_id = session.id.clone();
        session.save(dir.path()).unwrap();

        let thread_count = 16;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(thread_count));
        let sessions_dir = std::sync::Arc::new(dir.path().to_path_buf());
        let mut handles = Vec::new();

        for index in 0..thread_count {
            let barrier = std::sync::Arc::clone(&barrier);
            let sessions_dir = std::sync::Arc::clone(&sessions_dir);
            let session_id = session_id.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                let agent_session_id = format!("agent-{index:02}");
                persist_agent_session_id(&sessions_dir, &session_id, &agent_session_id).unwrap();
                let event = if index % 2 == 0 {
                    "UserPromptSubmit"
                } else {
                    "PreToolUse"
                };
                persist_session_hook_event(&sessions_dir, &session_id, event).unwrap();
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let path = dir.path().join(format!("{session_id}.toml"));
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            toml::from_str::<toml::Value>(&raw).is_ok(),
            "session TOML must remain parseable after concurrent metadata updates:\n{raw}"
        );

        let loaded = Session::load(&path).unwrap();
        let mut history: Vec<_> = loaded
            .session_history
            .iter()
            .map(|entry| entry.agent_session_id.as_str())
            .collect();
        history.sort_unstable();
        let expected: Vec<_> = (0..thread_count)
            .map(|index| format!("agent-{index:02}"))
            .collect();
        assert_eq!(
            history,
            expected.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert!(
            matches!(
                loaded.last_hook_event.as_deref(),
                Some("UserPromptSubmit" | "PreToolUse")
            ),
            "last hook event should reflect one successful concurrent hook update"
        );

        let toml_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("toml"))
            .collect();
        assert_eq!(
            toml_files.len(),
            1,
            "session persistence must not leave temp files with .toml extension"
        );
    }

    // SPEC-1921 Phase 53 / FR-066: Session::load must not silently rewrite
    // launch_args. Migration must live in a named helper invoked explicitly.

    #[test]
    fn session_new_initializes_schema_version_to_current() {
        let session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        assert_eq!(
            session.schema_version,
            Session::CURRENT_SCHEMA_VERSION,
            "fresh sessions must use the current schema version"
        );
    }

    #[test]
    fn load_legacy_codex_toml_preserves_launch_args_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-codex-verbatim.toml");
        write_legacy_codex_session_file(
            &path,
            &[
                "--model=gpt-5.4".to_string(),
                "resume".to_string(),
                "sess-legacy".to_string(),
            ],
        );

        let loaded = Session::load(&path).unwrap();

        assert_eq!(
            loaded.schema_version, 0,
            "legacy TOML without schema_version must deserialize as version 0"
        );
        assert_eq!(
            loaded.launch_args,
            vec![
                "--model=gpt-5.4".to_string(),
                "resume".to_string(),
                "sess-legacy".to_string(),
            ],
            "Session::load must not rewrite launch_args (FR-066)"
        );
    }

    #[test]
    fn migrate_legacy_launch_args_injects_no_alt_screen_for_codex() {
        let mut session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        session.schema_version = 0;
        session.launch_command = "codex".into();
        session.launch_args = vec![
            "--model=gpt-5.4".to_string(),
            "resume".to_string(),
            "sess-legacy".to_string(),
        ];

        session.migrate_legacy_launch_args();

        assert_eq!(session.schema_version, Session::CURRENT_SCHEMA_VERSION);
        assert_eq!(
            session.launch_args,
            vec![
                "--no-alt-screen".to_string(),
                "--model=gpt-5.4".to_string(),
                "resume".to_string(),
                "sess-legacy".to_string(),
            ]
        );
    }

    #[test]
    fn migrate_legacy_launch_args_is_idempotent() {
        let mut session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        session.schema_version = 0;
        session.launch_command = "codex".into();
        session.launch_args = Vec::new();

        session.migrate_legacy_launch_args();
        let first_pass_args = session.launch_args.clone();
        let first_pass_version = session.schema_version;

        session.migrate_legacy_launch_args();

        assert_eq!(session.launch_args, first_pass_args);
        assert_eq!(session.schema_version, first_pass_version);
    }

    #[test]
    fn migrate_legacy_launch_args_removes_codex_hooks_enable_flag() {
        let mut session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        session.schema_version = 1;
        session.launch_command = "codex".into();
        session.launch_args = vec![
            "--no-alt-screen".to_string(),
            "resume".to_string(),
            "sess-legacy".to_string(),
            "--enable".to_string(),
            "codex_hooks".to_string(),
            "--enable".to_string(),
            "web_search".to_string(),
        ];

        session.migrate_legacy_launch_args();

        assert_eq!(session.schema_version, Session::CURRENT_SCHEMA_VERSION);
        assert_eq!(
            session.launch_args,
            vec![
                "--no-alt-screen".to_string(),
                "resume".to_string(),
                "sess-legacy".to_string(),
                "--enable".to_string(),
                "web_search".to_string(),
            ]
        );
    }

    #[test]
    fn migrate_legacy_launch_args_removes_codex_hooks_config_override() {
        let mut session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        session.schema_version = 1;
        session.launch_command = "codex".into();
        session.launch_args = vec![
            "--no-alt-screen".to_string(),
            "-c".to_string(),
            "features.codex_hooks = true".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ];

        session.migrate_legacy_launch_args();

        assert_eq!(session.schema_version, Session::CURRENT_SCHEMA_VERSION);
        assert_eq!(
            session.launch_args,
            vec![
                "--no-alt-screen".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ]
        );
    }

    #[test]
    fn migrate_legacy_launch_args_leaves_non_codex_sessions_unchanged() {
        let original = vec![
            "--dangerously-skip-permissions".to_string(),
            "--enable".to_string(),
            "codex_hooks".to_string(),
        ];
        let mut session = Session::new("/tmp/wt", "feature/x", AgentId::ClaudeCode);
        session.schema_version = 1;
        session.launch_command = "claude".into();
        session.launch_args = original.clone();

        session.migrate_legacy_launch_args();

        assert_eq!(session.schema_version, Session::CURRENT_SCHEMA_VERSION);
        assert_eq!(session.launch_args, original);
    }

    #[test]
    fn migrate_legacy_launch_args_skips_already_current_schema() {
        let mut session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        session.schema_version = Session::CURRENT_SCHEMA_VERSION;
        session.launch_command = "codex".into();
        session.launch_args = vec!["resume".to_string(), "sess-id".to_string()];
        let original = session.launch_args.clone();

        session.migrate_legacy_launch_args();

        assert_eq!(
            session.launch_args, original,
            "sessions already at current schema must not be touched"
        );
    }

    #[test]
    fn load_and_migrate_legacy_codex_toml_injects_no_alt_screen_into_launch_args() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-codex.toml");
        write_legacy_codex_session_file(
            &path,
            &[
                "--model=gpt-5.4".to_string(),
                "resume".to_string(),
                "sess-legacy".to_string(),
            ],
        );

        let loaded = Session::load_and_migrate(&path).unwrap();

        assert!(
            loaded
                .launch_args
                .iter()
                .any(|arg| arg == "--no-alt-screen"),
            "legacy Codex sessions loaded through load_and_migrate should preserve inline scrollback"
        );
        assert_eq!(
            loaded.launch_args,
            vec![
                "--no-alt-screen".to_string(),
                "--model=gpt-5.4".to_string(),
                "resume".to_string(),
                "sess-legacy".to_string(),
            ]
        );
        assert_eq!(loaded.schema_version, Session::CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn load_and_migrate_schema_one_codex_toml_removes_codex_hooks_enable_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-codex-schema-one.toml");
        write_session_file_with_schema_version(
            &path,
            AgentId::Codex,
            "codex",
            &[
                "--no-alt-screen".to_string(),
                "resume".to_string(),
                "sess-legacy".to_string(),
                "--enable".to_string(),
                "codex_hooks".to_string(),
                "--enable".to_string(),
                "web_search".to_string(),
            ],
            1,
        );

        let loaded = Session::load_and_migrate(&path).unwrap();

        assert_eq!(loaded.schema_version, Session::CURRENT_SCHEMA_VERSION);
        assert_eq!(
            loaded.launch_args,
            vec![
                "--no-alt-screen".to_string(),
                "resume".to_string(),
                "sess-legacy".to_string(),
                "--enable".to_string(),
                "web_search".to_string(),
            ]
        );
    }

    fn write_legacy_codex_session_file(path: &Path, launch_args: &[String]) {
        write_session_file_with_schema_version(path, AgentId::Codex, "codex", launch_args, 0);
    }

    fn write_session_file_with_schema_version(
        path: &Path,
        agent_id: AgentId,
        launch_command: &str,
        launch_args: &[String],
        schema_version: u32,
    ) {
        let session = Session::new("/tmp/wt", "feature/x", agent_id.clone());
        let mut legacy = toml::map::Map::new();
        legacy.insert("id".into(), toml::Value::String(session.id.clone()));
        legacy.insert(
            "worktree_path".into(),
            toml::Value::String(session.worktree_path.display().to_string()),
        );
        legacy.insert("branch".into(), toml::Value::String(session.branch.clone()));
        legacy.insert("agent_id".into(), toml::Value::try_from(agent_id).unwrap());
        legacy.insert(
            "status".into(),
            toml::Value::try_from(session.status).unwrap(),
        );
        legacy.insert(
            "launch_command".into(),
            toml::Value::String(launch_command.to_string()),
        );
        legacy.insert(
            "launch_args".into(),
            toml::Value::Array(
                launch_args
                    .iter()
                    .map(|arg| toml::Value::String(arg.clone()))
                    .collect(),
            ),
        );
        if schema_version > 0 {
            legacy.insert(
                "schema_version".into(),
                toml::Value::Integer(i64::from(schema_version)),
            );
        }
        legacy.insert(
            "created_at".into(),
            toml::Value::try_from(session.created_at).unwrap(),
        );
        legacy.insert(
            "updated_at".into(),
            toml::Value::try_from(session.updated_at).unwrap(),
        );
        legacy.insert(
            "last_activity_at".into(),
            toml::Value::try_from(session.last_activity_at).unwrap(),
        );
        legacy.insert(
            "display_name".into(),
            toml::Value::String(session.display_name),
        );

        std::fs::write(path, toml::to_string(&legacy).unwrap()).unwrap();
    }

    #[test]
    fn load_nonexistent_returns_error() {
        let result = Session::load(Path::new("/nonexistent/session.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn load_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "not valid toml {{{{").unwrap();
        let result = Session::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn hook_runtime_state_maps_idle_and_running_events() {
        for event in ["UserPromptSubmit", "PreToolUse", "PostToolUse"] {
            let runtime = SessionRuntimeState::from_hook_event(event).expect("running event");
            assert_eq!(runtime.status, AgentStatus::Running, "{event}");
            assert_eq!(runtime.source_event.as_deref(), Some(event));
        }

        let session_start =
            SessionRuntimeState::from_hook_event("SessionStart").expect("session start event");
        assert_eq!(
            serde_json::to_string(&session_start.status).unwrap(),
            "\"Idle\""
        );
        assert_eq!(session_start.source_event.as_deref(), Some("SessionStart"));

        let idle = SessionRuntimeState::from_hook_event("Stop").expect("idle event");
        assert_eq!(serde_json::to_string(&idle.status).unwrap(), "\"Idle\"");
        assert_eq!(idle.source_event.as_deref(), Some("Stop"));

        assert!(SessionRuntimeState::from_hook_event("Notification").is_none());
    }

    #[test]
    fn runtime_state_save_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime").join("session-123.json");
        let first = SessionRuntimeState::new(AgentStatus::Running);
        first.save(&path).unwrap();

        let second = SessionRuntimeState::new(AgentStatus::Idle);
        second.save(&path).unwrap();

        let loaded = SessionRuntimeState::load(&path).unwrap();
        assert_eq!(serde_json::to_string(&loaded.status).unwrap(), "\"Idle\"");
    }

    #[test]
    fn concurrent_runtime_state_writers_use_unique_atomic_replacements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime").join("session-123.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let writers = (1..=8)
            .map(|generation| {
                let path = path.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let state = SessionRuntimeState::new_for_execution_capability(
                        AgentStatus::Running,
                        generation,
                    );
                    barrier.wait();
                    state.save(&path)
                })
            })
            .collect::<Vec<_>>();

        for writer in writers {
            writer.join().expect("join runtime writer").unwrap();
        }
        let loaded = SessionRuntimeState::load(&path).expect("load one complete replacement");
        assert!(matches!(loaded.capability_generation, Some(1..=8)));
        let temporary_siblings = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(temporary_siblings, 0);
    }

    #[test]
    fn runtime_state_deserializes_legacy_sidecar_without_capability_epoch() {
        let runtime: SessionRuntimeState = serde_json::from_value(serde_json::json!({
            "status": "Running",
            "updated_at": "2026-08-05T00:00:00Z",
            "last_activity_at": "2026-08-05T00:00:00Z",
            "source_event": "SessionStart"
        }))
        .expect("deserialize legacy runtime sidecar");

        assert_eq!(runtime.status, AgentStatus::Running);
        assert_eq!(runtime.capability_generation, None);
        assert_eq!(runtime.source_event.as_deref(), Some("SessionStart"));
    }

    #[test]
    fn exact_running_capability_claim_has_one_winner_and_stamps_runtime_epoch() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut session = session_with_execution_owner();
        session.update_status(AgentStatus::Running);
        let session_id = session.id.clone();
        let initial = test_session_execution_binding(&session);
        session.save(dir.path()).expect("save Session");
        persist_session_execution_binding(dir.path(), &session_id, Some(initial.clone()))
            .expect("persist initial binding");
        let runtime_path = runtime_state_path(dir.path(), &session_id);

        let (claim_entered_tx, claim_entered_rx) = std::sync::mpsc::sync_channel(0);
        let (release_claim_tx, release_claim_rx) = std::sync::mpsc::sync_channel(0);
        let first_dir = dir.path().to_path_buf();
        let first_runtime = runtime_path.clone();
        let first_expected = initial.clone();
        let first = std::thread::spawn(move || {
            claim_running_session_execution_capability_if_unchanged_with(
                &first_dir,
                &first_expected,
                &first_runtime,
                |binding| {
                    claim_entered_tx
                        .send(binding.capability_generation)
                        .expect("report first claim");
                    release_claim_rx.recv().expect("release first claim");
                    "first"
                },
            )
            .expect("first claim")
        });
        let first_generation = claim_entered_rx.recv().expect("first claim entered");

        let second_operation_ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let second_operation_flag = std::sync::Arc::clone(&second_operation_ran);
        let second_dir = dir.path().to_path_buf();
        let second_runtime = runtime_path.clone();
        let second_expected = initial.clone();
        let second = std::thread::spawn(move || {
            claim_running_session_execution_capability_if_unchanged_with(
                &second_dir,
                &second_expected,
                &second_runtime,
                |_| {
                    second_operation_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    "second"
                },
            )
            .expect("second claim")
        });
        release_claim_tx.send(()).expect("release first claim");

        let first = first.join().expect("join first claim");
        let second = second.join().expect("join second claim");
        assert_eq!(first.as_ref().map(|(_, value)| *value), Some("first"));
        assert!(second.is_none(), "the stale exact claim must lose");
        assert!(!second_operation_ran.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(first_generation, initial.capability_generation + 1);

        let durable = Session::load(&dir.path().join(format!("{session_id}.toml")))
            .expect("load claimed Session");
        let claimed = durable.execution_binding.expect("claimed binding");
        assert_eq!(claimed.capability_generation, first_generation);
        assert_eq!(
            SessionRuntimeState::load(&runtime_path)
                .expect("load claimed runtime")
                .capability_generation,
            Some(first_generation),
            "the runtime sidecar must identify the same capability epoch"
        );
        assert!(
            !capability_claim_failure_matches(dir.path(), &claimed)
                .expect("successful claim receipt readback"),
            "a successful claim must clear its write-ahead failure receipt before issuance"
        );
    }

    #[test]
    fn failed_capability_runtime_publish_records_preissuance_recovery_proof() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut session = session_with_execution_owner();
        session.update_status(AgentStatus::Running);
        let session_id = session.id.clone();
        let initial = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(initial.clone()))
            .expect("bind initial capability");
        session.save(dir.path()).expect("save Session");
        let runtime_path = runtime_state_path(dir.path(), &session_id);
        std::fs::create_dir_all(&runtime_path)
            .expect("block runtime file replacement with a directory");
        let operation_ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let operation_flag = std::sync::Arc::clone(&operation_ran);

        let error = claim_running_session_execution_capability_if_unchanged_with(
            dir.path(),
            &initial,
            &runtime_path,
            |_| operation_flag.store(true, std::sync::atomic::Ordering::SeqCst),
        )
        .expect_err("runtime publication failure must abort before issuance");

        assert!(error
            .to_string()
            .contains("durable pre-issuance failure receipt"));
        assert!(!operation_ran.load(std::sync::atomic::Ordering::SeqCst));
        let durable = Session::load(&dir.path().join(format!("{session_id}.toml")))
            .expect("reload failed claim");
        assert_eq!(durable.status, AgentStatus::Running);
        let claimed = durable.execution_binding.expect("retain claimed binding");
        assert_eq!(
            claimed.capability_generation,
            initial.capability_generation + 1
        );
        assert!(
            capability_claim_failure_matches(dir.path(), &claimed).expect("read recovery proof"),
            "the exact failed epoch must be durably distinguishable from a late old stop"
        );
    }

    #[test]
    fn retry_cannot_overwrite_current_epoch_failure_receipt() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut session = session_with_execution_owner();
        session.update_status(AgentStatus::Running);
        let session_id = session.id.clone();
        let initial = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(initial.clone()))
            .expect("bind initial capability");
        session.save(dir.path()).expect("save Session");
        let runtime_path = runtime_state_path(dir.path(), &session_id);
        fs::create_dir_all(&runtime_path).expect("block runtime publication");

        claim_running_session_execution_capability_if_unchanged_with(
            dir.path(),
            &initial,
            &runtime_path,
            |_| panic!("failed claim must not issue capability"),
        )
        .expect_err("first claim must fail");
        let session_path = dir.path().join(format!("{session_id}.toml"));
        let durable_before = fs::read(&session_path).expect("read failed claim Session");
        let claimed = Session::load(&session_path)
            .expect("load failed claim Session")
            .execution_binding
            .expect("claimed binding");
        let receipt_path = capability_claim_failure_path(dir.path(), &session_id);
        let receipt_before = fs::read(&receipt_path).expect("read failed claim receipt");

        let retry_error = claim_running_session_execution_capability_if_unchanged_with(
            dir.path(),
            &claimed,
            &runtime_path,
            |_| panic!("unreconciled retry must not issue capability"),
        )
        .expect_err("unreconciled current epoch must refuse retry");

        assert!(retry_error.to_string().contains("unreconciled"));
        assert_eq!(
            fs::read(&session_path).expect("Session readback"),
            durable_before
        );
        assert_eq!(
            fs::read(&receipt_path).expect("receipt readback"),
            receipt_before,
            "a retry must preserve the current epoch's only failure proof"
        );
    }

    #[test]
    fn claimed_epoch_has_write_ahead_recovery_proof_before_followup_work() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut session = session_with_execution_owner();
        session.update_status(AgentStatus::Running);
        let session_id = session.id.clone();
        let initial = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(initial.clone()))
            .expect("bind initial capability");
        session.save(dir.path()).expect("save Session");
        let runtime_path = runtime_state_path(dir.path(), &session_id);
        let operation_ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let operation_flag = std::sync::Arc::clone(&operation_ran);
        set_capability_claim_post_session_write_hook_for_test(Box::new(|| {
            Err(io::Error::other("simulated crash boundary"))
        }));

        claim_running_session_execution_capability_if_unchanged_with(
            dir.path(),
            &initial,
            &runtime_path,
            |_| operation_flag.store(true, std::sync::atomic::Ordering::SeqCst),
        )
        .expect_err("the injected post-claim boundary must abort issuance");

        assert!(!operation_ran.load(std::sync::atomic::Ordering::SeqCst));
        let durable = Session::load(&dir.path().join(format!("{session_id}.toml")))
            .expect("reload claimed Session");
        let claimed = durable.execution_binding.expect("retain claimed binding");
        assert_eq!(
            claimed.capability_generation,
            initial.capability_generation + 1
        );
        assert!(
            capability_claim_failure_matches(dir.path(), &claimed)
                .expect("read write-ahead recovery proof"),
            "the exact receipt must predate every post-claim failure boundary"
        );
    }

    #[test]
    fn receipt_unlink_unknown_outcome_restores_failed_phase_before_abort() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut session = session_with_execution_owner();
        session.update_status(AgentStatus::Running);
        let session_id = session.id.clone();
        let initial = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(initial.clone()))
            .expect("bind initial capability");
        session.save(dir.path()).expect("save Session");
        let runtime_path = runtime_state_path(dir.path(), &session_id);
        let operation_ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let operation_flag = std::sync::Arc::clone(&operation_ran);
        set_capability_receipt_post_unlink_failure_for_test();

        claim_running_session_execution_capability_if_unchanged_with(
            dir.path(),
            &initial,
            &runtime_path,
            |_| operation_flag.store(true, std::sync::atomic::Ordering::SeqCst),
        )
        .expect_err("unknown receipt unlink outcome must abort issuance");

        assert!(!operation_ran.load(std::sync::atomic::Ordering::SeqCst));
        let claimed = Session::load(&dir.path().join(format!("{session_id}.toml")))
            .expect("reload claimed Session")
            .execution_binding
            .expect("claimed binding");
        assert!(matches!(
            capability_claim_receipt_state(dir.path(), &claimed).expect("read restored receipt"),
            CapabilityClaimReceiptState::Failed
        ));
    }

    #[test]
    fn stale_or_legacy_status_writer_cannot_stop_a_newer_capability_epoch() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut session = session_with_execution_owner();
        session.update_status(AgentStatus::Running);
        let session_id = session.id.clone();
        let binding = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind producing Session");
        session.save(dir.path()).expect("save bound Session");
        let runtime_path = runtime_state_path(dir.path(), &session_id);
        SessionRuntimeState::new_for_execution_capability(
            AgentStatus::Running,
            binding.capability_generation,
        )
        .save(&runtime_path)
        .expect("save current runtime state");

        assert!(
            !persist_session_status_if_capability_generation_matches(
                dir.path(),
                &session_id,
                Some(binding.capability_generation - 1),
                AgentStatus::Stopped,
            )
            .expect("reject stale status writer"),
            "a stale window must not update the current Session"
        );
        assert!(
            !persist_session_status_if_capability_generation_matches(
                dir.path(),
                &session_id,
                None,
                AgentStatus::Stopped,
            )
            .expect("reject legacy status writer"),
            "an untagged legacy window must not update capability epoch 2+"
        );

        let retained = Session::load(&dir.path().join(format!("{session_id}.toml")))
            .expect("reload retained Session");
        assert_eq!(retained.status, AgentStatus::Running);
        let retained_runtime =
            SessionRuntimeState::load(&runtime_path).expect("reload retained runtime");
        assert_eq!(retained_runtime.status, AgentStatus::Running);
        assert_eq!(
            retained_runtime.capability_generation,
            Some(binding.capability_generation)
        );

        assert!(
            persist_session_status_if_capability_generation_matches(
                dir.path(),
                &session_id,
                Some(binding.capability_generation),
                AgentStatus::Stopped,
            )
            .expect("persist current status writer"),
            "the current window must retain a working terminal path"
        );
        let stopped = Session::load(&dir.path().join(format!("{session_id}.toml")))
            .expect("reload stopped Session");
        assert_eq!(stopped.status, AgentStatus::Stopped);
        let stopped_runtime =
            SessionRuntimeState::load(&runtime_path).expect("reload stopped runtime");
        assert_eq!(stopped_runtime.status, AgentStatus::Stopped);
        assert_eq!(
            stopped_runtime.capability_generation,
            Some(binding.capability_generation)
        );
    }

    #[test]
    fn current_epoch_status_writer_leaves_session_unchanged_when_sidecar_cannot_publish() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut session = session_with_execution_owner();
        session.update_status(AgentStatus::Running);
        let session_id = session.id.clone();
        let binding = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind producing Session");
        session.save(dir.path()).expect("save bound Session");
        let session_path = dir.path().join(format!("{session_id}.toml"));
        let session_before = fs::read(&session_path).expect("read Session before failed write");
        let runtime_path = runtime_state_path(dir.path(), &session_id);
        fs::create_dir_all(&runtime_path).expect("block sidecar replacement with directory");

        persist_session_status_if_capability_generation_matches(
            dir.path(),
            &session_id,
            Some(binding.capability_generation),
            AgentStatus::Stopped,
        )
        .expect_err("sidecar publication failure must abort the paired Session write");

        assert_eq!(
            fs::read(&session_path).expect("read Session after failed write"),
            session_before,
            "a failed exact-epoch sidecar write must leave Session byte-identical"
        );
    }

    #[test]
    fn sidecar_post_replace_error_uses_exact_readback_and_commits_session() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut session = session_with_execution_owner();
        session.update_status(AgentStatus::Running);
        let session_id = session.id.clone();
        let binding = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind producing Session");
        session.save(dir.path()).expect("save bound Session");
        set_atomic_state_post_replace_failure_for_test();

        assert!(persist_session_status_if_capability_generation_matches(
            dir.path(),
            &session_id,
            Some(binding.capability_generation),
            AgentStatus::Stopped,
        )
        .expect("exact sidecar readback must resolve post-replace error"));

        let durable = Session::load(&dir.path().join(format!("{session_id}.toml")))
            .expect("reload stopped Session");
        assert_eq!(durable.status, AgentStatus::Stopped);
        let runtime = SessionRuntimeState::load(&runtime_state_path(dir.path(), &session_id))
            .expect("reload terminal proof");
        assert_eq!(runtime.status, AgentStatus::Stopped);
        assert_eq!(
            runtime.capability_generation,
            Some(binding.capability_generation)
        );
    }

    #[test]
    fn session_write_failure_after_terminal_sidecar_keeps_exact_recovery_proof() {
        let dir = tempfile::tempdir().expect("temp sessions dir");
        let mut session = session_with_execution_owner();
        session.update_status(AgentStatus::Running);
        let session_id = session.id.clone();
        let binding = test_session_execution_binding(&session);
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind producing Session");
        session.save(dir.path()).expect("save bound Session");
        let session_path = dir.path().join(format!("{session_id}.toml"));
        let session_before = fs::read(&session_path).expect("read Session before failure");
        set_paired_runtime_before_session_write_failure_for_test();

        persist_session_status_if_capability_generation_matches(
            dir.path(),
            &session_id,
            Some(binding.capability_generation),
            AgentStatus::Stopped,
        )
        .expect_err("injected Session metadata failure");

        assert_eq!(
            fs::read(&session_path).expect("read Session after failure"),
            session_before,
            "Session commit marker must remain byte-identical"
        );
        let runtime = SessionRuntimeState::load(&runtime_state_path(dir.path(), &session_id))
            .expect("reload terminal proof");
        assert_eq!(runtime.status, AgentStatus::Stopped);
        assert_eq!(
            runtime.capability_generation,
            Some(binding.capability_generation)
        );
    }

    #[test]
    fn runtime_state_path_scopes_sidecars_to_current_process_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = runtime_state_path(dir.path(), "session-123");

        assert_eq!(
            path,
            dir.path()
                .join("runtime")
                .join(std::process::id().to_string())
                .join("session-123.json")
        );
    }

    #[test]
    fn sessions_dir_from_runtime_path_recovers_sessions_root() {
        let sessions_dir = PathBuf::from("/tmp/.gwt/sessions");
        let runtime_path = sessions_dir
            .join("runtime")
            .join("4242")
            .join("session-123.json");

        assert_eq!(
            sessions_dir_from_runtime_path(&runtime_path).as_deref(),
            Some(sessions_dir.as_path())
        );
    }

    #[test]
    fn session_from_launch_config_captures_launch_metadata() {
        let mut config = crate::AgentLaunchBuilder::new(AgentId::Codex)
            .working_dir("/tmp/worktree")
            .branch("feature/demo")
            .version("0.122.0")
            .build();
        config.command = "npx".to_string();
        config.args = vec![
            "--yes".to_string(),
            "@openai/codex@0.122.0".to_string(),
            "--no-alt-screen".to_string(),
        ];
        config.model = Some("gpt-5.5".to_string());
        config.reasoning_level = Some("high".to_string());
        config.skip_permissions = true;
        config.fast_mode = true;
        config.codex_fast_mode = true;
        config.runtime_target = LaunchRuntimeTarget::Docker;
        config.docker_service = Some("app".to_string());
        config.docker_lifecycle_intent = DockerLifecycleIntent::Restart;
        config.linked_issue_number = Some(1921);
        config.session_mode = crate::SessionMode::Continue;

        let session = Session::from_launch_config("/tmp/worktree", "feature/demo", &config);

        assert_eq!(session.branch, "feature/demo");
        assert_eq!(session.agent_id, AgentId::Codex);
        assert_eq!(session.launch_command, "npx");
        assert_eq!(
            session.launch_args,
            vec![
                "--yes".to_string(),
                "@openai/codex@0.122.0".to_string(),
                "--no-alt-screen".to_string(),
            ]
        );
        assert_eq!(session.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(session.reasoning_level.as_deref(), Some("high"));
        assert!(session.skip_permissions);
        assert!(session.fast_mode);
        assert!(session.codex_fast_mode);
        assert_eq!(session.runtime_target, LaunchRuntimeTarget::Docker);
        assert_eq!(session.docker_service.as_deref(), Some("app"));
        assert_eq!(
            session.docker_lifecycle_intent,
            DockerLifecycleIntent::Restart
        );
        assert_eq!(session.linked_issue_number, Some(1921));
        assert_eq!(session.session_mode, crate::SessionMode::Continue);
        assert_eq!(session.status, AgentStatus::Running);
    }

    #[test]
    fn session_from_launch_config_persists_windows_shell_choice() {
        let mut config = crate::AgentLaunchBuilder::new(AgentId::Codex)
            .working_dir("/tmp/worktree")
            .branch("feature/shell")
            .build();
        config.command = "pwsh".to_string();
        config.args = vec![
            "-NoLogo".to_string(),
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "& 'codex'".to_string(),
        ];
        config.windows_shell = Some(WindowsShellKind::PowerShell7);

        let session = Session::from_launch_config("/tmp/worktree", "feature/shell", &config);

        assert_eq!(session.windows_shell, Some(WindowsShellKind::PowerShell7));
        assert_eq!(session.launch_command, "pwsh");
        assert_eq!(session.launch_args, config.args);
    }

    #[test]
    fn load_and_migrate_marks_legacy_existing_worktree_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("repo-worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let path = dir.path().join("legacy.toml");
        let mut session = Session::new(&worktree, "feature/recover", AgentId::Codex);
        session.status = AgentStatus::Running;
        session.agent_session_id = Some("legacy-native-session".to_string());
        session.schema_version = 2;
        let mut value = toml::Value::try_from(&session)
            .unwrap()
            .as_table()
            .unwrap()
            .clone();
        value.remove("last_hook_event");
        value.remove("last_hook_event_at");
        value.remove("last_completed_stop_at");
        std::fs::write(&path, toml::to_string(&value).unwrap()).unwrap();

        let loaded = Session::load_and_migrate(&path).unwrap();

        assert_eq!(loaded.schema_version, Session::CURRENT_SCHEMA_VERSION);
        assert_eq!(loaded.status, AgentStatus::Interrupted);
        assert!(loaded.interrupted_recovery_candidate());
        assert!(
            !loaded.exact_auto_resume_candidate(),
            "legacy sessions without lifecycle evidence remain manually recoverable but must not be eagerly auto-resumed"
        );
    }

    #[test]
    fn lifecycle_events_drive_interrupted_recovery_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("repo-worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let mut session = Session::new(&worktree, "feature/recover", AgentId::Codex);

        session.record_hook_event("UserPromptSubmit");
        assert!(session.should_mark_interrupted_from_lifecycle());

        session.record_hook_event("Stop");
        assert!(session.should_mark_interrupted_from_lifecycle());

        session.record_completed_stop();
        assert!(!session.should_mark_interrupted_from_lifecycle());
    }

    #[test]
    fn completed_stop_session_remains_exact_auto_resume_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("repo-worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let mut session = Session::new(&worktree, "feature/recover", AgentId::Codex);
        session.agent_session_id = Some("codex-native-session".to_string());

        session.record_hook_event("Stop");
        session.record_completed_stop();

        assert!(!session.should_mark_interrupted_from_lifecycle());
        assert!(session.exact_auto_resume_candidate());

        std::fs::remove_dir_all(&worktree).unwrap();
        assert!(!session.exact_auto_resume_candidate());
        std::fs::create_dir_all(&worktree).unwrap();
        session.update_status(AgentStatus::Stopped);
        assert!(!session.exact_auto_resume_candidate());
    }

    #[test]
    fn placeholder_agent_session_id_is_not_exact_auto_resume_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("repo-worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let mut session = Session::new(&worktree, "feature/recover", AgentId::Codex);
        session.agent_session_id = Some(CODEX_PLACEHOLDER_SESSION_ID.to_string());

        session.record_hook_event("Stop");
        session.record_completed_stop();

        assert!(
            !session.exact_auto_resume_candidate(),
            "Codex hook placeholder ids are not valid `codex resume <id>` targets"
        );
    }

    #[test]
    fn reset_runtime_state_dir_for_pid_clears_only_target_pid_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let current_pid = 4242_u32;
        let other_pid = 4343_u32;
        let current_dir = dir.path().join("runtime").join(current_pid.to_string());
        let other_dir = dir.path().join("runtime").join(other_pid.to_string());

        std::fs::create_dir_all(&current_dir).unwrap();
        std::fs::create_dir_all(&other_dir).unwrap();
        std::fs::write(current_dir.join("session-a.json"), "{}").unwrap();
        std::fs::write(other_dir.join("session-b.json"), "{}").unwrap();

        reset_runtime_state_dir_for_pid(dir.path(), current_pid).unwrap();

        assert!(current_dir.is_dir());
        assert_eq!(std::fs::read_dir(&current_dir).unwrap().count(), 0);
        assert!(other_dir.join("session-b.json").exists());
    }
}
