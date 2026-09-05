//! Host-wide Refresh Broker (SPEC #1939 Phase 71 FR-414 / FR-415 / FR-416,
//! Issue #3772 AC-3).
//!
//! Every refresh entrypoint (startup, FrontendReady, Agent launch, search,
//! manual rebuild, repair, a future watcher) submits a [`RefreshIntent`]
//! instead of spawning a runner. The broker keeps one durable
//! [`RefreshTargetState`] record per normalized target so an event storm
//! collapses into the latest desired epoch / snapshot:
//!
//! - background intents wait for a quiet period measured from the latest
//!   epoch; a delayed older epoch never resets the deadline (FR-415)
//! - interactive / manual intents promote the target into the runnable queue
//!   and join the same owner (FR-415)
//! - a running target keeps at most one coalesced follow-up (AS-29)
//! - queue depth is bounded by the number of distinct targets, never by the
//!   number of events (SC-064)
//! - `inspect` is read-only and never touches durable bytes (AS-32)
//! - one target has at most one owner across OS processes; the kernel lock
//!   on the owner file is the exclusion truth, the JSON record is diagnostics
//!
//! The broker sits in front of the Phase 70 [`crate::index_coordinator`]: the
//! claim owner still runs the actual build through the coordinator so the
//! host-wide heavy lease and same-target coalescing remain in force.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::index_coordinator::{coordinator_root, coordinator_root_from, JobPriority};

/// Wire/protocol version of [`RefreshIntent`]. Bumped only when the intent
/// shape changes incompatibly; a mismatching submission is refused.
pub const REFRESH_INTENT_PROTOCOL_VERSION: u32 = 1;

/// Durable record schema version.
pub const REFRESH_BROKER_SCHEMA_VERSION: u32 = 1;

/// Quiet period for background dirty intents (FR-415).
pub const DEFAULT_REFRESH_QUIET_PERIOD: Duration = Duration::from_secs(30);

const BROKER_DIR_NAME: &str = "refresh-broker";
const TARGETS_DIR_NAME: &str = "targets";

/// Broker root under an explicit gwt home
/// (`<gwt_home>/runtime/index-coordinator/refresh-broker`).
pub fn refresh_broker_root_from(gwt_home: &Path) -> PathBuf {
    coordinator_root_from(gwt_home).join(BROKER_DIR_NAME)
}

/// Broker root for the current process
/// (`~/.gwt/runtime/index-coordinator/refresh-broker`).
pub fn refresh_broker_root() -> PathBuf {
    coordinator_root().join(BROKER_DIR_NAME)
}

/// File scopes covered by a refresh target. Base and overlay targets always
/// carry both Files scopes so one View head switches them atomically
/// (AS-23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshScope {
    Files,
    FilesDocs,
}

impl RefreshScope {
    pub fn as_str(self) -> &'static str {
        match self {
            RefreshScope::Files => "files",
            RefreshScope::FilesDocs => "files-docs",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshTargetKind {
    /// Canonical repository base: `(repo_hash, scope set)`.
    Base,
    /// Worktree overlay: `(repo_hash, worktree_hash, scope set)`.
    Overlay,
}

/// Normalized refresh target key (FR-414). Source fingerprints are not part of
/// the key; the latest desired snapshot coalesces onto the same record.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RefreshTarget {
    kind: RefreshTargetKind,
    repo_hash: String,
    worktree_hash: Option<String>,
    scopes: Vec<RefreshScope>,
}

impl RefreshTarget {
    pub fn base(
        repo_hash: impl Into<String>,
        scopes: impl IntoIterator<Item = RefreshScope>,
    ) -> Self {
        Self {
            kind: RefreshTargetKind::Base,
            repo_hash: repo_hash.into(),
            worktree_hash: None,
            scopes: normalize_scopes(scopes),
        }
    }

    pub fn overlay(
        repo_hash: impl Into<String>,
        worktree_hash: impl Into<String>,
        scopes: impl IntoIterator<Item = RefreshScope>,
    ) -> Self {
        Self {
            kind: RefreshTargetKind::Overlay,
            repo_hash: repo_hash.into(),
            worktree_hash: Some(worktree_hash.into()),
            scopes: normalize_scopes(scopes),
        }
    }

