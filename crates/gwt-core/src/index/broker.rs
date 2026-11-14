//! Durable, host-wide admission for file-index refresh work (SPEC #1939).
//!
//! The broker deliberately stores one record per normalized target. A single
//! ledger lock serializes state transitions across processes, while a second
//! per-target kernel lock remains held for the lifetime of a claim. The JSON
//! ledger is diagnostic and recovery state; the kernel lock is the ownership
//! truth.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::index_coordinator::JobPriority;

/// Version of the durable refresh-intent wire contract.
pub const REFRESH_INTENT_PROTOCOL_VERSION: u32 = 1;

/// Default quiet period for background file refreshes.
pub const DEFAULT_REFRESH_QUIET_PERIOD: Duration = Duration::from_secs(30);

const BROKER_LEDGER_SCHEMA_VERSION: u32 = 1;
const BROKER_DIR_NAME: &str = "refresh-broker";
const BROKER_LOCK_NAME: &str = "broker.lock";
const BROKER_STATE_NAME: &str = "state.json";
const WAITER_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// File-index scopes that are refreshed atomically for a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshScope {
    Files,
    FilesDocs,
}

impl RefreshScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::FilesDocs => "files-docs",
        }
    }
}

/// A logical refresh destination. Scope order and duplicates are normalized
/// at construction, so callers cannot create multiple queue entries for the
/// same scope set.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RefreshTarget {
    Base {
        repo_hash: String,
        scopes: Vec<RefreshScope>,
    },
    Overlay {
        repo_hash: String,
        worktree_hash: String,
        scopes: Vec<RefreshScope>,
    },
}

impl RefreshTarget {
    pub fn base(
        repo_hash: impl Into<String>,
        scopes: impl IntoIterator<Item = RefreshScope>,
    ) -> Self {
        Self::Base {
            repo_hash: repo_hash.into(),
            scopes: normalize_scopes(scopes),
        }
    }

    pub fn overlay(
        repo_hash: impl Into<String>,
        worktree_hash: impl Into<String>,
        scopes: impl IntoIterator<Item = RefreshScope>,
    ) -> Self {
        Self::Overlay {
            repo_hash: repo_hash.into(),
            worktree_hash: worktree_hash.into(),
            scopes: normalize_scopes(scopes),
        }
    }

    pub fn repo_hash(&self) -> &str {
        match self {
            Self::Base { repo_hash, .. } | Self::Overlay { repo_hash, .. } => repo_hash,
        }
    }

    pub fn worktree_hash(&self) -> Option<&str> {
        match self {
            Self::Base { .. } => None,
            Self::Overlay { worktree_hash, .. } => Some(worktree_hash),
        }
    }

    pub fn scopes(&self) -> &[RefreshScope] {
        match self {
            Self::Base { scopes, .. } | Self::Overlay { scopes, .. } => scopes,
        }
    }

    fn normalize(self) -> Self {
        match self {
            Self::Base { repo_hash, scopes } => Self::base(repo_hash, scopes),
            Self::Overlay {
                repo_hash,
                worktree_hash,
                scopes,
            } => Self::overlay(repo_hash, worktree_hash, scopes),
        }
    }

    fn lock_stem(&self) -> String {
        let mut digest = Sha256::new();
        match self {
            Self::Base { repo_hash, scopes } => {
                digest.update(b"base\0");
                digest.update(repo_hash.as_bytes());
                for scope in scopes {
                    digest.update(b"\0");
                    digest.update(scope.as_str().as_bytes());
                }
            }
            Self::Overlay {
                repo_hash,
                worktree_hash,
                scopes,
            } => {
                digest.update(b"overlay\0");
                digest.update(repo_hash.as_bytes());
                digest.update(b"\0");
                digest.update(worktree_hash.as_bytes());
                for scope in scopes {
                    digest.update(b"\0");
                    digest.update(scope.as_str().as_bytes());
                }
            }
        }
        hex::encode(digest.finalize())
    }
}

fn normalize_scopes(scopes: impl IntoIterator<Item = RefreshScope>) -> Vec<RefreshScope> {
    let mut scopes: Vec<_> = scopes.into_iter().collect();
    scopes.sort_unstable();
    scopes.dedup();
    scopes
}

/// Why a caller wants the target refreshed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshReason {
    DirtyEvent,
    Search,
    Manual,
    Startup,
    Repair,
}

/// Contended host resource required to execute an intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshResourceClass {
    Embedding,
}

/// Versioned request submitted by every refresh entrypoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshIntent {
    pub protocol_version: u32,
    pub target: RefreshTarget,
    pub desired_epoch: u64,
    pub desired_snapshot: String,
    pub priority: JobPriority,
    pub reason: RefreshReason,
    pub resource_class: RefreshResourceClass,
}

/// How a submitted intent affected its normalized logical target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshDisposition {
    Accepted,
    Joined,
    Promoted,
    Coalesced,
    AlreadyReady,
}

/// Compatibility spelling for callers that prefer the fully-qualified name.
pub type RefreshSubmissionDisposition = RefreshDisposition;

/// Durable identity returned for every refresh submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshSubmission {
    pub disposition: RefreshDisposition,
    pub target: RefreshTarget,
    pub logical_job_id: String,
}

impl RefreshSubmission {
    pub fn disposition(&self) -> RefreshDisposition {
        self.disposition
    }

    pub fn target(&self) -> &RefreshTarget {
        &self.target
    }

    pub fn logical_job_id(&self) -> &str {
        &self.logical_job_id
    }
}

/// Durable lifecycle observed by explicitly registered waiters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshLogicalJobStatus {
    Pending,
    Running,
    Completed,
}

pub type RefreshJobStatus = RefreshLogicalJobStatus;

impl RefreshLogicalJobStatus {
    pub fn is_terminal(self) -> bool {
        self == Self::Completed
    }
}

/// Durable state of one normalized target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshTargetState {
    Idle,
    Quiet,
    Queued,
    Running,
    DirtyDuringRun,
    Ready,
    Recovering,
}

