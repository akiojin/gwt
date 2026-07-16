//! Host-wide cross-process coordinator for Project Index heavy jobs
//! (SPEC #1939 Phase 70, Issue #3264).
//!
//! Kernel file locks under `~/.gwt/runtime/index-coordinator/` are the single
//! source of truth for exclusion (FR-380). JSON tickets and state files exist
//! only for diagnostics and queue visibility; a stale or corrupt ticket must
//! never block a claimant that can take the kernel lock. Lock order is fixed:
//! target job -> host-wide heavy -> active generation (FR-392). The heavy
//! lease can only be acquired through an owned [`TargetJobGuard`] so the
//! order is enforced by construction.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Schema version stamped into every ticket / state JSON payload.
pub const COORDINATOR_SCHEMA_VERSION: u32 = 1;

const COORDINATOR_DIR_NAME: &str = "index-coordinator";

/// Coordinator root under an explicit gwt home (`<gwt_home>/runtime/index-coordinator`).
pub fn coordinator_root_from(gwt_home: &Path) -> PathBuf {
    gwt_home.join("runtime").join(COORDINATOR_DIR_NAME)
}

/// Coordinator root for the current process (`~/.gwt/runtime/index-coordinator`).
pub fn coordinator_root() -> PathBuf {
    crate::paths::gwt_runtime_dir().join(COORDINATOR_DIR_NAME)
}

/// Job target key (FR-382). Repo-shared scopes use `(repo_hash, scope)`;
/// worktree scopes add the worktree hash. Source fingerprints are not part
/// of the key: the owner re-reads the latest state after taking the lock so
/// same-target requests coalesce into one job.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetKey {
    repo_hash: String,
    scope: String,
    worktree_hash: Option<String>,
}

impl TargetKey {
    pub fn repo_shared(repo_hash: impl Into<String>, scope: impl Into<String>) -> Self {
        Self {
            repo_hash: repo_hash.into(),
            scope: scope.into(),
            worktree_hash: None,
        }
    }

    pub fn worktree(
        repo_hash: impl Into<String>,
        scope: impl Into<String>,
        worktree_hash: impl Into<String>,
    ) -> Self {
        Self {
            repo_hash: repo_hash.into(),
            scope: scope.into(),
            worktree_hash: Some(worktree_hash.into()),
        }
    }

    pub fn repo_hash(&self) -> &str {
        &self.repo_hash
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn worktree_hash(&self) -> Option<&str> {
        self.worktree_hash.as_deref()
    }

    /// Filesystem-safe stem used for lock / ticket / state file names.
    pub fn file_stem(&self) -> String {
        let mut stem = format!("{}--{}", sanitize(&self.repo_hash), sanitize(&self.scope));
        if let Some(worktree) = &self.worktree_hash {
            stem.push_str("--");
            stem.push_str(&sanitize(worktree));
        }
        stem
    }
}

fn sanitize(part: &str) -> String {
    part.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Job priority (FR-383): interactive search > manual rebuild > background
/// bootstrap / repair. Same priority is served in arrival order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobPriority {
    InteractiveSearch,
    ManualRebuild,
    Background,
}

impl JobPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            JobPriority::InteractiveSearch => "interactive-search",
            JobPriority::ManualRebuild => "manual-rebuild",
            JobPriority::Background => "background",
        }
    }
}

/// Owner identity recorded in tickets. `start_id` is a per-process token so a
/// recycled PID (FR-383) can be told apart from the original owner. The
/// kernel lock stays the exclusion truth; this is diagnostics input only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerIdentity {
    pub pid: u32,
    pub start_id: String,
}

impl OwnerIdentity {
    pub fn current() -> Self {
        use std::sync::OnceLock;
        static START_ID: OnceLock<String> = OnceLock::new();
        let start_id = START_ID.get_or_init(|| uuid::Uuid::new_v4().to_string());
        Self {
            pid: std::process::id(),
            start_id: start_id.clone(),
        }
    }
}

/// Diagnostic ticket persisted next to each kernel lock.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub schema_version: u32,
    pub target: String,
    pub priority: JobPriority,
    pub owner: OwnerIdentity,
    pub acquired_at_ms: u64,
}

/// Outcome of a shared job, as observed by the owner or a joined waiter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum JobOutcome {
    Completed,
    Failed { message: String },
    /// The owning process disappeared without publishing an outcome.
    OwnerGone,
}

#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("coordinator io error: {0}")]
    Io(#[from] io::Error),
    #[error("coordinator wait timed out after {waited_ms} ms")]
    Timeout { waited_ms: u64 },
    #[error("coordinator unavailable: {0}")]
    Unavailable(String),
}

/// Admission result for [`IndexCoordinator::request_job`].
pub enum JobAdmission {
    /// This caller owns the target job and may run the work.
    Owner(TargetJobGuard),
    /// Another live owner already runs the same target; wait on the shared
    /// outcome instead of starting a duplicate build.
    Joined(JobWaiter),
}