    pub fn kind(&self) -> RefreshTargetKind {
        self.kind
    }

    pub fn repo_hash(&self) -> &str {
        &self.repo_hash
    }

    pub fn worktree_hash(&self) -> Option<&str> {
        self.worktree_hash.as_deref()
    }

    pub fn scopes(&self) -> &[RefreshScope] {
        &self.scopes
    }

    /// Stable, filesystem-safe record stem. Human readable for diagnostics
    /// with a digest suffix so sanitized identities cannot collide.
    fn file_stem(&self) -> String {
        let identity = self.identity_string();
        let digest = format!("{:x}", Sha256::digest(identity.as_bytes()));
        let mut stem = String::new();
        stem.push_str(match self.kind {
            RefreshTargetKind::Base => "base",
            RefreshTargetKind::Overlay => "overlay",
        });
        stem.push('-');
        stem.push_str(&sanitize_component(&self.repo_hash));
        if let Some(worktree) = &self.worktree_hash {
            stem.push('-');
            stem.push_str(&sanitize_component(worktree));
        }
        stem.push('-');
        stem.push_str(&self.scope_label());
        stem.push('-');
        stem.push_str(&digest[..12]);
        stem
    }

    fn scope_label(&self) -> String {
        self.scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>()
            .join("+")
    }

    fn identity_string(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}",
            match self.kind {
                RefreshTargetKind::Base => "base",
                RefreshTargetKind::Overlay => "overlay",
            },
            self.repo_hash,
            self.worktree_hash.as_deref().unwrap_or(""),
            self.scope_label()
        )
    }
}

fn normalize_scopes(scopes: impl IntoIterator<Item = RefreshScope>) -> Vec<RefreshScope> {
    let mut scopes: Vec<RefreshScope> = scopes.into_iter().collect();
    scopes.sort();
    scopes.dedup();
    scopes
}

fn sanitize_component(value: &str) -> String {
    let mut sanitized: String = value
        .chars()
        .take(64)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        sanitized.push('_');
    }
    sanitized
}

/// Why an entrypoint asked for a refresh. Diagnostics only; it never changes
/// admission semantics beyond what `priority` already expresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshReason {
    Startup,
    Launch,
    DirtyEvent,
    Search,
    Manual,
    Repair,
}

/// Resource class a refresh will consume once admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshResourceClass {
    /// Model load / document encode; must go through the heavy lease.
    Embedding,
    /// Metadata-only maintenance without model work.
    Metadata,
}

/// One refresh request from an entrypoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshIntent {
    pub protocol_version: u32,
    pub target: RefreshTarget,
    /// Latest source event observed by the submitter. Older epochs never
    /// move a record backwards.
    pub desired_epoch: u64,
    /// Latest immutable identity once staged (canonical tree OID, source
    /// snapshot id, ...). Free-form for the broker.
    pub desired_snapshot: String,
    pub priority: JobPriority,
    pub reason: RefreshReason,
    pub resource_class: RefreshResourceClass,
}

/// Durable per-target state (data-model "Refresh Target State").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshTargetState {
    /// Background dirty intent waiting for its quiet deadline.
    Quiet,
    /// Runnable now (promoted or quiet deadline reached).
    Queued,
    /// Claimed by one owner.
    Running,
    /// Claimed by one owner while a newer epoch arrived; exactly one
    /// follow-up is retained.
    DirtyDuringRun,
    /// Latest desired epoch has been completed.
    Ready,
}

/// Injectable clock so admission timing is deterministic under test.
pub trait RefreshBrokerClock: Send + Sync {
    fn now_millis(&self) -> u64;
}

#[derive(Debug, Default)]
struct SystemRefreshBrokerClock;

