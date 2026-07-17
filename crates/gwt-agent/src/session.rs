//! Agent session persistence: save/load sessions as TOML files.

use std::{
    fs::{self, File, OpenOptions},
    io,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    launch::{normalize_launch_args, LaunchConfig, RecoveryContinuationHandoff},
    types::{
        AgentId, AgentStatus, DockerLifecycleIntent, LaunchRuntimeTarget, SessionMode,
        WindowsShellKind, WorkflowBypass,
    },
};

/// Maximum serialized size of one durable Session ledger (4 MiB).
pub const MAX_SESSION_TOML_BYTES: u64 = 4 * 1024 * 1024;
/// A complete startup/import inventory may contain at most this many directory
/// entries. 4,096 is well above a normal local Session inventory while keeping
/// startup CPU and path allocation bounded under hostile local state.
pub const MAX_SESSION_DIRECTORY_ENTRIES: usize = 4_096;
/// Maximum Session TOML files accepted in one complete inventory.
pub const MAX_SESSION_TOML_FILES: usize = 2_048;
/// Maximum aggregate size of all Session TOMLs in one complete inventory.
/// 256 MiB permits thousands of ordinary ledgers without allowing the later
/// deserialize pass to consume multi-gigabyte CPU or memory.
pub const MAX_SESSION_TOML_AGGREGATE_BYTES: u64 = 256 * 1024 * 1024;

/// Shared actual-read budget for one complete Session inventory pass.
///
/// Each file is opened no-follow and consumed through that same handle. The
/// remaining aggregate bound is also supplied to the handle, so a file that is
/// swapped or grows during the read cannot bypass the inventory cap.
#[derive(Debug)]
pub struct SessionInventoryReadBudget {
    limit: u64,
    remaining: u64,
}

impl Default for SessionInventoryReadBudget {
    fn default() -> Self {
        Self::with_limit(MAX_SESSION_TOML_AGGREGATE_BYTES)
    }
}

impl SessionInventoryReadBudget {
    pub fn with_limit(limit: u64) -> Self {
        Self {
            limit,
            remaining: limit,
        }
    }

    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        let max_bytes = MAX_SESSION_TOML_BYTES.min(self.remaining);
        let description = if self.remaining < MAX_SESSION_TOML_BYTES {
            "aggregate Session TOML inventory"
        } else {
            "Session TOML inventory entry"
        };
        let file = gwt_core::bounded_file::BoundedRegularFile::open(path, max_bytes, description)?;
        self.read_opened(file)
    }

    fn read_opened(
        &mut self,
        file: gwt_core::bounded_file::BoundedRegularFile,
    ) -> io::Result<Vec<u8>> {
        let bytes = file.read_all()?;
        self.remaining = self
            .remaining
            .checked_sub(bytes.len() as u64)
            .ok_or_else(|| {
                session_inventory_limit_error("aggregate Session TOML bytes", self.limit)
            })?;
        Ok(bytes)
    }
}

/// Idle duration (in seconds) after which a session is considered stopped.
const IDLE_TIMEOUT_SECS: i64 = 60;
const CODEX_PLACEHOLDER_SESSION_ID: &str = "agent-session";

/// Environment variable injected into agent PTYs so hooks can identify the
/// backing gwt session.
pub const GWT_SESSION_ID_ENV: &str = "GWT_SESSION_ID";
pub const GWT_RECOVERY_ID_ENV: &str = "GWT_RECOVERY_ID";
/// Environment variable injected into agent PTYs so hooks can write the
/// matching runtime sidecar without discovering gwt paths on their own.
pub const GWT_SESSION_RUNTIME_PATH_ENV: &str = "GWT_SESSION_RUNTIME_PATH";
/// Environment variable injected into agent PTYs so skills can locate the
/// gwt binary for calling gwtd CLI (GitHub operations, etc.).
pub const GWT_BIN_PATH_ENV: &str = "GWT_BIN_PATH";
/// Loopback endpoint used by daemon-owned hook live events.
pub const GWT_HOOK_FORWARD_URL_ENV: &str = "GWT_HOOK_FORWARD_URL";
/// Bearer token paired with [`GWT_HOOK_FORWARD_URL_ENV`].
pub const GWT_HOOK_FORWARD_TOKEN_ENV: &str = "GWT_HOOK_FORWARD_TOKEN";

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

/// Durable launch progress used to distinguish a recoverable checkpoint from
/// a session whose provider process was never started or bound.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryLaunchStage {
    Created,
    Prepared,
    WorktreeMaterialized,
    SpawnRequested,
    ProcessSpawned,
    ProviderBound,
    Ready,
    /// Source recovery completed after the replacement provider reached Ready.
    Resolved,
    /// User explicitly discarded the source recovery in Recovery Center.
    Discarded,
}

impl RecoveryLaunchStage {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Resolved | Self::Discarded)
    }
}