impl Default for RefreshTargetState {
    fn default() -> Self {
        Self::Idle
    }
}

/// Clock seam used to make quiet-window admission deterministic in tests.
pub trait RefreshBrokerClock: Send + Sync {
    fn now_millis(&self) -> u64;
}

#[derive(Debug)]
struct SystemRefreshBrokerClock;

impl RefreshBrokerClock for SystemRefreshBrokerClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }
}

/// Cross-process refresh broker rooted at one host-wide durable directory.
#[derive(Clone)]
pub struct RefreshBroker {
    root: PathBuf,
    quiet_period_millis: u64,
    clock: Arc<dyn RefreshBrokerClock>,
}

impl RefreshBroker {
    pub fn open(root: impl AsRef<Path>, quiet_period: Duration) -> io::Result<Self> {
        Self::open_with_clock(root, quiet_period, Arc::new(SystemRefreshBrokerClock))
    }

    pub fn open_default() -> io::Result<Self> {
        Self::open(
            crate::paths::gwt_runtime_dir().join(BROKER_DIR_NAME),
            DEFAULT_REFRESH_QUIET_PERIOD,
        )
    }

    pub fn open_with_clock(
        root: impl AsRef<Path>,
        quiet_period: Duration,
        clock: Arc<dyn RefreshBrokerClock>,
    ) -> io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        open_lock_file(&root.join(BROKER_LOCK_NAME))?;
        Ok(Self {
            root,
            quiet_period_millis: duration_millis(quiet_period),
            clock,
        })
    }

    /// Coalesce one event into its target record. Older epochs and replayed
    /// snapshot identities can still promote priority, but cannot replace
    /// desired state or extend quiet.
    pub fn submit(&self, mut intent: RefreshIntent) -> io::Result<RefreshSubmission> {
        if intent.protocol_version != REFRESH_INTENT_PROTOCOL_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "unsupported refresh intent protocol version {}; expected {}",
                    intent.protocol_version, REFRESH_INTENT_PROTOCOL_VERSION
                ),
            ));
        }
        intent.target = intent.target.normalize();
        let now = self.clock.now_millis();
        let quiet_deadline = now.saturating_add(self.quiet_period_millis);

        self.mutate_ledger(|ledger| {
            let normalized_target = intent.target.clone();
            let Some(index) = ledger
                .targets
                .iter()
                .position(|record| record.intent.target == intent.target)
            else {
                let logical_job_id = new_logical_job_id();
                let state = initial_state(&intent);
                let sequence = ledger.take_sequence();
                let observed_epoch_high_water = intent.desired_epoch;
                ledger.targets.push(TargetRecord {
                    intent,
                    state,
                    quiet_deadline_millis: (state == RefreshTargetState::Quiet)
                        .then_some(quiet_deadline),
                    follow_up_required: false,
                    sequence,
                    logical_job_id: logical_job_id.clone(),
                    observed_epoch_high_water,
                });
                ledger.jobs.push(LogicalJobRecord {
                    logical_job_id: logical_job_id.clone(),
                    target: normalized_target.clone(),
                    status: RefreshLogicalJobStatus::Pending,
                });
                prune_completed_jobs(ledger);
                return Ok((
                    RefreshSubmission {
                        disposition: RefreshDisposition::Accepted,
                        target: normalized_target,
                        logical_job_id,
                    },
                    true,
                ));
            };

            let current = &ledger.targets[index];
            let desired_changed = desired_changed(current, &intent);
            let higher_priority = intent.priority < current.intent.priority;
            let high_water_changed = intent.desired_epoch > record_epoch_high_water(current);

            // Ready + unchanged desired identity is immutable and terminal.
            // Priority never turns already-produced work back into a job.
            if current.state == RefreshTargetState::Ready && !desired_changed {
                let logical_job_id = record_logical_job_id(current);
                if high_water_changed {
                    ledger.targets[index].observed_epoch_high_water = intent.desired_epoch;
                }
                return Ok((
                    RefreshSubmission {
                        disposition: RefreshDisposition::AlreadyReady,
                        target: normalized_target,
                        logical_job_id,
                    },
                    high_water_changed,
                ));
            }

            let was_running = matches!(
                current.state,
                RefreshTargetState::Running | RefreshTargetState::DirtyDuringRun
            );

            // A changed desired identity after terminal Ready starts a new
            // durable logical job. Every non-terminal update below retains
            // the current id, including a dirty follow-up while running.
            if current.state == RefreshTargetState::Ready && desired_changed {
                let old_job_id = record_logical_job_id(current);
                set_job_status(
                    &mut ledger.jobs,
                    old_job_id,
                    normalized_target.clone(),
                    RefreshLogicalJobStatus::Completed,
                );
                let logical_job_id = new_logical_job_id();
                let sequence = ledger.next_sequence;
                ledger.targets[index].observed_epoch_high_water = intent.desired_epoch;
                let _ = merge_intent(&mut ledger.targets[index], intent, quiet_deadline, sequence);
                ledger.targets[index].logical_job_id = logical_job_id.clone();
                ledger.next_sequence = ledger.next_sequence.saturating_add(1);
                ledger.jobs.push(LogicalJobRecord {
                    logical_job_id: logical_job_id.clone(),
                    target: normalized_target.clone(),
                    status: RefreshLogicalJobStatus::Pending,
                });
                prune_completed_jobs(ledger);
                return Ok((
                    RefreshSubmission {
                        disposition: RefreshDisposition::Accepted,
                        target: normalized_target,
                        logical_job_id,
                    },
                    true,
                ));
            }

            let logical_job_id = record_logical_job_id(&ledger.targets[index]);
            let mut bookkeeping_changed = false;
            if (desired_changed || higher_priority)
                && ledger.targets[index].logical_job_id.is_empty()
            {
                ledger.targets[index].logical_job_id = logical_job_id.clone();
                bookkeeping_changed = true;
            }
            if (desired_changed || higher_priority)
                && ensure_job_record(
                    &mut ledger.jobs,
                    logical_job_id.clone(),
                    normalized_target.clone(),
                    status_for_target_state(ledger.targets[index].state),
                )
            {
                bookkeeping_changed = true;
            }

            let sequence = ledger.next_sequence;
            if high_water_changed {
                ledger.targets[index].observed_epoch_high_water = intent.desired_epoch;
            }
            let state_changed =
                merge_intent(&mut ledger.targets[index], intent, quiet_deadline, sequence);
            if state_changed {
                ledger.next_sequence = ledger.next_sequence.saturating_add(1);
                set_job_status(
                    &mut ledger.jobs,
                    logical_job_id.clone(),
                    normalized_target.clone(),
                    status_for_target_state(ledger.targets[index].state),
                );
            }

            // Disposition is about the caller's relationship to the current
            // logical job: elevation wins, an active owner is joined, and
            // all other non-terminal duplicates coalesce in place.
            let disposition = if higher_priority {
                RefreshDisposition::Promoted
            } else if was_running {
                RefreshDisposition::Joined
            } else {
                RefreshDisposition::Coalesced
            };
            Ok((
                RefreshSubmission {
                    disposition,
                    target: normalized_target,
                    logical_job_id,
                },
                state_changed || bookkeeping_changed || high_water_changed,
            ))
        })
    }

    /// Read durable broker state without taking locks, creating files, or
    /// performing deadline/recovery transitions.
    pub fn inspect(&self) -> io::Result<RefreshBrokerSnapshot> {
        Self::inspect_at(&self.root)
    }

    /// Inspect an arbitrary broker root without creating the root, lock, or
    /// state file. A missing root is a valid empty broker.
    pub fn inspect_at(root: impl AsRef<Path>) -> io::Result<RefreshBrokerSnapshot> {
        Ok(snapshot_from_ledger(read_ledger(
            &root.as_ref().join(BROKER_STATE_NAME),
        )?))
    }

    /// Register one explicit durable waiter for a submitted logical job.
    /// Event submission itself never creates waiter records.
    pub fn join(&self, submission: &RefreshSubmission) -> io::Result<RefreshWaiter> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let request_id_for_ledger = request_id.clone();
        let waiter_lock_path = waiter_lock_path(&self.root, &request_id);
        let target = submission.target.clone().normalize();
        let logical_job_id = submission.logical_job_id.clone();
        let waiter_lock = self.mutate_ledger(|ledger| {
            let status = ledger
                .jobs
                .iter()
                .find(|job| job.logical_job_id == logical_job_id && job.target == target)
                .map(|job| job.status)
                .or_else(|| {
                    ledger
                        .targets
                        .iter()
                        .find(|record| {
                            record.intent.target == target
                                && record_logical_job_id(record) == logical_job_id
                        })
                        .map(|record| status_for_target_state(record.state))
                })
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "refresh submission is not present in this broker",
                    )
                })?;
            let waiter_lock = open_lock_file(&waiter_lock_path)?;
            FileExt::lock_exclusive(&waiter_lock)?;
            ensure_job_record(
                &mut ledger.jobs,
                logical_job_id.clone(),
                target.clone(),
                status,
            );
            ledger.waiters.push(WaiterRecord {
                request_id: request_id_for_ledger,
                logical_job_id: logical_job_id.clone(),
                target: target.clone(),
            });
            Ok((waiter_lock, true))
        })?;
        Ok(RefreshWaiter {
            broker: Some(self.clone()),
            request_id,
            logical_job_id,
            target,
            waiter_lock: Some(waiter_lock),
            waiter_lock_path,
        })
    }

    /// Remove inactive overlay targets after the owner has authoritatively
    /// enumerated the repo's active worktrees. Running targets, base targets,
    /// and targets with explicit waiters are never removed.
    pub fn prune_inactive_overlays<I, S>(
        &self,
        repo_hash: &str,
        active_worktree_hashes: I,
    ) -> io::Result<usize>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let active_worktree_hashes: HashSet<String> = active_worktree_hashes
            .into_iter()
            .map(|hash| hash.as_ref().to_string())
            .collect();
        let root = self.root.clone();
        self.mutate_ledger(|ledger| {
            let removable_targets: Vec<_> = ledger
                .targets
                .iter()
                .filter(|record| {
                    let RefreshTarget::Overlay {
                        repo_hash: target_repo,
                        worktree_hash,
                        ..
                    } = &record.intent.target
                    else {
                        return false;
                    };
                    target_repo == repo_hash
                        && !active_worktree_hashes.contains(worktree_hash)
                        && !matches!(
                            record.state,
                            RefreshTargetState::Running | RefreshTargetState::DirtyDuringRun
                        )
                        && !ledger
                            .waiters
                            .iter()
                            .any(|waiter| waiter.target == record.intent.target)
                })
                .map(|record| record.intent.target.clone())
                .collect();
            if removable_targets.is_empty() {
                return Ok((0, false));
            }
            ledger
                .targets
                .retain(|record| !removable_targets.contains(&record.intent.target));
            ledger
                .jobs
                .retain(|job| !removable_targets.contains(&job.target));
            let _ = prune_completed_jobs(ledger);
            // Remove lock files while the broker lock still excludes a new
            // submit/claim for the same target. Unlinking after commit could
            // otherwise race a new owner and split the kernel-lock identity.
            for target in &removable_targets {
                let _ = fs::remove_file(target_lock_path(&root, target));
            }
            Ok((removable_targets.len(), true))
        })
    }

    /// Claim the highest-priority runnable distinct target. The returned
    /// value retains the target's kernel lock until completion or drop.
    pub fn claim_next(&self) -> io::Result<Option<RefreshClaim>> {
        let now = self.clock.now_millis();
        let root = self.root.clone();
        let broker = self.clone();
        self.mutate_ledger(move |ledger| {
            let mut candidates: Vec<_> = ledger
                .targets
                .iter()
                .enumerate()
                .filter(|(_, record)| record.may_be_claimable(now))
                .map(|(index, record)| (index, record.intent.priority, record.sequence))
                .collect();
            candidates.sort_by_key(|(_, priority, sequence)| (*priority, *sequence));

            let mut recovered_state = false;
            for (index, _, _) in candidates {
                let owner_lock = open_lock_file(&target_lock_path(
                    &root,
                    &ledger.targets[index].intent.target,
                ))?;
                match FileExt::try_lock_exclusive(&owner_lock) {
                    Ok(()) => {}
                    Err(error) if is_contended(&error) => continue,
                    Err(error) => return Err(error),
                }

                let record = &mut ledger.targets[index];
                if record.state == RefreshTargetState::DirtyDuringRun
                    && record.intent.priority == JobPriority::Background
                    && record
                        .quiet_deadline_millis
                        .is_some_and(|deadline| deadline > now)
                {
                    // The previous process disappeared before completing its
                    // claim. Preserve the one dirty follow-up and its latest
                    // quiet deadline instead of admitting it early.
                    record.state = RefreshTargetState::Quiet;
                    recovered_state = true;
                    FileExt::unlock(&owner_lock)?;
                    continue;
                }

                let intent = record.intent.clone();
                let logical_job_id = if record.logical_job_id.is_empty() {
                    let derived = record_logical_job_id(record);
                    record.logical_job_id = derived.clone();
                    derived
                } else {
                    record.logical_job_id.clone()
                };
                record.state = RefreshTargetState::Running;
                record.quiet_deadline_millis = None;
                record.follow_up_required = false;
                set_job_status(
                    &mut ledger.jobs,
                    logical_job_id.clone(),
                    intent.target.clone(),
                    RefreshLogicalJobStatus::Running,
                );
                let claim = RefreshClaim {
                    broker: Some(broker),
                    intent,
                    logical_job_id,
                    owner_lock: Some(owner_lock),
                };
                return Ok((Some(claim), true));
            }
            Ok((None, recovered_state))
        })
    }

    fn finish(&self, target: &RefreshTarget, logical_job_id: &str) -> io::Result<()> {
        self.mutate_ledger(|ledger| {
            let index = ledger
                .targets
                .iter()
                .position(|record| &record.intent.target == target)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "claimed refresh target is absent from broker ledger",
                    )
                })?;
            let record = &mut ledger.targets[index];
            if record_logical_job_id(record) != logical_job_id {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "claimed refresh logical job no longer owns its target",
                ));
            }

            let job_status = if record.follow_up_required {
                if is_immediate(&record.intent) {
                    record.state = RefreshTargetState::Queued;
                    record.quiet_deadline_millis = None;
                } else {
                    record.state = RefreshTargetState::Quiet;
                    record.quiet_deadline_millis.get_or_insert_with(|| {
                        self.clock
                            .now_millis()
                            .saturating_add(self.quiet_period_millis)
                    });
                }
                RefreshLogicalJobStatus::Pending
            } else {
                record.state = RefreshTargetState::Ready;
                record.quiet_deadline_millis = None;
                RefreshLogicalJobStatus::Completed
            };
            set_job_status(
                &mut ledger.jobs,
                logical_job_id.to_string(),
                target.clone(),
                job_status,
            );
            Ok(((), true))
        })
    }

    fn latest_intent_for(
        &self,
        target: &RefreshTarget,
        logical_job_id: &str,
    ) -> io::Result<RefreshIntent> {
        let ledger = read_ledger(&self.root.join(BROKER_STATE_NAME))?;
        ledger
            .targets
            .iter()
            .find(|record| {
                &record.intent.target == target && record_logical_job_id(record) == logical_job_id
            })
            .map(|record| record.intent.clone())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "refresh logical job is absent from broker ledger",
                )
            })
    }

    fn job_status(&self, logical_job_id: &str) -> io::Result<RefreshLogicalJobStatus> {
        let ledger = read_ledger(&self.root.join(BROKER_STATE_NAME))?;
        if let Some(job) = ledger
            .jobs
            .iter()
            .find(|job| job.logical_job_id == logical_job_id)
        {
            return Ok(job.status);
        }
        ledger
            .targets
            .iter()
            .find(|record| record_logical_job_id(record) == logical_job_id)
            .map(|record| status_for_target_state(record.state))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "refresh logical job is absent from broker ledger",
                )
            })
    }

    fn remove_waiter(&self, request_id: &str) -> io::Result<()> {
        self.mutate_ledger(|ledger| {
            let before = ledger.waiters.len();
            ledger
                .waiters
                .retain(|waiter| waiter.request_id != request_id);
            prune_completed_jobs(ledger);
            Ok(((), ledger.waiters.len() != before))
        })
    }

    fn mutate_ledger<R>(
        &self,
        operation: impl FnOnce(&mut BrokerLedger) -> io::Result<(R, bool)>,
    ) -> io::Result<R> {
        let lock = open_lock_file(&self.root.join(BROKER_LOCK_NAME))?;
        FileExt::lock_exclusive(&lock)?;
        let result = (|| {
            let mut ledger = read_ledger(&self.root.join(BROKER_STATE_NAME))?;
            // Lock order is broker -> waiter. Sweep only tries waiter locks,
            // so a Drop holding waiter -> waiting for broker cannot deadlock.
            let swept_waiters = sweep_orphan_waiters(&self.root, &mut ledger);
            let (value, changed) = operation(&mut ledger)?;
            if changed || swept_waiters {
                write_json_atomic(&self.root.join(BROKER_STATE_NAME), &ledger)?;
            }
            Ok(value)
        })();
        let unlock_result = FileExt::unlock(&lock);
        match result {
            Ok(value) => {
                unlock_result?;
                Ok(value)
            }
            Err(error) => {
                let _ = unlock_result;
                Err(error)
            }
        }
    }
}

