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
//!
//! Liveness never relies on PID probing: waiters and pending heavy claimants
//! keep a kernel shared lock on their registration file, so a crashed
//! process is detected by `try_lock_exclusive` succeeding on its file. This
//! makes PID reuse (FR-383) harmless by design.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Schema version stamped into every ticket / state JSON payload.
pub const COORDINATOR_SCHEMA_VERSION: u32 = 1;

/// Scope reserved for heavy verification runs (SPEC #3576 FR-2b). Verification
/// claims the same host-wide heavy lease as index jobs: the contended resource
/// is host CPU, so a second exclusion mechanism would only add a lock ordering
/// problem.
pub const VERIFICATION_SCOPE: &str = "verification";
/// Issue #4086: how long a refused verification claimant stays pending as a
/// reservation with no live process behind it. The gwt-verify retry cadence
/// is 3 minutes, so a reservation only lapses once the agent has truly given
/// up; background index jobs defer to it the whole time (AC-1).
pub const VERIFICATION_RESERVATION_TTL: Duration = Duration::from_secs(10 * 60);
/// Issue #4086: TTL horizon on index-job heavy leases. The kernel lock still
/// decides exclusion; the TTL makes a runaway runner visible as `expired` in
/// status output instead of `expires_at_ms=unknown`.
pub const INDEX_HEAVY_LEASE_TTL: Duration = Duration::from_secs(2 * 60 * 60);
/// Issue #4169: how long a target keeps its place in the heavy queue after its
/// claimant stopped waiting. `verify.run` caps one in-process wait at 1500s and
/// answers `deferred`; the agent then reruns, and the rerun has to resume the
/// place the first attempt earned instead of joining at the back. Sized to
/// cover that gap — the gwt-verify retry cadence is 3 minutes — without letting
/// a worktree that walked away hold a place for the rest of the day.
pub const HEAVY_QUEUE_POSITION_TTL: Duration = Duration::from_secs(10 * 60);
const HEAVY_PROGRESS_FILE: &str = "heavy.progress.json";
const RESERVATION_PREFIX: &str = "reservation-";

const COORDINATOR_DIR_NAME: &str = "index-coordinator";
const LEASE_EVENT_LOG_NAME: &str = "lease-events.jsonl";
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// How long a lockable registration file is treated as "still being
/// registered" rather than crash residue.
const REGISTRATION_RESIDUE_GRACE: Duration = Duration::from_secs(60);

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

    /// Target key for a heavy verification run on one worktree (SPEC #3576).
    /// The scope is fixed so every verification claimant lands on the same
    /// job namespace regardless of caller.
    pub fn verification(repo_hash: impl Into<String>, worktree_hash: impl Into<String>) -> Self {
        Self::worktree(repo_hash, VERIFICATION_SCOPE, worktree_hash)
    }

    pub fn is_verification(&self) -> bool {
        self.scope == VERIFICATION_SCOPE
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
/// bootstrap / repair. Variant order defines the ranking (`Ord`: smaller is
/// higher priority). Same-priority claimants are served in poll order, which
/// approximates arrival order without a persistent queue.
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
    /// Lease identity, present on heavy leases only (SPEC #3576 FR-5). Absent
    /// on target-job tickets and on tickets written by older versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_id: Option<String>,
    /// TTL deadline (SPEC #3576 FR-2). `None` means the lease lives exactly as
    /// long as its holder, which is how index jobs have always behaved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

impl Ticket {
    fn job(target: String, priority: JobPriority, owner: OwnerIdentity) -> Self {
        Self {
            schema_version: COORDINATOR_SCHEMA_VERSION,
            target,
            priority,
            owner,
            acquired_at_ms: now_ms(),
            lease_id: None,
            expires_at_ms: None,
        }
    }
}

/// Lifecycle transition of a verification lease (SPEC #3576 FR-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LeaseEventKind {
    Acquired,
    Extended,
    Released,
    Expired,
}

impl LeaseEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LeaseEventKind::Acquired => "acquired",
            LeaseEventKind::Extended => "extended",
            LeaseEventKind::Released => "released",
            LeaseEventKind::Expired => "expired",
        }
    }
}

/// One appended line of `lease-events.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseEvent {
    pub schema_version: u32,
    pub at_ms: u64,
    pub lease_id: String,
    pub kind: LeaseEventKind,
    pub target: String,
    pub owner: OwnerIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Why a heavy-lease holder handed the host-wide lease back before its own
/// job finished (Issue #4140).
///
/// A long job that keeps the lease for its whole run starves every other
/// claimant, and heavy verification is the claimant that cannot route around
/// it: an agent that cannot verify cannot open a PR. Yielding trades the
/// exclusion guarantee for liveness, so it is a deliberate decision the
/// caller records rather than a silent release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeavyYieldReason {
    /// A claimant with strictly higher priority queued behind this holder.
    Preempted,
    /// The lease outlived its TTL while the job was still running.
    CapReached,
}

impl HeavyYieldReason {
    pub fn as_str(self) -> &'static str {
        match self {
            HeavyYieldReason::Preempted => "preempted",
            HeavyYieldReason::CapReached => "cap-reached",
        }
    }
}

/// Read-only view of the host-wide heavy lease (SPEC #3576 US-1). The kernel
/// lock decides `held`; the ticket only enriches a lock that is genuinely
/// taken, so crash residue can never be mistaken for a live holder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeavyLeaseStatus {
    pub held: bool,
    pub lease_id: Option<String>,
    pub target: Option<String>,
    pub owner: Option<OwnerIdentity>,
    pub priority: Option<JobPriority>,
    pub acquired_at_ms: Option<u64>,
    pub expires_at_ms: Option<u64>,
    /// Milliseconds left before the TTL lapses; `None` for leases without a
    /// TTL, `Some(0)` once an expired lease is still physically held.
    pub remaining_ms: Option<u64>,
    pub expired: bool,
    /// Live claimants queued behind the current holder, reservations included.
    pub pending: usize,
    /// Issue #4169 AC-2: those claimants in the order they will be served.
    pub queue: Vec<HeavyQueueEntry>,
    /// Issue #4086 AC-4: what kind of job holds the lease.
    pub holder_kind: Option<HeavyHolderKind>,
    /// Index holders only: batches left according to the runner's progress.
    pub remaining_batches: Option<u64>,
    /// Best-effort wait estimate: the TTL remainder for a verification
    /// holder, `remaining_batches × batch_ms` for an index holder.
    pub estimated_remaining_ms: Option<u64>,
}

/// Outcome of a shared job, as observed by the owner or a joined waiter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum JobOutcome {
    Completed,
    Failed {
        message: String,
    },
    /// The owning process disappeared without publishing an outcome.
    OwnerGone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum JobStatus {
    Running,
    Completed,
    Failed,
    Abandoned,
}

/// Per-target job state, published atomically for waiters and diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JobState {
    schema_version: u32,
    epoch: u64,
    status: JobStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    owner: OwnerIdentity,
    priority: JobPriority,
    updated_at_ms: u64,
}

/// Waiter / pending-heavy registration payload (diagnostics only; liveness
/// comes from the kernel shared lock each registrant keeps on its file).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Registration {
    schema_version: u32,
    owner: OwnerIdentity,
    priority: JobPriority,
    registered_at_ms: u64,
    /// Issue #4086: a reservation is a registration that stays live until
    /// this instant even though nobody holds its shared lock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reserved_until_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// Issue #4169: which target this claimant speaks for. The queue is
    /// ordered per target, not per attempt, so the same worktree retrying is
    /// one place in line rather than a new one each time. `None` on payloads
    /// written by older versions and on target-job waiter registrations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    /// Issue #4169: when this target first joined the heavy queue, carried
    /// forward across attempts. Ordering uses this, never `registered_at_ms`,
    /// which restarts on every attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queued_at_ms: Option<u64>,
    /// Issue #4169: how long the remembered place outlives the attempt that
    /// earned it. A remembered place never holds anybody up — it only decides
    /// where its own target rejoins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position_until_ms: Option<u64>,
}

impl Registration {
    /// When this target joined the queue. Payloads written before Issue #4169
    /// only know when their attempt started, which is the same thing for a
    /// claimant that never retried.
    fn queued_at(&self) -> u64 {
        self.queued_at_ms.unwrap_or(self.registered_at_ms)
    }

    /// Issue #4086: this claimant still holds a turn between its retries, with
    /// no process of its own behind it.
    fn has_live_reservation(&self, now: u64) -> bool {
        self.reserved_until_ms.is_some_and(|at| at > now)
    }

    /// Anything still worth keeping on disk: a live reservation, or a place
    /// this target may still come back for.
    fn outlives(&self, now: u64) -> bool {
        [self.reserved_until_ms, self.position_until_ms]
            .into_iter()
            .flatten()
            .any(|at| at > now)
    }
}

/// One claimant queued for the host-wide heavy lease (Issue #4169 AC-2).
/// `pending` only ever said how many were waiting; an agent deciding whether
/// to keep waiting needs to know who is in front of it and for how long.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeavyQueueEntry {
    /// Coordinator target stem of the claimant, e.g. `repo--verification--wt`.
    /// `None` for a registration written before Issue #4169.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub priority: JobPriority,
    /// When this target joined the queue, preserved across its retries.
    pub queued_at_ms: u64,
    /// How long it has been queued, as of the snapshot.
    pub waiting_ms: u64,
}

