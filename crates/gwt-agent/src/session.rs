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
    launch::{normalize_launch_args, LaunchConfig, ManualLaunchRuntimeProof},
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

/// Durable cross-process fence for an in-place Active Session relaunch.
///
/// The owner and Session leases serialize creation with terminal successor
/// settlement. A marker survives process failure, so an uncertain launch can
/// never be mistaken for an exact terminal holder until recovery clears it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionActiveLaunchHandshake {
    pub schema_version: u32,
    pub nonce: String,
    pub execution_identity: SessionExecutionIdentity,
    pub host_pid: u32,
    pub host_started_at: u64,
    #[serde(default)]
    pub phase: SessionActiveLaunchPhase,
    pub created_at: DateTime<Utc>,
}

impl SessionActiveLaunchHandshake {
    pub const CURRENT_SCHEMA_VERSION: u32 = 2;
}

/// Durable progress of one exact Active Session launch.
///
/// `LegacyUnclassified` exists only so a phase-less schema-v1 marker remains
/// deserializable. Validation rejects it, keeping recovery fail-closed instead
/// of guessing whether a child was spawned before the Host crashed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionActiveLaunchPhase {
    #[default]
    LegacyUnclassified,
    PreSpawn,
    ChildSpawned {
        child_pid: u32,
        child_started_at: u64,
    },
}

/// Durable cross-process fence for one exact manual Session handoff.
///
/// The marker shares the Session lease with Active launch handshakes so the
/// two ownership transitions cannot be prepared concurrently. The host start
/// identity disambiguates PID reuse after a coordinator process restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionManualHandoffFence {
    pub schema_version: u32,
    pub nonce: String,
    pub execution_identity: SessionExecutionIdentity,
    pub host_pid: u32,
    pub host_started_at: u64,
    pub created_at: DateTime<Utc>,
}

impl SessionManualHandoffFence {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;
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

/// Path-independent package runner used for one versioned tool launch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolRuntimeRunnerKind {
    Npx,
}

/// Why gwt resolved an exact package version for one tool launch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolRuntimeResolutionReason {
    RequestedSelector,
    InstalledFallback,
    LegacyMigration,
}

/// Durable, non-secret provenance for a versioned tool launch.
///
/// Absolute executable paths are intentionally excluded so a Session can be
/// resumed after the npm installation or machine-local PATH changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolRuntimeProvenance {
    pub schema_version: u32,
    pub official_package: String,
    pub requested_selector: String,
    pub resolved_exact_version: String,
    pub runner_kind: ToolRuntimeRunnerKind,
    pub resolution_reason: ToolRuntimeResolutionReason,
}

impl ToolRuntimeProvenance {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;
}

/// Select the machine-independent command identity stored in a [`Session`].
///
/// Targeted Windows Host package launches execute the absolute `npx.cmd` path
/// selected by the launch environment, but that machine-local path must not
/// become durable Session state. All other launch configurations retain their
/// existing command unchanged.
#[must_use]
pub fn durable_session_launch_command(config: &LaunchConfig) -> String {
    if !cfg!(windows)
        || config.runtime_target != LaunchRuntimeTarget::Host
        || !matches!(config.agent_id, AgentId::Codex | AgentId::ClaudeCode)
    {
        return config.command.clone();
    }

    let Some(provenance) = config.tool_runtime_provenance.as_ref() else {
        return config.command.clone();
    };
    if config.agent_id.package_name() != Some(provenance.official_package.as_str()) {
        return config.command.clone();
    }

    let command_name = Path::new(&config.command)
        .file_name()
        .and_then(|name| name.to_str());
    match provenance.runner_kind {
        ToolRuntimeRunnerKind::Npx
            if command_name.is_some_and(|name| name.eq_ignore_ascii_case("npx.cmd")) =>
        {
            "npx.cmd".to_string()
        }
        _ => config.command.clone(),
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_runtime_provenance: Option<ToolRuntimeProvenance>,
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
    /// Exact durable producing Session represented by this runtime sidecar.
    /// Legacy and hook-only sidecars omit it and therefore cannot authorize a
    /// destructive terminal handoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_identity: Option<SessionExecutionIdentity>,
    /// Process-local PTY identity paired with [`Self::execution_identity`].
    /// A reused window id receives a new value, fencing late predecessor
    /// events from terminalizing its successor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_incarnation: Option<u64>,
    /// OS start timestamp for the Host process named by the sidecar's PID
    /// namespace. Recovery compares both values before trusting that namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_started_at: Option<u64>,
    /// OS process identity for the PTY child. The start timestamp prevents a
    /// recycled PID from keeping an abandoned launch fence alive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_started_at: Option<u64>,
    #[serde(default)]
    pub source_event: Option<String>,
    #[serde(default)]
    pub pending_discussion: Option<PendingDiscussionResume>,
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
            tool_runtime_provenance: None,
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
        session.tool_runtime_provenance = config.tool_runtime_provenance.clone();
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
        session.launch_command = durable_session_launch_command(config);
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
            execution_identity: None,
            runtime_incarnation: None,
            host_started_at: None,
            child_pid: None,
            child_started_at: None,
            source_event: None,
            pending_discussion: None,
        }
    }

    /// Create an exact runtime proof for one durable producing Session.
    pub fn for_execution(
        status: AgentStatus,
        identity: &SessionExecutionIdentity,
        incarnation: u64,
    ) -> Self {
        Self {
            execution_identity: Some(identity.clone()),
            runtime_incarnation: Some(incarnation),
            ..Self::new(status)
        }
    }

    /// Create exact Running proof including the PTY child process identity.
    pub fn for_execution_process(
        status: AgentStatus,
        identity: &SessionExecutionIdentity,
        incarnation: u64,
        host_started_at: u64,
        child_pid: u32,
        child_started_at: u64,
    ) -> Self {
        Self {
            host_started_at: Some(host_started_at),
            child_pid: Some(child_pid),
            child_started_at: Some(child_started_at),
            ..Self::for_execution(status, identity, incarnation)
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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let tmp_path = dir.join(format!(
            ".{}.tmp-{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("runtime.json"),
            Uuid::new_v4()
        ));

        {
            let mut tmp = std::fs::File::create(&tmp_path)?;
            tmp.write_all(content.as_bytes())?;
            tmp.write_all(b"\n")?;
            tmp.sync_all()?;
        }

        if let Err(error) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(error);
        }
        sync_parent_dir(dir)
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

pub fn active_launch_handshake_path(sessions_dir: &Path, session_id: &str) -> PathBuf {
    sessions_dir
        .join("active-launch-handshakes")
        .join(format!("{session_id}.json"))
}

pub fn manual_handoff_path(sessions_dir: &Path, session_id: &str) -> PathBuf {
    sessions_dir
        .join("manual-handoffs")
        .join(format!("{session_id}.json"))
}

fn validate_active_launch_handshake(
    handshake: &SessionActiveLaunchHandshake,
    session_id: &str,
) -> io::Result<()> {
    let valid_phase = matches!(handshake.phase, SessionActiveLaunchPhase::PreSpawn)
        || matches!(
            handshake.phase,
            SessionActiveLaunchPhase::ChildSpawned {
                child_pid,
                child_started_at,
            } if child_pid > 0 && child_started_at > 0
        );
    if handshake.schema_version != SessionActiveLaunchHandshake::CURRENT_SCHEMA_VERSION
        || handshake.nonce.trim().is_empty()
        || handshake.execution_identity.session_id != session_id
        || handshake.host_pid == 0
        || handshake.host_started_at == 0
        || !valid_phase
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "active launch handshake is invalid",
        ));
    }
    Ok(())
}