fn initial_state(intent: &RefreshIntent) -> RefreshTargetState {
    if is_immediate(intent) {
        RefreshTargetState::Queued
    } else {
        RefreshTargetState::Quiet
    }
}

fn is_immediate(intent: &RefreshIntent) -> bool {
    intent.priority != JobPriority::Background
        || matches!(intent.reason, RefreshReason::Manual | RefreshReason::Search)
}

fn merge_intent(
    record: &mut TargetRecord,
    incoming: RefreshIntent,
    quiet_deadline: u64,
    sequence: u64,
) -> bool {
    let desired_changed = desired_changed(record, &incoming);
    let higher_priority = incoming.priority < record.intent.priority;
    if !desired_changed && !higher_priority {
        return false;
    }

    // A completed target has no live promotion to retain. A changed desired
    // identity starts a fresh request with the incoming priority.
    if record.state == RefreshTargetState::Ready && desired_changed {
        record.intent = incoming;
        record.state = initial_state(&record.intent);
        record.quiet_deadline_millis =
            (record.state == RefreshTargetState::Quiet).then_some(quiet_deadline);
        record.follow_up_required = false;
        record.sequence = sequence;
        return true;
    }

    if desired_changed {
        record.intent.protocol_version = incoming.protocol_version;
        record.intent.desired_epoch = incoming.desired_epoch;
        record.intent.desired_snapshot = incoming.desired_snapshot.clone();
        record.intent.resource_class = incoming.resource_class;
        if incoming.priority <= record.intent.priority {
            record.intent.reason = incoming.reason;
        }
    }
    if higher_priority {
        record.intent.priority = incoming.priority;
        record.intent.reason = incoming.reason;
    }

    if !desired_changed {
        match record.state {
            RefreshTargetState::Quiet
            | RefreshTargetState::Idle
            | RefreshTargetState::Ready
            | RefreshTargetState::Recovering => {
                record.state = RefreshTargetState::Queued;
                record.quiet_deadline_millis = None;
                record.sequence = sequence;
            }
            RefreshTargetState::Queued
            | RefreshTargetState::Running
            | RefreshTargetState::DirtyDuringRun => {}
        }
        return true;
    }

    match record.state {
        RefreshTargetState::Running | RefreshTargetState::DirtyDuringRun => {
            record.state = RefreshTargetState::DirtyDuringRun;
            record.follow_up_required = true;
            if is_immediate(&record.intent) {
                record.quiet_deadline_millis = None;
            } else {
                record.quiet_deadline_millis = Some(quiet_deadline);
            }
        }
        RefreshTargetState::Quiet => {
            if is_immediate(&record.intent) {
                record.state = RefreshTargetState::Queued;
                record.quiet_deadline_millis = None;
                record.sequence = sequence;
            } else {
                record.quiet_deadline_millis = Some(quiet_deadline);
            }
        }
        RefreshTargetState::Queued => {}
        RefreshTargetState::Idle | RefreshTargetState::Ready | RefreshTargetState::Recovering => {
            record.state = initial_state(&record.intent);
            record.quiet_deadline_millis =
                (record.state == RefreshTargetState::Quiet).then_some(quiet_deadline);
            record.follow_up_required = false;
            record.sequence = sequence;
        }
    }
    true
}