impl RefreshBrokerClock for SystemRefreshBrokerClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshBrokerError {
    #[error("refresh broker io error: {0}")]
    Io(#[from] io::Error),
    #[error("refresh broker protocol error: {0}")]
    Protocol(String),
    #[error("refresh broker record corrupt at {path}: {detail}")]
    Corrupt { path: PathBuf, detail: String },
}

type BrokerResult<T> = Result<T, RefreshBrokerError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RunningJob {
    epoch: u64,
    snapshot: String,
    priority: JobPriority,
    reason: RefreshReason,
    resource_class: RefreshResourceClass,
    owner_pid: u32,
    claimed_at_millis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TargetRecord {
    schema_version: u32,
    target: RefreshTarget,
    desired_epoch: u64,
    desired_snapshot: String,
    /// Pending admission priority (most urgent intent since the last claim).
    priority: JobPriority,
    state: RefreshTargetState,
    quiet_deadline_millis: Option<u64>,
    running: Option<RunningJob>,
    /// Single coalesced follow-up flag; never an event list (AS-29).
    follow_up_required: bool,
    last_reason: RefreshReason,
    resource_class: RefreshResourceClass,
    completed_epoch: Option<u64>,
    last_error: Option<String>,
    updated_at_millis: u64,
}

/// Read-only projection of one target record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshTargetSnapshot {
    record: TargetRecord,
}

impl RefreshTargetSnapshot {
    pub fn target(&self) -> &RefreshTarget {
        &self.record.target
    }

    pub fn desired_epoch(&self) -> u64 {
        self.record.desired_epoch
    }

    pub fn desired_snapshot(&self) -> &str {
        &self.record.desired_snapshot
    }

    pub fn priority(&self) -> JobPriority {
        self.record.priority
    }

    pub fn state(&self) -> RefreshTargetState {
        self.record.state
    }

    pub fn quiet_deadline_millis(&self) -> Option<u64> {
        self.record.quiet_deadline_millis
    }

    /// Number of retained follow-ups: always `0` or `1`.
    pub fn follow_up_count(&self) -> usize {
        usize::from(self.record.follow_up_required)
    }

    pub fn running_epoch(&self) -> Option<u64> {
        self.record.running.as_ref().map(|job| job.epoch)
    }

    pub fn reason(&self) -> RefreshReason {
        self.record.last_reason
    }

    pub fn completed_epoch(&self) -> Option<u64> {
        self.record.completed_epoch
    }

    pub fn last_error(&self) -> Option<&str> {
        self.record.last_error.as_deref()
    }

    fn is_running(&self) -> bool {
        matches!(
            self.record.state,
            RefreshTargetState::Running | RefreshTargetState::DirtyDuringRun
        )
    }
}

/// Read-only snapshot of every target record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshBrokerSnapshot {
    targets: Vec<RefreshTargetSnapshot>,
}

impl RefreshBrokerSnapshot {
    pub fn targets(&self) -> &[RefreshTargetSnapshot] {
        &self.targets
    }

    pub fn target(&self, target: &RefreshTarget) -> Option<&RefreshTargetSnapshot> {
        self.targets
            .iter()
            .find(|snapshot| snapshot.target() == target)
    }

    /// Distinct normalized targets known to the broker.
    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    /// Targets runnable right now. Bounded by [`Self::target_count`], never
    /// by the number of submitted events.
    pub fn queue_depth(&self) -> usize {
        self.targets
            .iter()
            .filter(|snapshot| snapshot.state() == RefreshTargetState::Queued)
            .count()
    }

    pub fn running_count(&self) -> usize {
        self.targets
            .iter()
            .filter(|snapshot| snapshot.is_running())
            .count()
    }
}

/// Host-wide broker handle rooted at a durable directory.
pub struct RefreshBroker {
    root: PathBuf,
    quiet_period: Duration,
    clock: Arc<dyn RefreshBrokerClock>,
}

impl std::fmt::Debug for RefreshBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshBroker")
            .field("root", &self.root)
            .field("quiet_period", &self.quiet_period)
            .finish()
    }
}