fn validate_manual_handoff(
    handoff: &SessionManualHandoffFence,
    session_id: &str,
) -> io::Result<()> {
    if handoff.schema_version != SessionManualHandoffFence::CURRENT_SCHEMA_VERSION
        || handoff.nonce.trim().is_empty()
        || handoff.execution_identity.session_id != session_id
        || handoff.host_pid == 0
        || handoff.host_started_at == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "manual Session handoff fence is invalid",
        ));
    }
    Ok(())
}

fn load_active_launch_handshake(
    sessions_dir: &Path,
    session_id: &str,
) -> io::Result<Option<SessionActiveLaunchHandshake>> {
    let path = active_launch_handshake_path(sessions_dir, session_id);
    match fs::read(&path) {
        Ok(bytes) => {
            let handshake: SessionActiveLaunchHandshake =
                serde_json::from_slice(&bytes).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("active launch handshake is malformed: {error}"),
                    )
                })?;
            validate_active_launch_handshake(&handshake, session_id)?;
            Ok(Some(handshake))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn load_manual_handoff(
    sessions_dir: &Path,
    session_id: &str,
) -> io::Result<Option<SessionManualHandoffFence>> {
    let path = manual_handoff_path(sessions_dir, session_id);
    match fs::read(&path) {
        Ok(bytes) => {
            let handoff: SessionManualHandoffFence =
                serde_json::from_slice(&bytes).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("manual Session handoff fence is malformed: {error}"),
                    )
                })?;
            validate_manual_handoff(&handoff, session_id)?;
            Ok(Some(handoff))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn save_json_fence(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Session fence has no parent"))?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("session-fence.json"),
        Uuid::new_v4()
    ));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    sync_parent_dir(parent)
}

fn save_active_launch_handshake(
    sessions_dir: &Path,
    handshake: &SessionActiveLaunchHandshake,
) -> io::Result<()> {
    let path = active_launch_handshake_path(sessions_dir, &handshake.execution_identity.session_id);
    let bytes = serde_json::to_vec_pretty(handshake)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    save_json_fence(&path, &bytes)
}

fn save_manual_handoff(sessions_dir: &Path, handoff: &SessionManualHandoffFence) -> io::Result<()> {
    let path = manual_handoff_path(sessions_dir, &handoff.execution_identity.session_id);
    let bytes = serde_json::to_vec_pretty(handoff)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    save_json_fence(&path, &bytes)
}

/// Create a cross-process Active launch fence while the caller holds the
/// exact Session lease. Existing or malformed evidence fails closed.
pub fn begin_session_active_launch_handshake_under_lease(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
    nonce: &str,
    host_started_at: u64,
) -> io::Result<Option<SessionActiveLaunchHandshake>> {
    require_current_thread_session_lease()?;
    if nonce.trim().is_empty()
        || host_started_at == 0
        || exact_durable_session_under_lease(sessions_dir, expected)?.is_none()
    {
        return Ok(None);
    }
    if load_active_launch_handshake(sessions_dir, &expected.session_id)?.is_some()
        || load_manual_handoff(sessions_dir, &expected.session_id)?.is_some()
    {
        return Ok(None);
    }
    let handshake = SessionActiveLaunchHandshake {
        schema_version: SessionActiveLaunchHandshake::CURRENT_SCHEMA_VERSION,
        nonce: nonce.to_string(),
        execution_identity: expected.clone(),
        host_pid: std::process::id(),
        host_started_at,
        phase: SessionActiveLaunchPhase::PreSpawn,
        created_at: Utc::now(),
    };
    save_active_launch_handshake(sessions_dir, &handshake)?;
    Ok(Some(handshake))
}

pub fn read_session_active_launch_handshake_under_lease(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
) -> io::Result<Option<SessionActiveLaunchHandshake>> {
    require_current_thread_session_lease()?;
    if exact_durable_session_under_lease(sessions_dir, expected)?.is_none() {
        return Ok(None);
    }
    match load_active_launch_handshake(sessions_dir, &expected.session_id)? {
        None => Ok(None),
        Some(handshake) if handshake.execution_identity == *expected => Ok(Some(handshake)),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "active launch handshake does not match the exact Session identity",
        )),
    }
}

/// Advance an exact Active launch marker from `PreSpawn` to `ChildSpawned`.
///
/// The complete previously-read marker is the CAS token. A stale/replaced
/// marker, changed durable Session, invalid child identity, or non-`PreSpawn`
/// phase returns `None` without rewriting the marker.
pub fn mark_session_active_launch_handshake_child_spawned_under_lease(
    sessions_dir: &Path,
    expected: &SessionActiveLaunchHandshake,
    child_pid: u32,
    child_started_at: u64,
) -> io::Result<Option<SessionActiveLaunchHandshake>> {
    require_current_thread_session_lease()?;
    if child_pid == 0
        || child_started_at == 0
        || expected.phase != SessionActiveLaunchPhase::PreSpawn
        || exact_durable_session_under_lease(sessions_dir, &expected.execution_identity)?.is_none()
    {
        return Ok(None);
    }
    let Some(current) =
        load_active_launch_handshake(sessions_dir, &expected.execution_identity.session_id)?
    else {
        return Ok(None);
    };
    if current != *expected {
        return Ok(None);
    }

    let mut updated = current;
    updated.phase = SessionActiveLaunchPhase::ChildSpawned {
        child_pid,
        child_started_at,
    };
    save_active_launch_handshake(sessions_dir, &updated)?;
    Ok(Some(updated))
}

pub fn session_active_launch_handshake_matches_under_lease(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
) -> io::Result<bool> {
    Ok(read_session_active_launch_handshake_under_lease(sessions_dir, expected)?.is_some())
}

/// Clear only the exact fence captured by the launch worker. A replacement
/// marker is never removed by a stale success/failure response.
pub fn clear_session_active_launch_handshake_under_lease(
    sessions_dir: &Path,
    expected: &SessionActiveLaunchHandshake,
) -> io::Result<bool> {
    require_current_thread_session_lease()?;
    if exact_durable_session_under_lease(sessions_dir, &expected.execution_identity)?.is_none() {
        return Ok(false);
    }
    let Some(current) =
        load_active_launch_handshake(sessions_dir, &expected.execution_identity.session_id)?
    else {
        return Ok(false);
    };
    if current != *expected {
        return Ok(false);
    }
    fs::remove_file(active_launch_handshake_path(
        sessions_dir,
        &expected.execution_identity.session_id,
    ))?;
    Ok(true)
}