/// A claimant as the queue sees it, before it is published.
#[derive(Debug, Clone)]
struct QueueRecord {
    target: Option<String>,
    priority: JobPriority,
    queued_at_ms: u64,
    /// This claimant wants a turn: a process is polling for it, or a
    /// reservation (Issue #4086) stands in for it between retries. A record
    /// that is not waiting only remembers a place.
    waiting: bool,
    /// A process is polling for the lease right now.
    present: bool,
}

impl QueueRecord {
    /// Service order: priority first, then arrival, then target so the answer
    /// is total and every claimant computes the same one.
    fn rank(&self) -> (JobPriority, u64, Option<&str>) {
        (self.priority, self.queued_at_ms, self.target.as_deref())
    }

    /// Whether `me` has to leave a free lease alone because of this claimant.
    ///
    /// Live reservations keep their FIFO turn between retries. Their existing
    /// TTL bounds how long a departed claimant can hold up the next one.
    fn blocks(&self, me: &QueueRecord) -> bool {
        self.waiting && self.rank().cmp(&me.rank()) == Ordering::Less
    }

    fn published(&self, now: u64) -> HeavyQueueEntry {
        HeavyQueueEntry {
            target: self.target.clone(),
            priority: self.priority,
            queued_at_ms: self.queued_at_ms,
            waiting_ms: now.saturating_sub(self.queued_at_ms),
        }
    }
}

/// Who holds the host-wide heavy lease (Issue #4086 AC-4), derived from the
/// ticket target: verification targets carry the [`VERIFICATION_SCOPE`],
/// every other coordinator target is an index build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeavyHolderKind {
    Verification,
    Index,
    /// The lock is taken but no ticket describes the holder.
    Other,
}

impl HeavyHolderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            HeavyHolderKind::Verification => "verification",
            HeavyHolderKind::Index => "index",
            HeavyHolderKind::Other => "other",
        }
    }

    fn of_target(target: &str) -> Self {
        match target.split("--").nth(1) {
            Some(scope) if scope == VERIFICATION_SCOPE => HeavyHolderKind::Verification,
            Some(_) => HeavyHolderKind::Index,
            None => HeavyHolderKind::Other,
        }
    }
}

/// Batch progress an index runner publishes next to the heavy ticket
/// (Issue #4086 AC-4) so a refused claimant can estimate the wait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeavyProgress {
    /// Coordinator target stem of the running job, e.g. `repo--issues`.
    pub target: String,
    pub done: u64,
    pub total: u64,
    pub batch_size: u64,
    /// Wall-clock milliseconds of the most recent batch.
    pub batch_ms: u64,
    pub updated_at_ms: u64,
}

impl HeavyProgress {
    /// Batches still to run, rounding a partial batch up.
    pub fn remaining_batches(&self) -> u64 {
        let remaining = self.total.saturating_sub(self.done);
        let batch = self.batch_size.max(1);
        remaining.div_ceil(batch)
    }
}

