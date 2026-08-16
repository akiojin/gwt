//! Execution Control Record (SPEC-3248 P8a, FR-033/FR-034).
//!
//! Every Execution launch with a linked owner — SPEC or plain Issue, from
//! Issue Monitor or Start Work — materializes a worktree-local Execution
//! Control Record **before prompt injection** (T-107). The record makes the
//! execution lifecycle machine-visible independent of skill state: the Stop
//! gate (`hook/execution_control_stop_check`) keeps the session working until
//! the record is settled, even when the agent never called `build.start`
//! (T-108/T-109, AS-30), so a plain-Issue `$gwt-fix-issue` launch cannot
//! bypass the lifecycle that `$gwt-build-spec` follows.
//!
//! Settlement is explicit: `execution.complete` marks the execution done,
//! `execution.blocked` records a terminal blocked exit with the blocker
//! reason and missing verification (blocked is not done, AS-26 analog).
//! `build.complete` also settles the record for build-spec flows. Both
//! settlement paths bind to the current `GWT_SESSION_ID` and refuse another
//! session's record (T-100 semantics — note the pre-existing `build.complete`
//! owner-only check is intentionally left unchanged for skill state).
//!
//! Scope notes (dependent follow-ups, phase contract T-263):
//! - The authoritative copy lives in the repo-scoped trusted store (P9b,
//!   T-172/T-173-lite); `.gwt/skill-state/execution-control.json` is the
//!   human-inspectable mirror. Integrity hashes and audited ownership
//!   transfer are P9a. A fresh relaunch takes over with a fresh active
//!   record; a resume preserves an existing settled record for the same
//!   owner.
//! - Read-modify-write cycles (launch materialization, settlement,
//!   adoption) run under the owner write lease
//!   (`trusted_store::with_write_lease`, T-149): a second concurrent gwt
//!   writer gets an explicit-retry refusal instead of last-writer-wins.

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Utc};
use gwt_github::{client::ApiError, SpecOpsError};
use serde::{Deserialize, Serialize};

use super::CliEnv;

/// Worktree-relative path of the Execution Control Record's mirror (the
/// authoritative copy lives in the repo-scoped trusted store, P9b).
pub const EXECUTION_CONTROL_STATE_RELATIVE: &str = ".gwt/skill-state/execution-control.json";
/// Worktree-local mirror of the trusted link from a worktree projection to
/// its owner-scoped generation. The link deliberately lives outside the flat
/// ECR schema: an older typed writer may strip unknown ECR fields, but cannot
/// silently rewrite this independently hashed record.
pub const EXECUTION_GENERATION_POINTER_STATE_RELATIVE: &str =
    ".gwt/skill-state/execution-generation-pointer.json";
const RECOVERY_ENVELOPE_PREFIX: &str = "gwt:execution-recovery:v1:";
const GENERATION_LEDGER_SCHEMA_VERSION: u32 = 1;
const GENERATION_LEDGER_FILE: &str = "generation-ledger.json";
const GENERATION_POINTER_FILE: &str = "execution-generation-pointer.json";
const EXECUTION_REPAIR_AUDIT_FILE: &str = "execution-repair-audit.json";
#[doc(hidden)]
pub const BINDING_REPAIR_OUTCOME_FILE: &str = "binding-repair-outcome.json";
const BINDING_REPAIR_OUTCOME_SCHEMA_VERSION: u32 = 1;
const BINDING_REPAIR_OPERATION_ID: &str = "continue-work-local-repair";
const GENERATION_BINDING_MISMATCH_PREFIX: &str = "generation settlement binding mismatch:";
const RECOVERY_SESSION_CHANGED_PREFIX: &str = "execution_recovery_session_changed:";
const ACTIVE_BINDING_LEASE_WAIT: Duration = Duration::from_secs(2);

#[cfg(test)]
#[derive(Debug, Clone)]
enum RecoverySessionRace {
    Delete,
    Replace(Box<gwt_agent::Session>),
    Corrupt,
}

#[cfg(test)]
type RepairOwnerActivationRace = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
std::thread_local! {
    static RECOVERY_SESSION_RACE:
        std::cell::RefCell<Option<RecoverySessionRace>> = const { std::cell::RefCell::new(None) };
    static REPAIR_BINDING_SESSION_RACE:
        std::cell::RefCell<Option<RecoverySessionRace>> = const { std::cell::RefCell::new(None) };
    static REPAIR_OWNER_ACTIVATION_RACE:
        std::cell::RefCell<Option<RepairOwnerActivationRace>> = const { std::cell::RefCell::new(None) };
    static REPAIR_BINDING_AUTHORITY_RACE:
        std::cell::RefCell<Option<ExecutionOwnerKey>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_recovery_session_race(race: RecoverySessionRace) {
    RECOVERY_SESSION_RACE.with(|slot| {
        assert!(
            slot.borrow_mut().replace(race).is_none(),
            "recovery Session race injection must not be nested"
        );
    });
}

#[cfg(test)]
fn set_repair_binding_session_race(race: RecoverySessionRace) {
    REPAIR_BINDING_SESSION_RACE.with(|slot| {
        assert!(
            slot.borrow_mut().replace(race).is_none(),
            "repair binding Session race injection must not be nested"
        );
    });
}

#[cfg(test)]
fn set_repair_owner_activation_race(race: impl FnOnce(&Path) + 'static) {
    REPAIR_OWNER_ACTIVATION_RACE.with(|slot| {
        assert!(
            slot.borrow_mut().replace(Box::new(race)).is_none(),
            "repair owner activation race injection must not be nested"
        );
    });
}

#[cfg(test)]
fn set_repair_binding_authority_race(owner: ExecutionOwnerKey) {
    REPAIR_BINDING_AUTHORITY_RACE.with(|slot| {
        assert!(
            slot.borrow_mut().replace(owner).is_none(),
            "repair binding authority race injection must not be nested"
        );
    });
}

#[cfg(test)]
fn inject_recovery_session_race_if_requested(session_id: &str) {
    let race = RECOVERY_SESSION_RACE.with(|slot| slot.borrow_mut().take());
    let Some(race) = race else {
        return;
    };
    inject_recovery_session_race(session_id, race);
}

#[cfg(test)]
fn inject_repair_binding_session_race_if_requested(session_id: &str) {
    let race = REPAIR_BINDING_SESSION_RACE.with(|slot| slot.borrow_mut().take());
    let Some(race) = race else {
        return;
    };
    inject_recovery_session_race(session_id, race);
}

#[cfg(test)]
fn inject_repair_owner_activation_race_if_requested(worktree: &Path) {
    let race = REPAIR_OWNER_ACTIVATION_RACE.with(|slot| slot.borrow_mut().take());
    if let Some(race) = race {
        race(worktree);
    }
}

#[cfg(test)]
fn inject_repair_binding_authority_race_if_requested(worktree: &Path) -> io::Result<()> {
    let owner = REPAIR_BINDING_AUTHORITY_RACE.with(|slot| slot.borrow_mut().take());
    let Some(owner) = owner else {
        return Ok(());
    };
    let context = GenerationTransactionContext::resolve(worktree, owner)?;
    let now = Utc::now();
    let mut record = ExecutionControlRecord {
        owner_kind: owner.kind,
        owner_number: owner.number,
        primary_session_id: "session-foreign-generation-race".to_string(),
        entrypoint: "test-foreign-generation-race".to_string(),
        bundled_required_owners: Vec::new(),
        status: ExecutionControlStatus::Active,
        blocked_reason: None,
        missing_verification: None,
        launched_at: now,
        settled_at: None,
        transfers: Vec::new(),
        recoveries: Vec::new(),
        content_hash: String::new(),
    };
    let projection = String::from_utf8(serialize_execution_control(&record)?).map_err(|error| {
        invalid_generation_data(format!("foreign race projection is not UTF-8: {error}"))
    })?;
    record = serde_json::from_str(&projection).map_err(|error| {
        invalid_generation_data(format!("foreign race projection is malformed: {error}"))
    })?;
    let generation_id = format!("gen-foreign-race-{}", uuid::Uuid::new_v4().simple());
    let mut generation = ExecutionGeneration {
        identity: ExecutionGenerationIdentity {
            owner,
            generation_id: generation_id.clone(),
            predecessor_generation_id: None,
            predecessor_content_hash: None,
            session_binding_id: format!("foreign-race-{}", uuid::Uuid::new_v4().simple()),
            initial_session_id: record.primary_session_id.clone(),
            worktree_binding_hash: context.worktree_binding_hash.clone(),
            entrypoint: record.entrypoint.clone(),
            activated_at: now,
        },
        status: ExecutionControlStatus::Active,
        execution_control_json: projection.clone(),
        content_hash: String::new(),
    };
    generation.content_hash = compute_generation_hash(&generation);
    let mut ledger = ExecutionGenerationLedger {
        schema_version: GENERATION_LEDGER_SCHEMA_VERSION,
        owner,
        generations: vec![generation],
        continuation_attempts: Vec::new(),
        takeover_attempts: Vec::new(),
        takeovers: Vec::new(),
        lifecycle_events: Vec::new(),
        continuation_validations: Vec::new(),
        current_generation_id: generation_id,
        content_hash: String::new(),
    };
    stamp_generation_ledger(&mut ledger);
    write_activated_generation(&context, &ledger, &projection)
}

#[cfg(test)]
fn inject_recovery_session_race(session_id: &str, race: RecoverySessionRace) {
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let session_path = sessions_dir.join(format!("{session_id}.toml"));
    match race {
        RecoverySessionRace::Delete => {
            std::fs::remove_file(&session_path).expect("inject durable Session deletion");
        }
        RecoverySessionRace::Replace(replacement) => {
            assert_eq!(replacement.id, session_id);
            replacement
                .save(&sessions_dir)
                .expect("inject durable Session replacement");
        }
        RecoverySessionRace::Corrupt => {
            std::fs::write(&session_path, b"broken = [")
                .expect("inject durable Session corruption");
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenerationWriteFailurePoint {
    AfterLedger,
    AfterProjection,
}

#[cfg(test)]
std::thread_local! {
    static GENERATION_WRITE_FAILURE:
        std::cell::Cell<Option<GenerationWriteFailurePoint>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
std::thread_local! {
    static CONTINUATION_VALIDATION_WRITE_FAILURE: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn set_continuation_validation_write_failure() {
    CONTINUATION_VALIDATION_WRITE_FAILURE.with(|slot| {
        assert!(
            !slot.replace(true),
            "continuation validation write failure injection must not be nested"
        );
    });
}

#[cfg(test)]
fn fail_continuation_validation_write_if_requested() -> io::Result<()> {
    CONTINUATION_VALIDATION_WRITE_FAILURE.with(|slot| {
        if slot.replace(false) {
            Err(io::Error::other(
                "injected continuation validation write failure",
            ))
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
fn set_generation_write_failure(point: GenerationWriteFailurePoint) {
    GENERATION_WRITE_FAILURE.with(|slot| {
        assert!(
            slot.replace(Some(point)).is_none(),
            "generation write failure injection must not be nested"
        );
    });
}

#[cfg(test)]
pub(crate) fn set_generation_write_failure_after_ledger() {
    set_generation_write_failure(GenerationWriteFailurePoint::AfterLedger);
}

#[cfg(test)]
fn fail_generation_write_if_requested(point: GenerationWriteFailurePoint) -> io::Result<()> {
    GENERATION_WRITE_FAILURE.with(|slot| {
        if slot.get() == Some(point) {
            slot.set(None);
            return Err(io::Error::other(format!(
                "injected generation write failure at {point:?}"
            )));
        }
        Ok(())
    })
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContinuationRebindFailurePoint {
    BeforePrepareCommit,
    BeforeActivationCommit,
}

#[cfg(test)]
std::thread_local! {
    static CONTINUATION_REBIND_FAILURE:
        std::cell::Cell<Option<ContinuationRebindFailurePoint>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_continuation_rebind_failure_before_prepare_commit() {
    CONTINUATION_REBIND_FAILURE.with(|slot| {
        assert!(
            slot.replace(Some(ContinuationRebindFailurePoint::BeforePrepareCommit))
                .is_none(),
            "continuation rebind failure injection must not be nested"
        );
    });
}

#[cfg(test)]
pub(crate) fn set_continuation_rebind_failure_before_activation_commit() {
    CONTINUATION_REBIND_FAILURE.with(|slot| {
        assert!(
            slot.replace(Some(ContinuationRebindFailurePoint::BeforeActivationCommit))
                .is_none(),
            "continuation rebind failure injection must not be nested"
        );
    });
}

#[cfg(test)]
fn fail_continuation_rebind_if_requested(point: ContinuationRebindFailurePoint) -> io::Result<()> {
    CONTINUATION_REBIND_FAILURE.with(|slot| {
        if slot.get() == Some(point) {
            slot.set(None);
            return Err(io::Error::other(format!(
                "injected continuation rebind failure at {point:?}"
            )));
        }
        Ok(())
    })
}

#[cfg(test)]
std::thread_local! {
    static REPAIR_AUDIT_WRITE_FAILURE: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn set_repair_audit_write_failure() {
    REPAIR_AUDIT_WRITE_FAILURE.with(|slot| {
        assert!(
            !slot.replace(true),
            "repair audit write failure injection must not be nested"
        );
    });
}

#[cfg(test)]
fn fail_repair_audit_write_if_requested() -> io::Result<()> {
    REPAIR_AUDIT_WRITE_FAILURE.with(|slot| {
        if slot.replace(false) {
            Err(io::Error::other("injected repair audit write failure"))
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
std::thread_local! {
    static REPAIR_QUARANTINE_FAILURE_AFTER:
        std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn set_repair_quarantine_failure_after(quarantined_count: usize) {
    REPAIR_QUARANTINE_FAILURE_AFTER.with(|slot| {
        assert!(
            slot.replace(Some(quarantined_count)).is_none(),
            "repair quarantine failure injection must not be nested"
        );
    });
}

#[cfg(test)]
fn fail_repair_quarantine_if_requested(quarantined_count: usize) -> io::Result<()> {
    REPAIR_QUARANTINE_FAILURE_AFTER.with(|slot| {
        if slot.get() == Some(quarantined_count) {
            slot.set(None);
            return Err(io::Error::other(format!(
                "injected repair quarantine failure after {quarantined_count} sources"
            )));
        }
        Ok(())
    })
}

/// Linked owner kind. A `gwt-spec`-labeled Issue is a SPEC owner; everything
/// else is a plain Issue owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOwnerKind {
    Spec,
    Issue,
}

impl ExecutionOwnerKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::Issue => "issue",
        }
    }
}

/// Lifecycle state of one Execution launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionControlStatus {
    Active,
    Completed,
    Blocked,
}

/// One audited ownership transfer (SPEC-3248 P9a, T-117/T-123): who held the
/// execution, who took it over, why, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnershipTransfer {
    pub from_session_id: String,
    pub to_session_id: String,
    pub reason: String,
    pub transferred_at: DateTime<Utc>,
}

/// One audited recovery of a terminal Blocked execution (FR-196): the
/// blocker and trusted evidence that justified returning the same owning
/// session to Active state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionRecovery {
    pub session_id: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_missing_verification: Option<String>,
    pub blocked_at: DateTime<Utc>,
    pub verification_record_id: String,
    pub verification_run_hash: String,
    pub verification_plan_hash: String,
    pub verification_plan_created_at: DateTime<Utc>,
    pub plan_derived: bool,
    pub worktree_fingerprint: String,
    pub verification_started_at: DateTime<Utc>,
    pub verification_created_at: DateTime<Utc>,
    pub reopened_at: DateTime<Utc>,
    /// Hash of the preceding recovery entry, or empty for the first entry.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub previous_recovery_hash: String,
    /// Integrity hash over this recovery entry with `content_hash` emptied.
    /// Recovery history is an extension ignored by pre-recovery binaries, so
    /// it carries its own append-only hash chain instead of changing the
    /// rolling-compatible ECR body hash.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
}

/// The Execution Control Record (T-106).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionControlRecord {
    pub owner_kind: ExecutionOwnerKind,
    pub owner_number: u64,
    /// The gwt session id (`GWT_SESSION_ID`) this execution was launched for.
    pub primary_session_id: String,
    /// How the session was started: the `$gwt-*` prompt token when the launch
    /// carried one, `resume` for resumed sessions, `launch` otherwise.
    pub entrypoint: String,
    /// Bundled-required owners copied from the Primary owner's plan (empty
    /// until intake classification materializes them — FR-033).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bundled_required_owners: Vec<u64>,
    pub status: ExecutionControlStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_verification: Option<String>,
    pub launched_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<DateTime<Utc>>,
    /// Audited ownership transfer chain (P9a, T-117/T-123): every takeover —
    /// `execution.adopt`, launch takeover, resume takeover — appends here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transfers: Vec<OwnershipTransfer>,
    /// Append-only recovery chain. Ownership transfers and same-session
    /// terminal-state recovery are distinct audit concepts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recoveries: Vec<ExecutionRecovery>,
    /// Integrity hash over the record content (P9a, T-119/T-122 core):
    /// sha256 of the canonical serialization with this field emptied. Every
    /// canonical writer recomputes it; gates reject records whose stored
    /// hash does not match (naive direct edits). Empty = legacy pre-P9a
    /// record, accepted for one release cycle so in-flight worktrees keep
    /// working (sunset is a dependent follow-up; the PreToolUse direct-write
    /// guard independently blocks agent edits to this file).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
}

/// Stable owner key within one canonical repository identity.
///
/// Repository identity is represented by the parent trusted-store directory.
/// The Primary owner number selects storage; `kind` is validated metadata so
/// label/cache drift cannot fork `issue-N` and `spec-N` ledgers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionOwnerKey {
    pub kind: ExecutionOwnerKind,
    pub number: u64,
}

impl ExecutionOwnerKey {
    fn storage_key(self) -> String {
        format!("owner-{}", self.number)
    }
}

/// Canonical paths and identity captured once for one owner-ledger
/// transaction. Generation authority must never fall back to worktree-local
/// storage or re-resolve through a retargeted `origin` while a lease is held.
#[derive(Debug, Clone)]
struct GenerationTransactionContext {
    worktree: PathBuf,
    worktree_binding_hash: String,
    worktree_trusted_dir: PathBuf,
    owner_dir: PathBuf,
    owner: ExecutionOwnerKey,
}

impl GenerationTransactionContext {
    fn resolve(worktree: &Path, owner: ExecutionOwnerKey) -> io::Result<Self> {
        validate_owner(owner)?;
        let worktree = dunce::canonicalize(worktree).map_err(|error| {
            io::Error::new(
                ErrorKind::InvalidInput,
                format!("execution generation worktree cannot be canonicalized: {error}"),
            )
        })?;
        let Some(worktree_trusted_dir) =
            crate::cli::trusted_store::trusted_dir_for_worktree(&worktree)
        else {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "execution generation authority requires a canonical repository identity",
            ));
        };
        let trusted_root = worktree_trusted_dir.parent().ok_or_else(|| {
            invalid_generation_data(
                "trusted worktree directory has no repository-scoped trusted parent",
            )
        })?;
        let owner_dir = trusted_root
            .join("execution-owners")
            .join(owner.storage_key());
        Ok(Self {
            worktree_binding_hash: worktree_binding_hash(&worktree),
            worktree,
            worktree_trusted_dir,
            owner_dir,
            owner,
        })
    }

    fn validate_unchanged(&self) -> io::Result<()> {
        let current_worktree = dunce::canonicalize(&self.worktree).map_err(|error| {
            generation_conflict(format!(
                "execution generation worktree identity changed during transaction: {error}"
            ))
        })?;
        let current_trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(
            &current_worktree,
        )
        .ok_or_else(|| {
            generation_conflict(
                "canonical repository identity disappeared during generation transaction",
            )
        })?;
        if current_worktree != self.worktree
            || current_trusted_dir != self.worktree_trusted_dir
            || worktree_binding_hash(&current_worktree) != self.worktree_binding_hash
        {
            return Err(generation_conflict(
                "canonical repository/worktree identity changed during generation transaction",
            ));
        }
        Ok(())
    }
}

/// Immutable identity/header for one Execution generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionGenerationIdentity {
    pub owner: ExecutionOwnerKey,
    pub generation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor_generation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor_content_hash: Option<String>,
    /// Opaque, non-secret binding identifier. Capability material must never
    /// be persisted in the ledger.
    pub session_binding_id: String,
    pub initial_session_id: String,
    pub worktree_binding_hash: String,
    pub entrypoint: String,
    pub activated_at: DateTime<Utc>,
}

/// Immutable snapshot created at generation activation/import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionGeneration {
    pub identity: ExecutionGenerationIdentity,
    pub status: ExecutionControlStatus,
    /// Exact canonical ECR projection at import/activation time. In
    /// particular, terminal legacy imports retain these bytes unchanged.
    pub execution_control_json: String,
    /// Hash over this generation with this field emptied.
    pub content_hash: String,
}

/// Input identity for an idempotent successor operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessorRequest {
    pub operation_id: String,
    pub principal_id: String,
    /// Optional Work correlation carried by user-facing Continue work.
    ///
    /// Generic generation writers leave this absent. New Continue work
    /// operations persist it so a Host restart cannot retarget an operation
    /// id to another Work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_id: Option<String>,
    pub source: String,
    pub session_binding_id: String,
    pub initial_session_id: String,
    pub entrypoint: String,
    pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationAttemptStatus {
    Prepared,
    Aborted,
    Activated,
}

/// Terminal state a successor operation is explicitly authorized to leave.
/// Ordinary Continue work remains Completed-only. The Blocked variant is
/// reserved for an explicit linked-owner launch that creates a fresh lifetime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuccessorPredecessorStatus {
    Active,
    #[default]
    Completed,
    Blocked,
}

pub const FRESH_LINKED_OWNER_LAUNCH_SOURCE: &str = "fresh-linked-owner-launch";
pub const MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE: &str = "manual-completed-owner-launch";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExactSessionRuntimeDisposition {
    Terminal(gwt_agent::ManualLaunchRuntimeProof),
    Defunct(gwt_agent::ManualLaunchRuntimeProof),
    Live,
    Unknown,
}

/// Classify exact runtime evidence without treating a dead host PID as child
/// exit proof. A terminal result names the exact PID namespace and process
/// incarnation that the later owner/Session transaction must revalidate.
pub fn classify_exact_session_runtime(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionIdentity,
) -> io::Result<ExactSessionRuntimeDisposition> {
    let runtime_root = sessions_dir.join("runtime");
    let entries = match fs::read_dir(&runtime_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(ExactSessionRuntimeDisposition::Unknown)
        }
        Err(error) => return Err(error),
    };
    let mut terminal = None;
    let mut defunct = None;
    let mut saw_unknown = false;
    for namespace in entries {
        let namespace = namespace?;
        let Some(host_pid) = namespace
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        let sidecar = namespace
            .path()
            .join(format!("{}.json", expected.session_id));
        match sidecar.try_exists() {
            Ok(false) => continue,
            Ok(true) => {}
            Err(error) => return Err(error),
        }
        let runtime = match gwt_agent::SessionRuntimeState::load(&sidecar) {
            Ok(runtime) => runtime,
            Err(_) => {
                saw_unknown = true;
                continue;
            }
        };
        if runtime.execution_identity.as_ref() != Some(expected) {
            saw_unknown = true;
            continue;
        }
        let Some(runtime_incarnation) = runtime.runtime_incarnation.filter(|value| *value > 0)
        else {
            saw_unknown = true;
            continue;
        };
        let proof = gwt_agent::ManualLaunchRuntimeProof {
            host_pid,
            runtime_incarnation,
        };
        let runtime_is_terminal = matches!(
            runtime.status,
            gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
        );
        if runtime_is_terminal {
            match (runtime.child_pid, runtime.child_started_at) {
                (Some(child_pid), Some(child_started_at))
                    if child_pid > 0 && child_started_at > 0 =>
                {
                    if crate::process::exact_pty_process_tree_is_alive(child_pid, child_started_at)
                    {
                        return Ok(ExactSessionRuntimeDisposition::Live);
                    }
                }
                (None, None) => {
                    saw_unknown = true;
                    continue;
                }
                _ => {
                    saw_unknown = true;
                    continue;
                }
            }
        } else {
            let Some(host_started_at) = runtime.host_started_at.filter(|value| *value > 0) else {
                saw_unknown = true;
                continue;
            };
            let Some((child_pid, child_started_at)) = runtime
                .child_pid
                .zip(runtime.child_started_at)
                .filter(|(pid, started_at)| *pid > 0 && *started_at > 0)
            else {
                saw_unknown = true;
                continue;
            };
            if crate::process::host_process_start_time(host_pid) == Some(host_started_at)
                || crate::process::exact_pty_process_tree_is_alive(child_pid, child_started_at)
            {
                return Ok(ExactSessionRuntimeDisposition::Live);
            }
            let handoff = fs::read(gwt_agent::manual_handoff_path(
                sessions_dir,
                &expected.session_id,
            ))
            .ok()
            .and_then(|bytes| {
                serde_json::from_slice::<gwt_agent::SessionManualHandoffFence>(&bytes).ok()
            });
            if handoff.as_ref().is_none_or(|handoff| {
                handoff.execution_identity != *expected
                    || handoff.host_pid != host_pid
                    || handoff.host_started_at != host_started_at
            }) {
                saw_unknown = true;
                continue;
            }
            if defunct.replace(proof).is_some() {
                return Ok(ExactSessionRuntimeDisposition::Unknown);
            }
            continue;
        }
        if terminal.replace(proof).is_some() {
            return Ok(ExactSessionRuntimeDisposition::Unknown);
        }
    }
    if saw_unknown || (terminal.is_some() && defunct.is_some()) {
        return Ok(ExactSessionRuntimeDisposition::Unknown);
    }
    if let Some(proof) = defunct {
        return Ok(ExactSessionRuntimeDisposition::Defunct(proof));
    }
    Ok(
        terminal.map_or(ExactSessionRuntimeDisposition::Unknown, |proof| {
            ExactSessionRuntimeDisposition::Terminal(proof)
        }),
    )
}

pub fn is_owner_launch_successor_attempt(attempt: &ContinuationAttempt) -> bool {
    attempt.request.work_id.is_none()
        && matches!(
            (attempt.predecessor_status, attempt.request.source.as_str()),
            (
                SuccessorPredecessorStatus::Blocked,
                FRESH_LINKED_OWNER_LAUNCH_SOURCE
            ) | (
                SuccessorPredecessorStatus::Completed,
                MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE
            )
        )
}

fn is_completed_successor_status(status: &SuccessorPredecessorStatus) -> bool {
    *status == SuccessorPredecessorStatus::Completed
}

/// Append-only successor-attempt event. Prepared/Aborted events are audit
/// records only and can never become the ledger's current generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuationAttempt {
    pub request: SuccessorRequest,
    #[serde(default, skip_serializing_if = "is_completed_successor_status")]
    pub predecessor_status: SuccessorPredecessorStatus,
    /// Canonical target worktree captured at Prepared time. This is part of
    /// the persisted request envelope and fences every later replay.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub worktree_binding_hash: String,
    pub predecessor: ExecutionGenerationIdentity,
    pub predecessor_generation_content_hash: String,
    pub candidate_generation_id: String,
    pub status: ContinuationAttemptStatus,
    pub recorded_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activated_generation: Option<ExecutionGenerationIdentity>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub previous_attempt_hash: String,
    pub content_hash: String,
}

/// Input identity for an idempotent same-generation stale-owner takeover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationTakeoverRequest {
    pub operation_id: String,
    pub principal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_id: Option<String>,
    /// `continue-work:resume` or `continue-work:handoff` for user-facing
    /// takeover operations. Generic ownership transfers leave it absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub from_session_id: String,
    pub to_session_id: String,
    pub reason: String,
    pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationTakeoverAttemptStatus {
    Prepared,
    Aborted,
    Activated,
}

/// Append-only CAS audit for a same-generation takeover. Prepared/Aborted
/// entries do not alter the current generation or its effective projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationTakeoverAttempt {
    pub request: GenerationTakeoverRequest,
    pub worktree_binding_hash: String,
    pub generation_id: String,
    pub predecessor_head_hash: String,
    pub status: GenerationTakeoverAttemptStatus,
    pub recorded_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activated_binding: Option<gwt_agent::ExecutionBindingIdentity>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub previous_attempt_hash: String,
    pub content_hash: String,
}

/// Audited stale-owner transfer that keeps the same generation identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationTakeoverAudit {
    pub sequence: u64,
    pub generation_id: String,
    pub from_session_id: String,
    pub to_session_id: String,
    pub reason: String,
    pub observed_at: DateTime<Utc>,
    pub execution_control_json: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub previous_event_hash: String,
    pub content_hash: String,
}

/// Append-only lifecycle transition for one immutable generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationLifecycleEvent {
    pub sequence: u64,
    pub generation_id: String,
    pub from_status: ExecutionControlStatus,
    pub to_status: ExecutionControlStatus,
    pub session_id: String,
    pub reason: String,
    /// Opaque provenance for a Host-owned lifecycle operation that must be
    /// distinguished from an ordinary agent settlement. Absent for legacy
    /// and agent-authored transitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub execution_control_json: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub previous_event_hash: String,
    pub content_hash: String,
}

/// Durable read-after-write proof that a Host continuation rebound the
/// current generation without creating a successor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContinuationValidationAudit {
    pub operation_id: String,
    pub session_id: String,
    pub generation_id: String,
    pub execution_binding: gwt_agent::ExecutionBindingIdentity,
    pub capability_generation: u64,
    pub recorded_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub previous_audit_hash: String,
    pub content_hash: String,
}

/// Versioned owner-scoped append-only generation ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionGenerationLedger {
    pub schema_version: u32,
    pub owner: ExecutionOwnerKey,
    pub generations: Vec<ExecutionGeneration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub continuation_attempts: Vec<ContinuationAttempt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub takeover_attempts: Vec<GenerationTakeoverAttempt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub takeovers: Vec<GenerationTakeoverAudit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_events: Vec<GenerationLifecycleEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub continuation_validations: Vec<ExecutionContinuationValidationAudit>,
    pub current_generation_id: String,
    pub content_hash: String,
}

impl ExecutionGenerationLedger {
    #[must_use]
    pub fn current_generation(&self) -> Option<&ExecutionGeneration> {
        self.generations
            .iter()
            .find(|generation| generation.identity.generation_id == self.current_generation_id)
    }

    fn lifecycle_events_for<'a>(
        &'a self,
        generation_id: &'a str,
    ) -> impl Iterator<Item = &'a GenerationLifecycleEvent> + 'a {
        self.lifecycle_events
            .iter()
            .filter(move |event| event.generation_id == generation_id)
    }

    fn effective_status_for(&self, generation: &ExecutionGeneration) -> ExecutionControlStatus {
        self.lifecycle_events_for(&generation.identity.generation_id)
            .max_by_key(|event| event.sequence)
            .map_or(generation.status, |event| event.to_status)
    }

    fn effective_projection_for<'a>(&'a self, generation: &'a ExecutionGeneration) -> &'a str {
        let lifecycle = self
            .lifecycle_events_for(&generation.identity.generation_id)
            .map(|event| (event.sequence, event.execution_control_json.as_str()));
        let takeovers = self
            .takeovers
            .iter()
            .filter(|event| event.generation_id == generation.identity.generation_id)
            .map(|event| (event.sequence, event.execution_control_json.as_str()));
        lifecycle
            .chain(takeovers)
            .max_by_key(|(sequence, _)| *sequence)
            .map_or(generation.execution_control_json.as_str(), |(_, json)| json)
    }

    #[must_use]
    pub fn current_effective_status(&self) -> Option<ExecutionControlStatus> {
        self.current_generation()
            .map(|generation| self.effective_status_for(generation))
    }

    fn next_generation_event_sequence(&self) -> u64 {
        self.lifecycle_events
            .iter()
            .map(|event| event.sequence)
            .chain(self.takeovers.iter().map(|event| event.sequence))
            .max()
            .unwrap_or(0)
            + 1
    }
}

/// Caller-provided classification for a verified legacy Active record.
///
/// Unknown (and every hashless Active record) is a zero-mutation,
/// backfill-required refusal. A stale takeover appends audit data while
/// retaining the imported generation identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyActiveDisposition {
    Live,
    Stale {
        new_session_id: String,
        reason: String,
        observed_at: DateTime<Utc>,
    },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExecutionGenerationPointer {
    schema_version: u32,
    owner: ExecutionOwnerKey,
    current_generation_id: String,
    current_generation_content_hash: String,
    projection_content_hash: String,
    content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExecutionRepairSource {
    source_path: String,
    quarantine_path: String,
    source_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    source_path_os_bytes_hex: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    quarantine_path_os_bytes_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QuarantinedExecutionAuthority {
    source_path: PathBuf,
    quarantine_path: PathBuf,
    source_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepairSessionSnapshot {
    id: String,
    worktree_path: PathBuf,
    project_state_root: Option<PathBuf>,
    repo_hash: Option<String>,
    branch: String,
    agent_id: gwt_agent::AgentId,
    linked_issue_number: Option<u64>,
    execution_binding: Option<gwt_agent::SessionExecutionBinding>,
}

impl From<&gwt_agent::Session> for RepairSessionSnapshot {
    fn from(session: &gwt_agent::Session) -> Self {
        Self {
            id: session.id.clone(),
            worktree_path: session.worktree_path.clone(),
            project_state_root: session.project_state_root.clone(),
            repo_hash: session.repo_hash.clone(),
            branch: session.branch.clone(),
            agent_id: session.agent_id.clone(),
            linked_issue_number: session.linked_issue_number,
            execution_binding: session.execution_binding.clone(),
        }
    }
}

impl QuarantinedExecutionAuthority {
    fn audit_source(&self) -> ExecutionRepairSource {
        ExecutionRepairSource {
            source_path: self.source_path.to_string_lossy().into_owned(),
            quarantine_path: self.quarantine_path.to_string_lossy().into_owned(),
            source_hash: self.source_hash.clone(),
            source_path_os_bytes_hex: hex::encode(self.source_path.as_os_str().as_encoded_bytes()),
            quarantine_path_os_bytes_hex: hex::encode(
                self.quarantine_path.as_os_str().as_encoded_bytes(),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExecutionRepairAudit {
    repair_id: String,
    actor_session_id: String,
    owner: ExecutionOwnerKey,
    reason: String,
    sources: Vec<ExecutionRepairSource>,
    new_generation_id: String,
    repaired_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    previous_audit_hash: String,
    content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ExecutionRepairOutcome {
    status: &'static str,
    owner: ExecutionOwnerKey,
    generation_id: String,
    repair_id: String,
    quarantined: Vec<ExecutionRepairSource>,
    binding_repaired: bool,
    warnings: Vec<String>,
}

/// Compute the integrity hash for a record (content with the hash field
/// emptied).
#[must_use]
pub fn compute_content_hash(record: &ExecutionControlRecord) -> String {
    use sha2::{Digest, Sha256};
    let canonical = recovery_storage_projection(record);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("{:x}", Sha256::digest(&bytes))
}

fn compute_legacy_hash_with_recoveries(record: &ExecutionControlRecord) -> String {
    use sha2::{Digest, Sha256};
    let mut canonical = record.clone();
    canonical.content_hash = String::new();
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("{:x}", Sha256::digest(&bytes))
}

#[must_use]
fn compute_recovery_hash(recovery: &ExecutionRecovery) -> String {
    use sha2::{Digest, Sha256};
    let mut canonical = recovery.clone();
    canonical.content_hash = String::new();
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("{:x}", Sha256::digest(&bytes))
}

fn stamp_recovery_chain(recoveries: &mut [ExecutionRecovery]) {
    let mut previous = String::new();
    for recovery in recoveries {
        recovery.previous_recovery_hash.clone_from(&previous);
        recovery.content_hash = compute_recovery_hash(recovery);
        previous.clone_from(&recovery.content_hash);
    }
}

fn recovery_envelope(recovery: &ExecutionRecovery) -> OwnershipTransfer {
    OwnershipTransfer {
        from_session_id: recovery.session_id.clone(),
        to_session_id: recovery.session_id.clone(),
        reason: format!(
            "{RECOVERY_ENVELOPE_PREFIX}{}",
            serde_json::to_string(recovery).unwrap_or_default()
        ),
        transferred_at: recovery.reopened_at,
    }
}

fn is_recovery_envelope_transfer(transfer: &OwnershipTransfer) -> bool {
    transfer.from_session_id == transfer.to_session_id
        && transfer.reason.starts_with(RECOVERY_ENVELOPE_PREFIX)
}

fn recovery_storage_projection(record: &ExecutionControlRecord) -> ExecutionControlRecord {
    let mut canonical = record.clone();
    stamp_recovery_chain(&mut canonical.recoveries);
    canonical
        .transfers
        .retain(|transfer| !is_recovery_envelope_transfer(transfer));
    let mut transfers = canonical
        .recoveries
        .iter()
        .map(recovery_envelope)
        .collect::<Vec<_>>();
    transfers.append(&mut canonical.transfers);
    canonical.transfers = transfers;
    canonical.recoveries.clear();
    canonical.content_hash = String::new();
    canonical
}

fn hydrate_recovery_envelopes(mut record: ExecutionControlRecord) -> ExecutionControlRecord {
    let mut transfers = Vec::with_capacity(record.transfers.len());
    let mut recoveries = Vec::new();
    let mut malformed = false;
    let mut saw_regular_transfer = false;
    for transfer in record.transfers {
        if !is_recovery_envelope_transfer(&transfer) {
            saw_regular_transfer = true;
            transfers.push(transfer);
            continue;
        }
        let raw = transfer
            .reason
            .strip_prefix(RECOVERY_ENVELOPE_PREFIX)
            .unwrap_or_default();
        if saw_regular_transfer {
            malformed = true;
        }
        match serde_json::from_str::<ExecutionRecovery>(raw) {
            Ok(recovery)
                if transfer.from_session_id == recovery.session_id
                    && transfer.transferred_at == recovery.reopened_at =>
            {
                recoveries.push(recovery);
            }
            Err(_) => malformed = true,
            Ok(_) => malformed = true,
        }
    }
    if !recoveries.is_empty() {
        if record.recoveries.is_empty() {
            record.recoveries = recoveries;
        } else {
            malformed = true;
        }
    }
    record.transfers = transfers;
    if malformed {
        record.content_hash = format!("invalid-recovery-envelope:{}", record.content_hash);
    }
    record
}

fn recovery_chain_integrity_ok(recoveries: &[ExecutionRecovery]) -> bool {
    let mut previous = "";
    for recovery in recoveries {
        if recovery.content_hash.is_empty()
            || recovery.previous_recovery_hash != previous
            || recovery.content_hash != compute_recovery_hash(recovery)
        {
            return false;
        }
        previous = &recovery.content_hash;
    }
    true
}

/// True when the stored integrity hash matches the content (or the record is
/// a legacy pre-P9a record without one).
#[must_use]
pub fn integrity_ok(record: &ExecutionControlRecord) -> bool {
    if record.content_hash.is_empty() {
        return record.recoveries.is_empty();
    }
    if record.content_hash == compute_content_hash(record) {
        return recovery_chain_integrity_ok(&record.recoveries);
    }
    // One in-flight development record may have been written by the initial
    // recovery implementation, whose ECR hash still included the extension
    // and whose recovery entries had no individual hashes. Accept it only as
    // a migration source; the next canonical save upgrades it.
    record.content_hash == compute_legacy_hash_with_recoveries(record)
        && record.recoveries.iter().all(|recovery| {
            recovery.previous_recovery_hash.is_empty() && recovery.content_hash.is_empty()
        })
}

fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

fn compute_generation_hash(generation: &ExecutionGeneration) -> String {
    let mut canonical = generation.clone();
    canonical.content_hash.clear();
    sha256_hex(serde_json::to_vec(&canonical).unwrap_or_default())
}

fn compute_continuation_attempt_hash(attempt: &ContinuationAttempt) -> String {
    let mut canonical = attempt.clone();
    canonical.content_hash.clear();
    sha256_hex(serde_json::to_vec(&canonical).unwrap_or_default())
}

fn compute_generation_takeover_attempt_hash(attempt: &GenerationTakeoverAttempt) -> String {
    let mut canonical = attempt.clone();
    canonical.content_hash.clear();
    sha256_hex(serde_json::to_vec(&canonical).unwrap_or_default())
}

fn compute_takeover_event_hash(event: &GenerationTakeoverAudit) -> String {
    let mut canonical = event.clone();
    canonical.content_hash.clear();
    sha256_hex(serde_json::to_vec(&canonical).unwrap_or_default())
}

fn compute_lifecycle_event_hash(event: &GenerationLifecycleEvent) -> String {
    let mut canonical = event.clone();
    canonical.content_hash.clear();
    sha256_hex(serde_json::to_vec(&canonical).unwrap_or_default())
}

fn compute_continuation_validation_hash(audit: &ExecutionContinuationValidationAudit) -> String {
    let mut canonical = audit.clone();
    canonical.content_hash.clear();
    sha256_hex(serde_json::to_vec(&canonical).unwrap_or_default())
}

fn compute_generation_ledger_hash(ledger: &ExecutionGenerationLedger) -> String {
    let mut canonical = ledger.clone();
    canonical.content_hash.clear();
    sha256_hex(serde_json::to_vec(&canonical).unwrap_or_default())
}

fn compute_generation_pointer_hash(pointer: &ExecutionGenerationPointer) -> String {
    let mut canonical = pointer.clone();
    canonical.content_hash.clear();
    sha256_hex(serde_json::to_vec(&canonical).unwrap_or_default())
}

fn invalid_generation_data(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message.into())
}

fn generation_conflict(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::AlreadyExists, message.into())
}

fn generation_backfill_required(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::WouldBlock, message.into())
}

fn validate_owner(owner: ExecutionOwnerKey) -> io::Result<()> {
    if owner.number == 0 {
        return Err(invalid_generation_data(
            "execution generation owner number must be non-zero",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn generation_owner_dir(worktree: &Path, owner: ExecutionOwnerKey) -> io::Result<PathBuf> {
    Ok(GenerationTransactionContext::resolve(worktree, owner)?.owner_dir)
}

#[cfg(test)]
fn generation_ledger_path(worktree: &Path, owner: ExecutionOwnerKey) -> io::Result<PathBuf> {
    Ok(generation_owner_dir(worktree, owner)?.join(GENERATION_LEDGER_FILE))
}

fn generation_pointer_path(worktree: &Path) -> PathBuf {
    worktree.join(EXECUTION_GENERATION_POINTER_STATE_RELATIVE)
}

fn read_owner_ledger_from_dir(owner_dir: &Path) -> io::Result<Option<String>> {
    crate::cli::trusted_store::read_from_resolved_dir(owner_dir, GENERATION_LEDGER_FILE)
}

fn owner_generation_ledger_exists(worktree: &Path, owner: ExecutionOwnerKey) -> io::Result<bool> {
    let Ok(context) = GenerationTransactionContext::resolve(worktree, owner) else {
        // Flat legacy records in non-git/degenerate worktrees remain
        // readable/writable. Only generation authority refuses this fallback.
        return Ok(false);
    };
    Ok(read_owner_ledger_from_dir(&context.owner_dir)?.is_some())
}

fn read_generation_pointer_contents(worktree: &Path) -> io::Result<Option<String>> {
    if let Some(worktree_trusted_dir) =
        crate::cli::trusted_store::trusted_dir_for_worktree(worktree)
    {
        return crate::cli::trusted_store::read_from_resolved_dir(
            &worktree_trusted_dir,
            GENERATION_POINTER_FILE,
        );
    }
    match fs::read_to_string(generation_pointer_path(worktree)) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_legacy_projection_from_context(
    context: &GenerationTransactionContext,
) -> io::Result<Option<String>> {
    if let Some(contents) = crate::cli::trusted_store::read_from_resolved_dir(
        &context.worktree_trusted_dir,
        "execution-control.json",
    )? {
        return Ok(Some(contents));
    }
    match fs::read_to_string(state_path(&context.worktree)) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn worktree_binding_hash(worktree: &Path) -> String {
    let canonical = dunce::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    let normalized = canonical
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    #[cfg(windows)]
    let normalized = normalized.to_lowercase();
    sha256_hex(normalized)
}

fn validate_generation_ledger(
    ledger: &ExecutionGenerationLedger,
    expected_owner: ExecutionOwnerKey,
) -> io::Result<()> {
    if ledger.schema_version != GENERATION_LEDGER_SCHEMA_VERSION {
        return Err(invalid_generation_data(format!(
            "unsupported execution generation ledger schema {}",
            ledger.schema_version
        )));
    }
    if ledger.owner != expected_owner {
        return Err(invalid_generation_data(
            "execution generation ledger owner does not match its owner-scoped path",
        ));
    }
    if ledger.content_hash.is_empty()
        || ledger.content_hash != compute_generation_ledger_hash(ledger)
    {
        return Err(invalid_generation_data(
            "execution generation ledger failed integrity validation",
        ));
    }
    if ledger.generations.is_empty() {
        return Err(invalid_generation_data(
            "execution generation ledger has no genesis generation",
        ));
    }

    let mut generation_ids = std::collections::HashSet::new();
    for (index, generation) in ledger.generations.iter().enumerate() {
        if !generation_ids.insert(generation.identity.generation_id.as_str()) {
            return Err(invalid_generation_data(
                "execution generation id appears more than once",
            ));
        }
        if generation.identity.owner != expected_owner
            || generation.identity.generation_id.trim().is_empty()
            || generation.identity.session_binding_id.trim().is_empty()
            || generation.identity.initial_session_id.trim().is_empty()
            || generation.identity.worktree_binding_hash.trim().is_empty()
            || generation.identity.entrypoint.trim().is_empty()
        {
            return Err(invalid_generation_data(
                "execution generation contains an incomplete immutable header",
            ));
        }
        if generation.content_hash.is_empty()
            || generation.content_hash != compute_generation_hash(generation)
        {
            return Err(invalid_generation_data(
                "execution generation failed integrity validation",
            ));
        }
        let snapshot =
            serde_json::from_str::<ExecutionControlRecord>(&generation.execution_control_json)
                .map(hydrate_recovery_envelopes)
                .map_err(|error| {
                    invalid_generation_data(format!(
                        "execution generation snapshot is malformed: {error}"
                    ))
                })?;
        if snapshot.owner_kind != expected_owner.kind
            || snapshot.owner_number != expected_owner.number
            || snapshot.status != generation.status
            || !integrity_ok(&snapshot)
        {
            return Err(invalid_generation_data(
                "execution generation snapshot owner/status/integrity does not match its header",
            ));
        }
        if generation.status != ExecutionControlStatus::Active && snapshot.settled_at.is_none() {
            return Err(invalid_generation_data(
                "terminal execution generation snapshot has no settlement timestamp",
            ));
        }
        if index == 0 {
            if generation.identity.predecessor_generation_id.is_some()
                || generation.identity.predecessor_content_hash.is_some()
            {
                return Err(invalid_generation_data(
                    "genesis execution generation must not name a predecessor",
                ));
            }
        } else {
            let predecessor = &ledger.generations[index - 1];
            let predecessor_head = effective_generation_head_hash(ledger, predecessor);
            if generation.identity.predecessor_generation_id.as_deref()
                != Some(predecessor.identity.generation_id.as_str())
                || generation.identity.predecessor_content_hash.as_deref()
                    != Some(predecessor_head.as_str())
            {
                return Err(invalid_generation_data(
                    "execution generation predecessor id/hash chain is invalid",
                ));
            }
        }
    }

    let mut previous_attempt_hash = "";
    let mut operations: std::collections::HashMap<
        &str,
        (
            &SuccessorRequest,
            &str,
            &ExecutionGenerationIdentity,
            &str,
            SuccessorPredecessorStatus,
            ContinuationAttemptStatus,
        ),
    > = std::collections::HashMap::new();
    for attempt in &ledger.continuation_attempts {
        if validate_successor_request(&attempt.request).is_err()
            || (attempt.predecessor_status == SuccessorPredecessorStatus::Blocked
                && (attempt.request.source != FRESH_LINKED_OWNER_LAUNCH_SOURCE
                    || attempt.request.work_id.is_some()))
            || (attempt.predecessor_status == SuccessorPredecessorStatus::Completed
                && (attempt.request.source == FRESH_LINKED_OWNER_LAUNCH_SOURCE
                    || (attempt.request.source == MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE
                        && attempt.request.work_id.is_some())))
            || (attempt.predecessor_status == SuccessorPredecessorStatus::Active
                && !matches!(
                    attempt.request.source.as_str(),
                    "execution-continue" | "continue-work:resume" | "continue-work:handoff"
                ))
            || attempt.worktree_binding_hash.trim().is_empty()
            || attempt.candidate_generation_id.trim().is_empty()
            || attempt
                .predecessor_generation_content_hash
                .trim()
                .is_empty()
            || attempt.content_hash.is_empty()
            || attempt.previous_attempt_hash != previous_attempt_hash
            || attempt.content_hash != compute_continuation_attempt_hash(attempt)
        {
            return Err(invalid_generation_data(
                "continuation attempt chain failed identity/integrity validation",
            ));
        }
        previous_attempt_hash = &attempt.content_hash;
        match operations.get(attempt.request.operation_id.as_str()) {
            None => {
                if attempt.status != ContinuationAttemptStatus::Prepared
                    || attempt.activated_generation.is_some()
                    || attempt.reason.is_some()
                {
                    return Err(invalid_generation_data(
                        "continuation operation must begin with one Prepared event",
                    ));
                }
                operations.insert(
                    attempt.request.operation_id.as_str(),
                    (
                        &attempt.request,
                        &attempt.worktree_binding_hash,
                        &attempt.predecessor,
                        &attempt.candidate_generation_id,
                        attempt.predecessor_status,
                        attempt.status,
                    ),
                );
            }
            Some((
                request,
                worktree_binding_hash,
                predecessor,
                candidate,
                predecessor_status,
                previous_status,
            )) => {
                if *request != &attempt.request
                    || *worktree_binding_hash != attempt.worktree_binding_hash
                    || *predecessor != &attempt.predecessor
                    || *candidate != attempt.candidate_generation_id
                    || *predecessor_status != attempt.predecessor_status
                    || *previous_status != ContinuationAttemptStatus::Prepared
                    || attempt.status == ContinuationAttemptStatus::Prepared
                {
                    return Err(invalid_generation_data(
                        "continuation operation id was reused or has a non-append-only lifecycle",
                    ));
                }
                if attempt.status == ContinuationAttemptStatus::Aborted
                    && (attempt.reason.as_deref().is_none_or(str::is_empty)
                        || attempt.activated_generation.is_some())
                {
                    return Err(invalid_generation_data(
                        "Aborted continuation attempt is missing its audit reason",
                    ));
                }
                if attempt.status == ContinuationAttemptStatus::Activated {
                    let activated = attempt.activated_generation.as_ref().ok_or_else(|| {
                        invalid_generation_data(
                            "Activated continuation attempt does not name its generation",
                        )
                    })?;
                    if activated.generation_id != attempt.candidate_generation_id
                        || activated.worktree_binding_hash != attempt.worktree_binding_hash
                        || !ledger
                            .generations
                            .iter()
                            .any(|generation| generation.identity == *activated)
                    {
                        return Err(invalid_generation_data(
                            "Activated continuation attempt does not match a generation",
                        ));
                    }
                }
                operations.insert(
                    attempt.request.operation_id.as_str(),
                    (
                        &attempt.request,
                        &attempt.worktree_binding_hash,
                        &attempt.predecessor,
                        &attempt.candidate_generation_id,
                        attempt.predecessor_status,
                        attempt.status,
                    ),
                );
            }
        }
    }
    for generation in ledger.generations.iter().skip(1) {
        if !ledger.continuation_attempts.iter().any(|attempt| {
            attempt.status == ContinuationAttemptStatus::Activated
                && attempt
                    .activated_generation
                    .as_ref()
                    .is_some_and(|identity| identity == &generation.identity)
        }) {
            return Err(invalid_generation_data(
                "non-genesis execution generation has no Activated CAS event",
            ));
        }
    }
    let mut previous_validation_hash = "";
    let mut validation_operations = std::collections::HashSet::new();
    for audit in &ledger.continuation_validations {
        if audit.operation_id.trim().is_empty()
            || audit.operation_id.len() > 512
            || audit.operation_id.chars().any(char::is_control)
            || audit.session_id.trim().is_empty()
            || audit.execution_binding.generation_id != audit.generation_id
            || audit.capability_generation == 0
            || audit.previous_audit_hash != previous_validation_hash
            || audit.content_hash.is_empty()
            || audit.content_hash != compute_continuation_validation_hash(audit)
            || !validation_operations.insert(audit.operation_id.as_str())
            || ledger
                .continuation_attempts
                .iter()
                .any(|attempt| attempt.request.operation_id == audit.operation_id)
            || ledger
                .takeover_attempts
                .iter()
                .any(|attempt| attempt.request.operation_id == audit.operation_id)
            || !ledger.generations.iter().any(|generation| {
                generation.identity.generation_id == audit.generation_id
                    && execution_binding_matches_historical_prefix(
                        ledger,
                        generation,
                        &audit.session_id,
                        &audit.execution_binding,
                    )
            })
        {
            return Err(invalid_generation_data(
                "continuation validation audit chain failed identity/integrity validation",
            ));
        }
        previous_validation_hash = &audit.content_hash;
    }
    let mut previous_takeover_attempt_hash = "";
    let mut takeover_operations: std::collections::HashMap<
        &str,
        (
            &GenerationTakeoverRequest,
            &str,
            &str,
            &str,
            GenerationTakeoverAttemptStatus,
        ),
    > = std::collections::HashMap::new();
    for attempt in &ledger.takeover_attempts {
        if validate_generation_takeover_request(&attempt.request).is_err()
            || attempt.worktree_binding_hash.trim().is_empty()
            || attempt.generation_id.trim().is_empty()
            || attempt.predecessor_head_hash.trim().is_empty()
            || attempt.content_hash.is_empty()
            || attempt.previous_attempt_hash != previous_takeover_attempt_hash
            || attempt.content_hash != compute_generation_takeover_attempt_hash(attempt)
            || ledger
                .continuation_attempts
                .iter()
                .any(|candidate| candidate.request.operation_id == attempt.request.operation_id)
        {
            return Err(invalid_generation_data(
                "generation takeover attempt chain failed identity/integrity validation",
            ));
        }
        previous_takeover_attempt_hash = &attempt.content_hash;
        match takeover_operations.get(attempt.request.operation_id.as_str()) {
            None => {
                if attempt.status != GenerationTakeoverAttemptStatus::Prepared
                    || attempt.activated_binding.is_some()
                    || attempt.resolution_reason.is_some()
                {
                    return Err(invalid_generation_data(
                        "generation takeover operation must begin with one Prepared event",
                    ));
                }
                takeover_operations.insert(
                    attempt.request.operation_id.as_str(),
                    (
                        &attempt.request,
                        &attempt.worktree_binding_hash,
                        &attempt.generation_id,
                        &attempt.predecessor_head_hash,
                        attempt.status,
                    ),
                );
            }
            Some((
                request,
                worktree_binding_hash,
                generation_id,
                predecessor_head_hash,
                previous_status,
            )) => {
                if *request != &attempt.request
                    || *worktree_binding_hash != attempt.worktree_binding_hash
                    || *generation_id != attempt.generation_id
                    || *predecessor_head_hash != attempt.predecessor_head_hash
                    || *previous_status != GenerationTakeoverAttemptStatus::Prepared
                    || attempt.status == GenerationTakeoverAttemptStatus::Prepared
                {
                    return Err(invalid_generation_data(
                        "generation takeover operation id was reused or has a non-append-only lifecycle",
                    ));
                }
                match attempt.status {
                    GenerationTakeoverAttemptStatus::Aborted => {
                        if attempt
                            .resolution_reason
                            .as_deref()
                            .is_none_or(str::is_empty)
                            || attempt.activated_binding.is_some()
                        {
                            return Err(invalid_generation_data(
                                "Aborted generation takeover is missing its audit reason",
                            ));
                        }
                    }
                    GenerationTakeoverAttemptStatus::Activated => {
                        let binding = attempt.activated_binding.as_ref().ok_or_else(|| {
                            invalid_generation_data(
                                "Activated generation takeover has no committed binding",
                            )
                        })?;
                        let generation = ledger
                            .generations
                            .iter()
                            .find(|generation| {
                                generation.identity.generation_id == attempt.generation_id
                            })
                            .ok_or_else(|| {
                                invalid_generation_data(
                                    "Activated generation takeover names an unknown generation",
                                )
                            })?;
                        if binding.generation_id != attempt.generation_id
                            || binding.binding_id != generation.identity.session_binding_id
                            || attempt.resolution_reason.is_some()
                            || !ledger.takeovers.iter().any(|takeover| {
                                takeover.generation_id == attempt.generation_id
                                    && takeover.from_session_id == attempt.request.from_session_id
                                    && takeover.to_session_id == attempt.request.to_session_id
                                    && takeover.reason == attempt.request.reason
                                    && takeover.observed_at == attempt.request.requested_at
                            })
                        {
                            return Err(invalid_generation_data(
                                "Activated generation takeover does not match its audit event",
                            ));
                        }
                    }
                    GenerationTakeoverAttemptStatus::Prepared => unreachable!(),
                }
                takeover_operations.insert(
                    attempt.request.operation_id.as_str(),
                    (
                        &attempt.request,
                        &attempt.worktree_binding_hash,
                        &attempt.generation_id,
                        &attempt.predecessor_head_hash,
                        attempt.status,
                    ),
                );
            }
        }
    }
    enum ProjectionEventRef<'a> {
        Takeover(&'a GenerationTakeoverAudit),
        Lifecycle(&'a GenerationLifecycleEvent),
    }
    impl ProjectionEventRef<'_> {
        fn sequence(&self) -> u64 {
            match self {
                Self::Takeover(event) => event.sequence,
                Self::Lifecycle(event) => event.sequence,
            }
        }
    }

    let mut effective_statuses = ledger
        .generations
        .iter()
        .map(|generation| {
            (
                generation.identity.generation_id.as_str(),
                generation.status,
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let mut effective_projections = ledger
        .generations
        .iter()
        .map(|generation| {
            let projection =
                serde_json::from_str::<ExecutionControlRecord>(&generation.execution_control_json)
                    .map(hydrate_recovery_envelopes)
                    .expect("generation snapshots were validated above");
            (generation.identity.generation_id.as_str(), projection)
        })
        .collect::<std::collections::HashMap<_, _>>();
    let mut projection_events = ledger
        .takeovers
        .iter()
        .map(ProjectionEventRef::Takeover)
        .chain(
            ledger
                .lifecycle_events
                .iter()
                .map(ProjectionEventRef::Lifecycle),
        )
        .collect::<Vec<_>>();
    projection_events.sort_by_key(ProjectionEventRef::sequence);
    let mut previous_projection_event_hash = String::new();
    for (index, event) in projection_events.iter().enumerate() {
        if event.sequence() != index as u64 + 1 {
            return Err(invalid_generation_data(
                "execution generation projection event sequence is not contiguous",
            ));
        }
        let (stored_previous_hash, stored_hash, computed_hash) = match event {
            ProjectionEventRef::Takeover(event) => (
                event.previous_event_hash.as_str(),
                event.content_hash.as_str(),
                compute_takeover_event_hash(event),
            ),
            ProjectionEventRef::Lifecycle(event) => (
                event.previous_event_hash.as_str(),
                event.content_hash.as_str(),
                compute_lifecycle_event_hash(event),
            ),
        };
        if stored_previous_hash != previous_projection_event_hash
            || stored_hash.is_empty()
            || stored_hash != computed_hash
        {
            return Err(invalid_generation_data(
                "execution generation projection event hash chain is invalid",
            ));
        }
        previous_projection_event_hash = stored_hash.to_string();
        match event {
            ProjectionEventRef::Takeover(takeover) => {
                let Some(status) = effective_statuses
                    .get(takeover.generation_id.as_str())
                    .copied()
                else {
                    return Err(invalid_generation_data(
                        "execution generation takeover names an unknown generation",
                    ));
                };
                let prior_projection = effective_projections
                    .get(takeover.generation_id.as_str())
                    .ok_or_else(|| {
                        invalid_generation_data(
                            "execution generation takeover has no prior projection",
                        )
                    })?
                    .clone();
                if status != ExecutionControlStatus::Active
                    || prior_projection.primary_session_id != takeover.from_session_id
                    || takeover.from_session_id.trim().is_empty()
                    || takeover.to_session_id.trim().is_empty()
                    || takeover.from_session_id == takeover.to_session_id
                    || takeover.reason.trim().is_empty()
                {
                    return Err(invalid_generation_data(
                        "execution generation takeover audit is incomplete",
                    ));
                }
                let projection = serde_json::from_str::<ExecutionControlRecord>(
                    &takeover.execution_control_json,
                )
                .map(hydrate_recovery_envelopes)
                .map_err(|error| {
                    invalid_generation_data(format!(
                        "takeover execution projection is malformed: {error}"
                    ))
                })?;
                let transfer = projection.transfers.last().ok_or_else(|| {
                    invalid_generation_data(
                        "takeover execution projection is missing its ownership transfer",
                    )
                })?;
                let accepted_import_reason =
                    format!("continue-work-stale-takeover: {}", takeover.reason);
                if transfer.from_session_id != takeover.from_session_id
                    || transfer.to_session_id != takeover.to_session_id
                    || transfer.transferred_at != takeover.observed_at
                    || (transfer.reason != takeover.reason
                        && transfer.reason != accepted_import_reason)
                {
                    return Err(invalid_generation_data(
                        "takeover execution projection does not match its audit event",
                    ));
                }
                let mut expected_projection = prior_projection;
                expected_projection.primary_session_id = takeover.to_session_id.clone();
                expected_projection.transfers.push(transfer.clone());
                expected_projection.content_hash = compute_content_hash(&expected_projection);
                if projection.owner_kind != expected_owner.kind
                    || projection.owner_number != expected_owner.number
                    || projection.status != ExecutionControlStatus::Active
                    || !integrity_ok(&projection)
                    || projection != expected_projection
                {
                    return Err(invalid_generation_data(
                        "takeover execution projection failed owner/status/session/integrity validation",
                    ));
                }
                effective_projections.insert(takeover.generation_id.as_str(), projection);
            }
            ProjectionEventRef::Lifecycle(lifecycle) => {
                let Some(status) = effective_statuses.get_mut(lifecycle.generation_id.as_str())
                else {
                    return Err(invalid_generation_data(
                        "execution lifecycle event names an unknown generation",
                    ));
                };
                let transition_allowed = matches!(
                    (lifecycle.from_status, lifecycle.to_status),
                    (
                        ExecutionControlStatus::Active,
                        ExecutionControlStatus::Completed | ExecutionControlStatus::Blocked
                    ) | (
                        ExecutionControlStatus::Blocked,
                        ExecutionControlStatus::Active
                    )
                );
                if *status != lifecycle.from_status
                    || !transition_allowed
                    || lifecycle.session_id.trim().is_empty()
                    || lifecycle.reason.trim().is_empty()
                    || lifecycle
                        .operation_id
                        .as_deref()
                        .is_some_and(|operation_id| {
                            operation_id.trim().is_empty()
                                || operation_id.len() > 256
                                || operation_id.chars().any(char::is_control)
                        })
                {
                    return Err(invalid_generation_data(
                        "execution lifecycle event has an invalid append-only transition",
                    ));
                }
                let projection = serde_json::from_str::<ExecutionControlRecord>(
                    &lifecycle.execution_control_json,
                )
                .map(hydrate_recovery_envelopes)
                .map_err(|error| {
                    invalid_generation_data(format!(
                        "lifecycle execution projection is malformed: {error}"
                    ))
                })?;
                let prior_projection = effective_projections
                    .get(lifecycle.generation_id.as_str())
                    .ok_or_else(|| {
                        invalid_generation_data("execution lifecycle event has no prior projection")
                    })?;
                if projection.owner_kind != expected_owner.kind
                    || projection.owner_number != expected_owner.number
                    || projection.status != lifecycle.to_status
                    || projection.primary_session_id != lifecycle.session_id
                    || prior_projection.primary_session_id != lifecycle.session_id
                    || !integrity_ok(&projection)
                    || (lifecycle.to_status != ExecutionControlStatus::Active
                        && projection.settled_at.is_none())
                {
                    return Err(invalid_generation_data(
                        "lifecycle execution projection failed owner/status/session/integrity validation",
                    ));
                }
                *status = lifecycle.to_status;
                effective_projections.insert(lifecycle.generation_id.as_str(), projection);
            }
        }
    }

    for (index, generation) in ledger.generations.iter().enumerate().skip(1) {
        let predecessor = &ledger.generations[index - 1];
        let authorized_predecessor_status = ledger
            .continuation_attempts
            .iter()
            .find(|attempt| {
                attempt.status == ContinuationAttemptStatus::Activated
                    && attempt.predecessor == predecessor.identity
                    && attempt
                        .activated_generation
                        .as_ref()
                        .is_some_and(|identity| identity == &generation.identity)
            })
            .map(|attempt| successor_predecessor_execution_status(attempt.predecessor_status));
        if authorized_predecessor_status
            != Some(effective_statuses[predecessor.identity.generation_id.as_str()])
            || generation.identity.predecessor_generation_id.as_deref()
                != Some(predecessor.identity.generation_id.as_str())
        {
            return Err(invalid_generation_data(
                "successor generation does not follow its authorized terminal predecessor",
            ));
        }
    }
    let superseded_active_generation_ids = ledger
        .continuation_attempts
        .iter()
        .filter(|attempt| {
            attempt.status == ContinuationAttemptStatus::Activated
                && attempt.predecessor_status == SuccessorPredecessorStatus::Active
        })
        .map(|attempt| attempt.predecessor.generation_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let active_generation_ids = effective_statuses
        .iter()
        .filter_map(|(generation_id, status)| {
            (*status == ExecutionControlStatus::Active
                && !superseded_active_generation_ids.contains(generation_id))
            .then_some(*generation_id)
        })
        .collect::<Vec<_>>();
    if active_generation_ids.len() > 1 {
        return Err(invalid_generation_data(
            "execution generation ledger contains more than one effective Active generation",
        ));
    }
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    if active_generation_ids
        .first()
        .is_some_and(|generation_id| **generation_id != current.identity.generation_id)
    {
        return Err(invalid_generation_data(
            "the sole effective Active execution generation is not current",
        ));
    }
    Ok(())
}

#[must_use]
pub fn generation_ledger_integrity_ok(ledger: &ExecutionGenerationLedger) -> bool {
    validate_generation_ledger(ledger, ledger.owner).is_ok()
}

fn validate_generation_pointer(
    pointer: &ExecutionGenerationPointer,
    ledger: &ExecutionGenerationLedger,
    projection: &str,
) -> io::Result<()> {
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    let effective_status = ledger.effective_status_for(current);
    let effective_projection = ledger.effective_projection_for(current);
    if pointer.schema_version != GENERATION_LEDGER_SCHEMA_VERSION
        || pointer.owner != ledger.owner
        || pointer.current_generation_id != current.identity.generation_id
        || pointer.current_generation_content_hash != current.content_hash
        || pointer.projection_content_hash != sha256_hex(projection)
        || projection != effective_projection
        || pointer.content_hash.is_empty()
        || pointer.content_hash != compute_generation_pointer_hash(pointer)
    {
        return Err(invalid_generation_data(
            "execution generation pointer/projection is stale, missing, or mismatched",
        ));
    }
    let projection_record = serde_json::from_str::<ExecutionControlRecord>(projection)
        .map(hydrate_recovery_envelopes)
        .map_err(|error| {
            invalid_generation_data(format!(
                "execution generation projection is malformed: {error}"
            ))
        })?;
    if projection_record.owner_kind != ledger.owner.kind
        || projection_record.owner_number != ledger.owner.number
        || projection_record.status != effective_status
        || !integrity_ok(&projection_record)
    {
        return Err(invalid_generation_data(
            "execution generation projection failed owner/status/integrity validation",
        ));
    }
    Ok(())
}

/// Load and validate only the repository/owner-scoped authoritative ledger.
///
/// This intentionally does not require a pointer in the caller's worktree:
/// cross-worktree preparation/activation races must CAS on one owner ledger.
fn load_owner_generation_ledger_from_context(
    context: &GenerationTransactionContext,
) -> io::Result<Option<ExecutionGenerationLedger>> {
    let Some(contents) = read_owner_ledger_from_dir(&context.owner_dir)? else {
        return Ok(None);
    };
    let ledger = serde_json::from_str::<ExecutionGenerationLedger>(&contents).map_err(|error| {
        invalid_generation_data(format!("malformed generation ledger: {error}"))
    })?;
    validate_generation_ledger(&ledger, context.owner)?;
    Ok(Some(ledger))
}

pub fn load_owner_generation_ledger(
    worktree: &Path,
    owner: ExecutionOwnerKey,
) -> io::Result<Option<ExecutionGenerationLedger>> {
    let context = match GenerationTransactionContext::resolve(worktree, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(None),
        Err(error) => return Err(error),
    };
    load_owner_generation_ledger_from_context(&context)
}

/// Load and validate an owner-scoped ledger together with the caller
/// worktree's independent pointer/projection. Mutation/verification gates use
/// this strict view so stale worktrees and old flat writers fail closed.
fn load_generation_ledger_from_context(
    context: &GenerationTransactionContext,
) -> io::Result<Option<ExecutionGenerationLedger>> {
    let Some(ledger) = load_owner_generation_ledger_from_context(context)? else {
        return Ok(None);
    };
    let pointer_contents = crate::cli::trusted_store::read_from_resolved_dir(
        &context.worktree_trusted_dir,
        GENERATION_POINTER_FILE,
    )?
    .ok_or_else(|| {
        invalid_generation_data(
            "execution generation pointer is missing after ledger ownership was established",
        )
    })?;
    let pointer =
        serde_json::from_str::<ExecutionGenerationPointer>(&pointer_contents).map_err(|error| {
            invalid_generation_data(format!("malformed execution generation pointer: {error}"))
        })?;
    let projection = crate::cli::trusted_store::read_from_resolved_dir(
        &context.worktree_trusted_dir,
        "execution-control.json",
    )?
    .ok_or_else(|| {
        invalid_generation_data(
            "flat execution projection is missing after ledger ownership was established",
        )
    })?;
    validate_generation_pointer(&pointer, &ledger, &projection)?;
    Ok(Some(ledger))
}

pub fn load_generation_ledger(
    worktree: &Path,
    owner: ExecutionOwnerKey,
) -> io::Result<Option<ExecutionGenerationLedger>> {
    let context = match GenerationTransactionContext::resolve(worktree, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(None),
        Err(error) => return Err(error),
    };
    load_generation_ledger_from_context(&context)
}

/// Return the owner named by the worktree's integrity-valid global
/// generation pointer/projection pair.
///
/// Recovery coordinators use this before repairing an owner-scoped ledger:
/// a different current owner means the older receipt is historical and must
/// never republish its projection over the newer authority.
pub fn current_generation_owner(worktree: &Path) -> io::Result<Option<ExecutionOwnerKey>> {
    let Some(pointer_contents) = read_generation_pointer_contents(worktree)? else {
        return Ok(None);
    };
    let pointer =
        serde_json::from_str::<ExecutionGenerationPointer>(&pointer_contents).map_err(|error| {
            invalid_generation_data(format!("malformed execution generation pointer: {error}"))
        })?;
    let owner = pointer.owner;
    load_generation_ledger(worktree, owner)?.ok_or_else(|| {
        invalid_generation_data(
            "execution generation pointer names an owner without strict generation authority",
        )
    })?;
    Ok(Some(owner))
}

/// Recovery-only owner probe that also recognizes an authoritative
/// ledger/projection pair whose pointer write was interrupted.
///
/// The owner ledger is committed before the projection and pointer. A stale
/// launch receipt from another owner must not overwrite that newer ledger
/// while its pointer is being repaired. Mutation gates must continue to use
/// the strict pointer-backed APIs.
pub fn recovery_generation_owner(worktree: &Path) -> io::Result<Option<ExecutionOwnerKey>> {
    if let Ok(Some(owner)) = current_generation_owner(worktree) {
        return Ok(Some(owner));
    }
    let Some(projection) = read_record_contents(worktree)? else {
        return Ok(None);
    };
    let record = serde_json::from_str::<ExecutionControlRecord>(&projection)
        .map(hydrate_recovery_envelopes)
        .map_err(|error| {
            invalid_generation_data(format!("malformed recovery execution projection: {error}"))
        })?;
    if !integrity_ok(&record) {
        return Err(invalid_generation_data(
            "recovery execution projection failed integrity validation",
        ));
    }
    let owner = ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    let ledger = load_owner_generation_ledger(worktree, owner)?.ok_or_else(|| {
        invalid_generation_data(
            "recovery execution projection owner has no authoritative generation ledger",
        )
    })?;
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("recovery generation ledger current id is missing")
    })?;
    if current.identity.worktree_binding_hash != worktree_binding_hash(worktree)
        || ledger.effective_projection_for(current) != projection
        || ledger.effective_status_for(current) != record.status
    {
        return Err(invalid_generation_data(
            "recovery owner ledger does not match the current worktree projection",
        ));
    }
    Ok(Some(owner))
}

/// Recovery-only hint from the integrity-valid worktree projection, without
/// requiring the owner ledger and pointer to have completed their write.
///
/// Callers use a foreign hint only to fail closed while that foreign owner's
/// ledger-first transaction is incomplete; it never authorizes cleanup or a
/// host-side mutation by itself.
pub fn recovery_projection_owner_hint(worktree: &Path) -> io::Result<Option<ExecutionOwnerKey>> {
    let Some(projection) = read_record_contents(worktree)? else {
        return Ok(None);
    };
    let record = serde_json::from_str::<ExecutionControlRecord>(&projection)
        .map(hydrate_recovery_envelopes)
        .map_err(|error| {
            invalid_generation_data(format!("malformed recovery execution projection: {error}"))
        })?;
    if !integrity_ok(&record) {
        return Err(invalid_generation_data(
            "recovery execution projection failed integrity validation",
        ));
    }
    Ok(Some(ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    }))
}

pub fn current_generation_identity(
    worktree: &Path,
    owner: ExecutionOwnerKey,
) -> io::Result<Option<ExecutionGenerationIdentity>> {
    Ok(
        load_owner_generation_ledger(worktree, owner)?.and_then(|ledger| {
            ledger
                .current_generation()
                .map(|generation| generation.identity.clone())
        }),
    )
}

fn effective_generation_head_hash(
    ledger: &ExecutionGenerationLedger,
    generation: &ExecutionGeneration,
) -> String {
    let mut events = ledger
        .lifecycle_events_for(&generation.identity.generation_id)
        .map(|event| {
            (
                event.sequence,
                "lifecycle",
                sha256_hex(serde_json::to_vec(event).unwrap_or_default()),
            )
        })
        .chain(
            ledger
                .takeovers
                .iter()
                .filter(|event| event.generation_id == generation.identity.generation_id)
                .map(|event| {
                    (
                        event.sequence,
                        "takeover",
                        sha256_hex(serde_json::to_vec(event).unwrap_or_default()),
                    )
                }),
        )
        .collect::<Vec<_>>();
    events.sort_by_key(|(sequence, _, _)| *sequence);
    sha256_hex(serde_json::to_vec(&(generation.content_hash.as_str(), events)).unwrap_or_default())
}

fn execution_binding_for_generation(
    ledger: &ExecutionGenerationLedger,
    generation: &ExecutionGeneration,
) -> gwt_agent::ExecutionBindingIdentity {
    gwt_agent::ExecutionBindingIdentity {
        generation_id: generation.identity.generation_id.clone(),
        binding_id: generation.identity.session_binding_id.clone(),
        ledger_head_hash: effective_generation_head_hash(ledger, generation),
    }
}

/// Accept an exact current binding or a binding for an actual prefix of the
/// same generation whose remaining events are lifecycle-only transitions
/// owned by the same Session.
///
/// A takeover in the suffix, a different generation/binding, or an arbitrary
/// non-prefix head always fails. This keeps the owner ledger as the single
/// lifecycle source of truth without requiring a second Session-file commit.
fn execution_binding_authorizes_lifecycle_descendant(
    ledger: &ExecutionGenerationLedger,
    generation: &ExecutionGeneration,
    expected_session_id: &str,
    expected: &gwt_agent::ExecutionBindingIdentity,
) -> bool {
    if expected.generation_id != generation.identity.generation_id
        || expected.binding_id != generation.identity.session_binding_id
    {
        return false;
    }
    let current = execution_binding_for_generation(ledger, generation);
    if current == *expected {
        return true;
    }

    let mut events = ledger
        .lifecycle_events_for(&generation.identity.generation_id)
        .map(|event| (event.sequence, Some(event.session_id.as_str())))
        .chain(
            ledger
                .takeovers
                .iter()
                .filter(|event| event.generation_id == generation.identity.generation_id)
                .map(|event| (event.sequence, None)),
        )
        .collect::<Vec<_>>();
    events.sort_by_key(|(sequence, _)| *sequence);

    for prefix_len in 0..events.len() {
        let cutoff = prefix_len.checked_sub(1).map(|index| events[index].0);
        let mut prefix = ledger.clone();
        prefix.lifecycle_events.retain(|event| {
            event.generation_id != generation.identity.generation_id
                || cutoff.is_some_and(|sequence| event.sequence <= sequence)
        });
        prefix.takeovers.retain(|event| {
            event.generation_id != generation.identity.generation_id
                || cutoff.is_some_and(|sequence| event.sequence <= sequence)
        });
        if execution_binding_for_generation(&prefix, generation) == *expected {
            return events[prefix_len..]
                .iter()
                .all(|(_, session_id)| *session_id == Some(expected_session_id));
        }
    }
    false
}

/// Validate one durable Session prefix against the current generation while
/// permitting only same-session lifecycle suffixes. Takeovers, generation
/// changes, and arbitrary stale heads remain fail-closed.
pub(crate) fn session_binding_authorizes_current_lifecycle_descendant(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    session_id: &str,
    expected: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<bool> {
    let Some(ledger) = load_generation_ledger(worktree, owner)? else {
        return Ok(false);
    };
    let Some(generation) = ledger.current_generation() else {
        return Ok(false);
    };
    Ok(execution_binding_authorizes_lifecycle_descendant(
        &ledger, generation, session_id, expected,
    ))
}

/// Reconstruct one Activated attempt's binding at activation time and require
/// that it is still the exact current Active authority.
///
/// Lifecycle and takeover events are append-only suffixes to the immutable
/// generation. Stripping all of them from a clone reconstructs the binding
/// emitted by activation; comparing that binding with the strict current head
/// makes every later suffix fail closed before repair can publish or bind a
/// Session.
fn exact_current_activated_continuation_binding(
    ledger: &ExecutionGenerationLedger,
    attempt: &ContinuationAttempt,
) -> Option<gwt_agent::ExecutionBindingIdentity> {
    if attempt.status != ContinuationAttemptStatus::Activated {
        return None;
    }
    let activated = attempt.activated_generation.as_ref()?;
    let current = ledger.current_generation()?;
    if current.identity != *activated
        || ledger.effective_status_for(current) != ExecutionControlStatus::Active
    {
        return None;
    }

    let mut activation_ledger = ledger.clone();
    activation_ledger
        .lifecycle_events
        .retain(|event| event.generation_id != current.identity.generation_id);
    activation_ledger
        .takeovers
        .retain(|event| event.generation_id != current.identity.generation_id);
    let activation_binding = execution_binding_for_generation(&activation_ledger, current);
    if execution_binding_for_generation(ledger, current) != activation_binding {
        return None;
    }

    let projection =
        serde_json::from_str::<ExecutionControlRecord>(ledger.effective_projection_for(current))
            .map(hydrate_recovery_envelopes)
            .ok()?;
    if projection.status != ExecutionControlStatus::Active
        || !integrity_ok(&projection)
        || projection.primary_session_id != attempt.request.initial_session_id
    {
        return None;
    }
    Some(activation_binding)
}

/// Recover the exact activation-time binding from an owner-ledger Activated
/// continuation when the worktree pointer/projection publication was lost.
/// Only the Session named by that exact attempt remains eligible, and any
/// lifecycle or takeover suffix makes the repair unavailable.
pub(crate) fn activated_continuation_binding_for_session(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    session_id: &str,
) -> io::Result<Option<gwt_agent::ExecutionBindingIdentity>> {
    let Some(ledger) = load_owner_generation_ledger(worktree, owner)? else {
        return Ok(None);
    };
    let Some(attempt) = ledger.continuation_attempts.iter().rev().find(|attempt| {
        attempt.status == ContinuationAttemptStatus::Activated
            && attempt.request.initial_session_id == session_id
    }) else {
        return Ok(None);
    };
    Ok(exact_current_activated_continuation_binding(
        &ledger, attempt,
    ))
}

/// Verify an immutable binding prefix against the writer that owned that
/// exact prefix. Later legal lifecycle or takeover events are historical
/// suffixes and cannot invalidate the earlier proof.
fn execution_binding_matches_historical_prefix(
    ledger: &ExecutionGenerationLedger,
    generation: &ExecutionGeneration,
    expected_session_id: &str,
    expected: &gwt_agent::ExecutionBindingIdentity,
) -> bool {
    if expected.generation_id != generation.identity.generation_id
        || expected.binding_id != generation.identity.session_binding_id
    {
        return false;
    }
    let mut sequences = ledger
        .lifecycle_events_for(&generation.identity.generation_id)
        .map(|event| event.sequence)
        .chain(
            ledger
                .takeovers
                .iter()
                .filter(|event| event.generation_id == generation.identity.generation_id)
                .map(|event| event.sequence),
        )
        .collect::<Vec<_>>();
    sequences.sort_unstable();

    for prefix_len in 0..=sequences.len() {
        let cutoff = prefix_len.checked_sub(1).map(|index| sequences[index]);
        let mut prefix = ledger.clone();
        prefix.lifecycle_events.retain(|event| {
            event.generation_id != generation.identity.generation_id
                || cutoff.is_some_and(|sequence| event.sequence <= sequence)
        });
        prefix.takeovers.retain(|event| {
            event.generation_id != generation.identity.generation_id
                || cutoff.is_some_and(|sequence| event.sequence <= sequence)
        });
        if execution_binding_for_generation(&prefix, generation) != *expected {
            continue;
        }
        let writer = serde_json::from_str::<ExecutionControlRecord>(
            prefix.effective_projection_for(generation),
        )
        .map(hydrate_recovery_envelopes)
        .ok()
        .map(|record| record.primary_session_id);
        return writer.as_deref() == Some(expected_session_id);
    }
    false
}

/// Recovery-only probe for the exact binding emitted when a genesis
/// generation was first published, before any later lifecycle or takeover
/// events advanced its effective head.
pub fn genesis_initial_execution_binding_matches(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    expected_session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<bool> {
    let context = match GenerationTransactionContext::resolve(worktree, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(false),
        Err(error) => return Err(error),
    };
    let Some(ledger) = load_owner_generation_ledger_from_context(&context)? else {
        return Ok(false);
    };
    let mut candidates = ledger.generations.iter().filter(|generation| {
        generation.identity.predecessor_generation_id.is_none()
            && generation.identity.initial_session_id == expected_session_id
            && generation.identity.generation_id == expected_identity.generation_id
            && generation.identity.session_binding_id == expected_identity.binding_id
    });
    let Some(generation) = candidates.next() else {
        return Ok(false);
    };
    if candidates.next().is_some() {
        return Ok(false);
    }
    let mut initial_ledger = ledger.clone();
    initial_ledger
        .lifecycle_events
        .retain(|event| event.generation_id != generation.identity.generation_id);
    initial_ledger
        .takeovers
        .retain(|event| event.generation_id != generation.identity.generation_id);
    Ok(execution_binding_for_generation(&initial_ledger, generation) == *expected_identity)
}

fn session_binding_authorizes_current_generation(
    context: &GenerationTransactionContext,
    ledger: &ExecutionGenerationLedger,
    session_id: &str,
    session_state: &gwt_agent::SessionPathState,
) -> io::Result<bool> {
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    let legacy_unbound_compatibility = current.identity.predecessor_generation_id.is_none()
        && current
            .identity
            .session_binding_id
            .starts_with("legacy-ecr-");
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|error| invalid_generation_data(format!("invalid Session id: {error}")))?;
    let session = match session_state {
        gwt_agent::SessionPathState::Present(session) => session.as_ref(),
        gwt_agent::SessionPathState::Missing => return Ok(legacy_unbound_compatibility),
        gwt_agent::SessionPathState::Error(_) => return Ok(false),
    };
    durable_session_binding_authorizes_current_generation(context, ledger, session_id, session)
}

fn durable_session_binding_authorizes_current_generation(
    context: &GenerationTransactionContext,
    ledger: &ExecutionGenerationLedger,
    session_id: &str,
    session: &gwt_agent::Session,
) -> io::Result<bool> {
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    let legacy_unbound_compatibility = current.identity.predecessor_generation_id.is_none()
        && current
            .identity
            .session_binding_id
            .starts_with("legacy-ecr-");
    let Some(binding) = session.execution_binding.as_ref() else {
        return Ok(legacy_unbound_compatibility);
    };
    let repo_hash =
        crate::index_worker::detect_repo_hash(&context.worktree).map(|value| value.to_string());
    Ok(session.id == session_id
        && session.worktree_path.exists()
        && worktree_binding_hash(&session.worktree_path) == context.worktree_binding_hash
        && session.repo_hash == repo_hash
        && binding.schema_version == gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
        && binding.session_id == session_id
        && Some(binding.repo_hash.as_str()) == repo_hash.as_deref()
        && binding.owner_kind == context.owner.kind.as_str()
        && binding.owner_number == context.owner.number
        && session.linked_issue_number == Some(context.owner.number)
        && binding.capability_generation > 0
        && execution_binding_authorizes_lifecycle_descendant(
            ledger,
            current,
            session_id,
            &binding.identity,
        ))
}

/// Exact non-secret identity consumed by Session/verification projections.
///
/// Prepared/Aborted attempt audit is deliberately excluded. The head changes
/// only when the current generation changes or its effective lifecycle /
/// takeover projection advances.
pub fn current_execution_binding(
    worktree: &Path,
    owner: ExecutionOwnerKey,
) -> io::Result<Option<gwt_agent::session::ExecutionBindingIdentity>> {
    let Some(ledger) = load_generation_ledger(worktree, owner)? else {
        return Ok(None);
    };
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    Ok(Some(execution_binding_for_generation(&ledger, current)))
}

/// Verify that a projected Session binding is either current or an authentic
/// same-Session lifecycle prefix of the current generation.
pub(crate) fn execution_binding_authorizes_current_generation(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    expected_session_id: &str,
    expected: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<bool> {
    let Some(ledger) = load_generation_ledger(worktree, owner)? else {
        return Ok(false);
    };
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    if current.identity.worktree_binding_hash != worktree_binding_hash(worktree) {
        return Ok(false);
    }
    let projection =
        serde_json::from_str::<ExecutionControlRecord>(ledger.effective_projection_for(current))
            .map(hydrate_recovery_envelopes)
            .map_err(|error| {
                invalid_generation_data(format!(
                    "execution generation projection is malformed: {error}"
                ))
            })?;
    Ok(projection.primary_session_id == expected_session_id
        && execution_binding_authorizes_lifecycle_descendant(
            &ledger,
            current,
            expected_session_id,
            expected,
        ))
}

/// Recovery-only identity derived from the authoritative owner ledger without
/// requiring the caller worktree's projection/pointer pair to be readable.
/// Mutation gates must continue to use [`current_execution_binding`].
pub fn current_owner_execution_binding(
    worktree: &Path,
    owner: ExecutionOwnerKey,
) -> io::Result<Option<gwt_agent::session::ExecutionBindingIdentity>> {
    let Some(ledger) = load_owner_generation_ledger(worktree, owner)? else {
        return Ok(None);
    };
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    Ok(Some(execution_binding_for_generation(&ledger, current)))
}

/// Capture the exact durable Session binding allowed to attempt a PR
/// mutation for the current generation.
///
/// Completed generations retain terminal handoff authority, while Blocked
/// generations are returned only so [`pr_handoff_refusal`] can emit the
/// lifecycle-specific recovery guidance before any external dispatch.
/// Ledgerless executions preserve their legacy compatibility behavior.
pub(crate) fn snapshot_pr_mutation_execution_binding(
    worktree: &Path,
    session_id: Option<&str>,
) -> io::Result<Option<gwt_agent::SessionExecutionBinding>> {
    let Some(record) = load(worktree)? else {
        return Ok(None);
    };
    let owner = ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    let Some(ledger) = load_generation_ledger(worktree, owner)? else {
        return Ok(None);
    };
    if !integrity_ok(&record)
        || !generation_ledger_integrity_ok(&ledger)
        || ledger.current_effective_status() != Some(record.status)
    {
        return Err(invalid_generation_data(
            "PR mutation execution authority is inconsistent",
        ));
    }
    let session_id = session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(generation_binding_mismatch)?;
    if record.primary_session_id != session_id {
        return Err(generation_binding_mismatch());
    }
    let context = GenerationTransactionContext::resolve(worktree, owner)?;
    let session_path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    let session = gwt_agent::Session::load(&session_path)?;
    if !durable_session_binding_authorizes_current_generation(
        &context, &ledger, session_id, &session,
    )? {
        return Err(generation_binding_mismatch());
    }
    session
        .execution_binding
        .ok_or_else(generation_binding_mismatch)
        .map(Some)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurrentExecutionBindingAuthority {
    ActiveMutation,
    BlockedBuildAbort,
    PrMutation,
}

impl CurrentExecutionBindingAuthority {
    fn allows(self, status: ExecutionControlStatus) -> bool {
        match self {
            Self::ActiveMutation => status == ExecutionControlStatus::Active,
            Self::BlockedBuildAbort => status == ExecutionControlStatus::Blocked,
            Self::PrMutation => matches!(
                status,
                ExecutionControlStatus::Active | ExecutionControlStatus::Completed
            ),
        }
    }
}

/// Return whether the caller carries the exact authority of the current
/// producing execution generation.
///
/// This is stricter than [`current_execution_binding`]: terminal generations
/// retain a verifiable identity for audit/evidence purposes, but can never
/// authorize a new host-side mutation. Missing/non-canonical state and any
/// session or binding mismatch return `false`; malformed canonical authority
/// returns an error so callers fail closed.
pub fn current_active_execution_binding_matches(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    expected_session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<bool> {
    let context = match GenerationTransactionContext::resolve(worktree, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(false),
        Err(error) => return Err(error),
    };
    current_active_execution_binding_matches_context(
        &context,
        expected_session_id,
        expected_identity,
    )
}

fn current_active_execution_binding_matches_context(
    context: &GenerationTransactionContext,
    expected_session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<bool> {
    current_execution_binding_matches_context(
        context,
        expected_session_id,
        expected_identity,
        CurrentExecutionBindingAuthority::ActiveMutation,
    )
}

pub(crate) fn blocked_build_abort_execution_binding_matches(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    expected_session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<bool> {
    let context = match GenerationTransactionContext::resolve(worktree, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(false),
        Err(error) => return Err(error),
    };
    blocked_build_abort_execution_binding_matches_context(
        &context,
        expected_session_id,
        expected_identity,
    )
}

fn blocked_build_abort_execution_binding_matches_context(
    context: &GenerationTransactionContext,
    expected_session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<bool> {
    if !current_execution_binding_matches_context(
        context,
        expected_session_id,
        expected_identity,
        CurrentExecutionBindingAuthority::BlockedBuildAbort,
    )? {
        return Ok(false);
    }
    Ok(
        gwt_core::skill_state::load(&context.worktree, "build-spec")?.is_some_and(|state| {
            state.active
                && state.owner_spec == Some(context.owner.number)
                && state.session_id == expected_session_id
        }),
    )
}

fn current_execution_binding_matches_context(
    context: &GenerationTransactionContext,
    expected_session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
    authority: CurrentExecutionBindingAuthority,
) -> io::Result<bool> {
    let Some(ledger) = load_generation_ledger_from_context(context)? else {
        return Ok(false);
    };
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    if !authority.allows(ledger.effective_status_for(current))
        || current.identity.worktree_binding_hash != context.worktree_binding_hash
    {
        return Ok(false);
    }
    let projection =
        serde_json::from_str::<ExecutionControlRecord>(ledger.effective_projection_for(current))
            .map(hydrate_recovery_envelopes)
            .map_err(|error| {
                invalid_generation_data(format!(
                    "execution generation projection is malformed: {error}"
                ))
            })?;
    Ok(projection.primary_session_id == expected_session_id
        && execution_binding_authorizes_lifecycle_descendant(
            &ledger,
            current,
            expected_session_id,
            expected_identity,
        ))
}

/// Execute one producing operation while its owner generation and durable
/// Session capability epoch are both leased.
///
/// The acquisition order is always owner → Session. The caller may acquire a
/// process-local capability registry lock inside `operation`, yielding the
/// full owner → Session → registry → mutation order.
pub fn with_current_active_execution_binding_lease<T>(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionBinding,
    operation: impl FnOnce() -> T,
) -> io::Result<Option<T>> {
    with_current_active_execution_binding_lease_wait(
        sessions_dir,
        expected,
        ACTIVE_BINDING_LEASE_WAIT,
        operation,
    )
}

/// [`with_current_active_execution_binding_lease`] with a bounded Session
/// wait for retry-aware callers and deterministic contention tests.
pub fn with_current_active_execution_binding_lease_wait<T>(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionBinding,
    session_wait: Duration,
    operation: impl FnOnce() -> T,
) -> io::Result<Option<T>> {
    with_current_execution_binding_lease_wait(
        sessions_dir,
        expected,
        session_wait,
        CurrentExecutionBindingAuthority::ActiveMutation,
        operation,
    )
}

/// Execute one PR mutation while the exact Active or Completed generation
/// binding and durable Session capability epoch remain leased.
///
/// Blocked generations never reach this dispatch boundary because
/// [`pr_handoff_refusal`] rejects them with recovery guidance first.
pub(crate) fn with_current_pr_mutation_execution_binding_lease<T>(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionBinding,
    operation: impl FnOnce() -> T,
) -> io::Result<Option<T>> {
    with_current_execution_binding_lease_wait(
        sessions_dir,
        expected,
        ACTIVE_BINDING_LEASE_WAIT,
        CurrentExecutionBindingAuthority::PrMutation,
        operation,
    )
}

fn with_current_execution_binding_lease_wait<T>(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionBinding,
    session_wait: Duration,
    authority: CurrentExecutionBindingAuthority,
    operation: impl FnOnce() -> T,
) -> io::Result<Option<T>> {
    if gwt_agent::current_thread_holds_session_lease() {
        return Err(io::Error::new(
            ErrorKind::WouldBlock,
            "owner lease must be acquired before the Session lease; retry outside the nested Session operation",
        ));
    }
    if gwt_agent::validate_session_id_path_component(&expected.session_id).is_err() {
        return Ok(None);
    }
    let owner = match expected.owner_kind.as_str() {
        "spec" => ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Spec,
            number: expected.owner_number,
        },
        "issue" => ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: expected.owner_number,
        },
        _ => return Ok(None),
    };
    let session_path = sessions_dir.join(format!("{}.toml", expected.session_id));
    let route_session = match gwt_agent::Session::load(&session_path) {
        Ok(session) => session,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if route_session.execution_binding.as_ref() != Some(expected) {
        return Ok(None);
    }
    let context = match GenerationTransactionContext::resolve(&route_session.worktree_path, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(None),
        Err(error) => return Err(error),
    };

    with_resolved_generation_owner_lease(&context, |context| {
        gwt_agent::with_session_lease_wait(
            sessions_dir,
            &expected.session_id,
            session_wait,
            |session| {
                let canonical_worktree = match dunce::canonicalize(&session.worktree_path) {
                    Ok(path) => path,
                    Err(_) => return Ok(None),
                };
                if session.id != expected.session_id
                    || session.execution_binding.as_ref() != Some(expected)
                    || session.repo_hash.as_deref() != Some(expected.repo_hash.as_str())
                    || session.linked_issue_number != Some(expected.owner_number)
                    || canonical_worktree != context.worktree
                    || !current_execution_binding_matches_context(
                        context,
                        &expected.session_id,
                        &expected.identity,
                        authority,
                    )?
                {
                    return Ok(None);
                }
                Ok(Some(operation()))
            },
        )
    })
}

/// Execute one active-generation operation only while the exact durable
/// Session incarnation and current owner binding are leased together.
///
/// Unlike [`with_current_active_execution_binding_lease`], this variant also
/// fences branch, Agent identity, project root, and capability generation.
/// Recovery paths use it to keep Session replacement from racing a Work
/// publication or its readback.
pub fn with_current_active_session_execution_identity_lease<T>(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionIdentity,
    operation: impl FnOnce() -> T,
) -> io::Result<Option<T>> {
    if gwt_agent::current_thread_holds_session_lease() {
        return Err(io::Error::new(
            ErrorKind::WouldBlock,
            "owner lease must be acquired before the Session lease; retry outside the nested Session operation",
        ));
    }
    let binding = &expected.execution_binding;
    if expected.session_id != binding.session_id
        || gwt_agent::validate_session_id_path_component(&expected.session_id).is_err()
    {
        return Ok(None);
    }
    let owner = match binding.owner_kind.as_str() {
        "spec" => ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Spec,
            number: binding.owner_number,
        },
        "issue" => ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: binding.owner_number,
        },
        _ => return Ok(None),
    };
    let session_path = sessions_dir.join(format!("{}.toml", expected.session_id));
    let route_session = match gwt_agent::Session::load(&session_path) {
        Ok(session) => session,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if gwt_agent::SessionExecutionIdentity::from_session(&route_session)
        .ok()
        .flatten()
        .as_ref()
        != Some(expected)
    {
        return Ok(None);
    }
    let context = match GenerationTransactionContext::resolve(&expected.worktree_path, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(None),
        Err(error) => return Err(error),
    };

    with_resolved_generation_owner_lease(&context, |context| {
        gwt_agent::with_session_lease(sessions_dir, &expected.session_id, |session| {
            let canonical_worktree = match dunce::canonicalize(&session.worktree_path) {
                Ok(path) => path,
                Err(_) => return Ok(None),
            };
            if gwt_agent::SessionExecutionIdentity::from_session(session)
                .ok()
                .flatten()
                .as_ref()
                != Some(expected)
                || canonical_worktree != context.worktree
                || !current_active_execution_binding_matches_context(
                    context,
                    &expected.session_id,
                    &binding.identity,
                )?
            {
                return Ok(None);
            }
            Ok(Some(operation()))
        })
    })
}

/// Execute one still-Prepared candidate operation while the owner and exact
/// durable Session are leased together. Prepared candidates are not current
/// generation holders yet, so they require their own authority predicate.
fn with_prepared_session_execution_identity_lease<T>(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionIdentity,
    operation: impl FnOnce() -> T,
) -> io::Result<Option<T>> {
    if gwt_agent::current_thread_holds_session_lease() {
        return Err(io::Error::new(
            ErrorKind::WouldBlock,
            "owner lease must be acquired before the Session lease; retry outside the nested Session operation",
        ));
    }
    let binding = &expected.execution_binding;
    let owner = match binding.owner_kind.as_str() {
        "spec" => ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Spec,
            number: binding.owner_number,
        },
        "issue" => ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: binding.owner_number,
        },
        _ => return Ok(None),
    };
    let context = match GenerationTransactionContext::resolve(&expected.worktree_path, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(None),
        Err(error) => return Err(error),
    };
    with_resolved_generation_owner_lease(&context, |context| {
        if !prepared_execution_binding_matches(
            &context.worktree,
            owner,
            &expected.session_id,
            &binding.identity,
        )? {
            return Ok(None);
        }
        gwt_agent::with_session_lease(sessions_dir, &expected.session_id, |session| {
            let canonical_worktree = match dunce::canonicalize(&session.worktree_path) {
                Ok(path) => path,
                Err(_) => return Ok(None),
            };
            if gwt_agent::SessionExecutionIdentity::from_session(session)
                .ok()
                .flatten()
                .as_ref()
                != Some(expected)
                || canonical_worktree != context.worktree
            {
                return Ok(None);
            }
            Ok(Some(operation()))
        })
    })
}

/// Claim one Prepared candidate before its pane is created.
///
/// The owner lease serializes cross-process contenders, the exact candidate
/// Session is created once, and the durable launch handshake keeps later
/// contenders out until the winner publishes runtime proof or rolls back.
pub fn claim_prepared_session_launch(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    sessions_dir: &Path,
    candidate: &gwt_agent::Session,
) -> io::Result<Option<gwt_agent::SessionActiveLaunchHandshake>> {
    let Some(expected) = gwt_agent::SessionExecutionIdentity::from_session(candidate)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?
    else {
        return Ok(None);
    };
    if expected.execution_binding.owner_kind != owner.kind.as_str()
        || expected.execution_binding.owner_number != owner.number
    {
        return Ok(None);
    }
    let nonce = uuid::Uuid::new_v4().to_string();
    let Some(host_started_at) = crate::process::host_process_start_time(std::process::id()) else {
        return Ok(None);
    };
    with_generation_owner_lease(worktree, owner, |context| {
        if !prepared_execution_binding_matches(
            &context.worktree,
            owner,
            &expected.session_id,
            &expected.execution_binding.identity,
        )? {
            return Ok(None);
        }
        candidate.save_if_absent(sessions_dir)?;
        gwt_agent::with_session_lease(sessions_dir, &expected.session_id, |session| {
            if gwt_agent::SessionExecutionIdentity::from_session(session)
                .ok()
                .flatten()
                .as_ref()
                != Some(&expected)
            {
                return Ok(None);
            }
            if reconcile_active_launch_handshake_under_lease(sessions_dir, &expected)?
                || exact_session_runtime_fences_active_launch(sessions_dir, &expected)?
            {
                return Ok(None);
            }
            gwt_agent::begin_session_active_launch_handshake_under_lease(
                sessions_dir,
                &expected,
                &nonce,
                host_started_at,
            )
        })
    })
}

/// Execute a destructive local-holder stop only while the exact Active owner,
/// durable Session incarnation, and process-local runtime proof remain leased.
/// The callback may suspend the matching process-local capability before it
/// signals the child, and may persist terminal proof through the under-lease
/// gwt-agent primitive.
pub fn with_exact_active_manual_runtime_lease<T>(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionIdentity,
    runtime: gwt_agent::ManualLaunchRuntimeProof,
    retain_handoff_after_success: bool,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<Option<T>> {
    if runtime.host_pid != std::process::id() || runtime.runtime_incarnation == 0 {
        return Ok(None);
    }
    let Some(host_started_at) = crate::process::host_process_start_time(std::process::id()) else {
        return Ok(None);
    };
    let handoff_nonce = uuid::Uuid::new_v4().to_string();
    with_current_active_session_execution_identity_lease(sessions_dir, expected, || {
        let sidecar = gwt_agent::runtime_state_path_for_pid(
            sessions_dir,
            runtime.host_pid,
            &expected.session_id,
        );
        let current = match gwt_agent::SessionRuntimeState::load(&sidecar) {
            Ok(current) => current,
            Err(error) => return Err(error),
        };
        if current.execution_identity.as_ref() != Some(expected)
            || current.runtime_incarnation != Some(runtime.runtime_incarnation)
            || matches!(
                current.status,
                gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
            )
        {
            return Err(generation_conflict(
                "manual holder runtime proof changed before stop",
            ));
        }
        let handoff = gwt_agent::begin_session_manual_handoff_under_lease(
            sessions_dir,
            expected,
            &handoff_nonce,
            host_started_at,
        )?
        .ok_or_else(|| {
            io::Error::new(
                ErrorKind::PermissionDenied,
                "manual holder is fenced by another Session authority transition",
            )
        })?;
        match operation() {
            Ok(value) => {
                if !retain_handoff_after_success
                    && !gwt_agent::clear_session_manual_handoff_under_lease(sessions_dir, &handoff)?
                {
                    return Err(io::Error::other(
                        "completed Session stop lost its exact manual handoff fence",
                    ));
                }
                Ok(value)
            }
            Err(operation_error) => {
                if gwt_agent::clear_session_manual_handoff_under_lease(sessions_dir, &handoff)? {
                    Err(operation_error)
                } else {
                    Err(io::Error::other(format!(
                        "{operation_error}; exact durable manual handoff rollback failed"
                    )))
                }
            }
        }
    })
    .and_then(|result| result.transpose())
}

/// Durably fence an in-place Active Session launch before capability
/// issuance. The owner → Session lease order is shared with terminal
/// successor settlement, so exactly one side wins across gwt processes.
pub fn begin_active_session_launch_handshake(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionIdentity,
) -> io::Result<Option<gwt_agent::SessionActiveLaunchHandshake>> {
    let nonce = uuid::Uuid::new_v4().to_string();
    let Some(host_started_at) = crate::process::host_process_start_time(std::process::id()) else {
        return Ok(None);
    };
    with_current_active_session_execution_identity_lease(sessions_dir, expected, || {
        if reconcile_active_launch_handshake_under_lease(sessions_dir, expected)? {
            return Ok(None);
        }
        if exact_session_runtime_fences_active_launch(sessions_dir, expected)? {
            return Ok(None);
        }
        gwt_agent::begin_session_active_launch_handshake_under_lease(
            sessions_dir,
            expected,
            &nonce,
            host_started_at,
        )
    })
    .and_then(|result| result.transpose())
    .map(Option::flatten)
}

fn exact_session_runtime_fences_active_launch(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionIdentity,
) -> io::Result<bool> {
    let runtime_root = sessions_dir.join("runtime");
    let entries = match fs::read_dir(&runtime_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for namespace in entries {
        let namespace = namespace?;
        let sidecar = namespace
            .path()
            .join(format!("{}.json", expected.session_id));
        let runtime = match gwt_agent::SessionRuntimeState::load(&sidecar) {
            Ok(runtime) => runtime,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Active launch runtime evidence is unreadable: {error}"),
                ))
            }
        };
        if runtime.execution_identity.as_ref() != Some(expected)
            || runtime.runtime_incarnation.is_none_or(|value| value == 0)
        {
            return Err(invalid_generation_data(
                "Active launch runtime evidence does not match the exact Session identity",
            ));
        }
        if matches!(
            runtime.status,
            gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
        ) {
            match (runtime.child_pid, runtime.child_started_at) {
                (Some(child_pid), Some(child_started_at))
                    if child_pid > 0 && child_started_at > 0 =>
                {
                    if crate::process::exact_pty_process_tree_is_alive(child_pid, child_started_at)
                    {
                        return Ok(true);
                    }
                }
                (None, None) => return Ok(true),
                _ => return Ok(true),
            }
            continue;
        }
        match (runtime.child_pid, runtime.child_started_at) {
            (Some(child_pid), Some(child_started_at)) if child_pid > 0 && child_started_at > 0 => {
                if crate::process::exact_pty_process_tree_is_alive(child_pid, child_started_at) {
                    return Ok(true);
                }
            }
            _ => return Ok(true),
        }
    }
    Ok(false)
}

/// Reconcile an abandoned Active launch fence while the exact owner and
/// Session leases are held. A live/reused Host incarnation or a nonterminal
/// exact runtime remains fenced; only a dead Host with no live runtime proof
/// can have its exact marker cleared.
fn reconcile_active_launch_handshake_under_lease(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionIdentity,
) -> io::Result<bool> {
    let Some(handshake) =
        gwt_agent::read_session_active_launch_handshake_under_lease(sessions_dir, expected)?
    else {
        return Ok(false);
    };
    if crate::process::host_process_start_time(handshake.host_pid)
        == Some(handshake.host_started_at)
    {
        return Ok(true);
    }
    match handshake.phase {
        gwt_agent::SessionActiveLaunchPhase::LegacyUnclassified => {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "abandoned Active launch handshake has no durable launch phase",
            ));
        }
        gwt_agent::SessionActiveLaunchPhase::ChildSpawned {
            child_pid,
            child_started_at,
        } if crate::process::exact_pty_process_tree_is_alive(child_pid, child_started_at) => {
            return Ok(true);
        }
        gwt_agent::SessionActiveLaunchPhase::PreSpawn
        | gwt_agent::SessionActiveLaunchPhase::ChildSpawned { .. } => {}
    }
    let runtime_path = gwt_agent::runtime_state_path_for_pid(
        sessions_dir,
        handshake.host_pid,
        &expected.session_id,
    );
    match gwt_agent::SessionRuntimeState::load(&runtime_path) {
        Ok(runtime)
            if runtime.execution_identity.as_ref() == Some(expected)
                && !matches!(
                    runtime.status,
                    gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
                ) =>
        {
            match (runtime.child_pid, runtime.child_started_at) {
                (Some(child_pid), Some(child_started_at))
                    if crate::process::exact_pty_process_tree_is_alive(
                        child_pid,
                        child_started_at,
                    ) =>
                {
                    return Ok(true);
                }
                (Some(_), Some(_)) => {}
                _ => {
                    return Err(io::Error::new(
                        ErrorKind::InvalidData,
                        "abandoned Active launch runtime lacks exact child process identity",
                    ));
                }
            }
        }
        Ok(runtime) if runtime.execution_identity.as_ref() == Some(expected) => {}
        Ok(_)
            if matches!(
                handshake.phase,
                gwt_agent::SessionActiveLaunchPhase::PreSpawn
                    | gwt_agent::SessionActiveLaunchPhase::ChildSpawned { .. }
            ) => {}
        Ok(_) => {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "abandoned Active launch runtime proof does not match the exact Session identity",
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if !gwt_agent::clear_session_active_launch_handshake_under_lease(sessions_dir, &handshake)? {
        return Ok(true);
    }
    tracing::warn!(
        target: "gwt::execution_authority",
        session_id = %expected.session_id,
        host_pid = handshake.host_pid,
        host_started_at = handshake.host_started_at,
        handshake_nonce_digest = %sha256_hex(handshake.nonce.as_bytes())[..16],
        "cleared abandoned Active Session launch handshake after exact Host/runtime reconciliation"
    );
    Ok(false)
}

/// Clear the exact Active launch fence after a failed launch or after its
/// Running runtime proof is durably visible. A concurrent replacement marker
/// or owner transition fails closed.
pub fn finish_active_session_launch_handshake(
    sessions_dir: &Path,
    handshake: &gwt_agent::SessionActiveLaunchHandshake,
) -> io::Result<bool> {
    let active = with_current_active_session_execution_identity_lease(
        sessions_dir,
        &handshake.execution_identity,
        || gwt_agent::clear_session_active_launch_handshake_under_lease(sessions_dir, handshake),
    )
    .and_then(|result| result.transpose())?;
    if let Some(cleared) = active {
        return Ok(cleared);
    }
    with_prepared_session_execution_identity_lease(
        sessions_dir,
        &handshake.execution_identity,
        || gwt_agent::clear_session_active_launch_handshake_under_lease(sessions_dir, handshake),
    )
    .and_then(|result| result.transpose())
    .map(|result| result.unwrap_or(false))
}

/// Durably attach the exact spawned child identity to an Active launch fence.
/// The owner and Session remain leased across the marker CAS so terminal
/// settlement cannot observe an unclassified post-spawn handshake.
pub fn mark_active_session_launch_handshake_child_spawned(
    sessions_dir: &Path,
    handshake: &gwt_agent::SessionActiveLaunchHandshake,
    child_pid: u32,
    child_started_at: u64,
) -> io::Result<Option<gwt_agent::SessionActiveLaunchHandshake>> {
    let active = with_current_active_session_execution_identity_lease(
        sessions_dir,
        &handshake.execution_identity,
        || {
            gwt_agent::mark_session_active_launch_handshake_child_spawned_under_lease(
                sessions_dir,
                handshake,
                child_pid,
                child_started_at,
            )
        },
    )
    .and_then(|result| result.transpose())?;
    if active.is_some() {
        return Ok(active.flatten());
    }
    with_prepared_session_execution_identity_lease(
        sessions_dir,
        &handshake.execution_identity,
        || {
            gwt_agent::mark_session_active_launch_handshake_child_spawned_under_lease(
                sessions_dir,
                handshake,
                child_pid,
                child_started_at,
            )
        },
    )
    .and_then(|result| result.transpose())
    .map(Option::flatten)
}

/// Execute one active-generation operation beneath the canonical
/// worktree-global -> owner -> Session lease hierarchy.
///
/// Use this variant when `operation` must update a worktree-global trusted
/// record (for example a terminal Work settlement receipt). The resolved
/// trusted directory is passed to the callback so it can write through the
/// already-held global lease without attempting a nested acquisition.
pub fn with_current_active_session_execution_identity_global_lease<T>(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionIdentity,
    operation: impl FnOnce(&Path) -> T,
) -> io::Result<Option<T>> {
    with_current_session_execution_identity_global_lease(
        sessions_dir,
        expected,
        CurrentExecutionBindingAuthority::ActiveMutation,
        operation,
    )
}

pub(crate) fn with_blocked_build_abort_session_execution_identity_global_lease<T>(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionIdentity,
    operation: impl FnOnce(&Path) -> T,
) -> io::Result<Option<T>> {
    with_current_session_execution_identity_global_lease(
        sessions_dir,
        expected,
        CurrentExecutionBindingAuthority::BlockedBuildAbort,
        operation,
    )
}

fn with_current_session_execution_identity_global_lease<T>(
    sessions_dir: &Path,
    expected: &gwt_agent::SessionExecutionIdentity,
    authority: CurrentExecutionBindingAuthority,
    operation: impl FnOnce(&Path) -> T,
) -> io::Result<Option<T>> {
    if gwt_agent::current_thread_holds_session_lease() {
        return Err(io::Error::new(
            ErrorKind::WouldBlock,
            "worktree-global and owner leases must be acquired before the Session lease; retry outside the nested Session operation",
        ));
    }
    let binding = &expected.execution_binding;
    if expected.session_id != binding.session_id
        || gwt_agent::validate_session_id_path_component(&expected.session_id).is_err()
    {
        return Ok(None);
    }
    let owner = match binding.owner_kind.as_str() {
        "spec" => ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Spec,
            number: binding.owner_number,
        },
        "issue" => ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: binding.owner_number,
        },
        _ => return Ok(None),
    };
    let session_path = sessions_dir.join(format!("{}.toml", expected.session_id));
    let route_session = match gwt_agent::Session::load(&session_path) {
        Ok(session) => session,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if gwt_agent::SessionExecutionIdentity::from_session(&route_session)
        .ok()
        .flatten()
        .as_ref()
        != Some(expected)
    {
        return Ok(None);
    }
    let context = match GenerationTransactionContext::resolve(&expected.worktree_path, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(None),
        Err(error) => return Err(error),
    };

    crate::cli::trusted_store::with_write_lease_for_resolved_dir(
        &context.worktree_trusted_dir,
        || {
            context.validate_unchanged()?;
            with_resolved_generation_owner_lease(&context, |context| {
                gwt_agent::with_session_lease(sessions_dir, &expected.session_id, |session| {
                    let canonical_worktree = match dunce::canonicalize(&session.worktree_path) {
                        Ok(path) => path,
                        Err(_) => return Ok(None),
                    };
                    let execution_binding_matches = match authority {
                        CurrentExecutionBindingAuthority::ActiveMutation => {
                            current_active_execution_binding_matches_context(
                                context,
                                &expected.session_id,
                                &binding.identity,
                            )?
                        }
                        CurrentExecutionBindingAuthority::BlockedBuildAbort => {
                            blocked_build_abort_execution_binding_matches_context(
                                context,
                                &expected.session_id,
                                &binding.identity,
                            )?
                        }
                        CurrentExecutionBindingAuthority::PrMutation => false,
                    };
                    if gwt_agent::SessionExecutionIdentity::from_session(session)
                        .ok()
                        .flatten()
                        .as_ref()
                        != Some(expected)
                        || canonical_worktree != context.worktree
                        || !execution_binding_matches
                    {
                        return Ok(None);
                    }
                    Ok(Some(operation(&context.worktree_trusted_dir)))
                })
            })
        },
    )
}

/// Reachable repair guidance for an integrity failure.
#[must_use]
pub(crate) fn integrity_repair_guidance(_status: ExecutionControlStatus) -> &'static str {
    "Run JSON operation `execution.repair` with a non-empty `params.reason`; it quarantines the corrupt authority, records a trusted repair audit, and materializes a fresh Active generation."
}

/// Resolve the record path for a worktree.
#[must_use]
pub fn state_path(worktree: &Path) -> PathBuf {
    worktree.join(EXECUTION_CONTROL_STATE_RELATIVE)
}

/// Read the legacy flat record. Generation-aware [`load`] first discovers
/// authority and converts missing/malformed projection failures into an
/// integrity-invalid sentinel so historical fail-open consumers stay closed.
fn read_record_contents(worktree: &Path) -> io::Result<Option<String>> {
    // P9b: the repo-scoped trusted copy is authoritative; the worktree
    // mirror is a legacy/degenerate fallback only.
    let contents = match crate::cli::trusted_store::read(worktree, "execution-control.json")? {
        Some(contents) => contents,
        None => match fs::read_to_string(state_path(worktree)) {
            Ok(contents) => contents,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        },
    };
    Ok(Some(contents))
}

#[derive(Debug, Clone)]
struct GenerationAuthorityHint {
    owner: Option<ExecutionOwnerKey>,
    projection: Option<ExecutionControlRecord>,
    strictly_validated_owner: Option<ExecutionOwnerKey>,
}

fn generation_authority_hint(worktree: &Path) -> io::Result<Option<GenerationAuthorityHint>> {
    let worktree_binding = worktree_binding_hash(worktree);
    let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(worktree);
    let mut pointer_present = false;
    let mut pointer_owner = None;
    let mut pointer_paths = Vec::new();
    if let Some(dir) = &trusted_dir {
        pointer_paths.push(dir.join(GENERATION_POINTER_FILE));
    }
    pointer_paths.push(generation_pointer_path(worktree));
    for path in pointer_paths {
        match fs::read_to_string(&path) {
            Ok(contents) => {
                pointer_present = true;
                if pointer_owner.is_none() {
                    pointer_owner = serde_json::from_str::<ExecutionGenerationPointer>(&contents)
                        .ok()
                        .map(|pointer| pointer.owner);
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => {
                // The worktree-specific authority marker exists but is
                // unreadable. Its mere presence must block legacy fail-open
                // readers even though no owner metadata can be trusted.
                pointer_present = true;
            }
        }
    }

    if let Some(owner) = pointer_owner {
        if let Ok(Some(ledger)) = load_generation_ledger(worktree, owner) {
            let current = ledger.current_generation();
            if current.is_some_and(|generation| {
                generation.identity.worktree_binding_hash == worktree_binding
            }) {
                let projection = current.and_then(|generation| {
                    serde_json::from_str::<ExecutionControlRecord>(
                        ledger.effective_projection_for(generation),
                    )
                    .ok()
                    .map(hydrate_recovery_envelopes)
                });
                return Ok(Some(GenerationAuthorityHint {
                    owner: Some(owner),
                    projection,
                    strictly_validated_owner: Some(owner),
                }));
            }
        }
    }

    let mut matched_owner = None;
    let mut matched_projection = None;
    if let Some(trusted_dir) = &trusted_dir {
        if let Some(trusted_root) = trusted_dir.parent() {
            let owners_root = trusted_root.join("execution-owners");
            match fs::read_dir(&owners_root) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let ledger_path = entry.path().join(GENERATION_LEDGER_FILE);
                        let Ok(contents) = fs::read_to_string(&ledger_path) else {
                            continue;
                        };
                        let Ok(ledger) =
                            serde_json::from_str::<ExecutionGenerationLedger>(&contents)
                        else {
                            continue;
                        };
                        let current_for_worktree =
                            ledger.current_generation().is_some_and(|generation| {
                                generation.identity.worktree_binding_hash == worktree_binding
                            });
                        if !current_for_worktree
                            || validate_generation_ledger(&ledger, ledger.owner).is_err()
                        {
                            continue;
                        }
                        matched_owner = Some(ledger.owner);
                        matched_projection = ledger.current_generation().and_then(|generation| {
                            serde_json::from_str::<ExecutionControlRecord>(
                                ledger.effective_projection_for(generation),
                            )
                            .ok()
                            .map(hydrate_recovery_envelopes)
                        });
                        break;
                    }
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(_) if pointer_present => {}
                Err(error) => return Err(error),
            }
        }
    }

    if !pointer_present && matched_owner.is_none() {
        return Ok(None);
    }
    Ok(Some(GenerationAuthorityHint {
        owner: pointer_owner.or(matched_owner),
        projection: matched_projection,
        strictly_validated_owner: None,
    }))
}

fn invalid_generation_authority_record(hint: &GenerationAuthorityHint) -> ExecutionControlRecord {
    let owner = hint.owner.unwrap_or(ExecutionOwnerKey {
        kind: ExecutionOwnerKind::Issue,
        number: 1,
    });
    let mut record = hint
        .projection
        .clone()
        .unwrap_or_else(|| ExecutionControlRecord {
            owner_kind: owner.kind,
            owner_number: owner.number,
            primary_session_id: "invalid-generation-authority".to_string(),
            entrypoint: "generation-authority".to_string(),
            bundled_required_owners: Vec::new(),
            status: ExecutionControlStatus::Active,
            blocked_reason: None,
            missing_verification: None,
            launched_at: Utc::now(),
            settled_at: None,
            transfers: Vec::new(),
            recoveries: Vec::new(),
            content_hash: String::new(),
        });
    record.owner_kind = owner.kind;
    record.owner_number = owner.number;
    record.status = ExecutionControlStatus::Active;
    record.settled_at = None;
    record.content_hash = "invalid-generation-authority".to_string();
    record
}

pub fn load(worktree: &Path) -> io::Result<Option<ExecutionControlRecord>> {
    let authority = generation_authority_hint(worktree)?;
    let contents = match read_record_contents(worktree) {
        Ok(Some(contents)) => contents,
        Ok(None) if authority.is_some() => {
            return Ok(authority.as_ref().map(invalid_generation_authority_record))
        }
        Ok(None) => return Ok(None),
        Err(_) if authority.is_some() => {
            return Ok(authority.as_ref().map(invalid_generation_authority_record))
        }
        Err(error) => return Err(error),
    };
    let mut record = match serde_json::from_str::<ExecutionControlRecord>(&contents) {
        Ok(record) => record,
        Err(_) if authority.is_some() => {
            return Ok(authority.as_ref().map(invalid_generation_authority_record))
        }
        Err(error) => return Err(io::Error::new(ErrorKind::InvalidData, error)),
    };
    record = hydrate_recovery_envelopes(record);

    let owner = ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    let strictly_validated = authority
        .as_ref()
        .is_some_and(|hint| hint.strictly_validated_owner == Some(owner));
    let owner_ledger_exists =
        strictly_validated || owner_generation_ledger_exists(worktree, owner)?;
    if !strictly_validated && (authority.is_some() || owner_ledger_exists) {
        match load_generation_ledger(worktree, owner) {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                // Existing authority readers historically treat malformed I/O
                // as unavailable/fail-open. Return a deliberately
                // integrity-failed Active sentinel instead: Stop/PR gates
                // block it, completion projections stay false, and no stale
                // terminal status can be accepted after an old writer or a
                // missing/mismatched generation pointer.
                record.status = ExecutionControlStatus::Active;
                record.settled_at = None;
                record.content_hash = "invalid-generation-authority".to_string();
            }
        }
    }
    Ok(Some(record))
}

/// Whether the execution for `worktree` has settled as `Completed`.
///
/// A completed execution means its implementation and verification lifecycle
/// has settled; a PR handoff may still follow. Coordination-only
/// `workspace.update` calls (such as a post-merge stale reminder) must stop
/// appending to the git-tracked `events.jsonl` after this boundary (Issue
/// #3278). A missing or unreadable record is treated as *not* completed so
/// unlinked / standalone launches keep their existing append behavior.
#[must_use]
pub fn is_completed(worktree: &Path) -> bool {
    matches!(
        load(worktree),
        Ok(Some(record))
            if record.status == ExecutionControlStatus::Completed && integrity_ok(&record)
    )
}

fn same_execution_lifetime(left: &ExecutionControlRecord, right: &ExecutionControlRecord) -> bool {
    left.owner_kind == right.owner_kind
        && left.owner_number == right.owner_number
        && left.launched_at == right.launched_at
}

fn same_recovery_audit(left: &ExecutionRecovery, right: &ExecutionRecovery) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.previous_recovery_hash.clear();
    left.content_hash.clear();
    right.previous_recovery_hash.clear();
    right.content_hash.clear();
    left == right
}

fn recovery_history_extends(
    previous: &[ExecutionRecovery],
    incoming: &[ExecutionRecovery],
) -> bool {
    incoming.len() >= previous.len()
        && previous
            .iter()
            .zip(incoming)
            .all(|(left, right)| same_recovery_audit(left, right))
}

fn recovery_storage_needs_upgrade(worktree: &Path) -> io::Result<bool> {
    let Some(contents) = read_record_contents(worktree)? else {
        return Ok(false);
    };
    let stored = serde_json::from_str::<ExecutionControlRecord>(&contents)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    Ok(!stored.recoveries.is_empty())
}

fn load_existing_for_save(worktree: &Path) -> io::Result<Option<ExecutionControlRecord>> {
    if crate::cli::trusted_store::trusted_dir_for_worktree(worktree).is_some() {
        let Some(contents) = crate::cli::trusted_store::read(worktree, "execution-control.json")?
        else {
            return Ok(None);
        };
        let record = serde_json::from_str::<ExecutionControlRecord>(&contents)
            .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
        return Ok(Some(hydrate_recovery_envelopes(record)));
    }
    load(worktree)
}

fn serialize_execution_control(record: &ExecutionControlRecord) -> io::Result<Vec<u8>> {
    let mut canonical = record.clone();
    stamp_recovery_chain(&mut canonical.recoveries);
    let mut stored = recovery_storage_projection(&canonical);
    stored.content_hash = compute_content_hash(&canonical);
    serde_json::to_vec_pretty(&stored)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
}

/// Persist the record atomically (hooks read this file concurrently). The
/// integrity hash is recomputed on every save (P9a).
pub fn save(worktree: &Path, record: &ExecutionControlRecord) -> io::Result<()> {
    let record = record.clone();
    let owner = ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    if read_generation_pointer_contents(worktree)?.is_some()
        || owner_generation_ledger_exists(worktree, owner)?
    {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "flat execution-control writes are refused after owner generation ledger ownership; use the generation-aware lifecycle/CAS operation",
        ));
    }
    if record.transfers.iter().any(is_recovery_envelope_transfer) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "ownership transfer reason uses the reserved recovery-envelope namespace",
        ));
    }
    if let Some(previous) = load_existing_for_save(worktree)? {
        if previous
            .content_hash
            .starts_with("invalid-recovery-envelope:")
        {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "malformed recovery envelopes require a fresh execution lifetime",
            ));
        }
        if same_execution_lifetime(&previous, &record) {
            if !integrity_ok(&previous) {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "an integrity-failed execution record cannot be rewritten in the same lifetime",
                ));
            }
            if !recovery_history_extends(&previous.recoveries, &record.recoveries) {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "execution recovery history is append-only within one execution lifetime",
                ));
            }
        }
    }
    let serialized = serialize_execution_control(&record)?;
    // P9b: trusted copy is authoritative; the mirror is informational.
    crate::cli::trusted_store::write_with_mirror(
        worktree,
        "execution-control.json",
        &state_path(worktree),
        &serialized,
    )
}

fn with_generation_owner_lease<T>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    operation: impl FnOnce(&GenerationTransactionContext) -> io::Result<T>,
) -> io::Result<T> {
    let context = GenerationTransactionContext::resolve(worktree, owner)?;
    with_resolved_generation_owner_lease(&context, operation)
}

fn with_generation_activation_leases<T>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    operation: impl FnOnce(&GenerationTransactionContext) -> io::Result<T>,
) -> io::Result<T> {
    let context = GenerationTransactionContext::resolve(worktree, owner)?;
    // Activation is the only owner-ledger transaction that also publishes
    // worktree-global authority. Keep the lock order global -> owner so it
    // matches the lifecycle writers and cannot deadlock with them.
    crate::cli::trusted_store::with_write_lease_for_resolved_dir(
        &context.worktree_trusted_dir,
        || {
            context.validate_unchanged()?;
            with_resolved_generation_owner_lease(&context, operation)
        },
    )
}

fn with_resolved_generation_owner_lease<T>(
    context: &GenerationTransactionContext,
    operation: impl FnOnce(&GenerationTransactionContext) -> io::Result<T>,
) -> io::Result<T> {
    crate::cli::trusted_store::with_write_lease_for_resolved_dir(&context.owner_dir, || {
        context.validate_unchanged()?;
        operation(context)
    })
}

fn generation_pointer_owner_hint(worktree: &Path) -> io::Result<Option<ExecutionOwnerKey>> {
    let Some(contents) = read_generation_pointer_contents(worktree)? else {
        return Ok(None);
    };
    serde_json::from_str::<ExecutionGenerationPointer>(&contents)
        .map(|pointer| Some(pointer.owner))
        .map_err(|error| {
            invalid_generation_data(format!(
                "malformed execution generation pointer during activation CAS: {error}"
            ))
        })
}

fn validate_generation_activation_owner(
    context: &GenerationTransactionContext,
    activated_repair: bool,
) -> io::Result<()> {
    let reject_foreign = |source: &str, actual: ExecutionOwnerKey| {
        generation_conflict(format!(
            "generation activation CAS lost: {source} owner {}#{} replaced requested owner {}#{}",
            actual.kind.as_str(),
            actual.number,
            context.owner.kind.as_str(),
            context.owner.number,
        ))
    };

    if let Some(owner) = generation_pointer_owner_hint(&context.worktree)? {
        if owner != context.owner {
            return Err(reject_foreign("generation pointer", owner));
        }
    }
    if let Some(owner) = recovery_projection_owner_hint(&context.worktree)? {
        if owner != context.owner {
            return Err(reject_foreign("execution projection", owner));
        }
    }

    let mut authority_error = None;
    match current_generation_owner(&context.worktree) {
        Ok(Some(owner)) if owner == context.owner => return Ok(()),
        Ok(Some(owner)) => return Err(reject_foreign("current generation", owner)),
        Ok(None) => {}
        Err(error) => authority_error = Some(error),
    }
    match recovery_generation_owner(&context.worktree) {
        Ok(Some(owner)) if owner == context.owner => return Ok(()),
        Ok(Some(owner)) => return Err(reject_foreign("recovery generation", owner)),
        Ok(None) => {}
        Err(error) => authority_error = Some(error),
    }

    // An Activated owner ledger is the durable commit. Its idempotent retry
    // may repair a ledger-first partial write or re-create both missing
    // worktree artifacts, but the foreign-owner checks above always win.
    if activated_repair {
        return Ok(());
    }
    if let Some(error) = authority_error {
        return Err(error);
    }
    // A successor may intentionally claim a new worktree for a repository-
    // scoped owner ledger. No pointer/projection is the unclaimed CAS value;
    // any foreign global evidence was already rejected above.
    Ok(())
}

fn stamp_generation_ledger(ledger: &mut ExecutionGenerationLedger) {
    ledger.content_hash = compute_generation_ledger_hash(ledger);
}

fn append_continuation_attempt(
    ledger: &mut ExecutionGenerationLedger,
    mut attempt: ContinuationAttempt,
) -> ContinuationAttempt {
    attempt.previous_attempt_hash = ledger
        .continuation_attempts
        .last()
        .map_or_else(String::new, |previous| previous.content_hash.clone());
    attempt.content_hash = compute_continuation_attempt_hash(&attempt);
    ledger.continuation_attempts.push(attempt.clone());
    attempt
}

fn append_continuation_validation(
    ledger: &mut ExecutionGenerationLedger,
    mut audit: ExecutionContinuationValidationAudit,
) -> ExecutionContinuationValidationAudit {
    audit.previous_audit_hash = ledger
        .continuation_validations
        .last()
        .map_or_else(String::new, |previous| previous.content_hash.clone());
    audit.content_hash = compute_continuation_validation_hash(&audit);
    ledger.continuation_validations.push(audit.clone());
    audit
}

fn append_generation_takeover_attempt(
    ledger: &mut ExecutionGenerationLedger,
    mut attempt: GenerationTakeoverAttempt,
) -> GenerationTakeoverAttempt {
    attempt.previous_attempt_hash = ledger
        .takeover_attempts
        .last()
        .map_or_else(String::new, |previous| previous.content_hash.clone());
    attempt.content_hash = compute_generation_takeover_attempt_hash(&attempt);
    ledger.takeover_attempts.push(attempt.clone());
    attempt
}

fn latest_generation_event_hash(ledger: &ExecutionGenerationLedger) -> String {
    ledger
        .takeovers
        .iter()
        .map(|event| (event.sequence, event.content_hash.as_str()))
        .chain(
            ledger
                .lifecycle_events
                .iter()
                .map(|event| (event.sequence, event.content_hash.as_str())),
        )
        .max_by_key(|(sequence, _)| *sequence)
        .map_or_else(String::new, |(_, hash)| hash.to_string())
}

fn append_takeover_event(
    ledger: &mut ExecutionGenerationLedger,
    mut event: GenerationTakeoverAudit,
) {
    event.sequence = ledger.next_generation_event_sequence();
    event.previous_event_hash = latest_generation_event_hash(ledger);
    event.content_hash = compute_takeover_event_hash(&event);
    ledger.takeovers.push(event);
}

fn append_lifecycle_event(
    ledger: &mut ExecutionGenerationLedger,
    mut event: GenerationLifecycleEvent,
) {
    event.sequence = ledger.next_generation_event_sequence();
    event.previous_event_hash = latest_generation_event_hash(ledger);
    event.content_hash = compute_lifecycle_event_hash(&event);
    ledger.lifecycle_events.push(event);
}

fn write_owner_ledger(
    context: &GenerationTransactionContext,
    ledger: &ExecutionGenerationLedger,
) -> io::Result<()> {
    context.validate_unchanged()?;
    if ledger.owner != context.owner {
        return Err(invalid_generation_data(
            "generation ledger owner does not match the resolved transaction owner",
        ));
    }
    validate_generation_ledger(ledger, ledger.owner)?;
    let serialized = serde_json::to_vec_pretty(ledger).map_err(|error| {
        invalid_generation_data(format!("serialize generation ledger: {error}"))
    })?;
    crate::cli::trusted_store::write_to_resolved_dir(
        &context.owner_dir,
        GENERATION_LEDGER_FILE,
        &serialized,
    )
}

fn generation_pointer(
    ledger: &ExecutionGenerationLedger,
    projection: &str,
) -> io::Result<ExecutionGenerationPointer> {
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    let mut pointer = ExecutionGenerationPointer {
        schema_version: GENERATION_LEDGER_SCHEMA_VERSION,
        owner: ledger.owner,
        current_generation_id: current.identity.generation_id.clone(),
        current_generation_content_hash: current.content_hash.clone(),
        projection_content_hash: sha256_hex(projection),
        content_hash: String::new(),
    };
    pointer.content_hash = compute_generation_pointer_hash(&pointer);
    Ok(pointer)
}

fn write_generation_pointer(
    context: &GenerationTransactionContext,
    ledger: &ExecutionGenerationLedger,
    projection: &str,
) -> io::Result<()> {
    context.validate_unchanged()?;
    let pointer = generation_pointer(ledger, projection)?;
    let serialized = serde_json::to_vec_pretty(&pointer).map_err(|error| {
        invalid_generation_data(format!("serialize execution generation pointer: {error}"))
    })?;
    crate::cli::trusted_store::write_to_resolved_dir(
        &context.worktree_trusted_dir,
        GENERATION_POINTER_FILE,
        &serialized,
    )?;
    if let Err(error) =
        gwt_github::cache::write_atomic(&generation_pointer_path(&context.worktree), &serialized)
    {
        tracing::warn!(
            ?error,
            path = %generation_pointer_path(&context.worktree).display(),
            "worktree generation pointer mirror write failed after trusted store write"
        );
    }
    Ok(())
}

fn write_execution_projection(
    context: &GenerationTransactionContext,
    projection: &str,
) -> io::Result<()> {
    context.validate_unchanged()?;
    crate::cli::trusted_store::write_to_resolved_dir(
        &context.worktree_trusted_dir,
        "execution-control.json",
        projection.as_bytes(),
    )?;
    if let Err(error) =
        gwt_github::cache::write_atomic(&state_path(&context.worktree), projection.as_bytes())
    {
        tracing::warn!(
            ?error,
            path = %state_path(&context.worktree).display(),
            "worktree execution projection mirror write failed after trusted store write"
        );
    }
    Ok(())
}

fn write_activated_generation(
    context: &GenerationTransactionContext,
    ledger: &ExecutionGenerationLedger,
    projection: &str,
) -> io::Result<()> {
    // The ledger is authoritative and is committed first. A crash before the
    // projection/pointer pair completes is intentionally fail-closed on the
    // next read; the coordinator can then abort/repair the uncommitted launch
    // rather than accepting a ghost flat ECR.
    write_owner_ledger(context, ledger)?;
    #[cfg(test)]
    fail_generation_write_if_requested(GenerationWriteFailurePoint::AfterLedger)?;
    write_execution_projection(context, projection)?;
    #[cfg(test)]
    fail_generation_write_if_requested(GenerationWriteFailurePoint::AfterProjection)?;
    write_generation_pointer(context, ledger, projection)
}

fn deterministic_generation_id(seed: &[u8]) -> String {
    let digest = sha256_hex(seed);
    format!("gen-{}", &digest[..24])
}

fn parse_legacy_generation_source(
    contents: &str,
    owner: ExecutionOwnerKey,
) -> io::Result<ExecutionControlRecord> {
    let record = serde_json::from_str::<ExecutionControlRecord>(contents)
        .map(hydrate_recovery_envelopes)
        .map_err(|error| {
            invalid_generation_data(format!("legacy execution record is malformed: {error}"))
        })?;
    if record.owner_kind != owner.kind || record.owner_number != owner.number {
        return Err(invalid_generation_data(
            "legacy execution record owner does not match the requested generation owner",
        ));
    }
    Ok(record)
}

fn legacy_genesis(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    record: &ExecutionControlRecord,
    exact_projection: String,
) -> ExecutionGeneration {
    let projection_digest = sha256_hex(exact_projection.as_bytes());
    let generation_id = deterministic_generation_id(
        format!(
            "legacy-genesis:{}:{}:{projection_digest}",
            owner.kind.as_str(),
            owner.number
        )
        .as_bytes(),
    );
    let mut generation = ExecutionGeneration {
        identity: ExecutionGenerationIdentity {
            owner,
            generation_id,
            predecessor_generation_id: None,
            predecessor_content_hash: None,
            session_binding_id: format!("legacy-ecr-{}", &record.content_hash[..16]),
            initial_session_id: record.primary_session_id.clone(),
            worktree_binding_hash: worktree_binding_hash(worktree),
            entrypoint: record.entrypoint.clone(),
            activated_at: record.launched_at,
        },
        status: record.status,
        execution_control_json: exact_projection,
        content_hash: String::new(),
    };
    generation.content_hash = compute_generation_hash(&generation);
    generation
}

/// Import one verified flat ECR as deterministic owner-scoped genesis.
///
/// Completed/Blocked imports preserve the exact terminal projection bytes.
/// A legacy Active record requires a caller liveness classification. A
/// classified hashless Active record is canonicalized as part of the atomic
/// import; Unknown remains a zero-mutation refusal.
pub fn ensure_generation_ledger(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    active_disposition: LegacyActiveDisposition,
) -> io::Result<ExecutionGenerationLedger> {
    let context = GenerationTransactionContext::resolve(worktree, owner)?;
    if let Some(ledger) = load_owner_generation_ledger_from_context(&context)? {
        if matches!(load_generation_ledger_from_context(&context), Ok(Some(_))) {
            return Ok(ledger);
        }
        return with_resolved_generation_owner_lease(&context, |context| {
            let ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
                generation_conflict("generation ledger disappeared while repairing an import retry")
            })?;
            let current = ledger.current_generation().ok_or_else(|| {
                invalid_generation_data("execution generation ledger current id is missing")
            })?;
            if current.identity.worktree_binding_hash != context.worktree_binding_hash {
                return Err(generation_conflict(
                    "existing owner generation is bound to a different worktree",
                ));
            }
            let projection = ledger.effective_projection_for(current).to_string();
            write_activated_generation(context, &ledger, &projection)?;
            let readback = load_generation_ledger_from_context(context)?.ok_or_else(|| {
                invalid_generation_data(
                    "generation import retry readback lost generation authority",
                )
            })?;
            Ok(readback)
        });
    }

    let source_projection = read_legacy_projection_from_context(&context)?.ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            "no legacy execution record exists to import as generation genesis",
        )
    })?;
    let source_record = parse_legacy_generation_source(&source_projection, owner)?;
    if source_record.status == ExecutionControlStatus::Active
        && matches!(active_disposition, LegacyActiveDisposition::Unknown)
    {
        return Err(generation_backfill_required(
            "legacy Active execution liveness is unknown; classify it before generation backfill",
        ));
    }
    let (source_record, imported_projection) = if source_record.content_hash.is_empty() {
        if source_record.status != ExecutionControlStatus::Active {
            return Err(invalid_generation_data(
                "only integrity-verified legacy terminal executions can be imported",
            ));
        }
        let canonical =
            String::from_utf8(serialize_execution_control(&source_record)?).map_err(|error| {
                invalid_generation_data(format!(
                    "serialized legacy Active projection is not UTF-8: {error}"
                ))
            })?;
        let canonical_record = parse_legacy_generation_source(&canonical, owner)?;
        (canonical_record, canonical)
    } else {
        if !integrity_ok(&source_record) {
            return Err(invalid_generation_data(
                "legacy execution record failed integrity validation; no generation state was written",
            ));
        }
        (source_record, source_projection.clone())
    };
    if let LegacyActiveDisposition::Stale {
        new_session_id,
        reason,
        ..
    } = &active_disposition
    {
        if source_record.status == ExecutionControlStatus::Active
            && (new_session_id.trim().is_empty()
                || reason.trim().is_empty()
                || new_session_id == &source_record.primary_session_id)
        {
            return Err(invalid_generation_data(
                "stale legacy takeover requires a distinct non-empty session and reason",
            ));
        }
    }

    with_resolved_generation_owner_lease(&context, |context| {
        if read_owner_ledger_from_dir(&context.owner_dir)?.is_some() {
            return load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
                invalid_generation_data(
                    "owner generation ledger appeared without a valid worktree pointer",
                )
            });
        }
        let current_projection =
            read_legacy_projection_from_context(context)?.ok_or_else(|| {
                generation_conflict("legacy execution projection disappeared during import")
            })?;
        if current_projection != source_projection {
            return Err(generation_conflict(
                "legacy execution projection changed during generation import; retry",
            ));
        }

        let genesis = legacy_genesis(
            &context.worktree,
            owner,
            &source_record,
            imported_projection.clone(),
        );
        let mut ledger = ExecutionGenerationLedger {
            schema_version: GENERATION_LEDGER_SCHEMA_VERSION,
            owner,
            generations: vec![genesis],
            continuation_attempts: Vec::new(),
            takeover_attempts: Vec::new(),
            takeovers: Vec::new(),
            lifecycle_events: Vec::new(),
            continuation_validations: Vec::new(),
            current_generation_id: String::new(),
            content_hash: String::new(),
        };
        ledger.current_generation_id = ledger.generations[0].identity.generation_id.clone();

        let mut committed_projection = imported_projection.clone();
        if source_record.status == ExecutionControlStatus::Active {
            if let LegacyActiveDisposition::Stale {
                new_session_id,
                reason,
                observed_at,
            } = &active_disposition
            {
                let mut transferred = source_record.clone();
                let from_session_id = transferred.primary_session_id.clone();
                transferred.primary_session_id.clone_from(new_session_id);
                transferred.transfers.push(OwnershipTransfer {
                    from_session_id: from_session_id.clone(),
                    to_session_id: new_session_id.clone(),
                    reason: format!("continue-work-stale-takeover: {reason}"),
                    transferred_at: *observed_at,
                });
                committed_projection = String::from_utf8(serialize_execution_control(
                    &transferred,
                )?)
                .map_err(|error| {
                    invalid_generation_data(format!(
                        "serialized execution projection is not UTF-8: {error}"
                    ))
                })?;
                let generation_id = ledger.current_generation_id.clone();
                append_takeover_event(
                    &mut ledger,
                    GenerationTakeoverAudit {
                        sequence: 0,
                        generation_id,
                        from_session_id,
                        to_session_id: new_session_id.clone(),
                        reason: reason.clone(),
                        observed_at: *observed_at,
                        execution_control_json: committed_projection.clone(),
                        previous_event_hash: String::new(),
                        content_hash: String::new(),
                    },
                );
            }
        }
        stamp_generation_ledger(&mut ledger);
        // Even an unchanged mirror-only import must materialize the exact
        // verified projection in trusted storage before publishing the
        // pointer. Pointer-last makes recovery deterministic.
        write_activated_generation(context, &ledger, &committed_projection)?;
        Ok(ledger)
    })
}

fn validate_successor_request(request: &SuccessorRequest) -> io::Result<()> {
    if request.operation_id.trim().is_empty()
        || request.principal_id.trim().is_empty()
        || request.source.trim().is_empty()
        || request.session_binding_id.trim().is_empty()
        || request.initial_session_id.trim().is_empty()
        || request.entrypoint.trim().is_empty()
    {
        return Err(invalid_generation_data(
            "successor request contains an empty identity/binding field",
        ));
    }
    if request
        .work_id
        .as_deref()
        .is_some_and(|work_id| !canonical_continue_work_id(work_id))
    {
        return Err(invalid_generation_data(
            "successor request contains a non-canonical Work identity",
        ));
    }
    Ok(())
}

fn canonical_continue_work_id(work_id: &str) -> bool {
    work_id.trim() == work_id
        && !work_id.is_empty()
        && work_id.len() <= 512
        && !work_id.chars().any(char::is_control)
}

fn latest_operation_attempt<'a>(
    ledger: &'a ExecutionGenerationLedger,
    request: &SuccessorRequest,
    expected_worktree_binding_hash: &str,
) -> io::Result<Option<&'a ContinuationAttempt>> {
    let mut latest = None;
    for attempt in ledger
        .continuation_attempts
        .iter()
        .filter(|attempt| attempt.request.operation_id == request.operation_id)
    {
        if attempt.request != *request {
            return Err(generation_conflict(format!(
                "continuation operation {} was already used by a different principal/source/request",
                request.operation_id
            )));
        }
        if attempt.worktree_binding_hash != expected_worktree_binding_hash {
            return Err(generation_conflict(format!(
                "continuation operation {} is bound to a different worktree",
                request.operation_id
            )));
        }
        latest = Some(attempt);
    }
    Ok(latest)
}

fn validate_generation_takeover_request(request: &GenerationTakeoverRequest) -> io::Result<()> {
    if request.operation_id.trim().is_empty()
        || request.principal_id.trim().is_empty()
        || request.from_session_id.trim().is_empty()
        || request.to_session_id.trim().is_empty()
        || request.from_session_id == request.to_session_id
        || request.reason.trim().is_empty()
    {
        return Err(invalid_generation_data(
            "generation takeover request contains an empty or conflicting identity field",
        ));
    }
    if request
        .work_id
        .as_deref()
        .is_some_and(|work_id| !canonical_continue_work_id(work_id))
        || request.source.as_deref().is_some_and(|source| {
            !matches!(source, "continue-work:resume" | "continue-work:handoff")
        })
    {
        return Err(invalid_generation_data(
            "generation takeover request contains invalid continuation correlation",
        ));
    }
    Ok(())
}

fn latest_generation_takeover_attempt<'a>(
    ledger: &'a ExecutionGenerationLedger,
    request: &GenerationTakeoverRequest,
    expected_worktree_binding_hash: &str,
) -> io::Result<Option<&'a GenerationTakeoverAttempt>> {
    let mut latest = None;
    for attempt in ledger
        .takeover_attempts
        .iter()
        .filter(|attempt| attempt.request.operation_id == request.operation_id)
    {
        if attempt.request != *request {
            return Err(generation_conflict(format!(
                "generation takeover operation {} was already used by a different principal/request",
                request.operation_id
            )));
        }
        if attempt.worktree_binding_hash != expected_worktree_binding_hash {
            return Err(generation_conflict(format!(
                "generation takeover operation {} is bound to a different worktree",
                request.operation_id
            )));
        }
        latest = Some(attempt);
    }
    Ok(latest)
}

fn build_generation_takeover(
    ledger: &ExecutionGenerationLedger,
    request: &GenerationTakeoverRequest,
    expected_worktree_binding_hash: &str,
) -> io::Result<(
    GenerationTakeoverAudit,
    String,
    gwt_agent::ExecutionBindingIdentity,
)> {
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    if ledger.effective_status_for(current) != ExecutionControlStatus::Active
        || current.identity.worktree_binding_hash != expected_worktree_binding_hash
    {
        return Err(generation_conflict(
            "same-generation takeover requires the exact current Active worktree",
        ));
    }
    let mut projection =
        serde_json::from_str::<ExecutionControlRecord>(ledger.effective_projection_for(current))
            .map(hydrate_recovery_envelopes)
            .map_err(|error| {
                invalid_generation_data(format!(
                    "current generation takeover projection is malformed: {error}"
                ))
            })?;
    if projection.primary_session_id != request.from_session_id {
        return Err(generation_conflict(
            "same-generation takeover CAS lost: current owner Session changed",
        ));
    }
    let transfer = OwnershipTransfer {
        from_session_id: request.from_session_id.clone(),
        to_session_id: request.to_session_id.clone(),
        reason: request.reason.clone(),
        transferred_at: request.requested_at,
    };
    projection.primary_session_id = request.to_session_id.clone();
    projection.transfers.push(transfer);
    let projection = serialized_execution_projection(&projection)?;
    let event = GenerationTakeoverAudit {
        sequence: 0,
        generation_id: current.identity.generation_id.clone(),
        from_session_id: request.from_session_id.clone(),
        to_session_id: request.to_session_id.clone(),
        reason: request.reason.clone(),
        observed_at: request.requested_at,
        execution_control_json: projection.clone(),
        previous_event_hash: String::new(),
        content_hash: String::new(),
    };
    let mut planned = ledger.clone();
    append_takeover_event(&mut planned, event.clone());
    let generation = planned
        .current_generation()
        .ok_or_else(|| invalid_generation_data("planned takeover generation is not current"))?;
    let binding = execution_binding_for_generation(&planned, generation);
    Ok((event, projection, binding))
}

fn successor_candidate_id(
    owner: ExecutionOwnerKey,
    ledger: &ExecutionGenerationLedger,
    predecessor: &ExecutionGeneration,
    request: &SuccessorRequest,
    worktree_binding_hash: &str,
) -> String {
    deterministic_generation_id(
        &serde_json::to_vec(&(
            owner,
            predecessor.identity.generation_id.as_str(),
            effective_generation_head_hash(ledger, predecessor),
            request,
            worktree_binding_hash,
        ))
        .unwrap_or_default(),
    )
}

fn build_successor_generation(
    owner: ExecutionOwnerKey,
    ledger: &ExecutionGenerationLedger,
    predecessor: &ExecutionGeneration,
    attempt: &ContinuationAttempt,
    worktree_binding_hash: &str,
) -> io::Result<(ExecutionGeneration, String)> {
    let predecessor_head = effective_generation_head_hash(ledger, predecessor);
    let predecessor_record = serde_json::from_str::<ExecutionControlRecord>(
        ledger.effective_projection_for(predecessor),
    )
    .map(hydrate_recovery_envelopes)
    .map_err(|error| {
        invalid_generation_data(format!(
            "predecessor execution snapshot is malformed: {error}"
        ))
    })?;
    let request = &attempt.request;
    let successor_record = ExecutionControlRecord {
        owner_kind: owner.kind,
        owner_number: owner.number,
        primary_session_id: request.initial_session_id.clone(),
        entrypoint: request.entrypoint.clone(),
        bundled_required_owners: predecessor_record.bundled_required_owners,
        status: ExecutionControlStatus::Active,
        blocked_reason: None,
        missing_verification: None,
        launched_at: request.requested_at,
        settled_at: None,
        transfers: Vec::new(),
        recoveries: Vec::new(),
        content_hash: String::new(),
    };
    let projection =
        String::from_utf8(serialize_execution_control(&successor_record)?).map_err(|error| {
            invalid_generation_data(format!(
                "serialized successor projection is not UTF-8: {error}"
            ))
        })?;
    let mut successor = ExecutionGeneration {
        identity: ExecutionGenerationIdentity {
            owner,
            generation_id: attempt.candidate_generation_id.clone(),
            predecessor_generation_id: Some(predecessor.identity.generation_id.clone()),
            predecessor_content_hash: Some(predecessor_head),
            session_binding_id: request.session_binding_id.clone(),
            initial_session_id: request.initial_session_id.clone(),
            worktree_binding_hash: worktree_binding_hash.to_string(),
            entrypoint: request.entrypoint.clone(),
            activated_at: request.requested_at,
        },
        status: ExecutionControlStatus::Active,
        execution_control_json: projection.clone(),
        content_hash: String::new(),
    };
    successor.content_hash = compute_generation_hash(&successor);
    Ok((successor, projection))
}

fn prepared_successor_generation(
    context: &GenerationTransactionContext,
    ledger: &ExecutionGenerationLedger,
    request: &SuccessorRequest,
) -> io::Result<Option<ExecutionGeneration>> {
    let Some(latest) = latest_operation_attempt(ledger, request, &context.worktree_binding_hash)?
    else {
        return Ok(None);
    };
    if latest.status != ContinuationAttemptStatus::Prepared {
        return Ok(None);
    }
    let predecessor = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    let predecessor_head = effective_generation_head_hash(ledger, predecessor);
    if predecessor.identity != latest.predecessor
        || predecessor_head != latest.predecessor_generation_content_hash
        || ledger.effective_status_for(predecessor)
            != successor_predecessor_execution_status(latest.predecessor_status)
    {
        return Err(generation_conflict(
            "prepared successor CAS lost: current generation or authorized terminal status changed",
        ));
    }
    build_successor_generation(
        context.owner,
        ledger,
        predecessor,
        latest,
        &context.worktree_binding_hash,
    )
    .map(|(generation, _)| Some(generation))
}

/// Compute the exact non-secret binding that a Prepared successor will carry
/// after its activation CAS. No current pointer or generation is advanced.
pub fn prepared_successor_execution_binding(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
) -> io::Result<gwt_agent::ExecutionBindingIdentity> {
    validate_successor_request(request)?;
    with_generation_owner_lease(worktree, owner, |context| {
        let ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                "owner generation ledger is not initialized",
            )
        })?;
        let successor =
            prepared_successor_generation(context, &ledger, request)?.ok_or_else(|| {
                io::Error::new(
                    ErrorKind::NotFound,
                    "Prepared successor attempt is missing or terminal",
                )
            })?;
        let mut planned = ledger;
        planned.current_generation_id = successor.identity.generation_id.clone();
        planned.generations.push(successor);
        let generation = planned.current_generation().ok_or_else(|| {
            invalid_generation_data("planned successor generation is not current")
        })?;
        Ok(execution_binding_for_generation(&planned, generation))
    })
}

/// Read the latest durable attempt for one continuation operation.
///
/// This is the response-loss reconciliation seam used by the Host
/// coordinator. The operation id is correlation only; repository + owner
/// scope remain mandatory authority inputs.
pub fn continuation_attempt_for_operation(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    operation_id: &str,
) -> io::Result<Option<ContinuationAttempt>> {
    if operation_id.trim() != operation_id
        || operation_id.is_empty()
        || operation_id.len() > 256
        || operation_id.chars().any(char::is_control)
    {
        return Err(invalid_generation_data(
            "continuation operation id must be canonical",
        ));
    }
    Ok(
        load_owner_generation_ledger(worktree, owner)?.and_then(|ledger| {
            ledger
                .continuation_attempts
                .iter()
                .rev()
                .find(|attempt| attempt.request.operation_id == operation_id)
                .cloned()
        }),
    )
}

pub fn record_rebound_continuation_validation(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    operation_id: &str,
    session_id: &str,
    binding: &gwt_agent::SessionExecutionBinding,
) -> io::Result<ExecutionContinuationValidationAudit> {
    if operation_id.trim() != operation_id
        || operation_id.is_empty()
        || operation_id.len() > 512
        || operation_id.chars().any(char::is_control)
        || session_id.trim().is_empty()
        || binding.session_id != session_id
        || binding.owner_kind != owner.kind.as_str()
        || binding.owner_number != owner.number
        || binding.capability_generation == 0
    {
        return Err(invalid_generation_data(
            "rebound continuation validation identity is invalid",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        let mut ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                "owner generation ledger is not initialized",
            )
        })?;
        if let Some(existing) = ledger
            .continuation_validations
            .iter()
            .find(|audit| audit.operation_id == operation_id)
        {
            if existing.session_id == session_id
                && existing.execution_binding == binding.identity
                && existing.capability_generation == binding.capability_generation
            {
                return Ok(existing.clone());
            }
            return Err(generation_conflict(
                "continuation operation id is already bound to another rebound validation",
            ));
        }
        if ledger
            .continuation_attempts
            .iter()
            .any(|attempt| attempt.request.operation_id == operation_id)
            || ledger
                .takeover_attempts
                .iter()
                .any(|attempt| attempt.request.operation_id == operation_id)
        {
            return Err(generation_conflict(
                "continuation operation id is already bound to another operation",
            ));
        }
        let current = ledger.current_generation().ok_or_else(|| {
            invalid_generation_data("execution generation ledger current id is missing")
        })?;
        let projection = serde_json::from_str::<ExecutionControlRecord>(
            ledger.effective_projection_for(current),
        )
        .map(hydrate_recovery_envelopes)
        .map_err(|error| {
            invalid_generation_data(format!(
                "current execution projection is malformed: {error}"
            ))
        })?;
        if ledger.effective_status_for(current) != ExecutionControlStatus::Active
            || projection.primary_session_id != session_id
            || execution_binding_for_generation(&ledger, current) != binding.identity
        {
            return Err(generation_conflict(
                "rebound continuation validation no longer matches current authority",
            ));
        }
        let projection_json = ledger.effective_projection_for(current).to_string();
        let audit = append_continuation_validation(
            &mut ledger,
            ExecutionContinuationValidationAudit {
                operation_id: operation_id.to_string(),
                session_id: session_id.to_string(),
                generation_id: binding.identity.generation_id.clone(),
                execution_binding: binding.identity.clone(),
                capability_generation: binding.capability_generation,
                recorded_at: Utc::now(),
                previous_audit_hash: String::new(),
                content_hash: String::new(),
            },
        );
        stamp_generation_ledger(&mut ledger);
        #[cfg(test)]
        fail_continuation_validation_write_if_requested()?;
        write_activated_generation(context, &ledger, &projection_json)?;
        let readback = load_owner_generation_ledger_from_context(context)?
            .and_then(|ledger| ledger.continuation_validations.last().cloned())
            .filter(|readback| readback == &audit)
            .ok_or_else(|| {
                invalid_generation_data("rebound continuation validation readback failed")
            })?;
        Ok(readback)
    })
}

pub fn continuation_validation_for_operation(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    operation_id: &str,
) -> io::Result<Option<ExecutionContinuationValidationAudit>> {
    if operation_id.trim() != operation_id
        || operation_id.is_empty()
        || operation_id.len() > 512
        || operation_id.chars().any(char::is_control)
    {
        return Err(invalid_generation_data(
            "continuation validation operation id must be canonical",
        ));
    }
    Ok(
        load_owner_generation_ledger(worktree, owner)?.and_then(|ledger| {
            ledger
                .continuation_validations
                .into_iter()
                .find(|audit| audit.operation_id == operation_id)
        }),
    )
}

/// Verify that a durable successor attempt identifies the exact non-secret
/// binding captured by a Host recovery receipt.
///
/// Unlike [`prepared_execution_binding_matches`], this recovery-only probe
/// also accepts an integrity-valid Aborted attempt. It reconstructs the
/// candidate from the immutable predecessor/request rather than treating an
/// operation id or the two public ids as sufficient cleanup authority.
pub fn continuation_attempt_execution_binding_matches(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    attempt: &ContinuationAttempt,
    expected_session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<bool> {
    let context = match GenerationTransactionContext::resolve(worktree, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(false),
        Err(error) => return Err(error),
    };
    let Some(ledger) = load_owner_generation_ledger_from_context(&context)? else {
        return Ok(false);
    };
    let Some(latest) =
        latest_operation_attempt(&ledger, &attempt.request, &context.worktree_binding_hash)?
    else {
        return Ok(false);
    };
    if latest != attempt || latest.request.initial_session_id != expected_session_id {
        return Ok(false);
    }
    let Some(predecessor) = ledger
        .generations
        .iter()
        .find(|generation| generation.identity == latest.predecessor)
    else {
        return Ok(false);
    };
    if effective_generation_head_hash(&ledger, predecessor)
        != latest.predecessor_generation_content_hash
        || ledger.effective_status_for(predecessor)
            != successor_predecessor_execution_status(latest.predecessor_status)
    {
        return Ok(false);
    }
    let (candidate, _) = build_successor_generation(
        owner,
        &ledger,
        predecessor,
        latest,
        &context.worktree_binding_hash,
    )?;
    Ok(execution_binding_for_generation(&ledger, &candidate) == *expected_identity)
}

/// Recover the one live Prepared fresh-launch attempt for a candidate
/// Session. This lets the Host rebuild its process-local readiness receipt
/// from the integrity-valid owner ledger without persisting a nonce or
/// trusting child-process input as authority.
pub fn prepared_fresh_linked_owner_launch_for_session(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    session_id: &str,
) -> io::Result<Option<ContinuationAttempt>> {
    if session_id.trim() != session_id
        || session_id.is_empty()
        || session_id.len() > 256
        || session_id.chars().any(char::is_control)
    {
        return Err(invalid_generation_data(
            "fresh launch Session id must be canonical",
        ));
    }
    let Some(ledger) = load_generation_ledger(worktree, owner)? else {
        return Ok(None);
    };
    let mut seen_operations = std::collections::HashSet::new();
    let mut candidates = ledger
        .continuation_attempts
        .iter()
        .rev()
        .filter(|attempt| seen_operations.insert(attempt.request.operation_id.as_str()))
        .filter(|attempt| {
            attempt.status == ContinuationAttemptStatus::Prepared
                && is_owner_launch_successor_attempt(attempt)
                && attempt.request.initial_session_id == session_id
        });
    let candidate = candidates.next().cloned();
    if candidates.next().is_some() {
        return Err(generation_conflict(
            "more than one Prepared fresh launch is bound to the same Session",
        ));
    }
    Ok(candidate)
}

/// Return the unique owner-launch successor still Prepared against the exact
/// current predecessor binding. Manual Launch Agent uses this read-only probe
/// to replay response loss instead of manufacturing a second operation.
pub fn prepared_owner_launch_successor_for_predecessor(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    expected: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<Option<ContinuationAttempt>> {
    let Some(ledger) = load_generation_ledger(worktree, owner)? else {
        return Ok(None);
    };
    let Some(current) = ledger.current_generation() else {
        return Ok(None);
    };
    if execution_binding_for_generation(&ledger, current) != *expected {
        return Ok(None);
    }
    let mut latest_operations = std::collections::HashSet::new();
    let mut candidates = ledger
        .continuation_attempts
        .iter()
        .rev()
        .filter(|attempt| latest_operations.insert(attempt.request.operation_id.as_str()))
        .filter(|attempt| {
            attempt.status == ContinuationAttemptStatus::Prepared
                && is_owner_launch_successor_attempt(attempt)
                && attempt.predecessor.generation_id == current.identity.generation_id
        });
    let candidate = candidates.next().cloned();
    if candidates.next().is_some() {
        return Err(generation_conflict(
            "more than one Prepared owner launch targets the current predecessor",
        ));
    }
    Ok(candidate)
}

/// Recover the one latest fresh linked-owner launch attempt for a candidate
/// Session, including terminal Aborted and Activated attempts.
///
/// Unlike [`prepared_fresh_linked_owner_launch_for_session`], this is the
/// process-restart reconciliation seam. The persisted Session id is only a
/// correlation key; the integrity-valid owner ledger and exact request remain
/// the authority. Reusing one Session id for multiple fresh operations is
/// refused as ambiguous instead of guessing which operation owns cleanup.
pub fn fresh_linked_owner_launch_for_session(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    session_id: &str,
) -> io::Result<Option<ContinuationAttempt>> {
    if session_id.trim() != session_id
        || session_id.is_empty()
        || session_id.len() > 256
        || session_id.chars().any(char::is_control)
    {
        return Err(invalid_generation_data(
            "fresh launch Session id must be canonical",
        ));
    }
    let Some(ledger) = load_owner_generation_ledger(worktree, owner)? else {
        return Ok(None);
    };
    let mut seen_operations = std::collections::HashSet::new();
    let mut candidates = ledger
        .continuation_attempts
        .iter()
        .rev()
        .filter(|attempt| seen_operations.insert(attempt.request.operation_id.as_str()))
        .filter(|attempt| {
            is_owner_launch_successor_attempt(attempt)
                && attempt.request.initial_session_id == session_id
        });
    let candidate = candidates.next().cloned();
    if candidates.next().is_some() {
        return Err(generation_conflict(
            "more than one fresh launch operation is bound to the same Session",
        ));
    }
    Ok(candidate)
}

/// Append a Prepared same-generation takeover without changing the current
/// owner projection.
pub fn prepare_generation_takeover(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &GenerationTakeoverRequest,
) -> io::Result<GenerationTakeoverAttempt> {
    validate_generation_takeover_request(request)?;
    with_generation_owner_lease(worktree, owner, |context| {
        let mut ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                "owner generation ledger is not initialized",
            )
        })?;
        if ledger
            .continuation_attempts
            .iter()
            .any(|attempt| attempt.request.operation_id == request.operation_id)
        {
            return Err(generation_conflict(
                "operation id is already bound to a successor generation",
            ));
        }
        if let Some(existing) =
            latest_generation_takeover_attempt(&ledger, request, &context.worktree_binding_hash)?
        {
            return Ok(existing.clone());
        }
        let current = ledger
            .current_generation()
            .ok_or_else(|| {
                invalid_generation_data("execution generation ledger current id is missing")
            })?
            .clone();
        let predecessor_head_hash = effective_generation_head_hash(&ledger, &current);
        // Validate the exact Active owner and deterministic post-takeover
        // projection before persisting a Prepared audit entry.
        let _ = build_generation_takeover(&ledger, request, &context.worktree_binding_hash)?;
        let attempt = append_generation_takeover_attempt(
            &mut ledger,
            GenerationTakeoverAttempt {
                request: request.clone(),
                worktree_binding_hash: context.worktree_binding_hash.clone(),
                generation_id: current.identity.generation_id,
                predecessor_head_hash,
                status: GenerationTakeoverAttemptStatus::Prepared,
                recorded_at: request.requested_at,
                resolution_reason: None,
                activated_binding: None,
                previous_attempt_hash: String::new(),
                content_hash: String::new(),
            },
        );
        stamp_generation_ledger(&mut ledger);
        write_owner_ledger(context, &ledger)?;
        Ok(attempt)
    })
}

/// Compute the exact current-generation binding after a Prepared takeover.
pub fn prepared_generation_takeover_execution_binding(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &GenerationTakeoverRequest,
) -> io::Result<gwt_agent::ExecutionBindingIdentity> {
    validate_generation_takeover_request(request)?;
    with_generation_owner_lease(worktree, owner, |context| {
        let ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                "owner generation ledger is not initialized",
            )
        })?;
        let latest =
            latest_generation_takeover_attempt(&ledger, request, &context.worktree_binding_hash)?
                .ok_or_else(|| {
                io::Error::new(ErrorKind::NotFound, "Prepared takeover attempt is missing")
            })?;
        if latest.status != GenerationTakeoverAttemptStatus::Prepared {
            return Err(generation_conflict(
                "generation takeover attempt is no longer Prepared",
            ));
        }
        let current = ledger.current_generation().ok_or_else(|| {
            invalid_generation_data("execution generation ledger current id is missing")
        })?;
        if current.identity.generation_id != latest.generation_id
            || effective_generation_head_hash(&ledger, current) != latest.predecessor_head_hash
        {
            return Err(generation_conflict(
                "Prepared takeover CAS lost: current generation/head changed",
            ));
        }
        build_generation_takeover(&ledger, request, &context.worktree_binding_hash)
            .map(|(_, _, binding)| binding)
    })
}

/// Append an Aborted event for a Prepared same-generation takeover.
pub fn abort_generation_takeover(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &GenerationTakeoverRequest,
    reason: &str,
) -> io::Result<GenerationTakeoverAttempt> {
    validate_generation_takeover_request(request)?;
    if reason.trim().is_empty() {
        return Err(invalid_generation_data(
            "aborting a generation takeover requires a non-empty reason",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        abort_generation_takeover_in_context(context, request, reason)
    })
}

fn abort_generation_takeover_in_context(
    context: &GenerationTransactionContext,
    request: &GenerationTakeoverRequest,
    reason: &str,
) -> io::Result<GenerationTakeoverAttempt> {
    let mut ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            "owner generation ledger is not initialized",
        )
    })?;
    let latest =
        latest_generation_takeover_attempt(&ledger, request, &context.worktree_binding_hash)?
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "takeover attempt is missing"))?
            .clone();
    match latest.status {
        GenerationTakeoverAttemptStatus::Aborted => {
            if latest.resolution_reason.as_deref() == Some(reason) {
                return Ok(latest);
            }
            return Err(generation_conflict(
                "takeover attempt was already aborted with a different reason",
            ));
        }
        GenerationTakeoverAttemptStatus::Activated => {
            return Err(generation_conflict(
                "an Activated generation takeover cannot be aborted",
            ));
        }
        GenerationTakeoverAttemptStatus::Prepared => {}
    }
    let mut aborted = latest;
    aborted.status = GenerationTakeoverAttemptStatus::Aborted;
    aborted.recorded_at = Utc::now();
    aborted.resolution_reason = Some(reason.to_string());
    aborted.activated_binding = None;
    aborted.previous_attempt_hash.clear();
    aborted.content_hash.clear();
    let aborted = append_generation_takeover_attempt(&mut ledger, aborted);
    stamp_generation_ledger(&mut ledger);
    write_owner_ledger(context, &ledger)?;
    Ok(aborted)
}

/// Abort one Prepared takeover and remove its exact Session while holding
/// leases in the canonical owner -> Session order.
#[allow(clippy::too_many_arguments)]
pub fn abort_generation_takeover_and_remove_exact_session<F>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &GenerationTakeoverRequest,
    reason: &str,
    sessions_dir: &Path,
    expected_session: &gwt_agent::SessionExecutionIdentity,
    after_abort: F,
) -> io::Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    validate_generation_takeover_request(request)?;
    if reason.trim().is_empty() {
        return Err(invalid_generation_data(
            "aborting a generation takeover requires a non-empty reason",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        gwt_agent::remove_session_if_execution_identity_matches_or_missing_with(
            sessions_dir,
            &expected_session.session_id,
            expected_session,
            || {
                abort_generation_takeover_in_context(context, request, reason)?;
                after_abort()
            },
        )
    })
}

/// Commit the same-generation ownership transfer CAS.
pub fn activate_generation_takeover(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &GenerationTakeoverRequest,
) -> io::Result<gwt_agent::ExecutionBindingIdentity> {
    validate_generation_takeover_request(request)?;
    with_generation_activation_leases(worktree, owner, |context| {
        activate_generation_takeover_in_context(context, request)
    })
}

fn activate_generation_takeover_in_context(
    context: &GenerationTransactionContext,
    request: &GenerationTakeoverRequest,
) -> io::Result<gwt_agent::ExecutionBindingIdentity> {
    let mut ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            "owner generation ledger is not initialized",
        )
    })?;
    let latest =
        latest_generation_takeover_attempt(&ledger, request, &context.worktree_binding_hash)?
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "takeover attempt is missing"))?
            .clone();
    match latest.status {
        GenerationTakeoverAttemptStatus::Activated => {
            validate_generation_activation_owner(context, true)?;
            let binding = latest.activated_binding.ok_or_else(|| {
                invalid_generation_data("Activated generation takeover has no committed binding")
            })?;
            let current = ledger.current_generation().ok_or_else(|| {
                invalid_generation_data("execution generation ledger current id is missing")
            })?;
            let projection = ledger.effective_projection_for(current).to_string();
            write_activated_generation(context, &ledger, &projection)?;
            return Ok(binding);
        }
        GenerationTakeoverAttemptStatus::Aborted => {
            return Err(generation_conflict(
                "an Aborted generation takeover cannot be activated",
            ));
        }
        GenerationTakeoverAttemptStatus::Prepared => {}
    }
    validate_generation_activation_owner(context, false)?;
    let current = ledger
        .current_generation()
        .ok_or_else(|| {
            invalid_generation_data("execution generation ledger current id is missing")
        })?
        .clone();
    if current.identity.generation_id != latest.generation_id
        || effective_generation_head_hash(&ledger, &current) != latest.predecessor_head_hash
    {
        return Err(generation_conflict(
            "generation takeover activation CAS lost: current generation/head changed",
        ));
    }
    let (event, projection, planned_binding) =
        build_generation_takeover(&ledger, request, &context.worktree_binding_hash)?;
    append_takeover_event(&mut ledger, event);
    let committed_binding = {
        let generation = ledger.current_generation().ok_or_else(|| {
            invalid_generation_data("takeover generation disappeared before commit")
        })?;
        execution_binding_for_generation(&ledger, generation)
    };
    if committed_binding != planned_binding {
        return Err(invalid_generation_data(
            "planned generation takeover binding changed before commit",
        ));
    }
    let mut activated = latest;
    activated.status = GenerationTakeoverAttemptStatus::Activated;
    activated.recorded_at = Utc::now();
    activated.resolution_reason = None;
    activated.activated_binding = Some(committed_binding.clone());
    activated.previous_attempt_hash.clear();
    activated.content_hash.clear();
    append_generation_takeover_attempt(&mut ledger, activated);
    stamp_generation_ledger(&mut ledger);
    write_activated_generation(context, &ledger, &projection)?;
    let readback = load_generation_ledger_from_context(context)?
        .ok_or_else(|| invalid_generation_data("takeover readback lost generation authority"))?;
    let readback_generation = readback
        .current_generation()
        .ok_or_else(|| invalid_generation_data("takeover readback lost current generation"))?;
    if execution_binding_for_generation(&readback, readback_generation) != committed_binding {
        return Err(invalid_generation_data(
            "takeover readback does not match committed binding",
        ));
    }
    Ok(committed_binding)
}

/// Run one Work transaction while the Prepared takeover owner and exact
/// candidate Session remain leased. The supplied activation callback commits
/// inside the already-held global -> owner -> Session lease hierarchy.
#[allow(clippy::too_many_arguments)]
pub fn with_prepared_generation_takeover_exact_session_activation<T>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &GenerationTakeoverRequest,
    sessions_dir: &Path,
    expected_session: &gwt_agent::SessionExecutionIdentity,
    operation: impl FnOnce(
        &mut dyn FnMut() -> io::Result<gwt_agent::ExecutionBindingIdentity>,
    ) -> io::Result<T>,
) -> io::Result<Option<T>> {
    validate_generation_takeover_request(request)?;
    if request.to_session_id != expected_session.session_id
        || expected_session.execution_binding.session_id != expected_session.session_id
        || expected_session.execution_binding.owner_kind != owner.kind.as_str()
        || expected_session.execution_binding.owner_number != owner.number
        || expected_session.execution_binding.capability_generation != 1
    {
        return Ok(None);
    }
    with_generation_activation_leases(worktree, owner, |context| {
        gwt_agent::with_session_lease(sessions_dir, &expected_session.session_id, |session| {
            if gwt_agent::SessionExecutionIdentity::from_session(session)
                .ok()
                .flatten()
                .as_ref()
                != Some(expected_session)
                || dunce::canonicalize(&session.worktree_path).ok().as_ref()
                    != Some(&context.worktree)
            {
                return Ok(None);
            }
            let ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
                io::Error::new(
                    ErrorKind::NotFound,
                    "owner generation ledger is not initialized",
                )
            })?;
            let latest = latest_generation_takeover_attempt(
                &ledger,
                request,
                &context.worktree_binding_hash,
            )?
            .ok_or_else(|| {
                io::Error::new(ErrorKind::NotFound, "Prepared takeover attempt is missing")
            })?;
            if latest.status != GenerationTakeoverAttemptStatus::Prepared {
                return Ok(None);
            }
            let planned =
                build_generation_takeover(&ledger, request, &context.worktree_binding_hash)
                    .map(|(_, _, binding)| binding)?;
            if planned != expected_session.execution_binding.identity {
                return Ok(None);
            }
            let mut activate = || activate_generation_takeover_in_context(context, request);
            operation(&mut activate).map(Some)
        })
    })
}

pub fn generation_takeover_attempt_for_operation(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    operation_id: &str,
) -> io::Result<Option<GenerationTakeoverAttempt>> {
    if operation_id.trim() != operation_id
        || operation_id.is_empty()
        || operation_id.len() > 256
        || operation_id.chars().any(char::is_control)
    {
        return Err(invalid_generation_data(
            "generation takeover operation id must be canonical",
        ));
    }
    Ok(
        load_owner_generation_ledger(worktree, owner)?.and_then(|ledger| {
            ledger
                .takeover_attempts
                .iter()
                .rev()
                .find(|attempt| attempt.request.operation_id == operation_id)
                .cloned()
        }),
    )
}

/// Verify that a durable same-generation takeover attempt identifies the
/// exact candidate binding captured by a Host recovery receipt.
///
/// Prepared and Aborted attempts reconstruct the deterministic post-takeover
/// head from their immutable predecessor/request. Activated attempts require
/// the committed audit event to reproduce the binding stored in the attempt.
/// In every status, generation and binding ids without the exact ledger head
/// are insufficient cleanup authority.
pub fn generation_takeover_attempt_execution_binding_matches(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    attempt: &GenerationTakeoverAttempt,
    expected_session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<bool> {
    let context = match GenerationTransactionContext::resolve(worktree, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(false),
        Err(error) => return Err(error),
    };
    let Some(ledger) = load_owner_generation_ledger_from_context(&context)? else {
        return Ok(false);
    };
    let Some(latest) = latest_generation_takeover_attempt(
        &ledger,
        &attempt.request,
        &context.worktree_binding_hash,
    )?
    else {
        return Ok(false);
    };
    if latest != attempt || latest.request.to_session_id != expected_session_id {
        return Ok(false);
    }
    let Some(generation) = ledger
        .generations
        .iter()
        .find(|generation| generation.identity.generation_id == latest.generation_id)
    else {
        return Ok(false);
    };

    let candidate = match latest.status {
        GenerationTakeoverAttemptStatus::Prepared | GenerationTakeoverAttemptStatus::Aborted => {
            if ledger.current_generation_id != latest.generation_id
                || effective_generation_head_hash(&ledger, generation)
                    != latest.predecessor_head_hash
            {
                return Ok(false);
            }
            let (_, _, candidate) = build_generation_takeover(
                &ledger,
                &latest.request,
                &context.worktree_binding_hash,
            )?;
            candidate
        }
        GenerationTakeoverAttemptStatus::Activated => {
            let Some(committed) = latest.activated_binding.as_ref() else {
                return Ok(false);
            };
            let reconstructed = execution_binding_for_generation(&ledger, generation);
            if reconstructed != *committed {
                return Ok(false);
            }
            reconstructed
        }
    };
    Ok(candidate == *expected_identity)
}

/// Side-effect-free Host probe for a still-Prepared exact execution binding.
pub fn prepared_execution_binding_matches(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    expected_session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
) -> io::Result<bool> {
    let context = match GenerationTransactionContext::resolve(worktree, owner) {
        Ok(context) => context,
        Err(error) if error.kind() == ErrorKind::InvalidInput => return Ok(false),
        Err(error) => return Err(error),
    };
    let Some(ledger) = load_owner_generation_ledger_from_context(&context)? else {
        return Ok(false);
    };
    if let Some(attempt) = ledger.continuation_attempts.iter().rev().find(|attempt| {
        attempt.status == ContinuationAttemptStatus::Prepared
            && attempt.request.initial_session_id == expected_session_id
    }) {
        let Some(successor) = prepared_successor_generation(&context, &ledger, &attempt.request)?
        else {
            return Ok(false);
        };
        let mut planned = ledger;
        planned.current_generation_id = successor.identity.generation_id.clone();
        planned.generations.push(successor);
        let generation = planned.current_generation().ok_or_else(|| {
            invalid_generation_data("planned successor generation is not current")
        })?;
        return Ok(execution_binding_for_generation(&planned, generation) == *expected_identity);
    }
    let Some(attempt) = ledger.takeover_attempts.iter().rev().find(|attempt| {
        attempt.status == GenerationTakeoverAttemptStatus::Prepared
            && attempt.request.to_session_id == expected_session_id
    }) else {
        return Ok(false);
    };
    let current = ledger.current_generation().ok_or_else(|| {
        invalid_generation_data("execution generation ledger current id is missing")
    })?;
    if current.identity.generation_id != attempt.generation_id
        || effective_generation_head_hash(&ledger, current) != attempt.predecessor_head_hash
    {
        return Ok(false);
    }
    let (_, _, planned) =
        build_generation_takeover(&ledger, &attempt.request, &context.worktree_binding_hash)?;
    Ok(planned == *expected_identity)
}

/// Append a Prepared attempt without changing the current generation.
pub fn prepare_successor(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
) -> io::Result<ContinuationAttempt> {
    if request.source == FRESH_LINKED_OWNER_LAUNCH_SOURCE {
        return Err(invalid_generation_data(
            "fresh linked-owner launch successors require the Blocked-specific prepare operation",
        ));
    }
    prepare_successor_for_status(
        worktree,
        owner,
        request,
        SuccessorPredecessorStatus::Completed,
    )
}

/// Prepare one new generation that deliberately fences a currently Active
/// generation owned by another Session.
pub fn prepare_active_continuation_successor(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
) -> io::Result<ContinuationAttempt> {
    if !matches!(
        request.source.as_str(),
        "execution-continue" | "continue-work:resume" | "continue-work:handoff"
    ) {
        return Err(invalid_generation_data(
            "active continuation successors require a canonical continuation source",
        ));
    }
    prepare_successor_for_status(worktree, owner, request, SuccessorPredecessorStatus::Active)
}

/// Prepare a fresh execution lifetime from an integrity-valid terminal
/// Blocked generation. This is intentionally separate from Continue work and
/// cannot be used for Completed predecessors or same-lifetime recovery.
pub fn prepare_fresh_linked_owner_launch_successor(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
) -> io::Result<ContinuationAttempt> {
    if request.source != FRESH_LINKED_OWNER_LAUNCH_SOURCE || request.work_id.is_some() {
        return Err(invalid_generation_data(
            "fresh linked-owner launch successor must use the canonical source without a Continue Work identity",
        ));
    }
    prepare_successor_for_status(
        worktree,
        owner,
        request,
        SuccessorPredecessorStatus::Blocked,
    )
}

/// Prepare the exact generation classified by the manual Launch Agent
/// preflight. Completed and Blocked predecessors retain their existing
/// successor semantics; Active is accepted only through the exact terminal
/// Session proof transaction below.
#[derive(Debug, Clone, Copy)]
pub struct ExactManualLaunchPredecessor<'a> {
    pub sessions_dir: &'a Path,
    pub session: Option<&'a gwt_agent::SessionExecutionIdentity>,
    pub runtime: Option<gwt_agent::ManualLaunchRuntimeProof>,
    pub binding: &'a gwt_agent::ExecutionBindingIdentity,
    pub status: SuccessorPredecessorStatus,
    pub terminal_reason: &'a str,
}

pub fn prepare_exact_manual_launch_successor(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
    predecessor: ExactManualLaunchPredecessor<'_>,
) -> io::Result<ContinuationAttempt> {
    let ExactManualLaunchPredecessor {
        sessions_dir,
        session: expected_session,
        runtime: expected_runtime,
        binding: expected_binding,
        status: predecessor_status,
        terminal_reason,
    } = predecessor;
    if predecessor_status == SuccessorPredecessorStatus::Active {
        let expected_session = expected_session.ok_or_else(|| {
            invalid_generation_data("Active manual successor requires an exact Session identity")
        })?;
        let expected_runtime = expected_runtime.ok_or_else(|| {
            invalid_generation_data("Active manual successor requires exact runtime proof")
        })?;
        if &expected_session.execution_binding.identity != expected_binding {
            return Err(generation_conflict(
                "Active manual successor binding differs from its exact Session identity",
            ));
        }
        return prepare_exact_terminal_active_successor(
            worktree,
            owner,
            request,
            sessions_dir,
            expected_session,
            expected_runtime,
            terminal_reason,
        );
    }
    validate_successor_request(request)?;
    let request_is_canonical = match predecessor_status {
        SuccessorPredecessorStatus::Blocked => {
            request.source == FRESH_LINKED_OWNER_LAUNCH_SOURCE && request.work_id.is_none()
        }
        SuccessorPredecessorStatus::Completed => {
            request.source == MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE && request.work_id.is_none()
        }
        SuccessorPredecessorStatus::Active => unreachable!(),
    };
    if !request_is_canonical {
        return Err(invalid_generation_data(
            "manual owner launch successor source does not match its predecessor status",
        ));
    }
    if expected_session.is_some_and(|session| request.initial_session_id == session.session_id) {
        return Err(invalid_generation_data(
            "successor Session must differ from the predecessor Session",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        let mut ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                "owner generation ledger is not initialized",
            )
        })?;
        let current = ledger
            .current_generation()
            .ok_or_else(|| {
                invalid_generation_data("execution generation ledger current id is missing")
            })?
            .clone();
        let projection = serde_json::from_str::<ExecutionControlRecord>(
            ledger.effective_projection_for(&current),
        )
        .map(hydrate_recovery_envelopes)
        .map_err(|error| {
            invalid_generation_data(format!(
                "manual predecessor projection is malformed: {error}"
            ))
        })?;
        gwt_agent::with_session_path_lease(
            sessions_dir,
            &projection.primary_session_id,
            |session_state| {
                let manual_handoff = match session_state {
                    gwt_agent::SessionPathState::Present(session) => {
                        if !durable_session_binding_authorizes_current_generation(
                            context,
                            &ledger,
                            &projection.primary_session_id,
                            &session,
                        )? {
                            return Err(generation_conflict(
                                "manual predecessor Session no longer authorizes the current generation",
                            ));
                        }
                        let exact_session = gwt_agent::SessionExecutionIdentity::from_session(
                            &session,
                        )
                        .map_err(invalid_generation_data)?
                        .ok_or_else(|| {
                            generation_conflict(
                                "manual predecessor Session no longer carries exact execution identity",
                            )
                        })?;
                        if reconcile_active_launch_handshake_under_lease(
                            sessions_dir,
                            &exact_session,
                        )? {
                            return Err(io::Error::new(
                                ErrorKind::PermissionDenied,
                                "manual terminal predecessor is fenced by an in-flight Session authority transition",
                            ));
                        }
                        gwt_agent::read_session_manual_handoff_under_lease(
                            sessions_dir,
                            &exact_session,
                        )?
                    }
                    gwt_agent::SessionPathState::Missing => None,
                    gwt_agent::SessionPathState::Error(error) => return Err(error),
                };
                if let Some(replay) =
                    latest_operation_attempt(&ledger, request, &context.worktree_binding_hash)?
                        .cloned()
                {
                    if replay.predecessor_status != predecessor_status
                        || replay.predecessor.generation_id != expected_binding.generation_id
                        || replay.predecessor_generation_content_hash
                            != expected_binding.ledger_head_hash
                    {
                        return Err(generation_conflict(
                            "manual successor replay no longer targets the exact predecessor",
                        ));
                    }
                    if let Some(handoff) = manual_handoff.as_ref() {
                        let settlement_operation_id =
                            format!("{}:terminalize-predecessor", request.operation_id);
                        let exact_post_stop_replay = predecessor_status
                            == SuccessorPredecessorStatus::Blocked
                            && ledger.lifecycle_events.iter().rev().any(|event| {
                                event.generation_id == current.identity.generation_id
                                    && event.from_status == ExecutionControlStatus::Active
                                    && event.to_status == ExecutionControlStatus::Blocked
                                    && event.session_id == projection.primary_session_id
                                    && event.operation_id.as_deref()
                                        == Some(settlement_operation_id.as_str())
                            });
                        if !exact_post_stop_replay {
                            return Err(io::Error::new(
                                ErrorKind::PermissionDenied,
                                "manual terminal predecessor is fenced by an unrelated Session authority transition",
                            ));
                        }
                        if !gwt_agent::clear_session_manual_handoff_under_lease(
                            sessions_dir,
                            handoff,
                        )? {
                            return Err(io::Error::other(
                                "manual successor replay lost its exact durable handoff fence",
                            ));
                        }
                    }
                    return Ok(replay);
                }
                if manual_handoff.is_some() {
                    return Err(io::Error::new(
                        ErrorKind::PermissionDenied,
                        "manual terminal predecessor is fenced by an in-flight Session authority transition",
                    ));
                }
                if execution_binding_for_generation(&ledger, &current) != *expected_binding {
                    return Err(generation_conflict(
                        "manual predecessor no longer owns the exact current generation binding",
                    ));
                }
                if ledger.effective_status_for(&current)
                    != successor_predecessor_execution_status(predecessor_status)
                {
                    return Err(generation_conflict(
                        "manual predecessor status changed before successor preparation",
                    ));
                }
                if ledger.continuation_attempts.iter().any(|attempt| {
                    attempt.predecessor.generation_id == current.identity.generation_id
                        && attempt.status == ContinuationAttemptStatus::Prepared
                }) || ledger.takeover_attempts.iter().any(|attempt| {
                    attempt.generation_id == current.identity.generation_id
                        && attempt.status == GenerationTakeoverAttemptStatus::Prepared
                }) {
                    return Err(generation_conflict(
                        "manual successor refuses while a Prepared successor or takeover targets the current generation",
                    ));
                }
                let attempt =
                    plan_successor_for_status(context, &mut ledger, request, predecessor_status)?;
                stamp_generation_ledger(&mut ledger);
                write_owner_ledger(context, &ledger)?;
                Ok(attempt)
            },
        )
    })
}

/// Prepare a fresh successor after proving that the exact current producing
/// Session is durably terminal.
///
/// This is the only Active-predecessor adapter for a fresh linked-owner
/// launch. It revalidates the owner generation and the complete persisted
/// Session incarnation while holding the canonical owner -> Session leases,
/// records the audited Active -> Blocked transition, and appends the one
/// Prepared successor before releasing those leases. A live, changed, or
/// malformed Session therefore fails before any owner bytes are written.
pub fn prepare_exact_terminal_active_successor(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
    sessions_dir: &Path,
    expected_session: &gwt_agent::SessionExecutionIdentity,
    expected_runtime: gwt_agent::ManualLaunchRuntimeProof,
    reason: &str,
) -> io::Result<ContinuationAttempt> {
    validate_successor_request(request)?;
    if request.source != FRESH_LINKED_OWNER_LAUNCH_SOURCE || request.work_id.is_some() {
        return Err(invalid_generation_data(
            "exact-terminal fresh successor must use the canonical source without a Continue Work identity",
        ));
    }
    if reason.trim().is_empty() {
        return Err(invalid_generation_data(
            "exact-terminal fresh successor requires a non-empty settlement reason",
        ));
    }
    if request.initial_session_id == expected_session.session_id {
        return Err(invalid_generation_data(
            "successor Session must differ from the terminal predecessor Session",
        ));
    }
    if expected_session.execution_binding.session_id != expected_session.session_id
        || expected_session.execution_binding.owner_kind != owner.kind.as_str()
        || expected_session.execution_binding.owner_number != owner.number
        || expected_session.execution_binding.capability_generation == 0
    {
        return Err(generation_conflict(
            "terminal predecessor Session identity is not canonical for the requested owner",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        gwt_agent::with_session_lease(sessions_dir, &expected_session.session_id, |session| {
            if gwt_agent::SessionExecutionIdentity::from_session(session)
                .ok()
                .flatten()
                .as_ref()
                != Some(expected_session)
                || dunce::canonicalize(&session.worktree_path).ok().as_ref()
                    != Some(&context.worktree)
            {
                return Err(generation_conflict(
                    "terminal predecessor Session changed before successor preparation",
                ));
            }
            if expected_runtime.host_pid == 0 || expected_runtime.runtime_incarnation == 0 {
                return Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "terminal predecessor runtime proof is invalid",
                ));
            }
            let runtime_path = gwt_agent::runtime_state_path_for_pid(
                sessions_dir,
                expected_runtime.host_pid,
                &expected_session.session_id,
            );
            let runtime = gwt_agent::SessionRuntimeState::load(&runtime_path).map_err(|error| {
                io::Error::new(
                    ErrorKind::PermissionDenied,
                    format!("terminal predecessor runtime sidecar is unavailable: {error}"),
                )
            })?;
            if runtime.execution_identity.as_ref() != Some(expected_session)
                || runtime.runtime_incarnation != Some(expected_runtime.runtime_incarnation)
            {
                return Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "terminal predecessor runtime proof changed",
                ));
            }
            let runtime_is_terminal = matches!(
                runtime.status,
                gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
            );
            if runtime_is_terminal {
                match (runtime.child_pid, runtime.child_started_at) {
                    (Some(child_pid), Some(child_started_at))
                        if child_pid > 0 && child_started_at > 0 =>
                    {
                        if crate::process::exact_pty_process_tree_is_alive(
                            child_pid,
                            child_started_at,
                        ) {
                            return Err(io::Error::new(
                                ErrorKind::PermissionDenied,
                                "terminal predecessor process tree is still live",
                            ));
                        }
                    }
                    (None, None) => {
                        return Err(io::Error::new(
                            ErrorKind::PermissionDenied,
                            "terminal predecessor process identity is missing",
                        ));
                    }
                    _ => {
                        return Err(io::Error::new(
                            ErrorKind::PermissionDenied,
                            "terminal predecessor process identity is incomplete",
                        ));
                    }
                }
            }
            let session_is_terminal = matches!(
                session.status,
                gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
            );
            if reconcile_active_launch_handshake_under_lease(sessions_dir, expected_session)? {
                return Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "terminal predecessor still has an in-flight Active launch handshake",
                ));
            }
            if exact_session_runtime_fences_active_launch(sessions_dir, expected_session)? {
                return Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "terminal predecessor still has an exact live runtime",
                ));
            }
            let manual_handoff =
                gwt_agent::read_session_manual_handoff_under_lease(sessions_dir, expected_session)?;
            let abandoned_manual_handoff = if runtime_is_terminal && session_is_terminal {
                false
            } else {
                let handoff = manual_handoff.as_ref().ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::PermissionDenied,
                        "nonterminal predecessor has no exact durable manual handoff fence",
                    )
                })?;
                let host_started_at = runtime.host_started_at.filter(|value| *value > 0);
                let child = runtime.child_pid.zip(runtime.child_started_at).filter(
                    |(child_pid, child_started_at)| *child_pid > 0 && *child_started_at > 0,
                );
                if handoff.execution_identity != *expected_session
                    || handoff.host_pid != expected_runtime.host_pid
                    || Some(handoff.host_started_at) != host_started_at
                    || host_started_at.is_some_and(|started_at| {
                        crate::process::host_process_start_time(expected_runtime.host_pid)
                            == Some(started_at)
                    })
                    || child.is_none_or(|(child_pid, child_started_at)| {
                        crate::process::exact_pty_process_tree_is_alive(child_pid, child_started_at)
                    })
                {
                    return Err(io::Error::new(
                        ErrorKind::PermissionDenied,
                        "manual handoff Host or child is still live or lacks exact exit evidence",
                    ));
                }
                true
            };

            let mut ledger =
                load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::NotFound,
                        "owner generation ledger is not initialized",
                    )
                })?;
            let current = ledger
                .current_generation()
                .ok_or_else(|| {
                    invalid_generation_data("execution generation ledger current id is missing")
                })?
                .clone();
            if ledger.effective_status_for(&current) == ExecutionControlStatus::Blocked {
                let replay = ledger
                        .continuation_attempts
                        .iter()
                        .rev()
                        .find(|attempt| {
                            attempt.request.operation_id == request.operation_id
                                && attempt.worktree_binding_hash == context.worktree_binding_hash
                        })
                        .cloned()
                    .ok_or_else(|| {
                        generation_conflict(
                            "terminal predecessor was Blocked without the requested successor attempt",
                        )
                    })?;
                if replay.predecessor.generation_id != current.identity.generation_id
                    || replay.predecessor_status != SuccessorPredecessorStatus::Blocked
                {
                    return Err(generation_conflict(
                            "terminal predecessor successor replay no longer targets the exact generation",
                        ));
                }
                let attempt = plan_successor_for_status(
                    context,
                    &mut ledger,
                    request,
                    SuccessorPredecessorStatus::Blocked,
                )?;
                if let Some(handoff) = manual_handoff.as_ref() {
                    if !gwt_agent::clear_session_manual_handoff_under_lease(sessions_dir, handoff)?
                    {
                        return Err(io::Error::other(
                            "Prepared successor replay lost its exact manual handoff fence",
                        ));
                    }
                }
                return Ok(attempt);
            }
            if execution_binding_for_generation(&ledger, &current)
                != expected_session.execution_binding.identity
            {
                return Err(generation_conflict(
                    "terminal predecessor no longer owns the exact current generation binding",
                ));
            }
            if ledger.effective_status_for(&current) != ExecutionControlStatus::Active {
                return Err(generation_conflict(
                    "exact-terminal successor requires a current Active generation",
                ));
            }
            if ledger.continuation_attempts.iter().any(|attempt| {
                attempt.predecessor.generation_id == current.identity.generation_id
                    && attempt.status == ContinuationAttemptStatus::Prepared
            }) || ledger.takeover_attempts.iter().any(|attempt| {
                attempt.generation_id == current.identity.generation_id
                    && attempt.status == GenerationTakeoverAttemptStatus::Prepared
            }) {
                return Err(generation_conflict(
                        "exact-terminal successor refuses while a Prepared successor or takeover targets the current generation",
                    ));
            }

            let mut record = serde_json::from_str::<ExecutionControlRecord>(
                ledger.effective_projection_for(&current),
            )
            .map(hydrate_recovery_envelopes)
            .map_err(|error| {
                invalid_generation_data(format!(
                    "Active terminal predecessor projection is malformed: {error}"
                ))
            })?;
            if !integrity_ok(&record)
                || record.owner_kind != owner.kind
                || record.owner_number != owner.number
                || record.primary_session_id != expected_session.session_id
                || record.status != ExecutionControlStatus::Active
                || record.settled_at.is_some()
            {
                return Err(generation_conflict(
                    "terminal predecessor lost exact Active projection authority",
                ));
            }

            if abandoned_manual_handoff
                && !gwt_agent::persist_session_terminal_status_for_exact_runtime_under_lease(
                    sessions_dir,
                    expected_session,
                    expected_runtime,
                    gwt_agent::AgentStatus::Interrupted,
                )?
            {
                return Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "abandoned manual handoff lost its exact runtime evidence",
                ));
            }

            let recorded_at = Utc::now();
            record.status = ExecutionControlStatus::Blocked;
            record.blocked_reason = Some(reason.to_string());
            record.missing_verification = Some("authenticated SessionStart readiness".to_string());
            record.settled_at = Some(recorded_at);
            let projection = serialized_execution_projection(&record)?;
            append_lifecycle_event(
                &mut ledger,
                GenerationLifecycleEvent {
                    sequence: 0,
                    generation_id: current.identity.generation_id,
                    from_status: ExecutionControlStatus::Active,
                    to_status: ExecutionControlStatus::Blocked,
                    session_id: expected_session.session_id.clone(),
                    reason: reason.to_string(),
                    operation_id: Some(format!("{}:terminalize-predecessor", request.operation_id)),
                    recorded_at,
                    execution_control_json: projection.clone(),
                    previous_event_hash: String::new(),
                    content_hash: String::new(),
                },
            );
            let attempt = plan_successor_for_status(
                context,
                &mut ledger,
                request,
                SuccessorPredecessorStatus::Blocked,
            )?;
            stamp_generation_ledger(&mut ledger);
            write_activated_generation(context, &ledger, &projection)?;
            if let Some(handoff) = manual_handoff.as_ref() {
                if !gwt_agent::clear_session_manual_handoff_under_lease(sessions_dir, handoff)? {
                    return Err(io::Error::other(
                        "Prepared successor committed but its exact manual handoff fence could not be cleared",
                    ));
                }
            }
            Ok(attempt)
        })
    })
}

fn successor_predecessor_execution_status(
    status: SuccessorPredecessorStatus,
) -> ExecutionControlStatus {
    match status {
        SuccessorPredecessorStatus::Active => ExecutionControlStatus::Active,
        SuccessorPredecessorStatus::Completed => ExecutionControlStatus::Completed,
        SuccessorPredecessorStatus::Blocked => ExecutionControlStatus::Blocked,
    }
}

fn prepare_successor_for_status(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
    predecessor_status: SuccessorPredecessorStatus,
) -> io::Result<ContinuationAttempt> {
    validate_successor_request(request)?;
    with_generation_owner_lease(worktree, owner, |context| {
        let mut ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                "owner generation ledger is not initialized",
            )
        })?;
        let attempt = plan_successor_for_status(context, &mut ledger, request, predecessor_status)?;
        stamp_generation_ledger(&mut ledger);
        write_owner_ledger(context, &ledger)?;
        Ok(attempt)
    })
}

fn plan_successor_for_status(
    context: &GenerationTransactionContext,
    ledger: &mut ExecutionGenerationLedger,
    request: &SuccessorRequest,
    predecessor_status: SuccessorPredecessorStatus,
) -> io::Result<ContinuationAttempt> {
    if ledger
        .takeover_attempts
        .iter()
        .any(|attempt| attempt.request.operation_id == request.operation_id)
    {
        return Err(generation_conflict(
            "operation id is already bound to a same-generation takeover",
        ));
    }
    if let Some(existing) =
        latest_operation_attempt(ledger, request, &context.worktree_binding_hash)?
    {
        return Ok(existing.clone());
    }
    let predecessor = ledger
        .current_generation()
        .ok_or_else(|| {
            invalid_generation_data("execution generation ledger current id is missing")
        })?
        .clone();
    let expected_status = successor_predecessor_execution_status(predecessor_status);
    if ledger.effective_status_for(&predecessor) != expected_status {
        return Err(generation_conflict(match predecessor_status {
            SuccessorPredecessorStatus::Active => {
                "an active continuation successor can only be prepared from an Active generation"
            }
            SuccessorPredecessorStatus::Completed => {
                "a Continue work successor can only be prepared from a Completed generation; Blocked requires an explicit fresh linked-owner launch"
            }
            SuccessorPredecessorStatus::Blocked => {
                "a fresh linked-owner launch successor can only be prepared from a Blocked generation"
            }
        }));
    }
    let predecessor_head = effective_generation_head_hash(ledger, &predecessor);
    let attempt = ContinuationAttempt {
        request: request.clone(),
        predecessor_status,
        worktree_binding_hash: context.worktree_binding_hash.clone(),
        predecessor: predecessor.identity.clone(),
        predecessor_generation_content_hash: predecessor_head,
        candidate_generation_id: successor_candidate_id(
            context.owner,
            ledger,
            &predecessor,
            request,
            &context.worktree_binding_hash,
        ),
        status: ContinuationAttemptStatus::Prepared,
        recorded_at: request.requested_at,
        reason: None,
        activated_generation: None,
        previous_attempt_hash: String::new(),
        content_hash: String::new(),
    };
    Ok(append_continuation_attempt(ledger, attempt))
}

/// Append an Aborted event for a Prepared attempt. Current generation and
/// generation count remain unchanged.
pub fn abort_successor(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
    reason: &str,
) -> io::Result<ContinuationAttempt> {
    validate_successor_request(request)?;
    if reason.trim().is_empty() {
        return Err(invalid_generation_data(
            "aborting a successor requires a non-empty reason",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        abort_successor_in_context(context, request, reason)
    })
}

fn abort_successor_in_context(
    context: &GenerationTransactionContext,
    request: &SuccessorRequest,
    reason: &str,
) -> io::Result<ContinuationAttempt> {
    let mut ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            "owner generation ledger is not initialized",
        )
    })?;
    let latest = latest_operation_attempt(&ledger, request, &context.worktree_binding_hash)?
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "successor attempt is missing"))?
        .clone();
    match latest.status {
        ContinuationAttemptStatus::Aborted => {
            if latest.reason.as_deref() == Some(reason) {
                return Ok(latest);
            }
            return Err(generation_conflict(
                "successor attempt was already aborted with a different reason",
            ));
        }
        ContinuationAttemptStatus::Activated => {
            return Err(generation_conflict(
                "an Activated successor cannot be aborted",
            ));
        }
        ContinuationAttemptStatus::Prepared => {}
    }
    let mut aborted = latest;
    aborted.status = ContinuationAttemptStatus::Aborted;
    aborted.recorded_at = Utc::now();
    aborted.reason = Some(reason.to_string());
    aborted.previous_attempt_hash.clear();
    aborted.content_hash.clear();
    let aborted = append_continuation_attempt(&mut ledger, aborted);
    stamp_generation_ledger(&mut ledger);
    write_owner_ledger(context, &ledger)?;
    Ok(aborted)
}

/// Abort one Prepared successor and remove its exact Session while holding
/// leases in the canonical owner -> Session order.
#[allow(clippy::too_many_arguments)]
pub fn abort_successor_and_remove_exact_session<F>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
    reason: &str,
    sessions_dir: &Path,
    expected_session: &gwt_agent::SessionExecutionIdentity,
    after_abort: F,
) -> io::Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    validate_successor_request(request)?;
    if reason.trim().is_empty() {
        return Err(invalid_generation_data(
            "aborting a successor requires a non-empty reason",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        gwt_agent::remove_session_if_execution_identity_matches_or_missing_with(
            sessions_dir,
            &expected_session.session_id,
            expected_session,
            || {
                abort_successor_in_context(context, request, reason)?;
                after_abort()
            },
        )
    })
}

/// Abort one Prepared successor only while its candidate Session remains
/// genuinely absent. The owner lease and Session lease are held together so
/// a same-id Session cannot materialize between the absence check and commit.
#[allow(clippy::too_many_arguments)]
pub fn abort_successor_if_session_missing_with<F>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
    reason: &str,
    sessions_dir: &Path,
    session_id: &str,
    after_abort: F,
) -> io::Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    validate_successor_request(request)?;
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|error| invalid_generation_data(format!("invalid Session id: {error}")))?;
    if request.initial_session_id != session_id {
        return Ok(false);
    }
    if reason.trim().is_empty() {
        return Err(invalid_generation_data(
            "aborting a successor requires a non-empty reason",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        gwt_agent::with_session_path_lease(sessions_dir, session_id, |state| match state {
            gwt_agent::SessionPathState::Missing => {
                abort_successor_in_context(context, request, reason)?;
                after_abort()?;
                Ok(true)
            }
            gwt_agent::SessionPathState::Present(_) => Ok(false),
            gwt_agent::SessionPathState::Error(error) => Err(error),
        })
    })
}

/// Commit the successor CAS. This is the only operation in this module that
/// appends a successor generation and advances `current_generation_id`.
pub fn activate_successor(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
) -> io::Result<ExecutionGenerationIdentity> {
    validate_successor_request(request)?;
    with_generation_activation_leases(worktree, owner, |context| {
        activate_successor_in_context(context, request)
    })
}

fn activate_successor_in_context(
    context: &GenerationTransactionContext,
    request: &SuccessorRequest,
) -> io::Result<ExecutionGenerationIdentity> {
    let ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            "owner generation ledger is not initialized",
        )
    })?;
    activate_successor_from_ledger_in_context(context, request, ledger)
}

fn activate_successor_from_ledger_in_context(
    context: &GenerationTransactionContext,
    request: &SuccessorRequest,
    mut ledger: ExecutionGenerationLedger,
) -> io::Result<ExecutionGenerationIdentity> {
    let latest = latest_operation_attempt(&ledger, request, &context.worktree_binding_hash)?
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "successor attempt is missing"))?
        .clone();
    match latest.status {
        ContinuationAttemptStatus::Activated => {
            validate_generation_activation_owner(context, true)?;
            let identity = latest.activated_generation.ok_or_else(|| {
                invalid_generation_data("Activated successor attempt does not name its generation")
            })?;
            if identity.worktree_binding_hash != context.worktree_binding_hash {
                return Err(generation_conflict(
                    "Activated successor is bound to a different worktree",
                ));
            }
            if ledger.current_generation_id == identity.generation_id {
                let generation = ledger
                    .generations
                    .iter()
                    .find(|generation| generation.identity == identity)
                    .ok_or_else(|| {
                        invalid_generation_data(
                            "Activated successor generation is missing from the ledger",
                        )
                    })?;
                let projection = ledger.effective_projection_for(generation).to_string();
                // A response-loss retry is also the repair operation for
                // ledger-first partial commits. Re-publish the exact
                // committed projection/pointer before returning success.
                write_activated_generation(context, &ledger, &projection)?;
            }
            let readback = load_generation_ledger_from_context(context)?.ok_or_else(|| {
                invalid_generation_data("Activated successor readback lost generation authority")
            })?;
            if !readback
                .generations
                .iter()
                .any(|generation| generation.identity == identity)
            {
                return Err(invalid_generation_data(
                    "Activated successor readback does not contain the committed generation",
                ));
            }
            return Ok(identity);
        }
        ContinuationAttemptStatus::Aborted => {
            return Err(generation_conflict(
                "an Aborted successor cannot be activated",
            ));
        }
        ContinuationAttemptStatus::Prepared => {}
    }
    validate_generation_activation_owner(context, false)?;

    let predecessor = ledger
        .current_generation()
        .ok_or_else(|| {
            invalid_generation_data("execution generation ledger current id is missing")
        })?
        .clone();
    let predecessor_head = effective_generation_head_hash(&ledger, &predecessor);
    if predecessor.identity != latest.predecessor
        || predecessor_head != latest.predecessor_generation_content_hash
        || ledger.effective_status_for(&predecessor)
            != successor_predecessor_execution_status(latest.predecessor_status)
    {
        return Err(generation_conflict(
                "successor activation CAS lost: current generation or authorized terminal status changed",
            ));
    }

    let (successor, projection) = build_successor_generation(
        context.owner,
        &ledger,
        &predecessor,
        &latest,
        &context.worktree_binding_hash,
    )?;
    let identity = successor.identity.clone();
    ledger.current_generation_id = identity.generation_id.clone();
    ledger.generations.push(successor);

    let mut activated = latest;
    activated.status = ContinuationAttemptStatus::Activated;
    activated.recorded_at = Utc::now();
    activated.activated_generation = Some(identity.clone());
    activated.reason = None;
    activated.previous_attempt_hash.clear();
    activated.content_hash.clear();
    append_continuation_attempt(&mut ledger, activated);
    stamp_generation_ledger(&mut ledger);
    write_activated_generation(context, &ledger, &projection)?;
    let readback = load_generation_ledger_from_context(context)?.ok_or_else(|| {
        invalid_generation_data("successor activation readback lost generation authority")
    })?;
    if readback
        .current_generation()
        .map(|generation| &generation.identity)
        != Some(&identity)
    {
        return Err(invalid_generation_data(
            "successor activation readback does not match committed identity",
        ));
    }
    Ok(identity)
}

/// Run one Work transaction while the Prepared successor owner and exact
/// candidate Session remain leased. Work staging, generation activation,
/// publication, and readback can therefore share one composite authority.
#[allow(clippy::too_many_arguments)]
pub fn with_prepared_successor_exact_session_activation<T>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
    sessions_dir: &Path,
    expected_session: &gwt_agent::SessionExecutionIdentity,
    operation: impl FnOnce(&mut dyn FnMut() -> io::Result<ExecutionGenerationIdentity>) -> io::Result<T>,
) -> io::Result<Option<T>> {
    validate_successor_request(request)?;
    if request.initial_session_id != expected_session.session_id
        || expected_session.execution_binding.session_id != expected_session.session_id
        || expected_session.execution_binding.owner_kind != owner.kind.as_str()
        || expected_session.execution_binding.owner_number != owner.number
        || expected_session.execution_binding.capability_generation != 1
    {
        return Ok(None);
    }
    with_generation_activation_leases(worktree, owner, |context| {
        gwt_agent::with_session_lease(sessions_dir, &expected_session.session_id, |session| {
            if gwt_agent::SessionExecutionIdentity::from_session(session)
                .ok()
                .flatten()
                .as_ref()
                != Some(expected_session)
                || dunce::canonicalize(&session.worktree_path).ok().as_ref()
                    != Some(&context.worktree)
            {
                return Ok(None);
            }
            let ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
                io::Error::new(
                    ErrorKind::NotFound,
                    "owner generation ledger is not initialized",
                )
            })?;
            let successor =
                prepared_successor_generation(context, &ledger, request)?.ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::NotFound,
                        "Prepared successor attempt is missing or terminal",
                    )
                })?;
            let mut planned = ledger;
            planned.current_generation_id = successor.identity.generation_id.clone();
            planned.generations.push(successor);
            let generation = planned.current_generation().ok_or_else(|| {
                invalid_generation_data("planned successor generation is not current")
            })?;
            if execution_binding_for_generation(&planned, generation)
                != expected_session.execution_binding.identity
            {
                return Ok(None);
            }
            let mut activate = || activate_successor_in_context(context, request);
            operation(&mut activate).map(Some)
        })
    })
}

/// Atomically plan and activate one successor, then rebind its existing
/// durable Session under the global → owner → Session lease order.
///
/// This is the in-place continuation path used by `execution.continue`.
/// Only the operation whose predecessor is still current can publish the
/// Prepared/Activated pair and update the Session; a stale competitor leaves
/// every authority byte untouched.
pub fn activate_successor_with_session_rebind(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
    predecessor_status: SuccessorPredecessorStatus,
    expected_current_binding: &gwt_agent::ExecutionBindingIdentity,
    sessions_dir: &Path,
    expected_session: &gwt_agent::Session,
) -> io::Result<
    Option<(
        ExecutionGenerationIdentity,
        gwt_agent::SessionExecutionBinding,
    )>,
> {
    validate_successor_request(request)?;
    let mut expected_session = expected_session.clone();
    expected_session.migrate_legacy_launch_args();
    let expected_exact_unbound = expected_session.linked_issue_number.is_none()
        && expected_session.execution_binding.is_none();
    if request.initial_session_id != expected_session.id
        || (expected_session.linked_issue_number != Some(owner.number) && !expected_exact_unbound)
        || expected_session
            .repo_hash
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Ok(None);
    }
    let expected_session_toml = toml::to_string(&expected_session)
        .map_err(|error| invalid_generation_data(format!("serialize expected Session: {error}")))?;
    with_generation_activation_leases(worktree, owner, |context| {
        let expected_repo_hash = expected_session
            .repo_hash
            .clone()
            .ok_or_else(|| invalid_generation_data("expected Session repo hash is missing"))?;
        let expected_worktree = dunce::canonicalize(&expected_session.worktree_path).ok();
        if expected_worktree.as_ref() != Some(&context.worktree) {
            return Ok(None);
        }
        let mut activated_identity = None;
        let updated = match gwt_agent::update_session_if_changed(
            sessions_dir,
            &expected_session.id,
            |session| {
                let durable_session_toml = toml::to_string(session).map_err(|error| {
                    invalid_generation_data(format!("serialize durable Session: {error}"))
                })?;
                if durable_session_toml != expected_session_toml
                    || session.repo_hash.as_deref() != Some(expected_repo_hash.as_str())
                    || dunce::canonicalize(&session.worktree_path).ok().as_ref()
                        != Some(&context.worktree)
                {
                    return Err(io::Error::new(
                        ErrorKind::WouldBlock,
                        "durable Session changed before successor activation",
                    ));
                }
                let mut ledger =
                    load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
                        io::Error::new(
                            ErrorKind::NotFound,
                            "owner generation ledger is not initialized",
                        )
                    })?;
                #[cfg(test)]
                fail_continuation_rebind_if_requested(
                    ContinuationRebindFailurePoint::BeforePrepareCommit,
                )?;
                let latest =
                    latest_operation_attempt(&ledger, request, &context.worktree_binding_hash)?
                        .cloned();
                let current = ledger.current_generation().ok_or_else(|| {
                    invalid_generation_data("execution generation ledger current id is missing")
                })?;
                let current_binding = execution_binding_for_generation(&ledger, current);
                let activated_retry_binding = latest.as_ref().and_then(|attempt| {
                    exact_current_activated_continuation_binding(&ledger, attempt)
                });
                if latest
                    .as_ref()
                    .is_some_and(|attempt| attempt.status == ContinuationAttemptStatus::Activated)
                {
                    if activated_retry_binding.as_ref() != Some(expected_current_binding) {
                        return Err(io::Error::new(
                            ErrorKind::WouldBlock,
                            "Activated successor no longer matches strict current authority",
                        ));
                    }
                } else if current_binding != *expected_current_binding {
                    return Err(io::Error::new(
                        ErrorKind::WouldBlock,
                        "successor predecessor changed before activation",
                    ));
                }
                if latest
                    .as_ref()
                    .is_none_or(|attempt| attempt.status != ContinuationAttemptStatus::Activated)
                {
                    plan_successor_for_status(context, &mut ledger, request, predecessor_status)?;
                }
                #[cfg(test)]
                fail_continuation_rebind_if_requested(
                    ContinuationRebindFailurePoint::BeforeActivationCommit,
                )?;
                let activated =
                    activate_successor_from_ledger_in_context(context, request, ledger)?;
                let readback = load_generation_ledger_from_context(context)?.ok_or_else(|| {
                    invalid_generation_data(
                        "successor Session rebind lost activated generation readback",
                    )
                })?;
                let generation = readback.current_generation().ok_or_else(|| {
                    invalid_generation_data("activated generation is not current")
                })?;
                if generation.identity != activated {
                    return Err(invalid_generation_data(
                        "successor Session rebind and activated generation disagree",
                    ));
                }
                let planned_identity = execution_binding_for_generation(&readback, generation);
                let capability_generation =
                    session.execution_binding.as_ref().map_or(1, |binding| {
                        if binding.identity == planned_identity {
                            binding.capability_generation
                        } else {
                            binding.capability_generation.saturating_add(1)
                        }
                    });
                session.linked_issue_number = Some(owner.number);
                session
                    .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
                        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                        session_id: session.id.clone(),
                        repo_hash: expected_repo_hash.clone(),
                        owner_kind: owner.kind.as_str().to_string(),
                        owner_number: owner.number,
                        identity: planned_identity.clone(),
                        capability_generation,
                    }))
                    .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
                activated_identity = Some(activated);
                Ok(())
            },
        ) {
            Ok(updated) => updated,
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        };
        let binding = updated.execution_binding.ok_or_else(|| {
            invalid_generation_data("successor Session rebind lost execution authority")
        })?;
        let activated = activated_identity.ok_or_else(|| {
            invalid_generation_data("successor Session rebind lost activation receipt")
        })?;
        Ok(Some((activated, binding)))
    })
}

/// Repair one Activated successor and run its Work commit/readback while the
/// exact current owner and durable Session incarnation remain leased.
///
/// This is intentionally separate from the Prepared activation helper:
/// response-loss recovery must be able to republish an already-committed
/// ledger projection without ever dropping the owner → Session lease between
/// authority repair and the corresponding Work decision.
#[allow(clippy::too_many_arguments)]
pub fn with_activated_successor_exact_session_repair<T>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    request: &SuccessorRequest,
    sessions_dir: &Path,
    expected_session: &gwt_agent::SessionExecutionIdentity,
    operation: impl FnOnce(&mut dyn FnMut() -> io::Result<ExecutionGenerationIdentity>) -> io::Result<T>,
) -> io::Result<Option<T>> {
    validate_successor_request(request)?;
    if request.initial_session_id != expected_session.session_id
        || expected_session.execution_binding.session_id != expected_session.session_id
        || expected_session.execution_binding.owner_kind != owner.kind.as_str()
        || expected_session.execution_binding.owner_number != owner.number
    {
        return Ok(None);
    }
    with_generation_activation_leases(worktree, owner, |context| {
        gwt_agent::with_session_lease(sessions_dir, &expected_session.session_id, |session| {
            if gwt_agent::SessionExecutionIdentity::from_session(session)
                .ok()
                .flatten()
                .as_ref()
                != Some(expected_session)
                || dunce::canonicalize(&session.worktree_path).ok().as_ref()
                    != Some(&context.worktree)
            {
                return Ok(None);
            }
            let ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
                io::Error::new(
                    ErrorKind::NotFound,
                    "owner generation ledger is not initialized",
                )
            })?;
            let latest =
                latest_operation_attempt(&ledger, request, &context.worktree_binding_hash)?
                    .ok_or_else(|| {
                        io::Error::new(ErrorKind::NotFound, "successor attempt is missing")
                    })?;
            let activated = match (
                latest.status,
                latest.activated_generation.as_ref(),
                ledger.current_generation(),
            ) {
                (ContinuationAttemptStatus::Activated, Some(activated), Some(current))
                    if activated == &current.identity
                        && ledger.effective_status_for(current)
                            == ExecutionControlStatus::Active =>
                {
                    activated
                }
                _ => return Ok(None),
            };
            if execution_binding_for_generation(
                &ledger,
                ledger
                    .current_generation()
                    .expect("validated current successor generation"),
            ) != expected_session.execution_binding.identity
                || activated.generation_id
                    != expected_session.execution_binding.identity.generation_id
                || activated.session_binding_id
                    != expected_session.execution_binding.identity.binding_id
            {
                return Ok(None);
            }
            let mut repair = || activate_successor_in_context(context, request);
            operation(&mut repair).map(Some)
        })
    })
}

fn serialized_execution_projection(record: &ExecutionControlRecord) -> io::Result<String> {
    String::from_utf8(serialize_execution_control(record)?).map_err(|error| {
        invalid_generation_data(format!(
            "serialized execution projection is not UTF-8: {error}"
        ))
    })
}

/// Stable, opaque provenance bound to one exact genesis generation/binding.
/// Ordinary `execution.blocked` transitions never carry this identifier.
#[must_use]
pub fn genesis_terminalization_operation_id(generation_id: &str, binding_id: &str) -> String {
    let digest = sha256_hex(
        serde_json::to_vec(&(
            "genesis-launch-terminalization-v1",
            generation_id,
            binding_id,
        ))
        .unwrap_or_default(),
    );
    format!("genesis-terminalization-{}", &digest[..32])
}

/// Terminalize the exact first generation when its Host launch failed after
/// generation publication but before the process/Work became reachable.
///
/// This is deliberately narrower than ordinary settlement: the durable
/// Session may not have been writable yet, so authority is the Host-held
/// pre-publication binding plus the one-generation/no-attempt ledger shape.
/// The failed generation remains immutable audit evidence and the existing
/// Blocked-successor launch path owns the next explicit retry.
pub fn block_uncommitted_genesis_launch(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    session_id: &str,
    expected_binding: &gwt_agent::ExecutionBindingIdentity,
    reason: &str,
) -> io::Result<ExecutionControlRecord> {
    validate_owner(owner)?;
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|error| invalid_generation_data(format!("invalid Session id: {error}")))?;
    if reason.trim().is_empty() {
        return Err(invalid_generation_data(
            "blocking a failed genesis launch requires a non-empty reason",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        block_uncommitted_genesis_launch_in_context(
            context,
            worktree,
            owner,
            session_id,
            expected_binding,
            reason,
        )
    })
}

fn block_uncommitted_genesis_launch_in_context(
    context: &GenerationTransactionContext,
    worktree: &Path,
    owner: ExecutionOwnerKey,
    session_id: &str,
    expected_binding: &gwt_agent::ExecutionBindingIdentity,
    reason: &str,
) -> io::Result<ExecutionControlRecord> {
    let mut ledger = load_owner_generation_ledger_from_context(context)?.ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            "owner generation ledger is not initialized",
        )
    })?;
    if ledger.generations.len() != 1
        || !ledger.continuation_attempts.is_empty()
        || !ledger.takeover_attempts.is_empty()
        || !ledger.takeovers.is_empty()
    {
        return Err(generation_conflict(
                "failed genesis terminalization requires one generation with no successor or takeover attempts",
            ));
    }
    let current = ledger
        .current_generation()
        .ok_or_else(|| {
            invalid_generation_data("execution generation ledger current id is missing")
        })?
        .clone();
    if current.identity.predecessor_generation_id.is_some()
        || current.identity.initial_session_id != session_id
        || current.identity.worktree_binding_hash != context.worktree_binding_hash
        || current.identity.generation_id != expected_binding.generation_id
        || current.identity.session_binding_id != expected_binding.binding_id
    {
        return Err(generation_conflict(
            "failed genesis terminalization lost its exact generation/session binding",
        ));
    }

    let effective_status = ledger.effective_status_for(&current);
    let operation_id = genesis_terminalization_operation_id(
        &expected_binding.generation_id,
        &expected_binding.binding_id,
    );
    if effective_status == ExecutionControlStatus::Blocked {
        let latest = ledger
            .lifecycle_events_for(&current.identity.generation_id)
            .max_by_key(|event| event.sequence)
            .ok_or_else(|| {
                invalid_generation_data("Blocked genesis generation has no lifecycle transition")
            })?;
        if latest.from_status != ExecutionControlStatus::Active
            || latest.to_status != ExecutionControlStatus::Blocked
            || latest.session_id != session_id
            || latest.reason != reason
            || latest.operation_id.as_deref() != Some(operation_id.as_str())
        {
            return Err(generation_conflict(
                "failed genesis generation was already terminalized by another outcome",
            ));
        }
        let record = serde_json::from_str::<ExecutionControlRecord>(
            ledger.effective_projection_for(&current),
        )
        .map(hydrate_recovery_envelopes)
        .map_err(|error| {
            invalid_generation_data(format!("Blocked genesis projection is malformed: {error}"))
        })?;
        // A response-loss retry also repairs a ledger-first partial
        // terminalization before reporting success.
        let projection = ledger.effective_projection_for(&current).to_string();
        write_activated_generation(context, &ledger, &projection)?;
        load_generation_ledger_from_context(context)?.ok_or_else(|| {
            invalid_generation_data(
                "failed genesis terminalization repair lost generation authority",
            )
        })?;
        return Ok(record);
    }
    if effective_status != ExecutionControlStatus::Active
        || !ledger.lifecycle_events.is_empty()
        || execution_binding_for_generation(&ledger, &current) != *expected_binding
    {
        return Err(generation_conflict(
            "failed genesis terminalization CAS lost current Active authority",
        ));
    }

    let mut record =
        serde_json::from_str::<ExecutionControlRecord>(ledger.effective_projection_for(&current))
            .map(hydrate_recovery_envelopes)
            .map_err(|error| {
                invalid_generation_data(format!("genesis execution snapshot is malformed: {error}"))
            })?;
    if !integrity_ok(&record)
        || record.owner_kind != owner.kind
        || record.owner_number != owner.number
        || record.primary_session_id != session_id
        || record.status != ExecutionControlStatus::Active
        || record.settled_at.is_some()
    {
        return Err(generation_conflict(
            "failed genesis projection no longer matches the exact Active launch",
        ));
    }
    let recorded_at = Utc::now();
    record.status = ExecutionControlStatus::Blocked;
    record.blocked_reason = Some(reason.to_string());
    record.missing_verification = Some("genesis launch readiness".to_string());
    record.settled_at = Some(recorded_at);
    let projection = serialized_execution_projection(&record)?;
    append_lifecycle_event(
        &mut ledger,
        GenerationLifecycleEvent {
            sequence: 0,
            generation_id: current.identity.generation_id,
            from_status: ExecutionControlStatus::Active,
            to_status: ExecutionControlStatus::Blocked,
            session_id: session_id.to_string(),
            reason: reason.to_string(),
            operation_id: Some(operation_id),
            recorded_at,
            execution_control_json: projection.clone(),
            previous_event_hash: String::new(),
            content_hash: String::new(),
        },
    );
    stamp_generation_ledger(&mut ledger);
    write_activated_generation(context, &ledger, &projection)?;
    let readback = load_generation_ledger_from_context(context)?.ok_or_else(|| {
        invalid_generation_data("failed genesis terminalization lost generation authority")
    })?;
    if readback.current_effective_status() != Some(ExecutionControlStatus::Blocked) {
        return Err(invalid_generation_data(
            "failed genesis terminalization readback is not Blocked",
        ));
    }
    load(worktree)?.ok_or_else(|| {
        invalid_generation_data("failed genesis terminalization lost its ECR projection")
    })
}

/// Terminalize one exact genesis generation and remove its exact Session
/// while holding leases in the canonical owner -> Session order.
#[allow(clippy::too_many_arguments)]
pub fn block_genesis_and_remove_exact_session<F>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
    reason: &str,
    sessions_dir: &Path,
    expected_session: &gwt_agent::SessionExecutionIdentity,
    after_block: F,
) -> io::Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    validate_owner(owner)?;
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|error| invalid_generation_data(format!("invalid Session id: {error}")))?;
    if reason.trim().is_empty() {
        return Err(invalid_generation_data(
            "blocking a failed genesis launch requires a non-empty reason",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        gwt_agent::remove_session_if_execution_identity_matches_or_missing_with(
            sessions_dir,
            session_id,
            expected_session,
            || {
                block_uncommitted_genesis_launch_in_context(
                    context,
                    worktree,
                    owner,
                    session_id,
                    expected_identity,
                    reason,
                )?;
                after_block()
            },
        )
    })
}

/// Terminalize one exact genesis generation only while its Session remains
/// genuinely absent under the canonical owner -> Session lease order.
#[allow(clippy::too_many_arguments)]
pub fn block_genesis_if_session_missing_with<F>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    session_id: &str,
    expected_identity: &gwt_agent::ExecutionBindingIdentity,
    reason: &str,
    sessions_dir: &Path,
    after_block: F,
) -> io::Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    validate_owner(owner)?;
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|error| invalid_generation_data(format!("invalid Session id: {error}")))?;
    if reason.trim().is_empty() {
        return Err(invalid_generation_data(
            "blocking a failed genesis launch requires a non-empty reason",
        ));
    }
    with_generation_owner_lease(worktree, owner, |context| {
        gwt_agent::with_session_path_lease(sessions_dir, session_id, |state| match state {
            gwt_agent::SessionPathState::Missing => {
                block_uncommitted_genesis_launch_in_context(
                    context,
                    worktree,
                    owner,
                    session_id,
                    expected_identity,
                    reason,
                )?;
                after_block()?;
                Ok(true)
            }
            gwt_agent::SessionPathState::Present(_) => Ok(false),
            gwt_agent::SessionPathState::Error(error) => Err(error),
        })
    })
}

/// Remove one exact Session under its owner lease without mutating owner
/// authority. Callers use this only after the authority is already terminal.
pub fn remove_exact_session_with_owner_lease<F>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    sessions_dir: &Path,
    expected_session: &gwt_agent::SessionExecutionIdentity,
    before_remove: F,
) -> io::Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    validate_owner(owner)?;
    with_generation_owner_lease(worktree, owner, |_| {
        gwt_agent::remove_session_if_execution_identity_matches_or_missing_with(
            sessions_dir,
            &expected_session.session_id,
            expected_session,
            before_remove,
        )
    })
}

/// Commit cleanup only while one Session id remains genuinely absent. This is
/// for already-terminal authority where the callback owns the remaining Work
/// mutation and must be serialized with same-id Session materialization.
pub fn commit_if_session_missing_with_owner_lease<F>(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    sessions_dir: &Path,
    session_id: &str,
    commit: F,
) -> io::Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    validate_owner(owner)?;
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|error| invalid_generation_data(format!("invalid Session id: {error}")))?;
    with_generation_owner_lease(worktree, owner, |_| {
        gwt_agent::with_session_path_lease(sessions_dir, session_id, |state| match state {
            gwt_agent::SessionPathState::Missing => {
                commit()?;
                Ok(true)
            }
            gwt_agent::SessionPathState::Present(_) => Ok(false),
            gwt_agent::SessionPathState::Error(error) => Err(error),
        })
    })
}

fn persist_generation_lifecycle_transition_if_owned(
    worktree: &Path,
    record: &ExecutionControlRecord,
    from_status: ExecutionControlStatus,
    reason: &str,
) -> io::Result<bool> {
    persist_generation_lifecycle_transition_if_owned_with_before_session_lease(
        worktree,
        record,
        from_status,
        reason,
        || {},
    )
}

fn canonical_recovery_session_snapshot(session: &gwt_agent::Session) -> io::Result<String> {
    toml::to_string(session).map_err(|error| {
        invalid_generation_data(format!("serialize recovery Session snapshot: {error}"))
    })
}

fn with_exact_recovery_session_lease<T>(
    expected_session: &gwt_agent::Session,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    gwt_agent::with_session_path_lease(
        &gwt_core::paths::gwt_sessions_dir(),
        &expected_session.id,
        |state| {
            ensure_recovery_session_snapshot_unchanged(&state, expected_session)?;
            operation()
        },
    )
}

fn ensure_recovery_session_snapshot_unchanged(
    state: &gwt_agent::SessionPathState,
    expected_session: &gwt_agent::Session,
) -> io::Result<()> {
    let expected = canonical_recovery_session_snapshot(expected_session)?;
    match state {
        gwt_agent::SessionPathState::Present(session)
            if canonical_recovery_session_snapshot(session)? == expected =>
        {
            Ok(())
        }
        gwt_agent::SessionPathState::Present(_) | gwt_agent::SessionPathState::Missing => {
            Err(io::Error::new(
                ErrorKind::PermissionDenied,
                format!(
                    "{RECOVERY_SESSION_CHANGED_PREFIX} durable Session changed after recovery preflight"
                ),
            ))
        }
        gwt_agent::SessionPathState::Error(error) => Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "{RECOVERY_SESSION_CHANGED_PREFIX} durable Session became unreadable after recovery preflight: {error}"
            ),
        )),
    }
}

fn save_legacy_recovery_record_if_session_unchanged(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    expected_session: &gwt_agent::Session,
    record: &ExecutionControlRecord,
) -> io::Result<()> {
    with_generation_owner_lease(worktree, owner, |_| {
        with_exact_recovery_session_lease(expected_session, || save(worktree, record))
    })
}

fn with_satisfied_recovery_session_lease<T>(
    worktree: &Path,
    record: &ExecutionControlRecord,
    expected_session: &gwt_agent::Session,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let owner = ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    with_generation_owner_lease(worktree, owner, |_| {
        with_exact_recovery_session_lease(expected_session, operation)
    })
}

fn generation_binding_mismatch() -> io::Error {
    io::Error::new(
        ErrorKind::PermissionDenied,
        format!(
            "{GENERATION_BINDING_MISMATCH_PREFIX} durable Session does not carry the exact current generation/binding/head"
        ),
    )
}

fn persist_generation_lifecycle_transition_if_owned_with_before_session_lease<F>(
    worktree: &Path,
    record: &ExecutionControlRecord,
    from_status: ExecutionControlStatus,
    reason: &str,
    before_session_lease: F,
) -> io::Result<bool>
where
    F: FnOnce(),
{
    persist_generation_lifecycle_transition_if_owned_with_session_snapshot_and_before_lease(
        worktree,
        record,
        from_status,
        reason,
        None,
        before_session_lease,
    )
}

fn persist_generation_lifecycle_transition_if_owned_for_recovery(
    worktree: &Path,
    record: &ExecutionControlRecord,
    from_status: ExecutionControlStatus,
    reason: &str,
    expected_session: &gwt_agent::Session,
) -> io::Result<bool> {
    persist_generation_lifecycle_transition_if_owned_with_session_snapshot_and_before_lease(
        worktree,
        record,
        from_status,
        reason,
        Some(expected_session),
        || {},
    )
}

fn persist_generation_lifecycle_transition_if_owned_with_session_snapshot_and_before_lease<F>(
    worktree: &Path,
    record: &ExecutionControlRecord,
    from_status: ExecutionControlStatus,
    reason: &str,
    expected_session: Option<&gwt_agent::Session>,
    before_session_lease: F,
) -> io::Result<bool>
where
    F: FnOnce(),
{
    let owner = ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    if !owner_generation_ledger_exists(worktree, owner)? {
        return Ok(false);
    }
    if reason.trim().is_empty() {
        return Err(invalid_generation_data(
            "generation lifecycle transition requires a non-empty reason",
        ));
    }
    let projection = serialized_execution_projection(record)?;
    with_generation_owner_lease(worktree, owner, |context| {
        let mut ledger = load_generation_ledger_from_context(context)?.ok_or_else(|| {
            invalid_generation_data("generation ledger disappeared during lifecycle transition")
        })?;
        let current = ledger
            .current_generation()
            .ok_or_else(|| {
                invalid_generation_data("execution generation ledger current id is missing")
            })?
            .clone();
        if current.identity.worktree_binding_hash != context.worktree_binding_hash {
            return Err(generation_conflict(
                "current execution generation is bound to a different worktree",
            ));
        }
        before_session_lease();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        gwt_agent::with_session_path_lease(
            &sessions_dir,
            &record.primary_session_id,
            |session_state| {
                if let Some(expected_session) = expected_session {
                    ensure_recovery_session_snapshot_unchanged(&session_state, expected_session)?;
                }
                if !session_binding_authorizes_current_generation(
                    context,
                    &ledger,
                    &record.primary_session_id,
                    &session_state,
                )? {
                    return Err(generation_binding_mismatch());
                }
                if from_status == ExecutionControlStatus::Active
                    && matches!(
                        record.status,
                        ExecutionControlStatus::Completed | ExecutionControlStatus::Blocked
                    )
                {
                    if let gwt_agent::SessionPathState::Present(session) = &session_state {
                        // Legacy genesis ledgers may have no execution-bound Session at all.
                        // Durable authority handshakes only exist for bound Sessions, so keep
                        // the existing legacy settlement compatibility while fencing every
                        // exact bound Session under the same owner -> Session lease.
                        if let Some(exact_session) =
                            gwt_agent::SessionExecutionIdentity::from_session(session)
                                .map_err(invalid_generation_data)?
                        {
                            if reconcile_active_launch_handshake_under_lease(
                                &sessions_dir,
                                &exact_session,
                            )? || gwt_agent::read_session_manual_handoff_under_lease(
                                &sessions_dir,
                                &exact_session,
                            )?
                            .is_some()
                            {
                                return Err(io::Error::new(
                                    ErrorKind::PermissionDenied,
                                    "execution transition is fenced by an in-flight Session authority handoff",
                                ));
                            }
                        }
                    }
                }
                let effective_status = ledger.effective_status_for(&current);
                if effective_status != from_status {
                    return Err(generation_conflict(format!(
                        "generation lifecycle CAS expected {from_status:?}, found {effective_status:?}"
                    )));
                }
                let prior_projection = serde_json::from_str::<ExecutionControlRecord>(
                    ledger.effective_projection_for(&current),
                )
                .map(hydrate_recovery_envelopes)
                .map_err(|error| {
                    invalid_generation_data(format!(
                        "current generation projection is malformed: {error}"
                    ))
                })?;
                if prior_projection.primary_session_id != record.primary_session_id {
                    return Err(generation_conflict(
                        "generation lifecycle session binding changed before commit",
                    ));
                }
                let allowed = matches!(
                    (from_status, record.status),
                    (
                        ExecutionControlStatus::Active,
                        ExecutionControlStatus::Completed | ExecutionControlStatus::Blocked
                    ) | (
                        ExecutionControlStatus::Blocked,
                        ExecutionControlStatus::Active
                    )
                );
                if !allowed {
                    return Err(generation_conflict(
                        "requested generation lifecycle transition is not allowed",
                    ));
                }
                let recorded_at = if record.status == ExecutionControlStatus::Active {
                    record
                        .recoveries
                        .last()
                        .map_or_else(Utc::now, |recovery| recovery.reopened_at)
                } else {
                    record.settled_at.unwrap_or_else(Utc::now)
                };
                append_lifecycle_event(
                    &mut ledger,
                    GenerationLifecycleEvent {
                        sequence: 0,
                        generation_id: current.identity.generation_id,
                        from_status,
                        to_status: record.status,
                        session_id: record.primary_session_id.clone(),
                        reason: reason.to_string(),
                        operation_id: None,
                        recorded_at,
                        execution_control_json: projection.clone(),
                        previous_event_hash: String::new(),
                        content_hash: String::new(),
                    },
                );
                stamp_generation_ledger(&mut ledger);
                write_activated_generation(context, &ledger, &projection)?;
                Ok(true)
            },
        )
    })
}

#[cfg(test)]
fn persist_generation_takeover_if_owned(
    worktree: &Path,
    record: &ExecutionControlRecord,
    transfer: &OwnershipTransfer,
) -> io::Result<bool> {
    persist_generation_takeover_if_owned_with_session(worktree, record, transfer, None)
}

fn persist_generation_takeover_if_owned_for_recovery(
    worktree: &Path,
    record: &ExecutionControlRecord,
    transfer: &OwnershipTransfer,
    expected_session: &gwt_agent::Session,
) -> io::Result<bool> {
    persist_generation_takeover_if_owned_with_session(
        worktree,
        record,
        transfer,
        Some(expected_session),
    )
}

fn persist_generation_takeover_if_owned_with_session(
    worktree: &Path,
    record: &ExecutionControlRecord,
    transfer: &OwnershipTransfer,
    expected_session: Option<&gwt_agent::Session>,
) -> io::Result<bool> {
    let owner = ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    if !owner_generation_ledger_exists(worktree, owner)? {
        return Ok(false);
    }
    let projection = serialized_execution_projection(record)?;
    with_generation_owner_lease(worktree, owner, |context| {
        let mut ledger = load_generation_ledger_from_context(context)?.ok_or_else(|| {
            invalid_generation_data("generation ledger disappeared during ownership transfer")
        })?;
        let current = ledger
            .current_generation()
            .ok_or_else(|| {
                invalid_generation_data("execution generation ledger current id is missing")
            })?
            .clone();
        let prior_projection = serde_json::from_str::<ExecutionControlRecord>(
            ledger.effective_projection_for(&current),
        )
        .map(hydrate_recovery_envelopes)
        .map_err(|error| {
            invalid_generation_data(format!(
                "current generation takeover projection is malformed: {error}"
            ))
        })?;
        let mut expected_projection = prior_projection.clone();
        expected_projection.primary_session_id = transfer.to_session_id.clone();
        expected_projection.transfers.push(transfer.clone());
        expected_projection.content_hash = compute_content_hash(&expected_projection);
        let mut actual_projection = record.clone();
        actual_projection.content_hash = compute_content_hash(&actual_projection);
        if ledger.effective_status_for(&current) != ExecutionControlStatus::Active
            || record.status != ExecutionControlStatus::Active
            || current.identity.worktree_binding_hash != context.worktree_binding_hash
            || prior_projection.primary_session_id != transfer.from_session_id
            || record.primary_session_id != transfer.to_session_id
            || actual_projection != expected_projection
        {
            return Err(generation_conflict(
                "generation takeover CAS does not match the current Active worktree/session/projection",
            ));
        }
        let commit = || {
            append_takeover_event(
                &mut ledger,
                GenerationTakeoverAudit {
                    sequence: 0,
                    generation_id: current.identity.generation_id,
                    from_session_id: transfer.from_session_id.clone(),
                    to_session_id: transfer.to_session_id.clone(),
                    reason: transfer.reason.clone(),
                    observed_at: transfer.transferred_at,
                    execution_control_json: projection.clone(),
                    previous_event_hash: String::new(),
                    content_hash: String::new(),
                },
            );
            stamp_generation_ledger(&mut ledger);
            write_activated_generation(context, &ledger, &projection)?;
            Ok(true)
        };
        match expected_session {
            Some(expected_session) => with_exact_recovery_session_lease(expected_session, commit),
            None => commit(),
        }
    })
}

/// T-107: materialize a fresh active record at launch. A fresh launch (or a
/// launch for a different owner) takes over the worktree's execution
/// lifecycle (P8a policy; authorized transfer / concurrent-owner rejection
/// is P9). A **resume** preserves an existing settled record for the same
/// owner — reopening a finished execution to inspect or discuss it must not
/// re-arm the Stop gate.
pub fn materialize_at_launch(
    worktree: &Path,
    owner_kind: ExecutionOwnerKind,
    owner_number: u64,
    session_id: &str,
    entrypoint: &str,
    resume: bool,
) -> io::Result<()> {
    // T-149: the load-modify-save cycle runs under the owner write lease so
    // concurrent launches cannot interleave into a lost update.
    crate::cli::trusted_store::with_write_lease(worktree, || {
        materialize_at_launch_locked(
            worktree,
            owner_kind,
            owner_number,
            session_id,
            entrypoint,
            resume,
        )
    })?;
    // T-181: launch is the cheap moment to sweep orphaned trusted entries
    // (runs outside the lease; GC only touches sibling directories whose
    // recorded worktree is gone).
    crate::cli::trusted_store::gc_best_effort(worktree);
    // T-275 staged core: emit the compact Phase Launch Packet (additive
    // launch context; rejection stays off until T-276 opt-in).
    crate::cli::launch_packet::write_best_effort(
        worktree,
        owner_kind.as_str(),
        owner_number,
        session_id,
        entrypoint,
    );
    Ok(())
}

fn materialize_at_launch_locked(
    worktree: &Path,
    owner_kind: ExecutionOwnerKind,
    owner_number: u64,
    session_id: &str,
    entrypoint: &str,
    resume: bool,
) -> io::Result<()> {
    // Carry the audited transfer chain forward (P9a, T-118/T-123): taking
    // over another session's ACTIVE record — fresh launch or resume — is an
    // implicit, recorded transfer instead of a silent overwrite.
    let mut transfers: Vec<OwnershipTransfer> = Vec::new();
    if let Ok(Some(existing)) = load(worktree) {
        if resume
            && existing.owner_number == owner_number
            && existing.status != ExecutionControlStatus::Active
        {
            // T-182 core: a settled record that only lives in the worktree
            // mirror (pre-P9b) is promoted into the trusted store here —
            // the resume path otherwise never rewrites it and the legacy
            // fallback would linger past its sunset.
            if crate::cli::trusted_store::read(worktree, "execution-control.json")?.is_none() {
                save(worktree, &existing)?;
            }
            return Ok(());
        }
        if existing.status == ExecutionControlStatus::Active
            && existing.primary_session_id != session_id
        {
            transfers = existing.transfers;
            transfers.push(OwnershipTransfer {
                from_session_id: existing.primary_session_id,
                to_session_id: session_id.to_string(),
                reason: if resume {
                    "resume-takeover".to_string()
                } else {
                    "launch-takeover".to_string()
                },
                transferred_at: Utc::now(),
            });
        }
    }
    save(
        worktree,
        &ExecutionControlRecord {
            owner_kind,
            owner_number,
            primary_session_id: session_id.to_string(),
            entrypoint: entrypoint.to_string(),
            bundled_required_owners: Vec::new(),
            status: ExecutionControlStatus::Active,
            blocked_reason: None,
            missing_verification: None,
            launched_at: Utc::now(),
            settled_at: None,
            transfers,
            recoveries: Vec::new(),
            content_hash: String::new(),
        },
    )
}

/// Best-effort owner-kind detection from the local issue cache: a
/// `gwt-spec`-labeled owner is a SPEC owner; uncached or unreadable owners
/// default to plain Issue (the gate mechanics do not depend on the kind).
#[must_use]
pub fn detect_owner_kind(repo_path: &Path, number: u64) -> ExecutionOwnerKind {
    let Some(cache_root) = crate::issue_cache::issue_cache_root_for_repo_path(repo_path) else {
        return ExecutionOwnerKind::Issue;
    };
    let meta_path = cache_root.join(number.to_string()).join("meta.json");
    let Ok(contents) = fs::read_to_string(&meta_path) else {
        return ExecutionOwnerKind::Issue;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return ExecutionOwnerKind::Issue;
    };
    let is_spec = value
        .get("labels")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|labels| {
            labels
                .iter()
                .any(|label| label.as_str() == Some("gwt-spec"))
        });
    if is_spec {
        ExecutionOwnerKind::Spec
    } else {
        ExecutionOwnerKind::Issue
    }
}

/// Derive the launch entrypoint for the record: the `$gwt-*` skill token from
/// the initial prompt when the launch carried one (Issue Monitor / Start Work
/// inject e.g. `$gwt-execute #N` as the trailing argv), `resume` for resumed
/// sessions, `launch` otherwise.
#[must_use]
pub fn entrypoint_from_launch(args: &[String], resume: bool) -> String {
    for arg in args.iter().rev() {
        let trimmed = arg.trim_start();
        if trimmed.starts_with("$gwt-") {
            if let Some(token) = trimmed.split_whitespace().next() {
                return token.trim_start_matches('$').to_string();
            }
        }
    }
    if resume {
        "resume".to_string()
    } else {
        "launch".to_string()
    }
}

/// Settlement outcome for [`settle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionSettlement {
    Completed,
    Blocked {
        reason: String,
        missing_verification: Option<String>,
    },
}

/// Result of a settlement attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettleResult {
    /// The record transitioned into the requested terminal state.
    Settled(ExecutionControlRecord),
    /// No record exists — settlement is an idempotent no-op (pre-P8a
    /// worktrees, unlinked launches).
    NoRecord,
    /// The record already carries a terminal state; kept as-is.
    AlreadySettled(ExecutionControlRecord),
    /// The record belongs to another session (T-100 semantics).
    SessionMismatch { record_session_id: String },
    /// The durable Session projection does not carry the exact current
    /// generation/binding/head. Session-id reuse cannot cross this fence.
    BindingMismatch,
    /// The stored integrity hash does not match the content (P9a, T-122):
    /// the record was edited outside the canonical operations.
    Tampered,
}

/// Settle the current worktree's record for `session_id`.
pub fn settle(
    worktree: &Path,
    session_id: &str,
    settlement: ExecutionSettlement,
) -> io::Result<SettleResult> {
    // T-149: settlement is a read-modify-write cycle — leased.
    crate::cli::trusted_store::with_write_lease(worktree, || {
        settle_locked(worktree, session_id, settlement)
    })
}

fn settle_locked(
    worktree: &Path,
    session_id: &str,
    settlement: ExecutionSettlement,
) -> io::Result<SettleResult> {
    let Some(mut record) = load(worktree)? else {
        return Ok(SettleResult::NoRecord);
    };
    if !integrity_ok(&record) {
        return Ok(SettleResult::Tampered);
    }
    if record.primary_session_id != session_id {
        return Ok(SettleResult::SessionMismatch {
            record_session_id: record.primary_session_id,
        });
    }
    if record.status != ExecutionControlStatus::Active {
        return Ok(SettleResult::AlreadySettled(record));
    }
    let lifecycle_reason = match settlement {
        ExecutionSettlement::Completed => {
            record.status = ExecutionControlStatus::Completed;
            "completed".to_string()
        }
        ExecutionSettlement::Blocked {
            reason,
            missing_verification,
        } => {
            record.status = ExecutionControlStatus::Blocked;
            record.blocked_reason = Some(reason.clone());
            record.missing_verification = missing_verification;
            reason
        }
    };
    record.settled_at = Some(Utc::now());
    let generation_updated = match persist_generation_lifecycle_transition_if_owned(
        worktree,
        &record,
        ExecutionControlStatus::Active,
        &lifecycle_reason,
    ) {
        Ok(updated) => updated,
        Err(error)
            if error.kind() == ErrorKind::PermissionDenied
                && error
                    .to_string()
                    .starts_with(GENERATION_BINDING_MISMATCH_PREFIX) =>
        {
            return Ok(SettleResult::BindingMismatch);
        }
        Err(error) => return Err(error),
    };
    if !generation_updated {
        save(worktree, &record)?;
    }
    Ok(SettleResult::Settled(load(worktree)?.unwrap_or(record)))
}

/// Complete an active execution only when the exact plan/run snapshot is
/// fresh, evaluating evidence and committing the terminal transition under
/// one owner write lease.
fn settle_completed_with_evidence(
    worktree: &Path,
    session_id: &str,
    expected_owner_number: Option<u64>,
) -> io::Result<Result<SettleResult, crate::cli::verification_record::EvidenceStatus>> {
    crate::cli::trusted_store::with_write_lease(worktree, || {
        let Some(record) = load(worktree)? else {
            return Ok(Ok(SettleResult::NoRecord));
        };
        if expected_owner_number.is_some_and(|expected| record.owner_number != expected) {
            return Ok(Ok(SettleResult::NoRecord));
        }
        if !integrity_ok(&record)
            || record.primary_session_id != session_id
            || record.status != ExecutionControlStatus::Active
        {
            return settle_locked(worktree, session_id, ExecutionSettlement::Completed).map(Ok);
        }

        use crate::cli::verification_record as vr;
        let verification = match vr::load(worktree) {
            Ok(Some(verification)) => verification,
            Ok(None) => return Ok(Err(vr::EvidenceStatus::MissingRecord)),
            Err(_) => return Ok(Err(vr::EvidenceStatus::Unreadable)),
        };
        let plan = match vr::load_plan(worktree) {
            Ok(plan) => plan,
            Err(_) => return Ok(Err(vr::EvidenceStatus::Unreadable)),
        };
        let status = vr::evaluate_evidence_snapshot(
            worktree,
            session_id,
            Some(record.owner_number),
            plan.as_ref(),
            &verification,
        );
        if status != vr::EvidenceStatus::Fresh {
            return Ok(Err(status));
        }
        settle_locked(worktree, session_id, ExecutionSettlement::Completed).map(Ok)
    })
}

/// Best-effort settlement used by sibling flows (`build.complete`): settles
/// the record as completed only when it exists, is active, belongs to the
/// current session, AND names the same owner the sibling flow completed —
/// a build for SPEC-N must not settle an execution launched for a different
/// owner. Every other case is silently left alone.
pub(crate) fn settle_completed_best_effort(
    worktree: &Path,
    session_id: &str,
    expected_owner_number: u64,
) {
    // T-247: build.complete settlement also skips while obligations stay
    // open — the ECR stays active and the Stop gate keeps holding.
    if let Some(refusal) =
        crate::cli::action_obligation::open_obligation_refusal(worktree, session_id, &[])
    {
        tracing::warn!(%refusal, "execution control settlement skipped");
        return;
    }
    match settle_completed_with_evidence(worktree, session_id, Some(expected_owner_number)) {
        Ok(Ok(_)) => {}
        Ok(Err(status)) => {
            tracing::warn!(?status, "execution control settlement evidence refused");
        }
        Err(error) => {
            tracing::warn!(?error, "execution control settlement failed");
        }
    }
}

/// SPEC-3248 P8b (T-112/FR-037/AS-33): PR handoff gate consumed by the
/// canonical PR operations.
///
/// - A terminally **blocked** execution refuses every PR mutation (create —
///   draft included —, edit, ready): a blocked execution cannot hand off.
/// - An **active** execution gates only Ready handoffs (`ready_handoff` =
///   non-draft create or `pr.ready`) on fresh, all-passing verification
///   evidence. Draft creation and `pr.edit` stay available as the
///   sanctioned mid-work sharing path (AGENTS Draft policy); the full PR
///   lifecycle matrix (Draft conversion, head/base drift) is T-199+.
/// - When owner generation authority exists, every mutation first authenticates
///   the ambient caller against the exact durable Session/binding. Legacy flat
///   ECRs and unmanaged repositories preserve their compatibility behavior.
pub(crate) fn pr_handoff_refusal(repo_path: &Path, ready_handoff: bool) -> Option<String> {
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let worktree = gwt_core::paths::resolve_current_worktree_root(repo_path);
    let record = match load(&worktree) {
        Ok(Some(record)) => record,
        Ok(None) => return None,
        Err(_) => {
            return Some(
                "PR mutation refused: current execution authority could not be read safely; repair or relaunch the owning execution before retrying."
                    .to_string(),
            )
        }
    };
    // P9a (T-122): a tampered record refuses every PR mutation for everyone.
    // The repair path depends on lifecycle status because adopt is Active-only.
    if !integrity_ok(&record) {
        return Some(format!(
            "PR handoff refused: the execution control record failed integrity validation (edited outside the canonical operations). {}",
            integrity_repair_guidance(record.status),
        ));
    }
    let caller_authenticated = match record.status {
        ExecutionControlStatus::Completed => {
            snapshot_pr_mutation_execution_binding(&worktree, session_id.as_deref()).map(|_| ())
        }
        ExecutionControlStatus::Active | ExecutionControlStatus::Blocked => {
            crate::cli::verification_record::authenticate_current_generation_caller(
                &worktree,
                session_id.as_deref(),
            )
        }
    };
    if caller_authenticated.is_err() {
        return Some(
            "PR mutation refused: current execution authority requires the exact durable owning Session and generation binding; relaunch or continue the owning Session before retrying."
                .to_string(),
        );
    }
    let session_id = session_id?;
    if record.primary_session_id != session_id {
        return None;
    }
    match record.status {
        ExecutionControlStatus::Completed => None,
        ExecutionControlStatus::Blocked => Some(format!(
            "PR handoff refused: the execution for {kind} #{number} is terminally blocked ({reason}). A blocked execution cannot hand off a PR. In the same owning session, resolve the blocker, register a derived matrix with `verify.plan` (`params.derive:true`), run it through `verify.run`, then call `execution.reopen` with a non-empty `params.reason`; otherwise use a fresh launch or leave the blocked report as the outcome.",
            kind = record.owner_kind.as_str(),
            number = record.owner_number,
            reason = record
                .blocked_reason
                .as_deref()
                .unwrap_or("no reason recorded"),
        )),
        ExecutionControlStatus::Active if ready_handoff => {
            let status = crate::cli::verification_record::evaluate_evidence(
                &worktree,
                &session_id,
                Some(record.owner_number),
            );
            if status == crate::cli::verification_record::EvidenceStatus::Fresh {
                None
            } else {
                Some(format!("PR handoff refused: {}", status.describe()))
            }
        }
        ExecutionControlStatus::Active => None,
    }
}

// ---------------------------------------------------------------------------
// CLI command surface (`execution.complete` / `execution.blocked`)
// ---------------------------------------------------------------------------

/// Commands of the `execution.*` JSON operation family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionCommand {
    /// Read-only diagnosis of the current execution and its recovery paths.
    Status,
    Complete,
    Blocked {
        reason: String,
        missing_verification: Option<String>,
    },
    /// P9a (T-117): take over the worktree's active record for the current
    /// session with an audited reason (crash recovery, window handoff).
    Adopt {
        reason: String,
    },
    /// Quarantine corrupt execution authority and materialize one fresh,
    /// auditable Active generation for the current owner/session.
    Repair {
        reason: String,
    },
    /// Ask the current Host to establish producing authority for this Session.
    Continue {
        operation_id: String,
    },
    /// Return the current session's terminal Blocked record to Active only
    /// after fresh, derived, post-block verification evidence exists.
    Reopen {
        reason: String,
    },
}

/// Stable lifecycle state exposed by [`diagnose`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionDiagnosisState {
    Active,
    Completed,
    Blocked,
    Missing,
    Corrupt,
}

/// Stable binding classification exposed by [`diagnose`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionBindingState {
    Bound,
    Missing,
    Stale,
    Terminal,
    HostUnreachable,
    Unknown,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionContinuationDiagnosis {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predecessor_generation_id: Option<String>,
    pub predecessor_stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_writer: Option<String>,
    pub generation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub takeover_audit_id: Option<String>,
    pub validated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionRepairDiagnosis {
    pub outcome: String,
    pub repair_id: String,
    pub new_generation_id: String,
    pub repaired_at: DateTime<Utc>,
    pub source_kinds: Vec<ExecutionRepairSourceKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRepairSourceKind {
    ExecutionControl,
    GenerationPointer,
    GenerationLedger,
    Unknown,
}

/// Stable reason why a local D3-R repair could not restore exact Host
/// authority. A failed record is diagnostic evidence only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BindingRepairFailureCause {
    SessionPersistenceFailed,
    CapabilityPromotionFailed,
    ProjectStateAnchorInvalid,
    ProbeTransportFailed,
    ProbeReceiptMismatch,
    ActiveAuthorityMismatch,
}

/// Truthful result of a local binding repair attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BindingRepairOutcome {
    Succeeded {
        host_instance_id: String,
        receipt_generation_id: String,
    },
    Failed {
        cause: BindingRepairFailureCause,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observed_generation_id: Option<String>,
    },
}

/// Integrity-protected D3-R outcome stored in the repository-scoped trusted
/// store. It never grants authority; it records what the authenticated Host
/// probe actually proved.
#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BindingRepairOutcomeRecord {
    pub schema_version: u32,
    pub operation_id: String,
    pub session_id: String,
    pub owner: ExecutionOwnerKey,
    pub binding: gwt_agent::SessionExecutionBinding,
    pub outcome: BindingRepairOutcome,
    pub recorded_at: DateTime<Utc>,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExecutionBindingRepairDiagnosis {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_cause: Option<BindingRepairFailureCause>,
    pub operation_id: String,
    pub session_id: String,
    pub generation_id: String,
    pub capability_generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation_id: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub matches_current_generation: bool,
    pub validated: bool,
}

#[doc(hidden)]
#[must_use]
pub fn new_binding_repair_outcome_record(
    binding: &gwt_agent::SessionExecutionBinding,
    owner: ExecutionOwnerKey,
    outcome: BindingRepairOutcome,
) -> BindingRepairOutcomeRecord {
    BindingRepairOutcomeRecord {
        schema_version: BINDING_REPAIR_OUTCOME_SCHEMA_VERSION,
        operation_id: BINDING_REPAIR_OPERATION_ID.to_string(),
        session_id: binding.session_id.clone(),
        owner,
        binding: binding.clone(),
        outcome,
        recorded_at: Utc::now(),
        content_hash: String::new(),
    }
}

fn binding_repair_outcome_hash(record: &BindingRepairOutcomeRecord) -> io::Result<String> {
    use sha2::{Digest, Sha256};

    let mut canonical = record.clone();
    canonical.content_hash.clear();
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_binding_repair_outcome(record: &BindingRepairOutcomeRecord) -> io::Result<()> {
    let invalid = |message| io::Error::new(ErrorKind::InvalidData, message);
    if record.schema_version != BINDING_REPAIR_OUTCOME_SCHEMA_VERSION
        || record.operation_id != BINDING_REPAIR_OPERATION_ID
        || record.owner.number == 0
        || record.binding.schema_version
            != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
        || record.binding.session_id != record.session_id
        || record.binding.owner_kind != record.owner.kind.as_str()
        || record.binding.owner_number != record.owner.number
        || record.binding.repo_hash.trim().is_empty()
        || record.binding.capability_generation == 0
        || record.binding.identity.generation_id.trim().is_empty()
        || record.binding.identity.binding_id.trim().is_empty()
        || record.binding.identity.ledger_head_hash.trim().is_empty()
        || gwt_agent::validate_session_id_path_component(&record.session_id).is_err()
    {
        return Err(invalid("binding repair outcome identity is not canonical"));
    }
    match &record.outcome {
        BindingRepairOutcome::Succeeded {
            host_instance_id,
            receipt_generation_id,
        } if !host_instance_id.trim().is_empty()
            && receipt_generation_id == &record.binding.identity.generation_id => {}
        BindingRepairOutcome::Failed {
            observed_generation_id,
            ..
        } if observed_generation_id
            .as_deref()
            .is_none_or(|generation| !generation.trim().is_empty()) => {}
        BindingRepairOutcome::Succeeded { .. } | BindingRepairOutcome::Failed { .. } => {
            return Err(invalid("binding repair outcome payload is not canonical"));
        }
    }
    if record.content_hash.is_empty() || record.content_hash != binding_repair_outcome_hash(record)?
    {
        return Err(invalid("binding repair outcome integrity mismatch"));
    }
    Ok(())
}

fn decode_binding_repair_outcome(contents: &str) -> io::Result<BindingRepairOutcomeRecord> {
    let record: BindingRepairOutcomeRecord = serde_json::from_str(contents)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    validate_binding_repair_outcome(&record)?;
    Ok(record)
}

/// Load the latest D3-R outcome without mutating execution state.
#[doc(hidden)]
pub fn load_binding_repair_outcome(
    worktree: &Path,
) -> io::Result<Option<BindingRepairOutcomeRecord>> {
    crate::cli::trusted_store::read(worktree, BINDING_REPAIR_OUTCOME_FILE)?
        .map(|contents| decode_binding_repair_outcome(&contents))
        .transpose()
}

/// Persist and read back one D3-R outcome under the trusted-store lease.
#[doc(hidden)]
pub fn persist_binding_repair_outcome(
    worktree: &Path,
    mut record: BindingRepairOutcomeRecord,
) -> io::Result<BindingRepairOutcomeRecord> {
    record.content_hash = binding_repair_outcome_hash(&record)?;
    validate_binding_repair_outcome(&record)?;
    let bytes = serde_json::to_vec_pretty(&record)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    let readback = crate::cli::trusted_store::write_with_readback(
        worktree,
        BINDING_REPAIR_OUTCOME_FILE,
        &bytes,
    )?;
    let decoded = decode_binding_repair_outcome(&readback)?;
    if decoded != record {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "binding repair outcome readback changed",
        ));
    }
    Ok(decoded)
}

/// Record one D3-R result. A successful repair is returned as true only when
/// its integrity-protected outcome was persisted and read back exactly.
#[doc(hidden)]
pub fn record_binding_repair_outcome(worktree: &Path, record: BindingRepairOutcomeRecord) -> bool {
    let session_id = record.session_id.clone();
    let succeeded = matches!(record.outcome, BindingRepairOutcome::Succeeded { .. });
    match persist_binding_repair_outcome(worktree, record) {
        Ok(_) => succeeded,
        Err(error) => {
            tracing::warn!(
                session_id,
                error = %error,
                "binding repair outcome could not be persisted and verified"
            );
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ExecutionObligationRevivalDiagnosis {
    Revived {
        kinds: Vec<crate::cli::action_obligation::ObligationKind>,
    },
    Deferred {
        reason: String,
    },
    PersistFailed {
        error: String,
    },
    StatusUnreadable {
        error: String,
    },
}

impl From<crate::cli::action_obligation::ObligationRevivalOutcome>
    for ExecutionObligationRevivalDiagnosis
{
    fn from(value: crate::cli::action_obligation::ObligationRevivalOutcome) -> Self {
        use crate::cli::action_obligation::ObligationRevivalOutcome;
        match value {
            ObligationRevivalOutcome::Revived { kinds } => Self::Revived { kinds },
            ObligationRevivalOutcome::Deferred { reason } => Self::Deferred { reason },
            ObligationRevivalOutcome::PersistFailed { error } => Self::PersistFailed { error },
        }
    }
}

/// Read-only aggregate used by both the JSON operation and GUI projection.
///
/// Facts are collected from the existing state-machine readers. This type
/// deliberately does not infer a successful state from unreadable data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionDiagnosisSnapshot {
    pub schema_version: u32,
    pub ecr_status: ExecutionDiagnosisState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_kind: Option<ExecutionOwnerKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_verification: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    pub binding_state: ExecutionBindingState,
    pub binding_cause: String,
    pub verification_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trivial_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generated_outputs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation: Option<ExecutionContinuationDiagnosis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_update_applicable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_update_applicability_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obligation_revival: Option<ExecutionObligationRevivalDiagnosis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_repair: Option<ExecutionBindingRepairDiagnosis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair: Option<ExecutionRepairDiagnosis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_event_receipt_generation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_event_receipt_matches_current_generation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settlement: Option<crate::cli::verification_record::WorkEventSettlementStatus>,
    pub settlement_severity: String,
    pub settlement_obligation_open: bool,
    pub open_obligations: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recovery_probes: Vec<crate::cli::governance::RecoveryProbe>,
    pub available_recoveries: Vec<String>,
    pub warnings: Vec<String>,
}

fn evidence_status_name(status: crate::cli::verification_record::EvidenceStatus) -> &'static str {
    use crate::cli::verification_record::EvidenceStatus;
    match status {
        EvidenceStatus::Fresh => "fresh",
        EvidenceStatus::MissingRecord => "missing_record",
        EvidenceStatus::WrongSession => "wrong_session",
        EvidenceStatus::WrongOwner => "wrong_owner",
        EvidenceStatus::WrongGeneration => "wrong_generation",
        EvidenceStatus::StaleFingerprint => "stale_fingerprint",
        EvidenceStatus::Failing => "failing",
        EvidenceStatus::Unreadable => "unreadable",
        EvidenceStatus::Tampered => "tampered",
        EvidenceStatus::PlanNotCovered => "plan_not_covered",
        EvidenceStatus::PlanChanged => "plan_changed",
    }
}

fn probe_blocked_build_abort_recovery(
    worktree: &Path,
    snapshot: &ExecutionDiagnosisSnapshot,
    session_id: Option<&str>,
    recovery_context: Option<
        &Result<crate::agent_project_state::ExecutionRecoveryContext, gwt_core::GwtError>,
    >,
) -> crate::cli::governance::RecoveryProbe {
    use crate::cli::governance::{
        GovernanceCause, GovernanceEffect, GovernanceMetadata, RecoveryProbe,
    };

    let governance = GovernanceMetadata {
        effect: Some(GovernanceEffect::Protected),
        retryable: Some(true),
        target_state: Some("discarded".to_string()),
        execution_generation: snapshot.generation_id.clone(),
        ..GovernanceMetadata::default()
    };
    let unavailable = |cause, reason: &str| {
        RecoveryProbe::unavailable(
            "build.abort",
            GovernanceMetadata {
                cause: Some(cause),
                retryable: Some(false),
                ..governance.clone()
            },
            reason,
        )
    };
    if snapshot.ecr_status != ExecutionDiagnosisState::Blocked {
        return unavailable(
            GovernanceCause::DomainInvalid,
            "build_abort_requires_blocked_execution",
        );
    }
    let (Some(owner_kind), Some(owner_number)) = (snapshot.owner_kind, snapshot.owner_number)
    else {
        return unavailable(
            GovernanceCause::Integrity,
            "build_abort_execution_owner_unavailable",
        );
    };
    let owner = ExecutionOwnerKey {
        kind: owner_kind,
        number: owner_number,
    };
    let (Some(session_id), Some(Ok(recovery_context))) = (session_id, recovery_context) else {
        return unavailable(
            GovernanceCause::ManagedIdentity,
            "execution_recovery_scope_invalid",
        );
    };
    let Some(binding) = recovery_context.session().execution_binding.as_ref() else {
        return unavailable(
            GovernanceCause::Authority,
            "build_abort_execution_binding_unavailable",
        );
    };
    if recovery_context.worktree() != worktree
        || binding.schema_version != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
        || binding.session_id != session_id
        || binding.owner_kind != owner.kind.as_str()
        || binding.owner_number != owner.number
        || binding.capability_generation == 0
    {
        return unavailable(
            GovernanceCause::Authority,
            "build_abort_execution_binding_mismatch",
        );
    }
    match blocked_build_abort_execution_binding_matches(
        worktree,
        owner,
        session_id,
        &binding.identity,
    ) {
        Ok(true) => {}
        Ok(false) => {
            return unavailable(
                GovernanceCause::NotReady,
                "build_abort_lifecycle_authority_unavailable",
            )
        }
        Err(error) => return unavailable(GovernanceCause::Integrity, &error.to_string()),
    }
    match crate::agent_project_state::snapshot_bound_terminal_compatibility_authority(
        worktree,
        session_id,
        crate::AgentWorkTerminalKind::Discarded,
    ) {
        Ok(Some(authority)) => match authority.requires_blocked_build_abort_bridge() {
            Ok(true) => RecoveryProbe::available("build.abort", governance),
            Ok(false) => unavailable(
                GovernanceCause::Authority,
                "build_abort_terminal_authority_unavailable",
            ),
            Err(error) => unavailable(GovernanceCause::Integrity, &error.to_string()),
        },
        Ok(None) => unavailable(
            GovernanceCause::Authority,
            "build_abort_requires_exact_host_work_authority",
        ),
        Err(error) => unavailable(GovernanceCause::Authority, &error.to_string()),
    }
}

fn generation_writer(ledger: &ExecutionGenerationLedger, generation_id: &str) -> Option<String> {
    let generation = ledger
        .generations
        .iter()
        .find(|generation| generation.identity.generation_id == generation_id)?;
    serde_json::from_str::<ExecutionControlRecord>(ledger.effective_projection_for(generation))
        .map(hydrate_recovery_envelopes)
        .ok()
        .map(|record| record.primary_session_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionDiagnosisMode {
    OperationLocal,
    Projection,
}

const PROTECTED_RECOVERY_OPERATIONS: [&str; 7] = [
    "build.abort",
    "execution.continue",
    "execution.repair",
    "execution.adopt",
    "execution.reopen",
    "workspace.update",
    "workspace.ensure",
];

/// Collect the current operation-local diagnosis without mutating trusted state.
#[must_use]
pub fn diagnose(worktree: &Path, session_id: Option<&str>) -> ExecutionDiagnosisSnapshot {
    diagnose_with_mode(worktree, session_id, ExecutionDiagnosisMode::OperationLocal)
}

/// Read an exact Worktree's durable diagnosis for GUI projection without
/// resolving Git identity or advertising unvalidated protected recovery.
#[doc(hidden)]
#[must_use]
pub fn diagnose_for_projection(
    worktree: &Path,
    session_id: Option<&str>,
) -> ExecutionDiagnosisSnapshot {
    diagnose_with_mode(worktree, session_id, ExecutionDiagnosisMode::Projection)
}

fn diagnose_with_mode(
    invocation_scope: &Path,
    session_id: Option<&str>,
    mode: ExecutionDiagnosisMode,
) -> ExecutionDiagnosisSnapshot {
    let session_id = session_id.map(str::trim).filter(|value| !value.is_empty());
    let recovery_context = (mode == ExecutionDiagnosisMode::OperationLocal)
        .then(|| {
            session_id.map(|session_id| {
                crate::agent_project_state::resolve_execution_recovery_context(
                    invocation_scope,
                    session_id,
                )
            })
        })
        .flatten();
    let resolved_worktree = recovery_context
        .as_ref()
        .and_then(|context| context.as_ref().ok())
        .map(|context| context.worktree().to_path_buf())
        .unwrap_or_else(|| match mode {
            ExecutionDiagnosisMode::OperationLocal => {
                gwt_core::paths::resolve_current_worktree_root(invocation_scope)
            }
            ExecutionDiagnosisMode::Projection => invocation_scope.to_path_buf(),
        });
    let worktree = resolved_worktree.as_path();
    let mut snapshot = ExecutionDiagnosisSnapshot {
        schema_version: 1,
        ecr_status: ExecutionDiagnosisState::Missing,
        owner_kind: None,
        owner_number: None,
        blocked_reason: None,
        missing_verification: None,
        generation_id: None,
        binding_state: ExecutionBindingState::Missing,
        binding_cause: "no_execution_control_record".to_string(),
        verification_state: "not_evaluated".to_string(),
        trivial_reason: None,
        generated_outputs: Vec::new(),
        capability_generation: None,
        continuation: None,
        workspace_update_applicable: None,
        workspace_update_applicability_reason: None,
        obligation_revival: None,
        binding_repair: None,
        repair: None,
        work_event_receipt_generation_id: None,
        work_event_receipt_matches_current_generation: None,
        settlement: None,
        settlement_severity: "unknown".to_string(),
        settlement_obligation_open: false,
        open_obligations: Vec::new(),
        recovery_probes: Vec::new(),
        available_recoveries: vec!["gwt-execute".to_string()],
        warnings: Vec::new(),
    };

    let record = match load(worktree) {
        Ok(Some(record)) => record,
        Ok(None) => {
            return finalize_diagnosis(
                mode,
                worktree,
                session_id,
                recovery_context.as_ref(),
                snapshot,
            )
        }
        Err(error) => {
            snapshot.ecr_status = ExecutionDiagnosisState::Corrupt;
            snapshot.binding_state = ExecutionBindingState::Corrupt;
            snapshot.binding_cause = "execution_control_unreadable".to_string();
            snapshot.available_recoveries = vec!["execution.repair".to_string()];
            snapshot
                .warnings
                .push(format!("execution control is unreadable: {error}"));
            return finalize_diagnosis(
                mode,
                worktree,
                session_id,
                recovery_context.as_ref(),
                snapshot,
            );
        }
    };

    snapshot.owner_kind = Some(record.owner_kind);
    snapshot.owner_number = Some(record.owner_number);
    snapshot.blocked_reason.clone_from(&record.blocked_reason);
    snapshot
        .missing_verification
        .clone_from(&record.missing_verification);
    if !integrity_ok(&record) {
        snapshot.ecr_status = ExecutionDiagnosisState::Corrupt;
        snapshot.binding_state = ExecutionBindingState::Corrupt;
        snapshot.binding_cause = "execution_control_integrity_failure".to_string();
        snapshot.available_recoveries = vec!["execution.repair".to_string()];
        snapshot
            .warnings
            .push("execution control integrity validation failed".to_string());
        return finalize_diagnosis(
            mode,
            worktree,
            session_id,
            recovery_context.as_ref(),
            snapshot,
        );
    }

    snapshot.ecr_status = match record.status {
        ExecutionControlStatus::Active => ExecutionDiagnosisState::Active,
        ExecutionControlStatus::Completed => ExecutionDiagnosisState::Completed,
        ExecutionControlStatus::Blocked => ExecutionDiagnosisState::Blocked,
    };
    let owner = ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    match load_generation_ledger(worktree, owner) {
        Ok(Some(ledger)) if generation_ledger_integrity_ok(&ledger) => {
            snapshot.generation_id = Some(ledger.current_generation_id.clone());
            let mut continuation_evidence = Vec::new();
            if let Some(attempt) = ledger
                .continuation_attempts
                .iter()
                .rev()
                .find(|attempt| attempt.status == ContinuationAttemptStatus::Activated)
                .or_else(|| ledger.continuation_attempts.last())
            {
                let status = match attempt.status {
                    ContinuationAttemptStatus::Prepared => "prepared",
                    ContinuationAttemptStatus::Aborted => "aborted",
                    ContinuationAttemptStatus::Activated => "activated",
                };
                let activated = attempt.activated_generation.as_ref();
                let validated = attempt.status == ContinuationAttemptStatus::Activated
                    && activated.is_some_and(|generation| {
                        generation.generation_id == ledger.current_generation_id
                    });
                continuation_evidence.push((
                    attempt.recorded_at,
                    ExecutionContinuationDiagnosis {
                        status: status.to_string(),
                        outcome: (attempt.status == ContinuationAttemptStatus::Activated)
                            .then(|| "successor_created".to_string()),
                        predecessor_generation_id: Some(attempt.predecessor.generation_id.clone()),
                        predecessor_stale: validated
                            && attempt.predecessor.generation_id != ledger.current_generation_id,
                        from_session_id: generation_writer(
                            &ledger,
                            &attempt.predecessor.generation_id,
                        ),
                        current_writer: generation_writer(&ledger, &ledger.current_generation_id),
                        generation_id: activated.map_or_else(
                            || attempt.candidate_generation_id.clone(),
                            |generation| generation.generation_id.clone(),
                        ),
                        takeover_audit_id: (attempt.predecessor_status
                            == SuccessorPredecessorStatus::Active)
                            .then(|| attempt.request.operation_id.clone()),
                        validated,
                    },
                ));
                snapshot.warnings.push(format!(
                    "latest_continuation_attempt:{status}:{}",
                    attempt.candidate_generation_id
                ));
            }
            if let Some(validation) = ledger.continuation_validations.last() {
                continuation_evidence.push((
                    validation.recorded_at,
                    ExecutionContinuationDiagnosis {
                        status: "validated".to_string(),
                        outcome: Some("rebound_current".to_string()),
                        predecessor_generation_id: None,
                        predecessor_stale: false,
                        from_session_id: None,
                        current_writer: generation_writer(&ledger, &ledger.current_generation_id),
                        generation_id: validation.generation_id.clone(),
                        takeover_audit_id: None,
                        validated: validation.generation_id == ledger.current_generation_id
                            && validation.execution_binding.generation_id
                                == ledger.current_generation_id,
                    },
                ));
                snapshot.warnings.push(format!(
                    "latest_continuation_validation:rebound_current:{}",
                    validation.generation_id
                ));
            }
            if let Some(takeover) = ledger.takeovers.last() {
                continuation_evidence.push((
                    takeover.observed_at,
                    ExecutionContinuationDiagnosis {
                        status: "activated".to_string(),
                        outcome: Some("takeover".to_string()),
                        predecessor_generation_id: Some(takeover.generation_id.clone()),
                        predecessor_stale: takeover.generation_id == ledger.current_generation_id,
                        from_session_id: Some(takeover.from_session_id.clone()),
                        current_writer: generation_writer(&ledger, &ledger.current_generation_id),
                        generation_id: takeover.generation_id.clone(),
                        takeover_audit_id: Some(takeover.content_hash.clone()),
                        validated: takeover.generation_id == ledger.current_generation_id,
                    },
                ));
                snapshot.warnings.push(format!(
                    "latest_takeover:{}:{}:{}",
                    takeover.generation_id, takeover.from_session_id, takeover.to_session_id
                ));
            }
            snapshot.continuation = continuation_evidence
                .into_iter()
                .max_by_key(|(recorded_at, _)| *recorded_at)
                .map(|(_, diagnosis)| diagnosis);
            match record.status {
                ExecutionControlStatus::Completed | ExecutionControlStatus::Blocked => {
                    snapshot.binding_state = ExecutionBindingState::Terminal;
                    snapshot.binding_cause = "terminal_generation".to_string();
                }
                ExecutionControlStatus::Active => match session_id {
                    Some(session_id) => {
                        match crate::cli::verification_record::
                            snapshot_current_generation_caller_binding(
                                worktree,
                                Some(session_id),
                            )
                        {
                            Ok(binding) => {
                                snapshot.binding_state = ExecutionBindingState::Bound;
                                snapshot.binding_cause = "current_generation".to_string();
                                snapshot.capability_generation =
                                    binding.map(|binding| binding.capability_generation);
                            }
                            Err(_) => {
                                snapshot.binding_state = ExecutionBindingState::Stale;
                                snapshot.binding_cause =
                                    "current_session_not_authorized".to_string();
                            }
                        }
                    }
                    None => {
                        snapshot.binding_state = ExecutionBindingState::Unknown;
                        snapshot.binding_cause = "session_id_unavailable".to_string();
                    }
                },
            }
        }
        Ok(Some(_)) => {
            snapshot.binding_state = ExecutionBindingState::Corrupt;
            snapshot.binding_cause = "generation_ledger_integrity_failure".to_string();
            snapshot
                .warnings
                .push("generation ledger integrity validation failed".to_string());
        }
        Ok(None) => {
            snapshot.binding_state = ExecutionBindingState::Missing;
            snapshot.binding_cause = "generation_ledger_missing".to_string();
        }
        Err(error) => {
            snapshot.binding_state = ExecutionBindingState::Corrupt;
            snapshot.binding_cause = "generation_ledger_unreadable".to_string();
            snapshot
                .warnings
                .push(format!("generation ledger is unreadable: {error}"));
        }
    }

    if let Some(session_id) = session_id {
        if mode == ExecutionDiagnosisMode::OperationLocal
            && snapshot.ecr_status == ExecutionDiagnosisState::Active
        {
            match crate::agent_project_state::diagnose_session_work_mutation_target(
                worktree, session_id,
            ) {
                Ok(_) => snapshot.workspace_update_applicable = Some(true),
                Err(failure) => {
                    snapshot.workspace_update_applicable = Some(false);
                    snapshot.workspace_update_applicability_reason =
                        Some(failure.reason.as_str().to_string());
                    snapshot.warnings.push(format!(
                        "workspace_update_not_applicable:{}",
                        failure.message
                    ));
                }
            }
        }
        if snapshot.binding_state == ExecutionBindingState::Bound
            && std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV).is_some()
        {
            match crate::daemon_runtime::HookForwardTarget::from_env_strict() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    snapshot.binding_state = ExecutionBindingState::HostUnreachable;
                    snapshot.binding_cause = "host_bridge_capability_missing".to_string();
                }
                Err(error) => {
                    snapshot.binding_state = ExecutionBindingState::HostUnreachable;
                    snapshot.binding_cause = "host_bridge_capability_invalid".to_string();
                    snapshot
                        .warnings
                        .push(format!("Host bridge is unavailable: {error}"));
                }
            }
        }
        if mode == ExecutionDiagnosisMode::OperationLocal {
            snapshot.verification_state =
                evidence_status_name(crate::cli::verification_record::evaluate_evidence(
                    worktree,
                    session_id,
                    Some(record.owner_number),
                ))
                .to_string();
        }
        if let Ok(Some(plan)) = crate::cli::verification_record::load_plan(worktree) {
            snapshot
                .generated_outputs
                .clone_from(&plan.generated_outputs);
            if plan.derived && plan.commands.is_empty() {
                if let Some(reason) = plan.surfaces.iter().find_map(|surface| {
                    surface
                        .strip_prefix("trivial(")
                        .and_then(|value| value.strip_suffix(')'))
                }) {
                    snapshot.trivial_reason = Some(reason.to_string());
                    snapshot
                        .warnings
                        .push(format!("trivial_verification_reason:{reason}"));
                }
            }
        }
        snapshot.open_obligations = crate::cli::action_obligation::open_kinds(worktree, session_id)
            .into_iter()
            .map(|kind| kind.as_str().to_string())
            .collect();
        match crate::cli::action_obligation::load_revival_record(worktree, session_id) {
            Ok(Some(record)) => {
                snapshot.obligation_revival = Some(record.result.clone().into());
                match record.result {
                    crate::cli::action_obligation::ObligationRevivalOutcome::Revived { .. } => {}
                    crate::cli::action_obligation::ObligationRevivalOutcome::Deferred {
                        reason,
                    } => snapshot
                        .warnings
                        .push(format!("obligation revival deferred: {reason}")),
                    crate::cli::action_obligation::ObligationRevivalOutcome::PersistFailed {
                        error,
                    } => snapshot
                        .warnings
                        .push(format!("obligation revival persistence failed: {error}")),
                }
            }
            Ok(None) => {
                if !record.recoveries.is_empty() {
                    snapshot.obligation_revival =
                        Some(ExecutionObligationRevivalDiagnosis::StatusUnreadable {
                            error: "revival_outcome_missing_after_reopen".to_string(),
                        });
                    snapshot.warnings.push(
                        "obligation revival status is missing after execution.reopen".to_string(),
                    );
                }
            }
            Err(error) => {
                let error = error.to_string();
                snapshot.obligation_revival =
                    Some(ExecutionObligationRevivalDiagnosis::StatusUnreadable {
                        error: error.clone(),
                    });
                snapshot
                    .warnings
                    .push(format!("obligation revival status is unreadable: {error}"));
            }
        }
    }
    match crate::cli::verification_record::load_work_event_settlement_record(worktree) {
        Ok(Some(settlement)) => {
            snapshot.work_event_receipt_generation_id = settlement
                .execution_binding
                .as_ref()
                .map(|binding| binding.generation_id.clone());
            match crate::cli::verification_record::work_event_receipt_authorizes_current_generation(
                worktree,
                &settlement,
            ) {
                Ok(matches) => {
                    snapshot.work_event_receipt_matches_current_generation = Some(matches);
                }
                Err(error) => snapshot.warnings.push(format!(
                    "work event settlement receipt authority is unreadable: {error}"
                )),
            }
            snapshot.settlement_obligation_open = settlement.obligation_open;
            snapshot.settlement_severity = match settlement.status.severity() {
                crate::cli::verification_record::WorkEventSettlementSeverity::Clear => "clear",
                crate::cli::verification_record::WorkEventSettlementSeverity::Warning => "warning",
                crate::cli::verification_record::WorkEventSettlementSeverity::Blocked => "blocked",
            }
            .to_string();
            snapshot.settlement = Some(settlement.status);
        }
        Ok(None) => {}
        Err(error) => snapshot.warnings.push(format!(
            "work event settlement record is unreadable: {error}"
        )),
    }
    match load_binding_repair_outcome(worktree) {
        Ok(Some(repair)) if repair.owner == owner => {
            let matches_current_generation = snapshot.generation_id.as_deref()
                == Some(repair.binding.identity.generation_id.as_str());
            let (status, failure_cause, host_instance_id, observed_generation_id, validated) =
                match repair.outcome {
                    BindingRepairOutcome::Succeeded {
                        host_instance_id,
                        receipt_generation_id,
                    } => (
                        "succeeded",
                        None,
                        Some(host_instance_id),
                        None,
                        receipt_generation_id == repair.binding.identity.generation_id,
                    ),
                    BindingRepairOutcome::Failed {
                        cause,
                        observed_generation_id,
                    } => ("failed", Some(cause), None, observed_generation_id, true),
                };
            snapshot.binding_repair = Some(ExecutionBindingRepairDiagnosis {
                status: status.to_string(),
                failure_cause,
                operation_id: repair.operation_id,
                session_id: repair.session_id,
                generation_id: repair.binding.identity.generation_id,
                capability_generation: repair.binding.capability_generation,
                host_instance_id,
                observed_generation_id,
                recorded_at: repair.recorded_at,
                matches_current_generation,
                validated,
            });
            if !matches_current_generation {
                snapshot
                    .warnings
                    .push("binding repair outcome belongs to a prior generation".to_string());
            }
        }
        Ok(Some(repair)) => snapshot.warnings.push(format!(
            "binding repair outcome owner mismatch: {} #{}",
            repair.owner.kind.as_str(),
            repair.owner.number
        )),
        Ok(None) => {}
        Err(error) => snapshot
            .warnings
            .push(format!("binding repair outcome is unreadable: {error}")),
    }

    if let Some(generation_id) = snapshot.generation_id.as_deref() {
        if let Ok(context) = GenerationTransactionContext::resolve(worktree, owner) {
            if let Ok(audits) =
                repair_audit_dir(&context).and_then(|audit_dir| load_repair_audits(&audit_dir))
            {
                if let Some(audit) = audits
                    .iter()
                    .rev()
                    .find(|audit| audit.new_generation_id == generation_id)
                {
                    let source_kinds = audit
                        .sources
                        .iter()
                        .map(|source| {
                            match Path::new(&source.source_path)
                                .file_name()
                                .and_then(|name| name.to_str())
                            {
                                Some("execution-control.json") => {
                                    ExecutionRepairSourceKind::ExecutionControl
                                }
                                Some(GENERATION_POINTER_FILE) => {
                                    ExecutionRepairSourceKind::GenerationPointer
                                }
                                Some(GENERATION_LEDGER_FILE) => {
                                    ExecutionRepairSourceKind::GenerationLedger
                                }
                                _ => ExecutionRepairSourceKind::Unknown,
                            }
                        })
                        .collect();
                    snapshot.repair = Some(ExecutionRepairDiagnosis {
                        outcome: "activated".to_string(),
                        repair_id: audit.repair_id.clone(),
                        new_generation_id: audit.new_generation_id.clone(),
                        repaired_at: audit.repaired_at,
                        source_kinds,
                    });
                }
            }
        }
    }

    let mut execution_recoveries = if snapshot.binding_state == ExecutionBindingState::Corrupt {
        vec!["execution.repair".to_string()]
    } else {
        match snapshot.ecr_status {
            ExecutionDiagnosisState::Missing => vec!["gwt-execute".to_string()],
            ExecutionDiagnosisState::Corrupt => vec!["execution.repair".to_string()],
            ExecutionDiagnosisState::Blocked => vec![
                "verify.plan".to_string(),
                "verify.run".to_string(),
                "execution.reopen".to_string(),
            ],
            ExecutionDiagnosisState::Completed => vec!["gwt-execute".to_string()],
            ExecutionDiagnosisState::Active
                if snapshot.binding_state != ExecutionBindingState::Bound =>
            {
                vec!["execution.continue".to_string()]
            }
            ExecutionDiagnosisState::Active => Vec::new(),
        }
    };
    if snapshot.workspace_update_applicable == Some(true) {
        execution_recoveries.push("workspace.update".to_string());
    } else if let Some(reason) = snapshot.workspace_update_applicability_reason.as_deref() {
        let recovery = if reason == "workspace_ensure_required" {
            "workspace.ensure"
        } else {
            "relaunch"
        };
        execution_recoveries.push(recovery.to_string());
    }
    execution_recoveries.sort();
    execution_recoveries.dedup();
    snapshot.available_recoveries = execution_recoveries;
    finalize_diagnosis(
        mode,
        worktree,
        session_id,
        recovery_context.as_ref(),
        snapshot,
    )
}

fn finalize_diagnosis(
    mode: ExecutionDiagnosisMode,
    worktree: &Path,
    session_id: Option<&str>,
    recovery_context: Option<
        &Result<crate::agent_project_state::ExecutionRecoveryContext, gwt_core::GwtError>,
    >,
    snapshot: ExecutionDiagnosisSnapshot,
) -> ExecutionDiagnosisSnapshot {
    match mode {
        ExecutionDiagnosisMode::OperationLocal => {
            finalize_recovery_probes(worktree, session_id, recovery_context, snapshot)
        }
        ExecutionDiagnosisMode::Projection => finalize_projection_recovery_probes(snapshot),
    }
}

fn finalize_projection_recovery_probes(
    mut snapshot: ExecutionDiagnosisSnapshot,
) -> ExecutionDiagnosisSnapshot {
    snapshot
        .available_recoveries
        .retain(|operation| !PROTECTED_RECOVERY_OPERATIONS.contains(&operation.as_str()));
    snapshot.available_recoveries.sort();
    snapshot.available_recoveries.dedup();
    snapshot.recovery_probes.clear();
    snapshot
}

fn finalize_recovery_probes(
    worktree: &Path,
    session_id: Option<&str>,
    recovery_context: Option<
        &Result<crate::agent_project_state::ExecutionRecoveryContext, gwt_core::GwtError>,
    >,
    mut snapshot: ExecutionDiagnosisSnapshot,
) -> ExecutionDiagnosisSnapshot {
    let caller = session_id.unwrap_or("");
    let ensure_candidate = crate::cli::workspace::workspace_ensure_status_candidate(caller);
    let continuation_root = recovery_context
        .and_then(|context| context.as_ref().ok())
        .map_or(worktree, |context| context.project_state_root());
    let probes = if recovery_context.is_some_and(Result::is_ok) {
        vec![
            probe_blocked_build_abort_recovery(worktree, &snapshot, session_id, recovery_context),
            probe_execution_continuation_for_recovery(continuation_root, caller, recovery_context),
            probe_execution_repair_for_recovery(worktree, session_id, recovery_context),
            probe_execution_adopt_for_recovery(worktree, caller, recovery_context),
            probe_execution_reopen_for_recovery(worktree, caller, recovery_context),
            crate::agent_project_state::probe_session_work_mutation_target(worktree, caller),
            crate::cli::workspace::probe_workspace_ensure(worktree, &ensure_candidate),
        ]
    } else {
        [
            "build.abort",
            "execution.continue",
            "execution.repair",
            "execution.adopt",
            "execution.reopen",
            "workspace.update",
            "workspace.ensure",
        ]
        .into_iter()
        .map(invalid_execution_recovery_scope_probe)
        .collect()
    };
    for probe in &probes {
        snapshot
            .available_recoveries
            .retain(|operation| operation != &probe.operation);
        if probe.advertise() {
            snapshot.available_recoveries.push(probe.operation.clone());
        }
    }
    snapshot.available_recoveries.sort();
    snapshot.available_recoveries.dedup();
    snapshot.recovery_probes = probes;
    snapshot
}

/// Replace an operation-specific terminal refusal with guidance derived from
/// the same operation-local diagnosis exposed by `execution.status`.
pub(crate) fn terminal_recovery_refusal(
    invocation_scope: &Path,
    session_id: &str,
    refusal: &str,
) -> String {
    let diagnosis = diagnose(invocation_scope, Some(session_id));
    if diagnosis.binding_state != ExecutionBindingState::Terminal {
        return refusal.to_string();
    }
    let refusal = refusal
        .split_once("; run workspace.ensure for this Session before retrying workspace.update")
        .map_or(refusal, |(reason, _)| reason);
    let available = if diagnosis.available_recoveries.is_empty() {
        "none".to_string()
    } else {
        diagnosis.available_recoveries.join(", ")
    };
    let reopen = diagnosis
        .recovery_probes
        .iter()
        .find(|probe| probe.operation == "execution.reopen")
        .and_then(|probe| probe.reason.as_deref())
        .map(|reason| format!("; recovery_probes[execution.reopen]={reason}"))
        .unwrap_or_default();
    format!(
        "{refusal}; current ecr_status={ecr_status}, binding_state=terminal; run JSON operation `execution.status` and follow its `available_recoveries` / `recovery_probes`; available_recoveries=[{available}]{reopen}",
        ecr_status = match diagnosis.ecr_status {
            ExecutionDiagnosisState::Active => "active",
            ExecutionDiagnosisState::Completed => "completed",
            ExecutionDiagnosisState::Blocked => "blocked",
            ExecutionDiagnosisState::Missing => "missing",
            ExecutionDiagnosisState::Corrupt => "corrupt",
        },
    )
}

fn probe_execution_continuation_for_recovery(
    project_state_root: &Path,
    session_id: &str,
    recovery_context: Option<
        &Result<crate::agent_project_state::ExecutionRecoveryContext, gwt_core::GwtError>,
    >,
) -> crate::cli::governance::RecoveryProbe {
    if recovery_context.is_some_and(Result::is_err) {
        return invalid_execution_recovery_scope_probe("execution.continue");
    }
    crate::agent_project_state::probe_authenticated_execution_continuation(
        project_state_root,
        session_id,
    )
}

fn probe_execution_adopt_for_recovery(
    worktree: &Path,
    session_id: &str,
    recovery_context: Option<
        &Result<crate::agent_project_state::ExecutionRecoveryContext, gwt_core::GwtError>,
    >,
) -> crate::cli::governance::RecoveryProbe {
    if recovery_context.is_some_and(Result::is_err) {
        return invalid_execution_recovery_scope_probe("execution.adopt");
    }
    if recovery_context
        .and_then(|context| context.as_ref().ok())
        .is_some_and(|context| context.exact_unbound_host())
        || crate::agent_project_state::session_requires_execution_continuation(session_id)
    {
        return crate::cli::governance::RecoveryProbe::unavailable(
            "execution.adopt",
            protected_recovery_metadata(
                Some(crate::cli::governance::GovernanceCause::Authority),
                false,
            ),
            "exact_unbound_session_requires_execution_continue",
        );
    }
    probe_execution_adopt(worktree, session_id)
}

fn probe_execution_repair_for_recovery(
    worktree: &Path,
    session_id: Option<&str>,
    recovery_context: Option<
        &Result<crate::agent_project_state::ExecutionRecoveryContext, gwt_core::GwtError>,
    >,
) -> crate::cli::governance::RecoveryProbe {
    if recovery_context.is_some_and(Result::is_err) {
        return invalid_execution_recovery_scope_probe("execution.repair");
    }
    probe_execution_repair(worktree, session_id)
}

fn probe_execution_reopen_for_recovery(
    worktree: &Path,
    session_id: &str,
    recovery_context: Option<
        &Result<crate::agent_project_state::ExecutionRecoveryContext, gwt_core::GwtError>,
    >,
) -> crate::cli::governance::RecoveryProbe {
    if recovery_context.is_some_and(Result::is_err) {
        return invalid_execution_recovery_scope_probe("execution.reopen");
    }
    probe_execution_reopen(worktree, session_id)
}

fn invalid_execution_recovery_scope_probe(
    operation: &'static str,
) -> crate::cli::governance::RecoveryProbe {
    crate::cli::governance::RecoveryProbe::unavailable(
        operation,
        protected_recovery_metadata(
            Some(crate::cli::governance::GovernanceCause::Authority),
            false,
        ),
        "execution_recovery_scope_invalid",
    )
}

fn probe_execution_repair(
    worktree: &Path,
    session_id: Option<&str>,
) -> crate::cli::governance::RecoveryProbe {
    use crate::cli::governance::{
        GovernanceCause, GovernanceEffect, GovernanceMetadata, RecoveryProbe,
    };
    let metadata = |cause| GovernanceMetadata {
        effect: Some(GovernanceEffect::Protected),
        cause,
        retryable: Some(false),
        target_state: Some("active".to_string()),
        ..GovernanceMetadata::default()
    };
    let Some(session_id) = session_id else {
        return RecoveryProbe::unavailable(
            "execution.repair",
            metadata(Some(GovernanceCause::ManagedIdentity)),
            "session_id_unavailable",
        );
    };
    let corrupt = match load(worktree) {
        Err(_) => true,
        Ok(Some(record)) if !integrity_ok(&record) => true,
        Ok(Some(record)) => {
            let owner = ExecutionOwnerKey {
                kind: record.owner_kind,
                number: record.owner_number,
            };
            load_generation_ledger(worktree, owner).is_err()
        }
        Ok(None) => false,
    };
    if !corrupt {
        return RecoveryProbe::unavailable(
            "execution.repair",
            metadata(Some(GovernanceCause::DomainInvalid)),
            "execution_repair_not_corrupt",
        );
    }
    let Some(trusted_dir) = crate::cli::trusted_store::trusted_dir_for_worktree(worktree) else {
        return RecoveryProbe::unavailable(
            "execution.repair",
            metadata(Some(GovernanceCause::Authority)),
            "execution_repair_unmanaged",
        );
    };
    match discover_repair_owner(worktree, session_id, &trusted_dir) {
        Ok(owner) => RecoveryProbe::available(
            "execution.repair",
            GovernanceMetadata {
                effect: Some(GovernanceEffect::Protected),
                cause: Some(GovernanceCause::Integrity),
                fingerprint: Some(format!("execution.repair:{}:{}", owner.number, session_id)),
                retryable: Some(true),
                target_state: Some("active".to_string()),
                ..GovernanceMetadata::default()
            },
        ),
        Err(error) => RecoveryProbe::unavailable(
            "execution.repair",
            metadata(Some(GovernanceCause::Authority)),
            error.to_string(),
        ),
    }
}

fn protected_recovery_metadata(
    cause: Option<crate::cli::governance::GovernanceCause>,
    retryable: bool,
) -> crate::cli::governance::GovernanceMetadata {
    crate::cli::governance::GovernanceMetadata {
        effect: Some(crate::cli::governance::GovernanceEffect::Protected),
        cause,
        retryable: Some(retryable),
        target_state: Some("active".to_string()),
        ..crate::cli::governance::GovernanceMetadata::default()
    }
}

#[derive(Debug, Clone)]
struct RecoveryPrerequisiteRefusal {
    cause: crate::cli::governance::GovernanceCause,
    reason: String,
}

impl RecoveryPrerequisiteRefusal {
    fn new(cause: crate::cli::governance::GovernanceCause, reason: impl Into<String>) -> Self {
        Self {
            cause,
            reason: reason.into(),
        }
    }
}

fn unavailable_recovery_prerequisite(
    cause: crate::cli::governance::GovernanceCause,
    reason: impl Into<String>,
) -> RecoveryPrerequisiteRefusal {
    RecoveryPrerequisiteRefusal::new(cause, reason)
}

fn corrupt_owner_generation_authority_refusal(
    status: ExecutionControlStatus,
) -> RecoveryPrerequisiteRefusal {
    RecoveryPrerequisiteRefusal::new(
        crate::cli::governance::GovernanceCause::Integrity,
        format!(
            "owner generation authority is corrupt or unreadable; {}",
            integrity_repair_guidance(status)
        ),
    )
}

fn recovery_record_prerequisite(
    worktree: &Path,
    missing_reason: &'static str,
) -> Result<ExecutionControlRecord, RecoveryPrerequisiteRefusal> {
    use crate::cli::governance::GovernanceCause;
    let contents = read_record_contents(worktree).map_err(|error| {
        RecoveryPrerequisiteRefusal::new(
            GovernanceCause::Integrity,
            format!("execution control record is unreadable: {error}"),
        )
    })?;
    let Some(contents) = contents else {
        return Err(RecoveryPrerequisiteRefusal::new(
            GovernanceCause::NotReady,
            missing_reason,
        ));
    };
    serde_json::from_str::<ExecutionControlRecord>(&contents)
        .map(hydrate_recovery_envelopes)
        .map_err(|error| {
            RecoveryPrerequisiteRefusal::new(
                GovernanceCause::Integrity,
                format!("execution control record is unreadable: {error}"),
            )
        })
}

fn strict_recovery_generation_binding(
    worktree: &Path,
    record: &ExecutionControlRecord,
) -> Result<Option<gwt_agent::ExecutionBindingIdentity>, RecoveryPrerequisiteRefusal> {
    let owner = ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    let corrupt_authority = || corrupt_owner_generation_authority_refusal(record.status);
    let authority_present = owner_generation_ledger_exists(worktree, owner)
        .map_err(|_| corrupt_authority())?
        || read_generation_pointer_contents(worktree)
            .map_err(|_| corrupt_authority())?
            .is_some();
    if !authority_present {
        return Ok(None);
    }
    let ledger = load_generation_ledger(worktree, owner)
        .map_err(|_| corrupt_authority())?
        .ok_or_else(corrupt_authority)?;
    let current = ledger.current_generation().ok_or_else(corrupt_authority)?;
    let projection =
        serde_json::from_str::<ExecutionControlRecord>(ledger.effective_projection_for(current))
            .map(hydrate_recovery_envelopes)
            .map_err(|_| corrupt_authority())?;
    if ledger.owner != owner
        || current.identity.worktree_binding_hash != worktree_binding_hash(worktree)
        || ledger.effective_status_for(current) != record.status
        || projection != *record
    {
        return Err(corrupt_authority());
    }
    Ok(Some(execution_binding_for_generation(&ledger, current)))
}

#[derive(Debug)]
enum ExecutionAdoptPrerequisites {
    Available {
        record: ExecutionControlRecord,
        binding: Option<gwt_agent::ExecutionBindingIdentity>,
    },
    Satisfied {
        record: ExecutionControlRecord,
        binding: Option<gwt_agent::ExecutionBindingIdentity>,
    },
}

fn evaluate_execution_adopt_prerequisites(
    worktree: &Path,
    session_id: &str,
) -> Result<ExecutionAdoptPrerequisites, RecoveryPrerequisiteRefusal> {
    use crate::cli::governance::GovernanceCause;
    if session_id.trim().is_empty() {
        return Err(RecoveryPrerequisiteRefusal::new(
            GovernanceCause::ManagedIdentity,
            "session_id_unavailable",
        ));
    }
    let record = recovery_record_prerequisite(worktree, "execution_adopt_record_missing")?;
    if !integrity_ok(&record) {
        return Err(RecoveryPrerequisiteRefusal::new(
            GovernanceCause::Integrity,
            integrity_repair_guidance(record.status),
        ));
    }
    let binding = strict_recovery_generation_binding(worktree, &record)?;
    if record.status != ExecutionControlStatus::Active {
        return Err(RecoveryPrerequisiteRefusal::new(
            GovernanceCause::DomainInvalid,
            "execution_adopt_requires_active",
        ));
    }
    if record.primary_session_id == session_id {
        Ok(ExecutionAdoptPrerequisites::Satisfied { record, binding })
    } else {
        Ok(ExecutionAdoptPrerequisites::Available { record, binding })
    }
}

fn probe_execution_adopt(
    worktree: &Path,
    session_id: &str,
) -> crate::cli::governance::RecoveryProbe {
    use crate::cli::governance::RecoveryProbe;
    match evaluate_execution_adopt_prerequisites(worktree, session_id) {
        Ok(ExecutionAdoptPrerequisites::Available { record, binding })
        | Ok(ExecutionAdoptPrerequisites::Satisfied { record, binding }) => {
            let satisfied = record.primary_session_id == session_id;
            let metadata = crate::cli::governance::GovernanceMetadata {
                fingerprint: Some(format!(
                    "execution.adopt:{}:{}:{}",
                    record.owner_number, record.primary_session_id, session_id
                )),
                execution_generation: binding.map(|binding| binding.generation_id),
                ..protected_recovery_metadata(None, true)
            };
            if satisfied {
                RecoveryProbe::satisfied("execution.adopt", metadata)
            } else {
                RecoveryProbe::available("execution.adopt", metadata)
            }
        }
        Err(refusal) => RecoveryProbe::unavailable(
            "execution.adopt",
            protected_recovery_metadata(Some(refusal.cause), false),
            refusal.reason,
        ),
    }
}

#[derive(Debug)]
enum ExecutionReopenPrerequisites {
    Available {
        record: ExecutionControlRecord,
        binding: Option<gwt_agent::ExecutionBindingIdentity>,
        blocked_at: DateTime<Utc>,
        plan: Box<crate::cli::verification_record::VerificationPlanRecord>,
        verification: Box<crate::cli::verification_record::VerificationRunRecord>,
        verification_started_at: DateTime<Utc>,
    },
    Satisfied {
        record: ExecutionControlRecord,
        binding: Option<gwt_agent::ExecutionBindingIdentity>,
    },
}

fn evaluate_execution_reopen_prerequisites(
    worktree: &Path,
    session_id: &str,
) -> Result<ExecutionReopenPrerequisites, RecoveryPrerequisiteRefusal> {
    use crate::cli::governance::GovernanceCause;
    if session_id.trim().is_empty() {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::ManagedIdentity,
            "session_id_unavailable",
        ));
    }
    let record = recovery_record_prerequisite(
        worktree,
        "no execution control record exists; start the linked owner through gwt-execute",
    )?;
    if record.content_hash.is_empty() || !integrity_ok(&record) {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::Integrity,
            "the execution control record has no valid integrity hash; use the canonical repair/fresh-launch path",
        ));
    }
    let binding = strict_recovery_generation_binding(worktree, &record)?;
    match record.status {
        ExecutionControlStatus::Active if record.primary_session_id == session_id => {
            return Ok(ExecutionReopenPrerequisites::Satisfied { record, binding });
        }
        ExecutionControlStatus::Active => {
            return Err(unavailable_recovery_prerequisite(
                GovernanceCause::Authority,
                format!(
                    "the Active record belongs to session {owner}, not the current session {current}; use the authorized ownership-transfer path",
                    owner = record.primary_session_id,
                    current = session_id,
                ),
            ));
        }
        ExecutionControlStatus::Completed => {
            return Err(unavailable_recovery_prerequisite(
                GovernanceCause::DomainInvalid,
                format!(
                    "Completed {kind} #{number} is immutable; use a fresh launch for new work",
                    kind = record.owner_kind.as_str(),
                    number = record.owner_number,
                ),
            ));
        }
        ExecutionControlStatus::Blocked => {}
    }
    if record.primary_session_id != session_id {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::Authority,
            format!(
                "the Blocked record belongs to session {owner}, not the current session {current}; use a fresh launch or the authorized ownership-transfer path",
                owner = record.primary_session_id,
                current = session_id,
            ),
        ));
    }
    let Some(blocked_at) = record.settled_at else {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::Integrity,
            "the Blocked record has no settled_at timestamp and cannot prove post-block evidence ordering",
        ));
    };
    if record
        .blocked_reason
        .as_deref()
        .is_none_or(|reason| reason.trim().is_empty())
    {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::Integrity,
            "the Blocked record has no non-empty blocker reason and cannot be recovered canonically",
        ));
    }
    use crate::cli::verification_record as vr;
    let plan = vr::load_plan(worktree)
        .map_err(|error| {
            unavailable_recovery_prerequisite(
                GovernanceCause::Integrity,
                format!(
                    "verification plan is unreadable: {error}; rerun verify.plan with params.derive:true"
                ),
            )
        })?
        .ok_or_else(|| {
            unavailable_recovery_prerequisite(
                GovernanceCause::NotReady,
                "no verification plan exists; run verify.plan with params.derive:true, then verify.run",
            )
        })?;
    if plan.content_hash.is_empty()
        || !vr::plan_integrity_ok(&plan)
        || plan.session_id != session_id
        || plan.owner_number != Some(record.owner_number)
    {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::NotReady,
            "verification plan hash/integrity/session/owner does not match the Blocked execution",
        ));
    }
    if !plan.derived {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::NotReady,
            "recovery requires a derived verification plan; run verify.plan with params.derive:true, then verify.run",
        ));
    }
    if plan.created_at <= blocked_at {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::NotReady,
            "the derived verification plan must be registered after the block; rerun verify.plan with params.derive:true",
        ));
    }
    let verification = vr::load(worktree)
        .map_err(|error| {
            unavailable_recovery_prerequisite(
                GovernanceCause::Integrity,
                format!("verification run is unreadable: {error}; rerun verify.run"),
            )
        })?
        .ok_or_else(|| {
            unavailable_recovery_prerequisite(
                GovernanceCause::NotReady,
                "no verification run record exists; run verify.run",
            )
        })?;
    if verification.content_hash.is_empty() || !vr::integrity_ok(&verification) {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::Integrity,
            "the verification run has no valid integrity hash; rerun verify.run",
        ));
    }
    let evidence_status = vr::evaluate_evidence_snapshot(
        worktree,
        session_id,
        Some(record.owner_number),
        Some(&plan),
        &verification,
    );
    if evidence_status != vr::EvidenceStatus::Fresh {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::NotReady,
            evidence_status.describe(),
        ));
    }
    if !verification.plan_derived {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::NotReady,
            "recovery requires a run bound to a derived verification plan; run verify.plan with params.derive:true, then verify.run",
        ));
    }
    let Some(verification_started_at) = verification.started_at else {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::Integrity,
            "the verification run has no trusted start timestamp; rerun verify.run after the block",
        ));
    };
    if verification_started_at <= blocked_at {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::NotReady,
            "verification must start after the block; rerun verify.run",
        ));
    }
    if verification.created_at <= blocked_at {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::NotReady,
            "verification evidence must be created after the block; rerun verify.run",
        ));
    }
    let fingerprint = vr::worktree_fingerprint_excluding(worktree, &plan.generated_outputs)
        .map_err(|error| {
            unavailable_recovery_prerequisite(
                GovernanceCause::Integrity,
                format!("generated output allowlist could not be evaluated: {error}"),
            )
        })?;
    if fingerprint != verification.worktree_fingerprint {
        return Err(unavailable_recovery_prerequisite(
            GovernanceCause::NotReady,
            "the worktree changed after verification; rerun verify.run on the final state",
        ));
    }
    Ok(ExecutionReopenPrerequisites::Available {
        record,
        binding,
        blocked_at,
        plan: Box::new(plan),
        verification: Box::new(verification),
        verification_started_at,
    })
}

fn probe_execution_reopen(
    worktree: &Path,
    session_id: &str,
) -> crate::cli::governance::RecoveryProbe {
    use crate::cli::governance::{GovernanceMetadata, RecoveryProbe};
    match evaluate_execution_reopen_prerequisites(worktree, session_id) {
        Ok(ExecutionReopenPrerequisites::Satisfied { binding, .. }) => RecoveryProbe::satisfied(
            "execution.reopen",
            GovernanceMetadata {
                execution_generation: binding.map(|binding| binding.generation_id),
                ..protected_recovery_metadata(None, true)
            },
        ),
        Ok(ExecutionReopenPrerequisites::Available {
            record,
            binding,
            plan,
            verification,
            ..
        }) => RecoveryProbe::available(
            "execution.reopen",
            GovernanceMetadata {
                fingerprint: Some(format!(
                    "execution.reopen:{}:{}:{}",
                    record.owner_number, plan.content_hash, verification.content_hash
                )),
                audit_id: Some(verification.record_id),
                execution_generation: binding.map(|binding| binding.generation_id),
                ..protected_recovery_metadata(None, true)
            },
        ),
        Err(refusal) => RecoveryProbe::unavailable(
            "execution.reopen",
            protected_recovery_metadata(Some(refusal.cause), false),
            refusal.reason,
        ),
    }
}

fn repair_audit_hash(audit: &ExecutionRepairAudit) -> String {
    let mut canonical = audit.clone();
    canonical.content_hash.clear();
    sha256_hex(serde_json::to_vec(&canonical).unwrap_or_default())
}

fn repair_audit_dir(context: &GenerationTransactionContext) -> io::Result<PathBuf> {
    let root = context.worktree_trusted_dir.parent().ok_or_else(|| {
        invalid_generation_data("trusted worktree directory has no repair audit parent")
    })?;
    Ok(root
        .join("execution-repairs")
        .join(&context.worktree_binding_hash))
}

fn load_repair_audits(audit_dir: &Path) -> io::Result<Vec<ExecutionRepairAudit>> {
    let Some(contents) =
        crate::cli::trusted_store::read_from_resolved_dir(audit_dir, EXECUTION_REPAIR_AUDIT_FILE)?
    else {
        return Ok(Vec::new());
    };
    let audits = serde_json::from_str::<Vec<ExecutionRepairAudit>>(&contents).map_err(|error| {
        invalid_generation_data(format!("malformed execution repair audit: {error}"))
    })?;
    let mut previous = "";
    for audit in &audits {
        if audit.repair_id.trim().is_empty()
            || audit.actor_session_id.trim().is_empty()
            || audit.reason.trim().is_empty()
            || audit.sources.is_empty()
            || audit.new_generation_id.trim().is_empty()
            || audit.previous_audit_hash != previous
            || audit.content_hash.is_empty()
            || audit.content_hash != repair_audit_hash(audit)
        {
            return Err(invalid_generation_data(
                "execution repair audit failed integrity validation",
            ));
        }
        previous = &audit.content_hash;
    }
    Ok(audits)
}

fn save_repair_audits(audit_dir: &Path, audits: &[ExecutionRepairAudit]) -> io::Result<()> {
    let serialized = serde_json::to_vec_pretty(audits).map_err(|error| {
        invalid_generation_data(format!("serialize execution repair audit: {error}"))
    })?;
    crate::cli::trusted_store::write_to_resolved_dir(
        audit_dir,
        EXECUTION_REPAIR_AUDIT_FILE,
        &serialized,
    )
}

fn owner_kind_from_value(value: &serde_json::Value) -> Option<ExecutionOwnerKind> {
    match value.as_str()? {
        "spec" => Some(ExecutionOwnerKind::Spec),
        "issue" => Some(ExecutionOwnerKind::Issue),
        _ => None,
    }
}

fn owner_hint_from_json(contents: &str) -> Option<ExecutionOwnerKey> {
    let value = serde_json::from_str::<serde_json::Value>(contents).ok()?;
    let owner = value.get("owner").unwrap_or(&value);
    let kind = owner_kind_from_value(owner.get("kind").or_else(|| owner.get("owner_kind"))?)?;
    let number = owner
        .get("number")
        .or_else(|| owner.get("owner_number"))?
        .as_u64()?;
    (number != 0).then_some(ExecutionOwnerKey { kind, number })
}

fn discover_repair_owner(
    worktree: &Path,
    session_id: &str,
    trusted_dir: &Path,
) -> io::Result<ExecutionOwnerKey> {
    let mut trusted_hints = Vec::new();
    for path in [
        trusted_dir.join("execution-control.json"),
        trusted_dir.join(GENERATION_POINTER_FILE),
    ] {
        match fs::read(&path) {
            Ok(contents) => {
                if let Some(owner) = std::str::from_utf8(&contents)
                    .ok()
                    .and_then(owner_hint_from_json)
                {
                    if !trusted_hints.contains(&owner) {
                        trusted_hints.push(owner);
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }

    let mut ledger_hints = Vec::new();
    let worktree_binding = worktree_binding_hash(worktree);
    let owners_root = trusted_dir
        .parent()
        .ok_or_else(|| invalid_generation_data("trusted worktree directory has no parent"))?
        .join("execution-owners");
    match fs::read_dir(owners_root) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let ledger_path = entry.path().join(GENERATION_LEDGER_FILE);
                let Some(contents) = read_optional_authority_bytes(&ledger_path)? else {
                    continue;
                };
                let Ok(ledger) = serde_json::from_slice::<ExecutionGenerationLedger>(&contents)
                else {
                    continue;
                };
                if validate_generation_ledger(&ledger, ledger.owner).is_ok()
                    && ledger.current_generation().is_some_and(|generation| {
                        generation.identity.worktree_binding_hash == worktree_binding
                    })
                    && !ledger_hints.contains(&ledger.owner)
                {
                    ledger_hints.push(ledger.owner);
                }
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let mut hints = trusted_hints;
    if hints.is_empty() {
        for path in [state_path(worktree), generation_pointer_path(worktree)] {
            match fs::read(&path) {
                Ok(contents) => {
                    if let Some(owner) = std::str::from_utf8(&contents)
                        .ok()
                        .and_then(owner_hint_from_json)
                    {
                        if !hints.contains(&owner) {
                            hints.push(owner);
                        }
                    }
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    if hints.is_empty() {
        hints = ledger_hints;
    }
    if hints.is_empty() {
        let session_path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
        if let Ok(session) = gwt_agent::Session::load(&session_path) {
            if let Some(number) = session.linked_issue_number {
                hints.push(ExecutionOwnerKey {
                    kind: detect_owner_kind(worktree, number),
                    number,
                });
            }
        }
    }
    match hints.as_slice() {
        [owner] => Ok(*owner),
        [] => Err(invalid_generation_data(
            "execution_repair_owner_unknown: corrupt authority does not expose an owner and the current Session has no linked owner",
        )),
        _ => Err(invalid_generation_data(
            "execution_repair_authority_mismatch: corrupt authority names conflicting owners",
        )),
    }
}

#[derive(Debug)]
struct RepairAuthorityExpectation {
    owner: ExecutionOwnerKey,
    projection: Option<Vec<u8>>,
    projection_mirror: Option<Vec<u8>>,
    pointer: Option<Vec<u8>>,
    pointer_mirror: Option<Vec<u8>>,
    owner_ledger: Option<Vec<u8>>,
}

fn discover_repair_authority_expectation(
    worktree: &Path,
    session_id: &str,
    trusted_dir: &Path,
) -> io::Result<RepairAuthorityExpectation> {
    let owner = discover_repair_owner(worktree, session_id, trusted_dir)?;
    let projection = read_optional_authority_bytes(&trusted_dir.join("execution-control.json"))?;
    let projection_mirror = read_optional_authority_bytes(&state_path(worktree))?;
    let pointer = read_optional_authority_bytes(&trusted_dir.join(GENERATION_POINTER_FILE))?;
    let pointer_mirror = read_optional_authority_bytes(&generation_pointer_path(worktree))?;
    let context = GenerationTransactionContext::resolve(worktree, owner)?;
    if context.worktree_trusted_dir != trusted_dir {
        return Err(generation_conflict(
            "execution repair repository identity changed during owner discovery",
        ));
    }
    let owner_ledger =
        read_optional_authority_bytes(&context.owner_dir.join(GENERATION_LEDGER_FILE))?;
    Ok(RepairAuthorityExpectation {
        owner,
        projection,
        projection_mirror,
        pointer,
        pointer_mirror,
        owner_ledger,
    })
}

fn validate_repair_authority_expectation(
    context: &GenerationTransactionContext,
    session_id: &str,
    expected: &RepairAuthorityExpectation,
) -> io::Result<()> {
    let discovered_owner =
        discover_repair_owner(&context.worktree, session_id, &context.worktree_trusted_dir)?;
    let unchanged = context.owner == expected.owner
        && discovered_owner == expected.owner
        && read_optional_authority_bytes(
            &context.worktree_trusted_dir.join("execution-control.json"),
        )? == expected.projection
        && read_optional_authority_bytes(&state_path(&context.worktree))?
            == expected.projection_mirror
        && read_optional_authority_bytes(
            &context.worktree_trusted_dir.join(GENERATION_POINTER_FILE),
        )? == expected.pointer
        && read_optional_authority_bytes(&generation_pointer_path(&context.worktree))?
            == expected.pointer_mirror
        && read_optional_authority_bytes(&context.owner_dir.join(GENERATION_LEDGER_FILE))?
            == expected.owner_ledger;
    if unchanged {
        Ok(())
    } else {
        Err(generation_conflict(
            "generation activation CAS lost: repair owner/generation authority changed after discovery",
        ))
    }
}

fn raw_execution_control_is_corrupt(contents: &[u8]) -> bool {
    serde_json::from_slice::<ExecutionControlRecord>(contents)
        .map(hydrate_recovery_envelopes)
        .map_or(true, |record| !integrity_ok(&record))
}

fn raw_pointer_is_corrupt(contents: &[u8]) -> bool {
    serde_json::from_slice::<ExecutionGenerationPointer>(contents).map_or(true, |pointer| {
        pointer.schema_version != GENERATION_LEDGER_SCHEMA_VERSION
            || pointer.content_hash.is_empty()
            || pointer.content_hash != compute_generation_pointer_hash(&pointer)
    })
}

fn read_optional_authority_bytes(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn repair_session_binding(
    worktree: &Path,
    session_id: &str,
    owner: ExecutionOwnerKey,
    identity: gwt_agent::ExecutionBindingIdentity,
    expected: &RepairSessionSnapshot,
) -> io::Result<()> {
    let repo_hash = gwt_core::repo_hash::detect_repo_hash(worktree)
        .ok_or_else(|| invalid_generation_data("execution repair repository hash is unavailable"))?
        .to_string();
    gwt_agent::update_session(
        &gwt_core::paths::gwt_sessions_dir(),
        session_id,
        move |session| {
            if session.id != expected.id
                || session.worktree_path != expected.worktree_path
                || session.project_state_root != expected.project_state_root
                || session.repo_hash != expected.repo_hash
                || session.branch != expected.branch
                || session.agent_id != expected.agent_id
                || session.linked_issue_number != expected.linked_issue_number
                || session.execution_binding != expected.execution_binding
                || dunce::canonicalize(&session.worktree_path).ok().as_deref() != Some(worktree)
            {
                return Err(io::Error::new(
                    ErrorKind::WouldBlock,
                    "durable Session changed before execution repair binding publication",
                ));
            }
            let capability_generation = session
                .execution_binding
                .as_ref()
                .map_or(1, |binding| binding.capability_generation.saturating_add(1));
            session
                .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
                    schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                    session_id: session_id.to_string(),
                    repo_hash,
                    owner_kind: owner.kind.as_str().to_string(),
                    owner_number: owner.number,
                    identity,
                    capability_generation,
                }))
                .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))
        },
    )
    .map(|_| ())
}

fn repair_session_binding_if_unchanged(
    worktree: &Path,
    expected_session: &gwt_agent::Session,
    owner: ExecutionOwnerKey,
    identity: gwt_agent::ExecutionBindingIdentity,
) -> io::Result<()> {
    match current_execution_binding(worktree, owner) {
        Ok(Some(current)) if current == identity => {}
        _ => {
            return Err(generation_conflict(
                "generation activation CAS lost: repair generation changed before Session binding",
            ))
        }
    }
    let repo_hash = gwt_core::repo_hash::detect_repo_hash(worktree)
        .ok_or_else(|| invalid_generation_data("execution repair repository hash is unavailable"))?
        .to_string();
    let mut updated = expected_session.clone();
    let capability_generation = updated
        .execution_binding
        .as_ref()
        .map_or(1, |binding| binding.capability_generation.saturating_add(1));
    updated
        .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: expected_session.id.clone(),
            repo_hash,
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation,
        }))
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    if updated.save_if_unchanged(&gwt_core::paths::gwt_sessions_dir(), expected_session)? {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "{RECOVERY_SESSION_CHANGED_PREFIX} durable Session changed before repair binding CAS"
            ),
        ))
    }
}

fn restore_quarantined_execution_authority(
    authority_paths: &[&Path],
    quarantined: &[QuarantinedExecutionAuthority],
    remove_all_authority: bool,
) -> io::Result<()> {
    let paths_to_remove = if remove_all_authority {
        authority_paths.to_vec()
    } else {
        quarantined
            .iter()
            .map(|source| source.source_path.as_path())
            .collect()
    };
    for path in paths_to_remove {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    for source in quarantined {
        fs::hard_link(&source.quarantine_path, &source.source_path)?;
        let restored = fs::read(&source.source_path)?;
        if sha256_hex(&restored) != source.source_hash {
            return Err(invalid_generation_data(
                "execution repair rollback restored bytes with the wrong hash",
            ));
        }
    }
    Ok(())
}

fn repair_error_after_audit_and_authority_restore(
    error: io::Error,
    authority_paths: &[&Path],
    quarantined: &[QuarantinedExecutionAuthority],
    audit_dir: &Path,
    previous_audit_bytes: Option<&[u8]>,
) -> io::Error {
    let authority_restore =
        restore_quarantined_execution_authority(authority_paths, quarantined, true);
    let audit_restore = match previous_audit_bytes {
        Some(bytes) => crate::cli::trusted_store::write_to_resolved_dir(
            audit_dir,
            EXECUTION_REPAIR_AUDIT_FILE,
            bytes,
        ),
        None => match fs::remove_file(audit_dir.join(EXECUTION_REPAIR_AUDIT_FILE)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
    };
    match (authority_restore, audit_restore) {
        (Ok(()), Ok(())) => error,
        (authority, audit) => io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "execution_repair_rollback_failed: {error}; authority_restore={}; audit_restore={}",
                authority
                    .err()
                    .map_or_else(|| "ok".to_string(), |failure| failure.to_string()),
                audit
                    .err()
                    .map_or_else(|| "ok".to_string(), |failure| failure.to_string()),
            ),
        ),
    }
}

struct ExecutionRepairPendingCommit {
    outcome: ExecutionRepairOutcome,
    binding: gwt_agent::ExecutionBindingIdentity,
    authority_paths: [PathBuf; 5],
    quarantined: Vec<QuarantinedExecutionAuthority>,
    audit_dir: PathBuf,
    previous_audit_bytes: Option<Vec<u8>>,
}

fn rollback_pending_execution_repair(
    pending: &ExecutionRepairPendingCommit,
    error: io::Error,
) -> io::Error {
    let authority_paths = pending
        .authority_paths
        .iter()
        .map(PathBuf::as_path)
        .collect::<Vec<_>>();
    let restored = repair_error_after_audit_and_authority_restore(
        error,
        &authority_paths,
        &pending.quarantined,
        &pending.audit_dir,
        pending.previous_audit_bytes.as_deref(),
    );
    restored
}

fn repair_error_after_partial_authority_restore(
    error: io::Error,
    authority_paths: &[&Path],
    quarantined: &[QuarantinedExecutionAuthority],
) -> io::Error {
    match restore_quarantined_execution_authority(authority_paths, quarantined, false) {
        Ok(()) => error,
        Err(restore_error) => io::Error::new(
            ErrorKind::InvalidData,
            format!("execution_repair_rollback_failed: {error}; authority_restore={restore_error}"),
        ),
    }
}

fn valid_current_generation_supersedes_repair(
    worktree: &Path,
    owner: ExecutionOwnerKey,
    repair_binding: &gwt_agent::ExecutionBindingIdentity,
) -> bool {
    match current_generation_owner(worktree) {
        Ok(Some(current_owner)) if current_owner != owner => true,
        Ok(Some(_)) => current_execution_binding(worktree, owner)
            .ok()
            .flatten()
            .is_some_and(|current| current != *repair_binding),
        Ok(None) | Err(_) => false,
    }
}

fn repair_corrupt_execution_with_session_snapshot(
    worktree: &Path,
    session_id: &str,
    expected_session: &gwt_agent::Session,
    reason: &str,
) -> io::Result<ExecutionRepairOutcome> {
    repair_corrupt_execution_impl(worktree, session_id, Some(expected_session), reason)
}

#[cfg(test)]
fn repair_corrupt_execution(
    worktree: &Path,
    session_id: &str,
    reason: &str,
) -> io::Result<ExecutionRepairOutcome> {
    repair_corrupt_execution_impl(worktree, session_id, None, reason)
}

fn repair_corrupt_execution_impl(
    worktree: &Path,
    session_id: &str,
    expected_session: Option<&gwt_agent::Session>,
    reason: &str,
) -> io::Result<ExecutionRepairOutcome> {
    if reason.trim().is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "execution.repair requires a non-empty params.reason",
        ));
    }
    let worktree = dunce::canonicalize(worktree).map_err(|error| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("execution repair worktree cannot be canonicalized: {error}"),
        )
    })?;
    let trusted_dir =
        crate::cli::trusted_store::trusted_dir_for_worktree(&worktree).ok_or_else(|| {
            io::Error::new(
                ErrorKind::InvalidInput,
                "execution_repair_unmanaged: repair requires repo-scoped trusted authority",
            )
        })?;
    let authority_expectation =
        discover_repair_authority_expectation(&worktree, session_id, &trusted_dir)?;
    let owner = authority_expectation.owner;
    let fallback_session_snapshot = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
    )
    .map(|session| RepairSessionSnapshot::from(&session))
    .map_err(|error| error.to_string());
    #[cfg(test)]
    inject_repair_owner_activation_race_if_requested(&worktree);
    with_generation_activation_leases(&worktree, owner, |context| {
        let repair = || {
            // Owner discovery happens before the activation leases so the
            // lock target can be selected. Revalidate global authority only
            // after global -> owner -> exact Session are all held; otherwise
            // a foreign generation activated in that gap could be
            // quarantined as though it were the originally observed corrupt
            // authority.
            validate_repair_authority_expectation(context, session_id, &authority_expectation)?;
            let audit_dir = repair_audit_dir(context)?;
            let previous_audit_bytes = match fs::read(audit_dir.join(EXECUTION_REPAIR_AUDIT_FILE)) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            let mut audits = load_repair_audits(&audit_dir)?;
            let ecr_path = context.worktree_trusted_dir.join("execution-control.json");
            let mirror_ecr_path = state_path(&context.worktree);
            let pointer_path = context.worktree_trusted_dir.join(GENERATION_POINTER_FILE);
            let mirror_pointer_path = generation_pointer_path(&context.worktree);
            let ledger_path = context.owner_dir.join(GENERATION_LEDGER_FILE);
            let authority_paths = [
                ecr_path.as_path(),
                mirror_ecr_path.as_path(),
                pointer_path.as_path(),
                mirror_pointer_path.as_path(),
                ledger_path.as_path(),
            ];
            let ecr = read_optional_authority_bytes(&ecr_path)?;
            let mirror_ecr = read_optional_authority_bytes(&mirror_ecr_path)?;
            let pointer = read_optional_authority_bytes(&pointer_path)?;
            let mirror_pointer = read_optional_authority_bytes(&mirror_pointer_path)?;
            let ledger = read_optional_authority_bytes(&ledger_path)?;
            let ecr_corrupt = ecr.as_deref().is_some_and(raw_execution_control_is_corrupt)
                || (ecr.is_none()
                    && mirror_ecr
                        .as_deref()
                        .is_some_and(raw_execution_control_is_corrupt));
            let pointer_corrupt = pointer.as_deref().is_some_and(raw_pointer_is_corrupt)
                || (pointer.is_none()
                    && mirror_pointer
                        .as_deref()
                        .is_some_and(raw_pointer_is_corrupt));
            let ledger_corrupt = ledger.as_deref().is_some_and(|contents| {
                serde_json::from_slice::<ExecutionGenerationLedger>(contents)
                    .map_or(true, |ledger| {
                        validate_generation_ledger(&ledger, owner).is_err()
                    })
            });
            let strict_authority_corrupt =
                if ledger.is_some() || pointer.is_some() || mirror_pointer.is_some() {
                    !matches!(load_generation_ledger_from_context(context), Ok(Some(_)))
                } else {
                    false
                };
            if !ecr_corrupt && !pointer_corrupt && !ledger_corrupt && !strict_authority_corrupt {
                return Err(io::Error::new(
                    ErrorKind::AlreadyExists,
                    "execution_repair_not_corrupt: current authority does not require repair",
                ));
            }

            let authority_snapshots = [
                (&ecr_path, ecr.as_deref()),
                (&mirror_ecr_path, mirror_ecr.as_deref()),
                (&pointer_path, pointer.as_deref()),
                (&mirror_pointer_path, mirror_pointer.as_deref()),
                (&ledger_path, ledger.as_deref()),
            ];
            let mut quarantined = Vec::new();
            for (path, expected) in authority_snapshots {
                let Some(expected) = expected else {
                    continue;
                };
                let moved = match crate::cli::trusted_store::quarantine_file(path) {
                    Ok(moved) => moved,
                    Err(error) => {
                        return Err(repair_error_after_partial_authority_restore(
                            error,
                            &authority_paths,
                            &quarantined,
                        ));
                    }
                };
                quarantined.push(QuarantinedExecutionAuthority {
                    source_path: path.clone(),
                    quarantine_path: moved.destination,
                    source_hash: moved.source_hash,
                });

                #[cfg(test)]
                if let Err(error) = fail_repair_quarantine_if_requested(quarantined.len()) {
                    return Err(repair_error_after_partial_authority_restore(
                        error,
                        &authority_paths,
                        &quarantined,
                    ));
                }

                let expected_hash = sha256_hex(expected);
                let quarantined_hash = quarantined
                    .last()
                    .and_then(|source| fs::read(&source.quarantine_path).ok())
                    .map(sha256_hex);
                if quarantined.last().map(|source| source.source_hash.as_str())
                    != Some(expected_hash.as_str())
                    || quarantined_hash.as_deref() != Some(expected_hash.as_str())
                {
                    return Err(repair_error_after_partial_authority_restore(
                        generation_conflict(
                            "execution repair authority changed while it was being quarantined",
                        ),
                        &authority_paths,
                        &quarantined,
                    ));
                }
            }
            for (path, expected) in authority_snapshots {
                if expected.is_none() {
                    let appeared = match read_optional_authority_bytes(path) {
                        Ok(contents) => contents.is_some(),
                        Err(error) => {
                            return Err(repair_error_after_partial_authority_restore(
                                error,
                                &authority_paths,
                                &quarantined,
                            ));
                        }
                    };
                    if appeared {
                        return Err(repair_error_after_partial_authority_restore(
                            generation_conflict(
                                "execution repair authority appeared after the repair snapshot",
                            ),
                            &authority_paths,
                            &quarantined,
                        ));
                    }
                }
            }
            if quarantined.is_empty() {
                return Err(io::Error::new(
                    ErrorKind::NotFound,
                    "execution_repair_source_missing: no authority file was available to quarantine",
                ));
            }

            let now = Utc::now();
            let mut record = ExecutionControlRecord {
                owner_kind: owner.kind,
                owner_number: owner.number,
                primary_session_id: session_id.to_string(),
                entrypoint: "execution.repair".to_string(),
                bundled_required_owners: Vec::new(),
                status: ExecutionControlStatus::Active,
                blocked_reason: None,
                missing_verification: None,
                launched_at: now,
                settled_at: None,
                transfers: Vec::new(),
                recoveries: Vec::new(),
                content_hash: String::new(),
            };
            let projection =
                String::from_utf8(serialize_execution_control(&record)?).map_err(|error| {
                    invalid_generation_data(format!("repair projection is not UTF-8: {error}"))
                })?;
            record = serde_json::from_str(&projection).map_err(|error| {
                invalid_generation_data(format!("repair projection is malformed: {error}"))
            })?;
            let generation_id = format!("gen-repair-{}", uuid::Uuid::new_v4().simple());
            let mut generation = ExecutionGeneration {
                identity: ExecutionGenerationIdentity {
                    owner,
                    generation_id: generation_id.clone(),
                    predecessor_generation_id: None,
                    predecessor_content_hash: None,
                    session_binding_id: format!("repair-{}", uuid::Uuid::new_v4().simple()),
                    initial_session_id: session_id.to_string(),
                    worktree_binding_hash: context.worktree_binding_hash.clone(),
                    entrypoint: record.entrypoint.clone(),
                    activated_at: now,
                },
                status: ExecutionControlStatus::Active,
                execution_control_json: projection.clone(),
                content_hash: String::new(),
            };
            generation.content_hash = compute_generation_hash(&generation);
            let mut fresh = ExecutionGenerationLedger {
                schema_version: GENERATION_LEDGER_SCHEMA_VERSION,
                owner,
                generations: vec![generation],
                continuation_attempts: Vec::new(),
                takeover_attempts: Vec::new(),
                takeovers: Vec::new(),
                lifecycle_events: Vec::new(),
                continuation_validations: Vec::new(),
                current_generation_id: generation_id.clone(),
                content_hash: String::new(),
            };
            stamp_generation_ledger(&mut fresh);
            let repair_id = format!("repair-{}", uuid::Uuid::new_v4().simple());
            let audit_sources = quarantined
                .iter()
                .map(QuarantinedExecutionAuthority::audit_source)
                .collect::<Vec<_>>();
            let mut audit = ExecutionRepairAudit {
                repair_id: repair_id.clone(),
                actor_session_id: session_id.to_string(),
                owner,
                reason: reason.to_string(),
                sources: audit_sources.clone(),
                new_generation_id: generation_id.clone(),
                repaired_at: Utc::now(),
                previous_audit_hash: audits
                    .last()
                    .map_or_else(String::new, |entry| entry.content_hash.clone()),
                content_hash: String::new(),
            };
            audit.content_hash = repair_audit_hash(&audit);
            audits.push(audit);
            #[cfg(test)]
            if let Err(error) = fail_repair_audit_write_if_requested() {
                return Err(repair_error_after_audit_and_authority_restore(
                    error,
                    &authority_paths,
                    &quarantined,
                    &audit_dir,
                    previous_audit_bytes.as_deref(),
                ));
            }
            if let Err(error) = save_repair_audits(&audit_dir, &audits) {
                return Err(repair_error_after_audit_and_authority_restore(
                    error,
                    &authority_paths,
                    &quarantined,
                    &audit_dir,
                    previous_audit_bytes.as_deref(),
                ));
            }
            let audited = match load_repair_audits(&audit_dir) {
                Ok(audited) => audited,
                Err(error) => {
                    return Err(repair_error_after_audit_and_authority_restore(
                        error,
                        &authority_paths,
                        &quarantined,
                        &audit_dir,
                        previous_audit_bytes.as_deref(),
                    ));
                }
            };
            if audited.last().map(|entry| entry.repair_id.as_str()) != Some(repair_id.as_str()) {
                return Err(repair_error_after_audit_and_authority_restore(
                    invalid_generation_data(
                        "execution_repair_audit_readback_failed: trusted repair audit is missing",
                    ),
                    &authority_paths,
                    &quarantined,
                    &audit_dir,
                    previous_audit_bytes.as_deref(),
                ));
            }

            // Commit the independent audit before materializing authority. If
            // the audit store is unavailable, the corrupt sources remain
            // preserved in quarantine and no fresh authority can become
            // active without its required audit trail.
            if let Err(error) = write_activated_generation(context, &fresh, &projection) {
                return Err(repair_error_after_audit_and_authority_restore(
                    error,
                    &authority_paths,
                    &quarantined,
                    &audit_dir,
                    previous_audit_bytes.as_deref(),
                ));
            }
            let readback = match load_generation_ledger_from_context(context) {
                Ok(Some(readback)) => readback,
                Ok(None) => {
                    return Err(repair_error_after_audit_and_authority_restore(
                        invalid_generation_data(
                            "execution_repair_readback_failed: fresh authority disappeared",
                        ),
                        &authority_paths,
                        &quarantined,
                        &audit_dir,
                        previous_audit_bytes.as_deref(),
                    ));
                }
                Err(error) => {
                    return Err(repair_error_after_audit_and_authority_restore(
                        error,
                        &authority_paths,
                        &quarantined,
                        &audit_dir,
                        previous_audit_bytes.as_deref(),
                    ));
                }
            };
            let loaded_record = match load(&worktree) {
                Ok(record) => record,
                Err(error) => {
                    return Err(repair_error_after_audit_and_authority_restore(
                        error,
                        &authority_paths,
                        &quarantined,
                        &audit_dir,
                        previous_audit_bytes.as_deref(),
                    ));
                }
            };
            if readback != fresh
                || loaded_record.as_ref() != Some(&record)
                || readback.current_effective_status() != Some(ExecutionControlStatus::Active)
            {
                return Err(repair_error_after_audit_and_authority_restore(
                    invalid_generation_data(
                        "execution_repair_readback_failed: owner ledger, ECR, and pointer disagree",
                    ),
                    &authority_paths,
                    &quarantined,
                    &audit_dir,
                    previous_audit_bytes.as_deref(),
                ));
            }
            let binding = execution_binding_for_generation(
                &readback,
                readback
                    .current_generation()
                    .expect("validated current generation"),
            );
            Ok(ExecutionRepairPendingCommit {
                outcome: ExecutionRepairOutcome {
                    status: "repaired",
                    owner,
                    generation_id,
                    repair_id,
                    quarantined: audit_sources,
                    binding_repaired: false,
                    warnings: Vec::new(),
                },
                binding,
                authority_paths: [
                    ecr_path,
                    mirror_ecr_path,
                    pointer_path,
                    mirror_pointer_path,
                    ledger_path,
                ],
                quarantined,
                audit_dir,
                previous_audit_bytes,
            })
        };
        let mut pending = match expected_session {
            Some(expected_session) => with_exact_recovery_session_lease(expected_session, repair),
            None => repair(),
        }?;

        #[cfg(test)]
        inject_repair_binding_session_race_if_requested(session_id);
        #[cfg(test)]
        inject_repair_binding_authority_race_if_requested(&worktree)?;
        let binding_repair = match expected_session {
            Some(expected_session) => repair_session_binding_if_unchanged(
                &worktree,
                expected_session,
                owner,
                pending.binding.clone(),
            ),
            None => fallback_session_snapshot
                .as_ref()
                .map_err(|error| io::Error::new(ErrorKind::NotFound, error.clone()))
                .and_then(|expected| {
                    repair_session_binding(
                        &worktree,
                        session_id,
                        owner,
                        pending.binding.clone(),
                        expected,
                    )
                }),
        };
        match (expected_session, binding_repair) {
            (_, Ok(())) => pending.outcome.binding_repaired = true,
            (Some(_), Err(error))
                if valid_current_generation_supersedes_repair(
                    &worktree,
                    owner,
                    &pending.binding,
                ) =>
            {
                return Err(error);
            }
            (Some(_), Err(error)) => {
                return Err(rollback_pending_execution_repair(&pending, error));
            }
            (None, Err(error)) => pending.outcome.warnings.push(format!(
                "execution_repair_binding_warning: fresh authority is valid, but the Session projection was not updated: {error}"
            )),
        }
        Ok(pending.outcome)
    })
}

fn run_repair(
    worktree: &Path,
    session_id: &str,
    expected_session: &gwt_agent::Session,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let probe = probe_execution_repair(worktree, Some(session_id));
    if !probe.executable() {
        out.push_str(&format!(
            "execution: repair refused — {}\n",
            probe
                .reason
                .as_deref()
                .unwrap_or("execution_repair_unavailable")
        ));
        return Ok(2);
    }
    match repair_corrupt_execution_with_session_snapshot(
        worktree,
        session_id,
        expected_session,
        reason,
    ) {
        Ok(outcome) => {
            out.push_str(&serde_json::to_string_pretty(&outcome).map_err(|error| {
                SpecOpsError::from(ApiError::Unexpected(format!(
                    "failed to serialize execution repair outcome: {error}"
                )))
            })?);
            out.push('\n');
            Ok(0)
        }
        Err(error) => {
            out.push_str(&format!("execution: repair refused — {error}\n"));
            Ok(2)
        }
    }
}

fn blocked_build_abort_guidance(record: &ExecutionControlRecord) -> String {
    format!(
        "execution: active build lifecycle remains for {kind} #{number}; run JSON operation `build.abort` with the same owner and a non-empty `params.reason` to close it\n",
        kind = record.owner_kind.as_str(),
        number = record.owner_number,
    )
}

/// Run an `execution.*` settlement command. Requires `GWT_SESSION_ID` so the
/// settlement binds to the session that owns the record.
pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: ExecutionCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let invocation_scope = env.repo_path().to_path_buf();
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    if matches!(&command, ExecutionCommand::Status) {
        let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok();
        let snapshot = diagnose(&invocation_scope, session_id.as_deref());
        out.push_str(&serde_json::to_string_pretty(&snapshot).map_err(|error| {
            SpecOpsError::from(ApiError::Unexpected(format!(
                "failed to serialize execution status: {error}"
            )))
        })?);
        out.push('\n');
        return Ok(0);
    }
    let recovery_operation = match &command {
        ExecutionCommand::Adopt { .. } => Some("adopt"),
        ExecutionCommand::Repair { .. } => Some("repair"),
        ExecutionCommand::Continue { .. } => Some("continuation"),
        ExecutionCommand::Reopen { .. } => Some("reopen"),
        _ => None,
    };
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let session_id = match session_id {
        Some(session_id) => session_id,
        None if recovery_operation.is_some() => {
            out.push_str(&format!(
                "execution: {} refused — execution recovery scope is invalid: ambient GWT_SESSION_ID is unavailable\n",
                recovery_operation.expect("protected recovery operation")
            ));
            return Ok(2);
        }
        None => {
            return Err(SpecOpsError::from(ApiError::Unexpected(
                "execution settlement requires GWT_SESSION_ID to bind to the owning session"
                    .to_string(),
            )))
        }
    };
    let recovery_context = recovery_operation.map(|_| {
        crate::agent_project_state::resolve_execution_recovery_context(
            &invocation_scope,
            &session_id,
        )
    });
    if let (Some(operation), Some(Err(error))) = (recovery_operation, recovery_context.as_ref()) {
        out.push_str(&format!(
            "execution: {operation} refused — execution recovery scope is invalid: {error}\n"
        ));
        return Ok(2);
    }
    #[cfg(test)]
    if recovery_operation.is_some() {
        inject_recovery_session_race_if_requested(&session_id);
    }
    let recovery_worktree = recovery_context
        .as_ref()
        .and_then(|context| context.as_ref().ok())
        .map_or(worktree.as_path(), |context| context.worktree());
    if let ExecutionCommand::Adopt { reason } = &command {
        let expected_session = recovery_context
            .as_ref()
            .and_then(|context| context.as_ref().ok())
            .expect("validated protected recovery context")
            .session();
        return run_adopt(
            recovery_worktree,
            &session_id,
            expected_session,
            reason,
            out,
        );
    }
    if let ExecutionCommand::Repair { reason } = &command {
        let expected_session = recovery_context
            .as_ref()
            .and_then(|context| context.as_ref().ok())
            .expect("validated protected recovery context")
            .session();
        return run_repair(
            recovery_worktree,
            &session_id,
            expected_session,
            reason,
            out,
        );
    }
    if let ExecutionCommand::Continue { operation_id } = &command {
        if recovery_context.is_none() {
            out.push_str(
                "execution: continuation refused — durable Session recovery context is unavailable; relaunch the Session\n",
            );
            return Ok(2);
        }
        let target = crate::daemon_runtime::HookForwardTarget::from_env_strict()
            .map_err(|error| SpecOpsError::from(ApiError::Unexpected(error)))?
            .ok_or_else(|| {
                SpecOpsError::from(ApiError::Unexpected(
                    "execution.continue requires the authenticated Host bridge; relaunch the Session"
                        .to_string(),
                ))
            })?;
        let request = crate::AgentExecutionContinuationRequest {
            schema_version: crate::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
            operation_id: operation_id.clone(),
        };
        return match crate::daemon_runtime::send_execution_continuation_via_agent_bridge(
            &target, &request,
        ) {
            Ok(receipt) => {
                out.push_str(&serde_json::to_string_pretty(&receipt).map_err(|error| {
                    SpecOpsError::from(ApiError::Unexpected(format!(
                        "failed to serialize execution continuation receipt: {error}"
                    )))
                })?);
                out.push('\n');
                Ok(0)
            }
            Err(error) => {
                out.push_str(&format!("execution: continuation refused — {error}\n"));
                Ok(2)
            }
        };
    }
    if let ExecutionCommand::Reopen { reason } = &command {
        let expected_session = recovery_context
            .as_ref()
            .and_then(|context| context.as_ref().ok())
            .expect("validated protected recovery context")
            .session();
        return run_reopen_with_session_snapshot(
            recovery_worktree,
            &session_id,
            expected_session,
            reason,
            out,
        );
    }
    if matches!(&command, ExecutionCommand::Complete) {
        if let Some(refusal) =
            crate::cli::verification_record::work_event_settlement_refusal(&worktree)
        {
            out.push_str(&format!("execution: completion refused — {refusal}\n"));
            return Ok(2);
        }
    }
    // P11 review fix: `execution.blocked` must defer open obligations on
    // EVERY outcome the agent reads as success — including NoRecord
    // (unlinked launches) and AlreadySettled — or the Stop gate's
    // advertised escape becomes a silent no-op.
    let mut deferral_reason: Option<String> = None;
    let result = match command {
        ExecutionCommand::Status
        | ExecutionCommand::Adopt { .. }
        | ExecutionCommand::Repair { .. }
        | ExecutionCommand::Continue { .. }
        | ExecutionCommand::Reopen { .. } => {
            unreachable!("handled above")
        }
        ExecutionCommand::Complete => {
            // T-247: completion cannot paper over unhandled prompt
            // obligations — settle or defer them first.
            if let Some(refusal) =
                crate::cli::action_obligation::open_obligation_refusal(&worktree, &session_id, &[])
            {
                out.push_str(&format!("execution: completion refused — {refusal}\n"));
                return Ok(2);
            }
            match settle_completed_with_evidence(&worktree, &session_id, None).map_err(|err| {
                SpecOpsError::from(ApiError::Unexpected(
                    crate::cli::trusted_store::store_health_error("settling execution state", &err),
                ))
            })? {
                Ok(result) => result,
                Err(status) => {
                    out.push_str(&format!(
                        "execution: completion refused — {}\n",
                        status.describe()
                    ));
                    return Ok(2);
                }
            }
        }
        ExecutionCommand::Blocked {
            reason,
            missing_verification,
        } => {
            if reason.trim().is_empty() {
                return Err(SpecOpsError::from(ApiError::Unexpected(
                    "execution.blocked requires a non-empty params.reason".to_string(),
                )));
            }
            deferral_reason = Some(reason.clone());
            settle(
                &worktree,
                &session_id,
                ExecutionSettlement::Blocked {
                    reason,
                    missing_verification,
                },
            )
            .map_err(|err| {
                SpecOpsError::from(ApiError::Unexpected(
                    crate::cli::trusted_store::store_health_error("settling execution state", &err),
                ))
            })?
        }
    };
    match result {
        SettleResult::Settled(record) => {
            if record.status == ExecutionControlStatus::Blocked {
                // P11: a terminal blocked settlement defers every open
                // obligation with the blocker on record.
                crate::cli::action_obligation::defer_all_best_effort(
                    &worktree,
                    &session_id,
                    record
                        .blocked_reason
                        .as_deref()
                        .unwrap_or("no reason recorded"),
                );
            }
            out.push_str(&format!(
                "execution: {status} for {kind} #{number} (session {session})\n",
                status = match record.status {
                    ExecutionControlStatus::Completed => "completed",
                    ExecutionControlStatus::Blocked => "blocked",
                    ExecutionControlStatus::Active => "active",
                },
                kind = record.owner_kind.as_str(),
                number = record.owner_number,
                session = record.primary_session_id,
            ));
            if record.status == ExecutionControlStatus::Blocked
                && diagnose(&invocation_scope, Some(&session_id))
                    .available_recoveries
                    .iter()
                    .any(|recovery| recovery == "build.abort")
            {
                out.push_str(&blocked_build_abort_guidance(&record));
            }
            Ok(0)
        }
        SettleResult::NoRecord => {
            if let Some(reason) = &deferral_reason {
                crate::cli::action_obligation::defer_all_best_effort(
                    &worktree,
                    &session_id,
                    reason,
                );
                out.push_str(
                    "execution: open action obligations deferred with the blocker reason\n",
                );
            }
            out.push_str(
                "execution: no execution control record for this worktree — nothing to settle\n",
            );
            Ok(0)
        }
        SettleResult::AlreadySettled(record) => {
            if let Some(reason) = &deferral_reason {
                crate::cli::action_obligation::defer_all_best_effort(
                    &worktree,
                    &session_id,
                    reason,
                );
                out.push_str(
                    "execution: open action obligations deferred with the blocker reason\n",
                );
            }
            out.push_str(&format!(
                "execution: record already settled ({status:?}) for {kind} #{number}\n",
                status = record.status,
                kind = record.owner_kind.as_str(),
                number = record.owner_number,
            ));
            if record.status == ExecutionControlStatus::Blocked
                && diagnose(&invocation_scope, Some(&session_id))
                    .available_recoveries
                    .iter()
                    .any(|recovery| recovery == "build.abort")
            {
                out.push_str(&blocked_build_abort_guidance(&record));
            }
            Ok(0)
        }
        SettleResult::SessionMismatch { record_session_id } => {
            // T-124: an unauthorized settlement attempt against an ACTIVE
            // record is bookkept as a deduped self-improvement candidate
            // (owner + violation kind). A mismatch against an already
            // settled record is a harmless retry — refused, not captured.
            let current_record = load(&worktree).ok().flatten();
            let note = match current_record.as_ref() {
                Some(record) if record.status == ExecutionControlStatus::Active => {
                    crate::cli::improvement::execution_integrity_capture_note(
                        &worktree,
                        "Execution settlement attempted by a session that does not own the record (unauthorized takeover path)",
                        &format!(
                            "{kind} #{number}: settlement session mismatch (T-124)",
                            kind = record.owner_kind.as_str(),
                            number = record.owner_number,
                        ),
                    )
                }
                _ => String::new(),
            };
            let handoff = match current_record.as_ref().map(|record| record.status) {
                Some(ExecutionControlStatus::Active) => {
                    "Take it over explicitly with JSON operation `execution.adopt` and a non-empty `params.reason` (T-117)."
                }
                Some(ExecutionControlStatus::Blocked | ExecutionControlStatus::Completed) => {
                    "A terminal record cannot be adopted; use a fresh linked-owner launch for new work."
                }
                None => "Reload the linked owner before retrying.",
            };
            out.push_str(&format!(
                "execution: settlement refused — record belongs to session {record_session_id}, not the current session. {handoff}{note}\n",
            ));
            Ok(2)
        }
        SettleResult::BindingMismatch => {
            out.push_str(
                "execution: settlement refused — the durable Session is not bound to the exact current execution generation/head; refresh or take over the current binding before retrying\n",
            );
            Ok(2)
        }
        SettleResult::Tampered => {
            let current_record = load(&worktree).ok().flatten();
            let owner = current_record
                .as_ref()
                .map(|record| {
                    format!(
                        "{kind} #{number}",
                        kind = record.owner_kind.as_str(),
                        number = record.owner_number,
                    )
                })
                .unwrap_or_else(|| "unknown owner".to_string());
            let repair = current_record
                .as_ref()
                .map_or("Reload the linked owner before retrying.", |record| {
                    integrity_repair_guidance(record.status)
                });
            let note = crate::cli::improvement::execution_integrity_capture_note(
                &worktree,
                "Execution control record failed integrity validation at settlement (edited outside the canonical operations)",
                &format!("{owner}: settlement tamper refusal (T-124)"),
            );
            out.push_str(&format!(
                "execution: settlement refused — the record failed integrity validation (edited outside the canonical operations). {repair}{note}\n",
            ));
            Ok(2)
        }
    }
}

/// FR-194..FR-196: recover a resolved terminal block without changing
/// ownership or fabricating completion. The entire decision and record write
/// is serialized by the same trusted-store lease used for settlement/adopt.
fn run_reopen_with_session_snapshot(
    worktree: &Path,
    session_id: &str,
    expected_session: &gwt_agent::Session,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    run_reopen_impl(worktree, session_id, Some(expected_session), reason, out)
}

#[cfg(test)]
fn run_reopen(
    worktree: &Path,
    session_id: &str,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    run_reopen_impl(worktree, session_id, None, reason, out)
}

fn run_reopen_impl(
    worktree: &Path,
    session_id: &str,
    expected_session: Option<&gwt_agent::Session>,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if reason.trim().is_empty() {
        return Err(SpecOpsError::from(ApiError::Unexpected(
            "execution.reopen requires a non-empty params.reason".to_string(),
        )));
    }
    let code = crate::cli::trusted_store::with_write_lease(worktree, || {
        Ok(run_reopen_locked(
            worktree,
            session_id,
            expected_session,
            reason,
            out,
        ))
    })
    .map_err(|err| {
        SpecOpsError::from(ApiError::Unexpected(
            crate::cli::trusted_store::store_health_error("settling execution state", &err),
        ))
    })??;
    // T-248 absorbed core: a real reopen revives the obligations the block
    // deferred, except the kinds the recovery evidence already covers
    // (implementation/verification are proven by the mandatory post-block
    // Fresh run). Runs after the lease — revival takes its own.
    if code == 0 && out.contains("execution: reopened") {
        let revival = crate::cli::action_obligation::revive_deferred(
            worktree,
            session_id,
            &[
                crate::cli::action_obligation::ObligationKind::IssueUpdate,
                crate::cli::action_obligation::ObligationKind::Pr,
            ],
        );
        let revival = serde_json::to_string(&revival).unwrap_or_else(|error| {
            format!(
                r#"{{"outcome":"persist_failed","error":"failed to serialize revival outcome: {error}"}}"#
            )
        });
        out.push_str(&format!(
            "execution: obligation revival outcome {revival}\n"
        ));
    }
    Ok(code)
}

fn run_reopen_locked(
    worktree: &Path,
    session_id: &str,
    expected_session: Option<&gwt_agent::Session>,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let prerequisites = match evaluate_execution_reopen_prerequisites(worktree, session_id) {
        Ok(prerequisites) => prerequisites,
        Err(refusal) => {
            out.push_str(&format!("execution: reopen refused — {}\n", refusal.reason));
            return Ok(2);
        }
    };
    let (mut record, blocked_at, plan, verification, verification_started_at) = match prerequisites
    {
        ExecutionReopenPrerequisites::Satisfied { record, binding } => {
            // Only an in-flight record with embedded recoveries needs the
            // rolling-upgrade write. A modern idempotent retry stays a true
            // no-op and cannot fail because of an unnecessary rewrite.
            let validate_and_upgrade = || {
                if recovery_storage_needs_upgrade(worktree)? && binding.is_none() {
                    save(worktree, &record)?;
                }
                Ok(())
            };
            let satisfied = match expected_session {
                Some(expected_session) => with_satisfied_recovery_session_lease(
                    worktree,
                    &record,
                    expected_session,
                    validate_and_upgrade,
                ),
                None => validate_and_upgrade(),
            };
            match satisfied {
                Ok(()) => {}
                Err(err) if err.to_string().starts_with(RECOVERY_SESSION_CHANGED_PREFIX) => {
                    out.push_str(&format!("execution: reopen refused — {err}\n"));
                    return Ok(2);
                }
                Err(err) => {
                    return Err(SpecOpsError::from(ApiError::Unexpected(
                        crate::cli::trusted_store::store_health_error(
                            "settling execution state",
                            &err,
                        ),
                    )))
                }
            }
            out.push_str(&format!(
                "execution: {kind} #{number} is already active for session {session}\n",
                kind = record.owner_kind.as_str(),
                number = record.owner_number,
                session = record.primary_session_id,
            ));
            return Ok(0);
        }
        ExecutionReopenPrerequisites::Available {
            record,
            blocked_at,
            plan,
            verification,
            verification_started_at,
            ..
        } => (
            record,
            blocked_at,
            plan,
            verification,
            verification_started_at,
        ),
    };

    let reopened_at = Utc::now();
    record.recoveries.push(ExecutionRecovery {
        session_id: session_id.to_string(),
        reason: reason.trim().to_string(),
        prior_blocked_reason: record.blocked_reason.take(),
        prior_missing_verification: record.missing_verification.take(),
        blocked_at,
        verification_record_id: verification.record_id.clone(),
        verification_run_hash: verification.content_hash.clone(),
        verification_plan_hash: plan.content_hash.clone(),
        verification_plan_created_at: plan.created_at,
        plan_derived: plan.derived,
        worktree_fingerprint: verification.worktree_fingerprint.clone(),
        verification_started_at,
        verification_created_at: verification.created_at,
        reopened_at,
        previous_recovery_hash: String::new(),
        content_hash: String::new(),
    });
    record.status = ExecutionControlStatus::Active;
    record.settled_at = None;
    let generation_update = match expected_session {
        Some(expected_session) => persist_generation_lifecycle_transition_if_owned_for_recovery(
            worktree,
            &record,
            ExecutionControlStatus::Blocked,
            reason.trim(),
            expected_session,
        ),
        None => persist_generation_lifecycle_transition_if_owned(
            worktree,
            &record,
            ExecutionControlStatus::Blocked,
            reason.trim(),
        ),
    };
    let generation_updated = match generation_update {
        Ok(updated) => updated,
        Err(err) if err.to_string().starts_with(RECOVERY_SESSION_CHANGED_PREFIX) => {
            out.push_str(&format!("execution: reopen refused — {err}\n"));
            return Ok(2);
        }
        Err(err) => {
            return Err(SpecOpsError::from(ApiError::Unexpected(
                crate::cli::trusted_store::store_health_error("settling execution state", &err),
            )))
        }
    };
    if !generation_updated {
        let owner = ExecutionOwnerKey {
            kind: record.owner_kind,
            number: record.owner_number,
        };
        let save_result = match expected_session {
            Some(expected_session) => save_legacy_recovery_record_if_session_unchanged(
                worktree,
                owner,
                expected_session,
                &record,
            ),
            None => save(worktree, &record),
        };
        match save_result {
            Ok(()) => {}
            Err(err) if err.to_string().starts_with(RECOVERY_SESSION_CHANGED_PREFIX) => {
                out.push_str(&format!("execution: reopen refused — {err}\n"));
                return Ok(2);
            }
            Err(err) => {
                return Err(SpecOpsError::from(ApiError::Unexpected(
                    crate::cli::trusted_store::store_health_error("settling execution state", &err),
                )))
            }
        }
    }
    out.push_str(&format!(
        "execution: reopened {kind} #{number} for session {session} using verification record {record_id}; completion remains pending\n",
        kind = record.owner_kind.as_str(),
        number = record.owner_number,
        session = session_id,
        record_id = verification.record_id,
    ));
    Ok(0)
}

/// `execution.adopt` (P9a, T-117): take over the worktree's record for the
/// current session with an audited transfer entry. Integrity-failed records
/// require a fresh execution lifetime: rewriting one here could canonize a
/// truncated recovery history.
fn run_adopt(
    worktree: &Path,
    session_id: &str,
    expected_session: &gwt_agent::Session,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if reason.trim().is_empty() {
        return Err(SpecOpsError::from(ApiError::Unexpected(
            "execution.adopt requires a non-empty params.reason".to_string(),
        )));
    }
    if reason.trim().starts_with(RECOVERY_ENVELOPE_PREFIX) {
        return Err(SpecOpsError::from(ApiError::Unexpected(
            "execution.adopt reason uses a reserved recovery-envelope namespace".to_string(),
        )));
    }
    if crate::agent_project_state::session_requires_execution_continuation(session_id) {
        out.push_str(
            "execution: adopt refused — exact-unbound Host Sessions must use execution.continue to bind canonical authority\n",
        );
        return Ok(2);
    }
    // T-149: adoption is a read-modify-write cycle — leased.
    crate::cli::trusted_store::with_write_lease(worktree, || {
        Ok(run_adopt_locked(
            worktree,
            session_id,
            expected_session,
            reason,
            out,
        ))
    })
    .map_err(|err| {
        SpecOpsError::from(ApiError::Unexpected(
            crate::cli::trusted_store::store_health_error("settling execution state", &err),
        ))
    })?
}

fn run_adopt_locked(
    worktree: &Path,
    session_id: &str,
    expected_session: &gwt_agent::Session,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let prerequisites = match evaluate_execution_adopt_prerequisites(worktree, session_id) {
        Ok(prerequisites) => prerequisites,
        Err(refusal) => {
            out.push_str(&format!("execution: adopt refused — {}\n", refusal.reason));
            return Ok(2);
        }
    };
    let mut record = match prerequisites {
        ExecutionAdoptPrerequisites::Satisfied { record, .. } => {
            match with_satisfied_recovery_session_lease(worktree, &record, expected_session, || {
                Ok(())
            }) {
                Ok(()) => {}
                Err(err) if err.to_string().starts_with(RECOVERY_SESSION_CHANGED_PREFIX) => {
                    out.push_str(&format!("execution: adopt refused — {err}\n"));
                    return Ok(2);
                }
                Err(err) => {
                    return Err(SpecOpsError::from(ApiError::Unexpected(
                        crate::cli::trusted_store::store_health_error(
                            "settling execution state",
                            &err,
                        ),
                    )))
                }
            }
            out.push_str("execution: the current session already owns this record\n");
            return Ok(0);
        }
        ExecutionAdoptPrerequisites::Available { record, .. } => record,
    };
    let transfer = OwnershipTransfer {
        from_session_id: record.primary_session_id.clone(),
        to_session_id: session_id.to_string(),
        reason: reason.trim().to_string(),
        transferred_at: Utc::now(),
    };
    record.transfers.push(transfer.clone());
    record.primary_session_id = session_id.to_string();
    let generation_updated = match persist_generation_takeover_if_owned_for_recovery(
        worktree,
        &record,
        &transfer,
        expected_session,
    ) {
        Ok(updated) => updated,
        Err(err) if err.to_string().starts_with(RECOVERY_SESSION_CHANGED_PREFIX) => {
            out.push_str(&format!("execution: adopt refused — {err}\n"));
            return Ok(2);
        }
        Err(err) => {
            return Err(SpecOpsError::from(ApiError::Unexpected(
                crate::cli::trusted_store::store_health_error("settling execution state", &err),
            )))
        }
    };
    if !generation_updated {
        let owner = ExecutionOwnerKey {
            kind: record.owner_kind,
            number: record.owner_number,
        };
        match save_legacy_recovery_record_if_session_unchanged(
            worktree,
            owner,
            expected_session,
            &record,
        ) {
            Ok(()) => {}
            Err(err) if err.to_string().starts_with(RECOVERY_SESSION_CHANGED_PREFIX) => {
                out.push_str(&format!("execution: adopt refused — {err}\n"));
                return Ok(2);
            }
            Err(err) => {
                return Err(SpecOpsError::from(ApiError::Unexpected(
                    crate::cli::trusted_store::store_health_error("settling execution state", &err),
                )))
            }
        }
    }
    out.push_str(&format!(
        "execution: adopted {kind} #{number} for session {session} ({transfers} transfer(s) on record)\n",
        kind = record.owner_kind.as_str(),
        number = record.owner_number,
        session = session_id,
        transfers = record.transfers.len(),
    ));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    #[test]
    fn app_runtime_exact_cleanup_never_acquires_owner_from_a_session_callback() {
        for (name, source) in [
            ("launch", include_str!("../app_runtime/launch.rs")),
            (
                "continuation",
                include_str!("../app_runtime/continuation.rs"),
            ),
        ] {
            assert!(
                !source.contains("remove_session_if_execution_identity_matches"),
                "{name} must route authority cleanup through owner-first execution_state composites",
            );
        }
    }

    #[test]
    fn app_runtime_missing_session_cleanup_uses_owner_first_composites() {
        let source = include_str!("../app_runtime/continuation.rs");
        let fresh = source
            .split_once("fn reconcile_fresh_launch_without_session")
            .unwrap()
            .1
            .split_once("fn reconcile_durable_genesis_launch")
            .unwrap()
            .0;
        assert!(fresh.contains("abort_successor_and_remove_exact_session"));
        assert!(fresh.contains("abort_successor_if_session_missing_with"));
        assert!(fresh.contains("commit_if_session_missing_with_owner_lease"));
        assert!(!fresh.contains("execution_state::abort_successor("));

        let genesis = source
            .split_once("fn reconcile_durable_genesis_launch")
            .unwrap()
            .1
            .split_once("fn finish_durable_aborted_fresh_execution_cleanup")
            .unwrap()
            .0;
        assert!(genesis.contains("block_genesis_and_remove_exact_session"));
        assert!(genesis.contains("block_genesis_if_session_missing_with"));
        assert!(!genesis.contains("block_uncommitted_genesis_launch("));

        let aborted = source
            .split_once("fn reconcile_aborted_continue_work_attempt")
            .unwrap()
            .1
            .split_once("fn reconcile_durable_continue_work_attempt")
            .unwrap()
            .0;
        assert!(aborted.contains("commit_if_session_missing_with_owner_lease"));
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct PreRecoveryControlRecord {
        owner_kind: ExecutionOwnerKind,
        owner_number: u64,
        primary_session_id: String,
        entrypoint: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bundled_required_owners: Vec<u64>,
        status: ExecutionControlStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocked_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missing_verification: Option<String>,
        launched_at: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settled_at: Option<DateTime<Utc>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        transfers: Vec<OwnershipTransfer>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        content_hash: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct InitialRecoveryControlRecord {
        owner_kind: ExecutionOwnerKind,
        owner_number: u64,
        primary_session_id: String,
        entrypoint: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bundled_required_owners: Vec<u64>,
        status: ExecutionControlStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocked_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missing_verification: Option<String>,
        launched_at: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settled_at: Option<DateTime<Utc>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        transfers: Vec<OwnershipTransfer>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        recoveries: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        content_hash: String,
    }

    fn active_record(session: &str) -> ExecutionControlRecord {
        ExecutionControlRecord {
            owner_kind: ExecutionOwnerKind::Spec,
            owner_number: 3248,
            primary_session_id: session.to_string(),
            entrypoint: "$gwt-execute".to_string(),
            bundled_required_owners: vec![3164],
            status: ExecutionControlStatus::Active,
            blocked_reason: None,
            missing_verification: None,
            launched_at: Utc::now(),
            settled_at: None,
            transfers: Vec::new(),
            recoveries: Vec::new(),
            content_hash: String::new(),
        }
    }

    fn test_recovery(session: &str, index: usize) -> ExecutionRecovery {
        let now = Utc::now();
        ExecutionRecovery {
            session_id: session.to_string(),
            reason: format!("recovery {index}"),
            prior_blocked_reason: Some(format!("blocker {index}")),
            prior_missing_verification: None,
            blocked_at: now,
            verification_record_id: format!("vrr-{index}"),
            verification_run_hash: format!("run-hash-{index}"),
            verification_plan_hash: format!("plan-hash-{index}"),
            verification_plan_created_at: now,
            plan_derived: true,
            worktree_fingerprint: format!("fingerprint-{index}"),
            verification_started_at: now,
            verification_created_at: now,
            reopened_at: now,
            previous_recovery_hash: String::new(),
            content_hash: String::new(),
        }
    }

    fn generation_owner() -> ExecutionOwnerKey {
        ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Spec,
            number: 2359,
        }
    }

    fn successor_request(operation_id: &str, principal_id: &str, source: &str) -> SuccessorRequest {
        SuccessorRequest {
            operation_id: operation_id.to_string(),
            principal_id: principal_id.to_string(),
            work_id: None,
            source: source.to_string(),
            session_binding_id: format!("binding-{operation_id}"),
            initial_session_id: format!("session-{operation_id}"),
            entrypoint: "continue-work".to_string(),
            requested_at: Utc::now(),
        }
    }

    #[test]
    fn active_generation_can_prepare_a_new_continuation_successor() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let request = successor_request("operation-active-successor", "host", "execution-continue");

        let prepared = prepare_active_continuation_successor(dir.path(), owner, &request).unwrap();

        assert_eq!(
            prepared.predecessor_status,
            SuccessorPredecessorStatus::Active
        );
        assert_eq!(prepared.status, ContinuationAttemptStatus::Prepared);
        assert_eq!(
            load(dir.path()).unwrap().unwrap().primary_session_id,
            "session-original",
            "prepare must not publish the successor before activation"
        );
    }

    #[test]
    fn exact_terminal_active_session_is_blocked_and_successor_prepared_atomically() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let session_id = "session-terminal-manual-launch";
        let mut active = active_record(session_id);
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let predecessor = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(dir.path(), owner, session_id, predecessor.clone());
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session_path = sessions_dir.join(format!("{session_id}.toml"));
        let mut terminal_session = gwt_agent::Session::load(&session_path).unwrap();
        terminal_session.update_status(gwt_agent::AgentStatus::Stopped);
        terminal_session.save(&sessions_dir).unwrap();
        let terminal_identity =
            gwt_agent::SessionExecutionIdentity::from_session(&terminal_session)
                .unwrap()
                .unwrap();
        gwt_agent::SessionRuntimeState::for_execution(
            gwt_agent::AgentStatus::Stopped,
            &terminal_identity,
            1,
        )
        .save(&gwt_agent::runtime_state_path(&sessions_dir, session_id))
        .unwrap();
        let request = successor_request(
            "manual-terminal-successor",
            "gwt-host-manual-launch",
            FRESH_LINKED_OWNER_LAUNCH_SOURCE,
        );

        let before_missing_child_proof = generation_authority_bytes(dir.path(), owner);
        let missing_child_error = prepare_exact_terminal_active_successor(
            dir.path(),
            owner,
            &request,
            &sessions_dir,
            &terminal_identity,
            gwt_agent::ManualLaunchRuntimeProof {
                host_pid: std::process::id(),
                runtime_incarnation: 1,
            },
            "exact producing runtime terminated before manual Launch Agent",
        )
        .expect_err("terminal authority without exact child identity must fail closed");
        assert_eq!(missing_child_error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(
            generation_authority_bytes(dir.path(), owner),
            before_missing_child_proof,
            "missing child identity must not mutate generation authority",
        );
        assert_eq!(
            classify_exact_session_runtime(&sessions_dir, &terminal_identity).unwrap(),
            ExactSessionRuntimeDisposition::Unknown,
            "a terminal sidecar without child identity is not an exact terminal proof",
        );
        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Stopped,
            &terminal_identity,
            1,
            crate::process::host_process_start_time(std::process::id()).unwrap(),
            i32::MAX as u32,
            1,
        )
        .save(&gwt_agent::runtime_state_path(&sessions_dir, session_id))
        .unwrap();

        let prepared = prepare_exact_terminal_active_successor(
            dir.path(),
            owner,
            &request,
            &sessions_dir,
            &terminal_identity,
            gwt_agent::ManualLaunchRuntimeProof {
                host_pid: std::process::id(),
                runtime_incarnation: 1,
            },
            "exact producing runtime terminated before manual Launch Agent",
        )
        .expect("exact terminal authority should prepare one successor");

        assert_eq!(prepared.status, ContinuationAttemptStatus::Prepared);
        assert_eq!(
            prepared.predecessor.generation_id,
            predecessor.generation_id
        );
        assert_eq!(
            load_generation_ledger(dir.path(), owner)
                .unwrap()
                .unwrap()
                .current_effective_status(),
            Some(ExecutionControlStatus::Blocked),
            "the predecessor remains current but is durably terminal before SessionStart"
        );
        assert_eq!(
            load(dir.path()).unwrap().unwrap().status,
            ExecutionControlStatus::Blocked,
            "the flat projection must be committed in the same owner transaction"
        );

        let replayed = prepare_exact_terminal_active_successor(
            dir.path(),
            owner,
            &request,
            &sessions_dir,
            &terminal_identity,
            gwt_agent::ManualLaunchRuntimeProof {
                host_pid: std::process::id(),
                runtime_incarnation: 1,
            },
            "exact producing runtime terminated before manual Launch Agent",
        )
        .expect("response-loss retry should replay the same Prepared attempt");
        assert_eq!(replayed, prepared);
        let ledger = load_generation_ledger(dir.path(), owner).unwrap().unwrap();
        assert_eq!(ledger.continuation_attempts.len(), 1);
        assert_eq!(ledger.lifecycle_events.len(), 1);

        let host_started_at = crate::process::host_process_start_time(std::process::id()).unwrap();
        let lingering_handoff = gwt_agent::with_session_lease(&sessions_dir, session_id, |_| {
            gwt_agent::begin_session_manual_handoff_under_lease(
                &sessions_dir,
                &terminal_identity,
                "response-loss-manual-handoff",
                host_started_at,
            )
        })
        .unwrap()
        .expect("recreate the fence left by a crash after owner commit");
        let blocked_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        let recovered = prepare_exact_manual_launch_successor(
            dir.path(),
            owner,
            &request,
            ExactManualLaunchPredecessor {
                sessions_dir: &sessions_dir,
                session: None,
                runtime: None,
                binding: &blocked_binding,
                status: SuccessorPredecessorStatus::Blocked,
                terminal_reason: "unused",
            },
        )
        .expect("restart replay should consume the exact post-commit manual fence");
        assert_eq!(recovered, prepared);
        gwt_agent::with_session_lease(&sessions_dir, session_id, |_| {
            assert!(!gwt_agent::session_manual_handoff_matches_under_lease(
                &sessions_dir,
                &lingering_handoff,
            )
            .unwrap());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn prepared_session_launch_claim_is_cross_process_exclusive_before_materialization() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let predecessor_session_id = "prepared-claim-predecessor";
        let mut active = active_record(predecessor_session_id);
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        settle(
            dir.path(),
            predecessor_session_id,
            ExecutionSettlement::Blocked {
                reason: "prepared claim fixture".to_string(),
                missing_verification: Some("successor pending".to_string()),
            },
        )
        .unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let request = successor_request(
            "prepared-session-launch-claim",
            "gwt-host-manual-launch",
            FRESH_LINKED_OWNER_LAUNCH_SOURCE,
        );
        let attempt =
            prepare_fresh_linked_owner_launch_successor(dir.path(), owner, &request).unwrap();
        let identity = prepared_successor_execution_binding(dir.path(), owner, &request).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut candidate =
            gwt_agent::Session::new(dir.path(), "work/issue-3547", gwt_agent::AgentId::Codex);
        candidate.id = attempt.request.initial_session_id.clone();
        candidate.project_state_root = Some(dir.path().to_path_buf());
        candidate.linked_issue_number = Some(owner.number);
        candidate
            .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: candidate.id.clone(),
                repo_hash: candidate.repo_hash.clone().unwrap(),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            }))
            .unwrap();

        let winner = claim_prepared_session_launch(dir.path(), owner, &sessions_dir, &candidate)
            .unwrap()
            .expect("the first Host must claim the Prepared candidate");
        let session_path = sessions_dir.join(format!("{}.toml", candidate.id));
        let session_bytes = fs::read(&session_path).unwrap();
        let authority_bytes = generation_authority_bytes(dir.path(), owner);

        assert!(
            claim_prepared_session_launch(dir.path(), owner, &sessions_dir, &candidate)
                .unwrap()
                .is_none(),
            "a live exact claim must reject a concurrent materializer",
        );
        assert_eq!(fs::read(&session_path).unwrap(), session_bytes);
        assert_eq!(
            generation_authority_bytes(dir.path(), owner),
            authority_bytes
        );
        assert!(finish_active_session_launch_handshake(&sessions_dir, &winner).unwrap());
    }

    #[test]
    fn abandoned_manual_handoff_with_dead_host_and_child_recovers_atomically() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let session_id = "session-abandoned-manual-handoff";
        let mut active = active_record(session_id);
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let predecessor = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(dir.path(), owner, session_id, predecessor);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session =
            gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
            .unwrap()
            .unwrap();
        let proof = gwt_agent::ManualLaunchRuntimeProof {
            host_pid: i32::MAX as u32,
            runtime_incarnation: 91,
        };
        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Running,
            &identity,
            proof.runtime_incarnation,
            1,
            (i32::MAX - 1) as u32,
            1,
        )
        .save(&gwt_agent::runtime_state_path_for_pid(
            &sessions_dir,
            proof.host_pid,
            session_id,
        ))
        .unwrap();
        let mut fence = gwt_agent::with_session_lease(&sessions_dir, session_id, |_| {
            gwt_agent::begin_session_manual_handoff_under_lease(
                &sessions_dir,
                &identity,
                "abandoned-manual-fence",
                1,
            )
        })
        .unwrap()
        .expect("create abandoned manual handoff fence");
        fence.host_pid = proof.host_pid;
        fence.host_started_at = 1;
        fs::write(
            gwt_agent::manual_handoff_path(&sessions_dir, session_id),
            serde_json::to_vec_pretty(&fence).unwrap(),
        )
        .unwrap();
        assert_eq!(
            crate::process::host_process_start_time(proof.host_pid),
            None
        );
        assert_eq!(
            crate::process::host_process_start_time((i32::MAX - 1) as u32),
            None
        );
        assert_eq!(
            classify_exact_session_runtime(&sessions_dir, &identity).unwrap(),
            ExactSessionRuntimeDisposition::Defunct(proof)
        );
        let request = successor_request(
            "recover-abandoned-manual-handoff",
            "gwt-host-manual-launch",
            FRESH_LINKED_OWNER_LAUNCH_SOURCE,
        );

        let prepared = prepare_exact_terminal_active_successor(
            dir.path(),
            owner,
            &request,
            &sessions_dir,
            &identity,
            proof,
            "exact Host and child exited during manual handoff",
        )
        .expect("dead exact Host and child should recover one successor");

        assert_eq!(prepared.status, ContinuationAttemptStatus::Prepared);
        assert_eq!(
            gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml")))
                .unwrap()
                .status,
            gwt_agent::AgentStatus::Interrupted
        );
        let runtime = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path_for_pid(
            &sessions_dir,
            proof.host_pid,
            session_id,
        ))
        .unwrap();
        assert_eq!(runtime.status, gwt_agent::AgentStatus::Interrupted);
        gwt_agent::with_session_lease(&sessions_dir, session_id, |_| {
            assert!(!gwt_agent::session_manual_handoff_matches_under_lease(
                &sessions_dir,
                &fence,
            )?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn active_launch_handshake_and_terminal_successor_are_cross_process_exclusive() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let session_id = "session-active-launch-handshake";
        let mut active = active_record(session_id);
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let predecessor = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(dir.path(), owner, session_id, predecessor);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session_path = sessions_dir.join(format!("{session_id}.toml"));
        let mut terminal_session = gwt_agent::Session::load(&session_path).unwrap();
        let identity = gwt_agent::SessionExecutionIdentity::from_session(&terminal_session)
            .unwrap()
            .unwrap();
        let handshake = begin_active_session_launch_handshake(&sessions_dir, &identity)
            .unwrap()
            .expect("Active launch should acquire the durable handshake first");
        let before_settlement = generation_authority_bytes(dir.path(), owner);
        let settle_error = settle(dir.path(), session_id, ExecutionSettlement::Completed)
            .expect_err("ordinary terminal settlement must also honor the Active launch fence");
        assert_eq!(settle_error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(
            generation_authority_bytes(dir.path(), owner),
            before_settlement
        );
        terminal_session.update_status(gwt_agent::AgentStatus::Stopped);
        terminal_session.save(&sessions_dir).unwrap();
        gwt_agent::SessionRuntimeState::for_execution(
            gwt_agent::AgentStatus::Stopped,
            &identity,
            41,
        )
        .save(&gwt_agent::runtime_state_path(&sessions_dir, session_id))
        .unwrap();
        let request = successor_request(
            "manual-handshake-race",
            "gwt-host-manual-launch",
            FRESH_LINKED_OWNER_LAUNCH_SOURCE,
        );
        let before = generation_authority_bytes(dir.path(), owner);

        let refused = prepare_exact_terminal_active_successor(
            dir.path(),
            owner,
            &request,
            &sessions_dir,
            &identity,
            gwt_agent::ManualLaunchRuntimeProof {
                host_pid: std::process::id(),
                runtime_incarnation: 41,
            },
            "exact producing runtime terminated before manual Launch Agent",
        )
        .expect_err("an in-flight Active launch handshake must fence terminal settlement");
        assert_eq!(refused.kind(), ErrorKind::PermissionDenied);
        assert_eq!(generation_authority_bytes(dir.path(), owner), before);

        assert!(finish_active_session_launch_handshake(&sessions_dir, &handshake).unwrap());
        let child_started_at = crate::process::host_process_start_time(std::process::id()).unwrap();
        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Running,
            &identity,
            41,
            child_started_at,
            std::process::id(),
            child_started_at,
        )
        .save(&gwt_agent::runtime_state_path(&sessions_dir, session_id))
        .unwrap();
        assert!(
            begin_active_session_launch_handshake(&sessions_dir, &identity)
                .unwrap()
                .is_none(),
            "a successfully published exact live runtime must fence a sequential duplicate rebound"
        );
        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Stopped,
            &identity,
            41,
            child_started_at,
            i32::MAX as u32,
            1,
        )
        .save(&gwt_agent::runtime_state_path(&sessions_dir, session_id))
        .unwrap();
        prepare_exact_terminal_active_successor(
            dir.path(),
            owner,
            &request,
            &sessions_dir,
            &identity,
            gwt_agent::ManualLaunchRuntimeProof {
                host_pid: std::process::id(),
                runtime_incarnation: 41,
            },
            "exact producing runtime terminated before manual Launch Agent",
        )
        .expect("settlement should win after the exact handshake is cleared");
        assert!(
            begin_active_session_launch_handshake(&sessions_dir, &identity)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn abandoned_active_launch_handshake_requires_absence_of_live_runtime_proof_to_recover() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let session_id = "session-abandoned-active-launch";
        let mut active = active_record(session_id);
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(dir.path(), owner, session_id, binding);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session =
            gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
            .unwrap()
            .unwrap();
        let mut abandoned = begin_active_session_launch_handshake(&sessions_dir, &identity)
            .unwrap()
            .unwrap();
        abandoned.host_pid = i32::MAX as u32;
        abandoned.host_started_at = 1;
        fs::write(
            gwt_agent::active_launch_handshake_path(&sessions_dir, session_id),
            serde_json::to_vec_pretty(&abandoned).unwrap(),
        )
        .unwrap();
        let stale_runtime_path =
            gwt_agent::runtime_state_path_for_pid(&sessions_dir, abandoned.host_pid, session_id);
        let child_started_at = crate::process::host_process_start_time(std::process::id()).unwrap();
        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Running,
            &identity,
            77,
            child_started_at,
            std::process::id(),
            child_started_at,
        )
        .save(&stale_runtime_path)
        .unwrap();

        assert!(
            begin_active_session_launch_handshake(&sessions_dir, &identity)
                .unwrap()
                .is_none(),
            "a stale Host PID is still fail-closed while exact live runtime proof remains"
        );

        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Running,
            &identity,
            77,
            child_started_at,
            std::process::id(),
            1,
        )
        .save(&stale_runtime_path)
        .unwrap();
        let recovered = begin_active_session_launch_handshake(&sessions_dir, &identity)
            .unwrap()
            .expect("dead Host plus exact terminal runtime permits audited fence recovery");
        assert_ne!(recovered.nonce, abandoned.nonce);
        assert!(finish_active_session_launch_handshake(&sessions_dir, &recovered).unwrap());
    }

    #[test]
    fn exact_terminal_active_successor_refuses_live_or_changed_session_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let session_id = "session-live-manual-launch";
        let mut active = active_record(session_id);
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let predecessor = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(dir.path(), owner, session_id, predecessor);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session_path = sessions_dir.join(format!("{session_id}.toml"));
        let live_session = gwt_agent::Session::load(&session_path).unwrap();
        let live_identity = gwt_agent::SessionExecutionIdentity::from_session(&live_session)
            .unwrap()
            .unwrap();
        gwt_agent::SessionRuntimeState::for_execution(
            gwt_agent::AgentStatus::Running,
            &live_identity,
            1,
        )
        .save(&gwt_agent::runtime_state_path(&sessions_dir, session_id))
        .unwrap();
        let request = successor_request(
            "manual-live-refusal",
            "gwt-host-manual-launch",
            FRESH_LINKED_OWNER_LAUNCH_SOURCE,
        );
        let before = generation_authority_bytes(dir.path(), owner);

        let live_error = prepare_exact_terminal_active_successor(
            dir.path(),
            owner,
            &request,
            &sessions_dir,
            &live_identity,
            gwt_agent::ManualLaunchRuntimeProof {
                host_pid: std::process::id(),
                runtime_incarnation: 1,
            },
            "exact producing runtime terminated before manual Launch Agent",
        )
        .expect_err("a live durable Session must never be fenced");
        assert_eq!(live_error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(generation_authority_bytes(dir.path(), owner), before);

        fs::write(
            gwt_agent::runtime_state_path(&sessions_dir, session_id),
            b"{malformed-runtime-proof",
        )
        .unwrap();
        let malformed_error = prepare_exact_terminal_active_successor(
            dir.path(),
            owner,
            &request,
            &sessions_dir,
            &live_identity,
            gwt_agent::ManualLaunchRuntimeProof {
                host_pid: std::process::id(),
                runtime_incarnation: 1,
            },
            "exact producing runtime terminated before manual Launch Agent",
        )
        .expect_err("malformed runtime evidence must fail closed");
        assert_eq!(malformed_error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(generation_authority_bytes(dir.path(), owner), before);

        let mut stale_identity = live_identity;
        stale_identity.execution_binding.capability_generation += 1;
        let stale_error = prepare_exact_terminal_active_successor(
            dir.path(),
            owner,
            &request,
            &sessions_dir,
            &stale_identity,
            gwt_agent::ManualLaunchRuntimeProof {
                host_pid: std::process::id(),
                runtime_incarnation: 1,
            },
            "exact producing runtime terminated before manual Launch Agent",
        )
        .expect_err("a stale Session capability epoch must fail closed");
        assert_eq!(stale_error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(generation_authority_bytes(dir.path(), owner), before);
    }

    #[test]
    fn exact_manual_launch_routes_completed_and_blocked_without_session_runtime_proof() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        for (index, (status, settlement, source)) in [
            (
                SuccessorPredecessorStatus::Completed,
                ExecutionSettlement::Completed,
                MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE,
            ),
            (
                SuccessorPredecessorStatus::Blocked,
                ExecutionSettlement::Blocked {
                    reason: "manual recovery fixture".to_string(),
                    missing_verification: None,
                },
                FRESH_LINKED_OWNER_LAUNCH_SOURCE,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let mut owner = generation_owner();
            owner.number += index as u64 + 1;
            let session_id = format!("manual-{status:?}-predecessor").to_lowercase();
            let mut active = active_record(&session_id);
            active.owner_number = owner.number;
            save(dir.path(), &active).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let binding = current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap();
            persist_generation_session_binding(dir.path(), owner, &session_id, binding.clone());
            assert!(matches!(
                settle(dir.path(), &session_id, settlement).unwrap(),
                SettleResult::Settled(_)
            ));
            let binding = current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap();
            let request = successor_request(
                &format!("manual-{status:?}-successor").to_lowercase(),
                "gwt-host-manual-launch",
                source,
            );

            let prepared = prepare_exact_manual_launch_successor(
                dir.path(),
                owner,
                &request,
                ExactManualLaunchPredecessor {
                    sessions_dir: &gwt_core::paths::gwt_sessions_dir(),
                    session: None,
                    runtime: None,
                    binding: &binding,
                    status,
                    terminal_reason: "unused for an already-terminal predecessor",
                },
            )
            .expect("terminal generation should prepare through its canonical route");

            assert_eq!(prepared.status, ContinuationAttemptStatus::Prepared);
            assert_eq!(prepared.predecessor_status, status);
            assert_eq!(prepared.request.source, source);
        }
    }

    #[test]
    fn historical_continuation_audit_survives_legal_takeover() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let original_identity = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-original".to_string(),
            repo_hash: crate::index_worker::detect_repo_hash(dir.path())
                .unwrap()
                .to_string(),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: original_identity.clone(),
            capability_generation: 1,
        };
        record_rebound_continuation_validation(
            dir.path(),
            owner,
            "historical-continuation-audit",
            "session-original",
            &binding,
        )
        .expect("record rebound validation");
        let recovery_session =
            persist_recovery_session_snapshot(dir.path(), owner, "session-successor");

        let mut out = String::new();
        assert_eq!(
            run_adopt(
                dir.path(),
                "session-successor",
                &recovery_session,
                "legal handoff after rebound validation",
                &mut out,
            )
            .expect("legal takeover must preserve historical audit validity"),
            0,
            "{out}"
        );

        let ledger = load_generation_ledger(dir.path(), owner)
            .expect("load valid post-takeover ledger")
            .expect("post-takeover ledger");
        assert!(generation_ledger_integrity_ok(&ledger));
        assert_eq!(ledger.continuation_validations.len(), 1);
        assert_eq!(
            ledger.continuation_validations[0].execution_binding,
            original_identity
        );
        assert_ne!(
            current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap(),
            original_identity,
            "the historical audit must not grant current mutation authority"
        );
    }

    #[test]
    fn historical_continuation_audit_rejects_wrong_identity_writer_and_tamper_matrix() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let identity = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        record_rebound_continuation_validation(
            dir.path(),
            owner,
            "historical-negative-matrix",
            "session-original",
            &gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: "session-original".to_string(),
                repo_hash: crate::index_worker::detect_repo_hash(dir.path())
                    .unwrap()
                    .to_string(),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            },
        )
        .unwrap();
        let valid = load_generation_ledger(dir.path(), owner).unwrap().unwrap();

        let restamp = |ledger: &mut ExecutionGenerationLedger| {
            let audit = ledger.continuation_validations.last_mut().unwrap();
            audit.content_hash = compute_continuation_validation_hash(audit);
            ledger.content_hash = compute_generation_ledger_hash(ledger);
        };
        let mut wrong_head = valid.clone();
        wrong_head
            .continuation_validations
            .last_mut()
            .unwrap()
            .execution_binding
            .ledger_head_hash = "wrong-head".to_string();
        restamp(&mut wrong_head);

        let mut wrong_writer = valid.clone();
        wrong_writer
            .continuation_validations
            .last_mut()
            .unwrap()
            .session_id = "foreign-writer".to_string();
        restamp(&mut wrong_writer);

        let mut wrong_generation = valid.clone();
        wrong_generation
            .continuation_validations
            .last_mut()
            .unwrap()
            .generation_id = "foreign-generation".to_string();
        restamp(&mut wrong_generation);

        let mut wrong_audit_hash = valid.clone();
        wrong_audit_hash
            .continuation_validations
            .last_mut()
            .unwrap()
            .content_hash = "tampered-audit-hash".to_string();
        wrong_audit_hash.content_hash = compute_generation_ledger_hash(&wrong_audit_hash);

        let mut wrong_ledger_hash = valid;
        wrong_ledger_hash.content_hash = "tampered-ledger-hash".to_string();

        for (label, ledger) in [
            ("wrong head", wrong_head),
            ("wrong writer", wrong_writer),
            ("wrong generation", wrong_generation),
            ("wrong audit hash", wrong_audit_hash),
            ("tampered ledger hash", wrong_ledger_hash),
        ] {
            assert!(
                validate_generation_ledger(&ledger, owner).is_err(),
                "{label} must never validate as historical authority"
            );
        }
    }

    fn takeover_request(operation_id: &str) -> GenerationTakeoverRequest {
        GenerationTakeoverRequest {
            operation_id: operation_id.to_string(),
            principal_id: "gwt-host-continuation".to_string(),
            work_id: Some(format!("work-{operation_id}")),
            source: Some("continue-work:resume".to_string()),
            from_session_id: "session-original".to_string(),
            to_session_id: format!("session-{operation_id}"),
            reason: "verified dead pane".to_string(),
            requested_at: Utc::now(),
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct GenerationAuthorityBytes {
        trusted_projection: Vec<u8>,
        mirror_projection: Vec<u8>,
        trusted_pointer: Vec<u8>,
        mirror_pointer: Vec<u8>,
        owner_ledger: Vec<u8>,
    }

    fn generation_authority_bytes(
        worktree: &Path,
        owner: ExecutionOwnerKey,
    ) -> GenerationAuthorityBytes {
        let context = GenerationTransactionContext::resolve(worktree, owner).unwrap();
        GenerationAuthorityBytes {
            trusted_projection: fs::read(
                context.worktree_trusted_dir.join("execution-control.json"),
            )
            .unwrap(),
            mirror_projection: fs::read(state_path(worktree)).unwrap(),
            trusted_pointer: fs::read(context.worktree_trusted_dir.join(GENERATION_POINTER_FILE))
                .unwrap(),
            mirror_pointer: fs::read(generation_pointer_path(worktree)).unwrap(),
            owner_ledger: fs::read(context.owner_dir.join(GENERATION_LEDGER_FILE)).unwrap(),
        }
    }

    fn persist_same_id_replacement(
        sessions_dir: &Path,
        worktree: &Path,
        session_id: &str,
    ) -> Vec<u8> {
        let mut replacement =
            gwt_agent::Session::new(worktree, "work/replacement", gwt_agent::AgentId::Codex);
        replacement.id = session_id.to_string();
        replacement
            .save(sessions_dir)
            .expect("materialize same-id replacement Session");
        fs::read(sessions_dir.join(format!("{session_id}.toml")))
            .expect("read replacement Session bytes")
    }

    #[test]
    fn missing_fresh_successor_cleanup_rejects_same_id_materialization_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let worktree = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let owner = generation_owner();
        let mut blocked = active_record("session-predecessor");
        blocked.owner_number = owner.number;
        blocked.status = ExecutionControlStatus::Blocked;
        blocked.blocked_reason = Some("fresh launch predecessor".to_string());
        blocked.missing_verification = Some("fresh launch verification".to_string());
        blocked.settled_at = Some(Utc::now());
        save(worktree.path(), &blocked).unwrap();
        ensure_generation_ledger(worktree.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let request = successor_request(
            "missing-fresh-race",
            "gwt-host-launch",
            FRESH_LINKED_OWNER_LAUNCH_SOURCE,
        );
        prepare_fresh_linked_owner_launch_successor(worktree.path(), owner, &request).unwrap();
        let candidate_path = sessions
            .path()
            .join(format!("{}.toml", request.initial_session_id));
        assert!(matches!(
            gwt_agent::inspect_session_path(&candidate_path),
            gwt_agent::SessionPathState::Missing
        ));
        let replacement_before = persist_same_id_replacement(
            sessions.path(),
            worktree.path(),
            &request.initial_session_id,
        );
        let authority_before = generation_authority_bytes(worktree.path(), owner);
        let committed = std::cell::Cell::new(false);

        let cleaned = abort_successor_if_session_missing_with(
            worktree.path(),
            owner,
            &request,
            "Host restarted before fresh launch Session persistence",
            sessions.path(),
            &request.initial_session_id,
            || {
                committed.set(true);
                Ok(())
            },
        )
        .unwrap();

        assert!(!cleaned);
        assert!(!committed.get());
        assert_eq!(
            generation_authority_bytes(worktree.path(), owner),
            authority_before
        );
        assert_eq!(fs::read(candidate_path).unwrap(), replacement_before);
        assert_eq!(
            continuation_attempt_for_operation(worktree.path(), owner, &request.operation_id)
                .unwrap()
                .unwrap()
                .status,
            ContinuationAttemptStatus::Prepared
        );
    }

    #[test]
    fn missing_genesis_cleanup_rejects_same_id_materialization_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let worktree = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let owner = generation_owner();
        let session_id = "missing-genesis-race";
        let mut active = active_record(session_id);
        active.owner_number = owner.number;
        save(worktree.path(), &active).unwrap();
        ensure_generation_ledger(worktree.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let binding = current_owner_execution_binding(worktree.path(), owner)
            .unwrap()
            .unwrap();
        let candidate_path = sessions.path().join(format!("{session_id}.toml"));
        assert!(matches!(
            gwt_agent::inspect_session_path(&candidate_path),
            gwt_agent::SessionPathState::Missing
        ));
        let replacement_before =
            persist_same_id_replacement(sessions.path(), worktree.path(), session_id);
        let authority_before = generation_authority_bytes(worktree.path(), owner);
        let committed = std::cell::Cell::new(false);

        let cleaned = block_genesis_if_session_missing_with(
            worktree.path(),
            owner,
            session_id,
            &binding,
            "Host restarted before genesis launch readiness",
            sessions.path(),
            || {
                committed.set(true);
                Ok(())
            },
        )
        .unwrap();

        assert!(!cleaned);
        assert!(!committed.get());
        assert_eq!(
            generation_authority_bytes(worktree.path(), owner),
            authority_before
        );
        assert_eq!(fs::read(candidate_path).unwrap(), replacement_before);
    }

    #[test]
    fn missing_work_cleanup_rejects_same_id_materialization_without_commit() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let worktree = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let owner = generation_owner();
        let session_id = "missing-aborted-work-race";
        let mut active = active_record("session-current");
        active.owner_number = owner.number;
        save(worktree.path(), &active).unwrap();
        ensure_generation_ledger(worktree.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let candidate_path = sessions.path().join(format!("{session_id}.toml"));
        assert!(matches!(
            gwt_agent::inspect_session_path(&candidate_path),
            gwt_agent::SessionPathState::Missing
        ));
        let replacement_before =
            persist_same_id_replacement(sessions.path(), worktree.path(), session_id);
        let committed = std::cell::Cell::new(false);

        let cleaned = commit_if_session_missing_with_owner_lease(
            worktree.path(),
            owner,
            sessions.path(),
            session_id,
            || {
                committed.set(true);
                Ok(())
            },
        )
        .unwrap();

        assert!(!cleaned);
        assert!(!committed.get());
        assert_eq!(fs::read(candidate_path).unwrap(), replacement_before);
    }

    fn replace_current_generation_authority(
        worktree: &Path,
        owner: ExecutionOwnerKey,
    ) -> GenerationAuthorityBytes {
        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(worktree).unwrap();
        for path in [
            trusted_dir.join("execution-control.json"),
            trusted_dir.join(GENERATION_POINTER_FILE),
            state_path(worktree),
            generation_pointer_path(worktree),
        ] {
            fs::remove_file(path).unwrap();
        }
        materialize_at_launch(
            worktree,
            owner.kind,
            owner.number,
            "session-foreign-owner",
            "launch",
            false,
        )
        .unwrap();
        ensure_generation_ledger(worktree, owner, LegacyActiveDisposition::Live).unwrap();
        generation_authority_bytes(worktree, owner)
    }

    fn persist_generation_session_binding(
        worktree: &Path,
        owner: ExecutionOwnerKey,
        session_id: &str,
        identity: gwt_agent::ExecutionBindingIdentity,
    ) {
        let branch_output = gwt_core::process::run_git_logged(
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
            Some(worktree),
        )
        .unwrap();
        assert!(branch_output.status.success());
        let branch = String::from_utf8(branch_output.stdout).unwrap();
        let mut session =
            gwt_agent::Session::new(worktree, branch.trim(), gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.project_state_root = Some(worktree.to_path_buf());
        session.repo_hash =
            crate::index_worker::detect_repo_hash(worktree).map(|repo_hash| repo_hash.to_string());
        session.linked_issue_number = Some(owner.number);
        session.execution_binding = Some(gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session_id.to_string(),
            repo_hash: session.repo_hash.clone().unwrap(),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation: 1,
        });
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
    }

    fn persist_recovery_session_snapshot(
        worktree: &Path,
        owner: ExecutionOwnerKey,
        session_id: &str,
    ) -> gwt_agent::Session {
        let branch = gwt_core::process::run_git_logged(
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
            Some(worktree),
        )
        .expect("read recovery Session branch");
        assert!(branch.status.success());
        let branch = String::from_utf8(branch.stdout).expect("UTF-8 branch");
        let mut session =
            gwt_agent::Session::new(worktree, branch.trim(), gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.project_state_root = Some(worktree.to_path_buf());
        session.repo_hash =
            crate::index_worker::detect_repo_hash(worktree).map(|repo_hash| repo_hash.to_string());
        session.linked_issue_number = Some(owner.number);
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("persist recovery Session");
        session
    }

    fn unset_live_session_env() -> Vec<ScopedEnvVar> {
        vec![
            ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV),
            ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV),
            ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV),
            ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV),
        ]
    }

    #[test]
    fn generation_authority_missing_projection_fails_closed_for_all_gates() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-original");
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();

        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();

        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(dir.path()).unwrap();
        fs::remove_file(trusted_dir.join("execution-control.json")).unwrap();
        fs::remove_file(state_path(dir.path())).unwrap();

        let loaded = load(dir.path())
            .expect("generation-owned missing projection must be represented, not propagated")
            .expect("generation authority must not be mistaken for no record");
        assert_eq!(loaded.status, ExecutionControlStatus::Active);
        assert!(!integrity_ok(&loaded));
        assert!(!is_completed(dir.path()));
        assert!(
            pr_handoff_refusal(dir.path(), false).is_some(),
            "all PR mutations must refuse an invalid generation authority"
        );
        assert_eq!(
            settle(
                dir.path(),
                "session-original",
                ExecutionSettlement::Completed
            )
            .unwrap(),
            SettleResult::Tampered
        );
    }

    #[test]
    fn generation_authority_missing_stale_pointer_does_not_revive_predecessor_projection() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let predecessor_dir = tempfile::tempdir().unwrap();
        let successor_dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(predecessor_dir.path());
        crate::cli::trusted_store::init_git_repo_with_origin(successor_dir.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(predecessor_dir.path(), &completed).unwrap();
        ensure_generation_ledger(
            predecessor_dir.path(),
            owner,
            LegacyActiveDisposition::Unknown,
        )
        .unwrap();

        let request = successor_request("operation-new-worktree", "principal-a", "continue-work");
        prepare_successor(successor_dir.path(), owner, &request).unwrap();
        activate_successor(successor_dir.path(), owner, &request).unwrap();
        let predecessor_trusted_dir =
            crate::cli::trusted_store::trusted_dir_for_worktree(predecessor_dir.path()).unwrap();
        fs::remove_file(predecessor_trusted_dir.join(GENERATION_POINTER_FILE)).unwrap();
        fs::remove_file(generation_pointer_path(predecessor_dir.path())).unwrap();

        let loaded = load(predecessor_dir.path())
            .unwrap()
            .expect("owner-ledger authority must survive a lost stale-worktree pointer");
        assert_eq!(loaded.status, ExecutionControlStatus::Active);
        assert!(
            !integrity_ok(&loaded),
            "a stale predecessor projection must fail closed instead of reviving completion"
        );
    }

    #[test]
    fn generation_authority_malformed_projection_fails_closed_for_all_gates() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-original");
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();

        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();

        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(dir.path()).unwrap();
        fs::write(
            trusted_dir.join("execution-control.json"),
            b"{\"owner_kind\":",
        )
        .unwrap();
        fs::write(state_path(dir.path()), b"{\"owner_kind\":").unwrap();

        let loaded = load(dir.path())
            .expect("generation-owned malformed projection must be represented, not propagated")
            .expect("generation authority must not be mistaken for no record");
        assert_eq!(loaded.status, ExecutionControlStatus::Active);
        assert!(!integrity_ok(&loaded));
        assert!(!is_completed(dir.path()));
        assert!(
            pr_handoff_refusal(dir.path(), false).is_some(),
            "all PR mutations must refuse malformed generation authority"
        );
    }

    #[test]
    fn generation_ledger_import_materializes_mirror_only_projection_before_pointer() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();

        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        let mirror_projection = fs::read_to_string(state_path(dir.path())).unwrap();
        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(dir.path()).unwrap();
        fs::remove_file(trusted_dir.join("execution-control.json")).unwrap();

        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();

        assert_eq!(
            fs::read_to_string(trusted_dir.join("execution-control.json")).unwrap(),
            mirror_projection,
            "import must copy exact verified bytes into the trusted projection before publishing authority"
        );
        assert!(
            load_generation_ledger(dir.path(), owner).unwrap().is_some(),
            "the imported authority must be readable immediately"
        );
    }

    #[test]
    fn generation_ledger_blocked_predecessor_cannot_prepare_successor() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut blocked = active_record("session-original");
        blocked.owner_number = owner.number;
        blocked.status = ExecutionControlStatus::Blocked;
        blocked.blocked_reason = Some("recover with execution.reopen".to_string());
        blocked.settled_at = Some(Utc::now());
        save(dir.path(), &blocked).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();

        let request = successor_request("operation-blocked", "principal-a", "quick-start");
        let error = prepare_successor(dir.path(), owner, &request).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(
            load_owner_generation_ledger(dir.path(), owner)
                .unwrap()
                .unwrap()
                .continuation_attempts
                .is_empty(),
            "Blocked remains execution.reopen-only and must not leave a Prepared attempt"
        );
    }

    #[test]
    fn generation_ledger_fresh_launch_abort_preserves_blocked_predecessor() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut blocked = active_record("session-blocked");
        blocked.owner_number = owner.number;
        blocked.status = ExecutionControlStatus::Blocked;
        blocked.blocked_reason = Some("legacy terminal blocker".to_string());
        blocked.settled_at = Some(Utc::now());
        save(dir.path(), &blocked).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let before = load_generation_ledger(dir.path(), owner)
            .unwrap()
            .expect("blocked ledger");
        let predecessor = before.current_generation().unwrap().clone();
        let predecessor_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .expect("blocked binding");
        let request = successor_request(
            "fresh-operation-abort",
            "gwt-host-launch",
            FRESH_LINKED_OWNER_LAUNCH_SOURCE,
        );

        let prepared =
            prepare_fresh_linked_owner_launch_successor(dir.path(), owner, &request).unwrap();
        assert_eq!(
            prepared.predecessor_status,
            SuccessorPredecessorStatus::Blocked
        );
        let _planned = prepared_successor_execution_binding(dir.path(), owner, &request).unwrap();
        abort_successor(dir.path(), owner, &request, "candidate readiness failed").unwrap();

        let after = load_generation_ledger(dir.path(), owner)
            .unwrap()
            .expect("ledger after abort");
        assert_eq!(after.generations, before.generations);
        assert_eq!(after.current_generation_id, before.current_generation_id);
        assert_eq!(
            after.current_generation().unwrap().execution_control_json,
            predecessor.execution_control_json,
            "abort must preserve the exact terminal predecessor projection bytes",
        );
        assert_eq!(
            current_execution_binding(dir.path(), owner).unwrap(),
            Some(predecessor_binding),
            "abort must leave the Blocked predecessor binding current",
        );
        assert_eq!(after.continuation_attempts.len(), 2);
        assert_eq!(
            after.continuation_attempts.last().unwrap().status,
            ContinuationAttemptStatus::Aborted
        );
    }

    #[test]
    fn owner_first_abort_and_exact_or_missing_session_cleanup_repeats_without_deadlock() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let sessions = tempfile::tempdir().unwrap();

        for index in 0..12 {
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                number: generation_owner().number + index,
                ..generation_owner()
            };
            let mut blocked = active_record(&format!("blocked-{index}"));
            blocked.owner_number = owner.number;
            blocked.status = ExecutionControlStatus::Blocked;
            blocked.blocked_reason = Some("terminal predecessor".to_string());
            blocked.settled_at = Some(Utc::now());
            save(dir.path(), &blocked).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
            let request = successor_request(
                &format!("owner-first-{index}"),
                "gwt-host-launch",
                FRESH_LINKED_OWNER_LAUNCH_SOURCE,
            );
            prepare_fresh_linked_owner_launch_successor(dir.path(), owner, &request).unwrap();
            let identity =
                prepared_successor_execution_binding(dir.path(), owner, &request).unwrap();
            let binding = gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: request.initial_session_id.clone(),
                repo_hash: gwt_core::repo_hash::detect_repo_hash(dir.path())
                    .unwrap()
                    .to_string(),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            };
            let mut session =
                gwt_agent::Session::new(dir.path(), "work/issue-2359", gwt_agent::AgentId::Codex);
            session.id = request.initial_session_id.clone();
            session.linked_issue_number = Some(owner.number);
            session.execution_binding = Some(binding.clone());
            session.save(sessions.path()).unwrap();
            let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
                .unwrap()
                .unwrap();
            if index % 2 == 1 {
                fs::remove_file(
                    sessions
                        .path()
                        .join(format!("{}.toml", request.initial_session_id)),
                )
                .expect("alternate iteration starts from true Missing");
            }

            assert!(
                abort_successor_and_remove_exact_session(
                    dir.path(),
                    owner,
                    &request,
                    "candidate readiness failed",
                    sessions.path(),
                    &session_identity,
                    || Ok(()),
                )
                .unwrap(),
                "iteration {index} must remove the exact candidate",
            );
            assert_eq!(
                continuation_attempt_for_operation(dir.path(), owner, &request.operation_id)
                    .unwrap()
                    .unwrap()
                    .status,
                ContinuationAttemptStatus::Aborted,
            );
        }
    }

    #[test]
    fn activated_successor_abort_conflict_does_not_run_after_abort_or_remove_session() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut blocked = active_record("activated-abort-predecessor");
        blocked.owner_number = owner.number;
        blocked.status = ExecutionControlStatus::Blocked;
        blocked.blocked_reason = Some("terminal predecessor".to_string());
        blocked.settled_at = Some(Utc::now());
        save(dir.path(), &blocked).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let request = successor_request(
            "activated-abort-conflict",
            "gwt-host-launch",
            FRESH_LINKED_OWNER_LAUNCH_SOURCE,
        );
        prepare_fresh_linked_owner_launch_successor(dir.path(), owner, &request).unwrap();
        let identity = prepared_successor_execution_binding(dir.path(), owner, &request).unwrap();
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: request.initial_session_id.clone(),
            repo_hash: gwt_core::repo_hash::detect_repo_hash(dir.path())
                .unwrap()
                .to_string(),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation: 1,
        };
        let mut session =
            gwt_agent::Session::new(dir.path(), "work/issue-2359", gwt_agent::AgentId::Codex);
        session.id = request.initial_session_id.clone();
        session.linked_issue_number = Some(owner.number);
        session.execution_binding = Some(binding.clone());
        session.save(sessions.path()).unwrap();
        let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
            .unwrap()
            .unwrap();
        activate_successor(dir.path(), owner, &request).unwrap();
        let authority_before = generation_authority_bytes(dir.path(), owner);
        let session_path = sessions.path().join(format!("{}.toml", session.id));
        let session_before = fs::read(&session_path).unwrap();
        let after_abort_ran = std::cell::Cell::new(false);

        let error = abort_successor_and_remove_exact_session(
            dir.path(),
            owner,
            &request,
            "stale failure callback",
            sessions.path(),
            &session_identity,
            || {
                after_abort_ran.set(true);
                Ok(())
            },
        )
        .expect_err("Activated successor cannot be aborted");

        assert!(error.to_string().contains("Activated"), "{error}");
        assert!(
            !after_abort_ran.get(),
            "Work rejection callback must run only after an abort commits",
        );
        assert_eq!(
            generation_authority_bytes(dir.path(), owner),
            authority_before
        );
        assert_eq!(fs::read(&session_path).unwrap(), session_before);
    }

    #[test]
    fn activated_takeover_abort_conflict_does_not_run_after_abort_or_remove_session() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let request = takeover_request("activated-takeover-abort-conflict");
        prepare_generation_takeover(dir.path(), owner, &request).unwrap();
        let identity =
            prepared_generation_takeover_execution_binding(dir.path(), owner, &request).unwrap();
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: request.to_session_id.clone(),
            repo_hash: gwt_core::repo_hash::detect_repo_hash(dir.path())
                .unwrap()
                .to_string(),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation: 1,
        };
        let mut session =
            gwt_agent::Session::new(dir.path(), "work/issue-2359", gwt_agent::AgentId::Codex);
        session.id = request.to_session_id.clone();
        session.linked_issue_number = Some(owner.number);
        session.execution_binding = Some(binding);
        session.save(sessions.path()).unwrap();
        let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
            .unwrap()
            .unwrap();
        activate_generation_takeover(dir.path(), owner, &request).unwrap();
        let authority_before = generation_authority_bytes(dir.path(), owner);
        let session_path = sessions.path().join(format!("{}.toml", session.id));
        let session_before = fs::read(&session_path).unwrap();
        let after_abort_ran = std::cell::Cell::new(false);

        let error = abort_generation_takeover_and_remove_exact_session(
            dir.path(),
            owner,
            &request,
            "stale takeover failure callback",
            sessions.path(),
            &session_identity,
            || {
                after_abort_ran.set(true);
                Ok(())
            },
        )
        .expect_err("Activated takeover cannot be aborted");

        assert!(error.to_string().contains("Activated"), "{error}");
        assert!(!after_abort_ran.get());
        assert_eq!(
            generation_authority_bytes(dir.path(), owner),
            authority_before
        );
        assert_eq!(fs::read(&session_path).unwrap(), session_before);
    }

    #[test]
    fn successful_abort_with_after_abort_error_retains_session_and_error_detail() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut blocked = active_record("callback-error-predecessor");
        blocked.owner_number = owner.number;
        blocked.status = ExecutionControlStatus::Blocked;
        blocked.blocked_reason = Some("terminal predecessor".to_string());
        blocked.settled_at = Some(Utc::now());
        save(dir.path(), &blocked).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let request = successor_request(
            "after-abort-error",
            "gwt-host-launch",
            FRESH_LINKED_OWNER_LAUNCH_SOURCE,
        );
        prepare_fresh_linked_owner_launch_successor(dir.path(), owner, &request).unwrap();
        let identity = prepared_successor_execution_binding(dir.path(), owner, &request).unwrap();
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: request.initial_session_id.clone(),
            repo_hash: gwt_core::repo_hash::detect_repo_hash(dir.path())
                .unwrap()
                .to_string(),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation: 1,
        };
        let mut session =
            gwt_agent::Session::new(dir.path(), "work/issue-2359", gwt_agent::AgentId::Codex);
        session.id = request.initial_session_id.clone();
        session.linked_issue_number = Some(owner.number);
        session.execution_binding = Some(binding);
        session.save(sessions.path()).unwrap();
        let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
            .unwrap()
            .unwrap();
        let session_path = sessions.path().join(format!("{}.toml", session.id));
        let session_before = fs::read(&session_path).unwrap();

        let error = abort_successor_and_remove_exact_session(
            dir.path(),
            owner,
            &request,
            "candidate readiness failed",
            sessions.path(),
            &session_identity,
            || Err(io::Error::other("Work rejection write failed")),
        )
        .expect_err("post-abort callback failure must remain retryable");

        assert!(error.to_string().contains("Work rejection write failed"));
        assert_eq!(
            continuation_attempt_for_operation(dir.path(), owner, &request.operation_id)
                .unwrap()
                .unwrap()
                .status,
            ContinuationAttemptStatus::Aborted,
        );
        assert_eq!(fs::read(&session_path).unwrap(), session_before);
    }

    #[test]
    fn generation_ledger_fresh_launch_activation_preserves_terminal_history_and_fences_old_binding()
    {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut blocked = active_record("session-blocked");
        blocked.owner_number = owner.number;
        blocked.status = ExecutionControlStatus::Blocked;
        blocked.blocked_reason = Some("legacy terminal blocker".to_string());
        blocked.settled_at = Some(Utc::now());
        save(dir.path(), &blocked).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let before = load_generation_ledger(dir.path(), owner)
            .unwrap()
            .expect("blocked ledger");
        let predecessor = before.current_generation().unwrap().clone();
        let predecessor_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .expect("blocked binding");
        let request = successor_request(
            "fresh-operation-activate",
            "gwt-host-launch",
            FRESH_LINKED_OWNER_LAUNCH_SOURCE,
        );
        prepare_fresh_linked_owner_launch_successor(dir.path(), owner, &request).unwrap();
        let planned = prepared_successor_execution_binding(dir.path(), owner, &request).unwrap();

        let activated = activate_successor(dir.path(), owner, &request).unwrap();

        let after = load_generation_ledger(dir.path(), owner)
            .unwrap()
            .expect("ledger after activation");
        assert_eq!(after.generations.len(), before.generations.len() + 1);
        assert_eq!(after.generations[0], predecessor);
        assert_eq!(after.current_generation_id, activated.generation_id);
        assert_eq!(
            after.current_effective_status(),
            Some(ExecutionControlStatus::Active)
        );
        assert_eq!(
            current_execution_binding(dir.path(), owner).unwrap(),
            Some(planned)
        );
        assert_ne!(
            current_execution_binding(dir.path(), owner).unwrap(),
            Some(predecessor_binding),
            "the terminal predecessor binding must be stale after fresh activation",
        );
        assert_eq!(
            after.generations[0].execution_control_json,
            before.generations[0].execution_control_json,
            "fresh activation must not rewrite the terminal predecessor snapshot",
        );
    }

    #[test]
    fn generation_ledger_prepare_replay_is_bound_to_original_worktree() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let original = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(original.path());
        crate::cli::trusted_store::init_git_repo_with_origin(other.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(original.path(), &completed).unwrap();
        ensure_generation_ledger(original.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let request = successor_request("operation-worktree", "principal-a", "quick-start");
        prepare_successor(original.path(), owner, &request).unwrap();

        let error = prepare_successor(other.path(), owner, &request).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    }

    #[test]
    fn generation_ledger_operation_id_is_bound_to_original_continue_work() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();

        let mut original =
            successor_request("operation-work", "principal-a", "continue-work:resume");
        original.work_id = Some("work-original".to_string());
        prepare_successor(dir.path(), owner, &original).unwrap();

        let mut retargeted = original.clone();
        retargeted.work_id = Some("work-foreign".to_string());
        let error = prepare_successor(dir.path(), owner, &retargeted).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            continuation_attempt_for_operation(dir.path(), owner, "operation-work")
                .unwrap()
                .unwrap()
                .request
                .work_id
                .as_deref(),
            Some("work-original")
        );
    }

    #[test]
    fn generation_ledger_activated_replay_is_bound_to_original_worktree() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let original = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(original.path());
        crate::cli::trusted_store::init_git_repo_with_origin(other.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(original.path(), &completed).unwrap();
        ensure_generation_ledger(original.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let request = successor_request("operation-activated", "principal-a", "quick-start");
        prepare_successor(original.path(), owner, &request).unwrap();
        activate_successor(original.path(), owner, &request).unwrap();

        let error = activate_successor(other.path(), owner, &request).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    }

    #[test]
    fn successor_prepared_activation_rejects_foreign_current_owner_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let foreign_owner = ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: owner.number + 1,
        };
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let request = successor_request(
            "operation-foreign-owner-prepared-successor",
            "principal-a",
            "quick-start",
        );
        prepare_successor(dir.path(), owner, &request).unwrap();
        let foreign_before = replace_current_generation_authority(dir.path(), foreign_owner);

        let error = activate_successor(dir.path(), owner, &request).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            generation_authority_bytes(dir.path(), foreign_owner),
            foreign_before,
            "a stale Prepared successor must not overwrite foreign generation authority",
        );
        assert_eq!(
            continuation_attempt_for_operation(dir.path(), owner, &request.operation_id)
                .unwrap()
                .unwrap()
                .status,
            ContinuationAttemptStatus::Prepared,
            "a rejected stale activation must leave its attempt Prepared",
        );
    }

    #[test]
    fn successor_activated_repair_rejects_foreign_current_owner_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let foreign_owner = ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: owner.number + 1,
        };
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let request = successor_request(
            "operation-foreign-owner-activated-successor",
            "principal-a",
            "quick-start",
        );
        prepare_successor(dir.path(), owner, &request).unwrap();
        activate_successor(dir.path(), owner, &request).unwrap();
        let foreign_before = replace_current_generation_authority(dir.path(), foreign_owner);

        let error = activate_successor(dir.path(), owner, &request).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            generation_authority_bytes(dir.path(), foreign_owner),
            foreign_before,
            "a stale Activated repair must not overwrite foreign generation authority",
        );
        assert_eq!(
            continuation_attempt_for_operation(dir.path(), owner, &request.operation_id)
                .unwrap()
                .unwrap()
                .status,
            ContinuationAttemptStatus::Activated,
        );
    }

    #[test]
    fn generation_ledger_activation_retry_repairs_committed_projection_and_pointer() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let request = successor_request("operation-repair", "principal-a", "quick-start");
        prepare_successor(dir.path(), owner, &request).unwrap();
        let committed = activate_successor(dir.path(), owner, &request).unwrap();

        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(dir.path()).unwrap();
        fs::remove_file(trusted_dir.join("execution-control.json")).unwrap();
        fs::remove_file(trusted_dir.join(GENERATION_POINTER_FILE)).unwrap();
        fs::remove_file(state_path(dir.path())).unwrap();
        fs::remove_file(generation_pointer_path(dir.path())).unwrap();

        assert_eq!(
            activate_successor(dir.path(), owner, &request).unwrap(),
            committed,
            "retry after a ledger commit must return the committed identity"
        );
        assert!(
            load_generation_ledger(dir.path(), owner).unwrap().is_some(),
            "the same retry must repair both projection and pointer"
        );
    }

    #[test]
    fn generation_ledger_activation_failure_after_commit_is_repaired_by_same_operation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        for (index, failure_point) in [
            GenerationWriteFailurePoint::AfterLedger,
            GenerationWriteFailurePoint::AfterProjection,
        ]
        .into_iter()
        .enumerate()
        {
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                number: generation_owner().number + index as u64,
                ..generation_owner()
            };
            let mut completed = active_record("session-original");
            completed.owner_number = owner.number;
            completed.status = ExecutionControlStatus::Completed;
            completed.settled_at = Some(Utc::now());
            save(dir.path(), &completed).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
            let request = successor_request(
                &format!("operation-injected-repair-{index}"),
                "principal-a",
                "quick-start",
            );
            prepare_successor(dir.path(), owner, &request).unwrap();

            set_generation_write_failure(failure_point);
            assert_eq!(
                activate_successor(dir.path(), owner, &request)
                    .unwrap_err()
                    .kind(),
                ErrorKind::Other
            );
            let committed = load_owner_generation_ledger(dir.path(), owner)
                .unwrap()
                .unwrap()
                .current_generation()
                .unwrap()
                .identity
                .clone();

            assert_eq!(
                activate_successor(dir.path(), owner, &request).unwrap(),
                committed
            );
            assert_eq!(
                load_generation_ledger(dir.path(), owner)
                    .unwrap()
                    .unwrap()
                    .current_generation()
                    .unwrap()
                    .identity,
                committed
            );
        }
    }

    #[test]
    fn genesis_terminalization_failure_after_ledger_commit_is_repaired_by_same_operation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        for (index, failure_point) in [
            GenerationWriteFailurePoint::AfterLedger,
            GenerationWriteFailurePoint::AfterProjection,
        ]
        .into_iter()
        .enumerate()
        {
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                number: generation_owner().number + 100 + index as u64,
                ..generation_owner()
            };
            let session_id = format!("genesis-terminalization-repair-{index}");
            let mut active = active_record(&session_id);
            active.owner_number = owner.number;
            save(dir.path(), &active).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let binding = current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap();
            let reason = "Host failed before genesis readiness";

            set_generation_write_failure(failure_point);
            assert_eq!(
                block_uncommitted_genesis_launch(dir.path(), owner, &session_id, &binding, reason,)
                    .unwrap_err()
                    .kind(),
                ErrorKind::Other,
            );
            assert_eq!(
                load_owner_generation_ledger(dir.path(), owner)
                    .unwrap()
                    .unwrap()
                    .current_effective_status(),
                Some(ExecutionControlStatus::Blocked),
                "the ledger commit is authoritative even when projection publication fails",
            );

            let repaired =
                block_uncommitted_genesis_launch(dir.path(), owner, &session_id, &binding, reason)
                    .unwrap();

            assert_eq!(repaired.status, ExecutionControlStatus::Blocked);
            let ledger = load_generation_ledger(dir.path(), owner).unwrap().unwrap();
            let event = ledger.lifecycle_events.last().unwrap();
            assert_eq!(
                event.operation_id.as_deref(),
                Some(
                    genesis_terminalization_operation_id(
                        &binding.generation_id,
                        &binding.binding_id,
                    )
                    .as_str(),
                ),
            );
        }
    }

    #[test]
    fn generation_ledger_import_failure_after_commit_is_repaired_by_same_operation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();

        set_generation_write_failure(GenerationWriteFailurePoint::AfterLedger);
        assert_eq!(
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown)
                .unwrap_err()
                .kind(),
            ErrorKind::Other
        );
        let repaired =
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();

        assert_eq!(
            load_generation_ledger(dir.path(), owner).unwrap(),
            Some(repaired)
        );
    }

    #[test]
    fn generation_ledger_rejects_noncanonical_repo_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();

        let error = ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown)
            .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(
            !dir.path()
                .join(".gwt/skill-state/execution-owners")
                .exists(),
            "canonical owner authority must never fork into a worktree-local ledger"
        );
    }

    #[test]
    fn generation_ledger_repo_retarget_during_lease_is_zero_mutation_conflict() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let original_ledger_path = generation_ledger_path(dir.path(), owner).unwrap();
        let original_ledger = fs::read(&original_ledger_path).unwrap();
        let worktree = dir.path().to_path_buf();
        crate::cli::trusted_store::set_write_lease_acquired_hook(move || {
            let status = gwt_core::process::hidden_command("git")
                .args([
                    "remote",
                    "set-url",
                    "origin",
                    "https://example.com/t/retargeted.git",
                ])
                .current_dir(worktree)
                .status()
                .unwrap();
            assert!(status.success());
        });

        let error = prepare_successor(
            dir.path(),
            owner,
            &successor_request("operation-retarget", "principal-a", "quick-start"),
        )
        .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read(original_ledger_path).unwrap(),
            original_ledger,
            "a repository identity drift must be detected before the leased ledger write"
        );
    }

    #[test]
    fn generation_ledger_owner_number_cannot_fork_by_kind() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let spec_worktree = tempfile::tempdir().unwrap();
        let issue_worktree = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(spec_worktree.path());
        crate::cli::trusted_store::init_git_repo_with_origin(issue_worktree.path());
        let spec_owner = generation_owner();
        let issue_owner = ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: spec_owner.number,
        };

        let mut spec_completed = active_record("session-spec");
        spec_completed.owner_number = spec_owner.number;
        spec_completed.status = ExecutionControlStatus::Completed;
        spec_completed.settled_at = Some(Utc::now());
        save(spec_worktree.path(), &spec_completed).unwrap();
        ensure_generation_ledger(
            spec_worktree.path(),
            spec_owner,
            LegacyActiveDisposition::Unknown,
        )
        .unwrap();

        let mut issue_completed = spec_completed;
        issue_completed.owner_kind = ExecutionOwnerKind::Issue;
        issue_completed.primary_session_id = "session-issue".to_string();
        issue_completed.content_hash.clear();
        let error = save(issue_worktree.path(), &issue_completed).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(
            generation_owner_dir(spec_worktree.path(), spec_owner).unwrap(),
            generation_owner_dir(issue_worktree.path(), issue_owner).unwrap(),
            "canonical Primary owner number, not mutable kind classification, selects the ledger path"
        );
    }

    #[test]
    fn generation_ledger_takeover_refuses_terminal_generation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let mut transferred = load(dir.path()).unwrap().unwrap();
        let transfer = OwnershipTransfer {
            from_session_id: "session-original".to_string(),
            to_session_id: "session-new".to_string(),
            reason: "terminal takeover must fail".to_string(),
            transferred_at: Utc::now(),
        };
        transferred.primary_session_id = transfer.to_session_id.clone();
        transferred.transfers.push(transfer.clone());

        let error =
            persist_generation_takeover_if_owned(dir.path(), &transferred, &transfer).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            load(dir.path()).unwrap().unwrap().primary_session_id,
            "session-original"
        );
    }

    #[test]
    fn generation_ledger_takeover_requires_contiguous_from_session_and_exact_projection() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let original = load(dir.path()).unwrap().unwrap();

        let mut wrong_from = original.clone();
        let wrong_transfer = OwnershipTransfer {
            from_session_id: "session-not-current".to_string(),
            to_session_id: "session-next".to_string(),
            reason: "wrong predecessor".to_string(),
            transferred_at: Utc::now(),
        };
        wrong_from.primary_session_id = wrong_transfer.to_session_id.clone();
        wrong_from.transfers.push(wrong_transfer.clone());
        assert_eq!(
            persist_generation_takeover_if_owned(dir.path(), &wrong_from, &wrong_transfer)
                .unwrap_err()
                .kind(),
            ErrorKind::AlreadyExists
        );

        let mut mutated = original;
        let transfer = OwnershipTransfer {
            from_session_id: "session-original".to_string(),
            to_session_id: "session-next".to_string(),
            reason: "valid predecessor".to_string(),
            transferred_at: Utc::now(),
        };
        mutated.primary_session_id = transfer.to_session_id.clone();
        mutated.entrypoint = "mutated-during-takeover".to_string();
        mutated.transfers.push(transfer.clone());
        assert_eq!(
            persist_generation_takeover_if_owned(dir.path(), &mutated, &transfer)
                .unwrap_err()
                .kind(),
            ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn generation_ledger_stale_same_session_cannot_settle_successor() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-reused");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        assert!(matches!(
            settle(dir.path(), "session-reused", ExecutionSettlement::Completed).unwrap(),
            SettleResult::Settled(_)
        ));
        let predecessor_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(
            dir.path(),
            owner,
            "session-reused",
            predecessor_binding,
        );
        let mut request =
            successor_request("operation-reused-session", "principal-a", "quick-start");
        request.initial_session_id = "session-reused".to_string();
        prepare_successor(dir.path(), owner, &request).unwrap();
        activate_successor(dir.path(), owner, &request).unwrap();

        let stale_result = settle(
            dir.path(),
            "session-reused",
            ExecutionSettlement::Blocked {
                reason: "stale predecessor request".to_string(),
                missing_verification: None,
            },
        )
        .unwrap();

        assert!(
            !matches!(stale_result, SettleResult::Settled(_)),
            "session id reuse must not authorize a predecessor binding to settle its successor"
        );
        assert_eq!(
            load_generation_ledger(dir.path(), owner)
                .unwrap()
                .unwrap()
                .current_effective_status(),
            Some(ExecutionControlStatus::Active)
        );
        let successor_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(dir.path(), owner, "session-reused", successor_binding);
        assert!(matches!(
            settle(
                dir.path(),
                "session-reused",
                ExecutionSettlement::Blocked {
                    reason: "current successor request".to_string(),
                    missing_verification: None,
                },
            )
            .unwrap(),
            SettleResult::Settled(_)
        ));
    }

    #[test]
    fn generation_ledger_imports_verified_completed_and_preserves_terminal_bytes() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();

        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        let terminal_bytes = crate::cli::trusted_store::read(dir.path(), "execution-control.json")
            .unwrap()
            .unwrap();

        let ledger =
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        assert_eq!(ledger.schema_version, 1);
        assert_eq!(ledger.owner, owner);
        assert_eq!(ledger.generations.len(), 1);
        assert_eq!(
            ledger.current_generation_id,
            ledger.generations[0].identity.generation_id
        );
        assert_eq!(
            ledger.generations[0].status,
            ExecutionControlStatus::Completed
        );
        assert_eq!(ledger.generations[0].execution_control_json, terminal_bytes);
        assert!(generation_ledger_integrity_ok(&ledger));
        assert_eq!(
            crate::cli::trusted_store::read(dir.path(), "execution-control.json")
                .unwrap()
                .unwrap(),
            terminal_bytes,
            "terminal projection must remain byte-for-byte unchanged during import"
        );

        let identity = current_generation_identity(dir.path(), owner)
            .unwrap()
            .unwrap();
        assert_eq!(identity, ledger.generations[0].identity);
        assert!(
            generation_ledger_path(dir.path(), owner)
                .unwrap()
                .starts_with(home.path()),
            "git-backed owner ledger must live in the repo-scoped trusted store"
        );
    }

    #[test]
    fn generation_ledger_imports_verified_blocked_for_reopen_compatibility() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut blocked = active_record("session-original");
        blocked.owner_number = owner.number;
        blocked.status = ExecutionControlStatus::Blocked;
        blocked.blocked_reason = Some("verification dependency unresolved".to_string());
        blocked.missing_verification = Some("derived matrix".to_string());
        blocked.settled_at = Some(Utc::now());
        save(dir.path(), &blocked).unwrap();
        let terminal_bytes = crate::cli::trusted_store::read(dir.path(), "execution-control.json")
            .unwrap()
            .unwrap();

        let ledger =
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        assert_eq!(ledger.generations.len(), 1);
        assert_eq!(
            ledger.current_effective_status(),
            Some(ExecutionControlStatus::Blocked)
        );
        assert_eq!(
            ledger.current_generation().unwrap().execution_control_json,
            terminal_bytes
        );
        assert_eq!(
            load_generation_ledger(dir.path(), owner).unwrap().unwrap(),
            ledger
        );
    }

    #[test]
    fn generation_ledger_prepared_and_aborted_attempts_never_become_current() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        let imported =
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();
        let predecessor = imported.generations[0].identity.clone();

        let request = successor_request("operation-prepare", "principal-a", "quick-start");
        let prepared = prepare_successor(dir.path(), owner, &request).unwrap();
        assert_eq!(prepared.status, ContinuationAttemptStatus::Prepared);
        assert_eq!(prepared.predecessor, predecessor);
        let after_prepare = load_generation_ledger(dir.path(), owner).unwrap().unwrap();
        assert_eq!(after_prepare.generations.len(), 1);
        assert_eq!(
            after_prepare.current_generation_id,
            predecessor.generation_id
        );

        let aborted =
            abort_successor(dir.path(), owner, &request, "candidate launch failed").unwrap();
        assert_eq!(aborted.status, ContinuationAttemptStatus::Aborted);
        let after_abort = load_generation_ledger(dir.path(), owner).unwrap().unwrap();
        assert_eq!(after_abort.generations.len(), 1);
        assert_eq!(after_abort.current_generation_id, predecessor.generation_id);
        assert_eq!(
            after_abort
                .continuation_attempts
                .iter()
                .map(|attempt| attempt.status)
                .collect::<Vec<_>>(),
            vec![
                ContinuationAttemptStatus::Prepared,
                ContinuationAttemptStatus::Aborted
            ]
        );

        let mut conflicting = request.clone();
        conflicting.principal_id = "principal-b".to_string();
        let conflict = prepare_successor(dir.path(), owner, &conflicting).unwrap_err();
        assert_eq!(conflict.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            load_generation_ledger(dir.path(), owner)
                .unwrap()
                .unwrap()
                .current_generation_id,
            predecessor.generation_id
        );
    }

    #[test]
    fn prepared_successor_binding_predicts_exact_activated_authority() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();

        let request =
            successor_request("operation-planned-binding", "principal-a", "continue-work");
        prepare_successor(dir.path(), owner, &request).unwrap();
        let predecessor_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        let planned = prepared_successor_execution_binding(dir.path(), owner, &request).unwrap();
        assert_ne!(planned, predecessor_binding);
        assert!(prepared_execution_binding_matches(
            dir.path(),
            owner,
            &request.initial_session_id,
            &planned,
        )
        .unwrap());
        assert!(!prepared_execution_binding_matches(
            dir.path(),
            owner,
            "another-session",
            &planned,
        )
        .unwrap());
        let mut mismatched = planned.clone();
        mismatched.ledger_head_hash.push_str("-mismatch");
        assert!(!prepared_execution_binding_matches(
            dir.path(),
            owner,
            &request.initial_session_id,
            &mismatched,
        )
        .unwrap());

        activate_successor(dir.path(), owner, &request).unwrap();
        assert_eq!(
            current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap(),
            planned,
            "Prepared capability identity must equal the post-CAS Active identity byte-for-byte"
        );
        assert!(
            !prepared_execution_binding_matches(
                dir.path(),
                owner,
                &request.initial_session_id,
                &planned,
            )
            .unwrap(),
            "Activated attempts are no longer Prepared authority"
        );
    }

    #[test]
    fn prepared_takeover_binding_predicts_exact_same_generation_authority() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        let imported =
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let generation_id = imported.current_generation_id.clone();
        let predecessor_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        let request = GenerationTakeoverRequest {
            operation_id: "operation-takeover".to_string(),
            principal_id: "gwt-host-continuation".to_string(),
            work_id: Some("work-takeover".to_string()),
            source: Some("continue-work:resume".to_string()),
            from_session_id: "session-original".to_string(),
            to_session_id: "session-successor".to_string(),
            reason: "verified dead pane".to_string(),
            requested_at: Utc::now(),
        };

        let prepared = prepare_generation_takeover(dir.path(), owner, &request).unwrap();
        assert_eq!(prepared.status, GenerationTakeoverAttemptStatus::Prepared);
        let mut retargeted = request.clone();
        retargeted.work_id = Some("work-foreign".to_string());
        assert_eq!(
            prepare_generation_takeover(dir.path(), owner, &retargeted)
                .unwrap_err()
                .kind(),
            ErrorKind::AlreadyExists,
            "the same takeover operation cannot be retargeted to another Work"
        );
        let planned =
            prepared_generation_takeover_execution_binding(dir.path(), owner, &request).unwrap();
        assert_eq!(planned.generation_id, predecessor_binding.generation_id);
        assert_eq!(planned.binding_id, predecessor_binding.binding_id);
        assert_ne!(
            planned.ledger_head_hash,
            predecessor_binding.ledger_head_hash
        );
        assert!(prepared_execution_binding_matches(
            dir.path(),
            owner,
            &request.to_session_id,
            &planned,
        )
        .unwrap());
        assert_eq!(
            load(dir.path()).unwrap().unwrap().primary_session_id,
            request.from_session_id,
            "Prepared takeover must not change the current owner"
        );

        let activated = activate_generation_takeover(dir.path(), owner, &request).unwrap();
        assert_eq!(activated, planned);
        assert_eq!(
            current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap(),
            planned
        );
        assert_eq!(
            load(dir.path()).unwrap().unwrap().primary_session_id,
            request.to_session_id
        );
        let ledger = load_generation_ledger(dir.path(), owner).unwrap().unwrap();
        assert_eq!(ledger.current_generation_id, generation_id);
        assert_eq!(ledger.generations.len(), 1);
        assert_eq!(ledger.takeovers.len(), 1);
        let diagnosis = diagnose(dir.path(), None);
        assert!(
            diagnosis.warnings.iter().any(|warning| {
                warning
                    == &format!(
                        "latest_takeover:{generation_id}:session-original:session-successor"
                    )
            }),
            "{:?}",
            diagnosis.warnings
        );
        assert_eq!(
            diagnosis
                .continuation
                .as_ref()
                .and_then(|continuation| continuation.outcome.as_deref()),
            Some("takeover")
        );
        assert_eq!(
            diagnosis
                .continuation
                .as_ref()
                .and_then(|continuation| continuation.takeover_audit_id.as_deref()),
            ledger
                .takeovers
                .last()
                .map(|takeover| takeover.content_hash.as_str())
        );
        let continuation = diagnosis.continuation.expect("durable takeover diagnosis");
        assert!(continuation.predecessor_stale);
        assert_eq!(
            continuation.from_session_id.as_deref(),
            Some(request.from_session_id.as_str())
        );
        assert_eq!(
            continuation.current_writer.as_deref(),
            Some(request.to_session_id.as_str())
        );
        assert_eq!(
            ledger
                .takeover_attempts
                .iter()
                .map(|attempt| attempt.status)
                .collect::<Vec<_>>(),
            vec![
                GenerationTakeoverAttemptStatus::Prepared,
                GenerationTakeoverAttemptStatus::Activated,
            ]
        );
        assert!(
            !prepared_execution_binding_matches(
                dir.path(),
                owner,
                &request.to_session_id,
                &planned,
            )
            .unwrap(),
            "Activated takeover attempts are no longer Prepared authority"
        );
    }

    #[test]
    fn takeover_attempt_binding_match_is_exact_for_every_durable_status() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let owner = generation_owner();

        let aborted_dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(aborted_dir.path());
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(aborted_dir.path(), &active).unwrap();
        ensure_generation_ledger(aborted_dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let aborted_request = takeover_request("exact-binding-aborted");
        let prepared =
            prepare_generation_takeover(aborted_dir.path(), owner, &aborted_request).unwrap();
        let planned = prepared_generation_takeover_execution_binding(
            aborted_dir.path(),
            owner,
            &aborted_request,
        )
        .unwrap();
        assert!(generation_takeover_attempt_execution_binding_matches(
            aborted_dir.path(),
            owner,
            &prepared,
            &aborted_request.to_session_id,
            &planned,
        )
        .unwrap());
        let mut mismatched_head = planned.clone();
        mismatched_head.ledger_head_hash.push_str("-different");
        assert_eq!(mismatched_head.generation_id, planned.generation_id);
        assert!(!generation_takeover_attempt_execution_binding_matches(
            aborted_dir.path(),
            owner,
            &prepared,
            &aborted_request.to_session_id,
            &mismatched_head,
        )
        .unwrap());

        let aborted = abort_generation_takeover(
            aborted_dir.path(),
            owner,
            &aborted_request,
            "candidate Session was not Ready",
        )
        .unwrap();
        assert!(generation_takeover_attempt_execution_binding_matches(
            aborted_dir.path(),
            owner,
            &aborted,
            &aborted_request.to_session_id,
            &planned,
        )
        .unwrap());
        assert!(!generation_takeover_attempt_execution_binding_matches(
            aborted_dir.path(),
            owner,
            &aborted,
            &aborted_request.to_session_id,
            &mismatched_head,
        )
        .unwrap());

        let activated_dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(activated_dir.path());
        let activated_owner = ExecutionOwnerKey {
            number: owner.number + 1,
            ..owner
        };
        let mut active = active_record("session-original");
        active.owner_number = activated_owner.number;
        save(activated_dir.path(), &active).unwrap();
        ensure_generation_ledger(
            activated_dir.path(),
            activated_owner,
            LegacyActiveDisposition::Live,
        )
        .unwrap();
        let activated_request = takeover_request("exact-binding-activated");
        prepare_generation_takeover(activated_dir.path(), activated_owner, &activated_request)
            .unwrap();
        let activated_binding =
            activate_generation_takeover(activated_dir.path(), activated_owner, &activated_request)
                .unwrap();
        let activated = generation_takeover_attempt_for_operation(
            activated_dir.path(),
            activated_owner,
            &activated_request.operation_id,
        )
        .unwrap()
        .unwrap();
        assert!(generation_takeover_attempt_execution_binding_matches(
            activated_dir.path(),
            activated_owner,
            &activated,
            &activated_request.to_session_id,
            &activated_binding,
        )
        .unwrap());
        let mut mismatched_head = activated_binding;
        mismatched_head.ledger_head_hash.push_str("-different");
        assert!(!generation_takeover_attempt_execution_binding_matches(
            activated_dir.path(),
            activated_owner,
            &activated,
            &activated_request.to_session_id,
            &mismatched_head,
        )
        .unwrap());
    }

    #[test]
    fn takeover_prepared_activation_rejects_foreign_current_owner_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let foreign_owner = ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: owner.number + 1,
        };
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let request = takeover_request("foreign-owner-prepared-takeover");
        prepare_generation_takeover(dir.path(), owner, &request).unwrap();
        let foreign_before = replace_current_generation_authority(dir.path(), foreign_owner);

        let error = activate_generation_takeover(dir.path(), owner, &request).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            generation_authority_bytes(dir.path(), foreign_owner),
            foreign_before,
            "a stale Prepared takeover must not overwrite foreign generation authority",
        );
        assert_eq!(
            generation_takeover_attempt_for_operation(dir.path(), owner, &request.operation_id,)
                .unwrap()
                .unwrap()
                .status,
            GenerationTakeoverAttemptStatus::Prepared,
            "a rejected stale activation must leave its attempt Prepared",
        );
    }

    #[test]
    fn takeover_activated_repair_rejects_foreign_current_owner_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let foreign_owner = ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: owner.number + 1,
        };
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let request = takeover_request("foreign-owner-activated-takeover");
        prepare_generation_takeover(dir.path(), owner, &request).unwrap();
        activate_generation_takeover(dir.path(), owner, &request).unwrap();
        let foreign_before = replace_current_generation_authority(dir.path(), foreign_owner);

        let error = activate_generation_takeover(dir.path(), owner, &request).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            generation_authority_bytes(dir.path(), foreign_owner),
            foreign_before,
            "a stale Activated repair must not overwrite foreign generation authority",
        );
        assert_eq!(
            generation_takeover_attempt_for_operation(dir.path(), owner, &request.operation_id,)
                .unwrap()
                .unwrap()
                .status,
            GenerationTakeoverAttemptStatus::Activated,
        );
    }

    #[test]
    fn prepared_takeover_abort_and_lost_cas_leave_current_owner_unchanged() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let base_request = GenerationTakeoverRequest {
            operation_id: "operation-abort".to_string(),
            principal_id: "gwt-host-continuation".to_string(),
            work_id: Some("work-takeover".to_string()),
            source: Some("continue-work:resume".to_string()),
            from_session_id: "session-original".to_string(),
            to_session_id: "session-aborted".to_string(),
            reason: "verified dead pane".to_string(),
            requested_at: Utc::now(),
        };
        prepare_generation_takeover(dir.path(), owner, &base_request).unwrap();
        abort_generation_takeover(
            dir.path(),
            owner,
            &base_request,
            "candidate launch failed before Ready",
        )
        .unwrap();
        assert_eq!(
            load(dir.path()).unwrap().unwrap().primary_session_id,
            "session-original"
        );
        assert!(activate_generation_takeover(dir.path(), owner, &base_request).is_err());

        let mut winner = base_request.clone();
        winner.operation_id = "operation-winner".to_string();
        winner.to_session_id = "session-winner".to_string();
        winner.requested_at = Utc::now();
        let mut loser = base_request;
        loser.operation_id = "operation-loser".to_string();
        loser.to_session_id = "session-loser".to_string();
        loser.requested_at = Utc::now();
        prepare_generation_takeover(dir.path(), owner, &winner).unwrap();
        prepare_generation_takeover(dir.path(), owner, &loser).unwrap();
        activate_generation_takeover(dir.path(), owner, &winner).unwrap();
        assert_eq!(
            activate_generation_takeover(dir.path(), owner, &loser)
                .unwrap_err()
                .kind(),
            ErrorKind::AlreadyExists
        );
        abort_generation_takeover(
            dir.path(),
            owner,
            &loser,
            "takeover CAS lost to another Ready pane",
        )
        .unwrap();
        assert_eq!(
            load(dir.path()).unwrap().unwrap().primary_session_id,
            "session-winner"
        );
        assert_eq!(
            load_generation_ledger(dir.path(), owner)
                .unwrap()
                .unwrap()
                .generations
                .len(),
            1
        );
    }

    #[test]
    fn generation_ledger_activation_cas_has_exactly_one_winner() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let competing_dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(competing_dir.path());
        let owner = generation_owner();
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(dir.path(), &completed).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Unknown).unwrap();

        let request_a = successor_request("operation-a", "principal-a", "quick-start");
        let request_b = successor_request("operation-b", "principal-b", "work-detail");
        prepare_successor(dir.path(), owner, &request_a).unwrap();
        prepare_successor(competing_dir.path(), owner, &request_b).unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut handles = Vec::new();
        for (worktree, request) in [
            (dir.path().to_path_buf(), request_a.clone()),
            (competing_dir.path().to_path_buf(), request_b.clone()),
        ] {
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                activate_successor(&worktree, owner, &request)
            }));
        }
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);

        let winner_index = results.iter().position(Result::is_ok).unwrap();
        let winner_worktree = if winner_index == 0 {
            dir.path()
        } else {
            competing_dir.path()
        };
        let loser_worktree = if winner_index == 0 {
            competing_dir.path()
        } else {
            dir.path()
        };
        let ledger = load_generation_ledger(winner_worktree, owner)
            .unwrap()
            .unwrap();
        assert_eq!(ledger.generations.len(), 2);
        assert_eq!(
            ledger
                .generations
                .iter()
                .filter(|generation| generation.status == ExecutionControlStatus::Active)
                .count(),
            1
        );
        let current = ledger.current_generation().unwrap();
        assert_eq!(current.status, ExecutionControlStatus::Active);
        assert_eq!(
            results
                .iter()
                .find_map(|result| result.as_ref().ok())
                .unwrap(),
            &current.identity
        );

        let winner_request = if winner_index == 0 {
            &request_a
        } else {
            &request_b
        };
        assert_eq!(
            activate_successor(winner_worktree, owner, winner_request).unwrap(),
            current.identity
        );
        assert_eq!(
            load_generation_ledger(loser_worktree, owner)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData,
            "losing/stale worktree pointer must not follow the owner-wide winner"
        );
    }

    #[test]
    fn generation_ledger_terminal_transition_updates_binding_head_not_attempts() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();

        let initial_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        let result = settle(
            dir.path(),
            "session-original",
            ExecutionSettlement::Completed,
        )
        .unwrap();
        assert!(matches!(result, SettleResult::Settled(_)));
        let settled_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        assert_eq!(settled_binding.generation_id, initial_binding.generation_id);
        assert_eq!(settled_binding.binding_id, initial_binding.binding_id);
        assert_ne!(
            settled_binding.ledger_head_hash, initial_binding.ledger_head_hash,
            "same-generation terminal lifecycle must advance the exact binding head"
        );
        let settled = load_generation_ledger(dir.path(), owner).unwrap().unwrap();
        assert_eq!(
            settled.current_effective_status(),
            Some(ExecutionControlStatus::Completed)
        );

        let request = successor_request("operation-after-settle", "principal-a", "quick-start");
        prepare_successor(dir.path(), owner, &request).unwrap();
        assert_eq!(
            current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap(),
            settled_binding,
            "Prepared audit must not advance the effective generation lifecycle head"
        );
        abort_successor(dir.path(), owner, &request, "cancelled before launch").unwrap();
        assert_eq!(
            current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap(),
            settled_binding,
            "Aborted audit must not advance the effective generation lifecycle head"
        );

        let successor_request =
            successor_request("operation-activate", "principal-b", "work-detail");
        prepare_successor(dir.path(), owner, &successor_request).unwrap();
        let successor = activate_successor(dir.path(), owner, &successor_request).unwrap();
        assert_eq!(
            successor.predecessor_content_hash.as_deref(),
            Some(settled_binding.ledger_head_hash.as_str()),
            "successor predecessor must bind the terminal lifecycle head"
        );
        let activated = load_generation_ledger(dir.path(), owner).unwrap().unwrap();
        assert_eq!(activated.generations.len(), 2);
        assert_eq!(
            activated
                .generations
                .iter()
                .filter(|generation| {
                    activated.effective_status_for(generation) == ExecutionControlStatus::Active
                })
                .count(),
            1
        );
        assert_eq!(
            activated.current_effective_status(),
            Some(ExecutionControlStatus::Active)
        );
        assert_eq!(
            activated.effective_status_for(&activated.generations[0]),
            ExecutionControlStatus::Completed
        );

        let before_flat_refusal =
            crate::cli::trusted_store::read(dir.path(), "execution-control.json")
                .unwrap()
                .unwrap();
        active.owner_kind = ExecutionOwnerKind::Issue;
        active.owner_number = owner.number + 999;
        active.primary_session_id = "flat-only-writer".to_string();
        let refusal = save(dir.path(), &active).unwrap_err();
        assert_eq!(refusal.kind(), ErrorKind::PermissionDenied);
        assert_eq!(
            crate::cli::trusted_store::read(dir.path(), "execution-control.json")
                .unwrap()
                .unwrap(),
            before_flat_refusal,
            "pointer-owned projection bytes must remain unchanged even when a flat writer changes owner"
        );
    }

    #[test]
    fn generation_lifecycle_descendant_authorizes_unchanged_durable_session_binding() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let session_id = "session-lifecycle-binding";
        let mut active = active_record(session_id);
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let active_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(dir.path(), owner, session_id, active_binding.clone());
        let session_path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
        let session_before = fs::read(&session_path).unwrap();

        assert!(matches!(
            settle(
                dir.path(),
                session_id,
                ExecutionSettlement::Blocked {
                    reason: "post-block verification required".to_string(),
                    missing_verification: Some("verify.run".to_string()),
                },
            )
            .unwrap(),
            SettleResult::Settled(_)
        ));

        let blocked_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        assert_ne!(
            blocked_binding.ledger_head_hash, active_binding.ledger_head_hash,
            "Blocked lifecycle must advance the generation head"
        );
        assert_eq!(
            fs::read(&session_path).unwrap(),
            session_before,
            "lifecycle settlement must keep the durable Session byte-identical"
        );
        let durable = gwt_agent::Session::load(&session_path)
            .unwrap()
            .execution_binding
            .unwrap();
        assert_eq!(
            durable.identity, active_binding,
            "the Session remains bound to its authentic pre-lifecycle head"
        );
        assert_eq!(
            durable.capability_generation, 1,
            "a ledger-only lifecycle transition must not rotate the Host capability epoch"
        );
        assert_eq!(
            crate::cli::verification_record::snapshot_current_generation_caller_binding(
                dir.path(),
                Some(session_id),
            )
            .expect("post-block verification caller remains authorized"),
            Some(durable),
        );
    }

    #[cfg(unix)]
    #[test]
    fn generation_lifecycle_refuses_dangling_legacy_session_without_mutation() {
        use std::os::unix::fs::symlink;

        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-dangling-legacy");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        fs::create_dir_all(&sessions_dir).expect("create Sessions directory");
        let session_path = sessions_dir.join("session-dangling-legacy.toml");
        let missing_target = sessions_dir.join("missing-legacy-session-target");
        symlink(&missing_target, &session_path).expect("create dangling legacy Session");
        let authority_before = generation_authority_bytes(dir.path(), owner);

        let result = settle(
            dir.path(),
            "session-dangling-legacy",
            ExecutionSettlement::Completed,
        )
        .expect("dangling legacy Session refusal");

        assert!(
            matches!(result, SettleResult::BindingMismatch),
            "{result:?}"
        );
        assert_eq!(
            generation_authority_bytes(dir.path(), owner),
            authority_before
        );
        assert!(fs::symlink_metadata(&session_path)
            .expect("dangling legacy Session must remain")
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_link(&session_path).unwrap(), missing_target);
    }

    #[test]
    fn generation_lifecycle_rechecks_missing_session_under_lease_before_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let session_id = "session-legacy-missing-race";
        let mut active = active_record(session_id);
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        fs::create_dir_all(&sessions_dir).expect("create Sessions directory");
        let session_path = sessions_dir.join(format!("{session_id}.toml"));
        assert!(matches!(
            gwt_agent::inspect_session_path(&session_path),
            gwt_agent::SessionPathState::Missing
        ));
        let mut completed = active.clone();
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        let authority_before = generation_authority_bytes(dir.path(), owner);
        let current = current_owner_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();

        let result = persist_generation_lifecycle_transition_if_owned_with_before_session_lease(
            dir.path(),
            &completed,
            ExecutionControlStatus::Active,
            "legacy race settlement",
            || {
                let repo_hash = crate::index_worker::detect_repo_hash(dir.path())
                    .expect("repo hash")
                    .to_string();
                let mut foreign = gwt_agent::Session::new(
                    dir.path(),
                    "work/foreign-binding",
                    gwt_agent::AgentId::Codex,
                );
                foreign.id = session_id.to_string();
                foreign.repo_hash = Some(repo_hash.clone());
                foreign.linked_issue_number = Some(owner.number);
                let mut foreign_identity = current.clone();
                foreign_identity.binding_id = "foreign-materialized-binding".to_string();
                foreign.execution_binding = Some(gwt_agent::SessionExecutionBinding {
                    schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                    session_id: session_id.to_string(),
                    repo_hash,
                    owner_kind: owner.kind.as_str().to_string(),
                    owner_number: owner.number,
                    identity: foreign_identity,
                    capability_generation: 1,
                });
                foreign
                    .save(&sessions_dir)
                    .expect("materialize foreign same-id Session after missing observation");
            },
        )
        .expect_err("foreign Session materialization must revoke legacy Missing compatibility");

        assert_eq!(result.kind(), ErrorKind::PermissionDenied);
        assert_eq!(
            generation_authority_bytes(dir.path(), owner),
            authority_before
        );
        let retained = gwt_agent::Session::load(&session_path).expect("foreign Session retained");
        assert_eq!(
            retained
                .execution_binding
                .expect("foreign binding retained")
                .identity
                .binding_id,
            "foreign-materialized-binding"
        );
    }

    #[test]
    fn generation_active_binding_match_refuses_terminal_and_mismatched_authority() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();

        let active_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        assert!(current_active_execution_binding_matches(
            dir.path(),
            owner,
            "session-original",
            &active_binding,
        )
        .unwrap());
        assert!(!current_active_execution_binding_matches(
            dir.path(),
            owner,
            "session-other",
            &active_binding,
        )
        .unwrap());
        let mut mismatched_binding = active_binding.clone();
        mismatched_binding.binding_id.push_str("-stale");
        assert!(!current_active_execution_binding_matches(
            dir.path(),
            owner,
            "session-original",
            &mismatched_binding,
        )
        .unwrap());

        assert!(matches!(
            settle(
                dir.path(),
                "session-original",
                ExecutionSettlement::Completed,
            )
            .unwrap(),
            SettleResult::Settled(_)
        ));
        let terminal_binding = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        assert!(!current_active_execution_binding_matches(
            dir.path(),
            owner,
            "session-original",
            &terminal_binding,
        )
        .unwrap());
    }

    #[test]
    fn leased_active_binding_fences_epoch_rotation_before_operation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let identity = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(dir.path(), owner, "session-original", identity);
        let session_path = gwt_core::paths::gwt_sessions_dir().join("session-original.toml");
        let expected = gwt_agent::Session::load(&session_path)
            .unwrap()
            .execution_binding
            .unwrap();
        let dispatches = std::cell::Cell::new(0_u8);

        let current = with_current_active_execution_binding_lease(
            &gwt_core::paths::gwt_sessions_dir(),
            &expected,
            || {
                dispatches.set(dispatches.get() + 1);
                "current"
            },
        )
        .unwrap();
        assert_eq!(current, Some("current"));
        assert_eq!(dispatches.get(), 1);

        gwt_agent::rotate_session_execution_capability(
            &gwt_core::paths::gwt_sessions_dir(),
            "session-original",
        )
        .unwrap();
        let stale = with_current_active_execution_binding_lease(
            &gwt_core::paths::gwt_sessions_dir(),
            &expected,
            || {
                dispatches.set(dispatches.get() + 1);
                "stale"
            },
        )
        .unwrap();
        assert_eq!(stale, None);
        assert_eq!(
            dispatches.get(),
            1,
            "an old Host epoch must be rejected before its operation closure"
        );
    }

    #[test]
    fn leased_active_binding_times_out_session_contention_and_retries_without_stranding_owner() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let identity = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(dir.path(), owner, "session-original", identity);
        let expected = gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir().join("session-original.toml"),
        )
        .unwrap()
        .execution_binding
        .unwrap();

        let (lease_acquired_tx, lease_acquired_rx) = std::sync::mpsc::sync_channel(1);
        let (release_lease_tx, release_lease_rx) = std::sync::mpsc::sync_channel(1);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let holder = std::thread::spawn(move || {
            gwt_agent::with_session_lease_wait(
                &sessions_dir,
                "session-original",
                std::time::Duration::from_secs(1),
                |_| {
                    lease_acquired_tx.send(()).unwrap();
                    release_lease_rx.recv().unwrap();
                    Ok(())
                },
            )
            .unwrap()
        });
        lease_acquired_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        let error = with_current_active_execution_binding_lease_wait(
            &gwt_core::paths::gwt_sessions_dir(),
            &expected,
            std::time::Duration::from_millis(25),
            || "must-not-run",
        )
        .expect_err("Session contention must return a bounded retry error");
        assert_eq!(error.kind(), ErrorKind::WouldBlock);
        assert!(error.to_string().contains("retry"));
        assert!(!error.to_string().contains("session-original"));
        with_generation_owner_lease(dir.path(), owner, |_| Ok(()))
            .expect("Session timeout must release the owner lease before returning");

        release_lease_tx.send(()).unwrap();
        holder.join().unwrap();
        let retried = with_current_active_execution_binding_lease_wait(
            &gwt_core::paths::gwt_sessions_dir(),
            &expected,
            std::time::Duration::from_millis(100),
            || "retried",
        )
        .unwrap();
        assert_eq!(retried, Some("retried"));
    }

    #[test]
    fn leased_active_binding_refuses_session_to_owner_reverse_nesting() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let identity = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        persist_generation_session_binding(dir.path(), owner, "session-original", identity);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let expected = gwt_agent::Session::load(&sessions_dir.join("session-original.toml"))
            .unwrap()
            .execution_binding
            .unwrap();

        let started = std::time::Instant::now();
        let error = gwt_agent::with_session_lease_wait(
            &sessions_dir,
            "session-original",
            std::time::Duration::from_secs(1),
            |_| {
                with_current_active_execution_binding_lease_wait(
                    &sessions_dir,
                    &expected,
                    std::time::Duration::from_secs(1),
                    || (),
                )
                .map(|_| ())
            },
        )
        .expect_err("Session-to-owner reverse nesting must be refused");
        assert_eq!(error.kind(), ErrorKind::WouldBlock);
        assert!(error.to_string().contains("owner lease"));
        assert!(error.to_string().contains("before the Session lease"));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "reverse nesting must fail immediately rather than waiting for its own Session lock"
        );
    }

    #[test]
    fn generation_ledger_adopt_keeps_generation_and_advances_binding_head() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        let before = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        let recovery_session =
            persist_recovery_session_snapshot(dir.path(), owner, "session-adopting");

        let mut out = String::new();
        assert_eq!(
            run_adopt(
                dir.path(),
                "session-adopting",
                &recovery_session,
                "explicit handoff",
                &mut out,
            )
            .unwrap(),
            0,
            "{out}"
        );
        let after = current_execution_binding(dir.path(), owner)
            .unwrap()
            .unwrap();
        assert_eq!(after.generation_id, before.generation_id);
        assert_eq!(after.binding_id, before.binding_id);
        assert_ne!(after.ledger_head_hash, before.ledger_head_hash);
        assert!(
            !execution_binding_authorizes_current_generation(
                dir.path(),
                owner,
                "session-adopting",
                &before,
            )
            .unwrap(),
            "a takeover suffix must never authorize the predecessor head"
        );
        let ledger = load_generation_ledger(dir.path(), owner).unwrap().unwrap();
        assert_eq!(ledger.generations.len(), 1);
        assert_eq!(ledger.takeovers.len(), 1);
        assert_eq!(
            ledger.current_effective_status(),
            Some(ExecutionControlStatus::Active)
        );
        assert_eq!(
            load(dir.path()).unwrap().unwrap().primary_session_id,
            "session-adopting"
        );
    }

    #[test]
    fn generation_ledger_rejects_hash_predecessor_and_terminal_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let owner = generation_owner();

        let terminal_dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(terminal_dir.path());
        let mut completed = active_record("session-original");
        completed.owner_number = owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(terminal_dir.path(), &completed).unwrap();
        ensure_generation_ledger(terminal_dir.path(), owner, LegacyActiveDisposition::Unknown)
            .unwrap();
        let terminal_path = generation_ledger_path(terminal_dir.path(), owner).unwrap();
        let original_terminal_ledger = fs::read(&terminal_path).unwrap();
        let mut terminal_ledger: ExecutionGenerationLedger =
            serde_json::from_slice(&original_terminal_ledger).unwrap();
        terminal_ledger.generations[0].execution_control_json = terminal_ledger.generations[0]
            .execution_control_json
            .replace("\"completed\"", "\"active\"");
        fs::write(
            &terminal_path,
            serde_json::to_vec_pretty(&terminal_ledger).unwrap(),
        )
        .unwrap();
        assert_eq!(
            load_generation_ledger(terminal_dir.path(), owner)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );

        let predecessor_dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(predecessor_dir.path());
        let predecessor_owner = ExecutionOwnerKey {
            kind: owner.kind,
            number: owner.number + 1,
        };
        let mut predecessor_completed = completed.clone();
        predecessor_completed.owner_number = predecessor_owner.number;
        save(predecessor_dir.path(), &predecessor_completed).unwrap();
        ensure_generation_ledger(
            predecessor_dir.path(),
            predecessor_owner,
            LegacyActiveDisposition::Unknown,
        )
        .unwrap();
        let request = successor_request("operation-successor", "principal-a", "quick-start");
        prepare_successor(predecessor_dir.path(), predecessor_owner, &request).unwrap();
        activate_successor(predecessor_dir.path(), predecessor_owner, &request).unwrap();
        let predecessor_path =
            generation_ledger_path(predecessor_dir.path(), predecessor_owner).unwrap();
        let mut predecessor_ledger: ExecutionGenerationLedger =
            serde_json::from_slice(&fs::read(&predecessor_path).unwrap()).unwrap();
        predecessor_ledger.generations[1]
            .identity
            .predecessor_content_hash = Some("validly-rehashed-but-wrong".to_string());
        predecessor_ledger.generations[1].content_hash =
            compute_generation_hash(&predecessor_ledger.generations[1]);
        predecessor_ledger.content_hash = compute_generation_ledger_hash(&predecessor_ledger);
        fs::write(
            &predecessor_path,
            serde_json::to_vec_pretty(&predecessor_ledger).unwrap(),
        )
        .unwrap();
        assert_eq!(
            load_generation_ledger(predecessor_dir.path(), predecessor_owner)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
    }

    #[test]
    fn generation_ledger_projection_event_chain_rejects_truncate_reorder_and_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_owner();
        let mut active = active_record("session-original");
        active.owner_number = owner.number;
        save(dir.path(), &active).unwrap();
        ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
        settle(
            dir.path(),
            "session-original",
            ExecutionSettlement::Blocked {
                reason: "dependency blocked".to_string(),
                missing_verification: Some("focused matrix".to_string()),
            },
        )
        .unwrap();

        let mut reopened = load(dir.path()).unwrap().unwrap();
        reopened
            .recoveries
            .push(test_recovery("session-original", 1));
        reopened.blocked_reason = None;
        reopened.missing_verification = None;
        reopened.status = ExecutionControlStatus::Active;
        reopened.settled_at = None;
        assert!(persist_generation_lifecycle_transition_if_owned(
            dir.path(),
            &reopened,
            ExecutionControlStatus::Blocked,
            "verified recovery",
        )
        .unwrap());

        let path = generation_ledger_path(dir.path(), owner).unwrap();
        let original = fs::read(&path).unwrap();
        let baseline: ExecutionGenerationLedger = serde_json::from_slice(&original).unwrap();
        assert_eq!(baseline.lifecycle_events.len(), 2);

        let write_and_reject = |ledger: &mut ExecutionGenerationLedger| {
            ledger.content_hash = compute_generation_ledger_hash(ledger);
            fs::write(&path, serde_json::to_vec_pretty(ledger).unwrap()).unwrap();
            assert_eq!(
                load_owner_generation_ledger(dir.path(), owner)
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidData
            );
            fs::write(&path, &original).unwrap();
        };

        let mut mutated = baseline.clone();
        mutated.lifecycle_events[0].execution_control_json = mutated.lifecycle_events[0]
            .execution_control_json
            .replace("\"blocked\"", "\"completed\"");
        write_and_reject(&mut mutated);

        let mut truncated = baseline.clone();
        truncated.lifecycle_events.remove(0);
        truncated.lifecycle_events[0].sequence = 1;
        truncated.lifecycle_events[0].previous_event_hash.clear();
        truncated.lifecycle_events[0].content_hash =
            compute_lifecycle_event_hash(&truncated.lifecycle_events[0]);
        write_and_reject(&mut truncated);

        let mut reordered = baseline;
        reordered.lifecycle_events.swap(0, 1);
        let mut previous_hash = String::new();
        for (index, event) in reordered.lifecycle_events.iter_mut().enumerate() {
            event.sequence = index as u64 + 1;
            event.previous_event_hash.clone_from(&previous_hash);
            event.content_hash = compute_lifecycle_event_hash(event);
            previous_hash.clone_from(&event.content_hash);
        }
        write_and_reject(&mut reordered);
    }

    #[test]
    fn generation_ledger_classifies_legacy_active_and_refuses_old_writer() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = unset_live_session_env();
        let unknown_owner = generation_owner();

        let unknown_dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(unknown_dir.path());
        let mut active = active_record("session-original");
        active.owner_number = unknown_owner.number;
        save(unknown_dir.path(), &active).unwrap();
        let active_bytes =
            crate::cli::trusted_store::read(unknown_dir.path(), "execution-control.json")
                .unwrap()
                .unwrap();
        let error = ensure_generation_ledger(
            unknown_dir.path(),
            unknown_owner,
            LegacyActiveDisposition::Unknown,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::WouldBlock);
        assert!(!generation_ledger_path(unknown_dir.path(), unknown_owner)
            .unwrap()
            .exists());
        assert!(!generation_pointer_path(unknown_dir.path()).exists());
        assert_eq!(
            crate::cli::trusted_store::read(unknown_dir.path(), "execution-control.json")
                .unwrap()
                .unwrap(),
            active_bytes,
            "unknown legacy liveness must be a zero-mutation refusal"
        );

        let stale_dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(stale_dir.path());
        let stale_owner = ExecutionOwnerKey {
            kind: unknown_owner.kind,
            number: unknown_owner.number + 1,
        };
        let mut stale_active = active.clone();
        stale_active.owner_number = stale_owner.number;
        save(stale_dir.path(), &stale_active).unwrap();
        let stale = ensure_generation_ledger(
            stale_dir.path(),
            stale_owner,
            LegacyActiveDisposition::Stale {
                new_session_id: "session-successor".to_string(),
                reason: "dead pane takeover".to_string(),
                observed_at: Utc::now(),
            },
        )
        .unwrap();
        assert_eq!(stale.generations.len(), 1);
        assert_eq!(stale.takeovers.len(), 1);
        assert_eq!(
            stale.takeovers[0].generation_id,
            stale.current_generation_id
        );
        assert_eq!(
            stale
                .current_generation()
                .unwrap()
                .identity
                .initial_session_id,
            "session-original"
        );
        assert_eq!(
            load(stale_dir.path()).unwrap().unwrap().primary_session_id,
            "session-successor"
        );

        let hashless_dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(hashless_dir.path());
        let hashless_owner = ExecutionOwnerKey {
            kind: unknown_owner.kind,
            number: unknown_owner.number + 2,
        };
        let mut hashless_active = active.clone();
        hashless_active.owner_number = hashless_owner.number;
        let hashless_bytes = serde_json::to_vec_pretty(&hashless_active).unwrap();
        gwt_github::cache::write_atomic(&state_path(hashless_dir.path()), &hashless_bytes).unwrap();
        let error = ensure_generation_ledger(
            hashless_dir.path(),
            hashless_owner,
            LegacyActiveDisposition::Unknown,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::WouldBlock);
        assert!(!generation_ledger_path(hashless_dir.path(), hashless_owner)
            .unwrap()
            .exists());
        assert!(!generation_pointer_path(hashless_dir.path()).exists());
        let imported_hashless = ensure_generation_ledger(
            hashless_dir.path(),
            hashless_owner,
            LegacyActiveDisposition::Live,
        )
        .unwrap();
        assert_eq!(
            imported_hashless.current_effective_status(),
            Some(ExecutionControlStatus::Active)
        );
        assert!(
            !load(hashless_dir.path())
                .unwrap()
                .unwrap()
                .content_hash
                .is_empty(),
            "an explicitly classified hashless Active record is canonicalized during import"
        );

        let old_writer_dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(old_writer_dir.path());
        let old_writer_owner = ExecutionOwnerKey {
            kind: unknown_owner.kind,
            number: unknown_owner.number + 3,
        };
        let mut completed = active.clone();
        completed.owner_number = old_writer_owner.number;
        completed.status = ExecutionControlStatus::Completed;
        completed.settled_at = Some(Utc::now());
        save(old_writer_dir.path(), &completed).unwrap();
        ensure_generation_ledger(
            old_writer_dir.path(),
            old_writer_owner,
            LegacyActiveDisposition::Unknown,
        )
        .unwrap();
        let mut old_writer_projection = load(old_writer_dir.path()).unwrap().unwrap();
        old_writer_projection.primary_session_id = "session-old-writer".to_string();
        old_writer_projection.content_hash = compute_content_hash(&old_writer_projection);
        let old_writer_bytes = serde_json::to_vec_pretty(&old_writer_projection).unwrap();
        crate::cli::trusted_store::write_with_mirror(
            old_writer_dir.path(),
            "execution-control.json",
            &state_path(old_writer_dir.path()),
            &old_writer_bytes,
        )
        .unwrap();
        assert_eq!(
            load_generation_ledger(old_writer_dir.path(), old_writer_owner)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData,
            "a flat projection rewrite after ledger ownership must be refused"
        );
        let normal_reader = load(old_writer_dir.path()).unwrap().unwrap();
        assert_eq!(normal_reader.status, ExecutionControlStatus::Active);
        assert!(
            !integrity_ok(&normal_reader),
            "canonical flat reader must return a fail-closed integrity sentinel"
        );
        assert!(
            !is_completed(old_writer_dir.path()),
            "stale terminal projection must never release completion gates"
        );
    }

    // T-106: roundtrip with owner kind/number, primary session id,
    // entrypoint, bundled-required owners, state, and timestamps.
    #[test]
    fn record_roundtrips_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let record = active_record("sess-1");
        save(dir.path(), &record).unwrap();
        // save() stamps the integrity hash (P9a); content must roundtrip and
        // the stored hash must validate.
        let loaded = load(dir.path()).unwrap().unwrap();
        assert!(integrity_ok(&loaded));
        assert!(!loaded.content_hash.is_empty());
        let mut normalized = loaded.clone();
        normalized.content_hash = String::new();
        assert_eq!(normalized, record);
    }

    #[test]
    fn recovery_history_is_anchored_in_old_schema_storage_projection() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = active_record("sess-1");
        record.recoveries = vec![test_recovery("sess-1", 1), test_recovery("sess-1", 2)];
        save(dir.path(), &record).unwrap();

        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(state_path(dir.path())).unwrap()).unwrap();
        assert!(
            raw.get("recoveries").is_none(),
            "all recovery generations must see the old-schema ECR projection"
        );
        let stored_transfers = raw["transfers"].as_array().unwrap();
        assert_eq!(stored_transfers.len(), 2);
        assert!(stored_transfers.iter().all(|transfer| transfer["reason"]
            .as_str()
            .unwrap()
            .starts_with("gwt:execution-recovery:v1:")));

        let loaded = load(dir.path()).unwrap().unwrap();
        assert!(loaded.transfers.is_empty());
        assert_eq!(loaded.recoveries.len(), 2);
        assert!(integrity_ok(&loaded));

        let mut truncated = loaded.clone();
        truncated.recoveries.pop();
        assert!(!integrity_ok(&truncated));
        truncated.recoveries.clear();
        assert!(!integrity_ok(&truncated));
    }

    #[test]
    fn canonical_save_refuses_recovery_history_truncation_or_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = active_record("sess-1");
        record.recoveries = vec![test_recovery("sess-1", 1), test_recovery("sess-1", 2)];
        save(dir.path(), &record).unwrap();
        let loaded = load(dir.path()).unwrap().unwrap();

        let mut truncated = loaded.clone();
        truncated.recoveries.pop();
        let err = save(dir.path(), &truncated).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);

        let mut replaced = loaded;
        replaced.recoveries[0].reason = "replacement".to_string();
        let err = save(dir.path(), &replaced).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn old_typed_writers_preserve_recovery_envelopes_and_integrity() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = active_record("sess-1");
        record.recoveries = vec![test_recovery("sess-1", 1), test_recovery("sess-1", 2)];
        save(dir.path(), &record).unwrap();
        let path = state_path(dir.path());

        let raw = fs::read_to_string(&path).unwrap();
        let mut pre_recovery: PreRecoveryControlRecord = serde_json::from_str(&raw).unwrap();
        let stored_hash = pre_recovery.content_hash.clone();
        pre_recovery.content_hash.clear();
        let expected_hash = format!(
            "{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(serde_json::to_vec(&pre_recovery).unwrap())
        );
        assert_eq!(stored_hash, expected_hash);
        pre_recovery.transfers.push(OwnershipTransfer {
            from_session_id: "sess-1".to_string(),
            to_session_id: "sess-old-writer".to_string(),
            reason: format!("{RECOVERY_ENVELOPE_PREFIX}legacy-prefix-collision"),
            transferred_at: Utc::now(),
        });
        pre_recovery.primary_session_id = "sess-old-writer".to_string();
        pre_recovery.content_hash = {
            let bytes = serde_json::to_vec(&pre_recovery).unwrap();
            format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(bytes))
        };
        fs::write(&path, serde_json::to_vec_pretty(&pre_recovery).unwrap()).unwrap();
        let after_pre_recovery = load(dir.path()).unwrap().unwrap();
        assert_eq!(after_pre_recovery.recoveries.len(), 2);
        assert_eq!(after_pre_recovery.transfers.len(), 1);
        assert!(integrity_ok(&after_pre_recovery));

        let raw = fs::read_to_string(&path).unwrap();
        let mut initial: InitialRecoveryControlRecord = serde_json::from_str(&raw).unwrap();
        assert!(initial.recoveries.is_empty());
        initial.content_hash.clear();
        initial.content_hash = {
            let bytes = serde_json::to_vec(&initial).unwrap();
            format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(bytes))
        };
        fs::write(&path, serde_json::to_vec_pretty(&initial).unwrap()).unwrap();
        let after_initial_writer = load(dir.path()).unwrap().unwrap();
        assert_eq!(after_initial_writer.recoveries.len(), 2);
        assert_eq!(after_initial_writer.transfers.len(), 1);
        assert!(integrity_ok(&after_initial_writer));
    }

    #[test]
    fn raw_recovery_envelope_corruption_fails_closed() {
        let make_raw = || {
            let dir = tempfile::tempdir().unwrap();
            let mut record = active_record("sess-1");
            record.recoveries = vec![test_recovery("sess-1", 1), test_recovery("sess-1", 2)];
            save(dir.path(), &record).unwrap();
            let raw: ExecutionControlRecord =
                serde_json::from_str(&fs::read_to_string(state_path(dir.path())).unwrap()).unwrap();
            (dir, raw)
        };

        let (tail_dir, mut tail) = make_raw();
        tail.transfers.remove(1);
        fs::write(
            state_path(tail_dir.path()),
            serde_json::to_vec_pretty(&tail).unwrap(),
        )
        .unwrap();
        let tail_record = load(tail_dir.path()).unwrap().unwrap();
        assert!(!integrity_ok(&tail_record));
        assert_eq!(
            save(tail_dir.path(), &tail_record).unwrap_err().kind(),
            ErrorKind::InvalidData,
            "same-lifetime save must not launder a shortened recovery history"
        );

        let (all_dir, mut all) = make_raw();
        all.transfers.clear();
        fs::write(
            state_path(all_dir.path()),
            serde_json::to_vec_pretty(&all).unwrap(),
        )
        .unwrap();
        let all_record = load(all_dir.path()).unwrap().unwrap();
        assert!(!integrity_ok(&all_record));
        assert_eq!(
            save(all_dir.path(), &all_record).unwrap_err().kind(),
            ErrorKind::InvalidData,
            "same-lifetime save must not launder a fully deleted recovery history"
        );

        let (mixed_dir, mut mixed) = make_raw();
        mixed.recoveries.push(test_recovery("sess-1", 3));
        fs::write(
            state_path(mixed_dir.path()),
            serde_json::to_vec_pretty(&mixed).unwrap(),
        )
        .unwrap();
        assert!(!integrity_ok(&load(mixed_dir.path()).unwrap().unwrap()));

        let (interleaved_dir, mut interleaved) = make_raw();
        interleaved.transfers.insert(
            1,
            OwnershipTransfer {
                from_session_id: "a".to_string(),
                to_session_id: "b".to_string(),
                reason: "real transfer".to_string(),
                transferred_at: Utc::now(),
            },
        );
        fs::write(
            state_path(interleaved_dir.path()),
            serde_json::to_vec_pretty(&interleaved).unwrap(),
        )
        .unwrap();
        assert!(!integrity_ok(
            &load(interleaved_dir.path()).unwrap().unwrap()
        ));

        let (malformed_dir, mut malformed) = make_raw();
        malformed.transfers[0].reason = format!("{RECOVERY_ENVELOPE_PREFIX}not-json");
        fs::write(
            state_path(malformed_dir.path()),
            serde_json::to_vec_pretty(&malformed).unwrap(),
        )
        .unwrap();
        assert!(!integrity_ok(&load(malformed_dir.path()).unwrap().unwrap()));

        let (identity_dir, mut identity) = make_raw();
        identity.transfers[0].to_session_id = "different-session".to_string();
        fs::write(
            state_path(identity_dir.path()),
            serde_json::to_vec_pretty(&identity).unwrap(),
        )
        .unwrap();
        assert!(!integrity_ok(&load(identity_dir.path()).unwrap().unwrap()));
    }

    #[test]
    fn fresh_execution_lifetime_may_reset_recovery_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut previous = active_record("sess-1");
        previous.recoveries.push(test_recovery("sess-1", 1));
        save(dir.path(), &previous).unwrap();

        let mut fresh = active_record("sess-2");
        fresh.launched_at = previous.launched_at + chrono::Duration::nanoseconds(1);
        save(dir.path(), &fresh).unwrap();
        let loaded = load(dir.path()).unwrap().unwrap();
        assert!(loaded.recoveries.is_empty());
        assert!(integrity_ok(&loaded));
    }

    // P9b (T-174 core): once a repo-scoped trusted copy exists, editing the
    // worktree mirror changes nothing the gates trust — even a forged mirror
    // with a *valid* integrity hash is ignored.
    #[test]
    fn trusted_copy_overrides_worktree_mirror_edits() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        save(dir.path(), &active_record("sess-1")).unwrap();

        let mut forged = active_record("sess-1");
        forged.status = ExecutionControlStatus::Completed;
        forged.content_hash = compute_content_hash(&forged);
        let serialized = serde_json::to_vec_pretty(&forged).unwrap();
        gwt_github::cache::write_atomic(&state_path(dir.path()), &serialized).unwrap();

        let loaded = load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.status, ExecutionControlStatus::Active);
        assert!(integrity_ok(&loaded));
    }

    // P9b: once the trusted (authoritative) copy is written, a mirror write
    // failure must not report the save as failed — the gates already honor
    // the trusted copy, and "reported failed but actually effective" is the
    // worse asymmetry.
    #[test]
    fn mirror_write_failure_after_trusted_write_is_not_an_error() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        // Make the mirror unwritable: occupy `.gwt` with a plain file so the
        // mirror's parent directory cannot be created.
        fs::write(dir.path().join(".gwt"), b"not a directory").unwrap();

        save(dir.path(), &active_record("sess-1")).unwrap();
        let loaded = load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.primary_session_id, "sess-1");
        assert!(!state_path(dir.path()).exists());
    }

    // T-149 wiring: settlement contends on the owner write lease — while a
    // concurrent writer holds it past the bounded wait, settle() surfaces
    // the explicit-retry error instead of interleaving.
    #[test]
    fn settle_refuses_with_retry_while_lease_is_held() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &active_record("sess-1")).unwrap();

        let worktree = dir.path().to_path_buf();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            crate::cli::trusted_store::with_write_lease(&worktree, || {
                acquired_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(std::time::Duration::from_secs(10));
                Ok(())
            })
            .unwrap();
        });
        acquired_rx.recv().unwrap();
        let err = settle(dir.path(), "sess-1", ExecutionSettlement::Completed).unwrap_err();
        assert!(err.to_string().contains("retry"), "{err}");
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        // The record is untouched by the refused settlement.
        assert_eq!(
            load(dir.path()).unwrap().unwrap().status,
            ExecutionControlStatus::Active
        );
    }

    // T-182 core: resuming a settled pre-P9b worktree promotes the valid
    // mirror-only record into the trusted store (the resume path otherwise
    // never rewrites it).
    #[test]
    fn resume_imports_mirror_only_settled_record_into_trusted_store() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        // Pre-P9b shape: a settled record living only in the mirror.
        let mut legacy = active_record("sess-old");
        legacy.status = ExecutionControlStatus::Completed;
        legacy.settled_at = Some(Utc::now());
        legacy.content_hash = compute_content_hash(&legacy);
        let serialized = serde_json::to_vec_pretty(&legacy).unwrap();
        gwt_github::cache::write_atomic(&state_path(dir.path()), &serialized).unwrap();
        assert!(
            crate::cli::trusted_store::read(dir.path(), "execution-control.json")
                .unwrap()
                .is_none()
        );

        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-new",
            "resume",
            true,
        )
        .unwrap();
        // The settled record is preserved AND now trusted-store resident.
        let trusted = crate::cli::trusted_store::read(dir.path(), "execution-control.json")
            .unwrap()
            .expect("trusted copy imported");
        let record: ExecutionControlRecord = serde_json::from_str(&trusted).unwrap();
        assert_eq!(record.status, ExecutionControlStatus::Completed);
        assert_eq!(record.primary_session_id, "sess-old");
        assert!(integrity_ok(&record));
    }

    // P9b: a mirror-only record (written before the trusted store existed)
    // still loads — legacy fallback with the same one-release-cycle sunset
    // policy as the P9a empty integrity hashes.
    #[test]
    fn mirror_only_record_loads_as_legacy_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        let mut legacy = active_record("sess-legacy");
        legacy.content_hash = compute_content_hash(&legacy);
        let serialized = serde_json::to_vec_pretty(&legacy).unwrap();
        gwt_github::cache::write_atomic(&state_path(dir.path()), &serialized).unwrap();

        let loaded = load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.primary_session_id, "sess-legacy");
    }

    #[test]
    fn load_returns_none_when_absent_and_invalid_data_when_malformed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), None);
        let path = state_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not json").unwrap();
        assert_eq!(load(dir.path()).unwrap_err().kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn materialize_at_launch_writes_fresh_active_record() {
        let dir = tempfile::tempdir().unwrap();
        // A previous settled record is replaced by a new launch (P8a policy).
        let mut old = active_record("sess-old");
        old.status = ExecutionControlStatus::Completed;
        save(dir.path(), &old).unwrap();

        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            42,
            "sess-new",
            "resume",
            false,
        )
        .unwrap();
        let record = load(dir.path()).unwrap().unwrap();
        assert_eq!(record.owner_kind, ExecutionOwnerKind::Issue);
        assert_eq!(record.owner_number, 42);
        assert_eq!(record.primary_session_id, "sess-new");
        assert_eq!(record.entrypoint, "resume");
        assert_eq!(record.status, ExecutionControlStatus::Active);
        assert_eq!(record.settled_at, None);
    }

    #[test]
    fn settle_completes_and_blocks_with_session_binding() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &active_record("sess-1")).unwrap();

        // T-100 semantics: another session cannot settle.
        let result = settle(dir.path(), "other", ExecutionSettlement::Completed).unwrap();
        assert!(matches!(result, SettleResult::SessionMismatch { .. }));
        assert_eq!(
            load(dir.path()).unwrap().unwrap().status,
            ExecutionControlStatus::Active
        );

        // Owning session completes.
        let result = settle(dir.path(), "sess-1", ExecutionSettlement::Completed).unwrap();
        let SettleResult::Settled(record) = result else {
            panic!("expected settled");
        };
        assert_eq!(record.status, ExecutionControlStatus::Completed);
        assert!(record.settled_at.is_some());

        // Second settlement is idempotent.
        let result = settle(dir.path(), "sess-1", ExecutionSettlement::Completed).unwrap();
        assert!(matches!(result, SettleResult::AlreadySettled(_)));
    }

    #[test]
    fn settle_blocked_records_reason_and_missing_verification() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &active_record("sess-1")).unwrap();
        let result = settle(
            dir.path(),
            "sess-1",
            ExecutionSettlement::Blocked {
                reason: "E2E runner unavailable in this environment".to_string(),
                missing_verification: Some("managed-hook lifecycle E2E".to_string()),
            },
        )
        .unwrap();
        let SettleResult::Settled(record) = result else {
            panic!("expected settled");
        };
        assert_eq!(record.status, ExecutionControlStatus::Blocked);
        assert_eq!(
            record.blocked_reason.as_deref(),
            Some("E2E runner unavailable in this environment")
        );
        assert_eq!(
            record.missing_verification.as_deref(),
            Some("managed-hook lifecycle E2E")
        );
    }

    #[test]
    fn settle_without_record_is_idempotent_no_op() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            settle(dir.path(), "sess-1", ExecutionSettlement::Completed).unwrap(),
            SettleResult::NoRecord
        );
    }

    // Review follow-up: a resume must not re-arm the gate over a settled
    // record for the same owner, while fresh launches (and resumes of an
    // interrupted active execution) take over.
    #[test]
    fn resume_preserves_settled_record_for_same_owner() {
        let dir = tempfile::tempdir().unwrap();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-1",
            "launch",
            false,
        )
        .unwrap();
        settle(dir.path(), "sess-1", ExecutionSettlement::Completed).unwrap();

        // Resume with the same owner: settled record preserved.
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-2",
            "resume",
            true,
        )
        .unwrap();
        let record = load(dir.path()).unwrap().unwrap();
        assert_eq!(record.status, ExecutionControlStatus::Completed);
        assert_eq!(record.primary_session_id, "sess-1");

        // Resume of an ACTIVE record takes over (crash/window-close recovery).
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-3",
            "launch",
            false,
        )
        .unwrap();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-4",
            "resume",
            true,
        )
        .unwrap();
        let record = load(dir.path()).unwrap().unwrap();
        assert_eq!(record.status, ExecutionControlStatus::Active);
        assert_eq!(record.primary_session_id, "sess-4");

        // Fresh launch always takes over, even over a settled record.
        settle(dir.path(), "sess-4", ExecutionSettlement::Completed).unwrap();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-5",
            "launch",
            false,
        )
        .unwrap();
        let record = load(dir.path()).unwrap().unwrap();
        assert_eq!(record.status, ExecutionControlStatus::Active);
        assert_eq!(record.primary_session_id, "sess-5");
    }

    // P9a (T-117/T-118/T-123): takeovers are audited transfers, and the
    // chain survives subsequent takeovers.
    #[test]
    fn takeovers_append_audited_transfer_chain() {
        let dir = tempfile::tempdir().unwrap();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-1",
            "launch",
            false,
        )
        .unwrap();
        // Fresh launch takes over another session's ACTIVE record.
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-2",
            "launch",
            false,
        )
        .unwrap();
        // Resume takeover of the active record by a third session.
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-3",
            "resume",
            true,
        )
        .unwrap();
        let record = load(dir.path()).unwrap().unwrap();
        assert_eq!(record.primary_session_id, "sess-3");
        assert_eq!(record.transfers.len(), 2);
        assert_eq!(record.transfers[0].from_session_id, "sess-1");
        assert_eq!(record.transfers[0].to_session_id, "sess-2");
        assert_eq!(record.transfers[0].reason, "launch-takeover");
        assert_eq!(record.transfers[1].from_session_id, "sess-2");
        assert_eq!(record.transfers[1].to_session_id, "sess-3");
        assert_eq!(record.transfers[1].reason, "resume-takeover");
        assert!(integrity_ok(&record));
    }

    // P9a (T-122): a record edited outside the canonical operations fails
    // integrity validation — settlement refuses it.
    #[test]
    fn tampered_record_refuses_settlement_and_same_lifetime_repair() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &active_record("sess-1")).unwrap();
        // Naive direct edit: flip the status without recomputing the hash.
        let path = state_path(dir.path());
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("\"active\"", "\"completed\"");
        fs::write(&path, edited).unwrap();
        let loaded = load(dir.path()).unwrap().unwrap();
        assert!(!integrity_ok(&loaded), "edited record must fail integrity");

        assert_eq!(
            settle(dir.path(), "sess-1", ExecutionSettlement::Completed).unwrap(),
            SettleResult::Tampered
        );

        // A same-lifetime canonical rewrite must not launder the tamper: it
        // could otherwise sign a truncated recovery history as the baseline.
        // A genuinely fresh launch (new launched_at) remains available.
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("\"completed\"", "\"active\"")
            .replace("\"$gwt-execute\"", "\"$gwt-forged\"");
        fs::write(&path, edited).unwrap();
        let mut recovery_session =
            gwt_agent::Session::new(dir.path(), "main", gwt_agent::AgentId::Codex);
        recovery_session.id = "sess-2".to_string();
        let mut out = String::new();
        assert_eq!(
            run_adopt(
                dir.path(),
                "sess-2",
                &recovery_session,
                "tamper repair",
                &mut out,
            )
            .unwrap(),
            2,
            "{out}"
        );
        assert!(out.contains("execution.repair"), "{out}");
        let mut record = load(dir.path()).unwrap().unwrap();
        assert!(!integrity_ok(&record));
        record.transfers.push(OwnershipTransfer {
            from_session_id: record.primary_session_id.clone(),
            to_session_id: "sess-2".to_string(),
            reason: "tamper repair".to_string(),
            transferred_at: Utc::now(),
        });
        record.primary_session_id = "sess-2".to_string();
        let err = save(dir.path(), &record).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);

        let mut fresh = active_record("sess-2");
        fresh.launched_at = record.launched_at + chrono::Duration::nanoseconds(1);
        save(dir.path(), &fresh).unwrap();
        assert!(integrity_ok(&load(dir.path()).unwrap().unwrap()));
    }

    // T-107 helpers: entrypoint derivation from the launch argv.
    #[test]
    fn entrypoint_derives_skill_token_resume_or_launch() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            entrypoint_from_launch(&args(&["--flag", "$gwt-execute #3248"]), false),
            "gwt-execute"
        );
        assert_eq!(
            entrypoint_from_launch(&args(&["$gwt-build-spec SPEC-3248"]), false),
            "gwt-build-spec"
        );
        assert_eq!(
            entrypoint_from_launch(&args(&["--resume", "abc"]), true),
            "resume"
        );
        assert_eq!(entrypoint_from_launch(&args(&[]), false), "launch");
    }

    // T-107 helpers: owner kind from cached labels, defaulting to Issue.
    #[test]
    fn detect_owner_kind_defaults_issue_without_cache() {
        let dir = tempfile::tempdir().unwrap();
        // No git repo / no cache → plain Issue.
        assert_eq!(
            detect_owner_kind(dir.path(), 3248),
            ExecutionOwnerKind::Issue
        );
    }

    // ------------------------------------------------------------------
    // execution.complete / execution.blocked command behavior
    // ------------------------------------------------------------------

    mod command {
        use super::*;
        use crate::cli::{run_collect, CliCommand, TestEnv};

        fn run_cmd(
            repo: &Path,
            command: ExecutionCommand,
        ) -> Result<(i32, String), gwt_github::SpecOpsError> {
            if let ExecutionCommand::Reopen { reason } = &command {
                if let Some(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                {
                    let session_path =
                        gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
                    if !session_path.exists() {
                        let mut out = String::new();
                        let code = run_reopen(repo, &session_id, reason, &mut out)?;
                        return Ok((code, out));
                    }
                }
            }
            let mut env = TestEnv::new(repo.to_path_buf());
            run_collect(&mut env, CliCommand::Execution(command))
        }

        fn repair_authority_paths(worktree: &Path, owner: ExecutionOwnerKey) -> Vec<PathBuf> {
            let context = GenerationTransactionContext::resolve(worktree, owner).unwrap();
            vec![
                context.worktree_trusted_dir.join("execution-control.json"),
                state_path(worktree),
                context.worktree_trusted_dir.join(GENERATION_POINTER_FILE),
                generation_pointer_path(worktree),
                context.owner_dir.join(GENERATION_LEDGER_FILE),
            ]
        }

        fn authority_bytes(paths: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
            paths
                .iter()
                .map(|path| match fs::read(path) {
                    Ok(bytes) => Some(bytes),
                    Err(error) if error.kind() == ErrorKind::NotFound => None,
                    Err(error) => panic!("read {}: {error}", path.display()),
                })
                .collect()
        }

        fn normalized_test_path(path: &Path) -> String {
            let canonical = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let rendered = canonical.to_string_lossy();
            rendered
                .strip_prefix("/private")
                .unwrap_or(&rendered)
                .to_string()
        }

        fn mirror_pointer_partial_authority(
            worktree: &Path,
            owner: ExecutionOwnerKey,
        ) -> Vec<PathBuf> {
            let mut active = active_record("repair-session");
            active.owner_kind = owner.kind;
            active.owner_number = owner.number;
            save(worktree, &active).unwrap();
            ensure_generation_ledger(worktree, owner, LegacyActiveDisposition::Live).unwrap();
            let paths = repair_authority_paths(worktree, owner);
            fs::remove_file(&paths[2]).unwrap();
            fs::remove_file(&paths[4]).unwrap();
            assert!(paths[0].exists());
            assert!(paths[1].exists());
            assert!(paths[3].exists());
            paths
        }

        fn settle_blocked(repo: &Path, session: &str) -> ExecutionControlRecord {
            if load(repo).unwrap().is_none() {
                save(repo, &active_record(session)).unwrap();
            }
            let result = settle(
                repo,
                session,
                ExecutionSettlement::Blocked {
                    reason: "verification dependency unresolved".to_string(),
                    missing_verification: Some("full pre-PR matrix".to_string()),
                },
            )
            .unwrap();
            let SettleResult::Settled(record) = result else {
                panic!("expected blocked settlement");
            };
            record
        }

        fn save_build_state(repo: &Path, session_id: &str, owner_spec: Option<u64>, active: bool) {
            gwt_core::skill_state::save(
                repo,
                crate::cli::build::SKILL_NAME,
                &gwt_core::skill_state::SkillState {
                    active,
                    owner_spec,
                    started_at: Utc::now(),
                    phase: Some("verify".to_string()),
                    session_id: session_id.to_string(),
                },
            )
            .expect("save build lifecycle fixture");
        }

        fn seed_build_abort_work_authority(
            repo: &Path,
            session_id: &str,
            owner: ExecutionOwnerKey,
        ) {
            let session = gwt_agent::Session::load(
                &gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
            )
            .expect("load bound Session fixture");
            let worktree = dunce::canonicalize(repo).expect("canonical fixture worktree");
            let work_id = format!("work-build-abort-{}-{}", owner.kind.as_str(), owner.number);
            let now = Utc::now();
            let mut projection =
                gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo);
            projection
                .agents
                .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                    session_id: session_id.to_string(),
                    window_id: Some(format!("project::{session_id}")),
                    agent_id: session.agent_id.command().to_string(),
                    display_name: session.agent_id.display_name().to_string(),
                    status_category:
                        gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                    current_focus: None,
                    title_summary: None,
                    worktree_path: Some(worktree.clone()),
                    branch: Some(session.branch.clone()),
                    last_board_entry_id: None,
                    last_board_entry_kind: None,
                    coordination_scope: None,
                    affiliation_status:
                        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                    workspace_id: Some(work_id.clone()),
                    updated_at: now,
                });
            gwt_core::workspace_projection::save_workspace_projection(repo, &projection)
                .expect("save exact build.abort Session assignment");

            let mut start = gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Start,
                &work_id,
                now,
            );
            start.owner = Some(match owner.kind {
                ExecutionOwnerKind::Spec => format!("SPEC-{}", owner.number),
                ExecutionOwnerKind::Issue => format!("Issue #{}", owner.number),
            });
            start.status_category =
                Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
            start.agent_session_id = Some(session_id.to_string());
            start.agent_id = Some(session.agent_id.command().to_string());
            start.execution_container = Some(
                gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                    branch: Some(session.branch),
                    worktree_path: Some(worktree),
                    pr_number: None,
                    pr_url: None,
                    pr_state: None,
                },
            );
            let mut work_items = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
            work_items.apply_event(start);
            let work_items_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo);
            gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
                &work_items_path,
                &work_items,
            )
            .expect("save exact build.abort Work authority");
        }

        fn prepare_generation_bound_execution(
            repo: &Path,
            session_id: &str,
            owner_number: u64,
            status: ExecutionControlStatus,
        ) -> ExecutionOwnerKey {
            prepare_generation_bound_execution_for_owner(
                repo,
                session_id,
                ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: owner_number,
                },
                status,
            )
        }

        fn prepare_generation_bound_execution_for_owner(
            repo: &Path,
            session_id: &str,
            owner: ExecutionOwnerKey,
            status: ExecutionControlStatus,
        ) -> ExecutionOwnerKey {
            crate::cli::trusted_store::init_git_repo_with_origin(repo);
            let mut active = active_record(session_id);
            active.owner_kind = owner.kind;
            active.owner_number = owner.number;
            save(repo, &active).expect("save active execution fixture");
            ensure_generation_ledger(repo, owner, LegacyActiveDisposition::Live)
                .expect("materialize generation ledger fixture");
            let active_binding = current_execution_binding(repo, owner)
                .expect("load active execution binding")
                .expect("active execution binding");
            persist_generation_session_binding(repo, owner, session_id, active_binding);
            if status != ExecutionControlStatus::Active {
                let settlement = match status {
                    ExecutionControlStatus::Blocked => ExecutionSettlement::Blocked {
                        reason: "canonical verification is externally blocked".to_string(),
                        missing_verification: Some("full matrix".to_string()),
                    },
                    ExecutionControlStatus::Completed => ExecutionSettlement::Completed,
                    ExecutionControlStatus::Active => unreachable!(),
                };
                assert!(matches!(
                    settle(repo, session_id, settlement).expect("settle execution fixture"),
                    SettleResult::Settled(_)
                ));
            }
            owner
        }

        fn status_snapshot(repo: &Path, session_id: &str) -> serde_json::Value {
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
            let (code, out) =
                run_cmd(repo, ExecutionCommand::Status).expect("run execution.status");
            assert_eq!(code, 0, "{out}");
            serde_json::from_str(out.trim()).expect("parse execution.status output")
        }

        fn write_failing_git_recorder(bin_dir: &Path) -> PathBuf {
            fs::create_dir_all(bin_dir).expect("create fake git directory");
            #[cfg(windows)]
            {
                let fake_git = bin_dir.join("git.cmd");
                fs::write(
                    &fake_git,
                    "@echo off\r\nif not \"%GWT_FAKE_GIT_LOG%\"==\"\" echo %*>>\"%GWT_FAKE_GIT_LOG%\"\r\nexit /b 1\r\n",
                )
                .expect("write fake git recorder");
                fake_git
            }
            #[cfg(not(windows))]
            {
                let fake_git = bin_dir.join("git");
                fs::write(
                    &fake_git,
                    r#"#!/bin/sh
if [ -n "$GWT_FAKE_GIT_LOG" ]; then
  printf '%s\n' "$*" >> "$GWT_FAKE_GIT_LOG"
fi
exit 1
"#,
                )
                .expect("write fake git recorder");
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o755))
                    .expect("make fake git executable");
                fake_git
            }
        }

        fn assert_projection_has_no_protected_recovery(snapshot: &ExecutionDiagnosisSnapshot) {
            assert!(
                snapshot.recovery_probes.is_empty(),
                "projection must leave protected recovery probes unevaluated"
            );
            for operation in PROTECTED_RECOVERY_OPERATIONS {
                assert!(
                    !snapshot
                        .available_recoveries
                        .iter()
                        .any(|candidate| candidate == operation),
                    "projection must not advertise unvalidated protected recovery {operation}"
                );
            }
        }

        #[test]
        fn status_is_read_only_and_reports_missing_execution_without_session() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
            let dir = tempfile::tempdir().unwrap();

            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Status).unwrap();

            assert_eq!(code, 0, "{out}");
            let snapshot: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
            assert_eq!(snapshot["ecr_status"], "missing");
            assert_eq!(snapshot["binding_state"], "missing");
            assert_eq!(
                snapshot["available_recoveries"],
                serde_json::json!(["gwt-execute"])
            );
            let probes = snapshot["recovery_probes"]
                .as_array()
                .expect("status recovery probes");
            assert_eq!(probes.len(), 7);
            assert!(probes.iter().all(|probe| {
                probe["state"] == "unavailable"
                    && probe["reason"] == "execution_recovery_scope_invalid"
            }));
        }

        #[test]
        fn blocked_output_guides_build_abort_when_matching_build_lifecycle_remains() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().expect("trusted store home");
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let session_id = "session-blocked-output";
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
            let repo = tempfile::tempdir().expect("execution fixture");
            prepare_generation_bound_execution(
                repo.path(),
                session_id,
                3248,
                ExecutionControlStatus::Active,
            );
            save_build_state(repo.path(), session_id, Some(3248), true);
            seed_build_abort_work_authority(
                repo.path(),
                session_id,
                ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 3248,
                },
            );

            let (code, out) = run_cmd(
                repo.path(),
                ExecutionCommand::Blocked {
                    reason: "canonical verification is externally blocked".to_string(),
                    missing_verification: Some("full matrix".to_string()),
                },
            )
            .expect("settle execution as blocked");

            assert_eq!(code, 0, "{out}");
            assert!(out.contains("build.abort"), "{out}");
            assert!(
                out.contains("active build lifecycle remains for spec #3248;"),
                "{out}"
            );
            assert!(
                gwt_core::skill_state::load(repo.path(), crate::cli::build::SKILL_NAME)
                    .expect("load build lifecycle after blocked settlement")
                    .expect("build lifecycle after blocked settlement")
                    .active,
                "execution.blocked must guide cleanup without auto-aborting the build lifecycle"
            );
        }

        #[test]
        fn blocked_output_uses_issue_owner_label_in_build_abort_guidance() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().expect("trusted store home");
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let session_id = "session-blocked-issue-output";
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
            let repo = tempfile::tempdir().expect("execution fixture");
            prepare_generation_bound_execution_for_owner(
                repo.path(),
                session_id,
                ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Issue,
                    number: 3580,
                },
                ExecutionControlStatus::Active,
            );
            save_build_state(repo.path(), session_id, Some(3580), true);
            seed_build_abort_work_authority(
                repo.path(),
                session_id,
                ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Issue,
                    number: 3580,
                },
            );

            let (code, out) = run_cmd(
                repo.path(),
                ExecutionCommand::Blocked {
                    reason: "canonical verification is externally blocked".to_string(),
                    missing_verification: Some("full matrix".to_string()),
                },
            )
            .expect("settle Issue execution as blocked");

            assert_eq!(code, 0, "{out}");
            assert!(
                out.contains("active build lifecycle remains for issue #3580;"),
                "{out}"
            );
        }

        #[test]
        fn status_advertises_build_abort_only_for_matching_blocked_build_lifecycle() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().expect("trusted store home");
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

            let matching = tempfile::tempdir().expect("matching fixture");
            let matching_session = "session-status-matching";
            prepare_generation_bound_execution(
                matching.path(),
                matching_session,
                3248,
                ExecutionControlStatus::Blocked,
            );
            save_build_state(matching.path(), matching_session, Some(3248), true);
            seed_build_abort_work_authority(
                matching.path(),
                matching_session,
                ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 3248,
                },
            );
            let matching_status = status_snapshot(matching.path(), matching_session);
            assert!(
                matching_status["available_recoveries"]
                    .as_array()
                    .expect("matching available_recoveries")
                    .iter()
                    .any(|recovery| recovery == "build.abort"),
                "matching Blocked execution and active build lifecycle must advertise build.abort: {matching_status:?}"
            );

            let no_build = tempfile::tempdir().expect("no-build fixture");
            let no_build_session = "session-status-no-build";
            prepare_generation_bound_execution(
                no_build.path(),
                no_build_session,
                3249,
                ExecutionControlStatus::Blocked,
            );

            let mismatched_owner = tempfile::tempdir().expect("owner-mismatch fixture");
            let mismatched_owner_session = "session-status-owner-mismatch";
            prepare_generation_bound_execution(
                mismatched_owner.path(),
                mismatched_owner_session,
                3250,
                ExecutionControlStatus::Blocked,
            );
            save_build_state(
                mismatched_owner.path(),
                mismatched_owner_session,
                Some(9999),
                true,
            );

            let foreign = tempfile::tempdir().expect("foreign-session fixture");
            let foreign_session = "session-status-foreign";
            prepare_generation_bound_execution(
                foreign.path(),
                foreign_session,
                3251,
                ExecutionControlStatus::Blocked,
            );
            save_build_state(foreign.path(), "session-other", Some(3251), true);

            let completed = tempfile::tempdir().expect("completed fixture");
            let completed_session = "session-status-completed";
            prepare_generation_bound_execution(
                completed.path(),
                completed_session,
                3252,
                ExecutionControlStatus::Completed,
            );
            save_build_state(completed.path(), completed_session, Some(3252), true);

            let corrupt_build = tempfile::tempdir().expect("corrupt-build fixture");
            let corrupt_build_session = "session-status-corrupt-build";
            prepare_generation_bound_execution(
                corrupt_build.path(),
                corrupt_build_session,
                3253,
                ExecutionControlStatus::Blocked,
            );
            let corrupt_build_path = gwt_core::skill_state::state_path(
                corrupt_build.path(),
                crate::cli::build::SKILL_NAME,
            );
            fs::create_dir_all(corrupt_build_path.parent().expect("build state parent"))
                .expect("create corrupt build state parent");
            fs::write(corrupt_build_path, b"{corrupt").expect("write corrupt build state");

            let corrupt_execution = tempfile::tempdir().expect("corrupt-execution fixture");
            let corrupt_execution_session = "session-status-corrupt-execution";
            prepare_generation_bound_execution(
                corrupt_execution.path(),
                corrupt_execution_session,
                3254,
                ExecutionControlStatus::Blocked,
            );
            save_build_state(
                corrupt_execution.path(),
                corrupt_execution_session,
                Some(3254),
                true,
            );
            let corrupt_execution_path =
                crate::cli::trusted_store::trusted_dir_for_worktree(corrupt_execution.path())
                    .expect("trusted execution directory")
                    .join("execution-control.json");
            fs::write(corrupt_execution_path, b"{corrupt")
                .expect("write corrupt execution control");

            for (reason, repo, session_id) in [
                ("no build", no_build.path(), no_build_session),
                (
                    "owner mismatch",
                    mismatched_owner.path(),
                    mismatched_owner_session,
                ),
                ("foreign session", foreign.path(), foreign_session),
                ("completed", completed.path(), completed_session),
                ("corrupt build", corrupt_build.path(), corrupt_build_session),
                (
                    "corrupt execution",
                    corrupt_execution.path(),
                    corrupt_execution_session,
                ),
            ] {
                let snapshot = status_snapshot(repo, session_id);
                assert!(
                    !snapshot["available_recoveries"]
                        .as_array()
                        .expect("available_recoveries")
                        .iter()
                        .any(|recovery| recovery == "build.abort"),
                    "{reason} must not advertise build.abort: {snapshot:?}"
                );
            }
        }

        #[test]
        fn status_does_not_advertise_build_abort_when_exact_work_preflight_is_unavailable() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().expect("trusted store home");
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let repo = tempfile::tempdir().expect("projectionless blocked fixture");
            let session_id = "session-status-build-abort-without-work-authority";
            prepare_generation_bound_execution(
                repo.path(),
                session_id,
                3587,
                ExecutionControlStatus::Blocked,
            );
            save_build_state(repo.path(), session_id, Some(3587), true);

            let diagnosis = diagnose(repo.path(), Some(session_id));

            assert!(
                !diagnosis
                    .available_recoveries
                    .contains(&"build.abort".to_string()),
                "an unavailable exact Work preflight must remove build.abort: {diagnosis:?}"
            );
            let probe = diagnosis
                .recovery_probes
                .iter()
                .find(|probe| probe.operation == "build.abort")
                .expect("build.abort operation-local recovery probe");
            assert_eq!(
                probe.state,
                crate::cli::governance::RecoveryProbeState::Unavailable
            );
            assert_eq!(
                probe.governance.cause,
                Some(crate::cli::governance::GovernanceCause::Authority)
            );
            assert_eq!(probe.governance.retryable, Some(false));
            assert!(
                probe
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("Session assignment")),
                "{probe:?}"
            );
        }

        fn assert_all_operation_local_recovery_probes(snapshot: &ExecutionDiagnosisSnapshot) {
            let mut operations = snapshot
                .recovery_probes
                .iter()
                .map(|probe| probe.operation.as_str())
                .collect::<Vec<_>>();
            operations.sort_unstable();
            assert_eq!(
                operations,
                vec![
                    "build.abort",
                    "execution.adopt",
                    "execution.continue",
                    "execution.reopen",
                    "execution.repair",
                    "workspace.ensure",
                    "workspace.update",
                ],
            );
            for probe in &snapshot.recovery_probes {
                assert_eq!(
                    snapshot.available_recoveries.contains(&probe.operation),
                    probe.state == crate::cli::governance::RecoveryProbeState::Available,
                    "{} advertisement must exactly match its operation-local evaluator",
                    probe.operation,
                );
            }
        }

        #[derive(Debug, PartialEq, Eq)]
        struct RecoveryOperationAuthorityBytes {
            generation: GenerationAuthorityBytes,
            sessions: Vec<(String, Option<Vec<u8>>)>,
            capabilities: Vec<(String, Option<String>)>,
            work: Vec<(PathBuf, Option<Vec<u8>>)>,
        }

        fn recovery_operation_authority_bytes(
            worktree: &Path,
            owner: ExecutionOwnerKey,
            session_ids: &[&str],
        ) -> RecoveryOperationAuthorityBytes {
            let sessions = session_ids
                .iter()
                .map(|session_id| {
                    let path =
                        gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
                    ((*session_id).to_string(), fs::read(path).ok())
                })
                .collect();
            let capabilities = session_ids
                .iter()
                .map(|session_id| {
                    let path =
                        gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
                    let capability = gwt_agent::Session::load(&path)
                        .ok()
                        .and_then(|session| session.execution_binding)
                        .map(|binding| format!("{binding:?}"));
                    ((*session_id).to_string(), capability)
                })
                .collect();
            let work_paths = [
                gwt_core::paths::gwt_workspace_projection_path_for_repo_path(worktree),
                gwt_core::paths::gwt_workspace_journal_path_for_repo_path(worktree),
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(worktree),
                gwt_core::paths::gwt_repo_local_work_events_path(worktree),
            ];
            let work = work_paths
                .into_iter()
                .map(|path| {
                    let bytes = fs::read(&path).ok();
                    (path, bytes)
                })
                .collect();
            RecoveryOperationAuthorityBytes {
                generation: generation_authority_bytes(worktree, owner),
                sessions,
                capabilities,
                work,
            }
        }

        fn seed_recovery_work_bytes(worktree: &Path) {
            for (index, path) in [
                gwt_core::paths::gwt_workspace_projection_path_for_repo_path(worktree),
                gwt_core::paths::gwt_workspace_journal_path_for_repo_path(worktree),
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(worktree),
                gwt_core::paths::gwt_repo_local_work_events_path(worktree),
            ]
            .into_iter()
            .enumerate()
            {
                fs::create_dir_all(path.parent().expect("recovery Work path parent")).unwrap();
                fs::write(path, format!("recovery-work-sentinel-{index}\n")).unwrap();
            }
        }

        fn snapshot_recovery_authority_files(worktree: &Path) -> Vec<(PathBuf, Vec<u8>)> {
            fn visit(root: &Path, current: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
                let mut entries = fs::read_dir(current)
                    .expect("read recovery authority directory")
                    .collect::<io::Result<Vec<_>>>()
                    .expect("read recovery authority entries");
                entries.sort_by_key(fs::DirEntry::file_name);
                for entry in entries {
                    let path = entry.path();
                    let file_type = entry.file_type().expect("read recovery authority type");
                    if file_type.is_dir() {
                        visit(root, &path, files);
                    } else if file_type.is_file()
                        && !path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.ends_with(".lock") || name == ".write-lease")
                    {
                        files.push((
                            path.strip_prefix(root)
                                .expect("recovery authority belongs to trusted root")
                                .to_path_buf(),
                            fs::read(&path).expect("read recovery authority bytes"),
                        ));
                    }
                }
            }

            let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(worktree)
                .expect("trusted worktree directory");
            let trusted_root = trusted_dir
                .parent()
                .expect("trusted worktree directory has repository root");
            let mut files = Vec::new();
            visit(trusted_root, trusted_root, &mut files);
            files
        }

        fn recovery_probe<'a>(
            snapshot: &'a ExecutionDiagnosisSnapshot,
            operation: &str,
        ) -> &'a crate::cli::governance::RecoveryProbe {
            snapshot
                .recovery_probes
                .iter()
                .find(|probe| probe.operation == operation)
                .unwrap_or_else(|| panic!("missing {operation} recovery probe"))
        }

        #[test]
        fn adopt_revalidates_changed_session_before_authority_mutation() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let session_id = "session-adopting";
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);

            for race_index in 0..3 {
                let repo = tempfile::tempdir().unwrap();
                crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
                let owner = ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 3248 + race_index,
                };
                let mut active = active_record("session-original");
                active.owner_number = owner.number;
                save(repo.path(), &active).unwrap();
                ensure_generation_ledger(repo.path(), owner, LegacyActiveDisposition::Live)
                    .unwrap();
                seed_recovery_work_bytes(repo.path());
                let session = persist_recovery_session_snapshot(repo.path(), owner, session_id);
                let before = recovery_operation_authority_bytes(
                    repo.path(),
                    owner,
                    &["session-original", session_id],
                );

                let expected_session = match race_index {
                    0 => {
                        set_recovery_session_race(RecoverySessionRace::Delete);
                        None
                    }
                    1 => {
                        let mut replacement = session.clone();
                        replacement.agent_id = gwt_agent::AgentId::ClaudeCode;
                        let expected = toml::to_string_pretty(&replacement)
                            .expect("serialize replacement Session")
                            .into_bytes();
                        set_recovery_session_race(RecoverySessionRace::Replace(Box::new(
                            replacement,
                        )));
                        Some(expected)
                    }
                    2 => {
                        set_recovery_session_race(RecoverySessionRace::Corrupt);
                        Some(b"broken = [".to_vec())
                    }
                    _ => unreachable!(),
                };

                let (code, out) = run_cmd(
                    repo.path(),
                    ExecutionCommand::Adopt {
                        reason: "recover crashed owner".to_string(),
                    },
                )
                .expect("stale recovery preflight must return a typed refusal");
                assert_eq!(code, 2, "{out}");
                assert!(
                    out.contains(RECOVERY_SESSION_CHANGED_PREFIX)
                        || out.contains("recovery scope is invalid"),
                    "{out}"
                );

                let after = recovery_operation_authority_bytes(
                    repo.path(),
                    owner,
                    &["session-original", session_id],
                );
                assert_eq!(after.generation, before.generation);
                assert_eq!(after.work, before.work);
                assert_eq!(after.sessions[1].1, expected_session);
                assert_eq!(after.capabilities[1].1, None);
            }
        }

        #[test]
        fn reopen_revalidates_changed_session_before_authority_mutation() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let session_id = "session-reopening";
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);

            for race_index in 0..3 {
                let repo = tempfile::tempdir().unwrap();
                crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
                let remote = format!("https://example.invalid/reopen-race-{race_index}.git");
                let remote_update = gwt_core::process::run_git_logged(
                    &["remote", "set-url", "origin", &remote],
                    Some(repo.path()),
                )
                .unwrap();
                assert!(remote_update.status.success());
                let owner = ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 3248,
                };
                let mut active = active_record(session_id);
                active.owner_number = owner.number;
                save(repo.path(), &active).unwrap();
                ensure_generation_ledger(repo.path(), owner, LegacyActiveDisposition::Live)
                    .unwrap();
                let binding = current_execution_binding(repo.path(), owner)
                    .unwrap()
                    .expect("current reopen binding");
                persist_generation_session_binding(repo.path(), owner, session_id, binding);
                settle_blocked(repo.path(), session_id);
                seed_recovery_work_bytes(repo.path());
                save_covering_evidence(repo.path(), session_id, true);
                let session_path =
                    gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
                let session = gwt_agent::Session::load(&session_path).unwrap();
                let authority_before = snapshot_recovery_authority_files(repo.path());
                let work_before =
                    recovery_operation_authority_bytes(repo.path(), owner, &[session_id]).work;

                let expected_session = match race_index {
                    0 => {
                        set_recovery_session_race(RecoverySessionRace::Delete);
                        None
                    }
                    1 => {
                        let mut replacement = session.clone();
                        replacement.agent_id = gwt_agent::AgentId::ClaudeCode;
                        let expected = toml::to_string_pretty(&replacement)
                            .expect("serialize replacement Session")
                            .into_bytes();
                        set_recovery_session_race(RecoverySessionRace::Replace(Box::new(
                            replacement,
                        )));
                        Some(expected)
                    }
                    2 => {
                        set_recovery_session_race(RecoverySessionRace::Corrupt);
                        Some(b"broken = [".to_vec())
                    }
                    _ => unreachable!(),
                };

                let result = run_cmd(
                    repo.path(),
                    ExecutionCommand::Reopen {
                        reason: "fresh evidence is available".to_string(),
                    },
                );
                let (code, out) = result.expect("stale reopen preflight must return a refusal");
                assert_eq!(code, 2, "{out}");
                assert!(out.contains(RECOVERY_SESSION_CHANGED_PREFIX), "{out}");
                assert_eq!(
                    snapshot_recovery_authority_files(repo.path()),
                    authority_before
                );
                assert_eq!(
                    recovery_operation_authority_bytes(repo.path(), owner, &[session_id]).work,
                    work_before
                );
                assert_eq!(fs::read(&session_path).ok(), expected_session);
            }
        }

        #[test]
        fn repair_refuses_owner_replacement_between_discovery_and_activation() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
            let original_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3269,
            };
            let foreign_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3270,
            };
            let session_id = "session-repair-owner-race";
            let foreign_session_id = "session-foreign-owner";
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
            let mut active = active_record(session_id);
            active.owner_number = original_owner.number;
            save(repo.path(), &active).unwrap();
            ensure_generation_ledger(repo.path(), original_owner, LegacyActiveDisposition::Live)
                .unwrap();
            persist_recovery_session_snapshot(repo.path(), original_owner, session_id);
            seed_recovery_work_bytes(repo.path());
            let trusted_dir =
                crate::cli::trusted_store::trusted_dir_for_worktree(repo.path()).unwrap();
            fs::write(trusted_dir.join("execution-control.json"), b"{corrupt").unwrap();

            let raced_snapshot = std::rc::Rc::new(std::cell::RefCell::new(None));
            let raced_snapshot_for_hook = std::rc::Rc::clone(&raced_snapshot);
            set_repair_owner_activation_race(move |worktree| {
                replace_current_generation_authority(worktree, foreign_owner);
                let binding = current_execution_binding(worktree, foreign_owner)
                    .unwrap()
                    .expect("foreign generation binding");
                persist_generation_session_binding(
                    worktree,
                    foreign_owner,
                    foreign_session_id,
                    binding,
                );
                raced_snapshot_for_hook.replace(Some((
                    snapshot_recovery_authority_files(worktree),
                    recovery_operation_authority_bytes(
                        worktree,
                        foreign_owner,
                        &[session_id, foreign_session_id],
                    ),
                )));
            });

            let (code, out) = run_cmd(
                repo.path(),
                ExecutionCommand::Repair {
                    reason: "recover corrupt authority".to_string(),
                },
            )
            .expect("owner replacement must return a typed refusal");
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("generation activation CAS lost"), "{out}");

            let (authority_after_race, state_after_race) = raced_snapshot
                .borrow_mut()
                .take()
                .expect("foreign owner activation snapshot");
            assert_eq!(
                snapshot_recovery_authority_files(repo.path()),
                authority_after_race,
                "repair must not mutate foreign owner authority"
            );
            assert_eq!(
                recovery_operation_authority_bytes(
                    repo.path(),
                    foreign_owner,
                    &[session_id, foreign_session_id],
                ),
                state_after_race,
                "repair must preserve foreign ECR, pointer, ledger, Sessions, and Work bytes"
            );
        }

        #[test]
        fn adopt_satisfied_revalidates_changed_session_before_success() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let session_id = "session-adopt-satisfied";
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);

            for race_index in 0..3 {
                let repo = tempfile::tempdir().unwrap();
                crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
                let owner = ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 3271 + race_index,
                };
                let mut active = active_record(session_id);
                active.owner_number = owner.number;
                save(repo.path(), &active).unwrap();
                ensure_generation_ledger(repo.path(), owner, LegacyActiveDisposition::Live)
                    .unwrap();
                seed_recovery_work_bytes(repo.path());
                let session = persist_recovery_session_snapshot(repo.path(), owner, session_id);
                let authority_before = snapshot_recovery_authority_files(repo.path());
                let work_before =
                    recovery_operation_authority_bytes(repo.path(), owner, &[session_id]).work;
                let session_path =
                    gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));

                let expected_session = match race_index {
                    0 => {
                        set_recovery_session_race(RecoverySessionRace::Delete);
                        None
                    }
                    1 => {
                        let mut replacement = session.clone();
                        replacement.agent_id = gwt_agent::AgentId::ClaudeCode;
                        let expected = toml::to_string_pretty(&replacement)
                            .expect("serialize replacement Session")
                            .into_bytes();
                        set_recovery_session_race(RecoverySessionRace::Replace(Box::new(
                            replacement,
                        )));
                        Some(expected)
                    }
                    2 => {
                        set_recovery_session_race(RecoverySessionRace::Corrupt);
                        Some(b"broken = [".to_vec())
                    }
                    _ => unreachable!(),
                };

                let (code, out) = run_cmd(
                    repo.path(),
                    ExecutionCommand::Adopt {
                        reason: "idempotent recovery".to_string(),
                    },
                )
                .expect("stale satisfied adopt must return a typed refusal");
                assert_eq!(code, 2, "{out}");
                assert!(out.contains(RECOVERY_SESSION_CHANGED_PREFIX), "{out}");
                assert_eq!(
                    snapshot_recovery_authority_files(repo.path()),
                    authority_before
                );
                assert_eq!(
                    recovery_operation_authority_bytes(repo.path(), owner, &[session_id]).work,
                    work_before
                );
                assert_eq!(fs::read(&session_path).ok(), expected_session);
            }
        }

        #[test]
        fn reopen_satisfied_revalidates_changed_session_before_legacy_upgrade() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let session_id = "session-reopen-satisfied";
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);

            for race_index in 0..3 {
                let repo = tempfile::tempdir().unwrap();
                crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
                let owner = ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 3274 + race_index,
                };
                let now = Utc::now();
                let mut transitional = active_record(session_id);
                transitional.owner_number = owner.number;
                transitional.recoveries.push(ExecutionRecovery {
                    session_id: session_id.to_string(),
                    reason: "initial recovery schema".to_string(),
                    prior_blocked_reason: Some("temporary blocker".to_string()),
                    prior_missing_verification: None,
                    blocked_at: now,
                    verification_record_id: "vrr-transition".to_string(),
                    verification_run_hash: "run-hash".to_string(),
                    verification_plan_hash: "plan-hash".to_string(),
                    verification_plan_created_at: now,
                    plan_derived: true,
                    worktree_fingerprint: "fingerprint".to_string(),
                    verification_started_at: now,
                    verification_created_at: now,
                    reopened_at: now,
                    previous_recovery_hash: String::new(),
                    content_hash: String::new(),
                });
                transitional.content_hash = compute_legacy_hash_with_recoveries(&transitional);
                let serialized = serde_json::to_vec_pretty(&transitional).unwrap();
                let trusted_dir =
                    crate::cli::trusted_store::trusted_dir_for_worktree(repo.path()).unwrap();
                fs::create_dir_all(&trusted_dir).unwrap();
                fs::write(trusted_dir.join("execution-control.json"), &serialized).unwrap();
                let mirror = state_path(repo.path());
                fs::create_dir_all(mirror.parent().unwrap()).unwrap();
                fs::write(&mirror, &serialized).unwrap();
                assert!(recovery_storage_needs_upgrade(repo.path()).unwrap());
                seed_recovery_work_bytes(repo.path());
                let session = persist_recovery_session_snapshot(repo.path(), owner, session_id);
                let authority_before = snapshot_recovery_authority_files(repo.path());
                let mirror_before = fs::read(&mirror).unwrap();
                let work_before = [
                    gwt_core::paths::gwt_workspace_projection_path_for_repo_path(repo.path()),
                    gwt_core::paths::gwt_workspace_journal_path_for_repo_path(repo.path()),
                    gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo.path()),
                    gwt_core::paths::gwt_repo_local_work_events_path(repo.path()),
                ]
                .into_iter()
                .map(|path| (path.clone(), fs::read(path).ok()))
                .collect::<Vec<_>>();
                let session_path =
                    gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));

                let expected_session = match race_index {
                    0 => {
                        set_recovery_session_race(RecoverySessionRace::Delete);
                        None
                    }
                    1 => {
                        let mut replacement = session.clone();
                        replacement.agent_id = gwt_agent::AgentId::ClaudeCode;
                        let expected = toml::to_string_pretty(&replacement)
                            .expect("serialize replacement Session")
                            .into_bytes();
                        set_recovery_session_race(RecoverySessionRace::Replace(Box::new(
                            replacement,
                        )));
                        Some(expected)
                    }
                    2 => {
                        set_recovery_session_race(RecoverySessionRace::Corrupt);
                        Some(b"broken = [".to_vec())
                    }
                    _ => unreachable!(),
                };

                let (code, out) = run_cmd(
                    repo.path(),
                    ExecutionCommand::Reopen {
                        reason: "idempotent recovery".to_string(),
                    },
                )
                .expect("stale satisfied reopen must return a typed refusal");
                assert_eq!(code, 2, "{out}");
                assert!(out.contains(RECOVERY_SESSION_CHANGED_PREFIX), "{out}");
                assert_eq!(
                    snapshot_recovery_authority_files(repo.path()),
                    authority_before
                );
                assert_eq!(fs::read(&mirror).unwrap(), mirror_before);
                assert_eq!(
                    [
                        gwt_core::paths::gwt_workspace_projection_path_for_repo_path(repo.path()),
                        gwt_core::paths::gwt_workspace_journal_path_for_repo_path(repo.path()),
                        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo.path()),
                        gwt_core::paths::gwt_repo_local_work_events_path(repo.path()),
                    ]
                    .into_iter()
                    .map(|path| (path.clone(), fs::read(path).ok()))
                    .collect::<Vec<_>>(),
                    work_before
                );
                assert_eq!(fs::read(&session_path).ok(), expected_session);
            }
        }

        #[test]
        fn repair_revalidates_changed_session_before_authority_mutation() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let session_id = "session-repairing";
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);

            for race_index in 0..3 {
                let repo = tempfile::tempdir().unwrap();
                crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
                let owner = ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 3270 + race_index,
                };
                let mut active = active_record(session_id);
                active.owner_number = owner.number;
                save(repo.path(), &active).unwrap();
                ensure_generation_ledger(repo.path(), owner, LegacyActiveDisposition::Live)
                    .unwrap();
                let session = persist_recovery_session_snapshot(repo.path(), owner, session_id);
                seed_recovery_work_bytes(repo.path());
                let trusted_dir =
                    crate::cli::trusted_store::trusted_dir_for_worktree(repo.path()).unwrap();
                fs::write(trusted_dir.join("execution-control.json"), b"{corrupt").unwrap();
                let session_path =
                    gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
                let authority_before = snapshot_recovery_authority_files(repo.path());
                let work_before =
                    recovery_operation_authority_bytes(repo.path(), owner, &[session_id]).work;

                let expected_session = match race_index {
                    0 => {
                        set_recovery_session_race(RecoverySessionRace::Delete);
                        None
                    }
                    1 => {
                        let mut replacement = session.clone();
                        replacement.agent_id = gwt_agent::AgentId::ClaudeCode;
                        let expected = toml::to_string_pretty(&replacement)
                            .expect("serialize replacement Session")
                            .into_bytes();
                        set_recovery_session_race(RecoverySessionRace::Replace(Box::new(
                            replacement,
                        )));
                        Some(expected)
                    }
                    2 => {
                        set_recovery_session_race(RecoverySessionRace::Corrupt);
                        Some(b"broken = [".to_vec())
                    }
                    _ => unreachable!(),
                };

                let result = run_cmd(
                    repo.path(),
                    ExecutionCommand::Repair {
                        reason: "recover corrupt authority".to_string(),
                    },
                );
                let (code, out) = result.expect("stale repair preflight must return a refusal");
                assert_eq!(code, 2, "{out}");
                assert!(out.contains(RECOVERY_SESSION_CHANGED_PREFIX), "{out}");
                assert_eq!(
                    snapshot_recovery_authority_files(repo.path()),
                    authority_before
                );
                assert_eq!(
                    recovery_operation_authority_bytes(repo.path(), owner, &[session_id]).work,
                    work_before
                );
                assert_eq!(fs::read(&session_path).ok(), expected_session);
            }
        }

        #[test]
        fn repair_binding_cas_failure_rolls_back_authority_and_refuses() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3280,
            };
            let session_id = "session-repair-binding-cas";
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
            let mut active = active_record(session_id);
            active.owner_number = owner.number;
            save(repo.path(), &active).unwrap();
            ensure_generation_ledger(repo.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let expected_session =
                persist_recovery_session_snapshot(repo.path(), owner, session_id);
            seed_recovery_work_bytes(repo.path());
            let trusted_dir =
                crate::cli::trusted_store::trusted_dir_for_worktree(repo.path()).unwrap();
            fs::write(trusted_dir.join("execution-control.json"), b"{corrupt").unwrap();
            let before = recovery_operation_authority_bytes(repo.path(), owner, &[session_id]);
            let context = GenerationTransactionContext::resolve(repo.path(), owner).unwrap();
            let audit_path = repair_audit_dir(&context)
                .unwrap()
                .join(EXECUTION_REPAIR_AUDIT_FILE);
            let audit_before = fs::read(&audit_path).ok();

            let mut replacement = expected_session.clone();
            replacement.agent_id = gwt_agent::AgentId::ClaudeCode;
            let replacement_bytes = toml::to_string_pretty(&replacement)
                .expect("serialize replacement Session")
                .into_bytes();
            set_repair_binding_session_race(RecoverySessionRace::Replace(Box::new(replacement)));

            let (code, out) = run_cmd(
                repo.path(),
                ExecutionCommand::Repair {
                    reason: "recover corrupt authority".to_string(),
                },
            )
            .expect("binding CAS loss must return a typed refusal");

            assert_eq!(code, 2, "{out}");
            assert!(out.contains(RECOVERY_SESSION_CHANGED_PREFIX), "{out}");
            assert_eq!(
                fs::read(gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")))
                    .unwrap(),
                replacement_bytes,
                "repair binding CAS must not overwrite a same-id replacement Session"
            );
            let after = recovery_operation_authority_bytes(repo.path(), owner, &[session_id]);
            assert_eq!(after.generation, before.generation);
            assert_eq!(after.work, before.work);
            assert_eq!(fs::read(&audit_path).ok(), audit_before);
        }

        #[test]
        fn repair_binding_refuses_a_foreign_current_generation() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
            let repair_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3281,
            };
            let foreign_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3282,
            };
            let session_id = "session-repair-foreign-generation";
            let mut active = active_record(session_id);
            active.owner_number = repair_owner.number;
            save(repo.path(), &active).unwrap();
            ensure_generation_ledger(repo.path(), repair_owner, LegacyActiveDisposition::Live)
                .unwrap();
            let expected_session =
                persist_recovery_session_snapshot(repo.path(), repair_owner, session_id);
            let stale_binding = current_execution_binding(repo.path(), repair_owner)
                .unwrap()
                .expect("repair generation binding");

            let foreign_authority =
                replace_current_generation_authority(repo.path(), foreign_owner);
            let session_path =
                gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
            let session_before = fs::read(&session_path).unwrap();

            let error = repair_session_binding_if_unchanged(
                repo.path(),
                &expected_session,
                repair_owner,
                stale_binding,
            )
            .expect_err("foreign current generation must fence stale repair binding");
            assert!(error.to_string().contains("generation activation CAS lost"));
            assert_eq!(
                generation_authority_bytes(repo.path(), foreign_owner),
                foreign_authority
            );
            assert_eq!(fs::read(&session_path).unwrap(), session_before);
        }

        #[test]
        fn repair_foreign_generation_race_refuses_without_stale_session_binding() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
            let repair_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3283,
            };
            let foreign_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3284,
            };
            let session_id = "session-repair-generation-race";
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
            let mut active = active_record(session_id);
            active.owner_number = repair_owner.number;
            save(repo.path(), &active).unwrap();
            ensure_generation_ledger(repo.path(), repair_owner, LegacyActiveDisposition::Live)
                .unwrap();
            persist_recovery_session_snapshot(repo.path(), repair_owner, session_id);
            seed_recovery_work_bytes(repo.path());
            let trusted_dir =
                crate::cli::trusted_store::trusted_dir_for_worktree(repo.path()).unwrap();
            fs::write(trusted_dir.join("execution-control.json"), b"{corrupt").unwrap();
            let session_path =
                gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
            let session_before = fs::read(&session_path).unwrap();
            let work_before =
                recovery_operation_authority_bytes(repo.path(), repair_owner, &[session_id]).work;
            set_repair_binding_authority_race(foreign_owner);

            let (code, out) = run_cmd(
                repo.path(),
                ExecutionCommand::Repair {
                    reason: "recover corrupt authority".to_string(),
                },
            )
            .expect("foreign generation race must return a typed refusal");
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("generation activation CAS lost"), "{out}");
            assert_eq!(
                current_generation_owner(repo.path()).unwrap(),
                Some(foreign_owner)
            );
            assert_eq!(
                load_generation_ledger(repo.path(), foreign_owner)
                    .unwrap()
                    .unwrap()
                    .current_effective_status(),
                Some(ExecutionControlStatus::Active)
            );
            assert_eq!(fs::read(&session_path).unwrap(), session_before);
            assert_eq!(
                [
                    gwt_core::paths::gwt_workspace_projection_path_for_repo_path(repo.path()),
                    gwt_core::paths::gwt_workspace_journal_path_for_repo_path(repo.path()),
                    gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo.path()),
                    gwt_core::paths::gwt_repo_local_work_events_path(repo.path()),
                ]
                .into_iter()
                .map(|path| (path.clone(), fs::read(path).ok()))
                .collect::<Vec<_>>(),
                work_before
            );
        }

        #[test]
        fn status_always_reports_all_seven_operation_local_probes() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

            let active_repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(active_repo.path());
            gwt_core::process::run_git_logged(
                &[
                    "remote",
                    "set-url",
                    "origin",
                    "https://example.invalid/probes-active.git",
                ],
                Some(active_repo.path()),
            )
            .unwrap();
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(active_repo.path(), &active_record("foreign-session")).unwrap();
            ensure_generation_ledger(active_repo.path(), owner, LegacyActiveDisposition::Live)
                .unwrap();
            assert_all_operation_local_recovery_probes(&diagnose(
                active_repo.path(),
                Some("candidate-session"),
            ));

            let blocked_repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(blocked_repo.path());
            gwt_core::process::run_git_logged(
                &[
                    "remote",
                    "set-url",
                    "origin",
                    "https://example.invalid/probes-blocked.git",
                ],
                Some(blocked_repo.path()),
            )
            .unwrap();
            save(blocked_repo.path(), &active_record("blocked-session")).unwrap();
            ensure_generation_ledger(blocked_repo.path(), owner, LegacyActiveDisposition::Live)
                .unwrap();
            settle_blocked(blocked_repo.path(), "blocked-session");
            assert_all_operation_local_recovery_probes(&diagnose(
                blocked_repo.path(),
                Some("blocked-session"),
            ));

            let corrupt_repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(corrupt_repo.path());
            gwt_core::process::run_git_logged(
                &[
                    "remote",
                    "set-url",
                    "origin",
                    "https://example.invalid/probes-corrupt.git",
                ],
                Some(corrupt_repo.path()),
            )
            .unwrap();
            save(corrupt_repo.path(), &active_record("corrupt-session")).unwrap();
            let trusted_dir =
                crate::cli::trusted_store::trusted_dir_for_worktree(corrupt_repo.path())
                    .expect("trusted worktree directory");
            std::fs::write(trusted_dir.join("execution-control.json"), b"{broken")
                .expect("corrupt trusted ECR fixture");
            assert_all_operation_local_recovery_probes(&diagnose(
                corrupt_repo.path(),
                Some("corrupt-session"),
            ));
        }

        #[test]
        fn projection_reads_active_blocked_and_corrupt_durable_state_without_spawning_git() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _live_session_env = unset_live_session_env();
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            let blocked_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3249,
            };
            let corrupt_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3250,
            };

            let active_repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(active_repo.path());
            save(active_repo.path(), &active_record("projection-active")).unwrap();
            ensure_generation_ledger(active_repo.path(), owner, LegacyActiveDisposition::Live)
                .unwrap();
            let active_binding = current_execution_binding(active_repo.path(), owner)
                .unwrap()
                .expect("active generation binding");
            persist_generation_session_binding(
                active_repo.path(),
                owner,
                "projection-active",
                active_binding,
            );
            fs::write(active_repo.path().join("projection-artifact.json"), "{}")
                .expect("write generated output fixture");
            crate::cli::verification_record::save_plan(
                active_repo.path(),
                &crate::cli::verification_record::VerificationPlanRecord {
                    session_id: "projection-active".to_string(),
                    owner_number: Some(owner.number),
                    execution_binding: None,
                    commands: vec!["git --version".to_string()],
                    derived: true,
                    surfaces: vec!["rust".to_string()],
                    generated_outputs: vec!["projection-artifact.json".to_string()],
                    worktree_fingerprint: String::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            crate::cli::verification_record::run_verification(
                active_repo.path(),
                "projection-active",
                &["git --version".to_string()],
            )
            .unwrap();

            let blocked_repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(blocked_repo.path());
            let mut blocked_record = active_record("projection-blocked");
            blocked_record.owner_number = blocked_owner.number;
            save(blocked_repo.path(), &blocked_record).unwrap();
            ensure_generation_ledger(
                blocked_repo.path(),
                blocked_owner,
                LegacyActiveDisposition::Live,
            )
            .unwrap();
            settle_blocked(blocked_repo.path(), "projection-blocked");

            let corrupt_repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(corrupt_repo.path());
            let mut corrupt_record = active_record("projection-corrupt");
            corrupt_record.owner_number = corrupt_owner.number;
            save(corrupt_repo.path(), &corrupt_record).unwrap();
            let corrupt_ecr =
                crate::cli::trusted_store::trusted_dir_for_worktree(corrupt_repo.path())
                    .expect("corrupt fixture trusted directory")
                    .join("execution-control.json");
            fs::write(corrupt_ecr, b"{corrupt").expect("corrupt ECR fixture");

            let fake_git = write_failing_git_recorder(&home.path().join("fake-git"));
            let mut path = vec![fake_git.parent().unwrap().to_path_buf()];
            if let Some(existing) = std::env::var_os("PATH") {
                path.extend(std::env::split_paths(&existing));
            }
            let _path = ScopedEnvVar::set("PATH", std::env::join_paths(path).unwrap());
            let git_log = home.path().join("projection-git.log");
            let _git_log = ScopedEnvVar::set("GWT_FAKE_GIT_LOG", &git_log);

            let active = diagnose_for_projection(active_repo.path(), Some("projection-active"));
            assert_eq!(active.ecr_status, ExecutionDiagnosisState::Active);
            assert_eq!(active.binding_state, ExecutionBindingState::Bound);
            assert_eq!(active.owner_number, Some(owner.number));
            assert!(active.generation_id.is_some());
            assert_eq!(active.verification_state, "not_evaluated");
            assert_eq!(active.workspace_update_applicable, None);
            assert_eq!(
                active.generated_outputs,
                vec!["projection-artifact.json".to_string()]
            );
            assert_projection_has_no_protected_recovery(&active);

            let blocked = diagnose_for_projection(blocked_repo.path(), Some("projection-blocked"));
            assert_eq!(blocked.ecr_status, ExecutionDiagnosisState::Blocked);
            assert_eq!(blocked.binding_state, ExecutionBindingState::Terminal);
            assert_eq!(
                blocked.blocked_reason.as_deref(),
                Some("verification dependency unresolved")
            );
            assert_eq!(blocked.verification_state, "not_evaluated");
            assert_eq!(
                blocked.available_recoveries,
                vec!["verify.plan".to_string(), "verify.run".to_string()]
            );
            assert_projection_has_no_protected_recovery(&blocked);

            let corrupt = diagnose_for_projection(corrupt_repo.path(), Some("projection-corrupt"));
            assert_eq!(corrupt.ecr_status, ExecutionDiagnosisState::Corrupt);
            assert_eq!(corrupt.verification_state, "not_evaluated");
            assert_projection_has_no_protected_recovery(&corrupt);

            let invocations = fs::read_to_string(&git_log).unwrap_or_default();
            assert!(
                invocations.trim().is_empty(),
                "projection must not spawn Git; invocations:\n{invocations}"
            );
        }

        #[test]
        fn adopt_refuses_corrupt_owner_ledger_before_any_authority_mutation() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(repo.path(), &active_record("session-original")).unwrap();
            ensure_generation_ledger(repo.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let binding = current_execution_binding(repo.path(), owner)
                .unwrap()
                .expect("current generation binding");
            persist_generation_session_binding(repo.path(), owner, "session-original", binding);
            let recovery_session =
                persist_recovery_session_snapshot(repo.path(), owner, "session-adopting");
            seed_recovery_work_bytes(repo.path());
            fs::write(
                generation_ledger_path(repo.path(), owner).unwrap(),
                b"{corrupt",
            )
            .unwrap();
            let before = recovery_operation_authority_bytes(
                repo.path(),
                owner,
                &["session-original", "session-adopting"],
            );

            let probe = probe_execution_adopt(repo.path(), "session-adopting");
            assert_eq!(
                probe.state,
                crate::cli::governance::RecoveryProbeState::Unavailable
            );
            let reason = probe.reason.clone().expect("stable adopt refusal reason");
            assert!(reason.contains("owner generation authority"), "{reason}");
            let diagnosis = diagnose(repo.path(), Some("session-adopting"));
            assert!(!diagnosis
                .available_recoveries
                .contains(&"execution.adopt".to_string()));
            assert_eq!(
                recovery_probe(&diagnosis, "execution.adopt")
                    .reason
                    .as_deref(),
                Some(reason.as_str())
            );

            let mut out = String::new();
            assert_eq!(
                run_adopt(
                    repo.path(),
                    "session-adopting",
                    &recovery_session,
                    "recover crashed owner",
                    &mut out,
                )
                .unwrap(),
                2,
                "{out}"
            );
            assert!(out.contains(&reason), "{out}");
            assert_eq!(
                recovery_operation_authority_bytes(
                    repo.path(),
                    owner,
                    &["session-original", "session-adopting"],
                ),
                before,
                "adopt refusal must preserve Session, ECR, pointer, ledger, capability, and Work bytes"
            );
        }

        #[test]
        fn reopen_refuses_corrupt_owner_ledger_before_any_authority_mutation() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(repo.path(), &active_record("sess-reopen")).unwrap();
            ensure_generation_ledger(repo.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let binding = current_execution_binding(repo.path(), owner)
                .unwrap()
                .expect("current generation binding");
            persist_generation_session_binding(repo.path(), owner, "sess-reopen", binding);
            settle_blocked(repo.path(), "sess-reopen");
            seed_recovery_work_bytes(repo.path());
            save_covering_evidence(repo.path(), "sess-reopen", true);
            fs::write(
                generation_ledger_path(repo.path(), owner).unwrap(),
                b"{corrupt",
            )
            .unwrap();
            let before = recovery_operation_authority_bytes(repo.path(), owner, &["sess-reopen"]);

            let probe = probe_execution_reopen(repo.path(), "sess-reopen");
            assert_eq!(
                probe.state,
                crate::cli::governance::RecoveryProbeState::Unavailable
            );
            let reason = probe.reason.clone().expect("stable reopen refusal reason");
            assert!(reason.contains("owner generation authority"), "{reason}");
            let diagnosis = diagnose(repo.path(), Some("sess-reopen"));
            assert!(!diagnosis
                .available_recoveries
                .contains(&"execution.reopen".to_string()));
            assert_eq!(
                recovery_probe(&diagnosis, "execution.reopen")
                    .reason
                    .as_deref(),
                Some(reason.as_str())
            );

            let mut out = String::new();
            assert_eq!(
                run_reopen(
                    repo.path(),
                    "sess-reopen",
                    "fresh evidence is available",
                    &mut out,
                )
                .unwrap(),
                2,
                "{out}"
            );
            assert!(out.contains(&reason), "{out}");
            assert_eq!(
                recovery_operation_authority_bytes(
                    repo.path(),
                    owner,
                    &["sess-reopen"],
                ),
                before,
                "reopen refusal must preserve Session, ECR, pointer, ledger, capability, and Work bytes"
            );
        }

        #[test]
        fn available_adopt_probe_executes_successfully() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(repo.path(), &active_record("session-original")).unwrap();
            ensure_generation_ledger(repo.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let recovery_session =
                persist_recovery_session_snapshot(repo.path(), owner, "session-adopting");

            let probe = probe_execution_adopt(repo.path(), "session-adopting");
            assert_eq!(
                probe.state,
                crate::cli::governance::RecoveryProbeState::Available
            );
            let mut out = String::new();
            assert_eq!(
                run_adopt(
                    repo.path(),
                    "session-adopting",
                    &recovery_session,
                    "recover crashed owner",
                    &mut out,
                )
                .unwrap(),
                0,
                "{out}"
            );
            assert_eq!(
                load(repo.path()).unwrap().unwrap().primary_session_id,
                "session-adopting"
            );
        }

        #[test]
        fn available_reopen_probe_executes_successfully() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(repo.path(), &active_record("sess-reopen")).unwrap();
            ensure_generation_ledger(repo.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let binding = current_execution_binding(repo.path(), owner)
                .unwrap()
                .expect("current generation binding");
            persist_generation_session_binding(repo.path(), owner, "sess-reopen", binding);
            settle_blocked(repo.path(), "sess-reopen");
            save_covering_evidence(repo.path(), "sess-reopen", true);

            let probe = probe_execution_reopen(repo.path(), "sess-reopen");
            assert_eq!(
                probe.state,
                crate::cli::governance::RecoveryProbeState::Available
            );
            let mut out = String::new();
            assert_eq!(
                run_reopen(
                    repo.path(),
                    "sess-reopen",
                    "fresh evidence is available",
                    &mut out,
                )
                .unwrap(),
                0,
                "{out}"
            );
            assert_eq!(
                load(repo.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Active
            );
        }

        #[test]
        fn unavailable_recovery_probe_reasons_match_execution_refusals() {
            let missing = tempfile::tempdir().unwrap();
            let missing_probe = probe_execution_reopen(missing.path(), "sess-reopen");
            let mut missing_out = String::new();
            assert_eq!(
                run_reopen(missing.path(), "sess-reopen", "resolved", &mut missing_out,).unwrap(),
                2
            );
            assert!(
                missing_out.contains(missing_probe.reason.as_deref().unwrap()),
                "{missing_out}"
            );

            let completed = tempfile::tempdir().unwrap();
            save(completed.path(), &active_record("sess-reopen")).unwrap();
            settle(
                completed.path(),
                "sess-reopen",
                ExecutionSettlement::Completed,
            )
            .unwrap();
            let completed_probe = probe_execution_reopen(completed.path(), "sess-reopen");
            let mut completed_out = String::new();
            assert_eq!(
                run_reopen(
                    completed.path(),
                    "sess-reopen",
                    "resolved",
                    &mut completed_out,
                )
                .unwrap(),
                2
            );
            assert!(
                completed_out.contains(completed_probe.reason.as_deref().unwrap()),
                "{completed_out}"
            );

            let foreign = tempfile::tempdir().unwrap();
            settle_blocked(foreign.path(), "session-original");
            let foreign_probe = probe_execution_reopen(foreign.path(), "sess-reopen");
            let mut foreign_out = String::new();
            assert_eq!(
                run_reopen(foreign.path(), "sess-reopen", "resolved", &mut foreign_out,).unwrap(),
                2
            );
            assert!(
                foreign_out.contains(foreign_probe.reason.as_deref().unwrap()),
                "{foreign_out}"
            );

            let active = tempfile::tempdir().unwrap();
            save(active.path(), &active_record("session-original")).unwrap();
            let adopt_probe = probe_execution_adopt(active.path(), "session-adopting");
            assert_eq!(
                adopt_probe.state,
                crate::cli::governance::RecoveryProbeState::Available
            );
            let terminal = tempfile::tempdir().unwrap();
            save(terminal.path(), &active_record("session-original")).unwrap();
            settle(
                terminal.path(),
                "session-original",
                ExecutionSettlement::Completed,
            )
            .unwrap();
            let terminal_probe = probe_execution_adopt(terminal.path(), "session-adopting");
            let mut recovery_session =
                gwt_agent::Session::new(terminal.path(), "main", gwt_agent::AgentId::Codex);
            recovery_session.id = "session-adopting".to_string();
            let mut terminal_out = String::new();
            assert_eq!(
                run_adopt(
                    terminal.path(),
                    "session-adopting",
                    &recovery_session,
                    "recover owner",
                    &mut terminal_out,
                )
                .unwrap(),
                2
            );
            assert!(
                terminal_out.contains(terminal_probe.reason.as_deref().unwrap()),
                "{terminal_out}"
            );
        }

        #[test]
        fn status_distinguishes_host_bridge_unreachable_from_stale_binding() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(dir.path(), &active_record("sess-status")).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let binding = current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap();
            persist_generation_session_binding(dir.path(), owner, "sess-status", binding);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-status");
            let _runtime = ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
                dir.path().join("runtime.json"),
            );
            let _url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
            let _token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);

            let snapshot = diagnose(dir.path(), Some("sess-status"));

            assert_eq!(
                snapshot.binding_state,
                ExecutionBindingState::HostUnreachable
            );
            assert_eq!(snapshot.binding_cause, "host_bridge_capability_missing");
            assert_eq!(
                snapshot.available_recoveries,
                vec!["workspace.ensure".to_string()],
            );
            assert_eq!(
                snapshot
                    .recovery_probes
                    .iter()
                    .find(|probe| probe.operation == "execution.continue")
                    .map(|probe| probe.state),
                Some(crate::cli::governance::RecoveryProbeState::Satisfied),
                "an exact current binding needs host reachability recovery, not continuation",
            );
            assert_eq!(
                snapshot.workspace_update_applicability_reason.as_deref(),
                Some("workspace_ensure_required")
            );
            assert!(
                snapshot
                    .warnings
                    .iter()
                    .any(|warning| warning.starts_with("workspace_update_not_applicable:")),
                "{:?}",
                snapshot.warnings
            );
        }

        #[test]
        fn status_reports_blocked_reason_obligations_evidence_and_recovery() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            save(dir.path(), &active_record("sess-status")).unwrap();
            ensure_generation_ledger(
                dir.path(),
                ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 3248,
                },
                LegacyActiveDisposition::Live,
            )
            .unwrap();
            crate::cli::action_obligation::mark_from_prompt(
                dir.path(),
                "sess-status",
                "Issue #3248 にコメントを追加して",
            )
            .unwrap();
            settle_blocked(dir.path(), "sess-status");

            let snapshot = diagnose(dir.path(), Some("sess-status"));

            assert_eq!(snapshot.ecr_status, ExecutionDiagnosisState::Blocked);
            assert_eq!(snapshot.binding_state, ExecutionBindingState::Terminal);
            assert_eq!(
                snapshot.blocked_reason.as_deref(),
                Some("verification dependency unresolved")
            );
            assert_eq!(
                snapshot.missing_verification.as_deref(),
                Some("full pre-PR matrix")
            );
            assert_eq!(snapshot.verification_state, "missing_record");
            assert_eq!(snapshot.open_obligations, vec!["issue_update"]);
            assert_eq!(
                snapshot.available_recoveries,
                vec!["verify.plan", "verify.run"]
            );
            assert_eq!(
                snapshot
                    .recovery_probes
                    .iter()
                    .find(|probe| probe.operation == "execution.reopen")
                    .map(|probe| probe.state),
                Some(crate::cli::governance::RecoveryProbeState::Unavailable),
                "reopen is not executable before fresh derived evidence exists",
            );
        }

        #[test]
        fn adopt_and_reopen_execution_share_operation_local_probe_refusal() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let repo = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
            save(repo.path(), &active_record("blocked-session")).unwrap();
            settle_blocked(repo.path(), "blocked-session");
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            persist_recovery_session_snapshot(repo.path(), owner, "blocked-session");
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "blocked-session");
            let diagnosis = diagnose(repo.path(), Some("blocked-session"));
            let adopt_probe = diagnosis
                .recovery_probes
                .iter()
                .find(|probe| probe.operation == "execution.adopt")
                .expect("adopt probe");
            let reopen_probe = diagnosis
                .recovery_probes
                .iter()
                .find(|probe| probe.operation == "execution.reopen")
                .expect("reopen probe");
            assert_eq!(
                adopt_probe.state,
                crate::cli::governance::RecoveryProbeState::Unavailable
            );
            assert_eq!(
                reopen_probe.state,
                crate::cli::governance::RecoveryProbeState::Unavailable
            );
            let trusted_path = crate::cli::trusted_store::trusted_dir_for_worktree(repo.path())
                .unwrap()
                .join("execution-control.json");
            let before = std::fs::read(&trusted_path).unwrap();

            let (adopt_code, adopt_out) = run_cmd(
                repo.path(),
                ExecutionCommand::Adopt {
                    reason: "cannot adopt terminal record".to_string(),
                },
            )
            .unwrap();
            assert_eq!(adopt_code, 2, "{adopt_out}");
            assert!(
                adopt_out.contains(adopt_probe.reason.as_deref().unwrap()),
                "{adopt_out}"
            );

            let (reopen_code, reopen_out) = run_cmd(
                repo.path(),
                ExecutionCommand::Reopen {
                    reason: "evidence is not ready".to_string(),
                },
            )
            .unwrap();
            assert_eq!(reopen_code, 2, "{reopen_out}");
            assert!(reopen_out.contains("verify.plan"), "{reopen_out}");
            assert_eq!(
                std::fs::read(&trusted_path).unwrap(),
                before,
                "unavailable adopt/reopen must preserve authority bytes"
            );
        }

        #[test]
        fn status_distinguishes_stale_and_current_work_event_receipt_generations() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(dir.path(), &active_record("session-original")).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();

            let predecessor_receipt =
                crate::cli::verification_record::save_work_event_settlement_record(
                    dir.path(),
                    "session-original",
                    false,
                )
                .unwrap();
            let predecessor_generation = predecessor_receipt
                .execution_binding
                .as_ref()
                .expect("receipt is generation-bound")
                .generation_id
                .clone();
            let before = diagnose(dir.path(), None);
            assert_eq!(
                before.work_event_receipt_generation_id.as_deref(),
                Some(predecessor_generation.as_str())
            );
            assert_eq!(
                before.work_event_receipt_matches_current_generation,
                Some(true)
            );

            let mut unbound_receipt = predecessor_receipt.clone();
            unbound_receipt.execution_binding = None;
            crate::cli::verification_record::persist_work_event_settlement_record(
                dir.path(),
                &unbound_receipt,
            )
            .unwrap();
            assert_eq!(
                diagnose(dir.path(), None).work_event_receipt_matches_current_generation,
                Some(false),
                "diagnosis must reject an unbound receipt in a generation-aware worktree",
            );
            assert!(
                crate::cli::verification_record::work_event_settlement_refusal(dir.path())
                    .is_some(),
                "the status projection and mutation gate must share the same authority decision",
            );

            let mut foreign_receipt = predecessor_receipt.clone();
            foreign_receipt.session_id = "session-foreign".to_string();
            crate::cli::verification_record::persist_work_event_settlement_record(
                dir.path(),
                &foreign_receipt,
            )
            .unwrap();
            assert_eq!(
                diagnose(dir.path(), None).work_event_receipt_matches_current_generation,
                Some(false),
                "diagnosis must reject foreign receipt provenance like the mutation gate",
            );

            let mut tampered_receipt = predecessor_receipt.clone();
            tampered_receipt
                .execution_binding
                .as_mut()
                .expect("receipt is generation-bound")
                .ledger_head_hash = "arbitrary-ledger-head".to_string();
            crate::cli::verification_record::persist_work_event_settlement_record(
                dir.path(),
                &tampered_receipt,
            )
            .unwrap();
            assert_eq!(
                diagnose(dir.path(), None).work_event_receipt_matches_current_generation,
                Some(false),
                "diagnosis must reject an arbitrary same-generation head",
            );
            crate::cli::verification_record::persist_work_event_settlement_record(
                dir.path(),
                &predecessor_receipt,
            )
            .unwrap();

            let request = successor_request(
                "operation-status-receipt-successor",
                "host",
                "execution-continue",
            );
            prepare_active_continuation_successor(dir.path(), owner, &request).unwrap();
            activate_successor(dir.path(), owner, &request).unwrap();

            let stale = diagnose(dir.path(), None);
            assert_ne!(
                stale.generation_id.as_deref(),
                Some(predecessor_generation.as_str())
            );
            assert_eq!(
                stale.work_event_receipt_generation_id.as_deref(),
                Some(predecessor_generation.as_str())
            );
            assert_eq!(
                stale.work_event_receipt_matches_current_generation,
                Some(false)
            );

            let current_receipt =
                crate::cli::verification_record::save_work_event_settlement_record(
                    dir.path(),
                    &request.initial_session_id,
                    true,
                )
                .unwrap();
            let current_generation = current_receipt
                .execution_binding
                .as_ref()
                .expect("replacement receipt is generation-bound")
                .generation_id
                .clone();
            let current = diagnose(dir.path(), None);
            assert_eq!(
                current.work_event_receipt_generation_id.as_deref(),
                Some(current_generation.as_str())
            );
            assert_eq!(
                current.work_event_receipt_matches_current_generation,
                Some(true)
            );
        }

        #[test]
        fn status_reports_work_event_settlement_fact_and_severity_matrix() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(dir.path(), &active_record("sess-status")).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let binding = current_execution_binding(dir.path(), owner)
                .unwrap()
                .expect("current execution binding");

            let cases = [
                (
                    crate::cli::verification_record::WorkEventSettlementStatus::Settled {
                        event_commit: "event-commit".to_string(),
                        upstream_ref: "origin/develop".to_string(),
                    },
                    false,
                    "clear",
                ),
                (
                    crate::cli::verification_record::WorkEventSettlementStatus::Blocked(
                        crate::cli::verification_record::WorkEventSettlementBlocker::
                            PathDirtyInUnreachableEnvironment {
                                states: vec![
                                    crate::cli::verification_record::WorkEventPathState::Unstaged,
                                ],
                                environment: crate::cli::verification_record::
                                    WorkEventSettlementEnvironment::MissingUpstream,
                            },
                    ),
                    true,
                    "warning",
                ),
                (
                    crate::cli::verification_record::WorkEventSettlementStatus::Blocked(
                        crate::cli::verification_record::WorkEventSettlementBlocker::PathDirty {
                            states: vec![
                                crate::cli::verification_record::WorkEventPathState::Staged,
                            ],
                        },
                    ),
                    true,
                    "blocked",
                ),
            ];

            for (status, obligation_open, expected_severity) in cases {
                crate::cli::verification_record::persist_work_event_settlement_record(
                    dir.path(),
                    &crate::cli::verification_record::WorkEventSettlementRecord {
                        schema_version:
                            crate::cli::verification_record::WORK_EVENT_SETTLEMENT_SCHEMA_VERSION,
                        session_id: "sess-status".to_string(),
                        execution_binding: Some(binding.clone()),
                        pending_delivery: None,
                        obligation_open,
                        status: status.clone(),
                        updated_at: Utc::now(),
                    },
                )
                .unwrap();

                let diagnosis = diagnose(dir.path(), Some("sess-status"));
                assert_eq!(diagnosis.settlement.as_ref(), Some(&status));
                assert_eq!(diagnosis.settlement_severity, expected_severity);
                assert_eq!(diagnosis.settlement_obligation_open, obligation_open);
                assert_eq!(
                    diagnosis.work_event_receipt_matches_current_generation,
                    Some(true)
                );
            }
        }

        #[test]
        fn status_reports_integrity_checked_binding_repair_success_and_failure() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(dir.path(), &active_record("sess-status")).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let identity = current_execution_binding(dir.path(), owner)
                .unwrap()
                .expect("current execution binding");
            let binding = gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: "sess-status".to_string(),
                repo_hash: crate::index_worker::detect_repo_hash(dir.path())
                    .unwrap()
                    .to_string(),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity: identity.clone(),
                capability_generation: 7,
            };

            persist_binding_repair_outcome(
                dir.path(),
                new_binding_repair_outcome_record(
                    &binding,
                    owner,
                    BindingRepairOutcome::Succeeded {
                        host_instance_id: "host-current".to_string(),
                        receipt_generation_id: identity.generation_id.clone(),
                    },
                ),
            )
            .unwrap();
            let success = diagnose(dir.path(), Some("sess-status"))
                .binding_repair
                .expect("successful binding repair diagnosis");
            assert_eq!(success.status, "succeeded");
            assert_eq!(success.failure_cause, None);
            assert_eq!(success.host_instance_id.as_deref(), Some("host-current"));
            assert!(success.matches_current_generation);
            assert!(success.validated);

            persist_binding_repair_outcome(
                dir.path(),
                new_binding_repair_outcome_record(
                    &binding,
                    owner,
                    BindingRepairOutcome::Failed {
                        cause: BindingRepairFailureCause::ProbeReceiptMismatch,
                        observed_generation_id: Some("generation-stale".to_string()),
                    },
                ),
            )
            .unwrap();
            let failure = diagnose(dir.path(), Some("sess-status"))
                .binding_repair
                .expect("failed binding repair diagnosis");
            assert_eq!(failure.status, "failed");
            assert_eq!(
                failure.failure_cause,
                Some(BindingRepairFailureCause::ProbeReceiptMismatch)
            );
            assert_eq!(
                failure.observed_generation_id.as_deref(),
                Some("generation-stale")
            );
            assert!(failure.matches_current_generation);
            assert!(failure.validated);
        }

        #[test]
        fn status_reports_deferred_and_persist_failed_obligation_revival() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let deferred = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(deferred.path());
            save(deferred.path(), &active_record("sess-status")).unwrap();
            ensure_generation_ledger(
                deferred.path(),
                ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 3248,
                },
                LegacyActiveDisposition::Live,
            )
            .unwrap();
            assert_eq!(
                crate::cli::action_obligation::revive_deferred(
                    deferred.path(),
                    "sess-status",
                    &[crate::cli::action_obligation::ObligationKind::IssueUpdate],
                ),
                crate::cli::action_obligation::ObligationRevivalOutcome::Deferred {
                    reason: "obligation_state_missing".to_string(),
                }
            );
            assert!(matches!(
                diagnose(deferred.path(), Some("sess-status")).obligation_revival,
                Some(ExecutionObligationRevivalDiagnosis::Deferred { ref reason })
                    if reason == "obligation_state_missing"
            ));

            let trusted_dir =
                crate::cli::trusted_store::trusted_dir_for_worktree(deferred.path()).unwrap();
            fs::write(trusted_dir.join("action-obligations.json"), b"{corrupt").unwrap();
            assert!(matches!(
                crate::cli::action_obligation::revive_deferred(
                    deferred.path(),
                    "sess-status",
                    &[crate::cli::action_obligation::ObligationKind::IssueUpdate],
                ),
                crate::cli::action_obligation::ObligationRevivalOutcome::PersistFailed { .. }
            ));
            assert!(matches!(
                diagnose(deferred.path(), Some("sess-status")).obligation_revival,
                Some(ExecutionObligationRevivalDiagnosis::PersistFailed { .. })
            ));
        }

        #[test]
        fn repair_quarantines_corrupt_ecr_or_ledger_and_materializes_fresh_authority() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

            let cases = tempfile::tempdir().unwrap();
            for (index, corrupt_ledger) in [false, true].into_iter().enumerate() {
                let worktree = cases.path().join(format!("case-{index}"));
                fs::create_dir_all(&worktree).unwrap();
                crate::cli::trusted_store::init_git_repo_with_origin(&worktree);
                let owner = ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 900_100 + index as u64,
                };
                let mut active = active_record("repair-session");
                active.owner_number = owner.number;
                save(&worktree, &active).unwrap();
                ensure_generation_ledger(&worktree, owner, LegacyActiveDisposition::Live).unwrap();
                let context = GenerationTransactionContext::resolve(&worktree, owner).unwrap();
                let corrupt_path = if corrupt_ledger {
                    context.owner_dir.join(GENERATION_LEDGER_FILE)
                } else {
                    context.worktree_trusted_dir.join("execution-control.json")
                };
                fs::write(&corrupt_path, b"{corrupt").unwrap();

                let outcome = repair_corrupt_execution(
                    &worktree,
                    "repair-session",
                    "recover corrupt authority",
                )
                .unwrap();

                assert_eq!(outcome.status, "repaired");
                assert_eq!(outcome.owner, owner);
                assert!(
                    outcome.quarantined.iter().any(|source| {
                        source.source_path == corrupt_path.to_string_lossy()
                            && fs::read(&source.quarantine_path).unwrap() == b"{corrupt"
                    }),
                    "corrupt source must survive at its quarantine path"
                );
                let record = load(&worktree).unwrap().unwrap();
                assert_eq!(record.status, ExecutionControlStatus::Active);
                assert_eq!(record.primary_session_id, "repair-session");
                let ledger = load_generation_ledger(&worktree, owner).unwrap().unwrap();
                assert_eq!(ledger.current_generation_id, outcome.generation_id);
                assert!(generation_ledger_integrity_ok(&ledger));
                let audits = load_repair_audits(&repair_audit_dir(&context).unwrap()).unwrap();
                assert_eq!(
                    audits.last().map(|audit| audit.repair_id.as_str()),
                    Some(outcome.repair_id.as_str())
                );
                let diagnosis = diagnose(&worktree, None);
                let repair = diagnosis.repair.expect("activated repair diagnosis");
                assert_eq!(repair.outcome, "activated");
                assert_eq!(repair.new_generation_id, outcome.generation_id);
                let expected_source = if corrupt_ledger {
                    ExecutionRepairSourceKind::GenerationLedger
                } else {
                    ExecutionRepairSourceKind::ExecutionControl
                };
                assert!(
                    repair.source_kinds.contains(&expected_source),
                    "{:?}",
                    repair.source_kinds
                );
                let retry =
                    repair_corrupt_execution(&worktree, "repair-session", "duplicate repair")
                        .unwrap_err();
                assert_eq!(retry.kind(), ErrorKind::AlreadyExists);
                assert!(retry.to_string().contains("execution_repair_not_corrupt"));
            }
        }

        #[test]
        fn repair_recovers_mirror_pointer_partial_authority_advertised_by_status() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_203,
            };
            let authority_paths = mirror_pointer_partial_authority(dir.path(), owner);
            let session = persist_recovery_session_snapshot(dir.path(), owner, "repair-session");

            let before = diagnose(dir.path(), Some(&session.id));
            assert_eq!(before.ecr_status, ExecutionDiagnosisState::Corrupt);
            assert_eq!(
                before.available_recoveries,
                vec!["execution.repair".to_string()]
            );
            let expected_quarantined = [0_usize, 1, 3]
                .into_iter()
                .map(|index| normalized_test_path(&authority_paths[index]))
                .collect::<std::collections::HashSet<_>>();

            let outcome = repair_corrupt_execution_with_session_snapshot(
                dir.path(),
                "repair-session",
                &session,
                "recover mirror pointer partial authority",
            )
            .expect("an advertised mirror-partial repair must be executable");

            assert_eq!(outcome.owner, owner);
            let quarantined_paths = outcome
                .quarantined
                .iter()
                .map(|source| normalized_test_path(Path::new(&source.source_path)))
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(
                quarantined_paths, expected_quarantined,
                "repair must preserve every authority source it overwrites"
            );
            let record = load(dir.path()).unwrap().unwrap();
            assert!(integrity_ok(&record));
            let ledger = load_generation_ledger(dir.path(), owner).unwrap().unwrap();
            assert_eq!(ledger.current_generation_id, outcome.generation_id);
        }

        #[test]
        fn repair_recovers_malformed_mirror_only_ecr() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_204,
            };
            let mut malformed = active_record("repair-session");
            malformed.owner_kind = owner.kind;
            malformed.owner_number = owner.number;
            save(dir.path(), &malformed).unwrap();
            let authority_paths = repair_authority_paths(dir.path(), owner);
            malformed = load(dir.path()).unwrap().unwrap();
            malformed.content_hash = "invalid-mirror-integrity".to_string();
            fs::write(
                &authority_paths[1],
                serde_json::to_vec_pretty(&malformed).unwrap(),
            )
            .unwrap();
            fs::remove_file(&authority_paths[0]).unwrap();
            let session = persist_recovery_session_snapshot(dir.path(), owner, "repair-session");

            let diagnosis = diagnose(dir.path(), Some(&session.id));
            assert_eq!(diagnosis.ecr_status, ExecutionDiagnosisState::Corrupt);
            assert!(diagnosis
                .available_recoveries
                .contains(&"execution.repair".to_string()));
            let malformed_bytes = fs::read(&authority_paths[1]).unwrap();
            let malformed_path = normalized_test_path(&authority_paths[1]);

            let outcome = repair_corrupt_execution_with_session_snapshot(
                dir.path(),
                "repair-session",
                &session,
                "recover malformed mirror-only execution control",
            )
            .expect("mirror-only corruption with an owner hint must be repairable");

            let mirror_source = outcome
                .quarantined
                .iter()
                .find(|source| {
                    normalized_test_path(Path::new(&source.source_path)) == malformed_path
                })
                .expect("malformed mirror must be quarantined");
            assert_eq!(
                fs::read(&mirror_source.quarantine_path).unwrap(),
                malformed_bytes
            );
            assert!(integrity_ok(&load(dir.path()).unwrap().unwrap()));
        }

        #[test]
        fn repair_prefers_trusted_owner_over_conflicting_mirror_owner() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let trusted_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_205,
            };
            let mirror_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_206,
            };
            let mut active = active_record("repair-session");
            active.owner_kind = trusted_owner.kind;
            active.owner_number = trusted_owner.number;
            save(dir.path(), &active).unwrap();
            ensure_generation_ledger(dir.path(), trusted_owner, LegacyActiveDisposition::Live)
                .unwrap();
            let paths = repair_authority_paths(dir.path(), trusted_owner);
            let mut historical = load_generation_ledger(dir.path(), trusted_owner)
                .unwrap()
                .unwrap();
            historical.owner = mirror_owner;
            for generation in &mut historical.generations {
                generation.identity.owner = mirror_owner;
                let mut projection = serde_json::from_str::<ExecutionControlRecord>(
                    &generation.execution_control_json,
                )
                .unwrap();
                projection.owner_kind = mirror_owner.kind;
                projection.owner_number = mirror_owner.number;
                generation.execution_control_json =
                    String::from_utf8(serialize_execution_control(&projection).unwrap()).unwrap();
                generation.content_hash = compute_generation_hash(generation);
            }
            stamp_generation_ledger(&mut historical);
            let historical_context =
                GenerationTransactionContext::resolve(dir.path(), mirror_owner).unwrap();
            crate::cli::trusted_store::write_to_resolved_dir(
                &historical_context.owner_dir,
                GENERATION_LEDGER_FILE,
                &serde_json::to_vec_pretty(&historical).unwrap(),
            )
            .unwrap();
            fs::remove_file(&paths[2]).unwrap();
            fs::remove_file(&paths[4]).unwrap();
            let mut pointer = ExecutionGenerationPointer {
                schema_version: GENERATION_LEDGER_SCHEMA_VERSION,
                owner: mirror_owner,
                current_generation_id: "foreign-generation".to_string(),
                current_generation_content_hash: "foreign-generation-hash".to_string(),
                projection_content_hash: "foreign-projection-hash".to_string(),
                content_hash: String::new(),
            };
            pointer.content_hash = compute_generation_pointer_hash(&pointer);
            fs::write(&paths[3], serde_json::to_vec_pretty(&pointer).unwrap()).unwrap();
            let outcome = repair_corrupt_execution(
                dir.path(),
                "repair-session",
                "trusted owner must override stale mirror metadata",
            )
            .expect("an informational mirror and historical ledger must not block trusted repair");

            assert_eq!(outcome.owner, trusted_owner);
            assert!(integrity_ok(&load(dir.path()).unwrap().unwrap()));
            assert_eq!(
                load_generation_ledger(dir.path(), trusted_owner)
                    .unwrap()
                    .unwrap()
                    .current_generation_id,
                outcome.generation_id
            );
        }

        #[test]
        fn repair_refuses_conflicting_trusted_owners_without_mutation() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let ecr_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_211,
            };
            let pointer_owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_212,
            };
            let mut active = active_record("repair-session");
            active.owner_kind = ecr_owner.kind;
            active.owner_number = ecr_owner.number;
            save(dir.path(), &active).unwrap();
            let paths = repair_authority_paths(dir.path(), ecr_owner);
            let mut pointer = ExecutionGenerationPointer {
                schema_version: GENERATION_LEDGER_SCHEMA_VERSION,
                owner: pointer_owner,
                current_generation_id: "foreign-generation".to_string(),
                current_generation_content_hash: "foreign-generation-hash".to_string(),
                projection_content_hash: "foreign-projection-hash".to_string(),
                content_hash: String::new(),
            };
            pointer.content_hash = compute_generation_pointer_hash(&pointer);
            fs::write(&paths[2], serde_json::to_vec_pretty(&pointer).unwrap()).unwrap();
            let before = authority_bytes(&paths);

            let error = repair_corrupt_execution(
                dir.path(),
                "repair-session",
                "must not choose between conflicting trusted owners",
            )
            .expect_err("conflicting trusted owners must refuse repair");

            assert!(
                error
                    .to_string()
                    .contains("execution_repair_authority_mismatch"),
                "{error}"
            );
            assert_eq!(authority_bytes(&paths), before);
        }

        #[test]
        fn repair_discovers_owner_from_sole_surviving_generation_ledger() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_209,
            };
            let mut active = active_record("repair-session");
            active.owner_kind = owner.kind;
            active.owner_number = owner.number;
            save(dir.path(), &active).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let paths = repair_authority_paths(dir.path(), owner);
            for index in [0_usize, 1, 2, 3] {
                fs::remove_file(&paths[index]).unwrap();
            }
            let session = persist_recovery_session_snapshot(dir.path(), owner, "repair-session");

            let diagnosis = diagnose(dir.path(), Some(&session.id));
            assert_eq!(diagnosis.ecr_status, ExecutionDiagnosisState::Corrupt);
            assert!(diagnosis
                .available_recoveries
                .contains(&"execution.repair".to_string()));

            let outcome = repair_corrupt_execution_with_session_snapshot(
                dir.path(),
                "repair-session",
                &session,
                "recover sole surviving owner ledger",
            )
            .expect("a repair advertised from a valid owner ledger must be executable");

            assert_eq!(outcome.owner, owner);
            assert!(integrity_ok(&load(dir.path()).unwrap().unwrap()));
        }

        #[test]
        fn repair_ignores_corrupt_pointer_mirror_when_trusted_authority_is_strict() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_208,
            };
            let mut active = active_record("repair-session");
            active.owner_kind = owner.kind;
            active.owner_number = owner.number;
            save(dir.path(), &active).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let paths = repair_authority_paths(dir.path(), owner);
            fs::write(&paths[3], b"{corrupt-mirror").unwrap();
            let before = authority_bytes(&paths);

            let diagnosis = diagnose(dir.path(), None);
            assert_ne!(diagnosis.ecr_status, ExecutionDiagnosisState::Corrupt);
            assert!(!diagnosis
                .available_recoveries
                .contains(&"execution.repair".to_string()));
            let error = repair_corrupt_execution(
                dir.path(),
                "repair-session",
                "healthy trusted authority must win",
            )
            .expect_err("an informational mirror must not trigger repair");

            assert_eq!(error.kind(), ErrorKind::AlreadyExists, "{error}");
            assert!(error.to_string().contains("execution_repair_not_corrupt"));
            assert_eq!(authority_bytes(&paths), before);
        }

        #[test]
        fn mirror_partial_repair_failure_restores_all_authority_bytes_for_retry() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_207,
            };
            let paths = mirror_pointer_partial_authority(dir.path(), owner);
            let before = authority_bytes(&paths);
            let context = GenerationTransactionContext::resolve(dir.path(), owner).unwrap();

            set_generation_write_failure(GenerationWriteFailurePoint::AfterProjection);
            let error = repair_corrupt_execution(
                dir.path(),
                "repair-session",
                "inject mirror rollback failure boundary",
            )
            .expect_err("injected activation failure must surface");

            assert!(
                error
                    .to_string()
                    .contains("injected generation write failure"),
                "{error}"
            );
            assert_eq!(
                authority_bytes(&paths),
                before,
                "rollback must restore presence and bytes for all five authority paths"
            );
            assert!(load_repair_audits(&repair_audit_dir(&context).unwrap())
                .unwrap()
                .is_empty());

            let retry = repair_corrupt_execution(
                dir.path(),
                "repair-session",
                "retry mirror partial repair",
            )
            .expect("byte-exact rollback must leave one deterministic retry");
            assert_eq!(retry.status, "repaired");
        }

        #[test]
        fn quarantine_failure_restores_already_moved_authority_for_retry() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_210,
            };
            let paths = mirror_pointer_partial_authority(dir.path(), owner);
            let before = authority_bytes(&paths);

            set_repair_quarantine_failure_after(1);
            let error = repair_corrupt_execution(
                dir.path(),
                "repair-session",
                "inject partial quarantine failure",
            )
            .expect_err("quarantine failure must abort repair");

            assert!(
                error
                    .to_string()
                    .contains("injected repair quarantine failure"),
                "{error}"
            );
            assert_eq!(
                authority_bytes(&paths),
                before,
                "a failed quarantine sequence must not leave partial authority"
            );
            let retry = repair_corrupt_execution(
                dir.path(),
                "repair-session",
                "retry after partial quarantine rollback",
            )
            .expect("rollback must preserve a deterministic retry");
            assert_eq!(retry.status, "repaired");
        }

        #[test]
        fn repair_session_binding_refuses_concurrent_session_incarnation_change() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let canonical_worktree = dunce::canonicalize(dir.path()).unwrap();
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_213,
            };
            let repo_hash = gwt_core::repo_hash::detect_repo_hash(dir.path())
                .unwrap()
                .to_string();
            let mut session =
                gwt_agent::Session::new(dir.path(), "work/issue-2359", gwt_agent::AgentId::Codex);
            session.id = "repair-session".to_string();
            session.repo_hash = Some(repo_hash.clone());
            session.linked_issue_number = Some(owner.number);
            session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
            let expected = RepairSessionSnapshot::from(&session);
            let concurrent_binding = gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session.id.clone(),
                repo_hash: repo_hash.clone(),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity: gwt_agent::ExecutionBindingIdentity {
                    generation_id: "concurrent-generation".to_string(),
                    binding_id: "concurrent-binding".to_string(),
                    ledger_head_hash: "concurrent-head".to_string(),
                },
                capability_generation: 1,
            };
            gwt_agent::update_session(
                &gwt_core::paths::gwt_sessions_dir(),
                &session.id,
                |current| {
                    current
                        .set_execution_binding(Some(concurrent_binding.clone()))
                        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
                },
            )
            .unwrap();

            let error = repair_session_binding(
                &canonical_worktree,
                &session.id,
                owner,
                gwt_agent::ExecutionBindingIdentity {
                    generation_id: "repair-generation".to_string(),
                    binding_id: "repair-binding".to_string(),
                    ledger_head_hash: "repair-head".to_string(),
                },
                &expected,
            )
            .expect_err("a concurrent Session incarnation must win over stale repair publication");

            assert_eq!(error.kind(), ErrorKind::WouldBlock, "{error}");
            assert_eq!(
                gwt_agent::Session::load(
                    &gwt_core::paths::gwt_sessions_dir().join("repair-session.toml")
                )
                .unwrap()
                .execution_binding,
                Some(concurrent_binding)
            );
        }

        #[cfg(unix)]
        #[test]
        fn repair_audit_source_preserves_exact_non_utf8_path_bytes() {
            use std::os::unix::ffi::{OsStrExt, OsStringExt};

            let source_path = PathBuf::from(std::ffi::OsString::from_vec(vec![
                b'/', b't', b'm', b'p', b'/', 0xff, b's',
            ]));
            let quarantine_path = PathBuf::from(std::ffi::OsString::from_vec(vec![
                b'/', b't', b'm', b'p', b'/', 0xfe, b'q',
            ]));
            let source = QuarantinedExecutionAuthority {
                source_path: source_path.clone(),
                quarantine_path: quarantine_path.clone(),
                source_hash: "hash".to_string(),
            };

            let audit = source.audit_source();

            assert_eq!(
                hex::decode(audit.source_path_os_bytes_hex).unwrap(),
                source_path.as_os_str().as_bytes()
            );
            assert_eq!(
                hex::decode(audit.quarantine_path_os_bytes_hex).unwrap(),
                quarantine_path.as_os_str().as_bytes()
            );
        }

        #[test]
        fn concurrent_repairs_serialize_to_one_fresh_generation() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_200,
            };
            let mut active = active_record("repair-session");
            active.owner_number = owner.number;
            save(dir.path(), &active).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let context = GenerationTransactionContext::resolve(dir.path(), owner).unwrap();
            fs::write(
                context.worktree_trusted_dir.join("execution-control.json"),
                b"{corrupt",
            )
            .unwrap();

            let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
            let handles = (0..2)
                .map(|index| {
                    let path = dir.path().to_path_buf();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        repair_corrupt_execution(
                            &path,
                            "repair-session",
                            &format!("concurrent repair {index}"),
                        )
                    })
                })
                .collect::<Vec<_>>();
            barrier.wait();
            let results = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>();

            assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
            assert_eq!(
                results
                    .iter()
                    .filter_map(|result| result.as_ref().err())
                    .filter(|error| error.kind() == ErrorKind::AlreadyExists)
                    .count(),
                1,
                "{results:?}"
            );
            assert!(
                load_generation_ledger(dir.path(), owner).unwrap().is_some(),
                "the winning repair must leave strict generation authority"
            );
        }

        #[test]
        fn repair_audit_failure_never_activates_fresh_authority() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_201,
            };
            let mut active = active_record("repair-session");
            active.owner_number = owner.number;
            save(dir.path(), &active).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let context = GenerationTransactionContext::resolve(dir.path(), owner).unwrap();
            fs::write(
                context.worktree_trusted_dir.join("execution-control.json"),
                b"{corrupt",
            )
            .unwrap();
            let audit_dir = repair_audit_dir(&context).unwrap();
            set_repair_audit_write_failure();

            let error =
                repair_corrupt_execution(dir.path(), "repair-session", "audit store unavailable")
                    .unwrap_err();

            assert_eq!(error.kind(), ErrorKind::Other, "{error}");
            assert!(
                context
                    .worktree_trusted_dir
                    .join("execution-control.json")
                    .exists()
                    && context
                        .worktree_trusted_dir
                        .join(GENERATION_POINTER_FILE)
                        .exists()
                    && context.owner_dir.join(GENERATION_LEDGER_FILE).exists()
                    && !audit_dir.join(EXECUTION_REPAIR_AUDIT_FILE).exists(),
                "audit failure must restore the prior authority rather than activate fresh state"
            );
            assert_eq!(
                fs::read(context.worktree_trusted_dir.join("execution-control.json")).unwrap(),
                b"{corrupt"
            );
            let quarantined_corrupt_source = fs::read_dir(&context.worktree_trusted_dir)
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("execution-control.json.corrupt-"))
                })
                .any(|path| {
                    fs::read(path).is_ok_and(|contents| contents.as_slice() == b"{corrupt")
                });
            assert!(
                quarantined_corrupt_source,
                "the original corrupt bytes must remain recoverable after audit failure"
            );
            let retry = repair_corrupt_execution(
                dir.path(),
                "repair-session",
                "retry after audit store recovery",
            )
            .unwrap();
            assert_eq!(retry.status, "repaired");
        }

        #[test]
        fn repair_activation_failure_restores_corrupt_authority_for_retry() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 900_202,
            };
            let mut active = active_record("repair-session");
            active.owner_number = owner.number;
            save(dir.path(), &active).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let context = GenerationTransactionContext::resolve(dir.path(), owner).unwrap();
            let ecr_path = context.worktree_trusted_dir.join("execution-control.json");
            fs::write(&ecr_path, b"{corrupt").unwrap();

            set_generation_write_failure(GenerationWriteFailurePoint::AfterLedger);
            let error = repair_corrupt_execution(
                dir.path(),
                "repair-session",
                "injected activation failure",
            )
            .unwrap_err();

            assert!(
                error
                    .to_string()
                    .contains("injected generation write failure"),
                "{error}"
            );
            assert_eq!(
                fs::read(&ecr_path).unwrap(),
                b"{corrupt",
                "failed activation must restore the corrupt source for a deterministic retry"
            );
            assert!(
                context
                    .worktree_trusted_dir
                    .join(GENERATION_POINTER_FILE)
                    .exists()
                    && context.owner_dir.join(GENERATION_LEDGER_FILE).exists(),
                "failed activation must restore the complete prior authority set"
            );
            assert!(
                load_repair_audits(&repair_audit_dir(&context).unwrap())
                    .unwrap()
                    .is_empty(),
                "failed activation must not leave a success-shaped repair audit"
            );

            let retry = repair_corrupt_execution(
                dir.path(),
                "repair-session",
                "retry after activation recovery",
            )
            .unwrap();
            assert_eq!(retry.status, "repaired");
            assert_eq!(
                load_generation_ledger(dir.path(), owner)
                    .unwrap()
                    .unwrap()
                    .current_generation_id,
                retry.generation_id
            );
            let audits = load_repair_audits(&repair_audit_dir(&context).unwrap()).unwrap();
            assert_eq!(audits.len(), 1);
            assert_eq!(audits[0].new_generation_id, retry.generation_id);
        }

        fn save_covering_evidence(repo: &Path, session: &str, derived: bool) -> String {
            use crate::cli::verification_record as vr;
            vr::save_plan(
                repo,
                &vr::VerificationPlanRecord {
                    session_id: session.to_string(),
                    owner_number: Some(3248),
                    execution_binding: None,
                    commands: vec!["git --version".to_string()],
                    derived,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            let (record, _) =
                vr::run_verification(repo, session, &["git --version".to_string()]).unwrap();
            record.record_id
        }

        // SPEC-3248 FR-194..FR-196 / AS-172, AS-175, AS-176: a terminal
        // Blocked execution can recover in the same owning session only from
        // fresh, derived, post-block evidence. Recovery remains distinct from
        // completion and preserves an append-only audit entry.
        #[test]
        fn reopen_recovers_verified_same_session_and_preserves_audit() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(dir.path(), &active_record("sess-reopen")).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            let active_binding = current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap();
            persist_generation_session_binding(
                dir.path(),
                owner,
                "sess-reopen",
                active_binding.clone(),
            );
            let blocked = settle_blocked(dir.path(), "sess-reopen");
            let blocked_at = blocked.settled_at.unwrap();
            let blocked_binding = current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap();
            assert_eq!(blocked_binding.generation_id, active_binding.generation_id);
            assert_ne!(
                blocked_binding.ledger_head_hash,
                active_binding.ledger_head_hash
            );
            let verification_record_id = save_covering_evidence(dir.path(), "sess-reopen", true);

            let (code, out) = run_cmd(
                dir.path(),
                ExecutionCommand::Reopen {
                    reason: "user confirmed the resolved dependency and requested PR handoff"
                        .to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 0, "{out}");
            assert!(out.contains("reopened"), "{out}");
            assert!(
                out.contains(r#""outcome":"deferred""#) && out.contains("obligation_state_missing"),
                "{out}"
            );

            let reopened = load(dir.path()).unwrap().unwrap();
            assert_eq!(reopened.status, ExecutionControlStatus::Active);
            assert_eq!(reopened.blocked_reason, None);
            assert_eq!(reopened.missing_verification, None);
            assert_eq!(reopened.settled_at, None);
            assert_eq!(reopened.recoveries.len(), 1);
            let reopened_binding = current_execution_binding(dir.path(), owner)
                .unwrap()
                .unwrap();
            assert_eq!(reopened_binding.generation_id, active_binding.generation_id);
            assert_ne!(
                reopened_binding.ledger_head_hash,
                blocked_binding.ledger_head_hash
            );
            let generation_ledger = load_generation_ledger(dir.path(), owner).unwrap().unwrap();
            assert_eq!(generation_ledger.generations.len(), 1);
            assert_eq!(generation_ledger.lifecycle_events.len(), 2);
            assert_eq!(
                generation_ledger.current_effective_status(),
                Some(ExecutionControlStatus::Active)
            );
            let recovery = &reopened.recoveries[0];
            assert_eq!(recovery.session_id, "sess-reopen");
            assert_eq!(
                recovery.prior_blocked_reason.as_deref(),
                Some("verification dependency unresolved")
            );
            assert_eq!(
                recovery.prior_missing_verification.as_deref(),
                Some("full pre-PR matrix")
            );
            assert_eq!(recovery.blocked_at, blocked_at);
            assert_eq!(recovery.verification_record_id, verification_record_id);
            assert!(!recovery.verification_run_hash.is_empty());
            assert!(!recovery.verification_plan_hash.is_empty());
            assert!(recovery.verification_plan_created_at > blocked_at);
            assert!(recovery.plan_derived);
            assert_eq!(
                recovery.worktree_fingerprint,
                crate::cli::verification_record::worktree_fingerprint(dir.path())
            );
            assert!(recovery.verification_started_at > blocked_at);
            assert!(recovery.verification_created_at > blocked_at);
            assert!(recovery.reopened_at >= recovery.verification_created_at);
            assert!(recovery.previous_recovery_hash.is_empty());
            assert!(!recovery.content_hash.is_empty());
            assert!(integrity_ok(&reopened));

            // PR mutations authenticate the ambient caller against the
            // durable Session's exact post-reopen ledger head before they
            // evaluate verification freshness. Model that production
            // handoff state so this assertion continues to isolate the
            // superseded verification binding rather than a missing Session.
            persist_generation_session_binding(
                dir.path(),
                owner,
                "sess-reopen",
                reopened_binding.clone(),
            );

            // The stored ECR remains readable by both old typed schemas:
            // recoveries are logical-only and their versioned envelopes are
            // anchored in the old-schema-known transfer prefix.
            let stored: ExecutionControlRecord = serde_json::from_str(
                &crate::cli::trusted_store::read(dir.path(), "execution-control.json")
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            assert!(stored.recoveries.is_empty());
            assert!(stored.transfers[0]
                .reason
                .starts_with(RECOVERY_ENVELOPE_PREFIX));
            let mut tampered_history = reopened.clone();
            tampered_history.recoveries[0].reason = "forged recovery".to_string();
            assert!(
                !integrity_ok(&tampered_history),
                "recovery extension tamper must fail its independent hash chain"
            );

            // Reopen advances the same generation's ledger head. The
            // recovery evidence remains in the audit, but exact-binding
            // completion/PR gates require a fresh plan/run for the reopened
            // head rather than accepting the superseded Blocked binding.
            assert!(pr_handoff_refusal(dir.path(), true)
                .is_some_and(|reason| reason.contains("predecessor")));
            save_covering_evidence(dir.path(), "sess-reopen", true);
            assert_eq!(pr_handoff_refusal(dir.path(), true), None);
            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 0, "{out}");
            let completed = load(dir.path()).unwrap().unwrap();
            assert_eq!(completed.status, ExecutionControlStatus::Completed);
            assert_eq!(completed.recoveries.len(), 1);
        }

        #[test]
        fn reopen_honors_derived_plan_generated_output_allowlist() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(dir.path(), &active_record("sess-reopen")).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            settle_blocked(dir.path(), "sess-reopen");
            crate::cli::action_obligation::mark_from_prompt(
                dir.path(),
                "sess-reopen",
                "Issue #3248 にコメントを追加して",
            )
            .unwrap();
            crate::cli::action_obligation::defer_all_best_effort(
                dir.path(),
                "sess-reopen",
                "offline",
            );

            let report = dir.path().join("artifacts/report.json");
            fs::create_dir_all(report.parent().unwrap()).unwrap();
            fs::write(&report, "{\"run\":1}").unwrap();
            let commands = vec!["git --version".to_string()];
            crate::cli::verification_record::save_plan(
                dir.path(),
                &crate::cli::verification_record::VerificationPlanRecord {
                    session_id: "sess-reopen".to_string(),
                    owner_number: Some(3248),
                    execution_binding: None,
                    commands: commands.clone(),
                    derived: true,
                    surfaces: vec!["rust(gwt)".to_string()],
                    generated_outputs: vec!["artifacts/report.json".to_string()],
                    worktree_fingerprint: String::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            crate::cli::verification_record::run_verification(dir.path(), "sess-reopen", &commands)
                .unwrap();
            fs::write(&report, "{\"run\":2}").unwrap();

            assert_eq!(
                crate::cli::verification_record::evaluate_evidence(
                    dir.path(),
                    "sess-reopen",
                    Some(3248),
                ),
                crate::cli::verification_record::EvidenceStatus::Fresh
            );
            let initial_diagnosis = diagnose(dir.path(), Some("sess-reopen"));
            assert_eq!(
                initial_diagnosis.generated_outputs,
                vec!["artifacts/report.json"]
            );
            assert_eq!(initial_diagnosis.verification_state, "fresh");
            let undeclared = dir.path().join("unexpected-output.txt");
            fs::write(&undeclared, "not declared by verify.plan").unwrap();
            assert_eq!(
                diagnose(dir.path(), Some("sess-reopen")).verification_state,
                "stale_fingerprint"
            );
            fs::remove_file(undeclared).unwrap();
            assert_eq!(
                diagnose(dir.path(), Some("sess-reopen")).verification_state,
                "fresh"
            );
            let mut out = String::new();
            assert_eq!(
                run_reopen(
                    dir.path(),
                    "sess-reopen",
                    "generated report refreshed after verification",
                    &mut out,
                )
                .unwrap(),
                0,
                "{out}"
            );
            assert!(out.contains("reopened"), "{out}");
            assert!(out.contains(r#""outcome":"revived""#), "{out}");
            assert!(matches!(
                diagnose(dir.path(), Some("sess-reopen")).obligation_revival,
                Some(ExecutionObligationRevivalDiagnosis::Revived { ref kinds })
                    if kinds == &[crate::cli::action_obligation::ObligationKind::IssueUpdate]
            ));
        }

        #[test]
        fn every_trivial_derivation_reaches_fresh_evidence_and_reopens_blocked_execution() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");
            let cases = tempfile::tempdir().unwrap();
            for (index, reason) in [
                "ledger_only",
                "deletion_only",
                "integration_branch",
                "merge_base_unavailable",
            ]
            .into_iter()
            .enumerate()
            {
                let worktree = cases.path().join(reason);
                fs::create_dir_all(&worktree).unwrap();
                crate::cli::trusted_store::init_git_repo_with_origin(&worktree);
                let git = |args: &[&str]| {
                    let status = gwt_core::process::hidden_command("git")
                        .arg("-C")
                        .arg(&worktree)
                        .args(args)
                        .status()
                        .unwrap();
                    assert!(status.success(), "{reason}: git {args:?}");
                };
                match reason {
                    "ledger_only" => {
                        git(&["update-ref", "refs/remotes/origin/develop", "HEAD"]);
                        git(&["checkout", "-q", "-b", "work/ledger-only"]);
                    }
                    "deletion_only" => {
                        git(&["update-ref", "refs/remotes/origin/develop", "HEAD"]);
                        git(&["checkout", "-q", "-b", "work/deletion-only"]);
                        fs::write(worktree.join("notes.md"), "# Notes\n").unwrap();
                        git(&["add", "notes.md"]);
                        git(&["commit", "-qm", "docs: add notes"]);
                        fs::remove_file(worktree.join("notes.md")).unwrap();
                    }
                    "integration_branch" => {
                        git(&["update-ref", "refs/remotes/origin/develop", "HEAD"]);
                        git(&["checkout", "-q", "-B", "develop"]);
                    }
                    "merge_base_unavailable" => {
                        git(&["checkout", "-q", "-b", "work/no-integration-base"]);
                    }
                    _ => unreachable!(),
                }

                let owner = ExecutionOwnerKey {
                    kind: ExecutionOwnerKind::Spec,
                    number: 3248 + index as u64,
                };
                let mut active = active_record("sess-reopen");
                active.owner_number = owner.number;
                save(&worktree, &active).unwrap();
                ensure_generation_ledger(&worktree, owner, LegacyActiveDisposition::Live).unwrap();
                let binding = current_execution_binding(&worktree, owner)
                    .unwrap()
                    .unwrap();
                persist_generation_session_binding(&worktree, owner, "sess-reopen", binding);
                settle_blocked(&worktree, "sess-reopen");

                let mut env = TestEnv::new(worktree.clone());
                let (plan_code, plan_out) = run_collect(
                    &mut env,
                    CliCommand::Verify(crate::cli::verification_record::VerifyCommand::Plan {
                        commands: Vec::new(),
                        derive: true,
                    }),
                )
                .unwrap();
                assert_eq!(plan_code, 0, "{reason}: {plan_out}");
                let plan = crate::cli::verification_record::load_plan(&worktree)
                    .unwrap()
                    .expect("derived plan");
                assert!(plan.derived, "{reason}: {plan:?}");
                assert!(plan.commands.is_empty(), "{reason}: {plan:?}");
                assert_eq!(
                    plan.surfaces,
                    vec![format!("trivial({reason})")],
                    "{reason}: verify.plan must persist the actual derivation result"
                );

                let mut env = TestEnv::new(worktree.clone());
                let (run_code, run_out) = run_collect(
                    &mut env,
                    CliCommand::Verify(crate::cli::verification_record::VerifyCommand::Run {
                        commands: Vec::new(),
                    }),
                )
                .unwrap();
                assert_eq!(run_code, 0, "{reason}: {run_out}");
                let verification = crate::cli::verification_record::load(&worktree)
                    .unwrap()
                    .expect("empty verification run");
                assert!(verification.all_passed, "{reason}: {verification:?}");
                assert!(verification.plan_covered, "{reason}: {verification:?}");
                let diagnosis = diagnose(&worktree, Some("sess-reopen"));
                assert_eq!(diagnosis.verification_state, "fresh", "{reason}");
                assert_eq!(
                    diagnosis.trivial_reason.as_deref(),
                    Some(reason),
                    "{reason}: execution.status must expose the derived reason directly"
                );

                let mut out = String::new();
                assert_eq!(
                    run_reopen(
                        &worktree,
                        "sess-reopen",
                        &format!("{reason} matrix is canonically fresh"),
                        &mut out,
                    )
                    .unwrap(),
                    0,
                    "{reason}: {out}"
                );
                assert!(out.contains("reopened"), "{reason}: {out}");
                assert_eq!(
                    load(&worktree).unwrap().unwrap().status,
                    ExecutionControlStatus::Active,
                    "{reason}: execution.reopen must restore Active"
                );
            }
        }

        #[test]
        fn reopen_reports_obligation_persist_failed_without_rolling_back_ecr() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            save(dir.path(), &active_record("sess-reopen")).unwrap();
            ensure_generation_ledger(dir.path(), owner, LegacyActiveDisposition::Live).unwrap();
            settle_blocked(dir.path(), "sess-reopen");
            save_covering_evidence(dir.path(), "sess-reopen", true);
            let trusted_dir =
                crate::cli::trusted_store::trusted_dir_for_worktree(dir.path()).unwrap();
            fs::write(trusted_dir.join("action-obligations.json"), b"{corrupt").unwrap();
            crate::cli::action_obligation::set_revival_record_write_failures(2);

            let mut out = String::new();
            assert_eq!(
                run_reopen(
                    dir.path(),
                    "sess-reopen",
                    "verification is now available",
                    &mut out,
                )
                .unwrap(),
                0,
                "{out}"
            );
            assert!(out.contains(r#""outcome":"persist_failed""#), "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Active,
                "obligation outcome persistence does not roll back a verified ECR reopen"
            );
            let diagnosis = diagnose(dir.path(), Some("sess-reopen"));
            assert!(matches!(
                diagnosis.obligation_revival,
                Some(ExecutionObligationRevivalDiagnosis::StatusUnreadable {
                    ref error
                }) if error == "revival_outcome_missing_after_reopen"
            ));
        }

        // FR-195 / AS-173: evidence must exist after the block and come from
        // a derived plan. Every refusal leaves the terminal record untouched.
        #[test]
        fn reopen_refuses_missing_pre_block_and_non_derived_evidence() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");

            let missing = tempfile::tempdir().unwrap();
            settle_blocked(missing.path(), "sess-reopen");
            let (code, out) = run_cmd(
                missing.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("verify.run"), "{out}");
            assert_eq!(
                load(missing.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Blocked
            );

            let pre_block = tempfile::tempdir().unwrap();
            save(pre_block.path(), &active_record("sess-reopen")).unwrap();
            save_covering_evidence(pre_block.path(), "sess-reopen", true);
            settle(
                pre_block.path(),
                "sess-reopen",
                ExecutionSettlement::Blocked {
                    reason: "later blocker".to_string(),
                    missing_verification: None,
                },
            )
            .unwrap();
            let (code, out) = run_cmd(
                pre_block.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("after the block"), "{out}");

            let non_derived = tempfile::tempdir().unwrap();
            settle_blocked(non_derived.path(), "sess-reopen");
            save_covering_evidence(non_derived.path(), "sess-reopen", false);
            let (code, out) = run_cmd(
                non_derived.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("derived verification plan"), "{out}");
            let unchanged = load(non_derived.path()).unwrap().unwrap();
            assert_eq!(unchanged.status, ExecutionControlStatus::Blocked);
            assert!(unchanged.recoveries.is_empty());
        }

        // FR-194 / AS-174: terminal completion is immutable and a different
        // session cannot reopen the owning session's blocked execution.
        #[test]
        fn reopen_refuses_completed_and_wrong_session_records() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");

            let completed = tempfile::tempdir().unwrap();
            let mut completed_record = active_record("sess-reopen");
            completed_record.status = ExecutionControlStatus::Completed;
            completed_record.settled_at = Some(Utc::now());
            save(completed.path(), &completed_record).unwrap();
            let (code, out) = run_cmd(
                completed.path(),
                ExecutionCommand::Reopen {
                    reason: "must stay completed".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("Completed"), "{out}");

            let other_owner = tempfile::tempdir().unwrap();
            settle_blocked(other_owner.path(), "sess-owner");
            let (code, out) = run_cmd(
                other_owner.path(),
                ExecutionCommand::Reopen {
                    reason: "unauthorized".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("sess-owner"), "{out}");
            assert_eq!(
                load(other_owner.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Blocked
            );

            let other_active = tempfile::tempdir().unwrap();
            save(other_active.path(), &active_record("sess-owner")).unwrap();
            let (code, out) = run_cmd(
                other_active.path(),
                ExecutionCommand::Reopen {
                    reason: "unauthorized active retry".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(
                out.contains("Active record belongs to session sess-owner"),
                "{out}"
            );
        }

        // FR-195 / FR-198 / AS-177: a run must remain bound to the exact
        // derived plan it covered. Replacing an explicit or derived plan
        // after the run cannot manufacture reopen eligibility.
        #[test]
        fn reopen_refuses_plan_substitution_after_verification_run() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");

            let explicit_then_derived = tempfile::tempdir().unwrap();
            settle_blocked(explicit_then_derived.path(), "sess-reopen");
            save_covering_evidence(explicit_then_derived.path(), "sess-reopen", false);
            crate::cli::verification_record::save_plan(
                explicit_then_derived.path(),
                &crate::cli::verification_record::VerificationPlanRecord {
                    session_id: "sess-reopen".to_string(),
                    owner_number: Some(3248),
                    execution_binding: None,
                    commands: vec!["git --version".to_string()],
                    derived: true,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            let (code, out) = run_cmd(
                explicit_then_derived.path(),
                ExecutionCommand::Reopen {
                    reason: "substituted plan".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("plan changed"), "{out}");

            let derived_a_then_b = tempfile::tempdir().unwrap();
            settle_blocked(derived_a_then_b.path(), "sess-reopen");
            save_covering_evidence(derived_a_then_b.path(), "sess-reopen", true);
            crate::cli::verification_record::save_plan(
                derived_a_then_b.path(),
                &crate::cli::verification_record::VerificationPlanRecord {
                    session_id: "sess-reopen".to_string(),
                    owner_number: Some(3248),
                    execution_binding: None,
                    commands: vec!["git --exec-path".to_string()],
                    derived: true,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            let (code, out) = run_cmd(
                derived_a_then_b.path(),
                ExecutionCommand::Reopen {
                    reason: "different derived plan".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("plan changed"), "{out}");
            assert_eq!(
                load(derived_a_then_b.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Blocked
            );
        }

        // FR-195 / AS-173: recovery never accepts legacy hashless evidence,
        // malformed terminal state, or a run whose commands started before
        // the terminal block.
        #[test]
        fn reopen_refuses_hashless_malformed_and_prestarted_state() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");

            let malformed = tempfile::tempdir().unwrap();
            let mut malformed_record = settle_blocked(malformed.path(), "sess-reopen");
            malformed_record.settled_at = None;
            save(malformed.path(), &malformed_record).unwrap();
            let (code, out) = run_cmd(
                malformed.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("settled_at"), "{out}");

            let empty_blocker = tempfile::tempdir().unwrap();
            let mut empty_blocker_record = settle_blocked(empty_blocker.path(), "sess-reopen");
            empty_blocker_record.blocked_reason = Some("   ".to_string());
            save(empty_blocker.path(), &empty_blocker_record).unwrap();
            let (code, out) = run_cmd(
                empty_blocker.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("non-empty blocker reason"), "{out}");

            let hashless_ecr = tempfile::tempdir().unwrap();
            let mut legacy = active_record("sess-reopen");
            legacy.status = ExecutionControlStatus::Blocked;
            legacy.blocked_reason = Some("legacy block".to_string());
            legacy.settled_at = Some(Utc::now());
            fs::create_dir_all(state_path(hashless_ecr.path()).parent().unwrap()).unwrap();
            fs::write(
                state_path(hashless_ecr.path()),
                serde_json::to_vec_pretty(&legacy).unwrap(),
            )
            .unwrap();
            let (code, out) = run_cmd(
                hashless_ecr.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("integrity hash"), "{out}");

            let hashless_plan = tempfile::tempdir().unwrap();
            settle_blocked(hashless_plan.path(), "sess-reopen");
            save_covering_evidence(hashless_plan.path(), "sess-reopen", true);
            let mut plan = crate::cli::verification_record::load_plan(hashless_plan.path())
                .unwrap()
                .unwrap();
            plan.content_hash.clear();
            fs::write(
                crate::cli::verification_record::plan_state_path(hashless_plan.path()),
                serde_json::to_vec_pretty(&plan).unwrap(),
            )
            .unwrap();
            let (code, out) = run_cmd(
                hashless_plan.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("plan hash"), "{out}");

            let hashless_run = tempfile::tempdir().unwrap();
            settle_blocked(hashless_run.path(), "sess-reopen");
            save_covering_evidence(hashless_run.path(), "sess-reopen", true);
            let mut run = crate::cli::verification_record::load(hashless_run.path())
                .unwrap()
                .unwrap();
            run.content_hash.clear();
            fs::write(
                crate::cli::verification_record::state_path(hashless_run.path()),
                serde_json::to_vec_pretty(&run).unwrap(),
            )
            .unwrap();
            let (code, out) = run_cmd(
                hashless_run.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("verification run"), "{out}");
            assert!(out.contains("integrity hash"), "{out}");

            let prestarted = tempfile::tempdir().unwrap();
            let blocked = settle_blocked(prestarted.path(), "sess-reopen");
            save_covering_evidence(prestarted.path(), "sess-reopen", true);
            let mut run = crate::cli::verification_record::load(prestarted.path())
                .unwrap()
                .unwrap();
            run.started_at = Some(blocked.settled_at.unwrap() - chrono::Duration::seconds(1));
            crate::cli::verification_record::save(prestarted.path(), &run).unwrap();
            let (code, out) = run_cmd(
                prestarted.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("start after the block"), "{out}");
        }

        #[test]
        fn reopen_is_idempotent_for_current_active_owner() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), &active_record("sess-reopen")).unwrap();
            let (code, out) = run_cmd(
                dir.path(),
                ExecutionCommand::Reopen {
                    reason: "idempotent retry".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 0, "{out}");
            assert!(out.contains("already active"), "{out}");
        }

        #[test]
        fn idempotent_reopen_upgrades_initial_recovery_schema() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");
            let dir = tempfile::tempdir().unwrap();
            let now = Utc::now();
            let mut transitional = active_record("sess-reopen");
            transitional.recoveries.push(ExecutionRecovery {
                session_id: "sess-reopen".to_string(),
                reason: "initial recovery schema".to_string(),
                prior_blocked_reason: Some("temporary blocker".to_string()),
                prior_missing_verification: None,
                blocked_at: now,
                verification_record_id: "vrr-transition".to_string(),
                verification_run_hash: "run-hash".to_string(),
                verification_plan_hash: "plan-hash".to_string(),
                verification_plan_created_at: now,
                plan_derived: true,
                worktree_fingerprint: "fingerprint".to_string(),
                verification_started_at: now,
                verification_created_at: now,
                reopened_at: now,
                previous_recovery_hash: String::new(),
                content_hash: String::new(),
            });
            transitional.content_hash = compute_legacy_hash_with_recoveries(&transitional);
            let path = state_path(dir.path());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, serde_json::to_vec_pretty(&transitional).unwrap()).unwrap();
            assert!(integrity_ok(&transitional));
            assert!(recovery_storage_needs_upgrade(dir.path()).unwrap());
            let mut forged_transition = transitional.clone();
            forged_transition.recoveries[0].previous_recovery_hash = "forged".to_string();
            forged_transition.content_hash =
                compute_legacy_hash_with_recoveries(&forged_transition);
            assert!(!integrity_ok(&forged_transition));
            let mut unsupported_intermediate = transitional.clone();
            stamp_recovery_chain(&mut unsupported_intermediate.recoveries);
            let mut old_projection = unsupported_intermediate.clone();
            old_projection.recoveries.clear();
            old_projection.content_hash.clear();
            unsupported_intermediate.content_hash = format!(
                "{:x}",
                <sha2::Sha256 as sha2::Digest>::digest(
                    serde_json::to_vec(&old_projection).unwrap()
                )
            );
            assert!(
                !integrity_ok(&unsupported_intermediate),
                "only the initial unchained whole-record schema is migratable"
            );

            let (code, out) = run_cmd(
                dir.path(),
                ExecutionCommand::Reopen {
                    reason: "normalize rolling-compatible integrity".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 0, "{out}");
            assert!(out.contains("already active"), "{out}");

            let upgraded = load(dir.path()).unwrap().unwrap();
            assert_eq!(upgraded.recoveries.len(), 1);
            assert_ne!(upgraded.content_hash, transitional.content_hash);
            assert!(!upgraded.recoveries[0].content_hash.is_empty());
            assert_eq!(upgraded.content_hash, compute_content_hash(&upgraded));
            assert!(integrity_ok(&upgraded));
            assert!(!recovery_storage_needs_upgrade(dir.path()).unwrap());
        }

        #[test]
        fn evidence_bound_transitions_contend_on_owner_write_lease() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), &active_record("sess-reopen")).unwrap();

            let worktree = dir.path().to_path_buf();
            let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
            let holder = std::thread::spawn(move || {
                crate::cli::trusted_store::with_write_lease(&worktree, || {
                    acquired_tx.send(()).unwrap();
                    let _ = release_rx.recv_timeout(std::time::Duration::from_secs(10));
                    Ok(())
                })
                .unwrap();
            });
            acquired_rx.recv().unwrap();

            let complete = run_cmd(dir.path(), ExecutionCommand::Complete)
                .expect_err("completion must contend on the owner lease");
            assert!(complete.to_string().contains("retry"), "{complete}");
            let reopen = run_cmd(
                dir.path(),
                ExecutionCommand::Reopen {
                    reason: "retry after lease".to_string(),
                },
            )
            .expect_err("reopen must contend on the owner lease");
            assert!(reopen.to_string().contains("retry"), "{reopen}");

            release_tx.send(()).unwrap();
            holder.join().unwrap();
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Active
            );
        }

        #[test]
        fn terminal_tamper_diagnostics_recommend_canonical_repair_not_adopt() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");
            let dir = tempfile::tempdir().unwrap();
            settle_blocked(dir.path(), "sess-reopen");
            let path = state_path(dir.path());
            let tampered = fs::read_to_string(&path)
                .unwrap()
                .replace("verification dependency unresolved", "forged blocker");
            fs::write(&path, tampered).unwrap();

            let refusal = pr_handoff_refusal(dir.path(), true).unwrap();
            assert!(refusal.contains("execution.repair"), "{refusal}");
            assert!(refusal.contains("quarantines"), "{refusal}");
            assert!(
                !refusal.contains("Repair it with JSON operation `execution.adopt`"),
                "{refusal}"
            );

            let (code, out) = run_cmd(
                dir.path(),
                ExecutionCommand::Blocked {
                    reason: "still blocked".to_string(),
                    missing_verification: None,
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("execution.repair"), "{out}");
            assert!(out.contains("quarantines"), "{out}");
            assert!(
                !out.contains("Repair it with JSON operation `execution.adopt`"),
                "{out}"
            );
        }

        #[test]
        fn reopen_rejection_matrix_preserves_blocked_record_byte_for_byte() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-reopen");
            use crate::cli::verification_record as vr;

            let failing = tempfile::tempdir().unwrap();
            settle_blocked(failing.path(), "sess-reopen");
            let failing_before = load(failing.path()).unwrap().unwrap();
            let failing_command = "git definitely-not-a-subcommand".to_string();
            vr::save_plan(
                failing.path(),
                &vr::VerificationPlanRecord {
                    session_id: "sess-reopen".to_string(),
                    owner_number: Some(3248),
                    execution_binding: None,
                    commands: vec![failing_command.clone()],
                    derived: true,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            vr::run_verification(failing.path(), "sess-reopen", &[failing_command]).unwrap();
            let (code, out) = run_cmd(
                failing.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("failing commands"), "{out}");
            assert_eq!(load(failing.path()).unwrap().unwrap(), failing_before);

            let uncovered = tempfile::tempdir().unwrap();
            settle_blocked(uncovered.path(), "sess-reopen");
            let uncovered_before = load(uncovered.path()).unwrap().unwrap();
            vr::save_plan(
                uncovered.path(),
                &vr::VerificationPlanRecord {
                    session_id: "sess-reopen".to_string(),
                    owner_number: Some(3248),
                    execution_binding: None,
                    commands: vec!["git --version".to_string(), "git --exec-path".to_string()],
                    derived: true,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            vr::run_verification(
                uncovered.path(),
                "sess-reopen",
                &["git --version".to_string()],
            )
            .unwrap();
            let (code, out) = run_cmd(
                uncovered.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("does not cover"), "{out}");
            assert_eq!(load(uncovered.path()).unwrap().unwrap(), uncovered_before);

            let wrong_owner = tempfile::tempdir().unwrap();
            settle_blocked(wrong_owner.path(), "sess-reopen");
            let wrong_owner_before = load(wrong_owner.path()).unwrap().unwrap();
            save_covering_evidence(wrong_owner.path(), "sess-reopen", true);
            let mut wrong_owner_run = vr::load(wrong_owner.path()).unwrap().unwrap();
            wrong_owner_run.owner_number = Some(999);
            vr::save(wrong_owner.path(), &wrong_owner_run).unwrap();
            let (code, out) = run_cmd(
                wrong_owner.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("different owner"), "{out}");
            assert_eq!(
                load(wrong_owner.path()).unwrap().unwrap(),
                wrong_owner_before
            );

            let tampered = tempfile::tempdir().unwrap();
            settle_blocked(tampered.path(), "sess-reopen");
            let tampered_before = load(tampered.path()).unwrap().unwrap();
            save_covering_evidence(tampered.path(), "sess-reopen", true);
            let run_path = vr::state_path(tampered.path());
            let forged = fs::read_to_string(&run_path)
                .unwrap()
                .replace("git --version", "git --exec-path");
            fs::write(run_path, forged).unwrap();
            let (code, out) = run_cmd(
                tampered.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("valid integrity hash"), "{out}");
            assert_eq!(load(tampered.path()).unwrap().unwrap(), tampered_before);

            let stale = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(stale.path());
            settle_blocked(stale.path(), "sess-reopen");
            let stale_before = load(stale.path()).unwrap().unwrap();
            save_covering_evidence(stale.path(), "sess-reopen", true);
            fs::write(stale.path().join("post-run-change.rs"), "fn changed() {}\n").unwrap();
            let (code, out) = run_cmd(
                stale.path(),
                ExecutionCommand::Reopen {
                    reason: "resolved".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("worktree changed"), "{out}");
            assert_eq!(load(stale.path()).unwrap().unwrap(), stale_before);
        }

        #[test]
        fn concurrent_reopen_is_idempotent_and_recovery_history_is_append_only() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let dir = tempfile::tempdir().unwrap();
            settle_blocked(dir.path(), "sess-reopen");
            save_covering_evidence(dir.path(), "sess-reopen", true);

            let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
            let mut workers = Vec::new();
            for reason in ["concurrent recovery A", "concurrent recovery B"] {
                let worktree = dir.path().to_path_buf();
                let barrier = barrier.clone();
                workers.push(std::thread::spawn(move || {
                    barrier.wait();
                    let mut out = String::new();
                    let code = run_reopen(&worktree, "sess-reopen", reason, &mut out).unwrap();
                    (code, out)
                }));
            }
            barrier.wait();
            let results: Vec<(i32, String)> = workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect();
            assert!(results.iter().all(|(code, _)| *code == 0), "{results:?}");
            assert!(
                results.iter().any(|(_, out)| out.contains("reopened")),
                "{results:?}"
            );
            assert!(
                results
                    .iter()
                    .any(|(_, out)| out.contains("already active")),
                "{results:?}"
            );
            let after_first = load(dir.path()).unwrap().unwrap();
            assert_eq!(after_first.recoveries.len(), 1);
            let first_recovery = after_first.recoveries[0].clone();

            settle(
                dir.path(),
                "sess-reopen",
                ExecutionSettlement::Blocked {
                    reason: "second genuine blocker".to_string(),
                    missing_verification: Some("second matrix".to_string()),
                },
            )
            .unwrap();
            save_covering_evidence(dir.path(), "sess-reopen", true);
            let mut out = String::new();
            assert_eq!(
                run_reopen(
                    dir.path(),
                    "sess-reopen",
                    "second blocker resolved",
                    &mut out
                )
                .unwrap(),
                0,
                "{out}"
            );
            let after_second = load(dir.path()).unwrap().unwrap();
            assert_eq!(after_second.recoveries.len(), 2);
            assert_eq!(after_second.recoveries[0], first_recovery);
            assert_eq!(
                after_second.recoveries[1].prior_blocked_reason.as_deref(),
                Some("second genuine blocker")
            );
            assert_eq!(
                after_second.recoveries[1]
                    .prior_missing_verification
                    .as_deref(),
                Some("second matrix")
            );
        }

        // T-124: unauthorized settlement attempts (session mismatch) and
        // tampered-record refusals auto-capture one deduped
        // issue-spec-workflow improvement candidate.
        #[test]
        fn complete_refused_while_obligations_open_then_defer_clears() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-t247");
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), &active_record("sess-t247")).unwrap();

            // An unsettled issue-update obligation refuses completion BEFORE
            // the evidence gate (the message names the obligation, not the
            // missing verification record).
            crate::cli::action_obligation::mark_from_prompt(
                dir.path(),
                "sess-t247",
                "Issue #1 にコメントを追加して",
            )
            .unwrap();
            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("open action obligations"), "{out}");
            assert!(out.contains("issue_update"), "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Active
            );

            // Deferring through execution.blocked clears the refusal; the
            // next completion attempt reaches the evidence gate instead.
            let (code, out) = run_cmd(
                dir.path(),
                ExecutionCommand::Blocked {
                    reason: "cannot comment".to_string(),
                    missing_verification: None,
                },
            )
            .unwrap();
            assert_eq!(code, 0, "{out}");
            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(
                code, 0,
                "already settled records report idempotently: {out}"
            );
            assert!(!out.contains("open action obligations"), "{out}");
        }

        // T-247: build.complete settlement skips while obligations stay
        // open — the ECR keeps holding the Stop gate.
        #[test]
        fn best_effort_settlement_skips_while_obligations_open() {
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), &active_record("sess-t247b")).unwrap();
            crate::cli::action_obligation::mark_from_prompt(dir.path(), "sess-t247b", "実装して")
                .unwrap();
            settle_completed_best_effort(dir.path(), "sess-t247b", 3248);
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Active,
                "open obligations must keep the record active"
            );
        }

        #[test]
        fn store_health_failure_surfaces_repair_guidance() {
            // T-177 core: a malformed record at a canonical operation
            // surfaces the store-health repair path, not a raw
            // network-flavored error. Hook readers stay fail-open.
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-health");
            let dir = tempfile::tempdir().unwrap();
            let path = state_path(dir.path());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "{not json").unwrap();

            let err = run_cmd(
                dir.path(),
                ExecutionCommand::Blocked {
                    reason: "runner down".to_string(),
                    missing_verification: None,
                },
            )
            .unwrap_err();
            let message = err.to_string();
            assert!(message.contains("trusted state unhealthy"), "{message}");
            assert!(message.contains("execution.repair"), "{message}");
            assert!(message.contains("execution.adopt"), "{message}");
            assert!(!message.contains("network"), "{message}");

            // The Stop gate keeps failing open on the same malformed state.
            assert_eq!(
                crate::cli::hook::execution_control_stop_check::handle_with_input(
                    dir.path(),
                    "{}",
                    Some("sess-health"),
                ),
                crate::cli::hook::HookOutput::Silent
            );
        }

        #[test]
        fn blocked_defers_obligations_on_no_record_and_already_settled() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-ob2");
            let dir = tempfile::tempdir().unwrap();

            // NoRecord path (unlinked launch): blocked still defers.
            crate::cli::action_obligation::mark_from_prompt(dir.path(), "sess-ob2", "実装して")
                .unwrap();
            let (code, out) = run_cmd(
                dir.path(),
                ExecutionCommand::Blocked {
                    reason: "runner unavailable".to_string(),
                    missing_verification: None,
                },
            )
            .unwrap();
            assert_eq!(code, 0, "{out}");
            assert!(out.contains("obligations deferred"), "{out}");
            assert!(crate::cli::action_obligation::open_kinds(dir.path(), "sess-ob2").is_empty());

            // AlreadySettled path: a settled record must not strand newly
            // armed obligations either.
            let mut settled = active_record("sess-ob2");
            settled.status = ExecutionControlStatus::Completed;
            settled.settled_at = Some(Utc::now());
            save(dir.path(), &settled).unwrap();
            crate::cli::action_obligation::mark_from_prompt(dir.path(), "sess-ob2", "検証して")
                .unwrap();
            let (code, out) = run_cmd(
                dir.path(),
                ExecutionCommand::Blocked {
                    reason: "cannot verify".to_string(),
                    missing_verification: None,
                },
            )
            .unwrap();
            assert_eq!(code, 0, "{out}");
            assert!(out.contains("obligations deferred"), "{out}");
            assert!(crate::cli::action_obligation::open_kinds(dir.path(), "sess-ob2").is_empty());
        }

        #[test]
        fn settlement_refusals_capture_improvement_candidate() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-intruder");
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), &active_record("sess-owner")).unwrap();

            // Unauthorized settle from a non-owner session.
            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("execution.adopt"), "{out}");
            assert!(out.contains("Self-improvement candidate"), "{out}");

            // Tampered record refusal (blocked settle has no evidence gate,
            // so it reaches the integrity check directly).
            let path = state_path(dir.path());
            let tampered = fs::read_to_string(&path)
                .unwrap()
                .replace("$gwt-execute", "$gwt-forged");
            fs::write(&path, tampered).unwrap();
            let (code, out) = run_cmd(
                dir.path(),
                ExecutionCommand::Blocked {
                    reason: "verification runner unavailable".to_string(),
                    missing_verification: None,
                },
            )
            .unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("integrity validation"), "{out}");
            assert!(out.contains("Self-improvement candidate"), "{out}");

            let candidates = crate::cli::improvement::candidate_public_values(dir.path());
            assert_eq!(candidates.len(), 1, "one deduped candidate expected");
            assert_eq!(
                candidates[0]
                    .get("legacy_occurrence_count")
                    .and_then(|v| v.as_u64()),
                Some(2)
            );
            // Owner attribution survives in the deduped candidate details.
            let store_raw = fs::read_to_string(
                crate::cli::improvement_store::candidate_store_path(dir.path()),
            )
            .unwrap();
            assert!(store_raw.contains("spec #3248"), "{store_raw}");

            // Benign retry: a mismatch against an ALREADY SETTLED record is
            // refused but not captured as a violation.
            let mut settled = active_record("sess-owner");
            settled.status = ExecutionControlStatus::Completed;
            settled.settled_at = Some(Utc::now());
            save(dir.path(), &settled).unwrap();
            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(!out.contains("Self-improvement candidate"), "{out}");
            let candidates = crate::cli::improvement::candidate_public_values(dir.path());
            assert_eq!(
                candidates[0]
                    .get("legacy_occurrence_count")
                    .and_then(|v| v.as_u64()),
                Some(2),
                "benign retry must not add an occurrence"
            );
        }

        // T-125: crash/resume handoff lifecycle E2E — the adopt transfer is
        // audited, verification evidence binds to the adopting session, and
        // the Ready PR handoff refuses until that session produces fresh
        // evidence.
        #[test]
        fn adopt_handoff_binds_evidence_to_new_session_and_gates_pr() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-b");
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

            // Session A launched the execution, then crashed.
            materialize_at_launch(
                dir.path(),
                ExecutionOwnerKind::Spec,
                3248,
                "sess-a",
                "$gwt-execute",
                false,
            )
            .unwrap();
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            let recovery_session = persist_recovery_session_snapshot(dir.path(), owner, "sess-b");

            // Session B adopts with an audited reason.
            let mut out = String::new();
            run_adopt(
                dir.path(),
                "sess-b",
                &recovery_session,
                "crash recovery",
                &mut out,
            )
            .unwrap();
            let record = load(dir.path()).unwrap().unwrap();
            assert_eq!(record.primary_session_id, "sess-b");
            assert_eq!(record.transfers.len(), 1);
            assert_eq!(record.transfers[0].from_session_id, "sess-a");
            assert_eq!(record.transfers[0].reason, "crash recovery");
            assert!(integrity_ok(&record));

            // Ready handoff refuses: no evidence for the adopting session.
            let refusal = pr_handoff_refusal(dir.path(), true);
            assert!(
                refusal.as_deref().unwrap_or("").contains("verify.run"),
                "{refusal:?}"
            );

            // Session B registers the plan and runs it through the canonical
            // executor — evidence binds to the new owner session.
            use crate::cli::verification_record as vr;
            vr::save_plan(
                dir.path(),
                &vr::VerificationPlanRecord {
                    session_id: "sess-b".to_string(),
                    owner_number: Some(3248),
                    execution_binding: None,
                    commands: vec!["git --version".to_string()],
                    derived: false,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            let (run_record, _) =
                vr::run_verification(dir.path(), "sess-b", &["git --version".to_string()]).unwrap();
            assert!(run_record.all_passed && run_record.plan_covered);
            assert_eq!(
                vr::evaluate_evidence(dir.path(), "sess-b", Some(3248)),
                vr::EvidenceStatus::Fresh
            );
            // The pre-crash session's claim to the same evidence stays dead.
            assert_ne!(
                vr::evaluate_evidence(dir.path(), "sess-a", Some(3248)),
                vr::EvidenceStatus::Fresh
            );

            // Ready handoff now passes, and session B settles cleanly.
            assert_eq!(pr_handoff_refusal(dir.path(), true), None);
            let result = settle(dir.path(), "sess-b", ExecutionSettlement::Completed).unwrap();
            assert!(matches!(result, SettleResult::Settled(_)));
        }

        #[test]
        fn complete_op_settles_record_for_current_session() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-op");
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), &active_record("sess-op")).unwrap();

            // T-111: completion without tool-generated evidence is refused.
            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("verify.run"), "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Active
            );

            // Fresh all-passing evidence (plan + covering run) unlocks it.
            crate::cli::verification_record::save_plan(
                dir.path(),
                &crate::cli::verification_record::VerificationPlanRecord {
                    session_id: "sess-op".to_string(),
                    owner_number: Some(3248),
                    execution_binding: None,
                    commands: vec!["git --version".to_string()],
                    derived: false,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            crate::cli::verification_record::run_verification(
                dir.path(),
                "sess-op",
                &["git --version".to_string()],
            )
            .unwrap();
            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 0, "{out}");
            assert!(out.contains("completed"), "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Completed
            );
        }

        #[test]
        fn complete_op_refuses_dirty_work_event_before_terminal_mutation() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-op");
            let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
            save(&fixture.repo, &active_record("sess-op")).unwrap();
            save_covering_evidence(&fixture.repo, "sess-op", false);
            fixture.append_event("terminal-update-awaiting-delivery");

            let (code, out) =
                run_cmd(&fixture.repo, ExecutionCommand::Complete).expect("run completion gate");

            assert_eq!(code, 2, "{out}");
            assert!(out.contains(".gwt/work/events.jsonl"), "{out}");
            assert!(out.contains("commit"), "{out}");
            assert!(out.contains("push"), "{out}");
            assert_eq!(
                load(&fixture.repo).unwrap().unwrap().status,
                ExecutionControlStatus::Active,
                "the execution record must stay active when Work delivery is unsettled"
            );
        }

        // T-111: a failing verification run never unlocks completion, while
        // execution.blocked stays available without evidence.
        #[test]
        fn failing_evidence_refuses_complete_but_blocked_stays_available() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-op");
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), &active_record("sess-op")).unwrap();
            crate::cli::verification_record::run_verification(
                dir.path(),
                "sess-op",
                &["git definitely-not-a-subcommand".to_string()],
            )
            .unwrap();

            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("failing"), "{out}");

            let (code, _) = run_cmd(
                dir.path(),
                ExecutionCommand::Blocked {
                    reason: "verification cannot pass in this environment".to_string(),
                    missing_verification: Some("full cargo matrix".to_string()),
                },
            )
            .unwrap();
            assert_eq!(code, 0);
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Blocked
            );
        }

        #[test]
        fn blocked_op_requires_reason_and_records_it() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-op");
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), &active_record("sess-op")).unwrap();

            assert!(run_cmd(
                dir.path(),
                ExecutionCommand::Blocked {
                    reason: "   ".to_string(),
                    missing_verification: None,
                }
            )
            .is_err());

            let (code, out) = run_cmd(
                dir.path(),
                ExecutionCommand::Blocked {
                    reason: "environment blocker".to_string(),
                    missing_verification: None,
                },
            )
            .unwrap();
            assert_eq!(code, 0, "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Blocked
            );
        }

        #[test]
        fn settlement_refuses_other_sessions_record() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-op");
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), &active_record("sess-owner")).unwrap();

            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 2, "{out}");
            assert!(out.contains("refused"), "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Active
            );
        }

        // Review follow-up: a vacuous `build.complete` (no active build
        // state) must not settle the record; a real finalize for the same
        // owner does; a real finalize for a different owner does not.
        #[test]
        fn build_complete_settles_only_real_matching_finalize() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-op");
            let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
            let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
            let _runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
            let dir = tempfile::tempdir().unwrap();
            let mut record = active_record("sess-op");
            record.owner_number = 3248;
            save(dir.path(), &record).unwrap();

            let run_build_complete = |repo: &Path, spec: u64| {
                let mut env = TestEnv::new(repo.to_path_buf());
                run_collect(
                    &mut env,
                    CliCommand::Build(crate::cli::SkillStateAction::Complete { spec }),
                )
                .unwrap()
            };

            // Vacuous: no build state exists — exit 0 but no settlement.
            let (code, out) = run_build_complete(dir.path(), 3248);
            assert_eq!(code, 0, "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Active,
                "vacuous build.complete must not settle the execution"
            );

            // Real finalize for a DIFFERENT owner — no settlement.
            gwt_core::skill_state::save(
                dir.path(),
                "build-spec",
                &gwt_core::skill_state::SkillState {
                    active: true,
                    owner_spec: Some(999),
                    started_at: Utc::now(),
                    phase: None,
                    session_id: "sess-op".to_string(),
                },
            )
            .unwrap();
            let (code, out) = run_build_complete(dir.path(), 999);
            assert_eq!(code, 0, "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Active,
                "a build for another owner must not settle this execution"
            );

            // Real finalize for the SAME owner but WITHOUT verification
            // evidence — build state finalizes, execution stays active
            // (T-111 evidence requirement piggybacks on build.complete).
            gwt_core::skill_state::save(
                dir.path(),
                "build-spec",
                &gwt_core::skill_state::SkillState {
                    active: true,
                    owner_spec: Some(3248),
                    started_at: Utc::now(),
                    phase: None,
                    session_id: "sess-op".to_string(),
                },
            )
            .unwrap();
            let (code, out) = run_build_complete(dir.path(), 3248);
            assert_eq!(code, 0, "{out}");
            assert!(out.contains("execution control not settled"), "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Active,
                "build completion without evidence must not settle the execution"
            );

            // With fresh evidence, a real matching finalize settles.
            gwt_core::skill_state::save(
                dir.path(),
                "build-spec",
                &gwt_core::skill_state::SkillState {
                    active: true,
                    owner_spec: Some(3248),
                    started_at: Utc::now(),
                    phase: None,
                    session_id: "sess-op".to_string(),
                },
            )
            .unwrap();
            crate::cli::verification_record::save_plan(
                dir.path(),
                &crate::cli::verification_record::VerificationPlanRecord {
                    session_id: "sess-op".to_string(),
                    owner_number: Some(3248),
                    execution_binding: None,
                    commands: vec!["git --version".to_string()],
                    derived: false,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            crate::cli::verification_record::run_verification(
                dir.path(),
                "sess-op",
                &["git --version".to_string()],
            )
            .unwrap();
            let (code, out) = run_build_complete(dir.path(), 3248);
            assert_eq!(code, 0, "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Completed,
                "a real matching finalize with fresh evidence must settle the execution"
            );
        }

        #[test]
        fn build_complete_refuses_dirty_work_event_before_finalizing_state() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-op");
            let _runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
            let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
            save(&fixture.repo, &active_record("sess-op")).unwrap();
            gwt_core::skill_state::save(
                &fixture.repo,
                "build-spec",
                &gwt_core::skill_state::SkillState {
                    active: true,
                    owner_spec: Some(3248),
                    started_at: Utc::now(),
                    phase: None,
                    session_id: "sess-op".to_string(),
                },
            )
            .unwrap();
            save_covering_evidence(&fixture.repo, "sess-op", false);
            fixture.append_event("terminal-update-awaiting-delivery");

            let mut env = TestEnv::new(fixture.repo.clone());
            let (code, out) = run_collect(
                &mut env,
                CliCommand::Build(crate::cli::SkillStateAction::Complete { spec: 3248 }),
            )
            .expect("run build completion gate");

            assert_eq!(code, 2, "{out}");
            assert!(out.contains(".gwt/work/events.jsonl"), "{out}");
            assert!(out.contains("commit"), "{out}");
            assert!(out.contains("push"), "{out}");
            assert!(
                gwt_core::skill_state::load(&fixture.repo, "build-spec")
                    .unwrap()
                    .unwrap()
                    .active,
                "build state must remain active while Work delivery is unsettled"
            );
            assert_eq!(
                load(&fixture.repo).unwrap().unwrap().status,
                ExecutionControlStatus::Active,
                "execution state must remain active while Work delivery is unsettled"
            );
        }

        // P9a (T-117): execution.adopt takes over with an audited reason and
        // then allows same-session settlement.
        #[test]
        fn adopt_op_transfers_ownership_with_reason() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-new");
            let dir = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
            save(dir.path(), &active_record("sess-old")).unwrap();
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Spec,
                number: 3248,
            };
            persist_recovery_session_snapshot(dir.path(), owner, "sess-new");

            // Reason is mandatory.
            assert!(run_cmd(
                dir.path(),
                ExecutionCommand::Adopt {
                    reason: "  ".to_string()
                }
            )
            .is_err());
            assert!(run_cmd(
                dir.path(),
                ExecutionCommand::Adopt {
                    reason: format!("{RECOVERY_ENVELOPE_PREFIX}collision")
                }
            )
            .is_err());

            let (code, out) = run_cmd(
                dir.path(),
                ExecutionCommand::Adopt {
                    reason: "crash recovery of the implementing window".to_string(),
                },
            )
            .unwrap();
            assert_eq!(code, 0, "{out}");
            let record = load(dir.path()).unwrap().unwrap();
            assert_eq!(record.primary_session_id, "sess-new");
            assert_eq!(record.transfers.len(), 1);
            assert_eq!(record.transfers[0].from_session_id, "sess-old");
            assert!(integrity_ok(&record));

            // Settlement now works from the adopting session (with evidence).
            crate::cli::verification_record::save_plan(
                dir.path(),
                &crate::cli::verification_record::VerificationPlanRecord {
                    session_id: "sess-new".to_string(),
                    owner_number: Some(3248),
                    execution_binding: None,
                    commands: vec!["git --version".to_string()],
                    derived: false,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    created_at: Utc::now(),
                    content_hash: String::new(),
                },
            )
            .unwrap();
            crate::cli::verification_record::run_verification(
                dir.path(),
                "sess-new",
                &["git --version".to_string()],
            )
            .unwrap();
            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 0, "{out}");
        }

        #[test]
        fn settlement_requires_session_env() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
            let dir = tempfile::tempdir().unwrap();
            let err = run_cmd(dir.path(), ExecutionCommand::Complete)
                .expect_err("missing GWT_SESSION_ID must fail");
            assert!(err.to_string().contains("GWT_SESSION_ID"), "{err}");
        }
    }
}