fn desired_changed(record: &TargetRecord, incoming: &RefreshIntent) -> bool {
    incoming.desired_epoch >= record_epoch_high_water(record)
        && incoming.desired_snapshot != record.intent.desired_snapshot
}

fn record_epoch_high_water(record: &TargetRecord) -> u64 {
    record
        .observed_epoch_high_water
        .max(record.intent.desired_epoch)
}

/// A live target claim. Dropping it without completion releases the kernel
/// lock; a later broker claimant can recover the durable running record.
pub struct RefreshClaim {
    broker: Option<RefreshBroker>,
    intent: RefreshIntent,
    logical_job_id: String,
    owner_lock: Option<File>,
}

impl RefreshClaim {
    pub fn intent(&self) -> &RefreshIntent {
        &self.intent
    }

    pub fn logical_job_id(&self) -> &str {
        &self.logical_job_id
    }

    /// Re-read the current coalesced intent for this live logical job.
    pub fn latest_intent(&self) -> io::Result<RefreshIntent> {
        self.broker
            .as_ref()
            .ok_or_else(|| io::Error::other("refresh claim is already completed"))?
            .latest_intent_for(&self.intent.target, &self.logical_job_id)
    }

    pub fn effective_priority(&self) -> io::Result<JobPriority> {
        Ok(self.latest_intent()?.priority)
    }