/// Create one exact durable manual handoff fence while the Session lease is
/// held. Active launch and manual handoff fences are mutually exclusive.
pub fn begin_session_manual_handoff_under_lease(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
    nonce: &str,
    host_started_at: u64,
) -> io::Result<Option<SessionManualHandoffFence>> {
    require_current_thread_session_lease()?;
    if nonce.trim().is_empty()
        || host_started_at == 0
        || exact_durable_session_under_lease(sessions_dir, expected)?.is_none()
    {
        return Ok(None);
    }
    if load_active_launch_handshake(sessions_dir, &expected.session_id)?.is_some()
        || load_manual_handoff(sessions_dir, &expected.session_id)?.is_some()
    {
        return Ok(None);
    }
    let handoff = SessionManualHandoffFence {
        schema_version: SessionManualHandoffFence::CURRENT_SCHEMA_VERSION,
        nonce: nonce.to_string(),
        execution_identity: expected.clone(),
        host_pid: std::process::id(),
        host_started_at,
        created_at: Utc::now(),
    };
    save_manual_handoff(sessions_dir, &handoff)?;
    Ok(Some(handoff))
}

/// Read a manual handoff fence only for the exact durable producing Session.
/// Malformed or foreign evidence fails closed instead of being ignored.
pub fn read_session_manual_handoff_under_lease(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
) -> io::Result<Option<SessionManualHandoffFence>> {
    require_current_thread_session_lease()?;
    if exact_durable_session_under_lease(sessions_dir, expected)?.is_none() {
        return Ok(None);
    }
    match load_manual_handoff(sessions_dir, &expected.session_id)? {
        None => Ok(None),
        Some(handoff) if handoff.execution_identity == *expected => Ok(Some(handoff)),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "manual Session handoff fence does not match the exact Session identity",
        )),
    }
}

pub fn session_manual_handoff_matches_under_lease(
    sessions_dir: &Path,
    expected: &SessionManualHandoffFence,
) -> io::Result<bool> {
    Ok(
        read_session_manual_handoff_under_lease(sessions_dir, &expected.execution_identity)?
            .as_ref()
            == Some(expected),
    )
}

/// Clear only the exact manual handoff fence observed by the caller. A stale
/// completion cannot remove a replacement fence or evidence for a replaced
/// durable Session.
pub fn clear_session_manual_handoff_under_lease(
    sessions_dir: &Path,
    expected: &SessionManualHandoffFence,
) -> io::Result<bool> {
    if !session_manual_handoff_matches_under_lease(sessions_dir, expected)? {
        return Ok(false);
    }
    fs::remove_file(manual_handoff_path(
        sessions_dir,
        &expected.execution_identity.session_id,
    ))?;
    Ok(true)
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
    let runtime_path = runtime_state_path(sessions_dir, session_id);
    let mut runtime = SessionRuntimeState::new(status);
    if let Ok(previous) = SessionRuntimeState::load(&runtime_path) {
        runtime.execution_identity = previous.execution_identity;
        runtime.runtime_incarnation = previous.runtime_incarnation;
        runtime.host_started_at = previous.host_started_at;
        runtime.child_pid = previous.child_pid;
        runtime.child_started_at = previous.child_started_at;
    }
    runtime.save(&runtime_path)
}

fn validate_exact_runtime_proof_request(
    expected: &SessionExecutionIdentity,
    runtime_incarnation: u64,
) -> io::Result<bool> {
    validate_session_id_path_component(&expected.session_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if runtime_incarnation == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime incarnation must be non-zero",
        ));
    }
    Ok(expected.execution_binding.session_id == expected.session_id)
}

fn exact_durable_session_under_lease(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
) -> io::Result<Option<Session>> {
    require_current_thread_session_lease()?;
    let session = match inspect_session_path(&session_file_path(sessions_dir, &expected.session_id))
    {
        SessionPathState::Present(session) => *session,
        SessionPathState::Missing => return Ok(None),
        SessionPathState::Error(error) => return Err(error),
    };
    Ok((SessionExecutionIdentity::from_session(&session)
        .ok()
        .flatten()
        .as_ref()
        == Some(expected))
    .then_some(session))
}

fn require_current_thread_session_lease() -> io::Result<()> {
    if current_thread_holds_session_lease() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "exact runtime persistence requires an existing Session lease",
        ))
    }
}

fn persist_session_running_state_if_execution_identity_matches_under_lease(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
    runtime_incarnation: u64,
    host_started_at: u64,
    child_pid: u32,
    child_started_at: u64,
) -> io::Result<bool> {
    if !validate_exact_runtime_proof_request(expected, runtime_incarnation)?
        || host_started_at == 0
        || child_pid == 0
        || child_started_at == 0
    {
        return Ok(false);
    }
    if exact_durable_session_under_lease(sessions_dir, expected)?.is_none() {
        return Ok(false);
    }
    SessionRuntimeState::for_execution_process(
        AgentStatus::Running,
        expected,
        runtime_incarnation,
        host_started_at,
        child_pid,
        child_started_at,
    )
    .save(&runtime_state_path(sessions_dir, &expected.session_id))?;
    Ok(true)
}

/// Publish the exact producing Session and process-local PTY incarnation only
/// while the durable Session still matches the identity captured by launch.
/// Missing, invalid, or replaced Sessions return `false` without replacing an
/// existing runtime sidecar.
pub fn persist_session_running_state_if_execution_identity_matches(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
    runtime_incarnation: u64,
    host_started_at: u64,
    child_pid: u32,
    child_started_at: u64,
) -> io::Result<bool> {
    if !validate_exact_runtime_proof_request(expected, runtime_incarnation)?
        || host_started_at == 0
        || child_pid == 0
        || child_started_at == 0
    {
        return Ok(false);
    }
    with_session_path_lease(sessions_dir, &expected.session_id, |_| {
        persist_session_running_state_if_execution_identity_matches_under_lease(
            sessions_dir,
            expected,
            runtime_incarnation,
            host_started_at,
            child_pid,
            child_started_at,
        )
    })
}

fn validate_terminal_runtime_status(status: AgentStatus) -> io::Result<()> {
    if !matches!(status, AgentStatus::Stopped | AgentStatus::Interrupted) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "exact runtime terminal proof requires Stopped or Interrupted status",
        ));
    }
    Ok(())
}