impl RefreshBroker {
    /// Open (and create when missing) the broker directory with the system
    /// clock.
    pub fn open(root: impl Into<PathBuf>, quiet_period: Duration) -> BrokerResult<Self> {
        Self::open_with_clock(root, quiet_period, Arc::new(SystemRefreshBrokerClock))
    }

    /// Open the broker with an injected clock (deterministic tests).
    pub fn open_with_clock(
        root: impl Into<PathBuf>,
        quiet_period: Duration,
        clock: Arc<dyn RefreshBrokerClock>,
    ) -> BrokerResult<Self> {
        let root = root.into();
        fs::create_dir_all(root.join(TARGETS_DIR_NAME))?;
        Ok(Self {
            root,
            quiet_period,
            clock,
        })
    }

    /// Open the default host-wide broker with the production quiet period.
    pub fn open_default() -> BrokerResult<Self> {
        Self::open(refresh_broker_root(), DEFAULT_REFRESH_QUIET_PERIOD)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn quiet_period(&self) -> Duration {
        self.quiet_period
    }

    fn now_millis(&self) -> u64 {
        self.clock.now_millis()
    }

    fn quiet_millis(&self) -> u64 {
        u64::try_from(self.quiet_period.as_millis()).unwrap_or(u64::MAX)
    }

    fn targets_dir(&self) -> PathBuf {
        self.root.join(TARGETS_DIR_NAME)
    }

    fn record_path(&self, stem: &str) -> PathBuf {
        self.targets_dir().join(format!("{stem}.json"))
    }

    fn state_lock_path(&self, stem: &str) -> PathBuf {
        self.targets_dir().join(format!("{stem}.state.lock"))
    }

    fn owner_lock_path(&self, stem: &str) -> PathBuf {
        self.targets_dir().join(format!("{stem}.owner.lock"))
    }

    /// Submit one intent. Events coalesce onto the target record; the
    /// returned snapshot is the record after coalescing.
    pub fn submit(&self, intent: RefreshIntent) -> BrokerResult<RefreshTargetSnapshot> {
        if intent.protocol_version != REFRESH_INTENT_PROTOCOL_VERSION {
            return Err(RefreshBrokerError::Protocol(format!(
                "unsupported refresh intent protocol version {} (expected {})",
                intent.protocol_version, REFRESH_INTENT_PROTOCOL_VERSION
            )));
        }
        let stem = intent.target.file_stem();
        let _state_guard = StateLockGuard::acquire(&self.state_lock_path(&stem))?;
        let record_path = self.record_path(&stem);
        let now = self.now_millis();
        let record = match self.read_record(&record_path)? {
            None => self.new_record(&intent, now),
            Some(existing) => self.coalesce(existing, &intent, now),
        };
        write_json_atomic(&record_path, &record)?;
        Ok(RefreshTargetSnapshot { record })
    }

    fn new_record(&self, intent: &RefreshIntent, now: u64) -> TargetRecord {
        let (state, deadline) = self.admission_for(intent.priority, now);
        TargetRecord {
            schema_version: REFRESH_BROKER_SCHEMA_VERSION,
            target: intent.target.clone(),
            desired_epoch: intent.desired_epoch,
            desired_snapshot: intent.desired_snapshot.clone(),
            priority: intent.priority,
            state,
            quiet_deadline_millis: deadline,
            running: None,
            follow_up_required: false,
            last_reason: intent.reason,
            resource_class: intent.resource_class,
            completed_epoch: None,
            last_error: None,
            updated_at_millis: now,
        }
    }

    /// Admission state for a fresh (or completed) target receiving `priority`.
    fn admission_for(&self, priority: JobPriority, now: u64) -> (RefreshTargetState, Option<u64>) {
        if is_urgent(priority) {
            (RefreshTargetState::Queued, None)
        } else {
            (
                RefreshTargetState::Quiet,
                Some(now.saturating_add(self.quiet_millis())),
            )
        }
    }

    fn coalesce(&self, mut record: TargetRecord, intent: &RefreshIntent, now: u64) -> TargetRecord {
        let newer = intent.desired_epoch > record.desired_epoch;
        if newer {
            record.desired_epoch = intent.desired_epoch;
            record.desired_snapshot = intent.desired_snapshot.clone();
        }
        let urgent = is_urgent(intent.priority);
        record.priority = more_urgent(record.priority, intent.priority);
        record.last_reason = intent.reason;
        record.resource_class = intent.resource_class;
        match record.state {
            RefreshTargetState::Running | RefreshTargetState::DirtyDuringRun => {
                let running_epoch = record.running.as_ref().map(|job| job.epoch).unwrap_or(0);
                if record.desired_epoch > running_epoch || (urgent && !record.follow_up_required) {
                    // FR-415: running keeps exactly one coalesced follow-up.
                    record.follow_up_required = true;
                    record.state = RefreshTargetState::DirtyDuringRun;
                }
            }
            RefreshTargetState::Quiet => {
                if urgent {
                    record.state = RefreshTargetState::Queued;
                    record.quiet_deadline_millis = None;
                } else if newer {
                    // Every later dirty event resets the full quiet period; a
                    // delayed older epoch leaves the deadline untouched.
                    record.quiet_deadline_millis = Some(now.saturating_add(self.quiet_millis()));
                }
            }
            RefreshTargetState::Queued => {}
            RefreshTargetState::Ready => {
                if newer || urgent {
                    let (state, deadline) = self.admission_for(record.priority, now);
                    record.state = state;
                    record.quiet_deadline_millis = deadline;
                }
            }
        }
        record.updated_at_millis = now;
        record
    }

    /// Read-only projection of every target. Never creates, appends, or
    /// rewrites durable bytes (AS-32).
    pub fn inspect(&self) -> BrokerResult<RefreshBrokerSnapshot> {
        let mut targets: Vec<RefreshTargetSnapshot> = self
            .list_records()?
            .into_iter()
            .map(|(_, record)| RefreshTargetSnapshot { record })
            .collect();
        targets.sort_by(|left, right| {
            left.record
                .target
                .file_stem()
                .cmp(&right.record.target.file_stem())
        });
        Ok(RefreshBrokerSnapshot { targets })
    }

    /// Claim the most urgent runnable target, if any.
    pub fn claim_next(&self) -> BrokerResult<Option<RefreshClaim>> {
        self.claim_next_where(|_| true)
    }

    /// Claim the most urgent runnable target accepted by `accept`. A target
    /// this process cannot execute (unknown project root, foreign repo) is
    /// left for another claimant without being touched.
    pub fn claim_next_where(
        &self,
        mut accept: impl FnMut(&RefreshTarget) -> bool,
    ) -> BrokerResult<Option<RefreshClaim>> {
        let now = self.now_millis();
        let mut candidates: Vec<(String, TargetRecord)> = self
            .list_records()?
            .into_iter()
            .filter(|(_, record)| is_claimable(record, now) && accept(&record.target))
            .collect();
        candidates.sort_by(|(left_stem, left), (right_stem, right)| {
            priority_rank(left.priority)
                .cmp(&priority_rank(right.priority))
                .then_with(|| {
                    left.quiet_deadline_millis
                        .unwrap_or(0)
                        .cmp(&right.quiet_deadline_millis.unwrap_or(0))
                })
                .then_with(|| left.updated_at_millis.cmp(&right.updated_at_millis))
                .then_with(|| left_stem.cmp(right_stem))
        });

        for (stem, _) in candidates {
            let owner_lock = open_lock_file(&self.owner_lock_path(&stem))?;
            match fs2::FileExt::try_lock_exclusive(&owner_lock) {
                Ok(()) => {}
                Err(error) if is_contended(&error) => continue,
                Err(error) => return Err(error.into()),
            }
            let claimed = {
                let _state_guard = StateLockGuard::acquire(&self.state_lock_path(&stem))?;
                let record_path = self.record_path(&stem);
                let mut record = self.read_record(&record_path)?;
                let now = self.now_millis();
                if let Some(record) = record.as_mut() {
                    if !is_claimable(record, now) {
                        None
                    } else {
                        let job = RunningJob {
                            epoch: record.desired_epoch,
                            snapshot: record.desired_snapshot.clone(),
                            priority: record.priority,
                            reason: record.last_reason,
                            resource_class: record.resource_class,
                            owner_pid: std::process::id(),
                            claimed_at_millis: now,
                        };
                        record.running = Some(job.clone());
                        record.state = RefreshTargetState::Running;
                        record.quiet_deadline_millis = None;
                        record.follow_up_required = false;
                        record.priority = JobPriority::Background;
                        record.updated_at_millis = now;
                        write_json_atomic(&record_path, record)?;
                        Some(RefreshIntent {
                            protocol_version: REFRESH_INTENT_PROTOCOL_VERSION,
                            target: record.target.clone(),
                            desired_epoch: job.epoch,
                            desired_snapshot: job.snapshot,
                            priority: job.priority,
                            reason: job.reason,
                            resource_class: job.resource_class,
                        })
                    }
                } else {
                    None
                }
            };
            match claimed {
                Some(intent) => {
                    return Ok(Some(RefreshClaim {
                        root: self.root.clone(),
                        quiet_period: self.quiet_period,
                        clock: self.clock.clone(),
                        stem,
                        intent,
                        owner_lock: Some(owner_lock),
                        settled: false,
                    }))
                }
                None => {
                    let _ = fs2::FileExt::unlock(&owner_lock);
                    continue;
                }
            }
        }
        Ok(None)
    }

    fn read_record(&self, path: &Path) -> BrokerResult<Option<TargetRecord>> {
        let raw = match fs::read(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let record: TargetRecord =
            serde_json::from_slice(&raw).map_err(|error| RefreshBrokerError::Corrupt {
                path: path.to_path_buf(),
                detail: error.to_string(),
            })?;
        if record.schema_version != REFRESH_BROKER_SCHEMA_VERSION {
            return Err(RefreshBrokerError::Corrupt {
                path: path.to_path_buf(),
                detail: format!("unsupported schema version {}", record.schema_version),
            });
        }
        Ok(Some(record))
    }

    fn list_records(&self) -> BrokerResult<Vec<(String, TargetRecord)>> {
        let targets_dir = self.targets_dir();
        let entries = match fs::read_dir(&targets_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            if stem.starts_with('.') {
                continue;
            }
            if let Some(record) = self.read_record(&entry.path())? {
                records.push((stem.to_string(), record));
            }
        }
        records.sort_by(|(left, _), (right, _)| left.cmp(right));
        Ok(records)
    }
}

fn is_urgent(priority: JobPriority) -> bool {
    priority != JobPriority::Background
}

/// Lower rank claims first.
fn priority_rank(priority: JobPriority) -> u8 {
    match priority {
        JobPriority::InteractiveSearch => 0,
        JobPriority::ManualRebuild => 1,
        JobPriority::Background => 2,
    }
}

fn more_urgent(left: JobPriority, right: JobPriority) -> JobPriority {
    if priority_rank(right) < priority_rank(left) {
        right
    } else {
        left
    }
}

/// A record is claimable when it is queued, its quiet deadline has passed,
/// or it is marked running by an owner whose kernel lock has been released
/// (crash residue). The owner lock, not this predicate, is the exclusion
/// truth.
fn is_claimable(record: &TargetRecord, now: u64) -> bool {
    match record.state {
        RefreshTargetState::Queued => true,
        RefreshTargetState::Quiet => record
            .quiet_deadline_millis
            .is_some_and(|deadline| deadline <= now),
        RefreshTargetState::Running | RefreshTargetState::DirtyDuringRun => true,
        RefreshTargetState::Ready => false,
    }
}

/// Owner handle for one claimed target. Dropping without
/// [`RefreshClaim::complete`] or [`RefreshClaim::fail`] re-queues the target
/// after a fresh quiet period so a crashed owner never wedges admission.
pub struct RefreshClaim {
    root: PathBuf,
    quiet_period: Duration,
    clock: Arc<dyn RefreshBrokerClock>,
    stem: String,
    intent: RefreshIntent,
    owner_lock: Option<File>,
    settled: bool,
}

impl std::fmt::Debug for RefreshClaim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshClaim")
            .field("stem", &self.stem)
            .field("intent", &self.intent)
            .finish()
    }
}

