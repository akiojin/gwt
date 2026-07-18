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
//! - Saves are atomic file replacements but the read-modify-write cycle is
//!   not serialized across processes; P9's write lease (T-124/T-125) owns
//!   real serialization.

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
    let mut canonical = record.clone();
    canonical.content_hash = String::new();
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("{:x}", Sha256::digest(&bytes))
}

/// True when the stored integrity hash matches the content (or the record is
/// a legacy pre-P9a record without one).
#[must_use]
pub fn integrity_ok(record: &ExecutionControlRecord) -> bool {
    record.content_hash.is_empty() || record.content_hash == compute_content_hash(record)
}

/// Resolve the record path for a worktree.
#[must_use]
pub fn state_path(worktree: &Path) -> PathBuf {
    worktree.join(EXECUTION_CONTROL_STATE_RELATIVE)
}

/// Load the record. `Ok(None)` when missing; malformed JSON and I/O failures
/// propagate so hook readers can fail open while writers surface the error.
pub fn load(worktree: &Path) -> io::Result<Option<ExecutionControlRecord>> {
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
    let record = serde_json::from_str::<ExecutionControlRecord>(&contents)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    Ok(Some(record))
}

/// Persist the record atomically (hooks read this file concurrently). The
/// integrity hash is recomputed on every save (P9a).
pub fn save(worktree: &Path, record: &ExecutionControlRecord) -> io::Result<()> {
    let mut record = record.clone();
    record.content_hash = compute_content_hash(&record);
    let serialized = serde_json::to_vec_pretty(&record)
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
    match load(worktree) {
        Ok(Some(record)) if record.owner_number == expected_owner_number => {
            if let Err(error) = settle(worktree, session_id, ExecutionSettlement::Completed) {
                tracing::warn!(?error, "execution control settlement failed");
            }
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(?error, "execution control settlement load failed");
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
    // P9a (T-122): a tampered record refuses every PR mutation for everyone —
    // repair it through `execution.adopt` (rewrites the record canonically)
    // before any handoff.
    if !integrity_ok(&record) {
        return Some(
            "PR handoff refused: the execution control record failed integrity validation (edited outside the canonical operations). Repair it with JSON operation `execution.adopt` and a non-empty `params.reason`, then re-verify.".to_string(),
        );
    }
    if record.primary_session_id != session_id {
        return None;
    }
    match record.status {
        ExecutionControlStatus::Completed => None,
        ExecutionControlStatus::Blocked => Some(format!(
            "PR handoff refused: the execution for {kind} #{number} is terminally blocked ({reason}). A blocked execution cannot hand off a PR — resolve the blocker and relaunch, or leave the blocked report as the outcome.",
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
    /// session with an audited reason (crash recovery, window handoff,
    /// tamper repair).
    Adopt {
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
    let settlement = match command {
        ExecutionCommand::Adopt { .. } => unreachable!("handled above"),
        ExecutionCommand::Complete => {
            // SPEC-3248 P8b (T-111/FR-035/FR-036): completion requires fresh,
            // all-passing, tool-generated verification evidence for this
            // session and owner. Blocked exits stay available without
            // evidence — blocked is the honest path when verification cannot
            // run.
            if let Ok(Some(record)) = load(&worktree) {
                if record.status == ExecutionControlStatus::Active
                    && record.primary_session_id == session_id
                {
                    let status = crate::cli::verification_record::evaluate_evidence(
                        &worktree,
                        &session_id,
                        Some(record.owner_number),
                    );
                    if status != crate::cli::verification_record::EvidenceStatus::Fresh {
                        out.push_str(&format!(
                            "execution: completion refused — {}\n",
                            status.describe()
                        ));
                        return Ok(2);
                    }
                }
            }
            ExecutionSettlement::Completed
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
            ExecutionSettlement::Blocked {
                reason,
                missing_verification,
            }
        }
    };
    let result = settle(&worktree, &session_id, settlement)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
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
            out.push_str(&format!(
                "execution: settlement refused — record belongs to session {record_session_id}, not the current session. Take it over explicitly with JSON operation `execution.adopt` and a non-empty `params.reason` (T-117)\n",
            ));
            Ok(2)
        }
        SettleResult::Tampered => {
            out.push_str(
                "execution: settlement refused — the record failed integrity validation (edited outside the canonical operations). Repair with JSON operation `execution.adopt` and a non-empty `params.reason`, then re-verify\n",
            );
            Ok(2)
        }
    }
}

/// `execution.adopt` (P9a, T-117): take over the worktree's record for the
/// current session with an audited transfer entry. Also the repair path for
/// records that failed integrity validation — the rewrite goes through the
/// canonical writer, restoring a valid hash while keeping the audit trail.
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
    if record.primary_session_id == session_id && integrity_ok(&record) {
        out.push_str("execution: the current session already owns this record\n");
        return Ok(0);
    }
    let was_tampered = !integrity_ok(&record);
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
        "execution: adopted {kind} #{number} for session {session} ({transfers} transfer(s) on record{repaired})\n",
        kind = record.owner_kind.as_str(),
        number = record.owner_number,
        session = session_id,
        transfers = record.transfers.len(),
        repaired = if was_tampered {
            ", integrity repaired"
        } else {
            ""
        },
    ));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

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
    fn tampered_record_refuses_settlement_and_adopt_repairs() {
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

        // adopt (canonical writer) repairs the record with an audited entry.
        // Tamper a different field this time (editing the status back would
        // just restore the originally hashed content).
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("\"completed\"", "\"active\"")
            .replace("\"$gwt-execute\"", "\"$gwt-forged\"");
        fs::write(&path, edited).unwrap();
        let mut record = load(dir.path()).unwrap().unwrap();
        assert!(!integrity_ok(&record));
        record.transfers.push(OwnershipTransfer {
            from_session_id: record.primary_session_id.clone(),
            to_session_id: "sess-2".to_string(),
            reason: "tamper repair".to_string(),
            transferred_at: Utc::now(),
        });
        record.primary_session_id = "sess-2".to_string();
        save(dir.path(), &record).unwrap();
        let repaired = load(dir.path()).unwrap().unwrap();
        assert!(integrity_ok(&repaired));
        assert_eq!(repaired.transfers.len(), 1);
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
