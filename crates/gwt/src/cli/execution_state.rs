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
};

use chrono::{DateTime, Utc};
use gwt_github::{client::ApiError, SpecOpsError};
use serde::{Deserialize, Serialize};

use super::CliEnv;

/// Worktree-relative path of the Execution Control Record's mirror (the
/// authoritative copy lives in the repo-scoped trusted store, P9b).
pub const EXECUTION_CONTROL_STATE_RELATIVE: &str = ".gwt/skill-state/execution-control.json";
const RECOVERY_ENVELOPE_PREFIX: &str = "gwt:execution-recovery:v1:";

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

/// Reachable repair guidance for an integrity failure. Adoption can rewrite
/// only Active records; terminal records require a fresh linked-owner launch.
#[must_use]
pub(crate) fn integrity_repair_guidance(status: ExecutionControlStatus) -> &'static str {
    match status {
        ExecutionControlStatus::Active => {
            "An integrity-failed Active record cannot be repaired in the same execution lifetime without risking audit loss; use a fresh linked-owner launch."
        }
        ExecutionControlStatus::Blocked | ExecutionControlStatus::Completed => {
            "A terminal record cannot be repaired with `execution.adopt`; start a fresh linked-owner launch to materialize canonical state."
        }
    }
}

/// Resolve the record path for a worktree.
#[must_use]
pub fn state_path(worktree: &Path) -> PathBuf {
    worktree.join(EXECUTION_CONTROL_STATE_RELATIVE)
}

/// Load the record. `Ok(None)` when missing; malformed JSON and I/O failures
/// propagate so hook readers can fail open while writers surface the error.
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

pub fn load(worktree: &Path) -> io::Result<Option<ExecutionControlRecord>> {
    let Some(contents) = read_record_contents(worktree)? else {
        return Ok(None);
    };
    let record = serde_json::from_str::<ExecutionControlRecord>(&contents)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    Ok(Some(hydrate_recovery_envelopes(record)))
}