/// Persist exact terminal process evidence while the caller already holds the
/// matching Session lease. Owner+Session coordinators use this primitive to
/// preserve their owner-before-Session lock order without recursively taking
/// the Session lock.
pub fn persist_session_terminal_status_if_execution_identity_matches_under_lease(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
    runtime_incarnation: u64,
    status: AgentStatus,
) -> io::Result<bool> {
    require_current_thread_session_lease()?;
    validate_terminal_runtime_status(status)?;
    if !validate_exact_runtime_proof_request(expected, runtime_incarnation)? {
        return Ok(false);
    }
    let Some(mut session) = exact_durable_session_under_lease(sessions_dir, expected)? else {
        return Ok(false);
    };

    session.update_status(status);
    let session_content = serialize_session_toml(&session)?;
    let runtime_path = runtime_state_path(sessions_dir, &expected.session_id);
    let mut runtime = SessionRuntimeState::for_execution(status, expected, runtime_incarnation);
    if let Ok(previous) = SessionRuntimeState::load(&runtime_path) {
        if previous.execution_identity.as_ref() == Some(expected)
            && previous.runtime_incarnation == Some(runtime_incarnation)
        {
            runtime.host_started_at = previous.host_started_at;
            runtime.child_pid = previous.child_pid;
            runtime.child_started_at = previous.child_started_at;
        }
    }
    write_session_toml_atomic(
        &session_file_path(sessions_dir, &expected.session_id),
        &session_content,
    )?;
    runtime.save(&runtime_path)?;
    Ok(true)
}