fn recovery_launch_stage_rank(stage: RecoveryLaunchStage) -> u8 {
    match stage {
        RecoveryLaunchStage::Created => 0,
        RecoveryLaunchStage::Prepared => 1,
        RecoveryLaunchStage::WorktreeMaterialized => 2,
        RecoveryLaunchStage::SpawnRequested => 3,
        RecoveryLaunchStage::ProcessSpawned => 4,
        RecoveryLaunchStage::ProviderBound => 5,
        RecoveryLaunchStage::Ready => 6,
        RecoveryLaunchStage::Resolved | RecoveryLaunchStage::Discarded => 7,
    }
}

/// Exclusive recovery claim. A bounded lease prevents two windows from
/// restoring the same durable session concurrently while still allowing a
/// crashed claimant to be replaced after expiry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRecoveryLease {
    pub lease_id: String,
    pub holder_id: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Whether the captured provider conversation is the root conversation for
/// this gwt session or a child agent that must not replace the root binding.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRootRole {
    Root,
    Subagent,
}

/// Root-role evidence carried by a provider hook observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRootObservationRole {
    Root,
    Subagent,
    Ambiguous,
}

/// Strength of the persisted provider-session binding.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProviderBindingQuality {
    Inferred,
    Verified,
}

mod optional_session_kind_serde {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    use gwt_skills::SessionKind;

    pub fn serialize<S>(value: &Option<SessionKind>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(kind) => serializer.serialize_some(kind.as_env_str()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SessionKind>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<String>::deserialize(deserializer)?;
        value
            .map(|raw| match raw.trim().to_ascii_lowercase().as_str() {
                "intake" => Ok(SessionKind::Intake),
                "execution" => Ok(SessionKind::Execution),
                _ => Err(D::Error::custom(format!(
                    "unknown persisted session kind: {raw}"
                ))),
            })
            .transpose()
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
    /// Durable lane identity. `None` means a pre-schema-v4 session whose lane
    /// cannot be classified safely from the historical ledger alone.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_session_kind_serde"
    )]
    pub session_kind: Option<gwt_skills::SessionKind>,
    /// Whether this session owns a disposable detached Intake worktree.
    #[serde(default)]
    pub is_ephemeral: bool,
    /// Requested committish used to materialize the Intake worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_base_ref: Option<String>,
    /// Resolved commit captured at materialization time. This remains stable
    /// even when the requested ref advances before a later recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_base_oid: Option<String>,
    /// Stable identity for coordinating retries of this recovery record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_id: Option<String>,
    /// Durable source/successor handoff used to repair source retirement after
    /// a process crash that loses the in-memory window mapping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_continuation: Option<RecoveryContinuationHandoff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_launch_stage: Option<RecoveryLaunchStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_lease: Option<SessionRecoveryLease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_root_role: Option<ProviderRootRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_binding_quality: Option<ProviderBindingQuality>,
    /// Monotonic checkpoint counter for rejecting stale recovery writes.
    #[serde(default)]
    pub checkpoint_revision: u64,
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
    /// Distinguishes an explicit current-format restore decision from legacy
    /// ledgers where `restore_window_on_startup = false` only came from the
    /// serde default. This preserves old open-window compatibility without
    /// letting a surviving placeholder undo a current Stop action.
    #[serde(default)]
    pub startup_restore_intent_recorded: bool,
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
    #[serde(default)]
    pub source_event: Option<String>,
    #[serde(default)]
    pub pending_discussion: Option<PendingDiscussionResume>,
}

impl Session {
    /// Current persisted session schema version. SPEC-1921 Phase 53 / FR-066.
    /// Bump when adding a new migration in `migrate_legacy_launch_args` and
    /// ensure the new migration is idempotent relative to this value.
    pub const CURRENT_SCHEMA_VERSION: u32 = 4;

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
            session_kind: Some(gwt_skills::SessionKind::Execution),
            is_ephemeral: false,
            ephemeral_base_ref: None,
            launch_base_oid: None,
            recovery_id: Some(Uuid::new_v4().to_string()),
            recovery_continuation: None,
            recovery_launch_stage: Some(RecoveryLaunchStage::Created),
            recovery_lease: None,
            // Provider role is unknown until a bridge or structured hook
            // proves that the observed conversation is the launch root.
            provider_root_role: None,
            provider_binding_quality: None,
            checkpoint_revision: 0,
            skip_permissions: false,
            fast_mode: false,
            codex_fast_mode: false,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: DockerLifecycleIntent::Connect,
            linked_issue_number: None,
            workflow_bypass: None,
            workflow_bypass_armed_at: None,
            launch_command: String::new(),
            launch_args: Vec::new(),
            restore_window_on_startup: false,
            startup_restore_intent_recorded: true,
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
        if let Some(session_id) = config.recovery_retry_session_id.as_ref() {
            session.id = session_id.clone();
        }
        if let Some(created_at) = config.recovery_retry_created_at {
            session.created_at = created_at;
        }
        if let Some(continuation) = config.recovery_continuation.as_ref() {
            session.recovery_id = Some(continuation.target_recovery_id.clone());
        }
        session.recovery_continuation = config.recovery_continuation.clone();
        session.display_name = config.display_name.clone();
        session.tool_version = config.tool_version.clone();
        session.model = config.model.clone();
        session.reasoning_level = config.reasoning_level.clone();
        session.session_mode = config.session_mode;
        session.session_kind = Some(gwt_skills::SessionKind::from_is_ephemeral(
            config.is_ephemeral,
        ));
        session.is_ephemeral = config.is_ephemeral;
        session.ephemeral_base_ref = config.ephemeral_base_ref.clone();
        session.recovery_launch_stage = Some(RecoveryLaunchStage::Prepared);
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