    /// Prune inactive overlays through the broker that owns this live claim.
    /// The claim's intent, logical job, and target owner lock are unchanged.
    pub fn prune_inactive_overlays<I, S>(
        &self,
        repo_hash: &str,
        active_worktree_hashes: I,
    ) -> io::Result<usize>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.broker
            .as_ref()
            .ok_or_else(|| io::Error::other("refresh claim is already completed"))?
            .prune_inactive_overlays(repo_hash, active_worktree_hashes)
    }

    pub fn complete(mut self) -> io::Result<()> {
        let broker = self
            .broker
            .take()
            .expect("refresh claim broker must exist until completion");
        let result = broker.finish(&self.intent.target, &self.logical_job_id);
        let unlock_result = self
            .owner_lock
            .take()
            .map(|lock| FileExt::unlock(&lock))
            .unwrap_or(Ok(()));
        match result {
            Ok(()) => unlock_result,
            Err(error) => {
                let _ = unlock_result;
                Err(error)
            }
        }
    }
}

/// Explicit durable waiter for one logical refresh job.
pub struct RefreshWaiter {
    broker: Option<RefreshBroker>,
    request_id: String,
    logical_job_id: String,
    target: RefreshTarget,
    waiter_lock: Option<File>,
    waiter_lock_path: PathBuf,
}

impl RefreshWaiter {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn durable_request_id(&self) -> &str {
        &self.request_id
    }