impl RefreshClaim {
    /// The coalesced intent this owner must execute.
    pub fn intent(&self) -> &RefreshIntent {
        &self.intent
    }

    pub fn target(&self) -> &RefreshTarget {
        &self.intent.target
    }

    /// Mark the claimed epoch complete. A newer epoch that arrived during
    /// the run becomes the single retained follow-up.
    pub fn complete(mut self) -> BrokerResult<()> {
        self.settle(None)
    }

    /// Mark the run failed. The target returns to quiet so a later dirty
    /// event or manual intent retries it; the message is kept for
    /// diagnostics.
    pub fn fail(mut self, message: impl Into<String>) -> BrokerResult<()> {
        self.settle(Some(message.into()))
    }

    fn settle(&mut self, error: Option<String>) -> BrokerResult<()> {
        let quiet_millis = u64::try_from(self.quiet_period.as_millis()).unwrap_or(u64::MAX);
        let record_path = self
            .root
            .join(TARGETS_DIR_NAME)
            .join(format!("{}.json", self.stem));
        let state_lock_path = self
            .root
            .join(TARGETS_DIR_NAME)
            .join(format!("{}.state.lock", self.stem));
        let result = (|| -> BrokerResult<()> {
            let _state_guard = StateLockGuard::acquire(&state_lock_path)?;
            let raw = fs::read(&record_path)?;
            let mut record: TargetRecord =
                serde_json::from_slice(&raw).map_err(|err| RefreshBrokerError::Corrupt {
                    path: record_path.clone(),
                    detail: err.to_string(),
                })?;
            let now = self.clock.now_millis();
            let running_epoch = record
                .running
                .as_ref()
                .map(|job| job.epoch)
                .unwrap_or(self.intent.desired_epoch);
            record.running = None;
            record.updated_at_millis = now;
            match error {
                None => {
                    record.completed_epoch = Some(running_epoch);
                    record.last_error = None;
                    let follow_up =
                        record.follow_up_required || record.desired_epoch > running_epoch;
                    if follow_up {
                        record.follow_up_required = true;
                        if is_urgent(record.priority) {
                            record.state = RefreshTargetState::Queued;
                            record.quiet_deadline_millis = None;
                        } else {
                            record.state = RefreshTargetState::Quiet;
                            record.quiet_deadline_millis = Some(now.saturating_add(quiet_millis));
                        }
                    } else {
                        record.follow_up_required = false;
                        record.state = RefreshTargetState::Ready;
                        record.quiet_deadline_millis = None;
                        record.priority = JobPriority::Background;
                    }
                }
                Some(message) => {
                    record.last_error = Some(message);
                    record.state = RefreshTargetState::Quiet;
                    record.quiet_deadline_millis = Some(now.saturating_add(quiet_millis));
                }
            }
            write_json_atomic(&record_path, &record)?;
            Ok(())
        })();
        self.settled = true;
        if let Some(lock) = self.owner_lock.take() {
            let _ = fs2::FileExt::unlock(&lock);
        }
        result
    }
}