    /// Advance the Session-side mirror of the recovery store launch stage.
    pub fn advance_recovery_launch_stage(
        &mut self,
        requested: RecoveryLaunchStage,
    ) -> io::Result<()> {
        if let Some(current) = self.recovery_launch_stage {
            if current.is_terminal() && current != requested {
                return Err(io::Error::other(format!(
                    "invalid recovery launch stage transition from {current:?} to {requested:?}"
                )));
            }
            if recovery_launch_stage_rank(requested) <= recovery_launch_stage_rank(current) {
                return Ok(());
            }
        }
        self.recovery_launch_stage = Some(requested);
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Record provider root evidence without allowing a child/root role flip.
    pub fn observe_provider_root_role(&mut self, role: ProviderRootRole) -> io::Result<()> {
        match self.provider_root_role {
            None => self.provider_root_role = Some(role),
            Some(current) if current == role => {}
            Some(current) => {
                return Err(io::Error::other(format!(
                    "provider root role conflicts: current {current:?}, observed {role:?}"
                )));
            }
        }
        self.updated_at = Utc::now();
        Ok(())
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
    /// fails. gwt deliberately does not read the agent tool's conversation store
    /// (no format coupling), so this only rejects ids that are structurally
    /// unusable; a handle that the agent CLI no longer has still launches and
    /// surfaces its own error.
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
        let content = serialize_session_toml(self)?;
        with_session_lock(dir, &self.id, || {
            write_session_toml_atomic(&session_file_path(dir, &self.id), &content)
        })
    }

    /// Deserialize a session from a TOML file verbatim. SPEC-1921 FR-066:
    /// `load` must not silently rewrite `launch_args`. Callers that need
    /// legacy migration applied should use [`Session::load_and_migrate`].
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let bytes = gwt_core::bounded_file::read_bounded_regular_file(
            path,
            MAX_SESSION_TOML_BYTES,
            "Session TOML",
        )?;
        Self::from_toml_bytes(&bytes)
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

    pub(crate) fn load_with_inventory_budget(
        path: &Path,
        budget: &mut SessionInventoryReadBudget,
    ) -> io::Result<Self> {
        let bytes = budget.read(path)?;
        Self::from_toml_bytes(&bytes)
    }

    fn from_toml_bytes(bytes: &[u8]) -> io::Result<Self> {
        let content = std::str::from_utf8(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut session: Self = toml::from_str(content)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        session.normalize_fast_mode_fields();
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

        if self.schema_version < 4 {
            // Schema 3 did not persist enough evidence to distinguish Intake
            // from Execution, verify a provider binding, or reconstruct an
            // exact detached base. Keep every new field at its serde default
            // instead of manufacturing recovery evidence from ambiguous data.
            self.schema_version = 4;
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

/// Return a complete, sorted inventory of Session TOML paths.
///
/// Enumeration errors and budget exhaustion are returned to the caller rather
/// than silently truncating the inventory. Startup recovery uses that signal to
/// suppress weak placeholder import and orphan pruning.
pub fn discover_session_toml_paths(sessions_dir: &Path) -> io::Result<Vec<PathBuf>> {
    discover_session_toml_paths_with_limits(
        sessions_dir,
        MAX_SESSION_DIRECTORY_ENTRIES,
        MAX_SESSION_TOML_FILES,
    )
}

pub(crate) fn discover_session_toml_paths_with_limits(
    sessions_dir: &Path,
    max_entries: usize,
    max_toml_files: usize,
) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(sessions_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut entry_count = 0_usize;
    let mut toml_count = 0_usize;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry?;
        entry_count = entry_count.saturating_add(1);
        if entry_count > max_entries {
            return Err(session_inventory_limit_error(
                "directory entries",
                max_entries as u64,
            ));
        }
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
        {
            continue;
        }
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Session inventory entry {} must be a regular file",
                    path.display()
                ),
            ));
        }
        toml_count = toml_count.saturating_add(1);
        if toml_count > max_toml_files {
            return Err(session_inventory_limit_error(
                "Session TOML files",
                max_toml_files as u64,
            ));
        }
        // Open through the same no-follow primitive used by the eventual
        // consumer. This rejects links and non-regular files during discovery;
        // actual bytes are independently bounded on their consuming handle.
        gwt_core::bounded_file::BoundedRegularFile::open(
            &path,
            MAX_SESSION_TOML_BYTES,
            "Session TOML inventory entry",
        )?;
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

fn session_inventory_limit_error(field: &str, limit: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Session inventory exceeds the {limit} {field} limit"),
    )
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

fn write_session_toml_atomic(path: &Path, content: &str) -> io::Result<()> {
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

    if cfg!(windows) && path.exists() {
        fs::remove_file(path)?;
    }

    if let Err(error) = fs::rename(&tmp_path, path) {
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
    with_session_lock(sessions_dir, session_id, || {
        let path = session_file_path(sessions_dir, session_id);
        let mut session = Session::load_and_migrate(&path)?;
        mutate(&mut session)?;
        let content = serialize_session_toml(&session)?;
        write_session_toml_atomic(&path, &content)?;
        Ok(session)
    })
}

/// Read the Session snapshot used to coordinate a following Recovery Store
/// mutation under the same per-Session lock as every writer.
///
/// Windows cannot replace an existing file through `std::fs::rename`, so the
/// atomic writer has a short remove/rename interval. An unlocked preliminary
/// read can otherwise observe that internal interval as a missing Session even
/// though the logical record is continuously owned by gwt.
fn load_session_for_coordinated_update(
    sessions_dir: &Path,
    session_id: &str,
) -> io::Result<Session> {
    with_session_lock(sessions_dir, session_id, || {
        Session::load_and_migrate(&session_file_path(sessions_dir, session_id))
    })
}

/// Inventory-aware variant used by startup scans. The Session is parsed from
/// the same handle charged to the shared aggregate budget while its normal
/// per-session lock is held, then persisted without a second content read.
pub fn update_session_with_inventory_budget<F>(
    sessions_dir: &Path,
    session_id: &str,
    budget: &mut SessionInventoryReadBudget,
    mutate: F,
) -> io::Result<Session>
where
    F: FnOnce(&mut Session) -> io::Result<()>,
{
    with_session_lock(sessions_dir, session_id, || {
        let path = session_file_path(sessions_dir, session_id);
        let mut session = Session::load_with_inventory_budget(&path, budget)?;
        session.migrate_legacy_launch_args();
        mutate(&mut session)?;
        let content = serialize_session_toml(&session)?;
        write_session_toml_atomic(&path, &content)?;
        Ok(session)
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
            source_event: None,
            pending_discussion: None,
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
            std::process::id()
        ));

        {
            let mut tmp = std::fs::File::create(&tmp_path)?;
            tmp.write_all(content.as_bytes())?;
            tmp.write_all(b"\n")?;
            tmp.sync_all()?;
        }

        if cfg!(windows) && path.exists() {
            std::fs::remove_file(path)?;
        }
        std::fs::rename(tmp_path, path)
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
    persist_agent_session_id_with_recovery_project_dir(
        sessions_dir,
        session_id,
        agent_session_id,
        None,
    )
}

/// Persist a provider session id only when the observation proves it is the
/// root conversation for the gwt launch.
pub fn persist_observed_provider_session_id(
    sessions_dir: &Path,
    session_id: &str,
    agent_session_id: &str,
    role: ProviderRootObservationRole,
) -> std::io::Result<()> {
    match role {
        ProviderRootObservationRole::Root => {
            persist_agent_session_id(sessions_dir, session_id, agent_session_id)?;
            // Root-role evidence belongs to the Session ledger even for a
            // legacy session that predates Recovery Store materialization.
            // When a recovery record exists, `persist_agent_session_id`
            // already bound the Store first and this observation is an
            // idempotent Session-side mirror.
            update_session(sessions_dir, session_id, |session| {
                session.observe_provider_root_role(ProviderRootRole::Root)
            })
            .map(|_| ())
        }
        ProviderRootObservationRole::Subagent | ProviderRootObservationRole::Ambiguous => Ok(()),
    }
}

/// Persist one successful provider/PTY launch boundary to the Recovery Store
/// first and then mirror it into the Session ledger.
pub fn persist_recovery_launch_stage(
    sessions_dir: &Path,
    session_id: &str,
    stage: RecoveryLaunchStage,
) -> std::io::Result<()> {
    persist_recovery_launch_stage_with_project_dir(sessions_dir, session_id, stage, None)
}

fn persist_recovery_launch_stage_with_project_dir(
    sessions_dir: &Path,
    session_id: &str,
    stage: RecoveryLaunchStage,
    recovery_project_dir: Option<&Path>,
) -> std::io::Result<()> {
    let before = load_session_for_coordinated_update(sessions_dir, session_id)?;
    if let (Some(recovery_id), Some(project_root)) = (
        before.recovery_id.as_deref(),
        before.project_state_root.as_deref(),
    ) {
        let project_dir = recovery_project_dir
            .map(Path::to_path_buf)
            .unwrap_or_else(|| gwt_core::paths::gwt_project_dir_for_repo_path(project_root));
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(project_dir);
        if store
            .load(recovery_id)
            .map_err(recovery_store_io_error)?
            .is_some()
        {
            let root_role = before.provider_root_role.map(|role| match role {
                ProviderRootRole::Root => gwt_core::recovery::ProviderRootRole::Root,
                ProviderRootRole::Subagent => gwt_core::recovery::ProviderRootRole::Subagent,
            });
            store
                .advance_launch_stage(
                    recovery_id,
                    core_recovery_launch_stage(stage),
                    root_role,
                    format!(
                        "launch-stage:{session_id}:{}",
                        recovery_launch_stage_name(stage)
                    ),
                )
                .map_err(recovery_store_io_error)?;
        }
    }
    update_session(sessions_dir, session_id, |session| {
        session.advance_recovery_launch_stage(stage)
    })
    .map(|_| ())
}

fn core_recovery_launch_stage(
    stage: RecoveryLaunchStage,
) -> gwt_core::recovery::RecoveryLaunchStage {
    match stage {
        RecoveryLaunchStage::Created => gwt_core::recovery::RecoveryLaunchStage::Created,
        RecoveryLaunchStage::Prepared => gwt_core::recovery::RecoveryLaunchStage::Prepared,
        RecoveryLaunchStage::WorktreeMaterialized => {
            gwt_core::recovery::RecoveryLaunchStage::WorktreeMaterialized
        }
        RecoveryLaunchStage::SpawnRequested => {
            gwt_core::recovery::RecoveryLaunchStage::SpawnRequested
        }
        RecoveryLaunchStage::ProcessSpawned => {
            gwt_core::recovery::RecoveryLaunchStage::ProcessSpawned
        }
        RecoveryLaunchStage::ProviderBound => {
            gwt_core::recovery::RecoveryLaunchStage::ProviderBound
        }
        RecoveryLaunchStage::Ready => gwt_core::recovery::RecoveryLaunchStage::Ready,
        RecoveryLaunchStage::Resolved => gwt_core::recovery::RecoveryLaunchStage::Resolved,
        RecoveryLaunchStage::Discarded => gwt_core::recovery::RecoveryLaunchStage::Discarded,
    }
}

fn recovery_launch_stage_name(stage: RecoveryLaunchStage) -> &'static str {
    match stage {
        RecoveryLaunchStage::Created => "created",
        RecoveryLaunchStage::Prepared => "prepared",
        RecoveryLaunchStage::WorktreeMaterialized => "worktree-materialized",
        RecoveryLaunchStage::SpawnRequested => "spawn-requested",
        RecoveryLaunchStage::ProcessSpawned => "process-spawned",
        RecoveryLaunchStage::ProviderBound => "provider-bound",
        RecoveryLaunchStage::Ready => "ready",
        RecoveryLaunchStage::Resolved => "resolved",
        RecoveryLaunchStage::Discarded => "discarded",
    }
}

fn persist_agent_session_id_with_recovery_project_dir(
    sessions_dir: &Path,
    session_id: &str,
    agent_session_id: &str,
    recovery_project_dir: Option<&Path>,
) -> std::io::Result<()> {
    let agent_session_id = agent_session_id.trim();
    if agent_session_id.is_empty() {
        return Ok(());
    }

    let before = load_session_for_coordinated_update(sessions_dir, session_id)?;
    let mut recovery_bound = false;
    if let (Some(recovery_id), Some(project_root)) = (
        before.recovery_id.as_deref(),
        before.project_state_root.as_deref(),
    ) {
        let project_dir = recovery_project_dir
            .map(Path::to_path_buf)
            .unwrap_or_else(|| gwt_core::paths::gwt_project_dir_for_repo_path(project_root));
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(project_dir);
        let recovery_exists = store
            .load(recovery_id)
            .map_err(recovery_store_io_error)?
            .is_some();
        if recovery_exists {
            match store.bind_root_semantic(
                recovery_id,
                agent_session_id,
                None,
                gwt_core::recovery::BindingQuality::Verified,
                format!("bind-provider-root:{agent_session_id}"),
            ) {
                Ok(_) => recovery_bound = true,
                Err(gwt_core::recovery::RecoveryStoreError::RootBindingConflict { .. }) => {
                    store
                        .set_lifecycle(
                            recovery_id,
                            gwt_core::recovery::RecoveryLifecycle::Attention,
                            Some("provider root changed after an exact binding".to_string()),
                            format!("root-conflict:{agent_session_id}"),
                        )
                        .map_err(recovery_store_io_error)?;
                }
                Err(error) => return Err(recovery_store_io_error(error)),
            }
        }
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
        if recovery_bound {
            session.observe_provider_root_role(ProviderRootRole::Root)?;
            session.provider_binding_quality = Some(ProviderBindingQuality::Verified);
            session.advance_recovery_launch_stage(RecoveryLaunchStage::ProviderBound)?;
        }
        Ok(())
    })
    .map(|_| ())
}

fn recovery_store_io_error(error: gwt_core::recovery::RecoveryStoreError) -> io::Error {
    io::Error::other(error.to_string())
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
        if session.restore_window_on_startup == restore && session.startup_restore_intent_recorded {
            return Ok(());
        }
        session.restore_window_on_startup = restore;
        session.startup_restore_intent_recorded = true;
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
    fn session_load_rejects_oversized_and_non_regular_toml() {
        let dir = tempfile::tempdir().unwrap();
        let oversized = dir.path().join("oversized.toml");
        File::create(&oversized)
            .unwrap()
            .set_len(MAX_SESSION_TOML_BYTES + 1)
            .unwrap();

        let oversized_error = Session::load(&oversized).unwrap_err();
        assert!(oversized_error.to_string().contains("size limit"));

        let directory = dir.path().join("directory.toml");
        fs::create_dir(&directory).unwrap();
        let directory_error = Session::load(&directory).unwrap_err();
        assert!(directory_error.to_string().contains("regular file"));
    }

    #[test]
    fn session_inventory_fails_closed_instead_of_truncating() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("one.toml"), b"one").unwrap();
        fs::write(dir.path().join("two.toml"), b"two").unwrap();
        fs::write(dir.path().join("unrelated.tmp"), b"three").unwrap();