    pub fn logical_job_id(&self) -> &str {
        &self.logical_job_id
    }

    pub fn target(&self) -> &RefreshTarget {
        &self.target
    }

    /// Read the durable job state without mutating waiter or owner state.
    pub fn status(&self) -> io::Result<RefreshLogicalJobStatus> {
        self.broker
            .as_ref()
            .ok_or_else(|| io::Error::other("refresh waiter is cancelled"))?
            .job_status(&self.logical_job_id)
    }

    pub fn wait(&self, timeout: Duration) -> io::Result<RefreshLogicalJobStatus> {
        let started = Instant::now();
        loop {
            let status = self.status()?;
            if status.is_terminal() {
                return Ok(status);
            }
            if started.elapsed() >= timeout {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "timed out waiting for refresh logical job {}",
                        self.logical_job_id
                    ),
                ));
            }
            std::thread::sleep(WAITER_POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())));
        }
    }

    /// Remove only this request's waiter registration. The job and its owner
    /// are deliberately unaffected.
    pub fn cancel(mut self) -> io::Result<()> {
        let Some(broker) = self.broker.take() else {
            self.release_waiter_lock();
            return Ok(());
        };
        let result = broker.remove_waiter(&self.request_id);
        self.release_waiter_lock();
        result
    }

    fn release_waiter_lock(&mut self) {
        if let Some(lock) = self.waiter_lock.take() {
            let _ = FileExt::unlock(&lock);
            drop(lock);
        }
        let _ = fs::remove_file(&self.waiter_lock_path);
    }
}

impl Drop for RefreshWaiter {
    fn drop(&mut self) {
        if let Some(broker) = self.broker.take() {
            let _ = broker.remove_waiter(&self.request_id);
        }
        self.release_waiter_lock();
    }
}

/// Immutable read projection returned by [`RefreshBroker::inspect`].
#[derive(Debug, Clone)]
pub struct RefreshBrokerSnapshot {
    targets: Vec<RefreshTargetSnapshot>,
    waiter_count: usize,
}

impl RefreshBrokerSnapshot {
    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    pub fn queue_depth(&self) -> usize {
        self.targets
            .iter()
            .filter(|target| target.state == RefreshTargetState::Queued)
            .count()
    }

    pub fn running_count(&self) -> usize {
        self.targets
            .iter()
            .filter(|target| {
                matches!(
                    target.state,
                    RefreshTargetState::Running
                        | RefreshTargetState::DirtyDuringRun
                        | RefreshTargetState::Recovering
                )
            })
            .count()
    }

    pub fn targets(&self) -> &[RefreshTargetSnapshot] {
        &self.targets
    }

    pub fn waiter_count(&self) -> usize {
        self.waiter_count
    }

    pub fn target(&self, target: &RefreshTarget) -> Option<&RefreshTargetSnapshot> {
        self.targets.iter().find(|state| &state.target == target)
    }
}

/// Read-only state of one broker target.
#[derive(Debug, Clone)]
pub struct RefreshTargetSnapshot {
    target: RefreshTarget,
    desired_epoch: u64,
    desired_snapshot: String,
    priority: JobPriority,
    state: RefreshTargetState,
    quiet_deadline_millis: Option<u64>,
    follow_up_count: usize,
    waiter_count: usize,
    logical_job_id: String,
    observed_epoch_high_water: u64,
}

impl RefreshTargetSnapshot {
    pub fn target(&self) -> &RefreshTarget {
        &self.target
    }

    pub fn desired_epoch(&self) -> u64 {
        self.desired_epoch
    }

    pub fn desired_snapshot(&self) -> &str {
        &self.desired_snapshot
    }

    pub fn priority(&self) -> JobPriority {
        self.priority
    }

    pub fn state(&self) -> RefreshTargetState {
        self.state
    }

    pub fn quiet_deadline_millis(&self) -> Option<u64> {
        self.quiet_deadline_millis
    }

    pub fn follow_up_count(&self) -> usize {
        usize::from(self.follow_up_count != 0)
    }

    pub fn waiter_count(&self) -> usize {
        self.waiter_count
    }

    pub fn logical_job_id(&self) -> &str {
        &self.logical_job_id
    }