/// Persist terminal status for an exact runtime namespace while the caller
/// already holds the matching Session lease.
///
/// Unlike the process-local convenience API, recovery coordinators use the
/// explicit Host PID and incarnation captured by their liveness proof. The
/// remote sidecar must already match that proof; missing or replaced evidence
/// returns `false` without creating a terminal record in the caller's runtime
/// namespace.
pub fn persist_session_terminal_status_for_exact_runtime_under_lease(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
    runtime: ManualLaunchRuntimeProof,
    status: AgentStatus,
) -> io::Result<bool> {
    require_current_thread_session_lease()?;
    validate_terminal_runtime_status(status)?;
    if !validate_exact_runtime_proof_request(expected, runtime.runtime_incarnation)?
        || runtime.host_pid == 0
    {
        return Ok(false);
    }
    let Some(mut session) = exact_durable_session_under_lease(sessions_dir, expected)? else {
        return Ok(false);
    };
    let runtime_path =
        runtime_state_path_for_pid(sessions_dir, runtime.host_pid, &expected.session_id);
    let mut exact_runtime = match SessionRuntimeState::load(&runtime_path) {
        Ok(runtime) => runtime,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if exact_runtime.execution_identity.as_ref() != Some(expected)
        || exact_runtime.runtime_incarnation != Some(runtime.runtime_incarnation)
    {
        return Ok(false);
    }

    session.update_status(status);
    exact_runtime.status = status;
    exact_runtime.updated_at = Utc::now();
    let session_content = serialize_session_toml(&session)?;
    write_session_toml_atomic(
        &session_file_path(sessions_dir, &expected.session_id),
        &session_content,
    )?;
    exact_runtime.save(&runtime_path)?;
    Ok(true)
}

/// Persist terminal process evidence only while the durable Session still has
/// the exact producing identity observed by the exiting runtime.
///
/// Callers must invoke this only after confirming that exact PTY process has
/// exited. Missing, unbound, invalid, or replaced Sessions return `false`
/// without rewriting either the Session TOML or runtime sidecar.
pub fn persist_session_terminal_status_if_execution_identity_matches(
    sessions_dir: &Path,
    expected: &SessionExecutionIdentity,
    runtime_incarnation: u64,
    status: AgentStatus,
) -> io::Result<bool> {
    validate_terminal_runtime_status(status)?;
    if !validate_exact_runtime_proof_request(expected, runtime_incarnation)? {
        return Ok(false);
    }
    with_session_path_lease(sessions_dir, &expected.session_id, |_| {
        persist_session_terminal_status_if_execution_identity_matches_under_lease(
            sessions_dir,
            expected,
            runtime_incarnation,
            status,
        )
    })
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
    let agent_session_id = agent_session_id.trim();
    if agent_session_id.is_empty() {
        return Ok(());
    }

    update_session(sessions_dir, session_id, |session| {
        if session.agent_session_id.as_deref() == Some(agent_session_id) {
            return Ok(());
        }
        // Forward-only Session history: record each distinct conversation UUID the
        // first time we see it, before promoting it to the latest. Splits already
        // arrive via the SessionStart hook, so appending here (instead of
        // overwriting) is enough to reconstruct the full Session list under a Work.
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
        Ok(())
    })
    .map(|_| ())
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
        assert!(session.tool_runtime_provenance.is_none());
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
    fn tool_runtime_provenance_roundtrips_without_absolute_runner_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = Session::new("/tmp/wt", "feature/npx-plan", AgentId::Codex);
        session.tool_version = Some("latest".to_string());
        session.tool_runtime_provenance = Some(ToolRuntimeProvenance {
            schema_version: ToolRuntimeProvenance::CURRENT_SCHEMA_VERSION,
            official_package: "@openai/codex".to_string(),
            requested_selector: "latest".to_string(),
            resolved_exact_version: "0.116.0".to_string(),
            runner_kind: ToolRuntimeRunnerKind::Npx,
            resolution_reason: ToolRuntimeResolutionReason::RequestedSelector,
        });

        session.save(dir.path()).expect("save Session");
        let path = dir.path().join(format!("{}.toml", session.id));
        let persisted = std::fs::read_to_string(&path).expect("read Session");
        let loaded = Session::load(&path).expect("load Session");

        assert_eq!(
            loaded.tool_runtime_provenance,
            session.tool_runtime_provenance
        );
        assert!(persisted.contains("resolved_exact_version = \"0.116.0\""));
        assert!(!persisted.contains("C:\\\\Program Files\\\\nodejs\\\\npx.cmd"));
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
    fn runtime_state_legacy_json_defaults_execution_proof_to_none() {
        let legacy = r#"{
  "status": "Running",
  "updated_at": "2026-08-13T00:00:00Z",
  "last_activity_at": "2026-08-13T00:00:00Z",
  "source_event": "SessionStart"
}"#;

        let runtime: SessionRuntimeState =
            serde_json::from_str(legacy).expect("deserialize legacy runtime sidecar");

        assert!(runtime.execution_identity.is_none());
        assert!(runtime.runtime_incarnation.is_none());
        assert!(runtime.host_started_at.is_none());
    }

    #[test]
    fn runtime_state_for_execution_carries_exact_identity_and_incarnation() {
        let mut session = session_with_execution_owner();
        session
            .set_execution_binding(Some(test_session_execution_binding(&session)))
            .expect("bind Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("derive identity")
            .expect("bound identity");

        let runtime = SessionRuntimeState::for_execution(AgentStatus::Running, &identity, 17);

        assert_eq!(runtime.execution_identity.as_ref(), Some(&identity));
        assert_eq!(runtime.runtime_incarnation, Some(17));
    }

    fn save_bound_session_for_fence_test(
        sessions_dir: &Path,
    ) -> (Session, SessionExecutionIdentity) {
        let mut session = session_with_execution_owner();
        session
            .set_execution_binding(Some(test_session_execution_binding(&session)))
            .expect("bind Session");
        session.save(sessions_dir).expect("save bound Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("derive identity")
            .expect("bound identity");
        (session, identity)
    }

    #[test]
    fn manual_and_active_session_fences_are_mutually_exclusive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (session, identity) = save_bound_session_for_fence_test(dir.path());

        with_session_lease(dir.path(), &session.id, |_| {
            assert!(begin_session_active_launch_handshake_under_lease(
                dir.path(),
                &identity,
                "active-zero-start",
                0,
            )?
            .is_none());
            assert!(!active_launch_handshake_path(dir.path(), &session.id).exists());

            let manual =
                begin_session_manual_handoff_under_lease(dir.path(), &identity, "manual-one", 101)?
                    .expect("begin manual handoff");
            assert_eq!(manual.execution_identity, identity);
            assert_eq!(manual.host_pid, std::process::id());
            assert_eq!(manual.host_started_at, 101);
            assert!(begin_session_active_launch_handshake_under_lease(
                dir.path(),
                &identity,
                "active-blocked",
                102,
            )?
            .is_none());

            assert!(clear_session_manual_handoff_under_lease(
                dir.path(),
                &manual,
            )?);
            let active = begin_session_active_launch_handshake_under_lease(
                dir.path(),
                &identity,
                "active-one",
                103,
            )?
            .expect("begin Active launch handshake");
            assert_eq!(active.host_started_at, 103);
            assert!(begin_session_manual_handoff_under_lease(
                dir.path(),
                &identity,
                "manual-blocked",
                104,
            )?
            .is_none());
            Ok(())
        })
        .expect("exercise mutually exclusive fences");
    }

    #[test]
    fn active_launch_handshake_exact_read_fails_closed_for_replacement_and_malformed_evidence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (session, identity) = save_bound_session_for_fence_test(dir.path());
        let handshake = with_session_lease(dir.path(), &session.id, |_| {
            begin_session_active_launch_handshake_under_lease(
                dir.path(),
                &identity,
                "active-read",
                151,
            )?
            .ok_or_else(|| io::Error::other("Active handshake was not created"))
        })
        .expect("begin Active handshake");
        with_session_lease(dir.path(), &session.id, |_| {
            assert_eq!(
                read_session_active_launch_handshake_under_lease(dir.path(), &identity)?.as_ref(),
                Some(&handshake),
            );
            Ok(())
        })
        .expect("read exact Active handshake");

        let mut replacement = session.clone();
        replacement.agent_id = AgentId::Custom("replacement".to_string());
        replacement
            .save(dir.path())
            .expect("replace durable Session");
        let replacement_identity = SessionExecutionIdentity::from_session(&replacement)
            .expect("derive replacement identity")
            .expect("bound replacement identity");
        let path = active_launch_handshake_path(dir.path(), &session.id);
        let handshake_bytes = fs::read(&path).expect("read Active handshake");
        with_session_lease(dir.path(), &session.id, |_| {
            assert!(
                read_session_active_launch_handshake_under_lease(dir.path(), &identity)?.is_none()
            );
            let mismatch =
                read_session_active_launch_handshake_under_lease(dir.path(), &replacement_identity)
                    .expect_err("foreign Active handshake must fail closed");
            assert_eq!(mismatch.kind(), io::ErrorKind::InvalidData);
            Ok(())
        })
        .expect("reject replaced Active handshake");
        assert_eq!(
            fs::read(&path).expect("retain Active handshake"),
            handshake_bytes
        );

        fs::write(&path, b"{malformed").expect("install malformed Active handshake");
        let malformed_bytes = fs::read(&path).expect("read malformed Active handshake");
        with_session_lease(dir.path(), &session.id, |_| {
            let error =
                read_session_active_launch_handshake_under_lease(dir.path(), &replacement_identity)
                    .expect_err("malformed Active handshake must fail closed");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            Ok(())
        })
        .expect("inspect malformed Active handshake");
        assert_eq!(
            fs::read(&path).expect("retain malformed Active handshake"),
            malformed_bytes,
        );
    }

    #[test]
    fn active_launch_handshake_phase_cas_publishes_exact_child_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (session, identity) = save_bound_session_for_fence_test(dir.path());

        with_session_lease(dir.path(), &session.id, |_| {
            let pre_spawn = begin_session_active_launch_handshake_under_lease(
                dir.path(),
                &identity,
                "active-phase",
                161,
            )?
            .expect("begin Active handshake");
            assert_eq!(pre_spawn.phase, SessionActiveLaunchPhase::PreSpawn);

            let child_spawned = mark_session_active_launch_handshake_child_spawned_under_lease(
                dir.path(),
                &pre_spawn,
                401,
                402,
            )?
            .expect("publish exact spawned child");
            assert_eq!(
                child_spawned.phase,
                SessionActiveLaunchPhase::ChildSpawned {
                    child_pid: 401,
                    child_started_at: 402,
                }
            );
            assert_eq!(
                read_session_active_launch_handshake_under_lease(dir.path(), &identity)?.as_ref(),
                Some(&child_spawned)
            );

            let phase_bytes = fs::read(active_launch_handshake_path(dir.path(), &session.id))?;
            assert!(
                mark_session_active_launch_handshake_child_spawned_under_lease(
                    dir.path(),
                    &pre_spawn,
                    501,
                    502,
                )?
                .is_none(),
                "a stale pre-spawn CAS must not replace child identity"
            );
            assert_eq!(
                fs::read(active_launch_handshake_path(dir.path(), &session.id))?,
                phase_bytes,
            );
            Ok(())
        })
        .expect("advance Active handshake phase exactly once");
    }

    #[test]
    fn active_launch_handshake_phase_cas_rejects_invalid_child_without_rewrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (session, identity) = save_bound_session_for_fence_test(dir.path());

        with_session_lease(dir.path(), &session.id, |_| {
            let pre_spawn = begin_session_active_launch_handshake_under_lease(
                dir.path(),
                &identity,
                "active-invalid-child",
                162,
            )?
            .expect("begin Active handshake");
            let path = active_launch_handshake_path(dir.path(), &session.id);
            let bytes = fs::read(&path)?;

            assert!(
                mark_session_active_launch_handshake_child_spawned_under_lease(
                    dir.path(),
                    &pre_spawn,
                    0,
                    402,
                )?
                .is_none()
            );
            assert!(
                mark_session_active_launch_handshake_child_spawned_under_lease(
                    dir.path(),
                    &pre_spawn,
                    401,
                    0,
                )?
                .is_none()
            );
            assert_eq!(fs::read(path)?, bytes);
            Ok(())
        })
        .expect("reject invalid child identities");
    }

    #[test]
    fn active_launch_handshake_phase_cas_rejects_replacement_malformed_and_legacy_markers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (session, identity) = save_bound_session_for_fence_test(dir.path());
        let path = active_launch_handshake_path(dir.path(), &session.id);

        let stale = with_session_lease(dir.path(), &session.id, |_| {
            begin_session_active_launch_handshake_under_lease(
                dir.path(),
                &identity,
                "active-stale",
                171,
            )?
            .ok_or_else(|| io::Error::other("begin stale Active handshake"))
        })
        .expect("begin stale Active handshake");
        with_session_lease(dir.path(), &session.id, |_| {
            assert!(clear_session_active_launch_handshake_under_lease(
                dir.path(),
                &stale,
            )?);
            begin_session_active_launch_handshake_under_lease(
                dir.path(),
                &identity,
                "active-replacement",
                172,
            )?
            .ok_or_else(|| io::Error::other("begin replacement Active handshake"))
        })
        .expect("replace Active handshake");
        let replacement_bytes = fs::read(&path).expect("read replacement marker");
        with_session_lease(dir.path(), &session.id, |_| {
            assert!(
                mark_session_active_launch_handshake_child_spawned_under_lease(
                    dir.path(),
                    &stale,
                    601,
                    602,
                )?
                .is_none()
            );
            Ok(())
        })
        .expect("reject stale marker CAS");
        assert_eq!(
            fs::read(&path).expect("retain replacement"),
            replacement_bytes
        );

        fs::write(&path, b"{malformed").expect("install malformed marker");
        let malformed_bytes = fs::read(&path).expect("read malformed marker");
        with_session_lease(dir.path(), &session.id, |_| {
            let error = mark_session_active_launch_handshake_child_spawned_under_lease(
                dir.path(),
                &stale,
                701,
                702,
            )
            .expect_err("malformed marker must fail closed");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            Ok(())
        })
        .expect("inspect malformed marker");
        assert_eq!(
            fs::read(&path).expect("retain malformed marker"),
            malformed_bytes
        );

        let mut legacy = serde_json::to_value(&stale).expect("serialize legacy fixture");
        legacy["schema_version"] = serde_json::json!(1);
        legacy
            .as_object_mut()
            .expect("handshake object")
            .remove("phase");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&legacy).expect("serialize legacy marker"),
        )
        .expect("install legacy marker");
        let legacy_bytes = fs::read(&path).expect("read legacy marker");
        with_session_lease(dir.path(), &session.id, |_| {
            let error = read_session_active_launch_handshake_under_lease(dir.path(), &identity)
                .expect_err("legacy phase-less marker must fail closed");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            Ok(())
        })
        .expect("inspect legacy marker");
        assert_eq!(fs::read(&path).expect("retain legacy marker"), legacy_bytes);
    }

    #[test]
    fn manual_handoff_exact_read_match_and_stale_clear_preserve_replacement() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (session, identity) = save_bound_session_for_fence_test(dir.path());

        with_session_lease(dir.path(), &session.id, |_| {
            let stale = begin_session_manual_handoff_under_lease(
                dir.path(),
                &identity,
                "manual-stale",
                201,
            )?
            .expect("begin stale handoff");
            assert_eq!(
                read_session_manual_handoff_under_lease(dir.path(), &identity)?.as_ref(),
                Some(&stale),
            );
            assert!(session_manual_handoff_matches_under_lease(
                dir.path(),
                &stale,
            )?);
            assert!(clear_session_manual_handoff_under_lease(
                dir.path(),
                &stale,
            )?);

            let replacement = begin_session_manual_handoff_under_lease(
                dir.path(),
                &identity,
                "manual-replacement",
                202,
            )?
            .expect("begin replacement handoff");
            let path = manual_handoff_path(dir.path(), &session.id);
            let replacement_bytes = fs::read(&path)?;

            assert!(!session_manual_handoff_matches_under_lease(
                dir.path(),
                &stale,
            )?);
            assert!(!clear_session_manual_handoff_under_lease(
                dir.path(),
                &stale,
            )?);
            assert_eq!(fs::read(&path)?, replacement_bytes);
            assert_eq!(
                read_session_manual_handoff_under_lease(dir.path(), &identity)?.as_ref(),
                Some(&replacement),
            );
            Ok(())
        })
        .expect("exercise exact manual handoff APIs");
    }

    #[test]
    fn manual_handoff_replacement_and_malformed_evidence_are_zero_mutation_refusals() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (session, identity) = save_bound_session_for_fence_test(dir.path());
        let fence = with_session_lease(dir.path(), &session.id, |_| {
            begin_session_manual_handoff_under_lease(dir.path(), &identity, "manual-original", 301)?
                .ok_or_else(|| io::Error::other("manual fence was not created"))
        })
        .expect("begin manual handoff");
        let fence_path = manual_handoff_path(dir.path(), &session.id);

        let mut replacement = session.clone();
        replacement.agent_id = AgentId::Custom("replacement".to_string());
        replacement
            .save(dir.path())
            .expect("replace durable Session");
        let session_path = dir.path().join(format!("{}.toml", session.id));
        let session_bytes = fs::read(&session_path).expect("read replacement Session");
        let fence_bytes = fs::read(&fence_path).expect("read manual fence");
        let replacement_identity = SessionExecutionIdentity::from_session(&replacement)
            .expect("derive replacement identity")
            .expect("bound replacement identity");
        with_session_lease(dir.path(), &session.id, |_| {
            assert!(read_session_manual_handoff_under_lease(dir.path(), &identity)?.is_none());
            let mismatch =
                read_session_manual_handoff_under_lease(dir.path(), &replacement_identity)
                    .expect_err("foreign manual fence must fail closed");
            assert_eq!(mismatch.kind(), io::ErrorKind::InvalidData);
            assert!(!session_manual_handoff_matches_under_lease(
                dir.path(),
                &fence,
            )?);
            assert!(!clear_session_manual_handoff_under_lease(
                dir.path(),
                &fence,
            )?);
            assert!(begin_session_manual_handoff_under_lease(
                dir.path(),
                &identity,
                "manual-stale-session",
                302,
            )?
            .is_none());
            Ok(())
        })
        .expect("reject replaced Session");
        assert_eq!(
            fs::read(&session_path).expect("retain replacement"),
            session_bytes
        );
        assert_eq!(
            fs::read(&fence_path).expect("retain manual fence"),
            fence_bytes
        );

        fs::write(&fence_path, b"{malformed").expect("install malformed manual fence");
        let malformed_bytes = fs::read(&fence_path).expect("read malformed fence");
        with_session_lease(dir.path(), &session.id, |_| {
            let read_error =
                read_session_manual_handoff_under_lease(dir.path(), &replacement_identity)
                    .expect_err("malformed manual evidence must reject exact reads");
            assert_eq!(read_error.kind(), io::ErrorKind::InvalidData);
            let error = begin_session_active_launch_handshake_under_lease(
                dir.path(),
                &replacement_identity,
                "active-blocked-by-malformed-manual",
                303,
            )
            .expect_err("malformed manual evidence must fail closed");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            Ok(())
        })
        .expect("inspect malformed manual evidence");
        assert_eq!(
            fs::read(&fence_path).expect("retain malformed evidence"),
            malformed_bytes,
        );
        assert!(!active_launch_handshake_path(dir.path(), &session.id).exists());
    }

    #[test]
    fn exact_running_runtime_publish_survives_natural_status_persistence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = session_with_execution_owner();
        session
            .set_execution_binding(Some(test_session_execution_binding(&session)))
            .expect("bind Session");
        session.save(dir.path()).expect("save Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("derive identity")
            .expect("bound identity");

        assert!(persist_session_running_state_if_execution_identity_matches(
            dir.path(),
            &identity,
            23,
            101,
            std::process::id(),
            1,
        )
        .expect("publish exact Running runtime"));
        persist_session_status(dir.path(), &session.id, AgentStatus::Stopped)
            .expect("persist natural process exit");

        let runtime = SessionRuntimeState::load(&runtime_state_path(dir.path(), &session.id))
            .expect("load naturally stopped runtime sidecar");
        assert_eq!(runtime.status, AgentStatus::Stopped);
        assert_eq!(runtime.execution_identity.as_ref(), Some(&identity));
        assert_eq!(runtime.runtime_incarnation, Some(23));
        assert_eq!(runtime.host_started_at, Some(101));
        assert_eq!(runtime.child_pid, Some(std::process::id()));
        assert_eq!(runtime.child_started_at, Some(1));
    }

    #[test]
    fn exact_running_runtime_publish_rejects_replacement_without_sidecar_rewrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = session_with_execution_owner();
        session
            .set_execution_binding(Some(test_session_execution_binding(&session)))
            .expect("bind Session");
        session.save(dir.path()).expect("save Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("derive identity")
            .expect("bound identity");
        let runtime_path = runtime_state_path(dir.path(), &session.id);
        SessionRuntimeState::new(AgentStatus::Idle)
            .save(&runtime_path)
            .expect("save sentinel sidecar");
        let sentinel_bytes = fs::read(&runtime_path).expect("read sentinel sidecar");

        let mut replacement = session.clone();
        replacement.agent_id = AgentId::Custom("replacement".to_string());
        replacement.save(dir.path()).expect("save replacement");
        let replacement_path = dir.path().join(format!("{}.toml", session.id));
        let replacement_bytes = fs::read(&replacement_path).expect("read replacement Session");

        assert!(
            !persist_session_running_state_if_execution_identity_matches(
                dir.path(),
                &identity,
                24,
                102,
                std::process::id(),
                1,
            )
            .expect("reject replaced Session")
        );
        assert_eq!(
            fs::read(&runtime_path).expect("read retained sentinel sidecar"),
            sentinel_bytes,
        );
        assert_eq!(
            fs::read(&replacement_path).expect("read retained replacement Session"),
            replacement_bytes,
        );
    }

    #[test]
    fn exact_running_runtime_publish_rejects_missing_host_identity_without_sidecar_rewrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (session, identity) = save_bound_session_for_fence_test(dir.path());
        let runtime_path = runtime_state_path(dir.path(), &session.id);
        SessionRuntimeState::new(AgentStatus::Idle)
            .save(&runtime_path)
            .expect("save sentinel sidecar");
        let sentinel_bytes = fs::read(&runtime_path).expect("read sentinel sidecar");

        assert!(
            !persist_session_running_state_if_execution_identity_matches(
                dir.path(),
                &identity,
                25,
                0,
                std::process::id(),
                1,
            )
            .expect("reject missing Host start identity")
        );
        assert_eq!(
            fs::read(runtime_path).expect("retain sentinel sidecar"),
            sentinel_bytes
        );
    }

    #[test]
    fn exact_terminal_status_persistence_updates_session_and_sidecar() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = session_with_execution_owner();
        session
            .set_execution_binding(Some(test_session_execution_binding(&session)))
            .expect("bind Session");
        session.save(dir.path()).expect("save Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("derive identity")
            .expect("bound identity");

        assert!(persist_session_running_state_if_execution_identity_matches(
            dir.path(),
            &identity,
            29,
            103,
            std::process::id(),
            2,
        )
        .expect("publish exact Running proof"));
        assert!(
            persist_session_terminal_status_if_execution_identity_matches(
                dir.path(),
                &identity,
                29,
                AgentStatus::Stopped,
            )
            .expect("persist exact terminal proof")
        );

        let durable = Session::load(&dir.path().join(format!("{}.toml", session.id)))
            .expect("load updated Session");
        assert_eq!(durable.status, AgentStatus::Stopped);
        let runtime = SessionRuntimeState::load(&runtime_state_path(dir.path(), &session.id))
            .expect("load exact runtime sidecar");
        assert_eq!(runtime.status, AgentStatus::Stopped);
        assert_eq!(runtime.execution_identity.as_ref(), Some(&identity));
        assert_eq!(runtime.runtime_incarnation, Some(29));
        assert_eq!(runtime.host_started_at, Some(103));
        assert_eq!(runtime.child_pid, Some(std::process::id()));
        assert_eq!(runtime.child_started_at, Some(2));
    }

    #[test]
    fn exact_terminal_status_under_lease_requires_and_reuses_existing_session_lease() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = session_with_execution_owner();
        session
            .set_execution_binding(Some(test_session_execution_binding(&session)))
            .expect("bind Session");
        session.save(dir.path()).expect("save Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("derive identity")
            .expect("bound identity");

        let error = persist_session_terminal_status_if_execution_identity_matches_under_lease(
            dir.path(),
            &identity,
            31,
            AgentStatus::Stopped,
        )
        .expect_err("under-lease primitive must reject an unlocked caller");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);

        assert!(with_session_lease(dir.path(), &session.id, |_| {
            persist_session_terminal_status_if_execution_identity_matches_under_lease(
                dir.path(),
                &identity,
                31,
                AgentStatus::Stopped,
            )
        })
        .expect("reuse existing Session lease"));
        let runtime = SessionRuntimeState::load(&runtime_state_path(dir.path(), &session.id))
            .expect("load leased terminal proof");
        assert_eq!(runtime.status, AgentStatus::Stopped);
        assert_eq!(runtime.execution_identity.as_ref(), Some(&identity));
        assert_eq!(runtime.runtime_incarnation, Some(31));
    }

    #[test]
    fn exact_terminal_status_under_lease_updates_the_proven_remote_runtime_namespace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = session_with_execution_owner();
        session
            .set_execution_binding(Some(test_session_execution_binding(&session)))
            .expect("bind Session");
        session.save(dir.path()).expect("save Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("derive identity")
            .expect("bound identity");
        let proof = crate::ManualLaunchRuntimeProof {
            host_pid: u32::MAX - 7,
            runtime_incarnation: 37,
        };
        let remote_path = runtime_state_path_for_pid(dir.path(), proof.host_pid, &session.id);
        SessionRuntimeState::for_execution_process(
            AgentStatus::Running,
            &identity,
            proof.runtime_incarnation,
            101,
            u32::MAX - 8,
            103,
        )
        .save(&remote_path)
        .expect("save remote exact runtime");

        assert!(with_session_lease(dir.path(), &session.id, |_| {
            persist_session_terminal_status_for_exact_runtime_under_lease(
                dir.path(),
                &identity,
                proof,
                AgentStatus::Interrupted,
            )
        })
        .expect("reuse existing Session lease"));

        let runtime = SessionRuntimeState::load(&remote_path).expect("load remote terminal proof");
        assert_eq!(runtime.status, AgentStatus::Interrupted);
        assert_eq!(runtime.execution_identity.as_ref(), Some(&identity));
        assert_eq!(runtime.runtime_incarnation, Some(proof.runtime_incarnation));
        assert_eq!(runtime.host_started_at, Some(101));
        assert_eq!(runtime.child_pid, Some(u32::MAX - 8));
        assert_eq!(runtime.child_started_at, Some(103));
        assert!(!runtime_state_path(dir.path(), &session.id).exists());
    }

    #[test]
    fn exact_terminal_status_persistence_rejects_changed_or_missing_session_without_rewrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = session_with_execution_owner();
        session
            .set_execution_binding(Some(test_session_execution_binding(&session)))
            .expect("bind Session");
        session.save(dir.path()).expect("save Session");
        let identity = SessionExecutionIdentity::from_session(&session)
            .expect("derive identity")
            .expect("bound identity");
        let runtime_path = runtime_state_path(dir.path(), &session.id);
        SessionRuntimeState::new(AgentStatus::Running)
            .save(&runtime_path)
            .expect("save sentinel sidecar");

        let mut replacement = session.clone();
        replacement.agent_id = AgentId::Custom("replacement".to_string());
        replacement.save(dir.path()).expect("save replacement");
        let session_path = dir.path().join(format!("{}.toml", session.id));
        let replacement_bytes = fs::read(&session_path).expect("read replacement bytes");
        let sentinel_bytes = fs::read(&runtime_path).expect("read sentinel bytes");

        assert!(
            !persist_session_terminal_status_if_execution_identity_matches(
                dir.path(),
                &identity,
                30,
                AgentStatus::Stopped,
            )
            .expect("reject replacement")
        );
        assert_eq!(
            fs::read(&session_path).expect("read retained replacement"),
            replacement_bytes
        );
        assert_eq!(
            fs::read(&runtime_path).expect("read retained sidecar"),
            sentinel_bytes
        );

        fs::remove_file(&session_path).expect("remove Session");
        assert!(
            !persist_session_terminal_status_if_execution_identity_matches(
                dir.path(),
                &identity,
                31,
                AgentStatus::Interrupted,
            )
            .expect("reject missing Session")
        );
        assert!(!session_path.exists());
        assert_eq!(
            fs::read(runtime_path).expect("read sidecar after missing CAS"),
            sentinel_bytes
        );
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

    #[cfg(windows)]
    #[test]
    fn session_from_launch_config_does_not_persist_absolute_targeted_npx_runner() {
        for (agent_id, package) in [
            (AgentId::Codex, "@openai/codex"),
            (AgentId::ClaudeCode, "@anthropic-ai/claude-code"),
        ] {
            let mut config = crate::AgentLaunchBuilder::new(agent_id)
                .working_dir(r"C:\worktree")
                .branch("feature/npx-plan")
                .version("latest")
                .build();
            config.command = r"C:\Program Files\nodejs\npx.cmd".to_string();
            config.tool_runtime_provenance = Some(ToolRuntimeProvenance {
                schema_version: ToolRuntimeProvenance::CURRENT_SCHEMA_VERSION,
                official_package: package.to_string(),
                requested_selector: "latest".to_string(),
                resolved_exact_version: "0.122.0".to_string(),
                runner_kind: ToolRuntimeRunnerKind::Npx,
                resolution_reason: ToolRuntimeResolutionReason::RequestedSelector,
            });

            let session = Session::from_launch_config(r"C:\worktree", "feature/npx-plan", &config);
            let sessions_dir = tempfile::tempdir().expect("sessions dir");
            session.save(sessions_dir.path()).expect("persist Session");
            let persisted =
                std::fs::read_to_string(sessions_dir.path().join(format!("{}.toml", session.id)))
                    .expect("read persisted Session");

            assert_eq!(session.launch_command, "npx.cmd");
            assert_eq!(config.command, r"C:\Program Files\nodejs\npx.cmd");
            assert!(persisted.contains("launch_command = \"npx.cmd\""));
            assert!(!persisted.contains("Program Files"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn session_from_launch_config_preserves_absolute_runner_outside_targeted_provenance() {
        let absolute_npx = r"C:\Program Files\nodejs\npx.cmd";
        let mut without_provenance = crate::AgentLaunchBuilder::new(AgentId::Codex)
            .working_dir(r"C:\worktree")
            .branch("feature/npx-plan")
            .version("latest")
            .build();
        without_provenance.command = absolute_npx.to_string();

        let mut unrelated = crate::AgentLaunchBuilder::new(AgentId::Gemini)
            .working_dir(r"C:\worktree")
            .branch("feature/npx-plan")
            .version("latest")
            .build();
        unrelated.command = absolute_npx.to_string();
        unrelated.tool_runtime_provenance = Some(ToolRuntimeProvenance {
            schema_version: ToolRuntimeProvenance::CURRENT_SCHEMA_VERSION,
            official_package: "@google/gemini-cli".to_string(),
            requested_selector: "latest".to_string(),
            resolved_exact_version: "0.1.0".to_string(),
            runner_kind: ToolRuntimeRunnerKind::Npx,
            resolution_reason: ToolRuntimeResolutionReason::RequestedSelector,
        });

        let mut container = crate::AgentLaunchBuilder::new(AgentId::Codex)
            .working_dir(r"C:\worktree")
            .branch("feature/npx-plan")
            .version("latest")
            .build();
        container.command = absolute_npx.to_string();
        container.runtime_target = LaunchRuntimeTarget::Docker;
        container.tool_runtime_provenance = Some(ToolRuntimeProvenance {
            schema_version: ToolRuntimeProvenance::CURRENT_SCHEMA_VERSION,
            official_package: "@openai/codex".to_string(),
            requested_selector: "latest".to_string(),
            resolved_exact_version: "0.122.0".to_string(),
            runner_kind: ToolRuntimeRunnerKind::Npx,
            resolution_reason: ToolRuntimeResolutionReason::RequestedSelector,
        });

        for config in [&without_provenance, &unrelated, &container] {
            let session = Session::from_launch_config(r"C:\worktree", "feature/npx-plan", config);
            assert_eq!(session.launch_command, absolute_npx);
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn session_from_launch_config_preserves_non_windows_runner_path() {
        let mut config = crate::AgentLaunchBuilder::new(AgentId::Codex)
            .working_dir("/tmp/worktree")
            .branch("feature/npx-plan")
            .version("latest")
            .build();
        config.command = "/opt/node/bin/npx".to_string();
        config.tool_runtime_provenance = Some(ToolRuntimeProvenance {
            schema_version: ToolRuntimeProvenance::CURRENT_SCHEMA_VERSION,
            official_package: "@openai/codex".to_string(),
            requested_selector: "latest".to_string(),
            resolved_exact_version: "0.122.0".to_string(),
            runner_kind: ToolRuntimeRunnerKind::Npx,
            resolution_reason: ToolRuntimeResolutionReason::RequestedSelector,
        });

        let session = Session::from_launch_config("/tmp/worktree", "feature/npx-plan", &config);

        assert_eq!(session.launch_command, "/opt/node/bin/npx");
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