        let entry_error = discover_session_toml_paths_with_limits(dir.path(), 2, 10)
            .expect_err("every directory entry must consume the inventory budget");
        assert!(entry_error.to_string().contains("directory entries"));

        let file_error = discover_session_toml_paths_with_limits(dir.path(), 10, 1)
            .expect_err("the Session file count must not be silently truncated");
        assert!(file_error.to_string().contains("Session TOML files"));

        let mut budget = SessionInventoryReadBudget::with_limit(5);
        assert_eq!(budget.read(&dir.path().join("one.toml")).unwrap(), b"one");
        let byte_error = budget
            .read(&dir.path().join("two.toml"))
            .expect_err("actual Session bytes must share one aggregate budget");
        assert!(byte_error.to_string().contains("aggregate Session TOML"));
    }

    #[test]
    fn session_inventory_actual_read_budget_survives_path_swap_and_growth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        let replacement = dir.path().join("replacement");
        fs::write(&path, b"old").unwrap();
        fs::write(&replacement, b"replacement bytes").unwrap();
        let opened = gwt_core::bounded_file::BoundedRegularFile::open(
            &path,
            5,
            "Session TOML inventory entry",
        )
        .unwrap();
        fs::remove_file(&path).unwrap();
        fs::rename(&replacement, &path).unwrap();
        let mut budget = SessionInventoryReadBudget::with_limit(5);
        assert_eq!(budget.read_opened(opened).unwrap(), b"old");
        assert_eq!(budget.remaining, 2);

        let growing = dir.path().join("growing.toml");
        fs::write(&growing, b"123").unwrap();
        let opened = gwt_core::bounded_file::BoundedRegularFile::open(
            &growing,
            5,
            "Session TOML inventory entry",
        )
        .unwrap();
        OpenOptions::new()
            .append(true)
            .open(&growing)
            .unwrap()
            .write_all(b"456")
            .unwrap();
        let mut budget = SessionInventoryReadBudget::with_limit(5);
        assert!(budget.read_opened(opened).is_err());
        assert_eq!(budget.remaining, 5, "failed reads must not consume budget");
    }

    #[cfg(unix)]
    #[test]
    fn session_load_and_inventory_reject_symlink_toml() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, b"not a session").unwrap();
        let link = dir.path().join("linked.toml");
        symlink(&target, &link).unwrap();

        assert!(Session::load(&link).is_err());
        assert!(discover_session_toml_paths(dir.path()).is_err());
    }

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
        assert_eq!(
            session.docker_lifecycle_intent,
            DockerLifecycleIntent::Connect
        );
        assert!(session.workflow_bypass.is_none());
        assert!(!session.restore_window_on_startup);
        assert!(session.startup_restore_intent_recorded);
        // SPEC-1921 FR-102: new sessions default to no backend override.
        assert!(session.backend_id.is_none());
        assert_eq!(
            session.session_kind,
            Some(gwt_skills::SessionKind::Execution)
        );
        assert!(!session.is_ephemeral);
        assert!(session.ephemeral_base_ref.is_none());
        assert!(session.launch_base_oid.is_none());
        assert!(session.recovery_id.is_some());
        assert_eq!(
            session.recovery_launch_stage,
            Some(RecoveryLaunchStage::Created)
        );
        assert!(session.recovery_lease.is_none());
        assert!(session.provider_root_role.is_none());
        assert!(session.provider_binding_quality.is_none());
        assert_eq!(session.checkpoint_revision, 0);
    }

    #[test]
    fn schema_four_recovery_metadata_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let mut session = Session::new("/tmp/wt", "", AgentId::Codex);
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.is_ephemeral = true;
        session.ephemeral_base_ref = Some("origin/develop".to_string());
        session.launch_base_oid = Some("1111111111111111111111111111111111111111".to_string());
        session.recovery_launch_stage = Some(RecoveryLaunchStage::ProviderBound);
        session.recovery_lease = Some(SessionRecoveryLease {
            lease_id: "lease-1".to_string(),
            holder_id: "window-1".to_string(),
            acquired_at: now,
            expires_at: now + chrono::Duration::minutes(5),
        });
        session.provider_root_role = Some(ProviderRootRole::Root);
        session.provider_binding_quality = Some(ProviderBindingQuality::Verified);
        session.checkpoint_revision = 7;

        session.save(dir.path()).unwrap();
        let loaded = Session::load(&dir.path().join(format!("{}.toml", session.id))).unwrap();

        assert_eq!(loaded.schema_version, Session::CURRENT_SCHEMA_VERSION);
        assert_eq!(loaded.session_kind, Some(gwt_skills::SessionKind::Intake));
        assert!(loaded.is_ephemeral);
        assert_eq!(loaded.ephemeral_base_ref.as_deref(), Some("origin/develop"));
        assert_eq!(loaded.launch_base_oid, session.launch_base_oid);
        assert_eq!(loaded.recovery_id, session.recovery_id);
        assert_eq!(
            loaded.recovery_launch_stage,
            Some(RecoveryLaunchStage::ProviderBound)
        );
        assert_eq!(loaded.recovery_lease, session.recovery_lease);
        assert_eq!(loaded.provider_root_role, Some(ProviderRootRole::Root));
        assert_eq!(
            loaded.provider_binding_quality,
            Some(ProviderBindingQuality::Verified)
        );
        assert_eq!(loaded.checkpoint_revision, 7);
    }

    #[test]
    fn schema_three_without_recovery_metadata_stays_unbound_when_migrated() {
        let legacy = r#"
id = "1d3d2d2d-3333-4444-5555-888888888888"
worktree_path = "/path/that/does/not/exist"
branch = "work"
agent_id = { type = "Codex" }
status = "Running"
launch_command = "codex"
launch_args = []
schema_version = 3
created_at = "2026-07-01T00:00:00Z"
updated_at = "2026-07-01T00:00:00Z"
last_activity_at = "2026-07-01T00:00:00Z"
display_name = "Codex"
"#;
        let mut session: Session = toml::from_str(legacy).expect("deserialize schema 3");

        assert!(session.session_kind.is_none());
        assert!(!session.is_ephemeral);
        assert!(session.recovery_id.is_none());
        assert!(session.provider_root_role.is_none());
        assert!(session.provider_binding_quality.is_none());
        assert_eq!(session.checkpoint_revision, 0);

        session.migrate_legacy_launch_args();

        assert_eq!(session.schema_version, Session::CURRENT_SCHEMA_VERSION);
        assert!(
            session.session_kind.is_none(),
            "legacy Intake-vs-Execution ambiguity must remain explicit"
        );
        assert!(session.recovery_id.is_none());
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
        assert!(!session.startup_restore_intent_recorded);
    }

    #[test]
    fn restore_window_on_startup_round_trips() {
        let mut session = Session::new("/tmp/wt", "main", AgentId::Codex);
        session.restore_window_on_startup = true;

        let serialized = toml::to_string(&session).expect("serialize");
        assert!(serialized.contains("restore_window_on_startup = true"));
        let parsed: Session = toml::from_str(&serialized).expect("deserialize");
        assert!(parsed.restore_window_on_startup);
        assert!(parsed.startup_restore_intent_recorded);
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

    #[test]
    fn persist_agent_session_id_binds_recovery_root_before_promoting_session() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("project-store");
        let worktree = dir.path().join("intake");
        std::fs::create_dir_all(&worktree).unwrap();
        let mut session = Session::new(&worktree, "work", AgentId::Codex);
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.is_ephemeral = true;
        session.project_state_root = Some(worktree.clone());
        let session_id = session.id.clone();
        let recovery_id = session.recovery_id.clone().unwrap();
        session.save(dir.path()).unwrap();
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&project_dir);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: recovery_id.clone(),
                    session_id: session_id.clone(),
                    repo_id: "repo".to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree,
                    launch_base_ref: Some("origin/develop".to_string()),
                    launch_base_oid: "1111111111111111111111111111111111111111".to_string(),
                    launch_head_oid: "1111111111111111111111111111111111111111".to_string(),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Investigate".to_string(),
                    created_at: session.created_at,
                },
                "create",
            )
            .unwrap();

        persist_agent_session_id_with_recovery_project_dir(
            dir.path(),
            &session_id,
            "provider-root",
            Some(&project_dir),
        )
        .unwrap();
        // Provider hooks are at-least-once. A repeated SessionStart for the
        // same root must reuse the semantic operation instead of conflicting
        // on a newly generated observation timestamp.
        persist_agent_session_id_with_recovery_project_dir(
            dir.path(),
            &session_id,
            "provider-root",
            Some(&project_dir),
        )
        .unwrap();

        let recovery = store.load(&recovery_id).unwrap().unwrap();
        assert_eq!(
            recovery
                .provider_root
                .as_ref()
                .map(|root| root.root_id.as_str()),
            Some("provider-root")
        );
        let loaded = Session::load(&dir.path().join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("provider-root"));
        assert_eq!(
            loaded.provider_binding_quality,
            Some(ProviderBindingQuality::Verified)
        );
        assert_eq!(
            loaded.recovery_launch_stage,
            Some(RecoveryLaunchStage::ProviderBound)
        );
    }

    #[test]
    fn persist_recovery_launch_stage_writes_store_before_session_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("project-store");
        let worktree = dir.path().join("intake");
        std::fs::create_dir_all(&worktree).unwrap();
        let mut session = Session::new(&worktree, "work", AgentId::Codex);
        session.project_state_root = Some(worktree.clone());
        let session_id = session.id.clone();
        let recovery_id = session.recovery_id.clone().unwrap();
        session.save(dir.path()).unwrap();
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&project_dir);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: recovery_id.clone(),
                    session_id: session_id.clone(),
                    repo_id: "repo".to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree,
                    launch_base_ref: Some("origin/develop".to_string()),
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "1".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Investigate".to_string(),
                    created_at: session.created_at,
                },
                "create-stage-test",
            )
            .unwrap();

        persist_recovery_launch_stage_with_project_dir(
            dir.path(),
            &session_id,
            RecoveryLaunchStage::SpawnRequested,
            Some(&project_dir),
        )
        .unwrap();
        assert_eq!(
            store.load(&recovery_id).unwrap().unwrap().launch_stage,
            gwt_core::recovery::RecoveryLaunchStage::SpawnRequested
        );
        assert_eq!(
            Session::load(&dir.path().join(format!("{session_id}.toml")))
                .unwrap()
                .recovery_launch_stage,
            Some(RecoveryLaunchStage::SpawnRequested)
        );

        persist_recovery_launch_stage_with_project_dir(
            dir.path(),
            &session_id,
            RecoveryLaunchStage::ProcessSpawned,
            Some(&project_dir),
        )
        .unwrap();
        assert_eq!(
            store.load(&recovery_id).unwrap().unwrap().launch_stage,
            gwt_core::recovery::RecoveryLaunchStage::ProcessSpawned
        );
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
        assert!(loaded.startup_restore_intent_recorded);

        persist_session_restore_window_on_startup(dir.path(), &session_id, false).unwrap();

        let loaded = Session::load(&dir.path().join(format!("{session_id}.toml"))).unwrap();
        assert!(!loaded.restore_window_on_startup);
        assert!(loaded.startup_restore_intent_recorded);
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
        config.is_ephemeral = true;
        config.ephemeral_base_ref = Some("origin/develop".to_string());
        let handoff = RecoveryContinuationHandoff {
            source_session_id: "source-session".to_string(),
            source_recovery_id: "source-recovery".to_string(),
            target_recovery_id: "target-recovery".to_string(),
            source_checkpoint_revision: 7,
            reason: "Recovery Center requested checkpoint continuation".to_string(),
            inherit_checkpoint: true,
        };
        config.recovery_continuation = Some(handoff.clone());

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
        assert_eq!(session.session_kind, Some(gwt_skills::SessionKind::Intake));
        assert!(session.is_ephemeral);
        assert_eq!(
            session.ephemeral_base_ref.as_deref(),
            Some("origin/develop")
        );
        assert_eq!(
            session.recovery_launch_stage,
            Some(RecoveryLaunchStage::Prepared)
        );
        assert_eq!(session.recovery_id.as_deref(), Some("target-recovery"));
        assert_eq!(session.recovery_continuation.as_ref(), Some(&handoff));
        let persisted = toml::to_string(&session).expect("serialize successor Session");
        let restored: Session = toml::from_str(&persisted).expect("restore successor Session");
        assert_eq!(restored.recovery_id.as_deref(), Some("target-recovery"));
        assert_eq!(restored.recovery_continuation, Some(handoff));
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