    pub fn observed_epoch_high_water(&self) -> u64 {
        self.observed_epoch_high_water
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct BrokerLedger {
    schema_version: u32,
    next_sequence: u64,
    targets: Vec<TargetRecord>,
    #[serde(default)]
    jobs: Vec<LogicalJobRecord>,
    #[serde(default)]
    waiters: Vec<WaiterRecord>,
}

impl Default for BrokerLedger {
    fn default() -> Self {
        Self {
            schema_version: BROKER_LEDGER_SCHEMA_VERSION,
            next_sequence: 1,
            targets: Vec::new(),
            jobs: Vec::new(),
            waiters: Vec::new(),
        }
    }
}

impl BrokerLedger {
    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        sequence
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TargetRecord {
    intent: RefreshIntent,
    state: RefreshTargetState,
    quiet_deadline_millis: Option<u64>,
    follow_up_required: bool,
    sequence: u64,
    #[serde(default)]
    logical_job_id: String,
    #[serde(default)]
    observed_epoch_high_water: u64,
}

impl TargetRecord {
    fn may_be_claimable(&self, now: u64) -> bool {
        match self.state {
            RefreshTargetState::Queued | RefreshTargetState::Running => true,
            RefreshTargetState::Quiet | RefreshTargetState::DirtyDuringRun => self
                .quiet_deadline_millis
                .is_none_or(|deadline| deadline <= now),
            RefreshTargetState::Idle
            | RefreshTargetState::Ready
            | RefreshTargetState::Recovering => false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LogicalJobRecord {
    logical_job_id: String,
    target: RefreshTarget,
    status: RefreshLogicalJobStatus,
}

#[derive(Debug, Serialize, Deserialize)]
struct WaiterRecord {
    request_id: String,
    logical_job_id: String,
    target: RefreshTarget,
}

fn new_logical_job_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn record_logical_job_id(record: &TargetRecord) -> String {
    if !record.logical_job_id.is_empty() {
        return record.logical_job_id.clone();
    }
    // Old ledgers predate explicit job ids. Derive a stable read-only id so
    // AlreadyReady and inspect do not need to rewrite compatible state.
    let mut digest = Sha256::new();
    digest.update(b"legacy-refresh-job\0");
    digest.update(record.intent.target.lock_stem().as_bytes());
    digest.update(b"\0");
    digest.update(record.intent.desired_epoch.to_be_bytes());
    digest.update(b"\0");
    digest.update(record.intent.desired_snapshot.as_bytes());
    format!("legacy-{}", hex::encode(digest.finalize()))
}

fn status_for_target_state(state: RefreshTargetState) -> RefreshLogicalJobStatus {
    match state {
        RefreshTargetState::Ready => RefreshLogicalJobStatus::Completed,
        RefreshTargetState::Running
        | RefreshTargetState::DirtyDuringRun
        | RefreshTargetState::Recovering => RefreshLogicalJobStatus::Running,
        RefreshTargetState::Idle | RefreshTargetState::Quiet | RefreshTargetState::Queued => {
            RefreshLogicalJobStatus::Pending
        }
    }
}

fn ensure_job_record(
    jobs: &mut Vec<LogicalJobRecord>,
    logical_job_id: String,
    target: RefreshTarget,
    status: RefreshLogicalJobStatus,
) -> bool {
    if let Some(job) = jobs
        .iter_mut()
        .find(|job| job.logical_job_id == logical_job_id)
    {
        if job.target == target && job.status == status {
            return false;
        }
        job.target = target;
        job.status = status;
        return true;
    }
    jobs.push(LogicalJobRecord {
        logical_job_id,
        target,
        status,
    });
    true
}

fn set_job_status(
    jobs: &mut Vec<LogicalJobRecord>,
    logical_job_id: String,
    target: RefreshTarget,
    status: RefreshLogicalJobStatus,
) {
    let _ = ensure_job_record(jobs, logical_job_id, target, status);
}

/// Retain current jobs and completed history that still has an explicit
/// durable waiter. Superseded completed jobs with no observer are bounded
/// away at the generation transition or when their final waiter departs.
fn prune_completed_jobs(ledger: &mut BrokerLedger) -> bool {
    let current_job_ids: Vec<_> = ledger.targets.iter().map(record_logical_job_id).collect();
    let waiter_job_ids: Vec<_> = ledger
        .waiters
        .iter()
        .map(|waiter| waiter.logical_job_id.clone())
        .collect();
    let before = ledger.jobs.len();
    ledger.jobs.retain(|job| {
        job.status != RefreshLogicalJobStatus::Completed
            || current_job_ids.contains(&job.logical_job_id)
            || waiter_job_ids.contains(&job.logical_job_id)
    });
    ledger.jobs.len() != before
}

fn sweep_orphan_waiters(root: &Path, ledger: &mut BrokerLedger) -> bool {
    let mut orphan_request_ids = HashSet::new();
    for waiter in &ledger.waiters {
        let path = waiter_lock_path(root, &waiter.request_id);
        let Ok(lock) = open_lock_file(&path) else {
            continue;
        };
        match FileExt::try_lock_exclusive(&lock) {
            Ok(()) => {
                let _ = FileExt::unlock(&lock);
                drop(lock);
                let _ = fs::remove_file(path);
                orphan_request_ids.insert(waiter.request_id.clone());
            }
            Err(error) if is_contended(&error) => {}
            Err(_) => {}
        }
    }
    let ledger_changed = !orphan_request_ids.is_empty();
    if ledger_changed {
        ledger
            .waiters
            .retain(|waiter| !orphan_request_ids.contains(&waiter.request_id));
        let _ = prune_completed_jobs(ledger);
    }
    sweep_unreferenced_waiter_lock_files(root, ledger);
    ledger_changed
}

fn sweep_unreferenced_waiter_lock_files(root: &Path, ledger: &BrokerLedger) {
    let referenced_request_ids: HashSet<_> = ledger
        .waiters
        .iter()
        .map(|waiter| waiter.request_id.as_str())
        .collect();
    let Ok(entries) = fs::read_dir(root.join("waiters")) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(request_id) = file_name.strip_suffix(".lock") else {
            continue;
        };
        let Ok(parsed_request_id) = uuid::Uuid::parse_str(request_id) else {
            continue;
        };
        if parsed_request_id.to_string() != request_id
            || referenced_request_ids.contains(request_id)
        {
            continue;
        }

        let path = entry.path();
        let Ok(lock) = OpenOptions::new().read(true).write(true).open(&path) else {
            continue;
        };
        if !lock
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == 0)
        {
            continue;
        }
        match FileExt::try_lock_exclusive(&lock) {
            Ok(()) => {
                let _ = FileExt::unlock(&lock);
                drop(lock);
                let _ = fs::remove_file(path);
            }
            Err(error) if is_contended(&error) => {}
            Err(_) => {}
        }
    }
}

fn snapshot_from_ledger(ledger: BrokerLedger) -> RefreshBrokerSnapshot {
    let BrokerLedger {
        targets: records,
        waiters,
        ..
    } = ledger;
    let waiter_count = waiters.len();
    let targets = records
        .into_iter()
        .map(|record| {
            let logical_job_id = record_logical_job_id(&record);
            let observed_epoch_high_water = record_epoch_high_water(&record);
            let target_waiter_count = waiters
                .iter()
                .filter(|waiter| waiter.target == record.intent.target)
                .count();
            RefreshTargetSnapshot {
                target: record.intent.target,
                desired_epoch: record.intent.desired_epoch,
                desired_snapshot: record.intent.desired_snapshot,
                priority: record.intent.priority,
                state: record.state,
                quiet_deadline_millis: record.quiet_deadline_millis,
                follow_up_count: usize::from(record.follow_up_required),
                waiter_count: target_waiter_count,
                logical_job_id,
                observed_epoch_high_water,
            }
        })
        .collect();
    RefreshBrokerSnapshot {
        targets,
        waiter_count,
    }
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn target_lock_path(root: &Path, target: &RefreshTarget) -> PathBuf {
    root.join("owners")
        .join(format!("{}.lock", target.lock_stem()))
}

fn waiter_lock_path(root: &Path, request_id: &str) -> PathBuf {
    root.join("waiters").join(format!("{request_id}.lock"))
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

fn read_ledger(path: &Path) -> io::Result<BrokerLedger> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BrokerLedger::default()),
        Err(error) => return Err(error),
    };
    let ledger: BrokerLedger = serde_json::from_slice(&bytes).map_err(json_error)?;
    if ledger.schema_version != BROKER_LEDGER_SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported refresh broker ledger schema {}; expected {}",
                ledger.schema_version, BROKER_LEDGER_SCHEMA_VERSION
            ),
        ));
    }
    validate_ledger(&ledger)?;
    Ok(ledger)
}

