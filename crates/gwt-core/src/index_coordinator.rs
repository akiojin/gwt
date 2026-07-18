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

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Schema version stamped into every ticket / state JSON payload.
pub const COORDINATOR_SCHEMA_VERSION: u32 = 1;

const COORDINATOR_DIR_NAME: &str = "index-coordinator";
const POLL_INTERVAL: Duration = Duration::from_millis(25);

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
                        &Ticket {
                            schema_version: COORDINATOR_SCHEMA_VERSION,
                            target: key.file_stem(),
                            priority,
                            owner: owner.clone(),
                            acquired_at_ms: now_ms(),
                        },
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
                    // Create and shared-lock the registration BEFORE writing
                    // content so a concurrent stale sweep never deletes a
                    // just-registered live waiter.
                    let waiter_file = open_lock_file(&waiter_path)?;
                    waiter_file.lock_shared()?;
                    let registration = Registration {
                        schema_version: COORDINATOR_SCHEMA_VERSION,
                        owner: OwnerIdentity::current(),
                        priority,
                        registered_at_ms: now_ms(),
                    };
                    let mut handle = &waiter_file;
                    handle.write_all(&serde_json::to_vec(&registration).map_err(io_invalid)?)?;
                    handle.flush()?;
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
        Ok(scan_live_pending(&self.heavy_pending_dir())?
            .into_iter()
            .any(|priority| priority < than))
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
        let pending_dir = self.root.join("heavy.pending");
        fs::create_dir_all(&pending_dir)?;
        let pending_path = pending_dir.join(format!("{}.json", uuid::Uuid::new_v4()));
        let pending_file = open_lock_file(&pending_path)?;
        pending_file.lock_shared()?;
        let registration = Registration {
            schema_version: COORDINATOR_SCHEMA_VERSION,
            owner: OwnerIdentity::current(),
            priority: self.priority,
            registered_at_ms: now_ms(),
        };
        {
            let mut handle = &pending_file;
            handle.write_all(&serde_json::to_vec(&registration).map_err(io_invalid)?)?;
            handle.flush()?;
        }
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
            let must_defer = self.priority != JobPriority::InteractiveSearch
                && scan_live_pending_excluding(&pending_dir, &pending_path)
                    .unwrap_or_default()
                    .into_iter()
                    .any(|priority| priority < self.priority);
            if !must_defer {
                match fs2::FileExt::try_lock_exclusive(&heavy_file) {
                    Ok(()) => {
                        let _ = write_json_atomic(
                            &self.root.join("heavy.ticket.json"),
                            &Ticket {
                                schema_version: COORDINATOR_SCHEMA_VERSION,
                                target: self.key.file_stem(),
                                priority: self.priority,
                                owner: OwnerIdentity::current(),
                                acquired_at_ms: now_ms(),
                            },
                        );
                        cleanup_pending(pending_file, &pending_path);
                        return Ok(HeavyLease {
                            _lock_file: heavy_file,
                            ticket_path: self.root.join("heavy.ticket.json"),
                        });
                    }
                    Err(err) if is_contended(&err) => {}
                    Err(err) => {
                        cleanup_pending(pending_file, &pending_path);
                        return Err(CoordinatorError::Io(err));
                    }
                }
            }
            if started.elapsed() >= timeout {
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

/// Host-wide heavy lease. Dropping releases the kernel heavy lock.
pub struct HeavyLease {
    _lock_file: File,
    ticket_path: PathBuf,
}

impl Drop for HeavyLease {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.ticket_path);
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

/// Scan a registration dir, sweep stale entries (their shared lock is gone,
/// so `try_lock_exclusive` succeeds), and return the live priorities.
fn scan_live_pending(dir: &Path) -> Result<Vec<JobPriority>, CoordinatorError> {
    Ok(sweep_live_registrations(dir)?
        .into_iter()
        .filter_map(|registration| registration.map(|r| r.priority))
        .collect())
}

fn scan_live_pending_excluding(
    dir: &Path,
    exclude: &Path,
) -> Result<Vec<JobPriority>, CoordinatorError> {
    Ok(sweep_live_registrations_excluding(dir, Some(exclude))?
        .into_iter()
        .filter_map(|registration| registration.map(|r| r.priority))
        .collect())
}

/// Returns one entry per live registration file (parse failures yield
/// `None` entries so callers can still count liveness).
fn sweep_live_registrations(dir: &Path) -> Result<Vec<Option<Registration>>, CoordinatorError> {
    sweep_live_registrations_excluding(dir, None)
}

fn sweep_live_registrations_excluding(
    dir: &Path,
    exclude: Option<&Path>,
) -> Result<Vec<Option<Registration>>, CoordinatorError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(CoordinatorError::Io(err)),
    };
    let mut live = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if exclude.is_some_and(|excluded| excluded == path) {
            live.push(read_registration(&path));
            continue;
        }
        let Ok(file) = open_lock_file(&path) else {
            continue;
        };
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => {
                // No live holder. A freshly created registration is briefly
                // lockable (and still empty) before its owner takes the
                // shared lock and writes the payload — leave it alone so a
                // concurrent sweep never unlinks a live claimant. Anything
                // empty for longer than a minute is a real crash residue.
                let (len, age) = file
                    .metadata()
                    .map(|meta| {
                        let age = meta
                            .modified()
                            .ok()
                            .and_then(|modified| modified.elapsed().ok())
                            .unwrap_or_default();
                        (meta.len(), age)
                    })
                    .unwrap_or((0, Duration::ZERO));
                if len > 0 || age > Duration::from_secs(60) {
                    drop(file);
                    let _ = fs::remove_file(&path);
                }
            }
            Err(err) if is_contended(&err) => {
                live.push(read_registration(&path));
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
        match coordinator
            .request_job(key, priority, Duration::from_secs(5))
            .expect("request job")
        {
            JobAdmission::Owner(guard) => guard,
            JobAdmission::Joined(_) => panic!(
                "expected ownership of {} (state: {:?})",
                key.file_stem(),
                std::fs::read_to_string(coordinator.target_state_path(key)).ok(),
            ),
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
}