/// A durable pending claim on the heavy lease (Issue #4086).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeavyReservation {
    pub target: String,
    pub priority: JobPriority,
    pub expires_at_ms: u64,
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
        fs::create_dir_all(root.join("targets"))?;
        fs::create_dir_all(root.join("heavy.pending"))?;
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
        target_lock_path(&self.root, key)
    }

    pub fn target_ticket_path(&self, key: &TargetKey) -> PathBuf {
        target_ticket_path(&self.root, key)
    }

    pub fn target_state_path(&self, key: &TargetKey) -> PathBuf {
        target_state_path(&self.root, key)
    }

    pub fn target_waiters_dir(&self, key: &TargetKey) -> PathBuf {
        target_waiters_dir(&self.root, key)
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

    /// Request the job slot for `key`. Returns [`JobAdmission::Owner`] when
    /// this caller takes the target kernel lock, or [`JobAdmission::Joined`]
    /// when a live owner already holds it. Stale tickets (dead PID, recycled
    /// PID with a different start id, crash before spawn) never block
    /// admission: the kernel lock is the only truth.
    pub fn request_job(
        &self,
        key: &TargetKey,
        priority: JobPriority,
        timeout: Duration,
    ) -> Result<JobAdmission, CoordinatorError> {
        fs::create_dir_all(self.root.join("targets"))?;
        let started = Instant::now();
        let lock_path = self.target_lock_path(key);
        loop {
            let lock_file = open_lock_file(&lock_path)?;
            match fs2::FileExt::try_lock_exclusive(&lock_file) {
                Ok(()) => {
                    let state_path = self.target_state_path(key);
                    let epoch = read_state(&state_path).map(|s| s.epoch).unwrap_or(0) + 1;
                    let owner = OwnerIdentity::current();
                    write_json_atomic(
                        &self.target_ticket_path(key),
                        &Ticket::job(key.file_stem(), priority, owner.clone()),
                    )?;
                    write_json_atomic(
                        &state_path,
                        &JobState {
                            schema_version: COORDINATOR_SCHEMA_VERSION,
                            epoch,
                            status: JobStatus::Running,
                            message: None,
                            owner,
                            priority,
                            updated_at_ms: now_ms(),
                        },
                    )?;
                    return Ok(JobAdmission::Owner(TargetJobGuard {
                        root: self.root.clone(),
                        key: key.clone(),
                        priority,
                        epoch,
                        _lock_file: lock_file,
                        completed: false,
                    }));
                }
                Err(err) if is_contended(&err) => {
                    // A live owner holds the target: join as a waiter. The
                    // owner may release between our probe and the join; the
                    // waiter's wait loop resolves that through the same
                    // kernel-lock probe.
                    let state = read_state(&self.target_state_path(key));
                    let joined_epoch = match &state {
                        Some(s) if s.status == JobStatus::Running => s.epoch,
                        Some(s) => s.epoch + 1,
                        None => 1,
                    };
                    let waiters_dir = self.target_waiters_dir(key);
                    fs::create_dir_all(&waiters_dir)?;
                    let waiter_path = waiters_dir.join(format!("{}.json", uuid::Uuid::new_v4()));
                    // Write the payload BEFORE taking the liveness lock: a
                    // Windows shared lock denies writes through the owning
                    // handle too. The sweep leaves any registration younger
                    // than `REGISTRATION_RESIDUE_GRACE` alone, so the window
                    // between the write and the lock is never mistaken for
                    // crash residue.
                    let waiter_file = open_lock_file(&waiter_path)?;
                    let registration = Registration {
                        schema_version: COORDINATOR_SCHEMA_VERSION,
                        owner: OwnerIdentity::current(),
                        priority,
                        registered_at_ms: now_ms(),
                        reserved_until_ms: None,
                        reason: None,
                        // A target-job waiter queues for one target's build,
                        // not for the host-wide heavy lease.
                        target: None,
                        queued_at_ms: None,
                        position_until_ms: None,
                    };
                    {
                        let mut handle = &waiter_file;
                        handle
                            .write_all(&serde_json::to_vec(&registration).map_err(io_invalid)?)?;
                        handle.flush()?;
                    }
                    waiter_file.lock_shared()?;
                    return Ok(JobAdmission::Joined(JobWaiter {
                        state_path: self.target_state_path(key),
                        lock_path,
                        waiter_path,
                        _waiter_file: waiter_file,
                        key: key.clone(),
                        joined_epoch,
                    }));
                }
                Err(err) => {
                    if started.elapsed() >= timeout {
                        return Err(CoordinatorError::Io(err));
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
            }
        }
    }

    /// True when any live caller with priority strictly higher than `than`
    /// is waiting for the heavy lease (FR-389: background claimants must not
    /// re-acquire while higher-priority tickets remain).
    pub fn pending_higher_priority(&self, than: JobPriority) -> Result<bool, CoordinatorError> {
        waiting_above(&self.heavy_pending_dir(), than)
    }

    /// Snapshot the host-wide heavy lease without joining the queue
    /// (SPEC #3576 US-1). Probing the kernel lock — not the ticket — decides
    /// whether the lease is held, so a ticket left behind by a crashed holder
    /// reads as free.
    pub fn heavy_lease_status(&self) -> Result<HeavyLeaseStatus, CoordinatorError> {
        let queue = published_heavy_queue(&self.heavy_pending_dir())?;
        let pending = queue.len();
        let probe = open_lock_file(&self.heavy_lock_path())?;
        match fs2::FileExt::try_lock_exclusive(&probe) {
            Ok(()) => {
                let _ = fs2::FileExt::unlock(&probe);
                return Ok(HeavyLeaseStatus {
                    pending,
                    queue,
                    ..HeavyLeaseStatus::default()
                });
            }
            Err(err) if is_contended(&err) => {}
            Err(err) => return Err(CoordinatorError::Io(err)),
        }
        let Some(ticket) = read_ticket(&self.heavy_ticket_path()) else {
            return Ok(HeavyLeaseStatus {
                held: true,
                pending,
                queue,
                ..HeavyLeaseStatus::default()
            });
        };
        let now = now_ms();
        let remaining_ms = ticket.expires_at_ms.map(|at| at.saturating_sub(now));
        let holder_kind = HeavyHolderKind::of_target(&ticket.target);
        let progress = match holder_kind {
            HeavyHolderKind::Index => self.read_heavy_progress().filter(|progress| {
                progress.target == ticket.target && progress.updated_at_ms >= ticket.acquired_at_ms
            }),
            _ => None,
        };
        let remaining_batches = progress.as_ref().map(HeavyProgress::remaining_batches);
        let estimated_remaining_ms = match holder_kind {
            HeavyHolderKind::Verification => remaining_ms,
            HeavyHolderKind::Index => progress.as_ref().map(|progress| {
                progress
                    .remaining_batches()
                    .saturating_mul(progress.batch_ms)
            }),
            HeavyHolderKind::Other => None,
        };
        Ok(HeavyLeaseStatus {
            held: true,
            lease_id: ticket.lease_id,
            target: Some(ticket.target),
            owner: Some(ticket.owner),
            priority: Some(ticket.priority),
            acquired_at_ms: Some(ticket.acquired_at_ms),
            expires_at_ms: ticket.expires_at_ms,
            remaining_ms,
            expired: ticket.expires_at_ms.is_some_and(|at| now >= at),
            pending,
            queue,
            holder_kind: Some(holder_kind),
            remaining_batches,
            estimated_remaining_ms,
        })
    }

    /// Reservation file for `key` under `heavy.pending/` (Issue #4086). One
    /// per claimant target, so a retry refreshes rather than duplicates it.
    pub fn heavy_reservation_path(&self, key: &TargetKey) -> PathBuf {
        heavy_queue_entry_path(&self.heavy_pending_dir(), &key.file_stem())
    }

    /// Leave a durable pending claim for `key` (Issue #4086 AC-1). Unlike a
    /// live registration it needs no process to stay behind it: background
    /// claimants defer to it until `ttl` lapses or the reserving target is
    /// granted the lease, whichever comes first.
    pub fn reserve_heavy(
        &self,
        key: &TargetKey,
        priority: JobPriority,
        ttl: Duration,
        reason: Option<&str>,
    ) -> Result<HeavyReservation, CoordinatorError> {
        let expires_at_ms = now_ms().saturating_add(ttl.as_millis() as u64);
        let path = self.heavy_reservation_path(key);
        // Issue #4169: reserving is the same claimant coming back, so it keeps
        // the place its earlier attempt earned rather than rejoining at the
        // back. Reserving never opens a remembered place of its own — that is
        // the enrolling claimant's business — so a reservation with nothing
        // behind it still lapses the moment its own TTL does.
        let mut registration = heavy_queue_entry(&path, &key.file_stem(), priority);
        registration.reserved_until_ms = Some(expires_at_ms);
        registration.reason = reason.map(str::to_string).or(registration.reason);
        write_json_atomic(&path, &registration)?;
        Ok(HeavyReservation {
            target: key.file_stem(),
            priority,
            expires_at_ms,
        })
    }

    /// Drop the reservation for `key`. Returns whether one existed.
    pub fn clear_heavy_reservation(&self, key: &TargetKey) -> Result<bool, CoordinatorError> {
        match fs::remove_file(self.heavy_reservation_path(key)) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(CoordinatorError::Io(err)),
        }
    }

    /// Publish (or refresh) the running index job's batch progress
    /// (Issue #4086 AC-4). The Python runner writes the same file directly.
    pub fn write_heavy_progress(&self, progress: &HeavyProgress) -> Result<(), CoordinatorError> {
        write_json_atomic(&self.root.join(HEAVY_PROGRESS_FILE), progress)?;
        Ok(())
    }

    fn read_heavy_progress(&self) -> Option<HeavyProgress> {
        let raw = fs::read(self.root.join(HEAVY_PROGRESS_FILE)).ok()?;
        serde_json::from_slice(&raw).ok()
    }

    /// Every recorded verification lease transition, oldest first
    /// (SPEC #3576 FR-5). Unparsable lines are skipped so one torn append can
    /// never hide the rest of the ledger.
    pub fn lease_events(&self) -> Result<Vec<LeaseEvent>, CoordinatorError> {
        let raw = match fs::read_to_string(self.lease_event_log_path()) {
            Ok(raw) => raw,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(CoordinatorError::Io(err)),
        };
        Ok(raw
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect())
    }

    pub fn lease_event_log_path(&self) -> PathBuf {
        self.root.join(LEASE_EVENT_LOG_NAME)
    }
}

/// Exclusive owner of one target job (kernel target lock held).
pub struct TargetJobGuard {
    root: PathBuf,
    key: TargetKey,
    priority: JobPriority,
    epoch: u64,
    _lock_file: File,
    completed: bool,
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
    /// order target job -> heavy (FR-392). Non-interactive claimants defer
    /// while live higher-priority claimants are pending (FR-383).
    pub fn acquire_heavy(&self, timeout: Duration) -> Result<HeavyLease, CoordinatorError> {
        self.acquire_heavy_inner(timeout, None)
    }

    /// Acquire the heavy lease with a TTL (SPEC #3576 FR-2). The TTL bounds
    /// how long crash residue can matter; it never interrupts a running
    /// holder, which keeps AC-Y3 (finish the command, then release) true.
    pub fn acquire_heavy_with_ttl(
        &self,
        timeout: Duration,
        ttl: Duration,
    ) -> Result<HeavyLease, CoordinatorError> {
        self.acquire_heavy_inner(timeout, Some(ttl))
    }

    fn acquire_heavy_inner(
        &self,
        timeout: Duration,
        ttl: Option<Duration>,
    ) -> Result<HeavyLease, CoordinatorError> {
        let pending_dir = self.root.join("heavy.pending");
        fs::create_dir_all(&pending_dir)?;
        // Issue #4169: join the queue before probing the lock. Taking the lock
        // first would be the whole bug — a claimant that never queued would win
        // the instant the lease frees, while everyone who did queue is asleep
        // between polls.
        let target = self.key.file_stem();
        let queued_at_ms = enroll_in_heavy_queue(&pending_dir, &target, self.priority);
        let me = QueueRecord {
            target: Some(target.clone()),
            priority: self.priority,
            queued_at_ms,
            waiting: true,
            present: true,
        };
        let pending_path = pending_dir.join(format!("{}.json", uuid::Uuid::new_v4()));
        let pending_file = open_lock_file(&pending_path)?;
        let registration = Registration {
            schema_version: COORDINATOR_SCHEMA_VERSION,
            owner: OwnerIdentity::current(),
            priority: self.priority,
            registered_at_ms: now_ms(),
            reserved_until_ms: None,
            reason: None,
            target: Some(target.clone()),
            queued_at_ms: Some(queued_at_ms),
            position_until_ms: None,
        };
        // Payload first, liveness lock second — see the waiter registration
        // above for why a Windows shared lock cannot come first.
        {
            let mut handle = &pending_file;
            handle.write_all(&serde_json::to_vec(&registration).map_err(io_invalid)?)?;
            handle.flush()?;
        }
        pending_file.lock_shared()?;
        let cleanup_pending = |file: File, path: &Path| {
            drop(file);
            let _ = fs::remove_file(path);
        };

        let started = Instant::now();
        let heavy_lock_path = self.root.join("heavy.lock");
        let heavy_file = match open_lock_file(&heavy_lock_path) {
            Ok(file) => file,
            Err(err) => {
                cleanup_pending(pending_file, &pending_path);
                return Err(CoordinatorError::Io(err));
            }
        };
        loop {
            let must_defer = heavy_queue(&pending_dir)
                .unwrap_or_default()
                .iter()
                .any(|other| other.target != me.target && other.blocks(&me));
            if !must_defer {
                match fs2::FileExt::try_lock_exclusive(&heavy_file) {
                    Ok(()) => {
                        let acquired_at_ms = now_ms();
                        let ticket = Ticket {
                            schema_version: COORDINATOR_SCHEMA_VERSION,
                            target: target.clone(),
                            priority: self.priority,
                            owner: OwnerIdentity::current(),
                            acquired_at_ms,
                            lease_id: Some(uuid::Uuid::new_v4().to_string()),
                            expires_at_ms: ttl
                                .map(|ttl| acquired_at_ms.saturating_add(ttl.as_millis() as u64)),
                        };
                        let _ = write_json_atomic(&self.root.join("heavy.ticket.json"), &ticket);
                        cleanup_pending(pending_file, &pending_path);
                        // The grant consumes the claimant's queue entry — its
                        // reservation (Issue #4086) and its place (Issue
                        // #4169) alike: its turn has come.
                        let _ = fs::remove_file(heavy_queue_entry_path(&pending_dir, &target));
                        let lease = HeavyLease {
                            _lock_file: heavy_file,
                            root: self.root.clone(),
                            ticket_path: self.root.join("heavy.ticket.json"),
                            // Only verification leases keep a ledger: index
                            // jobs run on the hot search path and gain
                            // nothing from an extra append per acquisition.
                            records_events: self.key.is_verification(),
                            ticket,
                            released: false,
                        };
                        lease.record_event(LeaseEventKind::Acquired, None);
                        return Ok(lease);
                    }
                    Err(err) if is_contended(&err) => {}
                    Err(err) => {
                        cleanup_pending(pending_file, &pending_path);
                        return Err(CoordinatorError::Io(err));
                    }
                }
            }
            if started.elapsed() >= timeout {
                // Issue #4169: the attempt ends, the place does not. A caller
                // that answers `deferred` and reruns resumes from here instead
                // of queueing again behind everyone who arrived meanwhile.
                let _ = enroll_in_heavy_queue(&pending_dir, &target, self.priority);
                if self.key.is_verification() {
                    // Keep the claim live before removing this poll's registration.
                    // Both verify.run and manual acquire retry after this returns.
                    let path = heavy_queue_entry_path(&pending_dir, &target);
                    let mut entry = heavy_queue_entry(&path, &target, self.priority);
                    entry.reserved_until_ms = Some(
                        now_ms().saturating_add(VERIFICATION_RESERVATION_TTL.as_millis() as u64),
                    );
                    let _ = write_json_atomic(&path, &entry);
                }
                cleanup_pending(pending_file, &pending_path);
                return Err(CoordinatorError::Timeout {
                    waited_ms: started.elapsed().as_millis() as u64,
                });
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Number of live waiters currently joined to this target job. Stale
    /// registrations (crashed waiters) are swept while counting.
    pub fn waiter_count(&self) -> Result<usize, CoordinatorError> {
        let waiters_dir = target_waiters_dir(&self.root, &self.key);
        Ok(sweep_live_registrations(&waiters_dir)?.len())
    }

    /// Publish the shared outcome and release the target job slot.
    pub fn complete(mut self, outcome: JobOutcome) -> Result<(), CoordinatorError> {
        let (status, message) = match outcome {
            JobOutcome::Completed => (JobStatus::Completed, None),
            JobOutcome::Failed { message } => (JobStatus::Failed, Some(message)),
            JobOutcome::OwnerGone => (JobStatus::Abandoned, None),
        };
        self.publish_state(status, message)?;
        let _ = fs::remove_file(target_ticket_path(&self.root, &self.key));
        self.completed = true;
        Ok(())
    }

    fn publish_state(
        &self,
        status: JobStatus,
        message: Option<String>,
    ) -> Result<(), CoordinatorError> {
        write_json_atomic(
            &target_state_path(&self.root, &self.key),
            &JobState {
                schema_version: COORDINATOR_SCHEMA_VERSION,
                epoch: self.epoch,
                status,
                message,
                owner: OwnerIdentity::current(),
                priority: self.priority,
                updated_at_ms: now_ms(),
            },
        )?;
        Ok(())
    }
}

impl Drop for TargetJobGuard {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.publish_state(JobStatus::Abandoned, None);
            let _ = fs::remove_file(target_ticket_path(&self.root, &self.key));
        }
        // Kernel target lock releases when `_lock_file` drops.
    }
}

/// Host-wide heavy lease. Dropping releases the kernel heavy lock, so a
/// crashed or killed holder never blocks the next claimant regardless of TTL
/// (T-IDX-383 / SPEC #3576 T-006).
pub struct HeavyLease {
    _lock_file: File,
    root: PathBuf,
    ticket_path: PathBuf,
    ticket: Ticket,
    records_events: bool,
    released: bool,
}

impl HeavyLease {
    /// Lease identity carried in the ticket and in every recorded event.
    pub fn id(&self) -> &str {
        self.ticket.lease_id.as_deref().unwrap_or_default()
    }

    pub fn target(&self) -> &str {
        &self.ticket.target
    }

    /// The published diagnostic ticket.
    pub fn ticket(&self) -> &Ticket {
        &self.ticket
    }

    pub fn acquired_at_ms(&self) -> u64 {
        self.ticket.acquired_at_ms
    }

    pub fn expires_at_ms(&self) -> Option<u64> {
        self.ticket.expires_at_ms
    }

    /// Time left before the TTL lapses, saturating at zero. `None` for leases
    /// taken without a TTL.
    pub fn remaining(&self) -> Option<Duration> {
        self.ticket
            .expires_at_ms
            .map(|at| Duration::from_millis(at.saturating_sub(now_ms())))
    }

    pub fn is_expired(&self) -> bool {
        self.ticket.expires_at_ms.is_some_and(|at| now_ms() >= at)
    }

    /// Push the TTL deadline out by `ttl` from now (FR-2). The republished
    /// ticket is what other processes read through
    /// [`IndexCoordinator::heavy_lease_status`].
    pub fn extend(&mut self, ttl: Duration) -> Result<(), CoordinatorError> {
        self.extend_until(now_ms().saturating_add(ttl.as_millis() as u64))
    }

    /// Move the TTL deadline to an absolute epoch-millisecond instant. The
    /// holder is the only writer of its own ticket, so out-of-process extend
    /// requests are applied through this method rather than by rewriting the
    /// ticket behind the holder's back.
    pub fn extend_until(&mut self, expires_at_ms: u64) -> Result<(), CoordinatorError> {
        self.ticket.expires_at_ms = Some(expires_at_ms);
        write_json_atomic(&self.ticket_path, &self.ticket)?;
        self.record_event(LeaseEventKind::Extended, None);
        Ok(())
    }

    /// Hold this lease for as long as `job_running` stays true, handing it
    /// back early when the TTL lapses or a strictly higher-priority claimant
    /// queues behind it (Issue #4140).
    ///
    /// `acquire_heavy_inner`'s priority check only runs *before* the lock is
    /// taken, so a holder that parks on a long child process is invisible to
    /// it — that is how a background index job kept heavy verification out for
    /// half an hour. This is the holding-side half of the same rule, and it
    /// mirrors the cooperative `BuildStep::Yielded` loop that in-process index
    /// builds already use.
    ///
    /// The lease is always released by the time this returns: `None` means the
    /// job finished first, `Some(reason)` means the job is still running
    /// without the lease. Nothing here interrupts the job — AC-Y3 (finish the
    /// command, then release) stays true, and yielding only gives up the
    /// exclusion, not the work.
    pub fn hold_while(
        self,
        poll: Duration,
        job_running: impl Fn() -> bool,
    ) -> Option<HeavyYieldReason> {
        let pending_dir = self.root.join("heavy.pending");
        let priority = self.ticket.priority;
        loop {
            if !job_running() {
                return None;
            }
            if self.is_expired() {
                return Some(HeavyYieldReason::CapReached);
            }
            if waiting_above(&pending_dir, priority).unwrap_or(false) {
                return Some(HeavyYieldReason::Preempted);
            }
            std::thread::sleep(poll);
        }
    }

    /// Release the lease explicitly, recording whether it ran past its TTL.
    /// Dropping does the same thing; this form surfaces I/O errors.
    pub fn release(mut self) -> Result<(), CoordinatorError> {
        self.settle();
        Ok(())
    }

    fn settle(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        if self.is_expired() {
            self.record_event(LeaseEventKind::Expired, Some("ttl elapsed"));
        } else {
            self.record_event(LeaseEventKind::Released, None);
        }
        let _ = fs::remove_file(&self.ticket_path);
    }

    fn record_event(&self, kind: LeaseEventKind, reason: Option<&str>) {
        if !self.records_events {
            return;
        }
        append_lease_event(
            &self.root,
            &LeaseEvent {
                schema_version: COORDINATOR_SCHEMA_VERSION,
                at_ms: now_ms(),
                lease_id: self.id().to_string(),
                kind,
                target: self.ticket.target.clone(),
                owner: self.ticket.owner.clone(),
                reason: reason.map(str::to_string),
            },
        );
    }
}

impl Drop for HeavyLease {
    fn drop(&mut self) {
        self.settle();
        // Kernel heavy lock releases when `_lock_file` drops.
    }
}

/// Waiter joined to another owner's target job. Dropping deregisters the
/// waiter; the shared job keeps running while other waiters remain (AS-8).
pub struct JobWaiter {
    state_path: PathBuf,
    lock_path: PathBuf,
    waiter_path: PathBuf,
    _waiter_file: File,
    key: TargetKey,
    joined_epoch: u64,
}

impl JobWaiter {
    pub fn key(&self) -> &TargetKey {
        &self.key
    }

    /// Wait for the shared outcome: `Completed` / `Failed` published by the
    /// owner, or `OwnerGone` when the owner vanished without publishing.
    pub fn wait(self, timeout: Duration) -> Result<JobOutcome, CoordinatorError> {
        let started = Instant::now();
        loop {
            if let Some(outcome) = self.published_outcome() {
                return Ok(outcome);
            }
            // Kernel-lock probe: if the target lock is free the owner either
            // finished (state re-read resolves it) or died without
            // publishing.
            let probe = open_lock_file(&self.lock_path)?;
            if fs2::FileExt::try_lock_exclusive(&probe).is_ok() {
                let outcome = self.published_outcome().unwrap_or(JobOutcome::OwnerGone);
                drop(probe);
                return Ok(outcome);
            }
            if started.elapsed() >= timeout {
                return Err(CoordinatorError::Timeout {
                    waited_ms: started.elapsed().as_millis() as u64,
                });
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn published_outcome(&self) -> Option<JobOutcome> {
        let state = read_state(&self.state_path)?;
        if state.epoch < self.joined_epoch {
            return None;
        }
        match state.status {
            JobStatus::Running => None,
            JobStatus::Completed => Some(JobOutcome::Completed),
            JobStatus::Failed => Some(JobOutcome::Failed {
                message: state.message.unwrap_or_default(),
            }),
            JobStatus::Abandoned => Some(JobOutcome::OwnerGone),
        }
    }
}

impl Drop for JobWaiter {
    fn drop(&mut self) {
        // Shared lock releases when `_waiter_file` drops; remove the
        // registration so owners stop counting this caller.
        let _ = fs::remove_file(&self.waiter_path);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn target_lock_path(root: &Path, key: &TargetKey) -> PathBuf {
    root.join("targets")
        .join(format!("{}.lock", key.file_stem()))
}

fn target_ticket_path(root: &Path, key: &TargetKey) -> PathBuf {
    root.join("targets")
        .join(format!("{}.ticket.json", key.file_stem()))
}

fn target_state_path(root: &Path, key: &TargetKey) -> PathBuf {
    root.join("targets")
        .join(format!("{}.state.json", key.file_stem()))
}

fn target_waiters_dir(root: &Path, key: &TargetKey) -> PathBuf {
    root.join("targets")
        .join(format!("{}.waiters", key.file_stem()))
}

fn open_lock_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
}

fn is_contended(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock
        || err.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

fn io_invalid(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Atomic JSON publish: write a sibling temp file, then rename over the
/// destination so readers never observe a torn payload.
fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "state".to_string()),
        std::process::id()
    ));
    let payload = serde_json::to_vec(value).map_err(io_invalid)?;
    fs::write(&tmp, payload)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn read_state(path: &Path) -> Option<JobState> {
    let raw = fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn read_ticket(path: &Path) -> Option<Ticket> {
    let raw = fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// Append one lease transition to `lease-events.jsonl` under an exclusive
/// kernel lock so concurrent holders never interleave a line. Recording is
/// diagnostics: a failure here must not fail the lease operation itself.
fn append_lease_event(root: &Path, event: &LeaseEvent) {
    let Ok(mut line) = serde_json::to_vec(event) else {
        return;
    };
    line.push(b'\n');
    // `read(true)` is not for reading: Windows `LockFileEx` refuses a handle
    // that only carries `FILE_APPEND_DATA` (ERROR_ACCESS_DENIED), so an
    // append-only ledger never recorded a single line there (Issue #4105).
    let Ok(file) = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(root.join(LEASE_EVENT_LOG_NAME))
    else {
        return;
    };
    if fs2::FileExt::lock_exclusive(&file).is_err() {
        return;
    }
    let mut handle = &file;
    let _ = handle.write_all(&line);
    let _ = handle.flush();
    let _ = fs2::FileExt::unlock(&file);
}

/// Path of a target's heavy queue entry: one file per claimant target, so a
/// retry refreshes the same place instead of adding another one.
fn heavy_queue_entry_path(dir: &Path, target: &str) -> PathBuf {
    dir.join(format!("{RESERVATION_PREFIX}{target}.json"))
}

/// This target's queue entry, or a fresh one. An entry that outlived both its
/// windows is residue, so a place is never revived from a claimant that walked
/// away long ago.
fn heavy_queue_entry(path: &Path, target: &str, priority: JobPriority) -> Registration {
    let now = now_ms();
    let existing = read_registration(path).filter(|entry| entry.outlives(now));
    Registration {
        schema_version: COORDINATOR_SCHEMA_VERSION,
        owner: OwnerIdentity::current(),
        priority,
        registered_at_ms: now,
        reserved_until_ms: existing.as_ref().and_then(|entry| entry.reserved_until_ms),
        reason: existing.as_ref().and_then(|entry| entry.reason.clone()),
        target: Some(target.to_string()),
        queued_at_ms: Some(existing.as_ref().map_or(now, Registration::queued_at)),
        position_until_ms: existing.as_ref().and_then(|entry| entry.position_until_ms),
    }
}

/// Join the heavy queue for `target` (Issue #4169) and answer with the arrival
/// time every claimant orders by. Joining twice keeps the first arrival: the
/// queue is per target, so a `deferred` rerun continues where it left off.
fn enroll_in_heavy_queue(dir: &Path, target: &str, priority: JobPriority) -> u64 {
    let path = heavy_queue_entry_path(dir, target);
    let mut entry = heavy_queue_entry(&path, target, priority);
    entry.position_until_ms =
        Some(now_ms().saturating_add(HEAVY_QUEUE_POSITION_TTL.as_millis() as u64));
    let queued_at_ms = entry.queued_at();
    let _ = write_json_atomic(&path, &entry);
    queued_at_ms
}

/// The heavy queue in service order, one record per claimant target.
fn heavy_queue(dir: &Path) -> Result<Vec<QueueRecord>, CoordinatorError> {
    let now = now_ms();
    let mut by_target: BTreeMap<String, QueueRecord> = BTreeMap::new();
    let mut untargeted: Vec<QueueRecord> = Vec::new();
    for live in sweep_live_registrations(dir)? {
        let Some(registration) = live.registration else {
            // A locked file nobody can parse is still a claimant; it just
            // cannot say who it is, so it takes the lowest standing.
            untargeted.push(QueueRecord {
                target: None,
                priority: JobPriority::Background,
                queued_at_ms: now,
                waiting: true,
                present: true,
            });
            continue;
        };
        let record = QueueRecord {
            target: registration.target.clone(),
            priority: registration.priority,
            queued_at_ms: registration.queued_at(),
            waiting: live.locked || registration.has_live_reservation(now),
            present: live.locked,
        };
        match record.target.clone() {
            // The same target can show up twice — its own attempt and the
            // entry that remembers its place — and that is one claimant.
            Some(target) => {
                by_target
                    .entry(target)
                    .and_modify(|kept| {
                        kept.queued_at_ms = kept.queued_at_ms.min(record.queued_at_ms);
                        kept.priority = kept.priority.min(record.priority);
                        kept.waiting |= record.waiting;
                        kept.present |= record.present;
                    })
                    .or_insert(record);
            }
            None => untargeted.push(record),
        }
    }
    let mut queue: Vec<QueueRecord> = by_target.into_values().chain(untargeted).collect();
    queue.sort_by(|left, right| left.rank().cmp(&right.rank()));
    Ok(queue)
}

/// The claimants actually waiting for a turn, as published in status output.
fn published_heavy_queue(dir: &Path) -> Result<Vec<HeavyQueueEntry>, CoordinatorError> {
    let now = now_ms();
    Ok(heavy_queue(dir)?
        .iter()
        .filter(|record| record.waiting)
        .map(|record| record.published(now))
        .collect())
}

/// True when a live claimant of strictly higher priority than `than` waits.
fn waiting_above(dir: &Path, than: JobPriority) -> Result<bool, CoordinatorError> {
    Ok(heavy_queue(dir)?
        .iter()
        .any(|record| record.waiting && record.priority < than))
}

/// One registration file that survived the sweep.
struct LiveRegistration {
    /// `None` when the payload could not be parsed.
    registration: Option<Registration>,
    /// A process holds the kernel shared lock on this file right now.
    locked: bool,
}

/// Scan a registration dir, sweep entries nothing keeps alive, and return what
/// remains. Liveness is the kernel shared lock its claimant holds, or — for a
/// heavy queue entry, which has no process behind it by design — a window that
/// has not lapsed.
fn sweep_live_registrations(dir: &Path) -> Result<Vec<LiveRegistration>, CoordinatorError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(CoordinatorError::Io(err)),
    };
    let now = now_ms();
    let mut live = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(file) = open_lock_file(&path) else {
            continue;
        };
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => {
                // Release the probe lock before reading: Windows refuses reads
                // through a second handle while this one holds the range.
                let _ = fs2::FileExt::unlock(&file);
                let registration = read_registration(&path);
                if let Some(entry) = &registration {
                    if entry.outlives(now) {
                        live.push(LiveRegistration {
                            registration,
                            locked: false,
                        });
                        continue;
                    }
                    if entry
                        .reserved_until_ms
                        .or(entry.position_until_ms)
                        .is_some()
                    {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                }
                // No holder and no window. A freshly created registration is
                // briefly lockable while its owner writes the payload and then
                // takes the shared lock — leave it alone so a concurrent sweep
                // never unlinks a live claimant. Anything still lockable after
                // the grace window is real crash residue.
                let age = file
                    .metadata()
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|modified| modified.elapsed().ok())
                    .unwrap_or_default();
                if age > REGISTRATION_RESIDUE_GRACE {
                    drop(file);
                    let _ = fs::remove_file(&path);
                }
            }
            Err(err) if is_contended(&err) => {
                live.push(LiveRegistration {
                    registration: read_registration(&path),
                    locked: true,
                });
            }
            Err(_) => {}
        }
    }
    Ok(live)
}

fn read_registration(path: &Path) -> Option<Registration> {
    let raw = fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(root: &Path) -> IndexCoordinator {
        IndexCoordinator::open(root).expect("open coordinator")
    }

    fn own(
        coordinator: &IndexCoordinator,
        key: &TargetKey,
        priority: JobPriority,
    ) -> TargetJobGuard {
        // A target lock released by a just-finished owner can still read as
        // contended for a scheduler tick under load, so `request_job` joins as
        // a waiter instead of taking ownership. Production self-heals (the
        // waiter's probe resolves it), but this helper asserts "I can own now,"
        // so it polls until the lock is genuinely free rather than failing on a
        // single spurious join (issue #3339). A stray join is dropped so its
        // waiter registration is removed before the next attempt.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match coordinator
                .request_job(key, priority, Duration::from_secs(5))
                .expect("request job")
            {
                JobAdmission::Owner(guard) => return guard,
                JobAdmission::Joined(waiter) => {
                    drop(waiter);
                    assert!(
                        Instant::now() < deadline,
                        "expected ownership of {} (state: {:?})",
                        key.file_stem(),
                        std::fs::read_to_string(coordinator.target_state_path(key)).ok(),
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    #[test]
    fn target_key_accessors_and_file_stem_are_sanitized() {
        let repo = TargetKey::repo_shared("repo/hash", "issues");
        assert_eq!(repo.repo_hash(), "repo/hash");
        assert_eq!(repo.scope(), "issues");
        assert_eq!(repo.worktree_hash(), None);
        assert_eq!(repo.file_stem(), "repo_hash--issues");

        let worktree = TargetKey::worktree("repo", "files-docs", "wt:1");
        assert_eq!(worktree.worktree_hash(), Some("wt:1"));
        assert_eq!(worktree.file_stem(), "repo--files-docs--wt_1");
    }

    #[test]
    fn priority_labels_and_ranking() {
        assert_eq!(
            JobPriority::InteractiveSearch.as_str(),
            "interactive-search"
        );
        assert_eq!(JobPriority::ManualRebuild.as_str(), "manual-rebuild");
        assert_eq!(JobPriority::Background.as_str(), "background");
        assert!(JobPriority::InteractiveSearch < JobPriority::Background);
    }

    #[test]
    fn owner_identity_is_stable_within_a_process() {
        let first = OwnerIdentity::current();
        let second = OwnerIdentity::current();
        assert_eq!(first, second);
        assert_eq!(first.pid, std::process::id());
    }

    #[test]
    fn open_default_creates_the_coordinator_root() {
        let coordinator = IndexCoordinator::open_default().expect("open default");
        assert!(coordinator.root().is_dir());
    }

    #[test]
    fn heavy_acquisition_times_out_while_another_owner_holds_it() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let holder = own(
            &coordinator,
            &TargetKey::repo_shared("repo-a", "issues"),
            JobPriority::Background,
        );
        let _heavy = holder.acquire_heavy(Duration::from_secs(5)).unwrap();

        let other = own(
            &coordinator,
            &TargetKey::repo_shared("repo-b", "specs"),
            JobPriority::Background,
        );
        match other.acquire_heavy(Duration::from_millis(120)) {
            Ok(_) => panic!("heavy lease must stay exclusive"),
            Err(CoordinatorError::Timeout { waited_ms }) => assert!(waited_ms >= 100),
            Err(other) => panic!("expected timeout, got {other:?}"),
        }
        other.complete(JobOutcome::Completed).unwrap();
        holder.complete(JobOutcome::Completed).unwrap();
    }

    #[test]
    fn background_defers_while_higher_priority_claimant_is_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let holder = own(
            &coordinator,
            &TargetKey::repo_shared("repo-a", "issues"),
            JobPriority::Background,
        );
        let heavy = holder.acquire_heavy(Duration::from_secs(5)).unwrap();

        // An interactive claimant queues for the heavy lease in a thread.
        let root = coordinator.root().to_path_buf();
        let interactive = std::thread::spawn(move || {
            let coordinator = open(&root);
            let guard = own(
                &coordinator,
                &TargetKey::repo_shared("repo-b", "files"),
                JobPriority::InteractiveSearch,
            );
            let heavy = guard
                .acquire_heavy(Duration::from_secs(10))
                .expect("interactive eventually acquires");
            drop(heavy);
            guard.complete(JobOutcome::Completed).unwrap();
        });

        // The pending interactive registration becomes visible (FR-383).
        // Generous deadline: the claimant thread competes with the whole
        // parallel test suite for CPU and disk.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if coordinator
                .pending_higher_priority(JobPriority::Background)
                .unwrap()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "interactive claimant must be visible as pending"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!coordinator
            .pending_higher_priority(JobPriority::InteractiveSearch)
            .unwrap());

        drop(heavy);
        holder.complete(JobOutcome::Completed).unwrap();
        interactive.join().unwrap();
    }

    #[test]
    fn verification_target_key_fixes_the_scope() {
        let key = TargetKey::verification("repo/hash", "wt:1");
        assert_eq!(key.scope(), VERIFICATION_SCOPE);
        assert_eq!(key.worktree_hash(), Some("wt:1"));
        assert!(key.is_verification());
        assert!(!TargetKey::repo_shared("repo", "issues").is_verification());
    }

    #[test]
    fn lease_ttl_expires_and_records_the_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let key = TargetKey::verification("repo-a", "wt-1");
        let guard = own(&coordinator, &key, JobPriority::ManualRebuild);
        let mut lease = guard
            .acquire_heavy_with_ttl(Duration::from_secs(5), Duration::from_secs(600))
            .unwrap();
        assert!(!lease.is_expired());
        assert!(lease.remaining().is_some());

        // Move the deadline into the past instead of sleeping up to a short
        // TTL: expiry is a plain `now >= expires_at` comparison either way,
        // and a wall-clock wait would make this test fail under exactly the
        // CPU contention the lease exists to prevent.
        lease.extend_until(lease.acquired_at_ms()).unwrap();
        assert!(
            lease.is_expired(),
            "the lease must expire once its deadline passes"
        );
        assert_eq!(lease.remaining(), Some(Duration::ZERO));

        let status = coordinator.heavy_lease_status().unwrap();
        assert!(status.held, "an expired lease is still physically held");
        assert!(status.expired, "status must surface the TTL expiry");
        assert_eq!(status.remaining_ms, Some(0));

        let lease_id = lease.id().to_string();
        lease.release().unwrap();
        guard.complete(JobOutcome::Completed).unwrap();

        let events = coordinator.lease_events().unwrap();
        let expiry = events
            .iter()
            .find(|event| event.kind == LeaseEventKind::Expired)
            .expect("TTL expiry must be recorded as an event");
        assert_eq!(expiry.lease_id, lease_id);
        assert_eq!(
            expiry.reason.as_deref(),
            Some("ttl elapsed"),
            "the expiry event must record why the lease lapsed"
        );
    }

    #[test]
    fn lease_extension_pushes_the_expiry_out() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let key = TargetKey::verification("repo-a", "wt-1");
        let guard = own(&coordinator, &key, JobPriority::ManualRebuild);
        let mut lease = guard
            .acquire_heavy_with_ttl(Duration::from_secs(5), Duration::from_millis(50))
            .unwrap();
        let first = lease.expires_at_ms().expect("ttl lease has an expiry");

        lease.extend(Duration::from_secs(120)).unwrap();
        let extended = lease.expires_at_ms().expect("extended lease has an expiry");
        assert!(
            extended > first,
            "extend must move the expiry forward ({first} -> {extended})"
        );

        std::thread::sleep(Duration::from_millis(70));
        assert!(!lease.is_expired(), "an extended lease must not expire");
        assert_eq!(
            coordinator.heavy_lease_status().unwrap().expires_at_ms,
            Some(extended),
            "the published ticket must carry the extended expiry"
        );

        lease.release().unwrap();
        guard.complete(JobOutcome::Completed).unwrap();
        let events = coordinator.lease_events().unwrap();
        assert!(events
            .iter()
            .any(|event| event.kind == LeaseEventKind::Extended));
    }

    #[test]
    fn status_is_free_when_no_claimant_holds_the_lease() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let status = coordinator.heavy_lease_status().unwrap();
        assert!(!status.held);
        assert_eq!(status.lease_id, None);
        assert_eq!(status.remaining_ms, None);
        assert!(!status.expired);
    }

    #[test]
    fn leases_without_a_ttl_never_expire() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let guard = own(
            &coordinator,
            &TargetKey::repo_shared("repo-a", "issues"),
            JobPriority::Background,
        );
        let lease = guard.acquire_heavy(Duration::from_secs(5)).unwrap();
        assert_eq!(lease.expires_at_ms(), None);
        assert_eq!(lease.remaining(), None);
        assert!(!lease.is_expired());
        assert!(!coordinator.heavy_lease_status().unwrap().expired);
        drop(lease);
        guard.complete(JobOutcome::Completed).unwrap();
    }

    #[test]
    fn waiter_times_out_when_owner_never_completes() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let key = TargetKey::repo_shared("repo-a", "issues");
        let owner = own(&coordinator, &key, JobPriority::Background);
        assert_eq!(owner.waiter_count().unwrap(), 0);
        assert_eq!(owner.priority(), JobPriority::Background);
        assert_eq!(owner.key().scope(), "issues");

        let waiter = match coordinator
            .request_job(&key, JobPriority::Background, Duration::from_secs(5))
            .unwrap()
        {
            JobAdmission::Joined(waiter) => waiter,
            JobAdmission::Owner(_) => panic!("owner already holds the target"),
        };
        assert_eq!(waiter.key().scope(), "issues");
        assert_eq!(owner.waiter_count().unwrap(), 1);
        let error = waiter
            .wait(Duration::from_millis(120))
            .expect_err("owner never completes");
        assert!(matches!(error, CoordinatorError::Timeout { .. }));
        owner.complete(JobOutcome::Completed).unwrap();
    }

    #[test]
    fn waiters_observe_failed_and_abandoned_outcomes() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let key = TargetKey::repo_shared("repo-a", "board");

        let owner = own(&coordinator, &key, JobPriority::Background);
        let waiter = match coordinator
            .request_job(&key, JobPriority::Background, Duration::from_secs(5))
            .unwrap()
        {
            JobAdmission::Joined(waiter) => waiter,
            JobAdmission::Owner(_) => panic!("expected join"),
        };
        owner
            .complete(JobOutcome::Failed {
                message: "disk full".to_string(),
            })
            .unwrap();
        assert_eq!(
            waiter.wait(Duration::from_secs(5)).unwrap(),
            JobOutcome::Failed {
                message: "disk full".to_string()
            }
        );

        // Owner dropping without publishing surfaces OwnerGone.
        let owner = own(&coordinator, &key, JobPriority::Background);
        let waiter = match coordinator
            .request_job(&key, JobPriority::Background, Duration::from_secs(5))
            .unwrap()
        {
            JobAdmission::Joined(waiter) => waiter,
            JobAdmission::Owner(_) => panic!("expected join"),
        };
        drop(owner);
        assert_eq!(
            waiter.wait(Duration::from_secs(5)).unwrap(),
            JobOutcome::OwnerGone
        );
    }

    // ------------------------------------------------------------------
    // Issue #4086: verification reservations outlive their claimant so a
    // background index job can never slip in between two retries.
    // ------------------------------------------------------------------

    fn verification_key() -> TargetKey {
        TargetKey::verification("repo-a", "wt-1")
    }

    #[test]
    fn heavy_reservation_counts_as_live_pending_without_a_holder() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let reservation = coordinator
            .reserve_heavy(
                &verification_key(),
                JobPriority::ManualRebuild,
                VERIFICATION_RESERVATION_TTL,
                Some("Issue 4086 RED"),
            )
            .expect("reserve");
        assert_eq!(reservation.priority, JobPriority::ManualRebuild);
        assert!(reservation.expires_at_ms > now_ms());

        // Nobody holds a lock on the reservation file, yet it is pending.
        assert!(coordinator
            .pending_higher_priority(JobPriority::Background)
            .unwrap());
        assert_eq!(coordinator.heavy_lease_status().unwrap().pending, 1);

        // A background index job defers for as long as the reservation lives.
        let index = own(
            &coordinator,
            &TargetKey::repo_shared("repo-a", "issues"),
            JobPriority::Background,
        );
        match index.acquire_heavy(Duration::from_millis(200)) {
            Ok(_) => panic!("background must defer to a live verification reservation"),
            Err(CoordinatorError::Timeout { .. }) => {}
            Err(other) => panic!("expected timeout, got {other:?}"),
        }

        assert!(coordinator
            .clear_heavy_reservation(&verification_key())
            .unwrap());
        assert!(!coordinator
            .pending_higher_priority(JobPriority::Background)
            .unwrap());
        let heavy = index.acquire_heavy(Duration::from_secs(5)).unwrap();
        drop(heavy);
        index.complete(JobOutcome::Completed).unwrap();
    }

    #[test]
    fn expired_heavy_reservation_is_swept_and_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        coordinator
            .reserve_heavy(
                &verification_key(),
                JobPriority::ManualRebuild,
                Duration::from_millis(1),
                None,
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert!(!coordinator
            .pending_higher_priority(JobPriority::Background)
            .unwrap());
        assert_eq!(coordinator.heavy_lease_status().unwrap().pending, 0);
        assert!(
            !coordinator
                .heavy_reservation_path(&verification_key())
                .exists(),
            "an expired reservation is swept on the first scan"
        );
    }

    #[test]
    fn granting_the_heavy_lease_clears_the_claimants_reservation() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        coordinator
            .reserve_heavy(
                &verification_key(),
                JobPriority::ManualRebuild,
                VERIFICATION_RESERVATION_TTL,
                None,
            )
            .unwrap();
        let guard = own(
            &coordinator,
            &verification_key(),
            JobPriority::ManualRebuild,
        );
        // The claimant's own reservation never defers the claimant itself.
        let heavy = guard
            .acquire_heavy_with_ttl(Duration::from_millis(250), Duration::from_secs(60))
            .expect("the reserving claimant acquires immediately");
        assert!(!coordinator
            .heavy_reservation_path(&verification_key())
            .exists());
        assert_eq!(coordinator.heavy_lease_status().unwrap().pending, 0);
        drop(heavy);
        guard.complete(JobOutcome::Completed).unwrap();
    }

    // ------------------------------------------------------------------
    // Issue #4169: a freed heavy lease is handed to the claimant that has
    // been queued longest, and a claimant that had to give up keeps its
    // place in the queue.
    // ------------------------------------------------------------------

    fn wait_until(mut done: impl FnMut() -> bool, context: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done() {
            assert!(Instant::now() < deadline, "{context}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// AC-1 / AC-5: a claimant that never queued must not take a lease a
    /// waiter has been queued for. Before the queue existed the freed lease
    /// went to whoever probed the kernel lock first, and a newcomer probes it
    /// the instant it starts while a waiter is asleep between polls — which is
    /// how #4119 spent 50 minutes waiting without ever getting a turn.
    #[test]
    fn a_newcomer_never_overtakes_a_queued_waiter() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let holder = own(
            &coordinator,
            &TargetKey::verification("repo", "holder"),
            JobPriority::ManualRebuild,
        );
        let heavy = holder
            .acquire_heavy_with_ttl(Duration::from_secs(5), Duration::from_secs(60))
            .unwrap();

        let root = coordinator.root().to_path_buf();
        let release_waiter = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (granted_tx, granted_rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn({
            let release = std::sync::Arc::clone(&release_waiter);
            move || {
                let coordinator = open(&root);
                let guard = own(
                    &coordinator,
                    &TargetKey::verification("repo", "waiter"),
                    JobPriority::ManualRebuild,
                );
                let lease = guard
                    .acquire_heavy_with_ttl(Duration::from_secs(60), Duration::from_secs(60))
                    .expect("the queued waiter must be served once the lease frees");
                granted_tx.send(()).expect("report the grant");
                while !release.load(std::sync::atomic::Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                lease.release().unwrap();
                guard.complete(JobOutcome::Completed).unwrap();
            }
        });
        wait_until(
            || coordinator.heavy_lease_status().unwrap().pending >= 1,
            "the waiter must become visible in the queue",
        );

        // The lease frees while the waiter is between polls — the exact
        // instant a newcomer used to win.
        drop(heavy);
        let newcomer = own(
            &coordinator,
            &TargetKey::verification("repo", "newcomer"),
            JobPriority::ManualRebuild,
        );
        let refused =
            newcomer.acquire_heavy_with_ttl(Duration::from_secs(1), Duration::from_secs(60));
        assert!(
            matches!(refused, Err(CoordinatorError::Timeout { .. })),
            "a newcomer must queue behind the waiter instead of taking the freed lease"
        );
        granted_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("the earliest waiter must be granted the freed lease");

        release_waiter.store(true, std::sync::atomic::Ordering::SeqCst);
        waiter.join().unwrap();
        newcomer.complete(JobOutcome::Completed).unwrap();
        holder.complete(JobOutcome::Completed).unwrap();
    }

    /// AC-3 / AC-2: a claimant that spent its whole wait budget and gave up
    /// keeps its arrival time, so its rerun stays ahead of a claimant that
    /// queued later — and the published queue says so, with an identifier and
    /// a wait time per entry.
    #[test]
    fn a_deferred_claimant_keeps_its_place_and_the_queue_shows_it() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let holder = own(
            &coordinator,
            &TargetKey::verification("repo", "holder"),
            JobPriority::ManualRebuild,
        );
        let heavy = holder
            .acquire_heavy_with_ttl(Duration::from_secs(5), Duration::from_secs(60))
            .unwrap();

        // The first claimant runs out of budget and returns `deferred`.
        let early_key = TargetKey::verification("repo", "early");
        let early = own(&coordinator, &early_key, JobPriority::ManualRebuild);
        assert!(
            matches!(
                early.acquire_heavy_with_ttl(Duration::from_millis(80), Duration::from_secs(60)),
                Err(CoordinatorError::Timeout { .. })
            ),
            "the held lease must defer the first claimant"
        );

        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let claim = |label: &'static str, guard: TargetJobGuard| {
            let order = std::sync::Arc::clone(&order);
            std::thread::spawn(move || {
                let lease = guard
                    .acquire_heavy_with_ttl(Duration::from_secs(60), Duration::from_secs(60))
                    .unwrap_or_else(|err| panic!("{label} must be granted the lease: {err}"));
                order.lock().unwrap().push(label.to_string());
                lease.release().unwrap();
                guard.complete(JobOutcome::Completed).unwrap();
            })
        };

        // A second claimant queues only after the first one gave up, so wall
        // clock order and queue order disagree unless the place is kept.
        let late_key = TargetKey::verification("repo", "late");
        let late = claim(
            "late",
            own(&coordinator, &late_key, JobPriority::ManualRebuild),
        );
        wait_until(
            || coordinator.heavy_lease_status().unwrap().pending >= 1,
            "the second claimant must become visible in the queue",
        );
        let early = claim("early", early);
        wait_until(
            || coordinator.heavy_lease_status().unwrap().queue.len() >= 2,
            "both claimants must be visible in the queue",
        );

        let queue = coordinator.heavy_lease_status().unwrap().queue;
        assert_eq!(
            queue[0].target.as_deref(),
            Some(early_key.file_stem().as_str()),
            "the deferred claimant keeps its place: {queue:?}"
        );
        assert_eq!(
            queue[1].target.as_deref(),
            Some(late_key.file_stem().as_str()),
            "{queue:?}"
        );
        assert!(
            queue[0].queued_at_ms <= queue[1].queued_at_ms,
            "the queue is ordered by arrival: {queue:?}"
        );
        assert!(
            queue[0].waiting_ms >= queue[1].waiting_ms,
            "the earliest entry has waited longest: {queue:?}"
        );

        drop(heavy);
        early.join().unwrap();
        late.join().unwrap();
        assert_eq!(
            order.lock().unwrap().clone(),
            vec!["early".to_string(), "late".to_string()],
            "the freed lease is handed out in queue order"
        );
        holder.complete(JobOutcome::Completed).unwrap();
    }

    #[test]
    fn reserved_waiter_blocks_equal_priority_newcomer_between_retries() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let early_key = TargetKey::verification("repo", "early");
        coordinator
            .reserve_heavy(
                &early_key,
                JobPriority::ManualRebuild,
                Duration::from_secs(60),
                None,
            )
            .unwrap();
        let late = own(
            &coordinator,
            &TargetKey::verification("repo", "late"),
            JobPriority::ManualRebuild,
        );
        assert!(
            matches!(
                late.acquire_heavy_with_ttl(Duration::ZERO, Duration::from_secs(60)),
                Err(CoordinatorError::Timeout { .. })
            ),
            "a newcomer must wait even while the first claimant is between retries"
        );
        coordinator.clear_heavy_reservation(&early_key).unwrap();
        let lease = late
            .acquire_heavy_with_ttl(Duration::ZERO, Duration::from_secs(60))
            .expect("clearing the reservation lets the next claimant proceed");
        lease.release().unwrap();
    }

    #[test]
    fn verification_poll_timeout_keeps_a_live_reservation() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let holder = own(
            &coordinator,
            &TargetKey::verification("repo", "holder"),
            JobPriority::ManualRebuild,
        );
        let lease = holder.acquire_heavy(Duration::ZERO).unwrap();
        let key = TargetKey::verification("repo", "waiter");
        let waiter = own(&coordinator, &key, JobPriority::ManualRebuild);
        for _ in 0..2 {
            assert!(matches!(
                waiter.acquire_heavy(Duration::ZERO),
                Err(CoordinatorError::Timeout { .. })
            ));
            assert_eq!(
                coordinator.heavy_lease_status().unwrap().queue.len(),
                1,
                "the waiter must remain pending between bounded polling calls"
            );
        }
        lease.release().unwrap();
        waiter
            .acquire_heavy(Duration::ZERO)
            .unwrap()
            .release()
            .unwrap();
    }

    /// Every queue record the predicate can be asked about, so the invariants
    /// below are checked against the whole space rather than a chosen corner.
    fn every_queue_record() -> Vec<QueueRecord> {
        let mut records = Vec::new();
        for priority in [
            JobPriority::InteractiveSearch,
            JobPriority::ManualRebuild,
            JobPriority::Background,
        ] {
            for queued_at_ms in [10_u64, 20, 30] {
                for target in [Some("a"), Some("b"), None] {
                    for waiting in [false, true] {
                        for present in [false, true] {
                            records.push(QueueRecord {
                                target: target.map(str::to_string),
                                priority,
                                queued_at_ms,
                                waiting,
                                present,
                            });
                        }
                    }
                }
            }
        }
        records
    }

    /// Issue #1939 raised this against the queue after its own burst cap
    /// livelocked: a scheme where claimants defer to one another can reach a
    /// state where every claimant yields and a free lease is taken by nobody.
    ///
    /// It cannot happen here, and the reason is structural rather than lucky.
    /// `blocks` is derived from `rank`, a strict total order, and only ever
    /// answers true for a claimant that ranks *strictly* ahead. A strict order
    /// is asymmetric, so "A defers to B" and "B defers to A" cannot both hold.
    #[test]
    fn deferral_is_asymmetric_so_two_claimants_never_defer_to_each_other() {
        let records = every_queue_record();
        for left in &records {
            for right in &records {
                if left.blocks(right) {
                    assert!(
                        !right.blocks(left),
                        "mutual deferral between {left:?} and {right:?}"
                    );
                }
            }
        }
    }

    /// Asymmetry rules out a two-party standoff; this rules out a deferral
    /// cycle of any length, which is what a livelock actually is.
    ///
    /// `blocks` answers true only when the blocker ranks strictly ahead — the
    /// priority branch implies it, because `rank` compares priority first, and
    /// the queue branch requires it outright. A relation contained in a strict
    /// order cannot contain a cycle, so every queue has a claimant that defers
    /// to nobody and the chain always terminates.
    ///
    /// That head-of-queue claimant may be a reservation with no process behind
    /// it (Issue #4086 keeps a claimant's turn between its retries). Waiting
    /// for it is a *bounded* wait, not a livelock: the reservation lapses at
    /// its TTL and the next claimant is served.
    #[test]
    fn deferral_never_forms_a_cycle() {
        let records = every_queue_record();
        for left in &records {
            for right in &records {
                if left.blocks(right) {
                    assert!(
                        left.rank() < right.rank(),
                        "a blocker must rank strictly ahead: {left:?} blocks {right:?}"
                    );
                }
            }
        }
        // The consequence, checked directly: every queue has a head that
        // defers to nobody, so the chain of deferrals always terminates.
        for window in records.windows(7) {
            let head = window
                .iter()
                .min_by(|left, right| left.rank().cmp(&right.rank()))
                .expect("a non-empty window has a lowest-ranked record");
            assert!(
                !window
                    .iter()
                    .any(|other| other.target != head.target && other.blocks(head)),
                "the head of the queue must defer to nobody: {head:?} in {window:?}"
            );
        }
    }

    #[test]
    fn heavy_lease_status_reports_holder_kind_and_index_estimate() {
        let tmp = tempfile::tempdir().unwrap();
        let coordinator = open(tmp.path());
        let key = TargetKey::repo_shared("repo-a", "issues");
        let index = own(&coordinator, &key, JobPriority::Background);
        let heavy = index
            .acquire_heavy_with_ttl(Duration::from_secs(5), INDEX_HEAVY_LEASE_TTL)
            .unwrap();

        let status = coordinator.heavy_lease_status().unwrap();
        assert_eq!(status.holder_kind, Some(HeavyHolderKind::Index));
        assert!(status.expires_at_ms.is_some(), "index leases carry a TTL");
        assert_eq!(status.remaining_batches, None);
        assert_eq!(status.estimated_remaining_ms, None);

        // The runner publishes its batch progress next to the ticket.
        coordinator
            .write_heavy_progress(&HeavyProgress {
                target: key.file_stem(),
                done: 10,
                total: 100,
                batch_size: 16,
                batch_ms: 2_000,
                updated_at_ms: now_ms(),
            })
            .unwrap();
        let status = coordinator.heavy_lease_status().unwrap();
        // ceil(90 / 16) = 6 batches × 2 s each.
        assert_eq!(status.remaining_batches, Some(6));
        assert_eq!(status.estimated_remaining_ms, Some(12_000));

        // Progress written for another target is not attributed to this holder.
        coordinator
            .write_heavy_progress(&HeavyProgress {
                target: "repo-b--files--wt".to_string(),
                done: 0,
                total: 1_000,
                batch_size: 16,
                batch_ms: 9_000,
                updated_at_ms: now_ms(),
            })
            .unwrap();
        let status = coordinator.heavy_lease_status().unwrap();
        assert_eq!(status.remaining_batches, None);
        assert_eq!(status.estimated_remaining_ms, None);
        drop(heavy);
        index.complete(JobOutcome::Completed).unwrap();

        let guard = own(
            &coordinator,
            &verification_key(),
            JobPriority::ManualRebuild,
        );
        let heavy = guard
            .acquire_heavy_with_ttl(Duration::from_secs(5), Duration::from_secs(600))
            .unwrap();
        let status = coordinator.heavy_lease_status().unwrap();
        assert_eq!(status.holder_kind, Some(HeavyHolderKind::Verification));
        assert_eq!(status.estimated_remaining_ms, status.remaining_ms);
        drop(heavy);
        guard.complete(JobOutcome::Completed).unwrap();

        assert_eq!(coordinator.heavy_lease_status().unwrap().holder_kind, None);
        assert_eq!(HeavyHolderKind::Index.as_str(), "index");
        assert_eq!(HeavyHolderKind::Verification.as_str(), "verification");
        assert_eq!(HeavyHolderKind::Other.as_str(), "other");
    }
}