/// Whether the execution for `worktree` has settled as `Completed`.
///
/// A completed execution means the Work's final commit / push / PR handoff is
/// done, so a coordination-only `workspace.update` (the kind a post-merge stale
/// reminder triggers) must stop appending to the git-tracked `events.jsonl`
/// (Issue #3278). A missing or unreadable record is treated as *not* completed
/// so unlinked / standalone launches keep their existing append behavior.
#[must_use]
pub fn is_completed(worktree: &Path) -> bool {
    matches!(
        load(worktree),
        Ok(Some(record)) if record.status == ExecutionControlStatus::Completed
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

/// Persist the record atomically (hooks read this file concurrently). The
/// integrity hash is recomputed on every save (P9a).
pub fn save(worktree: &Path, record: &ExecutionControlRecord) -> io::Result<()> {
    let mut record = record.clone();
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
    stamp_recovery_chain(&mut record.recoveries);
    let mut stored = recovery_storage_projection(&record);
    stored.content_hash = compute_content_hash(&record);
    let serialized = serde_json::to_vec_pretty(&stored)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    // P9b: trusted copy is authoritative; the mirror is informational.
    crate::cli::trusted_store::write_with_mirror(
        worktree,
        "execution-control.json",
        &state_path(worktree),
        &serialized,
    )
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
    })
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
    match settlement {
        ExecutionSettlement::Completed => {
            record.status = ExecutionControlStatus::Completed;
        }
        ExecutionSettlement::Blocked {
            reason,
            missing_verification,
        } => {
            record.status = ExecutionControlStatus::Blocked;
            record.blocked_reason = Some(reason);
            record.missing_verification = missing_verification;
        }
    }
    record.settled_at = Some(Utc::now());
    save(worktree, &record)?;
    Ok(SettleResult::Settled(record))
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
/// - No record, another session's record, and completed executions pass.
pub(crate) fn pr_handoff_refusal(repo_path: &Path, ready_handoff: bool) -> Option<String> {
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let worktree = gwt_core::paths::resolve_current_worktree_root(repo_path);
    let record = load(&worktree).ok().flatten()?;
    // P9a (T-122): a tampered record refuses every PR mutation for everyone.
    // The repair path depends on lifecycle status because adopt is Active-only.
    if !integrity_ok(&record) {
        return Some(format!(
            "PR handoff refused: the execution control record failed integrity validation (edited outside the canonical operations). {}",
            integrity_repair_guidance(record.status),
        ));
    }
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
    /// Return the current session's terminal Blocked record to Active only
    /// after fresh, derived, post-block verification evidence exists.
    Reopen {
        reason: String,
    },
}

/// Run an `execution.*` settlement command. Requires `GWT_SESSION_ID` so the
/// settlement binds to the session that owns the record.
pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: ExecutionCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SpecOpsError::from(ApiError::Unexpected(
                "execution settlement requires GWT_SESSION_ID to bind to the owning session"
                    .to_string(),
            ))
        })?;
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    if let ExecutionCommand::Adopt { reason } = &command {
        return run_adopt(&worktree, &session_id, reason, out);
    }
    if let ExecutionCommand::Reopen { reason } = &command {
        return run_reopen(&worktree, &session_id, reason, out);
    }
    if matches!(&command, ExecutionCommand::Complete) {
        if let Some(refusal) =
            crate::cli::verification_record::work_event_settlement_refusal(&worktree)
        {
            out.push_str(&format!("execution: completion refused — {refusal}\n"));
            return Ok(2);
        }
    }
    let result = match command {
        ExecutionCommand::Adopt { .. } | ExecutionCommand::Reopen { .. } => {
            unreachable!("handled above")
        }
        ExecutionCommand::Complete => {
            match settle_completed_with_evidence(&worktree, &session_id, None)
                .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?
            {
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
            settle(
                &worktree,
                &session_id,
                ExecutionSettlement::Blocked {
                    reason,
                    missing_verification,
                },
            )
            .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?
        }
    };
    match result {
        SettleResult::Settled(record) => {
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
            Ok(0)
        }
        SettleResult::NoRecord => {
            out.push_str(
                "execution: no execution control record for this worktree — nothing to settle\n",
            );
            Ok(0)
        }
        SettleResult::AlreadySettled(record) => {
            out.push_str(&format!(
                "execution: record already settled ({status:?}) for {kind} #{number}\n",
                status = record.status,
                kind = record.owner_kind.as_str(),
                number = record.owner_number,
            ));
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
fn run_reopen(
    worktree: &Path,
    session_id: &str,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if reason.trim().is_empty() {
        return Err(SpecOpsError::from(ApiError::Unexpected(
            "execution.reopen requires a non-empty params.reason".to_string(),
        )));
    }
    crate::cli::trusted_store::with_write_lease(worktree, || {
        Ok(run_reopen_locked(worktree, session_id, reason, out))
    })
    .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?
}

fn run_reopen_locked(
    worktree: &Path,
    session_id: &str,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let Some(mut record) =
        load(worktree).map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?
    else {
        out.push_str(
            "execution: reopen refused — no execution control record exists; start the linked owner through gwt-execute\n",
        );
        return Ok(2);
    };
    if record.content_hash.is_empty() || !integrity_ok(&record) {
        out.push_str(
            "execution: reopen refused — the execution control record has no valid integrity hash; use the canonical repair/fresh-launch path\n",
        );
        return Ok(2);
    }
    match record.status {
        ExecutionControlStatus::Active => {
            if record.primary_session_id != session_id {
                out.push_str(&format!(
                    "execution: reopen refused — the Active record belongs to session {owner}, not the current session {current}; use the authorized ownership-transfer path\n",
                    owner = record.primary_session_id,
                    current = session_id,
                ));
                return Ok(2);
            }
            // Only an in-flight record with embedded recoveries needs the
            // rolling-upgrade write. A modern idempotent retry stays a true
            // no-op and cannot fail because of an unnecessary rewrite.
            if recovery_storage_needs_upgrade(worktree)
                .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?
            {
                save(worktree, &record)
                    .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
            }
            out.push_str(&format!(
                "execution: {kind} #{number} is already active for session {session}\n",
                kind = record.owner_kind.as_str(),
                number = record.owner_number,
                session = record.primary_session_id,
            ));
            return Ok(0);
        }
        ExecutionControlStatus::Completed => {
            out.push_str(&format!(
                "execution: reopen refused — Completed {kind} #{number} is immutable; use a fresh launch for new work\n",
                kind = record.owner_kind.as_str(),
                number = record.owner_number,
            ));
            return Ok(2);
        }
        ExecutionControlStatus::Blocked => {}
    }
    if record.primary_session_id != session_id {
        out.push_str(&format!(
            "execution: reopen refused — the Blocked record belongs to session {owner}, not the current session {current}; use a fresh launch or the authorized ownership-transfer path\n",
            owner = record.primary_session_id,
            current = session_id,
        ));
        return Ok(2);
    }
    let Some(blocked_at) = record.settled_at else {
        out.push_str(
            "execution: reopen refused — the Blocked record has no settled_at timestamp and cannot prove post-block evidence ordering\n",
        );
        return Ok(2);
    };
    if !record
        .blocked_reason
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        out.push_str(
            "execution: reopen refused — the Blocked record has no non-empty blocker reason and cannot be recovered canonically\n",
        );
        return Ok(2);
    }

    use crate::cli::verification_record as vr;
    let Some(plan) = vr::load_plan(worktree)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?
    else {
        out.push_str(
            "execution: reopen refused — no verification plan exists; run verify.plan with params.derive:true, then verify.run\n",
        );
        return Ok(2);
    };
    if plan.content_hash.is_empty()
        || !vr::plan_integrity_ok(&plan)
        || plan.session_id != session_id
        || plan.owner_number != Some(record.owner_number)
    {
        out.push_str(
            "execution: reopen refused — verification plan hash/integrity/session/owner does not match the Blocked execution\n",
        );
        return Ok(2);
    }
    if !plan.derived {
        out.push_str(
            "execution: reopen refused — recovery requires a derived verification plan; run verify.plan with params.derive:true, then verify.run\n",
        );
        return Ok(2);
    }
    if plan.created_at <= blocked_at {
        out.push_str(
            "execution: reopen refused — the derived verification plan must be registered after the block; rerun verify.plan with params.derive:true\n",
        );
        return Ok(2);
    }
    let Some(verification) =
        vr::load(worktree).map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?
    else {
        out.push_str(
            "execution: reopen refused — no verification run record exists; run verify.run\n",
        );
        return Ok(2);
    };
    if verification.content_hash.is_empty() || !vr::integrity_ok(&verification) {
        out.push_str(
            "execution: reopen refused — the verification run has no valid integrity hash; rerun verify.run\n",
        );
        return Ok(2);
    }
    let evidence_status = vr::evaluate_evidence_snapshot(
        worktree,
        session_id,
        Some(record.owner_number),
        Some(&plan),
        &verification,
    );
    if evidence_status != vr::EvidenceStatus::Fresh {
        out.push_str(&format!(
            "execution: reopen refused — {}\n",
            evidence_status.describe()
        ));
        return Ok(2);
    }
    if !verification.plan_derived {
        out.push_str(
            "execution: reopen refused — recovery requires a run bound to a derived verification plan; run verify.plan with params.derive:true, then verify.run\n",
        );
        return Ok(2);
    }
    let Some(verification_started_at) = verification.started_at else {
        out.push_str(
            "execution: reopen refused — the verification run has no trusted start timestamp; rerun verify.run after the block\n",
        );
        return Ok(2);
    };
    if verification_started_at <= blocked_at {
        out.push_str(
            "execution: reopen refused — verification must start after the block; rerun verify.run\n",
        );
        return Ok(2);
    }
    if verification.created_at <= blocked_at {
        out.push_str(
            "execution: reopen refused — verification evidence must be created after the block; rerun verify.run\n",
        );
        return Ok(2);
    }
    let current_fingerprint = vr::worktree_fingerprint(worktree);
    if current_fingerprint != verification.worktree_fingerprint {
        out.push_str(
            "execution: reopen refused — the worktree changed after verification; rerun verify.run on the final state\n",
        );
        return Ok(2);
    }

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
    save(worktree, &record)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
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
    // T-149: adoption is a read-modify-write cycle — leased.
    crate::cli::trusted_store::with_write_lease(worktree, || {
        Ok(run_adopt_locked(worktree, session_id, reason, out))
    })
    .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?
}

fn run_adopt_locked(
    worktree: &Path,
    session_id: &str,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let Some(mut record) =
        load(worktree).map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?
    else {
        out.push_str("execution: no execution control record to adopt — a linked-owner launch materializes one\n");
        return Ok(0);
    };
    if record.status != ExecutionControlStatus::Active {
        out.push_str(&format!(
            "execution: record is already settled ({status:?}) — nothing to adopt; a fresh launch takes over\n",
            status = record.status,
        ));
        return Ok(0);
    }
    if !integrity_ok(&record) {
        out.push_str(&format!(
            "execution: adopt refused — {}\n",
            integrity_repair_guidance(record.status)
        ));
        return Ok(2);
    }
    if record.primary_session_id == session_id {
        out.push_str("execution: the current session already owns this record\n");
        return Ok(0);
    }
    record.transfers.push(OwnershipTransfer {
        from_session_id: record.primary_session_id.clone(),
        to_session_id: session_id.to_string(),
        reason: reason.trim().to_string(),
        transferred_at: Utc::now(),
    });
    record.primary_session_id = session_id.to_string();
    save(worktree, &record)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
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
        let mut out = String::new();
        assert_eq!(
            run_adopt(dir.path(), "sess-2", "tamper repair", &mut out).unwrap(),
            2,
            "{out}"
        );
        assert!(out.contains("fresh linked-owner launch"), "{out}");
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
            let mut env = TestEnv::new(repo.to_path_buf());
            run_collect(&mut env, CliCommand::Execution(command))
        }

        fn settle_blocked(repo: &Path, session: &str) -> ExecutionControlRecord {
            save(repo, &active_record(session)).unwrap();
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

        fn save_covering_evidence(repo: &Path, session: &str, derived: bool) -> String {
            use crate::cli::verification_record as vr;
            vr::save_plan(
                repo,
                &vr::VerificationPlanRecord {
                    session_id: session.to_string(),
                    owner_number: Some(3248),
                    commands: vec!["git --version".to_string()],
                    derived,
                    worktree_fingerprint: String::new(),
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

            let blocked = settle_blocked(dir.path(), "sess-reopen");
            let blocked_at = blocked.settled_at.unwrap();
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

            let reopened = load(dir.path()).unwrap().unwrap();
            assert_eq!(reopened.status, ExecutionControlStatus::Active);
            assert_eq!(reopened.blocked_reason, None);
            assert_eq!(reopened.missing_verification, None);
            assert_eq!(reopened.settled_at, None);
            assert_eq!(reopened.recoveries.len(), 1);
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

            // Reopen does not claim completion, but the same fresh evidence
            // unlocks the normal Ready/complete gates.
            assert_eq!(pr_handoff_refusal(dir.path(), true), None);
            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 0, "{out}");
            let completed = load(dir.path()).unwrap().unwrap();
            assert_eq!(completed.status, ExecutionControlStatus::Completed);
            assert_eq!(completed.recoveries.len(), 1);
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
                    commands: vec!["git --version".to_string()],
                    derived: true,
                    worktree_fingerprint: String::new(),
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
                    commands: vec!["git --exec-path".to_string()],
                    derived: true,
                    worktree_fingerprint: String::new(),
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
        fn terminal_tamper_diagnostics_never_recommend_active_only_adopt() {
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
            assert!(
                refusal.contains("terminal record cannot be repaired"),
                "{refusal}"
            );
            assert!(refusal.contains("fresh linked-owner launch"), "{refusal}");
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
            assert!(out.contains("terminal record cannot be repaired"), "{out}");
            assert!(out.contains("fresh linked-owner launch"), "{out}");
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
                    commands: vec![failing_command.clone()],
                    derived: true,
                    worktree_fingerprint: String::new(),
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
                    commands: vec!["git --version".to_string(), "git --exec-path".to_string()],
                    derived: true,
                    worktree_fingerprint: String::new(),
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

            // Session B adopts with an audited reason.
            let mut out = String::new();
            run_adopt(dir.path(), "sess-b", "crash recovery", &mut out).unwrap();
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
                    commands: vec!["git --version".to_string()],
                    derived: false,
                    worktree_fingerprint: String::new(),
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
                    commands: vec!["git --version".to_string()],
                    derived: false,
                    worktree_fingerprint: String::new(),
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
                    commands: vec!["git --version".to_string()],
                    derived: false,
                    worktree_fingerprint: String::new(),
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
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-new");
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), &active_record("sess-old")).unwrap();

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
                    commands: vec!["git --version".to_string()],
                    derived: false,
                    worktree_fingerprint: String::new(),
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