impl Drop for RefreshClaim {
    fn drop(&mut self) {
        if !self.settled {
            let _ = self.settle(Some(
                "refresh owner released the claim without settling".to_string(),
            ));
        }
    }
}

/// Short-lived exclusive lock serializing read-modify-write of one record.
struct StateLockGuard {
    file: File,
}

impl StateLockGuard {
    fn acquire(path: &Path) -> io::Result<Self> {
        let file = open_lock_file(path)?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(Self { file })
    }
}

impl Drop for StateLockGuard {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
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

fn is_contended(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

/// Atomic JSON publish: sibling temp file then rename so readers never see a
/// torn record.
fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "record".to_string()),
        std::process::id()
    ));
    let payload = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(&tmp, payload)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(target: RefreshTarget, epoch: u64, priority: JobPriority) -> RefreshIntent {
        RefreshIntent {
            protocol_version: REFRESH_INTENT_PROTOCOL_VERSION,
            target,
            desired_epoch: epoch,
            desired_snapshot: format!("snapshot-{epoch}"),
            priority,
            reason: RefreshReason::DirtyEvent,
            resource_class: RefreshResourceClass::Embedding,
        }
    }

    #[test]
    fn target_scope_order_normalizes_to_one_identity() {
        let left =
            RefreshTarget::overlay("repo", "wt", [RefreshScope::Files, RefreshScope::FilesDocs]);
        let right = RefreshTarget::overlay(
            "repo",
            "wt",
            [
                RefreshScope::FilesDocs,
                RefreshScope::Files,
                RefreshScope::Files,
            ],
        );
        assert_eq!(left, right);
        assert_eq!(left.file_stem(), right.file_stem());
        assert_ne!(
            RefreshTarget::base("repo", [RefreshScope::Files]).file_stem(),
            left.file_stem()
        );
    }

    #[test]
    fn unsupported_protocol_version_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let broker = RefreshBroker::open(tmp.path().join("broker"), Duration::ZERO).expect("open");
        let mut bad = intent(
            RefreshTarget::base("repo", [RefreshScope::Files]),
            1,
            JobPriority::Background,
        );
        bad.protocol_version = REFRESH_INTENT_PROTOCOL_VERSION + 1;
        assert!(matches!(
            broker.submit(bad),
            Err(RefreshBrokerError::Protocol(_))
        ));
    }

    #[test]
    fn failed_run_returns_to_quiet_and_keeps_the_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let broker = RefreshBroker::open(tmp.path().join("broker"), Duration::ZERO).expect("open");
        let target = RefreshTarget::base("repo", [RefreshScope::Files, RefreshScope::FilesDocs]);
        broker
            .submit(intent(target.clone(), 1, JobPriority::Background))
            .expect("submit");
        let claim = broker.claim_next().expect("claim").expect("claimable");
        claim.fail("runner exploded").expect("fail");
        let snapshot = broker.inspect().expect("inspect");
        let state = snapshot.target(&target).expect("target");
        assert_eq!(state.state(), RefreshTargetState::Quiet);
        assert_eq!(state.last_error(), Some("runner exploded"));
        assert!(broker.claim_next().expect("claim again").is_some());
    }

    #[test]
    fn dropped_claim_is_reclaimable_after_quiet() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let broker = RefreshBroker::open(tmp.path().join("broker"), Duration::ZERO).expect("open");
        let target = RefreshTarget::base("repo", [RefreshScope::Files]);
        broker
            .submit(intent(target.clone(), 1, JobPriority::Background))
            .expect("submit");
        let claim = broker.claim_next().expect("claim").expect("claimable");
        drop(claim);
        assert_eq!(broker.inspect().expect("inspect").running_count(), 0);
        assert!(broker.claim_next().expect("reclaim").is_some());
    }

    #[test]
    fn claim_filter_leaves_foreign_targets_untouched() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let broker = RefreshBroker::open(tmp.path().join("broker"), Duration::ZERO).expect("open");
        let mine = RefreshTarget::base("repo-mine", [RefreshScope::Files]);
        let foreign = RefreshTarget::base("repo-foreign", [RefreshScope::Files]);
        broker
            .submit(intent(foreign.clone(), 1, JobPriority::InteractiveSearch))
            .expect("submit foreign");
        broker
            .submit(intent(mine.clone(), 1, JobPriority::Background))
            .expect("submit mine");
        let claim = broker
            .claim_next_where(|target| target.repo_hash() == "repo-mine")
            .expect("claim")
            .expect("own target claimable");
        assert_eq!(claim.target(), &mine);
        let snapshot = broker.inspect().expect("inspect");
        assert_eq!(
            snapshot.target(&foreign).expect("foreign").state(),
            RefreshTargetState::Queued
        );
        claim.complete().expect("complete");
    }
}