fn validate_ledger(ledger: &BrokerLedger) -> io::Result<()> {
    for (index, record) in ledger.targets.iter().enumerate() {
        if record.intent.protocol_version != REFRESH_INTENT_PROTOCOL_VERSION {
            return Err(invalid_ledger(
                "target state uses an unsupported refresh intent protocol",
            ));
        }
        if record.intent.target.clone().normalize() != record.intent.target {
            return Err(invalid_ledger("refresh target scope set is not normalized"));
        }
        if record.observed_epoch_high_water != 0
            && record.observed_epoch_high_water < record.intent.desired_epoch
        {
            return Err(invalid_ledger("refresh epoch high-water regressed"));
        }
        if ledger.targets[..index]
            .iter()
            .any(|earlier| earlier.intent.target == record.intent.target)
        {
            return Err(invalid_ledger("duplicate refresh target record"));
        }
        if !record.logical_job_id.is_empty()
            && ledger.targets[..index]
                .iter()
                .any(|earlier| earlier.logical_job_id == record.logical_job_id)
        {
            return Err(invalid_ledger("duplicate current logical job id"));
        }
        let invalid_state = match record.state {
            RefreshTargetState::Quiet => record.quiet_deadline_millis.is_none(),
            RefreshTargetState::Queued => record.quiet_deadline_millis.is_some(),
            RefreshTargetState::Running => {
                record.quiet_deadline_millis.is_some() || record.follow_up_required
            }
            RefreshTargetState::DirtyDuringRun => !record.follow_up_required,
            RefreshTargetState::Ready => {
                record.quiet_deadline_millis.is_some() || record.follow_up_required
            }
            RefreshTargetState::Idle | RefreshTargetState::Recovering => true,
        };
        if invalid_state {
            return Err(invalid_ledger(
                "refresh target state invariants are invalid",
            ));
        }
    }
    for (index, job) in ledger.jobs.iter().enumerate() {
        if job.target.clone().normalize() != job.target
            || job.logical_job_id.is_empty()
            || ledger.jobs[..index]
                .iter()
                .any(|earlier| earlier.logical_job_id == job.logical_job_id)
        {
            return Err(invalid_ledger("invalid or duplicate logical job id"));
        }
    }
    for record in &ledger.targets {
        let logical_job_id = record_logical_job_id(record);
        let job = ledger
            .jobs
            .iter()
            .find(|job| job.logical_job_id == logical_job_id);
        if !record.logical_job_id.is_empty() && job.is_none() {
            return Err(invalid_ledger("current logical job record is missing"));
        }
        if let Some(job) = job {
            if job.target != record.intent.target
                || job.status != status_for_target_state(record.state)
            {
                return Err(invalid_ledger(
                    "logical job metadata does not match target state",
                ));
            }
        }
    }
    for job in &ledger.jobs {
        if job.status != RefreshLogicalJobStatus::Completed
            && !ledger.targets.iter().any(|record| {
                record_logical_job_id(record) == job.logical_job_id
                    && record.intent.target == job.target
            })
        {
            return Err(invalid_ledger(
                "nonterminal logical job has no current target",
            ));
        }
    }
    for (index, waiter) in ledger.waiters.iter().enumerate() {
        if waiter.request_id.is_empty()
            || uuid::Uuid::parse_str(&waiter.request_id).is_err()
            || waiter.logical_job_id.is_empty()
            || ledger.waiters[..index]
                .iter()
                .any(|earlier| earlier.request_id == waiter.request_id)
        {
            return Err(invalid_ledger("invalid or duplicate refresh waiter id"));
        }
        let referenced_target = ledger
            .jobs
            .iter()
            .find(|job| job.logical_job_id == waiter.logical_job_id)
            .map(|job| &job.target)
            .or_else(|| {
                ledger
                    .targets
                    .iter()
                    .find(|record| record_logical_job_id(record) == waiter.logical_job_id)
                    .map(|record| &record.intent.target)
            });
        if referenced_target != Some(&waiter.target) {
            return Err(invalid_ledger(
                "refresh waiter references an unknown logical job",
            ));
        }
    }
    Ok(())
}

fn invalid_ledger(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("refresh-broker"),
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let bytes = serde_json::to_vec(value).map_err(json_error)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        replace_path(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn replace_path(temporary: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temporary: Vec<u16> = temporary.as_os_str().encode_wide().chain([0]).collect();
    let destination: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: both buffers are live, NUL-terminated UTF-16 paths. The flags
    // replace the destination in one filesystem operation and wait for the
    // durable write, so lock-free readers never observe a delete gap.
    unsafe {
        MoveFileExW(
            PCWSTR(temporary.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| io::Error::other(error.to_string()))
}

#[cfg(not(windows))]
fn replace_path(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)?;
    if let Some(parent) = destination.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(all(not(unix), not(windows)))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn json_error(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}