/// Host-wide coordinator handle rooted at `~/.gwt/runtime/index-coordinator/`.
pub struct IndexCoordinator {
    root: PathBuf,
}

impl IndexCoordinator {
    /// Open (and create when missing) the coordinator directory.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, CoordinatorError> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Open the default host-wide coordinator root.
    pub fn open_default() -> Result<Self, CoordinatorError> {
        Self::open(coordinator_root())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn target_lock_path(&self, key: &TargetKey) -> PathBuf {
        self.targets_dir().join(format!("{}.lock", key.file_stem()))
    }

    pub fn target_ticket_path(&self, key: &TargetKey) -> PathBuf {
        self.targets_dir()
            .join(format!("{}.ticket.json", key.file_stem()))
    }

    pub fn target_state_path(&self, key: &TargetKey) -> PathBuf {
        self.targets_dir()
            .join(format!("{}.state.json", key.file_stem()))
    }

    pub fn target_waiters_dir(&self, key: &TargetKey) -> PathBuf {
        self.targets_dir()
            .join(format!("{}.waiters", key.file_stem()))
    }

    pub fn heavy_lock_path(&self) -> PathBuf {
        self.root.join("heavy.lock")
    }

    pub fn heavy_ticket_path(&self) -> PathBuf {
        self.root.join("heavy.ticket.json")
    }

    pub fn heavy_pending_dir(&self) -> PathBuf {
        self.root.join("heavy.pending")
    }

    fn targets_dir(&self) -> PathBuf {
        self.root.join("targets")
    }

    /// Request the job slot for `key`. Returns [`JobAdmission::Owner`] when
    /// this caller takes the target kernel lock, or [`JobAdmission::Joined`]
    /// when a live owner already holds it. Stale tickets (dead PID, recycled
    /// PID with a different start id, crash before spawn) must not block
    /// admission: the kernel lock is the only truth.
    pub fn request_job(
        &self,
        _key: &TargetKey,
        _priority: JobPriority,
        _timeout: Duration,
    ) -> Result<JobAdmission, CoordinatorError> {
        Err(CoordinatorError::Unavailable(
            "index coordinator job admission is not implemented yet (T-IDX-384)".to_string(),
        ))
    }

    /// True when any caller with priority strictly higher than `than` is
    /// waiting for the heavy lease (FR-389: background claimants must not
    /// re-acquire while higher-priority tickets remain).
    pub fn pending_higher_priority(&self, _than: JobPriority) -> Result<bool, CoordinatorError> {
        Err(CoordinatorError::Unavailable(
            "index coordinator pending-priority scan is not implemented yet (T-IDX-384)"
                .to_string(),
        ))
    }
}

/// Exclusive owner of one target job (kernel target lock held).
pub struct TargetJobGuard {
    key: TargetKey,
    priority: JobPriority,
}

impl TargetJobGuard {
    pub fn key(&self) -> &TargetKey {
        &self.key
    }

    pub fn priority(&self) -> JobPriority {
        self.priority
    }

    /// Acquire the host-wide heavy lease (model-loading runner slot). Only
    /// reachable through an owned target guard, enforcing the fixed lock
    /// order target job -> heavy (FR-392).
    pub fn acquire_heavy(&self, _timeout: Duration) -> Result<HeavyLease, CoordinatorError> {
        Err(CoordinatorError::Unavailable(
            "index coordinator heavy lease is not implemented yet (T-IDX-384)".to_string(),
        ))
    }

    /// Number of live waiters currently joined to this target job.
    pub fn waiter_count(&self) -> Result<usize, CoordinatorError> {
        Err(CoordinatorError::Unavailable(
            "index coordinator waiter accounting is not implemented yet (T-IDX-384)".to_string(),
        ))
    }

    /// Publish the shared outcome and release the target job slot.
    pub fn complete(self, _outcome: JobOutcome) -> Result<(), CoordinatorError> {
        Err(CoordinatorError::Unavailable(
            "index coordinator completion is not implemented yet (T-IDX-384)".to_string(),
        ))
    }
}

/// Host-wide heavy lease. Dropping releases the kernel heavy lock.
pub struct HeavyLease {
    _private: (),
}

/// Waiter joined to another owner's target job. Dropping deregisters the
/// waiter; the shared job keeps running while other waiters remain (AS-8).
pub struct JobWaiter {
    key: TargetKey,
}

impl JobWaiter {
    pub fn key(&self) -> &TargetKey {
        &self.key
    }

    /// Wait for the shared outcome: `Completed` / `Failed` published by the
    /// owner, or `OwnerGone` when the owner vanished without publishing.
    pub fn wait(self, _timeout: Duration) -> Result<JobOutcome, CoordinatorError> {
        Err(CoordinatorError::Unavailable(
            "index coordinator waiter wait is not implemented yet (T-IDX-384)".to_string(),
        ))
    }
}
