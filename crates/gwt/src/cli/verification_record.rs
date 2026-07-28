//! Verification Run Records (SPEC-3248 P8b, T-110/T-111, FR-035/FR-036).
//!
//! Completion gates must consume **tool-generated** verification evidence,
//! not agent prose. The `verify.run` JSON operation makes gwtd itself the
//! trusted executor: it runs the given verification commands in the worktree,
//! captures each exit code, and writes a [`VerificationRunRecord`] bound to
//! the session, the linked owner (from the Execution Control Record when
//! present), and a content-level worktree fingerprint (HEAD + `git diff
//! HEAD` + untracked file contents, `.gwt/` bookkeeping excluded; a run
//! during which the worktree changed is self-invalidated). `execution.complete` and the PR handoff
//! operations then accept only a fresh, all-passing record for the same
//! session/owner/fingerprint — handwritten claims, stale runs, cross-session
//! records, and failing runs are rejected (FR-036).
//!
//! Scope notes (dependent follow-ups, phase contract T-263):
//! - The authoritative copies live in the repo-scoped trusted store (P9b);
//!   the worktree files are mirrors, direct edits are hook-blocked (P9a
//!   T-120), and integrity hashes validate whichever copy loads (P9a).
//!   Deriving the required matrix from changed surfaces (full T-130
//!   Coverage Map) is still open; `verify.plan` records the declared one.
//! - Non-git worktrees get a degenerate fingerprint (no freshness signal);
//!   gwt executions always run in git worktrees.

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use gwt_agent::session::ExecutionBindingIdentity;
use gwt_github::{client::ApiError, SpecOpsError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::CliEnv;
use crate::cli::execution_state;

/// Worktree-relative path of the latest verification run record's mirror
/// (the authoritative copy lives in the repo-scoped trusted store, P9b).
pub const VERIFICATION_RUN_STATE_RELATIVE: &str = ".gwt/skill-state/verification-run.json";

/// Cap on the per-command output tail echoed back through the envelope.
const OUTPUT_TAIL_LIMIT: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationCommandResult {
    pub command: String,
    pub exit_code: i32,
}

/// One tool-generated verification run (T-110).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationRunRecord {
    pub record_id: String,
    pub session_id: String,
    /// Linked owner number copied from the Execution Control Record at run
    /// time (`None` for unlinked worktrees).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_number: Option<u64>,
    /// Exact generation/binding/head that produced this evidence. `None`
    /// remains readable only for pre-generation legacy executions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_binding: Option<ExecutionBindingIdentity>,
    /// Worktree fingerprint at run time: HEAD + tracked changes (see
    /// [`worktree_fingerprint`]). Completion recomputes and compares.
    pub worktree_fingerprint: String,
    pub commands: Vec<VerificationCommandResult>,
    pub all_passed: bool,
    /// Timestamp captured before the verification commands start. Recovery
    /// requires this to be later than the terminal block, so a run that was
    /// merely committed after the block cannot be replayed as post-block
    /// evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// T-130-lite: whether this run covered every command of the registered
    /// verification plan (`verify.plan`) for the same session/owner.
    #[serde(default)]
    pub plan_covered: bool,
    /// Planned commands the run did not execute (diagnostic for the gates).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planned_missing: Vec<String>,
    /// Integrity hash of the exact verification-plan snapshot consumed by
    /// this run. Replacing the plan invalidates the run even when both plans
    /// happen to contain commands covered by the run.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub verification_plan_hash: String,
    /// Whether the exact plan snapshot was derived from changed surfaces.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plan_derived: bool,
    /// Integrity hash over the record content (SPEC-3248 P9a, T-119/T-122
    /// core): sha256 of the canonical serialization with this field emptied.
    /// Gates reject records whose stored hash does not match. Empty = legacy
    /// pre-P9a record, accepted for one release cycle (see execution_state).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
}

/// Compute the integrity hash for a record (content with the hash emptied).
#[must_use]
pub fn compute_content_hash(record: &VerificationRunRecord) -> String {
    let mut canonical = record.clone();
    canonical.content_hash = String::new();
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("{:x}", Sha256::digest(&bytes))
}

/// True when the stored integrity hash matches the content (or the record is
/// a legacy pre-P9a record without one).
#[must_use]
pub fn integrity_ok(record: &VerificationRunRecord) -> bool {
    record.content_hash.is_empty() || record.content_hash == compute_content_hash(record)
}

/// Worktree-relative path of the registered verification plan (SPEC-3248
/// T-130-lite).
pub const VERIFICATION_PLAN_STATE_RELATIVE: &str = ".gwt/skill-state/verification-plan.json";

/// The declared verification matrix (T-130-lite): registered through
/// `verify.plan` BEFORE running, so the planned-vs-ran divergence is
/// machine-visible. Automatic derivation from changed surfaces / acceptance
/// scenarios is the full T-130; here the plan is a first-class recorded
/// contract that `verify.run` must cover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationPlanRecord {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_number: Option<u64>,
    /// Exact generation/binding/head for which this plan is authoritative.
    /// Legacy plans remain readable with `None`, but cannot authorize a
    /// generation-aware execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_binding: Option<ExecutionBindingIdentity>,
    pub commands: Vec<String>,
    /// Full T-130: the matrix was derived from changed surfaces instead of
    /// hand-picked (`verify.plan` with `params.derive:true`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub derived: bool,
    /// T-131 core Coverage Map: the surface classification the derived
    /// matrix covers (e.g. `rust(gwt,gwt-core)`, `skills`, `docs(2)`).
    /// Integrity-hash covered, so the coverage rationale is auditable and
    /// tamper-evident; empty for hand-picked plans. Acceptance-scenario
    /// (AS/FR) binding is T-131 full.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<String>,
    /// Content-level worktree fingerprint at plan registration. Derived
    /// matrices are valid only for this exact change set; a later surface
    /// change requires deriving and registering a new plan.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub worktree_fingerprint: String,
    pub created_at: DateTime<Utc>,
    /// Integrity hash (P9a convention).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
}

/// Compute the integrity hash for a plan (content with the hash emptied).
#[must_use]
pub fn compute_plan_hash(plan: &VerificationPlanRecord) -> String {
    let mut canonical = plan.clone();
    canonical.content_hash = String::new();
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("{:x}", Sha256::digest(&bytes))
}

/// True when the plan's stored integrity hash matches (or is legacy-empty).
#[must_use]
pub fn plan_integrity_ok(plan: &VerificationPlanRecord) -> bool {
    plan.content_hash.is_empty() || plan.content_hash == compute_plan_hash(plan)
}

/// Resolve the plan path for a worktree.
#[must_use]
pub fn plan_state_path(worktree: &Path) -> PathBuf {
    worktree.join(VERIFICATION_PLAN_STATE_RELATIVE)
}

/// Load the registered plan. `Ok(None)` when missing.
pub fn load_plan(worktree: &Path) -> io::Result<Option<VerificationPlanRecord>> {
    // P9b: trusted copy authoritative; mirror-only plans are refused in
    // managed worktrees (see `load` — same forgery window otherwise).
    let contents = match crate::cli::trusted_store::read(worktree, "verification-plan.json")? {
        Some(contents) => contents,
        None if crate::cli::trusted_store::under_trusted_management(worktree) => return Ok(None),
        None => match fs::read_to_string(plan_state_path(worktree)) {
            Ok(contents) => contents,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        },
    };
    let plan = serde_json::from_str::<VerificationPlanRecord>(&contents)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    Ok(Some(plan))
}

/// Persist the plan atomically with a fresh integrity hash.
///
/// This low-level primitive remains available for internal evidence fixtures.
/// Caller-facing `verify.plan` dispatch goes through
/// `register_plan_for_caller`, which authenticates durable Session authority
/// before invoking the unleased writer.
pub fn save_plan(worktree: &Path, plan: &VerificationPlanRecord) -> io::Result<()> {
    crate::cli::trusted_store::with_write_lease(worktree, || {
        let mut plan = plan.clone();
        let (_, execution_binding) = current_execution_context(worktree)?;
        plan.execution_binding = execution_binding;
        if plan.worktree_fingerprint.is_empty() {
            plan.worktree_fingerprint = worktree_fingerprint(worktree);
        }
        save_plan_unleased(worktree, &plan)
    })
}

fn save_plan_unleased(worktree: &Path, plan: &VerificationPlanRecord) -> io::Result<()> {
    let mut plan = plan.clone();
    plan.content_hash = compute_plan_hash(&plan);
    let serialized = serde_json::to_vec_pretty(&plan)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    crate::cli::trusted_store::write_with_mirror(
        worktree,
        "verification-plan.json",
        &plan_state_path(worktree),
        &serialized,
    )
}

fn register_plan_for_caller(
    worktree: &Path,
    session_id: &str,
    commands: Vec<String>,
    derived: bool,
    authority: &VerificationCallerAuthority,
) -> io::Result<VerificationPlanRecord> {
    crate::cli::trusted_store::with_write_lease(worktree, || {
        let fingerprint = worktree_fingerprint(worktree);
        revalidate_verification_caller_authority(worktree, session_id, authority)?;
        register_plan_with_context_unleased(
            worktree,
            session_id,
            commands,
            Vec::new(),
            derived,
            fingerprint,
            authority.owner_number,
            authority.execution_binding.clone(),
        )
    })
}

fn register_plan_with_context_unleased(
    worktree: &Path,
    session_id: &str,
    commands: Vec<String>,
    surfaces: Vec<String>,
    derived: bool,
    worktree_fingerprint: String,
    owner_number: Option<u64>,
    execution_binding: Option<ExecutionBindingIdentity>,
) -> io::Result<VerificationPlanRecord> {
    let mut plan = VerificationPlanRecord {
        session_id: session_id.to_string(),
        owner_number,
        execution_binding,
        commands,
        derived,
        surfaces,
        worktree_fingerprint,
        created_at: Utc::now(),
        content_hash: String::new(),
    };
    plan.content_hash = compute_plan_hash(&plan);
    save_plan_unleased(worktree, &plan)?;
    Ok(plan)
}

fn derive_and_register_plan_for_caller(
    worktree: &Path,
    session_id: &str,
    authority: &VerificationCallerAuthority,
) -> Result<
    (
        crate::cli::verify_derivation::DerivedPlan,
        VerificationPlanRecord,
    ),
    String,
> {
    crate::cli::trusted_store::with_write_lease(worktree, || {
        let fingerprint_before = worktree_fingerprint(worktree);
        let derived = crate::cli::verify_derivation::derive(worktree)
            .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
        let fingerprint_after = worktree_fingerprint(worktree);
        if fingerprint_before != fingerprint_after {
            return Err(io::Error::new(
                ErrorKind::WouldBlock,
                "the worktree changed while deriving the verification plan — retry verify.plan on a stable change set",
            ));
        }
        revalidate_verification_caller_authority(worktree, session_id, authority)?;
        let plan = register_plan_with_context_unleased(
            worktree,
            session_id,
            derived.commands.clone(),
            derived.surfaces.clone(),
            true,
            fingerprint_after,
            authority.owner_number,
            authority.execution_binding.clone(),
        )?;
        Ok((derived, plan))
    })
    .map_err(|err| err.to_string())
}

/// Resolve the record path for a worktree.
#[must_use]
pub fn state_path(worktree: &Path) -> PathBuf {
    worktree.join(VERIFICATION_RUN_STATE_RELATIVE)
}

/// Load the latest record. `Ok(None)` when missing; malformed JSON and I/O
/// failures propagate.
pub fn load(worktree: &Path) -> io::Result<Option<VerificationRunRecord>> {
    // P9b: trusted copy authoritative. The mirror is a legacy fallback for
    // unmanaged worktrees only — under trusted management every canonical
    // `verify.run` wrote a trusted copy, so a mirror-only record is not
    // evidence (T-174).
    let contents = match crate::cli::trusted_store::read(worktree, "verification-run.json")? {
        Some(contents) => contents,
        None if crate::cli::trusted_store::under_trusted_management(worktree) => return Ok(None),
        None => match fs::read_to_string(state_path(worktree)) {
            Ok(contents) => contents,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        },
    };
    let record = serde_json::from_str::<VerificationRunRecord>(&contents)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    Ok(Some(record))
}

/// Persist the record atomically. The integrity hash is recomputed on every
/// save (P9a).
pub fn save(worktree: &Path, record: &VerificationRunRecord) -> io::Result<()> {
    let mut record = record.clone();
    record.content_hash = compute_content_hash(&record);
    let serialized = serde_json::to_vec_pretty(&record)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    crate::cli::trusted_store::write_with_mirror(
        worktree,
        "verification-run.json",
        &state_path(worktree),
        &serialized,
    )
}

/// Compute the worktree fingerprint at **content level**: sha256 over
/// `git rev-parse HEAD`, the full `git diff HEAD` content (staged and
/// unstaged tracked changes), and every untracked file's path and bytes —
/// all with `.gwt/` excluded (the coordination bookkeeping under `.gwt/`
/// changes continuously and must not invalidate evidence). Status lines
/// alone are not enough: an edit to an already-modified file leaves
/// `git status --porcelain` byte-identical, which would let stale evidence
/// pass as fresh (FR-036). Non-git worktrees return a degenerate constant.
#[must_use]
pub fn worktree_fingerprint(worktree: &Path) -> String {
    let head = gwt_core::process::hidden_command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(worktree)
        .output();
    let Ok(head) = head else {
        return "no-git".to_string();
    };
    if !head.status.success() {
        return "no-git".to_string();
    }
    let mut hasher = Sha256::new();
    hasher.update(&head.stdout);
    hasher.update(b"\n--diff--\n");
    let diff = gwt_core::process::hidden_command("git")
        .args(["diff", "HEAD", "--", ".", ":(exclude).gwt"])
        .current_dir(worktree)
        .output();
    if let Ok(diff) = diff {
        if diff.status.success() {
            hasher.update(&diff.stdout);
        }
    }
    hasher.update(b"\n--untracked--\n");
    // `-uall` expands untracked directories to individual files so new files
    // inside an already-untracked directory change the fingerprint too.
    let untracked = gwt_core::process::hidden_command("git")
        .args([
            "status",
            "--porcelain",
            "-uall",
            "--",
            ".",
            ":(exclude).gwt",
        ])
        .current_dir(worktree)
        .output();
    if let Ok(untracked) = untracked {
        if untracked.status.success() {
            let listing = String::from_utf8_lossy(&untracked.stdout);
            for line in listing.lines() {
                let Some(path) = line.strip_prefix("?? ") else {
                    continue;
                };
                let path = path.trim().trim_matches('"');
                hasher.update(path.as_bytes());
                hasher.update(b"\n");
                if let Ok(bytes) = fs::read(worktree.join(path)) {
                    hasher.update(&bytes);
                }
                hasher.update(b"\n");
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

/// Exact tracked path whose delivery must settle before terminal mutations.
pub const WORK_EVENT_LOG_RELATIVE: &str = ".gwt/work/events.jsonl";

const WORK_EVENT_SETTLEMENT_RECORD_FILE: &str = "work-event-settlement.json";
const WORK_EVENT_SETTLEMENT_LEGACY_SCHEMA_VERSION: u64 = 1;
const WORK_EVENT_SETTLEMENT_PRE_GENERATION_SCHEMA_VERSION: u64 = 2;
const WORK_EVENT_SETTLEMENT_SCHEMA_VERSION: u64 = 3;

/// Independent dirty states reported for the exact tracked Work event path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkEventPathState {
    Staged,
    Unstaged,
    Untracked,
    Deleted,
}

/// Fail-closed reasons for Work event delivery settlement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkEventSettlementBlocker {
    PathDirty { states: Vec<WorkEventPathState> },
    CommitNotPushed,
    MissingUpstream,
    RemoteDiverged,
    GitStatusError,
    RemoteReadbackError,
    InvalidWorkOnlyCommit,
}

/// Current delivery status of the exact tracked Work event log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkEventSettlementStatus {
    /// Write-ahead reservation created before the corresponding terminal
    /// event can mutate any tracked or projected Work surface.
    PendingMutation {
        event_id: String,
        work_id: String,
        journal_entry_id: String,
        event_session_id: String,
    },
    Settled {
        event_commit: String,
        upstream_ref: String,
    },
    Blocked(WorkEventSettlementBlocker),
}

impl WorkEventSettlementStatus {
    #[must_use]
    pub fn is_settled(&self) -> bool {
        matches!(self, Self::Settled { .. })
    }
}

/// Machine-local record kept separately from verification evidence.
///
/// `obligation_open` is sticky while settlement is blocked. Calling
/// [`save_work_event_settlement_record`] with `open_obligation = false` only
/// refreshes the status; it cannot close an existing obligation until the
/// evaluator observes a clean event path and remotely contained HEAD. The originating
/// `session_id` is retained through that settled record for auditability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkEventSettlementRecord {
    pub schema_version: u64,
    pub session_id: String,
    /// Exact generation that owns this delivery obligation/receipt. Old
    /// records deserialize as `None` and are audit-only once a ledger exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_binding: Option<ExecutionBindingIdentity>,
    pub obligation_open: bool,
    pub status: WorkEventSettlementStatus,
    pub updated_at: DateTime<Utc>,
}

/// Load the machine-local Work event settlement record. No worktree mirror
/// is consulted: this state is an execution obligation, not tracked source.
pub fn load_work_event_settlement_record(
    worktree: &Path,
) -> io::Result<Option<WorkEventSettlementRecord>> {
    let contents = crate::cli::trusted_store::read(worktree, WORK_EVENT_SETTLEMENT_RECORD_FILE)?;
    decode_work_event_settlement_record(contents)
}

fn load_work_event_settlement_record_from_resolved_dir(
    trusted_dir: &Path,
) -> io::Result<Option<WorkEventSettlementRecord>> {
    let contents = crate::cli::trusted_store::read_from_resolved_dir(
        trusted_dir,
        WORK_EVENT_SETTLEMENT_RECORD_FILE,
    )?;
    decode_work_event_settlement_record(contents)
}

fn decode_work_event_settlement_record(
    contents: Option<String>,
) -> io::Result<Option<WorkEventSettlementRecord>> {
    let Some(contents) = contents else {
        return Ok(None);
    };
    let record = serde_json::from_str::<WorkEventSettlementRecord>(&contents)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    if !matches!(
        record.schema_version,
        WORK_EVENT_SETTLEMENT_LEGACY_SCHEMA_VERSION
            | WORK_EVENT_SETTLEMENT_PRE_GENERATION_SCHEMA_VERSION
            | WORK_EVENT_SETTLEMENT_SCHEMA_VERSION
    ) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "unsupported Work event settlement schema version: {}",
                record.schema_version
            ),
        ));
    }
    if record.schema_version == WORK_EVENT_SETTLEMENT_LEGACY_SCHEMA_VERSION
        && matches!(
            record.status,
            WorkEventSettlementStatus::PendingMutation { .. }
        )
    {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "legacy Work event settlement record cannot contain a pending mutation",
        ));
    }
    Ok(Some(record))
}

fn required_work_event_settlement_trusted_dir(worktree: &Path) -> io::Result<PathBuf> {
    crate::cli::trusted_store::trusted_dir_for_worktree(worktree).ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            "repo-scoped trusted store is unavailable for Work event settlement",
        )
    })
}

fn require_unchanged_work_event_settlement_trusted_dir(
    worktree: &Path,
    expected: &Path,
) -> io::Result<()> {
    let current = crate::cli::trusted_store::trusted_dir_for_worktree(worktree);
    if current.as_deref() == Some(expected) {
        return Ok(());
    }
    Err(io::Error::new(
        ErrorKind::PermissionDenied,
        "trusted store identity changed during Work event settlement; retry after repository identity stabilizes",
    ))
}

/// Atomically reserve a terminal Work mutation before any tracked event or
/// projection is written. This intentionally does not inspect Git: a clean,
/// pushed worktree must still retain the obligation until the exact event
/// generated by the locked transaction becomes observable.
pub fn prepare_work_event_settlement_record(
    worktree: &Path,
    session_id: &str,
    event: &gwt_core::workspace_projection::WorkEvent,
    journal_entry: &gwt_core::workspace_projection::WorkspaceJournalEntry,
) -> io::Result<WorkEventSettlementRecord> {
    let trusted_dir = required_work_event_settlement_trusted_dir(worktree)?;
    if event.kind != gwt_core::workspace_projection::WorkEventKind::Done
        || event.id.trim().is_empty()
        || event.work_item_id.trim().is_empty()
        || event.agent_session_id.as_deref() != Some(session_id)
        || journal_entry.id.trim().is_empty()
        || journal_entry.agent_session_id.as_deref() != Some(session_id)
        || journal_entry.status_category
            != Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done)
    {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Work event settlement preparation requires a matching Session-bound Done event and journal entry",
        ));
    }

    crate::cli::trusted_store::with_write_lease_for_resolved_dir(&trusted_dir, || {
        require_unchanged_work_event_settlement_trusted_dir(worktree, &trusted_dir)?;
        let current_binding = current_execution_context(worktree)?.1;
        let (record_session_id, execution_binding) =
            match load_work_event_settlement_record_from_resolved_dir(&trusted_dir)?
                .filter(|record| record.obligation_open)
            {
                Some(record) => (record.session_id, record.execution_binding),
                None => (session_id.to_string(), current_binding),
            };
        let record = WorkEventSettlementRecord {
            schema_version: WORK_EVENT_SETTLEMENT_SCHEMA_VERSION,
            session_id: record_session_id,
            execution_binding,
            obligation_open: true,
            status: WorkEventSettlementStatus::PendingMutation {
                event_id: event.id.clone(),
                work_id: event.work_item_id.clone(),
                journal_entry_id: journal_entry.id.clone(),
                event_session_id: session_id.to_string(),
            },
            updated_at: Utc::now(),
        };
        require_unchanged_work_event_settlement_trusted_dir(worktree, &trusted_dir)?;
        persist_work_event_settlement_record_to_resolved_dir(&trusted_dir, &record)?;
        Ok(record)
    })
}

/// Evaluate and atomically persist the machine-local settlement status.
/// Setting `open_obligation` records a new terminal-update obligation. A
/// previous open obligation remains open across refreshes until the exact
/// event path is clean and HEAD is confirmed on the configured upstream
/// remote. Refreshes and duplicate opens do not replace the originating
/// session provenance.
pub fn save_work_event_settlement_record(
    worktree: &Path,
    session_id: &str,
    open_obligation: bool,
) -> io::Result<WorkEventSettlementRecord> {
    let trusted_dir = required_work_event_settlement_trusted_dir(worktree)?;
    crate::cli::trusted_store::with_write_lease_for_resolved_dir(&trusted_dir, || {
        require_unchanged_work_event_settlement_trusted_dir(worktree, &trusted_dir)?;
        let previous = load_work_event_settlement_record_from_resolved_dir(&trusted_dir)?;
        if let Some(record) = previous.as_ref().filter(|record| record.obligation_open) {
            if let WorkEventSettlementStatus::PendingMutation {
                event_id,
                work_id,
                event_session_id,
                ..
            } = &record.status
            {
                match pending_work_event_is_persisted(worktree, event_id, work_id, event_session_id)
                {
                    Ok(true) => {}
                    Ok(false) => return Ok(record.clone()),
                    Err(error) => return Err(error),
                }
            }
        }
        let status = evaluate_work_event_settlement(worktree);
        let current_binding = current_execution_context(worktree)?.1;
        let (previous_open, record_session_id, execution_binding) = match previous {
            Some(record) if record.obligation_open || !open_obligation => (
                record.obligation_open,
                record.session_id,
                record.execution_binding,
            ),
            _ => (false, session_id.to_string(), current_binding),
        };
        let record = WorkEventSettlementRecord {
            schema_version: WORK_EVENT_SETTLEMENT_SCHEMA_VERSION,
            session_id: record_session_id,
            execution_binding,
            obligation_open: !status.is_settled() && (open_obligation || previous_open),
            status: status.clone(),
            updated_at: Utc::now(),
        };
        require_unchanged_work_event_settlement_trusted_dir(worktree, &trusted_dir)?;
        persist_work_event_settlement_record_to_resolved_dir(&trusted_dir, &record)?;
        Ok(record)
    })
}

#[cfg(test)]
fn persist_work_event_settlement_record(
    worktree: &Path,
    record: &WorkEventSettlementRecord,
) -> io::Result<()> {
    let trusted_dir = required_work_event_settlement_trusted_dir(worktree)?;
    persist_work_event_settlement_record_to_resolved_dir(&trusted_dir, record)
}

fn persist_work_event_settlement_record_to_resolved_dir(
    trusted_dir: &Path,
    record: &WorkEventSettlementRecord,
) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(record)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    crate::cli::trusted_store::write_to_resolved_dir(
        trusted_dir,
        WORK_EVENT_SETTLEMENT_RECORD_FILE,
        &bytes,
    )
}

fn pending_work_event_is_persisted(
    worktree: &Path,
    event_id: &str,
    work_id: &str,
    session_id: &str,
) -> io::Result<bool> {
    let contents = match fs::read_to_string(worktree.join(WORK_EVENT_LOG_RELATIVE)) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !contents.is_empty() && !contents.ends_with('\n') {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "tracked Work event log has an incomplete tail",
        ));
    }
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            io::Error::new(
                ErrorKind::InvalidData,
                format!("invalid tracked Work event at line {}: {error}", index + 1),
            )
        })?;
        if value.get("id").and_then(serde_json::Value::as_str) != Some(event_id) {
            continue;
        }
        let event = serde_json::from_value::<gwt_core::workspace_projection::WorkEvent>(value)
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
        if event.work_item_id != work_id
            || event.kind != gwt_core::workspace_projection::WorkEventKind::Done
            || event.agent_session_id.as_deref() != Some(session_id)
        {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "prepared Work event identity does not match the tracked event",
            ));
        }
        return Ok(true);
    }
    Ok(false)
}

/// Evaluate Work event delivery independently from verification freshness.
/// Every Git error and unknown state is blocked; this function never treats
/// a failed probe as evidence of settlement.
#[must_use]
pub fn evaluate_work_event_settlement(worktree: &Path) -> WorkEventSettlementStatus {
    let states = match work_event_path_states(worktree) {
        Ok(states) => states,
        Err(()) => {
            return WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::GitStatusError);
        }
    };
    if !states.is_empty() {
        return WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::PathDirty {
            states,
        });
    }

    let head_commit = match git_stdout(worktree, &["rev-parse", "HEAD"]) {
        Ok(commit) if !commit.is_empty() => commit,
        _ => {
            return WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::GitStatusError);
        }
    };
    let event_commit = match git_stdout(
        worktree,
        &[
            "rev-list",
            "-1",
            &head_commit,
            "--",
            WORK_EVENT_LOG_RELATIVE,
        ],
    ) {
        Ok(commit) if !commit.is_empty() => commit,
        _ => {
            return WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::GitStatusError);
        }
    };
    match event_commit_has_non_bookkeeping_change(worktree, &event_commit) {
        Ok(true) => {}
        Ok(false) => {
            let subject = match git_stdout(worktree, &["show", "-s", "--format=%s", &event_commit])
            {
                Ok(subject) => subject,
                Err(()) => {
                    return WorkEventSettlementStatus::Blocked(
                        WorkEventSettlementBlocker::GitStatusError,
                    );
                }
            };
            if !subject.starts_with("chore(work):") {
                return WorkEventSettlementStatus::Blocked(
                    WorkEventSettlementBlocker::InvalidWorkOnlyCommit,
                );
            }
        }
        Err(()) => {
            return WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::GitStatusError);
        }
    }

    let (remote, merge_ref, upstream_ref) = match configured_upstream(worktree) {
        Ok(upstream) => upstream,
        Err(UpstreamFailure::Missing) => {
            return WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::MissingUpstream);
        }
        Err(UpstreamFailure::Git) => {
            return WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::GitStatusError);
        }
    };
    let remote_tip = match fetch_upstream_tip(worktree, &remote, &merge_ref) {
        Ok(remote_tip) => remote_tip,
        Err(()) => {
            return WorkEventSettlementStatus::Blocked(
                WorkEventSettlementBlocker::RemoteReadbackError,
            );
        }
    };
    let head_on_remote = match git_is_ancestor(worktree, &head_commit, &remote_tip) {
        Ok(value) => value,
        Err(()) => {
            return WorkEventSettlementStatus::Blocked(
                WorkEventSettlementBlocker::RemoteReadbackError,
            );
        }
    };
    if head_on_remote {
        return WorkEventSettlementStatus::Settled {
            event_commit,
            upstream_ref,
        };
    }
    let remote_on_head = match git_is_ancestor(worktree, &remote_tip, &head_commit) {
        Ok(value) => value,
        Err(()) => {
            return WorkEventSettlementStatus::Blocked(
                WorkEventSettlementBlocker::RemoteReadbackError,
            );
        }
    };
    if remote_on_head {
        WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::CommitNotPushed)
    } else {
        WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::RemoteDiverged)
    }
}

/// Return an actionable refusal when this worktree has entered the tracked
/// Work-event lifecycle and its exact event log is not durably contained by
/// the configured upstream. Repositories that have never materialized the
/// event log (and have no trusted settlement receipt) remain outside this
/// contract so legacy/unmanaged command surfaces continue to work.
#[must_use]
pub fn work_event_settlement_refusal(worktree: &Path) -> Option<String> {
    let receipt = match load_work_event_settlement_record(worktree) {
        Ok(record) => record,
        Err(error) => {
            return Some(format!(
                "Work event settlement refused: the trusted settlement receipt is unreadable ({error}). Repair the trusted store, then commit `{WORK_EVENT_LOG_RELATIVE}` and push HEAD to its configured upstream before retrying."
            ));
        }
    };
    let current_binding = match current_execution_context(worktree) {
        Ok((_, binding)) => binding,
        Err(error) => {
            return Some(format!(
                "Work event settlement refused: current execution generation authority is unreadable ({error}). Repair or relaunch the execution before retrying."
            ));
        }
    };
    let path_exists = match fs::symlink_metadata(worktree.join(WORK_EVENT_LOG_RELATIVE)) {
        Ok(_) => true,
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => {
            return Some(format!(
                "Work event settlement refused: `{WORK_EVENT_LOG_RELATIVE}` could not be inspected ({error}). Repair the path, commit it, and push HEAD to its configured upstream before retrying."
            ));
        }
    };
    let path_tracked = if path_exists {
        false
    } else {
        gwt_core::process::hidden_command("git")
            .args(["ls-files", "--error-unmatch", "--", WORK_EVENT_LOG_RELATIVE])
            .current_dir(worktree)
            .output()
            .is_ok_and(|output| output.status.success())
    };
    if receipt.is_none() && !path_exists && !path_tracked {
        return None;
    }
    match receipt.as_ref() {
        Some(record) if record.execution_binding != current_binding => {
            return Some(
                "Work event settlement refused: this receipt belongs to a legacy or predecessor execution generation. Settle and push the current generation's own Work event before retrying."
                    .to_string(),
            );
        }
        None if current_binding.is_some() => {
            return Some(
                "Work event settlement refused: the current execution generation has no generation-scoped Work event receipt. Complete its terminal Work update, commit it, and push it before retrying."
                    .to_string(),
            );
        }
        _ => {}
    }

    let status = if let Some(receipt) = receipt {
        match save_work_event_settlement_record(worktree, &receipt.session_id, false) {
            Ok(refreshed) => refreshed.status,
            Err(error) => {
                return Some(format!(
                    "Work event settlement refused: the trusted settlement receipt could not be refreshed ({error}). Repair the trusted store, then commit `{WORK_EVENT_LOG_RELATIVE}` and push HEAD to its configured upstream before retrying."
                ));
            }
        }
    } else {
        evaluate_work_event_settlement(worktree)
    };
    match status {
        WorkEventSettlementStatus::PendingMutation {
            event_id,
            work_id,
            journal_entry_id,
            ..
        } => Some(work_event_settlement_pending_description(
            &event_id,
            &work_id,
            &journal_entry_id,
        )),
        WorkEventSettlementStatus::Settled { .. } => None,
        WorkEventSettlementStatus::Blocked(blocker) => {
            Some(work_event_settlement_blocker_description(&blocker))
        }
    }
}

pub(crate) fn work_event_settlement_pending_description(
    event_id: &str,
    work_id: &str,
    journal_entry_id: &str,
) -> String {
    format!(
        "Work event settlement refused: terminal event `{event_id}` for Work `{work_id}` (journal `{journal_entry_id}`) has not been persisted to `{WORK_EVENT_LOG_RELATIVE}`. Retry the terminal workspace.update so Host recovery can finish, then commit and push the event log before retrying."
    )
}

pub(crate) fn work_event_settlement_blocker_description(
    blocker: &WorkEventSettlementBlocker,
) -> String {
    let reason = match blocker {
        WorkEventSettlementBlocker::PathDirty { states } => {
            let states = states
                .iter()
                .map(|state| match state {
                    WorkEventPathState::Staged => "staged",
                    WorkEventPathState::Unstaged => "unstaged",
                    WorkEventPathState::Untracked => "untracked",
                    WorkEventPathState::Deleted => "deleted",
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("`{WORK_EVENT_LOG_RELATIVE}` is dirty ({states})")
        }
        WorkEventSettlementBlocker::CommitNotPushed => {
            "the current HEAD is not contained by its configured upstream".to_string()
        }
        WorkEventSettlementBlocker::MissingUpstream => {
            "the current branch has no usable configured upstream".to_string()
        }
        WorkEventSettlementBlocker::RemoteDiverged => {
            "the current HEAD and configured upstream have diverged".to_string()
        }
        WorkEventSettlementBlocker::GitStatusError => {
            format!("Git could not prove the state of `{WORK_EVENT_LOG_RELATIVE}`")
        }
        WorkEventSettlementBlocker::RemoteReadbackError => {
            "the configured upstream could not be fetched and read back".to_string()
        }
        WorkEventSettlementBlocker::InvalidWorkOnlyCommit =>
            "the commit containing only `.gwt/` bookkeeping does not use the exact `chore(work):` subject prefix"
                .to_string(),
    };
    format!(
        "Work event settlement refused: {reason}. Commit `{WORK_EVENT_LOG_RELATIVE}` with the related source changes (or use the exact `chore(work):` prefix for a bookkeeping-only commit), push HEAD to its configured upstream, and retry."
    )
}

fn work_event_path_states(worktree: &Path) -> Result<Vec<WorkEventPathState>, ()> {
    let output = gwt_core::process::hidden_command("git")
        .args([
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            WORK_EVENT_LOG_RELATIVE,
        ])
        .current_dir(worktree)
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    let stdout = String::from_utf8(output.stdout).map_err(|_| ())?;
    let mut states = Vec::new();
    for line in stdout.lines() {
        let bytes = line.as_bytes();
        if bytes.len() < 3 {
            return Err(());
        }
        let (index, worktree_state) = (bytes[0], bytes[1]);
        if index == b'?' && worktree_state == b'?' {
            states.push(WorkEventPathState::Untracked);
            continue;
        }
        if index == b'D' || worktree_state == b'D' {
            states.push(WorkEventPathState::Deleted);
        }
        if index != b' ' && index != b'D' {
            states.push(WorkEventPathState::Staged);
        }
        if worktree_state != b' ' && worktree_state != b'D' {
            states.push(WorkEventPathState::Unstaged);
        }
    }
    states.sort_unstable();
    states.dedup();
    Ok(states)
}

fn event_commit_has_non_bookkeeping_change(
    worktree: &Path,
    event_commit: &str,
) -> Result<bool, ()> {
    let output = gwt_core::process::hidden_command("git")
        .args([
            "show",
            "--root",
            "--first-parent",
            "--format=",
            "--name-only",
            "-r",
            "-z",
            event_commit,
        ])
        .current_dir(worktree)
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    let mut saw_event_path = false;
    let mut saw_non_bookkeeping = false;
    for path in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        if path == WORK_EVENT_LOG_RELATIVE.as_bytes() {
            saw_event_path = true;
        } else if !path.starts_with(b".gwt/") {
            saw_non_bookkeeping = true;
        }
    }
    if !saw_event_path {
        return Err(());
    }
    Ok(saw_non_bookkeeping)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamFailure {
    Missing,
    Git,
}

fn configured_upstream(worktree: &Path) -> Result<(String, String, String), UpstreamFailure> {
    let output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(worktree)
        .output()
        .map_err(|_| UpstreamFailure::Git)?;
    if !output.status.success() {
        return match output.status.code() {
            Some(1) => Err(UpstreamFailure::Missing),
            _ => Err(UpstreamFailure::Git),
        };
    }
    let branch = String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| UpstreamFailure::Git)?;
    if branch.is_empty() {
        return Err(UpstreamFailure::Missing);
    }
    let remote_key = format!("branch.{branch}.remote");
    let merge_key = format!("branch.{branch}.merge");
    let remote = git_config_value(worktree, &remote_key)?;
    let merge_ref = git_config_value(worktree, &merge_key)?;
    if remote.is_empty() || remote == "." || !merge_ref.starts_with("refs/heads/") {
        return Err(UpstreamFailure::Missing);
    }
    let branch_name = merge_ref
        .strip_prefix("refs/heads/")
        .ok_or(UpstreamFailure::Missing)?;
    let upstream_ref = format!("refs/remotes/{remote}/{branch_name}");
    Ok((remote, merge_ref, upstream_ref))
}

fn git_config_value(worktree: &Path, key: &str) -> Result<String, UpstreamFailure> {
    let output = gwt_core::process::hidden_command("git")
        .args(["config", "--get", key])
        .current_dir(worktree)
        .output()
        .map_err(|_| UpstreamFailure::Git)?;
    if !output.status.success() {
        return match output.status.code() {
            Some(1) => Err(UpstreamFailure::Missing),
            _ => Err(UpstreamFailure::Git),
        };
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| UpstreamFailure::Git)
}

fn fetch_upstream_tip(worktree: &Path, remote: &str, merge_ref: &str) -> Result<String, ()> {
    let output = gwt_core::process::hidden_command("git")
        .args(["fetch", "--quiet", "--no-tags", remote, merge_ref])
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(worktree)
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    git_stdout(worktree, &["rev-parse", "FETCH_HEAD"])
}

fn git_stdout(worktree: &Path, args: &[&str]) -> Result<String, ()> {
    let output = gwt_core::process::hidden_command("git")
        .args(args)
        .current_dir(worktree)
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| ())
}

fn git_is_ancestor(worktree: &Path, ancestor: &str, descendant: &str) -> Result<bool, ()> {
    let output = gwt_core::process::hidden_command("git")
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .current_dir(worktree)
        .output()
        .map_err(|_| ())?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(()),
    }
}

/// Minimal quote-aware command splitter: whitespace-separated arguments with
/// double- and single-quote grouping. Deliberately supports no shell
/// features (pipes, redirects, `&&`) — verification commands run as direct
/// process invocations so the recorded command is exactly what executed.
pub fn split_command_line(command: &str) -> Result<Vec<String>, String> {
    let mut args: Vec<(String, bool)> = Vec::new();
    let mut current = String::new();
    let mut current_quoted = false;
    let mut in_single = false;
    let mut in_double = false;
    let mut had_token = false;
    for ch in command.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                had_token = true;
                current_quoted = true;
            }
            '"' if !in_single => {
                in_double = !in_double;
                had_token = true;
                current_quoted = true;
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if had_token {
                    args.push((std::mem::take(&mut current), current_quoted));
                    had_token = false;
                    current_quoted = false;
                }
            }
            c => {
                current.push(c);
                had_token = true;
            }
        }
    }
    if in_single || in_double {
        return Err(format!("unbalanced quote in command: {command}"));
    }
    if had_token {
        args.push((current, current_quoted));
    }
    if args.is_empty() {
        return Err("empty command".to_string());
    }
    // Reject bare shell operators (there is no shell, so they would become
    // literal arguments and silently change the command's meaning). Quoted
    // occurrences are deliberate literals — `rg "&&" crates/` is fine.
    for meta in ["&&", "||", "|", ";", ">", "<"] {
        if args.iter().any(|(arg, quoted)| !quoted && arg == meta) {
            return Err(format!(
                "shell operator '{meta}' is not supported — run one plain command per entry"
            ));
        }
    }
    Ok(args.into_iter().map(|(arg, _)| arg).collect())
}

/// Execute one verification command in the worktree and return its exit code
/// plus a bounded output tail (stdout + stderr interleaved by section). A
/// spawn failure (missing binary, Windows `.cmd` shims that
/// `std::process::Command` cannot launch, …) is recorded as a failed result
/// (exit `-1`) instead of aborting the run — the record must be written even
/// when commands fail, and the partial transcript must survive.
fn execute_command(worktree: &Path, command: &str) -> Result<(i32, String), String> {
    let args = split_command_line(command)?;
    let output = match gwt_core::process::hidden_command(&args[0])
        .args(&args[1..])
        .current_dir(worktree)
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            return Ok((
                -1,
                format!("--- spawn error ---\nfailed to spawn '{command}': {err}\n"),
            ));
        }
    };
    let exit_code = output.status.code().unwrap_or(-1);
    let mut tail = String::new();
    for (label, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        if bytes.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(bytes);
        let text = text.trim_end();
        let clipped: String = if text.len() > OUTPUT_TAIL_LIMIT {
            // Snap the cut to a char boundary — runner output is frequently
            // multibyte (Japanese cargo messages) and a raw byte slice would
            // panic mid-character.
            let mut start = text.len() - OUTPUT_TAIL_LIMIT;
            while start < text.len() && !text.is_char_boundary(start) {
                start += 1;
            }
            format!("...[truncated]\n{}", &text[start..])
        } else {
            text.to_string()
        };
        if !clipped.is_empty() {
            tail.push_str(&format!("--- {label} ---\n{clipped}\n"));
        }
    }
    Ok((exit_code, tail))
}

/// Run the verification commands and persist the record (T-110). The record
/// is written even when commands fail — a failing run is evidence too, it
/// just never satisfies a completion gate.
///
/// This low-level primitive remains available for internal gate fixtures.
/// Caller-facing `verify.run` dispatch goes through
/// `run_verification_for_caller`, which authenticates durable Session
/// authority before command dispatch and again before persistence.
pub fn run_verification(
    worktree: &Path,
    session_id: &str,
    commands: &[String],
) -> Result<(VerificationRunRecord, String), String> {
    run_verification_inner(worktree, session_id, commands, None, || {})
}

fn run_verification_for_caller(
    worktree: &Path,
    session_id: &str,
    commands: &[String],
    authority: &VerificationCallerAuthority,
) -> Result<(VerificationRunRecord, String), String> {
    run_verification_inner(worktree, session_id, commands, Some(authority), || {})
}

fn run_verification_inner<F>(
    worktree: &Path,
    session_id: &str,
    commands: &[String],
    authority: Option<&VerificationCallerAuthority>,
    after_commands: F,
) -> Result<(VerificationRunRecord, String), String>
where
    F: FnOnce(),
{
    if commands.is_empty() {
        return Err("verify.run requires at least one command".to_string());
    }
    // Snapshot owner, plan, and worktree together. Commands deliberately run
    // outside the lease; the final commit reacquires it and rejects any
    // interleaving writer by invalidating the evidence snapshot.
    let (owner_number, execution_binding, plan_snapshot, fingerprint_before) =
        crate::cli::trusted_store::with_write_lease(worktree, || {
            let (owner_number, execution_binding) = if let Some(authority) = authority {
                (authority.owner_number, authority.execution_binding.clone())
            } else {
                current_execution_context(worktree)?
            };
            let plan = load_plan(worktree)?;
            let fingerprint = worktree_fingerprint(worktree);
            if let Some(authority) = authority {
                revalidate_verification_caller_authority(worktree, session_id, authority)?;
            }
            Ok((owner_number, execution_binding, plan, fingerprint))
        })
        .map_err(|err| {
            if err.kind() == ErrorKind::PermissionDenied {
                err.to_string()
            } else {
                format!("failed to snapshot verification state: {err}")
            }
        })?;
    let started_at = Utc::now();
    let mut results: Vec<VerificationCommandResult> = Vec::new();
    let mut transcript = String::new();
    for command in commands {
        transcript.push_str(&format!("$ {command}\n"));
        let (exit_code, tail) = execute_command(worktree, command)?;
        transcript.push_str(&tail);
        transcript.push_str(&format!("exit: {exit_code}\n"));
        results.push(VerificationCommandResult {
            command: command.clone(),
            exit_code,
        });
    }
    after_commands();
    let all_passed = results.iter().all(|result| result.exit_code == 0);
    // T-130: bind coverage to the exact pre-run plan snapshot. A missing,
    // cross-session, tampered, or legacy-hashless plan remains unplanned.
    let (plan_covered, planned_missing, verification_plan_hash, plan_derived) =
        match plan_snapshot.as_ref() {
            Some(plan)
                if !plan.content_hash.is_empty()
                    && plan_integrity_ok(plan)
                    && plan.session_id == session_id
                    && plan.owner_number == owner_number
                    && plan.execution_binding == execution_binding =>
            {
                let ran: std::collections::HashSet<&str> =
                    commands.iter().map(String::as_str).collect();
                let missing: Vec<String> = plan
                    .commands
                    .iter()
                    .filter(|planned| !ran.contains(planned.as_str()))
                    .cloned()
                    .collect();
                (
                    missing.is_empty() && plan.worktree_fingerprint == fingerprint_before,
                    missing,
                    plan.content_hash.clone(),
                    plan.derived,
                )
            }
            _ => (false, Vec::new(), String::new(), false),
        };
    let mut record = VerificationRunRecord {
        record_id: format!("vrr-{}", uuid::Uuid::new_v4().simple()),
        session_id: session_id.to_string(),
        owner_number,
        execution_binding: execution_binding.clone(),
        worktree_fingerprint: fingerprint_before.clone(),
        commands: results,
        all_passed,
        started_at: Some(started_at),
        created_at: Utc::now(),
        plan_covered,
        planned_missing,
        verification_plan_hash,
        plan_derived,
        content_hash: String::new(),
    };

    crate::cli::trusted_store::with_write_lease(worktree, || {
        if let Some(authority) = authority {
            revalidate_verification_caller_authority(worktree, session_id, authority)?;
        }
        let (current_owner, current_binding) = current_execution_context(worktree)?;
        let current_plan = load_plan(worktree)?;
        let fingerprint_after = worktree_fingerprint(worktree);
        if fingerprint_before != fingerprint_after {
            transcript.push_str(
                "warning: the worktree changed while verification ran — the record is invalidated; rerun `verify.run` on the final state\n",
            );
            record.worktree_fingerprint = "invalidated-by-concurrent-change".to_string();
        }
        if current_owner != owner_number {
            transcript.push_str(
                "warning: the execution owner changed while verification ran — the record is invalidated; rerun `verify.run` for the current owner\n",
            );
            record.plan_covered = false;
        }
        if current_binding != execution_binding {
            transcript.push_str(
                "warning: the execution generation changed while verification ran — the record is invalidated; rerun `verify.run` for the current generation\n",
            );
            record.plan_covered = false;
        }
        let same_plan = match (plan_snapshot.as_ref(), current_plan.as_ref()) {
            (Some(before), Some(after)) => {
                !before.content_hash.is_empty()
                    && before.content_hash == after.content_hash
                    && plan_integrity_ok(after)
            }
            (None, None) => true,
            _ => false,
        };
        if !same_plan {
            transcript.push_str(
                "warning: the verification plan changed while verification ran — the record is invalidated; rerun `verify.run` against the current plan\n",
            );
            record.plan_covered = false;
        }
        if let Some(authority) = authority {
            revalidate_verification_caller_authority(worktree, session_id, authority)?;
        }
        record.created_at = Utc::now();
        record.content_hash = compute_content_hash(&record);
        save(worktree, &record)
    })
    .map_err(|err| {
        if err.kind() == ErrorKind::PermissionDenied {
            err.to_string()
        } else {
            format!("failed to save verification record: {err}")
        }
    })?;

    if !record.plan_covered {
        transcript.push_str(
            "note: this run does not cover a registered verification plan for this session — register the matrix with `verify.plan` first, then run it (T-130)\n",
        );
    }
    Ok((record, transcript))
}

/// Evidence status consumed by completion and PR handoff gates (T-111/T-112).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceStatus {
    /// A fresh, all-passing record for this session/owner/fingerprint.
    Fresh,
    MissingRecord,
    WrongSession,
    WrongOwner,
    /// The record belongs to a legacy/predecessor execution generation.
    WrongGeneration,
    StaleFingerprint,
    Failing,
    Unreadable,
    /// P9a (T-122): the stored integrity hash does not match the content —
    /// the record was edited outside `verify.run`.
    Tampered,
    /// T-130-lite: the run did not cover the registered verification plan
    /// (or no plan was registered before running).
    PlanNotCovered,
    /// The currently registered plan is not the exact immutable plan
    /// snapshot consumed by the run.
    PlanChanged,
}

impl EvidenceStatus {
    /// Human guidance for the gate message.
    #[must_use]
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Fresh => "verification evidence is fresh",
            Self::MissingRecord => {
                "no verification run record exists — run the verification matrix through JSON operation `verify.run` with `params.commands:[...]`"
            }
            Self::WrongSession => {
                "the verification record belongs to another session — rerun `verify.run` from this session"
            }
            Self::WrongOwner => {
                "the verification record was taken for a different owner — rerun `verify.run` after the current launch"
            }
            Self::WrongGeneration => {
                "the verification record belongs to a legacy, predecessor, or superseded execution binding — register the plan and rerun `verify.run` from the current generation"
            }
            Self::StaleFingerprint => {
                "the worktree changed after the last verification run (stale evidence) — rerun `verify.run`"
            }
            Self::Failing => {
                "the last verification run has failing commands — fix the failures and rerun `verify.run`"
            }
            Self::Unreadable => {
                "the verification record is unreadable — rerun `verify.run` to rewrite it"
            }
            Self::Tampered => {
                "the verification record failed integrity validation (edited outside `verify.run`) — rerun `verify.run` to produce a genuine record"
            }
            Self::PlanNotCovered => {
                "the last verification run does not cover a registered plan — declare the required matrix with `verify.plan` (params.commands), then run it in full through `verify.run`"
            }
            Self::PlanChanged => {
                "the verification plan changed after the last run — rerun `verify.run` against the current plan"
            }
        }
    }
}

/// Evaluate one already-loaded plan/run snapshot. Callers that need an
/// atomic decision (execution completion/recovery) load both records while
/// holding the owner write lease and use this helper without reopening a
/// TOCTOU window.
#[must_use]
pub fn evaluate_evidence_snapshot(
    worktree: &Path,
    session_id: &str,
    expected_owner_number: Option<u64>,
    plan: Option<&VerificationPlanRecord>,
    record: &VerificationRunRecord,
) -> EvidenceStatus {
    if !integrity_ok(record) {
        return EvidenceStatus::Tampered;
    }
    if record.session_id != session_id {
        return EvidenceStatus::WrongSession;
    }
    if let Some(expected) = expected_owner_number {
        if record.owner_number != Some(expected) {
            return EvidenceStatus::WrongOwner;
        }
    }
    let current_binding = match current_execution_context(worktree) {
        Ok((_, binding)) => binding,
        Err(_) => return EvidenceStatus::Unreadable,
    };
    if record.execution_binding != current_binding {
        return EvidenceStatus::WrongGeneration;
    }
    if record.worktree_fingerprint != worktree_fingerprint(worktree) {
        return EvidenceStatus::StaleFingerprint;
    }
    if !record.all_passed {
        return EvidenceStatus::Failing;
    }
    if !record.verification_plan_hash.is_empty() {
        let Some(plan) = plan else {
            return EvidenceStatus::PlanChanged;
        };
        if plan.content_hash.is_empty()
            || !plan_integrity_ok(plan)
            || plan.session_id != session_id
            || plan.owner_number != record.owner_number
            || plan.execution_binding != record.execution_binding
            || plan.worktree_fingerprint.is_empty()
            || plan.worktree_fingerprint != record.worktree_fingerprint
            || record.verification_plan_hash != plan.content_hash
            || record.plan_derived != plan.derived
        {
            return EvidenceStatus::PlanChanged;
        }
    }
    if !record.plan_covered {
        return EvidenceStatus::PlanNotCovered;
    }
    if record.verification_plan_hash.is_empty() {
        return EvidenceStatus::PlanChanged;
    }
    EvidenceStatus::Fresh
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerificationCallerAuthority {
    owner_number: Option<u64>,
    execution_binding: Option<ExecutionBindingIdentity>,
    session_binding: Option<gwt_agent::SessionExecutionBinding>,
}

fn verification_caller_authority_error() -> io::Error {
    io::Error::new(
        ErrorKind::PermissionDenied,
        "verify.* requires current verification authority; relaunch or continue the owning Session before retrying",
    )
}

fn snapshot_verification_caller_authority(
    worktree: &Path,
    session_id: &str,
) -> io::Result<VerificationCallerAuthority> {
    let Some(execution) =
        execution_state::load(worktree).map_err(|_| verification_caller_authority_error())?
    else {
        return Ok(VerificationCallerAuthority {
            owner_number: None,
            execution_binding: None,
            session_binding: None,
        });
    };
    let owner = execution_state::ExecutionOwnerKey {
        kind: execution.owner_kind,
        number: execution.owner_number,
    };
    let current_binding = execution_state::current_execution_binding(worktree, owner)
        .map_err(|_| verification_caller_authority_error())?;
    let Some(current_binding) = current_binding else {
        return Ok(VerificationCallerAuthority {
            owner_number: Some(execution.owner_number),
            execution_binding: None,
            session_binding: None,
        });
    };
    let ledger = execution_state::load_generation_ledger(worktree, owner)
        .map_err(|_| verification_caller_authority_error())?
        .ok_or_else(verification_caller_authority_error)?;

    if !execution_state::integrity_ok(&execution)
        || !execution_state::generation_ledger_integrity_ok(&ledger)
        || ledger.current_effective_status() != Some(execution.status)
        || execution.primary_session_id != session_id
        || !matches!(
            execution.status,
            execution_state::ExecutionControlStatus::Active
                | execution_state::ExecutionControlStatus::Blocked
        )
    {
        return Err(verification_caller_authority_error());
    }
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|_| verification_caller_authority_error())?;
    let session_path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    let session = gwt_agent::Session::load(&session_path)
        .map_err(|_| verification_caller_authority_error())?;
    let session_binding = session
        .execution_binding
        .clone()
        .ok_or_else(verification_caller_authority_error)?;
    let canonical_worktree =
        dunce::canonicalize(worktree).map_err(|_| verification_caller_authority_error())?;
    let canonical_session_worktree = dunce::canonicalize(&session.worktree_path)
        .map_err(|_| verification_caller_authority_error())?;
    let repo_hash = gwt_core::repo_hash::detect_repo_hash(&canonical_worktree)
        .map(|value| value.as_str().to_string())
        .ok_or_else(verification_caller_authority_error)?;
    if canonical_session_worktree != canonical_worktree
        || session.id != session_id
        || session.repo_hash.as_deref() != Some(repo_hash.as_str())
        || session.linked_issue_number != Some(owner.number)
        || session_binding.schema_version
            != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
        || session_binding.session_id != session_id
        || session_binding.repo_hash != repo_hash
        || session_binding.owner_kind != owner.kind.as_str()
        || session_binding.owner_number != owner.number
        || session_binding.identity != current_binding
        || session_binding.capability_generation == 0
    {
        return Err(verification_caller_authority_error());
    }

    let mut structural_validation = session;
    structural_validation.execution_binding = None;
    structural_validation
        .set_execution_binding(Some(session_binding.clone()))
        .map_err(|_| verification_caller_authority_error())?;
    if execution.status == execution_state::ExecutionControlStatus::Active
        && !execution_state::current_active_execution_binding_matches(
            &canonical_worktree,
            owner,
            session_id,
            &current_binding,
        )
        .map_err(|_| verification_caller_authority_error())?
    {
        return Err(verification_caller_authority_error());
    }

    Ok(VerificationCallerAuthority {
        owner_number: Some(owner.number),
        execution_binding: Some(current_binding),
        session_binding: Some(session_binding),
    })
}

fn revalidate_verification_caller_authority(
    worktree: &Path,
    session_id: &str,
    expected: &VerificationCallerAuthority,
) -> io::Result<()> {
    let current = snapshot_verification_caller_authority(worktree, session_id)?;
    if &current != expected {
        return Err(verification_caller_authority_error());
    }
    Ok(())
}

/// Authenticate the ambient caller against the exact durable Session for the
/// current generation, when this worktree has generation authority.
///
/// Legacy flat ECRs and unmanaged worktrees intentionally remain outside this
/// gate. Callers receive only a boolean-free success/error result so no
/// Session or binding identity can leak into diagnostics.
pub(crate) fn authenticate_current_generation_caller(
    worktree: &Path,
    session_id: Option<&str>,
) -> io::Result<()> {
    snapshot_current_generation_caller_binding(worktree, session_id).map(|_| ())
}

/// Capture the exact durable Session capability epoch authenticated for the
/// current generation. PR mutations keep this snapshot across local payload
/// reads and lease it again immediately around external dispatch.
pub(crate) fn snapshot_current_generation_caller_binding(
    worktree: &Path,
    session_id: Option<&str>,
) -> io::Result<Option<gwt_agent::SessionExecutionBinding>> {
    let (_, current_binding) =
        current_execution_context(worktree).map_err(|_| verification_caller_authority_error())?;
    if current_binding.is_none() {
        return Ok(None);
    }
    let session_id = session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(verification_caller_authority_error)?;
    let authority = snapshot_verification_caller_authority(worktree, session_id)?;
    if authority.execution_binding != current_binding {
        return Err(verification_caller_authority_error());
    }
    Ok(authority.session_binding)
}

/// Resolve the current linked owner and its exact generation identity from
/// the authoritative ECR/ledger pair. A flat legacy ECR intentionally
/// returns an owner with no binding.
fn current_execution_context(
    worktree: &Path,
) -> io::Result<(Option<u64>, Option<ExecutionBindingIdentity>)> {
    let Some(execution) = execution_state::load(worktree)? else {
        return Ok((None, None));
    };
    let owner = execution_state::ExecutionOwnerKey {
        kind: execution.owner_kind,
        number: execution.owner_number,
    };
    let binding = execution_state::current_execution_binding(worktree, owner)?;
    Ok((Some(execution.owner_number), binding))
}

/// Evaluate the latest record against the current session, the Execution
/// Control Record's owner, and the current worktree fingerprint (FR-036).
#[must_use]
pub fn evaluate_evidence(
    worktree: &Path,
    session_id: &str,
    expected_owner_number: Option<u64>,
) -> EvidenceStatus {
    let record = match load(worktree) {
        Ok(Some(record)) => record,
        Ok(None) => return EvidenceStatus::MissingRecord,
        Err(_) => return EvidenceStatus::Unreadable,
    };
    let plan = match load_plan(worktree) {
        Ok(plan) => plan,
        Err(_) => return EvidenceStatus::Unreadable,
    };
    evaluate_evidence_snapshot(
        worktree,
        session_id,
        expected_owner_number,
        plan.as_ref(),
        &record,
    )
}

// ---------------------------------------------------------------------------
// CLI command surface (`verify.run`)
// ---------------------------------------------------------------------------

/// Commands of the `verify.*` JSON operation family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyCommand {
    Run {
        commands: Vec<String>,
    },
    /// T-130-lite: register the required verification matrix before running.
    /// Full T-130 core: `derive` classifies changed surfaces and derives the
    /// matrix when no explicit commands are given.
    Plan {
        commands: Vec<String>,
        derive: bool,
    },
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: VerifyCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SpecOpsError::from(ApiError::Unexpected(
                "verify.* requires GWT_SESSION_ID to bind the record to the session".to_string(),
            ))
        })?;
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    let authority =
        snapshot_verification_caller_authority(&worktree, &session_id).map_err(|err| {
            SpecOpsError::from(ApiError::Unexpected(
                if err.kind() == ErrorKind::PermissionDenied {
                    err.to_string()
                } else {
                    format!("failed to resolve verification authority: {err}")
                },
            ))
        })?;
    match command {
        VerifyCommand::Plan { commands, derive } => {
            let (commands, plan) = if derive {
                if !commands.is_empty() {
                    return Err(SpecOpsError::from(ApiError::Unexpected(
                        "verify.plan takes either params.derive:true or explicit params.commands, not both"
                            .to_string(),
                    )));
                }
                let (derived_plan, plan) =
                    derive_and_register_plan_for_caller(&worktree, &session_id, &authority)
                        .map_err(|err| SpecOpsError::from(ApiError::Unexpected(err)))?;
                out.push_str(&format!(
                    "verify: derived matrix from changed surfaces [{}]\n",
                    derived_plan.surfaces.join(", ")
                ));
                for command in &derived_plan.commands {
                    out.push_str(&format!("  - {command}\n"));
                }
                (derived_plan.commands, plan)
            } else {
                if commands.is_empty() {
                    return Err(SpecOpsError::from(ApiError::Unexpected(
                        "verify.plan requires at least one command (or params.derive:true)"
                            .to_string(),
                    )));
                }
                let plan = register_plan_for_caller(
                    &worktree,
                    &session_id,
                    commands.clone(),
                    false,
                    &authority,
                )
                .map_err(|err| {
                    SpecOpsError::from(if err.kind() == ErrorKind::PermissionDenied {
                        ApiError::Unexpected(err.to_string())
                    } else {
                        ApiError::Unexpected(crate::cli::trusted_store::store_health_error(
                            "updating verification state",
                            &err,
                        ))
                    })
                })?;
                (commands, plan)
            };
            out.push_str(&format!(
                "verify: plan registered — {count} command(s) for session {session_id} (owner {owner}{derived_note})\n",
                count = commands.len(),
                owner = plan
                    .owner_number
                    .map(|n| format!("#{n}"))
                    .unwrap_or_else(|| "none".to_string()),
                derived_note = if plan.derived { ", derived" } else { "" },
            ));
            Ok(0)
        }
        VerifyCommand::Run { commands } => {
            let (record, transcript) =
                run_verification_for_caller(&worktree, &session_id, &commands, &authority)
                    .map_err(|err| SpecOpsError::from(ApiError::Unexpected(err)))?;
            // T-131 core: surface the coverage map of the plan this run
            // covered, so the rationale travels with the evidence output.
            if record.plan_covered {
                if let Ok(Some(plan)) = load_plan(&worktree) {
                    if !plan.surfaces.is_empty() {
                        out.push_str(&format!(
                            "verify: coverage map [{}]\n",
                            plan.surfaces.join(", ")
                        ));
                    }
                }
            }
            if record.all_passed && record.plan_covered {
                // P11: only an all-passing run that covers the registered
                // plan settles implementation/verification obligations — a
                // plan-less trivial run must not (vacuous-settlement fix).
                crate::cli::action_obligation::settle_kinds_best_effort(
                    &worktree,
                    &session_id,
                    &[
                        crate::cli::action_obligation::ObligationKind::Verification,
                        crate::cli::action_obligation::ObligationKind::Implementation,
                    ],
                    &format!("verify.run {}", record.record_id),
                );
            }
            out.push_str(&transcript);
            out.push_str(&format!(
                "verify: {status} — record {id} ({count} command(s), owner {owner})\n",
                status = if record.all_passed { "PASS" } else { "FAIL" },
                id = record.record_id,
                count = record.commands.len(),
                owner = record
                    .owner_number
                    .map(|n| format!("#{n}"))
                    .unwrap_or_else(|| "none".to_string()),
            ));
            Ok(if record.all_passed { 0 } else { 1 })
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    /// Register a plan for the commands, then run them (the standard
    /// T-130-lite flow used everywhere Fresh evidence is needed).
    fn plan_and_run(
        worktree: &Path,
        session: &str,
        commands: &[String],
    ) -> (VerificationRunRecord, String) {
        let owner_number = crate::cli::execution_state::load(worktree)
            .ok()
            .flatten()
            .map(|record| record.owner_number);
        save_plan(
            worktree,
            &VerificationPlanRecord {
                session_id: session.to_string(),
                owner_number,
                execution_binding: None,
                commands: commands.to_vec(),
                derived: false,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                created_at: Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        run_verification(worktree, session, commands).unwrap()
    }

    fn passing_record(session: &str, fingerprint: &str) -> VerificationRunRecord {
        VerificationRunRecord {
            record_id: "vr-test".to_string(),
            session_id: session.to_string(),
            owner_number: Some(3248),
            execution_binding: None,
            worktree_fingerprint: fingerprint.to_string(),
            commands: vec![VerificationCommandResult {
                command: "git --version".to_string(),
                exit_code: 0,
            }],
            all_passed: true,
            started_at: Some(Utc::now()),
            created_at: Utc::now(),
            plan_covered: true,
            planned_missing: Vec::new(),
            verification_plan_hash: String::new(),
            plan_derived: false,
            content_hash: String::new(),
        }
    }

    // T-131 core: the Coverage Map (derived surface classification) is
    // persisted on the plan record, roundtrips, and is integrity-hash
    // covered — editing it after registration is tamper-evident.
    #[test]
    fn plan_surfaces_roundtrip_and_are_hash_covered() {
        let dir = tempfile::tempdir().unwrap();
        save_plan(
            dir.path(),
            &VerificationPlanRecord {
                session_id: "sess-cov".to_string(),
                owner_number: Some(3248),
                execution_binding: None,
                commands: vec!["cargo test -p gwt --lib".to_string()],
                derived: true,
                surfaces: vec!["rust(gwt)".to_string(), "docs(1)".to_string()],
                worktree_fingerprint: String::new(),
                created_at: Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        let loaded = load_plan(dir.path()).unwrap().unwrap();
        assert_eq!(
            loaded.surfaces,
            vec!["rust(gwt)".to_string(), "docs(1)".to_string()]
        );
        assert!(plan_integrity_ok(&loaded));

        let mut tampered = loaded.clone();
        tampered.surfaces = vec!["docs(1)".to_string()];
        assert!(
            !plan_integrity_ok(&tampered),
            "coverage map edits must break the integrity hash"
        );
    }

    // T-131 core: verify.plan derive:true persists the surface
    // classification it derived from the change set.
    #[test]
    fn derived_registration_records_coverage_map() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let git = |args: &[&str]| {
            let status = gwt_core::process::hidden_command("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        git(&["update-ref", "refs/remotes/origin/develop", "HEAD"]);
        git(&["checkout", "-q", "-b", "work/coverage"]);
        let src = dir.path().join("crates/gwt-core/src/lib.rs");
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, "pub fn x() {}").unwrap();

        let authority = snapshot_verification_caller_authority(dir.path(), "sess-cov").unwrap();
        let (derived, plan) =
            derive_and_register_plan_for_caller(dir.path(), "sess-cov", &authority).unwrap();
        assert!(!derived.surfaces.is_empty());
        assert_eq!(plan.surfaces, derived.surfaces);
        assert!(plan.derived);
        let loaded = load_plan(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.surfaces, derived.surfaces);
        assert!(plan_integrity_ok(&loaded));
    }

    // P9b (T-174 core): the repo-scoped trusted copy wins over a forged
    // worktree mirror — for both the run record and the registered plan.
    #[test]
    fn trusted_copies_override_worktree_mirror_edits() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        let mut failing = passing_record("sess-1", "fp-1");
        failing.all_passed = false;
        save(dir.path(), &failing).unwrap();
        // Forge an all-passing mirror with a valid integrity hash.
        let mut forged = passing_record("sess-1", "fp-1");
        forged.content_hash = compute_content_hash(&forged);
        let serialized = serde_json::to_vec_pretty(&forged).unwrap();
        gwt_github::cache::write_atomic(&state_path(dir.path()), &serialized).unwrap();
        assert!(!load(dir.path()).unwrap().unwrap().all_passed);

        let plan = VerificationPlanRecord {
            session_id: "sess-1".to_string(),
            owner_number: Some(3248),
            execution_binding: None,
            commands: vec!["cargo test -p gwt --lib".to_string()],
            derived: false,
            worktree_fingerprint: String::new(),
            surfaces: Vec::new(),
            created_at: Utc::now(),
            content_hash: String::new(),
        };
        save_plan(dir.path(), &plan).unwrap();
        // Forge a trivial (empty-matrix) plan in the mirror.
        let mut forged_plan = plan.clone();
        forged_plan.commands = vec!["git --version".to_string()];
        forged_plan.content_hash = compute_plan_hash(&forged_plan);
        let serialized = serde_json::to_vec_pretty(&forged_plan).unwrap();
        gwt_github::cache::write_atomic(&plan_state_path(dir.path()), &serialized).unwrap();
        assert_eq!(
            load_plan(dir.path()).unwrap().unwrap().commands,
            vec!["cargo test -p gwt --lib".to_string()]
        );
    }

    // P9b: mirror-only records/plans (pre-P9b) still load as legacy fallback.
    #[test]
    fn mirror_only_records_load_as_legacy_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        let mut legacy = passing_record("sess-legacy", "fp-legacy");
        legacy.content_hash = compute_content_hash(&legacy);
        let serialized = serde_json::to_vec_pretty(&legacy).unwrap();
        gwt_github::cache::write_atomic(&state_path(dir.path()), &serialized).unwrap();
        assert_eq!(load(dir.path()).unwrap().unwrap().session_id, "sess-legacy");
    }

    // P9b: in a managed worktree (trusted ECR exists from launch) a
    // mirror-only record or plan is NOT evidence — before the first real
    // `verify.run` the mirror must not become authoritative by default.
    #[test]
    fn managed_worktree_refuses_mirror_only_records() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        // Launch materialization writes the trusted ECR copy.
        crate::cli::execution_state::materialize_at_launch(
            dir.path(),
            crate::cli::execution_state::ExecutionOwnerKind::Spec,
            3248,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();

        // Forge an all-passing mirror-only run record with a valid hash.
        let mut forged = passing_record("sess-1", "fp-1");
        forged.content_hash = compute_content_hash(&forged);
        let serialized = serde_json::to_vec_pretty(&forged).unwrap();
        gwt_github::cache::write_atomic(&state_path(dir.path()), &serialized).unwrap();
        assert_eq!(load(dir.path()).unwrap(), None);

        // Same for a forged trivial plan.
        let mut forged_plan = VerificationPlanRecord {
            session_id: "sess-1".to_string(),
            owner_number: Some(3248),
            execution_binding: None,
            commands: vec!["git --version".to_string()],
            derived: false,
            worktree_fingerprint: String::new(),
            surfaces: Vec::new(),
            created_at: Utc::now(),
            content_hash: String::new(),
        };
        forged_plan.content_hash = compute_plan_hash(&forged_plan);
        let serialized = serde_json::to_vec_pretty(&forged_plan).unwrap();
        gwt_github::cache::write_atomic(&plan_state_path(dir.path()), &serialized).unwrap();
        assert_eq!(load_plan(dir.path()).unwrap(), None);
    }

    #[test]
    fn split_command_line_handles_quotes_and_rejects_shell_operators() {
        assert_eq!(
            split_command_line("cargo test -p gwt --lib").unwrap(),
            vec!["cargo", "test", "-p", "gwt", "--lib"]
        );
        assert_eq!(
            split_command_line(r#"git commit -m "two words""#).unwrap(),
            vec!["git", "commit", "-m", "two words"]
        );
        assert_eq!(
            split_command_line("echo 'single quoted arg'").unwrap(),
            vec!["echo", "single quoted arg"]
        );
        assert!(split_command_line("cargo test && cargo fmt").is_err());
        assert!(split_command_line("cargo test | tail").is_err());
        assert!(split_command_line("").is_err());
        assert!(split_command_line("echo \"unbalanced").is_err());
        // Quoted operator literals are deliberate arguments, not shell syntax.
        assert_eq!(
            split_command_line(r#"rg "&&" crates/"#).unwrap(),
            vec!["rg", "&&", "crates/"]
        );
        assert_eq!(
            split_command_line("grep ';' config.toml").unwrap(),
            vec!["grep", ";", "config.toml"]
        );
    }

    // Spawn failures are recorded as failed results, never dropped runs.
    #[test]
    fn spawn_failure_is_recorded_as_failed_result() {
        let dir = tempfile::tempdir().unwrap();
        let (record, transcript) = run_verification(
            dir.path(),
            "sess-1",
            &[
                "git --version".to_string(),
                "definitely-not-a-real-binary-xyz --flag".to_string(),
            ],
        )
        .unwrap();
        assert!(!record.all_passed);
        assert_eq!(record.commands.len(), 2);
        assert_eq!(record.commands[1].exit_code, -1);
        assert!(transcript.contains("spawn error"), "{transcript}");
        assert!(load(dir.path()).unwrap().is_some(), "record must persist");
    }

    #[test]
    fn record_roundtrips_and_missing_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), None);
        let record = VerificationRunRecord {
            record_id: "vrr-test".to_string(),
            session_id: "sess-1".to_string(),
            owner_number: Some(3248),
            execution_binding: None,
            worktree_fingerprint: "abc".to_string(),
            commands: vec![VerificationCommandResult {
                command: "git --version".to_string(),
                exit_code: 0,
            }],
            all_passed: true,
            started_at: Some(Utc::now()),
            created_at: Utc::now(),
            plan_covered: true,
            planned_missing: Vec::new(),
            verification_plan_hash: String::new(),
            plan_derived: false,
            content_hash: String::new(),
        };
        save(dir.path(), &record).unwrap();
        let loaded = load(dir.path()).unwrap().unwrap();
        assert!(integrity_ok(&loaded));
        assert!(!loaded.content_hash.is_empty());
        let mut normalized = loaded.clone();
        normalized.content_hash = String::new();
        assert_eq!(normalized, record);
    }

    // T-110: verify.run executes real commands, records exit codes, and the
    // record is written even when a command fails.
    #[test]
    fn run_verification_records_pass_and_fail() {
        let dir = tempfile::tempdir().unwrap();
        let (record, transcript) =
            run_verification(dir.path(), "sess-1", &["git --version".to_string()]).unwrap();
        assert!(record.all_passed, "{transcript}");
        assert_eq!(record.commands[0].exit_code, 0);
        assert_eq!(record.session_id, "sess-1");

        let (record, _) = run_verification(
            dir.path(),
            "sess-1",
            &[
                "git --version".to_string(),
                "git definitely-not-a-subcommand".to_string(),
            ],
        )
        .unwrap();
        assert!(!record.all_passed);
        assert_eq!(record.commands.len(), 2);
        assert_ne!(record.commands[1].exit_code, 0);
        // Latest record persisted.
        assert!(!load(dir.path()).unwrap().unwrap().all_passed);
    }

    // FR-198: canonical plan writes and the final verification-record commit
    // share the owner write lease with execution transitions. Contention is
    // an explicit retry, never last-writer-wins.
    #[test]
    fn verification_writers_refuse_with_retry_while_owner_lease_is_held() {
        let dir = tempfile::tempdir().unwrap();
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

        let plan_result = save_plan(
            dir.path(),
            &VerificationPlanRecord {
                session_id: "sess-1".to_string(),
                owner_number: None,
                execution_binding: None,
                commands: vec!["git --version".to_string()],
                derived: true,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                created_at: Utc::now(),
                content_hash: String::new(),
            },
        );
        let run_result = run_verification(dir.path(), "sess-1", &["git --version".to_string()]);
        release_tx.send(()).unwrap();
        holder.join().unwrap();

        let plan_error = plan_result.expect_err("plan writer must contend on owner lease");
        assert!(plan_error.to_string().contains("retry"), "{plan_error}");
        let run_error = run_result.expect_err("run commit must contend on owner lease");
        assert!(run_error.contains("retry"), "{run_error}");
    }

    // T-111/FR-036: evidence evaluation rejects missing, cross-session,
    // wrong-owner, and failing records; accepts a fresh matching one.
    #[test]
    fn evaluate_evidence_covers_rejection_matrix() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::MissingRecord
        );

        let (_, _) = plan_and_run(dir.path(), "sess-1", &["git --version".to_string()]);
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::Fresh
        );
        assert_eq!(
            evaluate_evidence(dir.path(), "other", None),
            EvidenceStatus::WrongSession
        );
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", Some(3248)),
            EvidenceStatus::WrongOwner,
            "record without owner must not satisfy an owned execution"
        );

        // Failing run never satisfies.
        run_verification(
            dir.path(),
            "sess-1",
            &["git definitely-not-a-subcommand".to_string()],
        )
        .unwrap();
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::Failing
        );
    }

    // Owner binding comes from the Execution Control Record at run time.
    #[test]
    fn run_verification_binds_owner_from_execution_record() {
        let dir = tempfile::tempdir().unwrap();
        crate::cli::execution_state::materialize_at_launch(
            dir.path(),
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            3248,
            "sess-1",
            "launch",
            false,
        )
        .unwrap();
        let (record, _) = plan_and_run(dir.path(), "sess-1", &["git --version".to_string()]);
        assert_eq!(record.owner_number, Some(3248));
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", Some(3248)),
            EvidenceStatus::Fresh
        );
    }

    // Freshness: a tracked-file change after the run invalidates evidence,
    // while .gwt/ bookkeeping churn does not.
    #[test]
    fn fingerprint_staleness_tracks_source_changes_not_gwt_bookkeeping() {
        let dir = tempfile::tempdir().unwrap();
        let init = |args: &[&str]| {
            let status = gwt_core::process::hidden_command("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        init(&["init", "-q"]);
        init(&["config", "user.email", "t@example.com"]);
        init(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("src.txt"), "v1").unwrap();
        init(&["add", "."]);
        init(&["commit", "-qm", "init"]);

        plan_and_run(dir.path(), "sess-1", &["git --version".to_string()]);
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::Fresh
        );

        // .gwt bookkeeping churn does not invalidate evidence.
        std::fs::create_dir_all(dir.path().join(".gwt")).unwrap();
        std::fs::write(dir.path().join(".gwt/events.jsonl"), "{}").unwrap();
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::Fresh
        );

        // A tracked source change does.
        std::fs::write(dir.path().join("src.txt"), "v2").unwrap();
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::StaleFingerprint
        );

        // Content-level staleness (FR-036): re-verify on the dirty state,
        // then edit the SAME already-dirty file again — porcelain output is
        // unchanged but the content differs, so evidence must go stale.
        plan_and_run(dir.path(), "sess-1", &["git --version".to_string()]);
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::Fresh
        );
        std::fs::write(dir.path().join("src.txt"), "v3-same-status-new-content").unwrap();
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::StaleFingerprint,
            "edits to already-dirty files must invalidate evidence"
        );

        // Same for a new file inside an already-untracked directory.
        std::fs::create_dir_all(dir.path().join("newdir")).unwrap();
        std::fs::write(dir.path().join("newdir/a.txt"), "a").unwrap();
        plan_and_run(dir.path(), "sess-1", &["git --version".to_string()]);
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::Fresh
        );
        std::fs::write(dir.path().join("newdir/b.txt"), "b").unwrap();
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::StaleFingerprint,
            "new files inside untracked directories must invalidate evidence"
        );
    }

    // T-130-lite: evidence requires a registered plan and a covering run.
    #[test]
    fn plan_coverage_gates_freshness() {
        let dir = tempfile::tempdir().unwrap();

        // No plan registered — a passing run is still not Fresh.
        run_verification(dir.path(), "sess-1", &["git --version".to_string()]).unwrap();
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::PlanNotCovered
        );

        // Plan with two commands; running only one is not covered.
        save_plan(
            dir.path(),
            &VerificationPlanRecord {
                session_id: "sess-1".to_string(),
                owner_number: None,
                execution_binding: None,
                commands: vec!["git --version".to_string(), "git --exec-path".to_string()],
                derived: false,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                created_at: Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        let (record, transcript) =
            run_verification(dir.path(), "sess-1", &["git --version".to_string()]).unwrap();
        assert!(!record.plan_covered, "{transcript}");
        assert_eq!(record.planned_missing, vec!["git --exec-path"]);
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::PlanNotCovered
        );

        // Running the full plan (superset allowed) is covered and Fresh.
        let (record, _) = run_verification(
            dir.path(),
            "sess-1",
            &[
                "git --version".to_string(),
                "git --exec-path".to_string(),
                "git --html-path".to_string(),
            ],
        )
        .unwrap();
        assert!(record.plan_covered);
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::Fresh
        );

        // Another session's plan does not count for this session's run.
        save_plan(
            dir.path(),
            &VerificationPlanRecord {
                session_id: "sess-other".to_string(),
                owner_number: None,
                execution_binding: None,
                commands: vec!["git --version".to_string()],
                derived: false,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                created_at: Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        let (record, _) =
            run_verification(dir.path(), "sess-1", &["git --version".to_string()]).unwrap();
        assert!(!record.plan_covered);
    }

    // FR-195 / AS-177: Fresh evidence is bound to the exact plan snapshot
    // used by the run. Re-registering any plan invalidates the old run.
    #[test]
    fn evidence_rejects_plan_replacement_after_run() {
        let dir = tempfile::tempdir().unwrap();
        let commands = vec!["git --version".to_string()];
        save_plan(
            dir.path(),
            &VerificationPlanRecord {
                session_id: "sess-1".to_string(),
                owner_number: None,
                execution_binding: None,
                commands: commands.clone(),
                derived: true,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                created_at: Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        let (run, _) = run_verification(dir.path(), "sess-1", &commands).unwrap();
        assert!(run.plan_covered);
        assert!(run.plan_derived);
        assert!(!run.verification_plan_hash.is_empty());
        assert!(run
            .started_at
            .is_some_and(|started| started <= run.created_at));
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::Fresh
        );

        save_plan(
            dir.path(),
            &VerificationPlanRecord {
                session_id: "sess-1".to_string(),
                owner_number: None,
                execution_binding: None,
                commands: vec!["git --exec-path".to_string()],
                derived: true,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                created_at: Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::PlanChanged
        );
    }

    // FR-195: a derived matrix belongs to the exact change set from which it
    // was derived. Adding a new surface after plan registration must not let
    // the old matrix bind itself to the newer run fingerprint.
    #[test]
    fn derived_plan_rejects_worktree_drift_before_run() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let commands = vec!["git --version".to_string()];
        save_plan(
            dir.path(),
            &VerificationPlanRecord {
                session_id: "sess-1".to_string(),
                owner_number: None,
                execution_binding: None,
                commands: commands.clone(),
                derived: true,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                created_at: Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        fs::write(dir.path().join("new-rust-surface.rs"), "fn added() {}\n").unwrap();

        let (run, _) = run_verification(dir.path(), "sess-1", &commands).unwrap();
        assert!(
            !run.plan_covered,
            "a drifted derived plan must be invalidated"
        );
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-1", None),
            EvidenceStatus::PlanChanged
        );
    }

    // verify.run command surface: session binding is required.
    #[test]
    fn verify_run_op_requires_session() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let dir = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(dir.path().to_path_buf());
        let err = crate::cli::run_collect(
            &mut env,
            crate::cli::CliCommand::Verify(VerifyCommand::Run {
                commands: vec!["git --version".to_string()],
            }),
        )
        .expect_err("missing GWT_SESSION_ID must fail");
        assert!(err.to_string().contains("GWT_SESSION_ID"), "{err}");
    }

    // P11 review fix: only a plan-covering all-passing run settles
    // implementation/verification obligations — a plan-less trivial run
    // must not (vacuous-settlement guard).
    #[test]
    fn verify_run_settles_obligations_only_when_plan_covered() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-ob");
        let dir = tempfile::tempdir().unwrap();
        crate::cli::action_obligation::mark_from_prompt(dir.path(), "sess-ob", "バグを修正して")
            .unwrap();

        // Plan-less run: passes, but does not cover any registered plan.
        let mut env = crate::cli::TestEnv::new(dir.path().to_path_buf());
        let (code, out) = crate::cli::run_collect(
            &mut env,
            crate::cli::CliCommand::Verify(VerifyCommand::Run {
                commands: vec!["git --version".to_string()],
            }),
        )
        .unwrap();
        assert_eq!(code, 0, "{out}");
        assert_eq!(
            crate::cli::action_obligation::open_kinds(dir.path(), "sess-ob"),
            vec![crate::cli::action_obligation::ObligationKind::Implementation],
            "plan-less run must not settle obligations"
        );

        // Registered plan + covering run settles.
        save_plan(
            dir.path(),
            &VerificationPlanRecord {
                session_id: "sess-ob".to_string(),
                owner_number: None,
                execution_binding: None,
                commands: vec!["git --version".to_string()],
                derived: false,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                created_at: Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        let mut env = crate::cli::TestEnv::new(dir.path().to_path_buf());
        let (code, out) = crate::cli::run_collect(
            &mut env,
            crate::cli::CliCommand::Verify(VerifyCommand::Run {
                commands: vec!["git --version".to_string()],
            }),
        )
        .unwrap();
        assert_eq!(code, 0, "{out}");
        assert!(
            crate::cli::action_obligation::open_kinds(dir.path(), "sess-ob").is_empty(),
            "plan-covering run must settle implementation obligations"
        );
    }

    const WORK_EVENTS_PATH: &str = ".gwt/work/events.jsonl";

    pub(crate) struct WorkEventGitFixture {
        _root: tempfile::TempDir,
        pub(crate) repo: PathBuf,
        remote: PathBuf,
    }

    impl WorkEventGitFixture {
        pub(crate) fn tracked() -> Self {
            Self::new(true)
        }

        fn without_tracked_event_log() -> Self {
            Self::new(false)
        }

        fn new(track_event_log: bool) -> Self {
            let root = tempfile::tempdir().expect("git fixture root");
            let repo = root.path().join("repo");
            let remote = root.path().join("upstream.git");
            fs::create_dir_all(&repo).expect("create fixture repository");

            let bare = gwt_core::process::hidden_command("git")
                .args(["init", "--bare", "-q"])
                .arg(&remote)
                .output()
                .expect("initialize bare upstream");
            assert!(
                bare.status.success(),
                "initialize bare upstream: {}",
                String::from_utf8_lossy(&bare.stderr)
            );
            let init = gwt_core::process::hidden_command("git")
                .args(["init", "-q", "-b", "main"])
                .arg(&repo)
                .output()
                .expect("initialize fixture repository");
            assert!(
                init.status.success(),
                "initialize fixture repository: {}",
                String::from_utf8_lossy(&init.stderr)
            );

            let fixture = Self {
                _root: root,
                repo,
                remote,
            };
            fixture.git_ok(&["config", "user.email", "settlement@example.com"]);
            fixture.git_ok(&["config", "user.name", "Settlement Test"]);
            fixture.git_ok(&["config", "commit.gpgsign", "false"]);
            fixture.git_ok(&[
                "remote",
                "add",
                "origin",
                fixture.remote.to_str().expect("UTF-8 remote path"),
            ]);
            fs::write(fixture.repo.join("src.txt"), "initial source\n")
                .expect("write initial source");
            if track_event_log {
                fixture.write_event_log("base");
            }
            fixture.git_ok(&["add", "--all"]);
            fixture.git_ok(&["commit", "-qm", "feat: initial source and Work event"]);
            fixture.git_ok(&["push", "-qu", "origin", "main"]);
            fixture
        }

        fn git_output(&self, args: &[&str]) -> std::process::Output {
            gwt_core::process::hidden_command("git")
                .args(args)
                .current_dir(&self.repo)
                .output()
                .unwrap_or_else(|error| panic!("spawn git {args:?}: {error}"))
        }

        fn git_ok(&self, args: &[&str]) {
            let output = self.git_output(args);
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn git_stdout(&self, args: &[&str]) -> String {
            let output = self.git_output(args);
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("UTF-8 git output")
                .trim()
                .to_string()
        }

        fn event_path(&self) -> PathBuf {
            self.repo.join(WORK_EVENTS_PATH)
        }

        fn write_event_log(&self, marker: &str) {
            let path = self.event_path();
            fs::create_dir_all(path.parent().expect("event parent"))
                .expect("create Work event directory");
            fs::write(path, format!(r#"{{"id":"{marker}"}}"#) + "\n")
                .expect("write Work event log");
        }

        pub(crate) fn append_event(&self, marker: &str) {
            let path = self.event_path();
            let mut contents = fs::read_to_string(&path).unwrap_or_default();
            contents.push_str(&format!(r#"{{"id":"{marker}"}}"#));
            contents.push('\n');
            fs::create_dir_all(path.parent().expect("event parent"))
                .expect("create Work event directory");
            fs::write(path, contents).expect("append Work event");
        }

        fn append_typed_event(&self, event: &gwt_core::workspace_projection::WorkEvent) {
            let path = self.event_path();
            let mut contents = fs::read_to_string(&path).unwrap_or_default();
            contents.push_str(&serde_json::to_string(event).expect("serialize Work event"));
            contents.push('\n');
            fs::create_dir_all(path.parent().expect("event parent"))
                .expect("create Work event directory");
            fs::write(path, contents).expect("append typed Work event");
        }

        pub(crate) fn stage_events(&self) {
            self.git_ok(&["add", "--", WORK_EVENTS_PATH]);
        }

        pub(crate) fn commit(&self, subject: &str) {
            self.git_ok(&["commit", "-qm", subject]);
        }

        pub(crate) fn push(&self) {
            self.git_ok(&["push", "-q", "origin", "main"]);
        }

        fn latest_event_commit(&self) -> String {
            self.git_stdout(&["rev-list", "-1", "HEAD", "--", WORK_EVENTS_PATH])
        }

        fn upstream_ref(&self) -> String {
            self.git_stdout(&["rev-parse", "--symbolic-full-name", "@{upstream}"])
        }

        fn settled_status(&self) -> WorkEventSettlementStatus {
            WorkEventSettlementStatus::Settled {
                event_commit: self.latest_event_commit(),
                upstream_ref: self.upstream_ref(),
            }
        }

        fn advance_remote_from_peer(&self) {
            let peer = self._root.path().join("peer");
            let clone = gwt_core::process::hidden_command("git")
                .args(["clone", "-q", "--branch", "main"])
                .arg(&self.remote)
                .arg(&peer)
                .output()
                .expect("clone peer fixture");
            assert!(
                clone.status.success(),
                "clone peer fixture: {}",
                String::from_utf8_lossy(&clone.stderr)
            );
            for args in [
                ["config", "user.email", "peer@example.com"].as_slice(),
                ["config", "user.name", "Settlement Peer"].as_slice(),
                ["config", "commit.gpgsign", "false"].as_slice(),
            ] {
                let output = gwt_core::process::hidden_command("git")
                    .args(args)
                    .current_dir(&peer)
                    .output()
                    .expect("configure peer fixture");
                assert!(output.status.success(), "configure peer: {args:?}");
            }
            fs::write(peer.join("peer.txt"), "remote source change\n").expect("write peer source");
            for args in [
                ["add", "peer.txt"].as_slice(),
                ["commit", "-qm", "fix: advance remote source"].as_slice(),
                ["push", "-q", "origin", "main"].as_slice(),
            ] {
                let output = gwt_core::process::hidden_command("git")
                    .args(args)
                    .current_dir(&peer)
                    .output()
                    .expect("advance peer fixture");
                assert!(
                    output.status.success(),
                    "peer git {args:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }

    fn generation_scoped_owner() -> crate::cli::execution_state::ExecutionOwnerKey {
        crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Spec,
            number: 2359,
        }
    }

    fn generation_scoped_successor_request(
        operation_id: &str,
        session_id: &str,
    ) -> crate::cli::execution_state::SuccessorRequest {
        crate::cli::execution_state::SuccessorRequest {
            operation_id: operation_id.to_string(),
            principal_id: format!("principal-{operation_id}"),
            work_id: None,
            source: "continue-work".to_string(),
            session_binding_id: format!("binding-{operation_id}"),
            initial_session_id: session_id.to_string(),
            entrypoint: "continue-work".to_string(),
            requested_at: Utc::now(),
        }
    }

    fn initialize_generation_scoped_execution(
        worktree: &Path,
        session_id: &str,
    ) -> gwt_agent::session::ExecutionBindingIdentity {
        let owner = generation_scoped_owner();
        crate::cli::execution_state::materialize_at_launch(
            worktree,
            owner.kind,
            owner.number,
            session_id,
            "gwt-execute",
            false,
        )
        .expect("materialize legacy Active execution");
        crate::cli::execution_state::ensure_generation_ledger(
            worktree,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("import Active execution generation");
        crate::cli::execution_state::current_execution_binding(worktree, owner)
            .expect("load current execution binding")
            .expect("execution generation binding exists")
    }

    fn persist_generation_scoped_session(
        worktree: &Path,
        session_id: &str,
        identity: gwt_agent::ExecutionBindingIdentity,
        capability_generation: u64,
    ) -> gwt_agent::SessionExecutionBinding {
        let owner = generation_scoped_owner();
        let mut session = gwt_agent::Session::new(
            worktree,
            "work/verification-authority",
            gwt_agent::AgentId::Codex,
        );
        session.id = session_id.to_string();
        session.project_state_root = Some(worktree.to_path_buf());
        session.linked_issue_number = Some(owner.number);
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session_id.to_string(),
            repo_hash: session
                .repo_hash
                .clone()
                .expect("generation fixture repository hash"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation,
        };
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind generation-scoped Session");
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("persist generation-scoped Session");
        binding
    }

    fn advance_generation_scoped_session_binding(
        session_id: &str,
        identity: gwt_agent::ExecutionBindingIdentity,
    ) -> gwt_agent::SessionExecutionBinding {
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let path = sessions_dir.join(format!("{session_id}.toml"));
        let mut session = gwt_agent::Session::load(&path).expect("load generation-scoped Session");
        let mut binding = session
            .execution_binding
            .clone()
            .expect("Session carries execution binding");
        binding.identity = identity;
        binding.capability_generation += 1;
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("advance generation-scoped Session binding");
        session
            .save(&sessions_dir)
            .expect("persist advanced generation-scoped Session binding");
        binding
    }

    #[derive(Debug, PartialEq, Eq)]
    struct VerificationArtifactBytes {
        trusted_plan: Option<Vec<u8>>,
        trusted_run: Option<Vec<u8>>,
        mirror_plan: Option<Vec<u8>>,
        mirror_run: Option<Vec<u8>>,
    }

    fn verification_artifact_bytes(worktree: &Path) -> VerificationArtifactBytes {
        fn read_optional(path: &Path) -> Option<Vec<u8>> {
            match fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => panic!("read verification artifact {}: {error}", path.display()),
            }
        }

        let trusted = crate::cli::trusted_store::trusted_dir_for_worktree(worktree);
        VerificationArtifactBytes {
            trusted_plan: trusted
                .as_ref()
                .and_then(|dir| read_optional(&dir.join("verification-plan.json"))),
            trusted_run: trusted
                .as_ref()
                .and_then(|dir| read_optional(&dir.join("verification-run.json"))),
            mirror_plan: read_optional(&plan_state_path(worktree)),
            mirror_run: read_optional(&state_path(worktree)),
        }
    }

    fn assert_verification_authority_denial(
        error: &gwt_github::SpecOpsError,
        reflected_secret: &str,
    ) {
        let diagnostic = error.to_string();
        assert!(
            diagnostic.contains("current verification authority"),
            "denial must be actionable: {diagnostic}"
        );
        assert!(
            !diagnostic.contains(reflected_secret),
            "denial must not reflect caller authority: {diagnostic}"
        );
    }

    fn run_verify_cli_as(
        worktree: &Path,
        session_id: &str,
        command: VerifyCommand,
    ) -> Result<(i32, String), gwt_github::SpecOpsError> {
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let mut env = crate::cli::TestEnv::new(worktree.to_path_buf());
        crate::cli::run_collect(&mut env, crate::cli::CliCommand::Verify(command))
    }

    #[test]
    fn generation_scoped_verify_plan_rejects_foreign_session_before_current_identity_stamp() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("generation-scoped caller authority repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        let current =
            initialize_generation_scoped_execution(dir.path(), "session-current-authority");
        persist_generation_scoped_session(
            dir.path(),
            "session-current-authority",
            current.clone(),
            1,
        );
        persist_generation_scoped_session(
            dir.path(),
            "session-foreign-authority-secret",
            current,
            1,
        );

        let artifacts_before = verification_artifact_bytes(dir.path());
        let error = run_verify_cli_as(
            dir.path(),
            "session-foreign-authority-secret",
            VerifyCommand::Plan {
                commands: vec!["git --version".to_string()],
                derive: false,
            },
        )
        .expect_err("foreign Session must not stamp the current generation identity");
        assert_verification_authority_denial(&error, "session-foreign-authority-secret");
        assert!(
            load_plan(dir.path())
                .expect("inspect rejected plan")
                .is_none(),
            "foreign authority must be rejected before plan persistence"
        );
        assert_eq!(
            verification_artifact_bytes(dir.path()),
            artifacts_before,
            "foreign authority denial must not mutate verification evidence"
        );
    }

    #[test]
    fn generation_scoped_verify_run_rejects_foreign_session_before_command_dispatch() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("generation-scoped run authority repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        let current =
            initialize_generation_scoped_execution(dir.path(), "session-current-run-authority");
        persist_generation_scoped_session(
            dir.path(),
            "session-current-run-authority",
            current.clone(),
            1,
        );
        persist_generation_scoped_session(
            dir.path(),
            "session-foreign-run-authority-secret",
            current,
            1,
        );

        let marker = dir.path().join("foreign-command-ran");
        let artifacts_before = verification_artifact_bytes(dir.path());
        let error = run_verify_cli_as(
            dir.path(),
            "session-foreign-run-authority-secret",
            VerifyCommand::Run {
                commands: vec![format!("touch {}", marker.display())],
            },
        )
        .expect_err("foreign Session must be rejected before command dispatch");

        assert_verification_authority_denial(&error, "session-foreign-run-authority-secret");
        assert!(
            !marker.exists(),
            "foreign verification command must never be dispatched"
        );
        assert_eq!(
            verification_artifact_bytes(dir.path()),
            artifacts_before,
            "foreign run denial must not mutate verification evidence"
        );
    }

    #[test]
    fn generation_scoped_verify_rejects_missing_malformed_and_stale_durable_session() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("invalid durable Session repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let session_id = "session-invalid-authority-secret";
        let current = initialize_generation_scoped_execution(dir.path(), session_id);

        let missing_before = verification_artifact_bytes(dir.path());
        let missing_error = run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Plan {
                commands: vec!["git --version".to_string()],
                derive: false,
            },
        )
        .expect_err("missing durable Session must fail closed");
        assert_verification_authority_denial(&missing_error, session_id);
        assert_eq!(
            verification_artifact_bytes(dir.path()),
            missing_before,
            "missing Session denial must be zero-write"
        );

        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        fs::create_dir_all(&sessions_dir).expect("create Session directory");
        fs::write(
            sessions_dir.join(format!("{session_id}.toml")),
            "this is not valid = [toml",
        )
        .expect("persist malformed Session");
        let malformed_before = verification_artifact_bytes(dir.path());
        let malformed_error = run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Plan {
                commands: vec!["git --version".to_string()],
                derive: false,
            },
        )
        .expect_err("malformed durable Session must fail closed");
        assert_verification_authority_denial(&malformed_error, session_id);
        assert_eq!(
            verification_artifact_bytes(dir.path()),
            malformed_before,
            "malformed Session denial must be zero-write"
        );

        let mut stale_identity = current;
        stale_identity.ledger_head_hash = "stale-ledger-head-secret".to_string();
        persist_generation_scoped_session(dir.path(), session_id, stale_identity, 1);
        let stale_before = verification_artifact_bytes(dir.path());
        let stale_error = run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Plan {
                commands: vec!["git --version".to_string()],
                derive: false,
            },
        )
        .expect_err("stale durable Session binding must fail closed");
        assert_verification_authority_denial(&stale_error, "stale-ledger-head-secret");
        assert_eq!(
            verification_artifact_bytes(dir.path()),
            stale_before,
            "stale Session denial must be zero-write"
        );
    }

    #[test]
    fn completed_generation_rejects_exact_terminal_session_without_writes_or_dispatch() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("completed generation repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_scoped_owner();
        let session_id = "session-completed-authority-secret";

        let active = initialize_generation_scoped_execution(dir.path(), session_id);
        persist_generation_scoped_session(dir.path(), session_id, active, 1);
        assert!(matches!(
            crate::cli::execution_state::settle(
                dir.path(),
                session_id,
                crate::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("settle completed generation"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        let terminal = crate::cli::execution_state::current_execution_binding(dir.path(), owner)
            .expect("load completed generation binding")
            .expect("completed generation binding exists");
        advance_generation_scoped_session_binding(session_id, terminal);

        let artifacts_before = verification_artifact_bytes(dir.path());
        let plan_error = run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Plan {
                commands: vec!["git --version".to_string()],
                derive: false,
            },
        )
        .expect_err("Completed generation must not register evidence");
        assert_verification_authority_denial(&plan_error, session_id);

        let marker = dir.path().join("completed-command-ran");
        let run_error = run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Run {
                commands: vec![format!("touch {}", marker.display())],
            },
        )
        .expect_err("Completed generation must not dispatch verification commands");
        assert_verification_authority_denial(&run_error, session_id);
        assert!(
            !marker.exists(),
            "Completed generation must be rejected before command dispatch"
        );
        assert_eq!(
            verification_artifact_bytes(dir.path()),
            artifacts_before,
            "terminal authority denial must not mutate verification evidence"
        );
    }

    #[test]
    fn blocked_generation_accepts_exact_same_session_for_recovery_evidence() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("blocked generation recovery repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_scoped_owner();
        let session_id = "session-blocked-recovery-authority";

        let active = initialize_generation_scoped_execution(dir.path(), session_id);
        persist_generation_scoped_session(dir.path(), session_id, active, 1);
        assert!(matches!(
            crate::cli::execution_state::settle(
                dir.path(),
                session_id,
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "verification evidence required".to_string(),
                    missing_verification: Some("verify.run".to_string()),
                },
            )
            .expect("settle blocked generation"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        let blocked = crate::cli::execution_state::current_execution_binding(dir.path(), owner)
            .expect("load blocked generation binding")
            .expect("blocked generation binding exists");
        advance_generation_scoped_session_binding(session_id, blocked.clone());
        let commands = vec!["git --version".to_string()];

        let (plan_code, _) = run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Plan {
                commands: commands.clone(),
                derive: false,
            },
        )
        .expect("exact Blocked owner Session may register recovery plan");
        assert_eq!(plan_code, 0);
        let (run_code, _) = run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Run {
                commands: commands.clone(),
            },
        )
        .expect("exact Blocked owner Session may produce recovery evidence");
        assert_eq!(run_code, 0);

        let plan = load_plan(dir.path())
            .expect("load Blocked recovery plan")
            .expect("Blocked recovery plan exists");
        let record = load(dir.path())
            .expect("load Blocked recovery run")
            .expect("Blocked recovery run exists");
        assert_eq!(plan.execution_binding.as_ref(), Some(&blocked));
        assert_eq!(record.execution_binding.as_ref(), Some(&blocked));
        assert_eq!(
            evaluate_evidence(dir.path(), session_id, Some(owner.number)),
            EvidenceStatus::Fresh
        );
    }

    #[test]
    fn ledgerless_execution_preserves_legacy_verify_cli_compatibility() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("ledgerless compatibility repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_scoped_owner();
        let session_id = "session-ledgerless-legacy";
        crate::cli::execution_state::materialize_at_launch(
            dir.path(),
            owner.kind,
            owner.number,
            session_id,
            "gwt-execute",
            false,
        )
        .expect("materialize ledgerless legacy execution");
        let commands = vec!["git --version".to_string()];

        assert_eq!(
            run_verify_cli_as(
                dir.path(),
                session_id,
                VerifyCommand::Plan {
                    commands: commands.clone(),
                    derive: false,
                },
            )
            .expect("ledgerless verify.plan remains compatible")
            .0,
            0
        );
        assert_eq!(
            run_verify_cli_as(
                dir.path(),
                session_id,
                VerifyCommand::Run {
                    commands: commands.clone(),
                },
            )
            .expect("ledgerless verify.run remains compatible")
            .0,
            0
        );
        let plan = load_plan(dir.path())
            .expect("load legacy plan")
            .expect("legacy plan exists");
        let record = load(dir.path())
            .expect("load legacy run")
            .expect("legacy run exists");
        assert_eq!(plan.owner_number, Some(owner.number));
        assert_eq!(record.owner_number, Some(owner.number));
        assert_eq!(plan.execution_binding, None);
        assert_eq!(record.execution_binding, None);
    }

    #[test]
    fn verify_plan_rejects_capability_rotation_after_dispatch_before_commit() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("verify.plan capability race repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let session_id = "session-plan-capability-race";
        let current = initialize_generation_scoped_execution(dir.path(), session_id);
        persist_generation_scoped_session(dir.path(), session_id, current.clone(), 1);
        let artifacts_before = verification_artifact_bytes(dir.path());
        let session_for_hook = session_id.to_string();
        crate::cli::trusted_store::set_write_lease_acquired_hook(move || {
            advance_generation_scoped_session_binding(&session_for_hook, current);
        });

        let error = run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Plan {
                commands: vec!["git --version".to_string()],
                derive: false,
            },
        )
        .expect_err("capability rotation before plan commit must fail closed");
        assert_verification_authority_denial(&error, session_id);
        assert_eq!(
            verification_artifact_bytes(dir.path()),
            artifacts_before,
            "capability race must leave plan/run evidence byte-equivalent"
        );
    }

    #[test]
    fn verify_run_rejects_capability_rotation_before_command_dispatch() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("verify.run dispatch race repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let session_id = "session-run-dispatch-capability-race";
        let current = initialize_generation_scoped_execution(dir.path(), session_id);
        persist_generation_scoped_session(dir.path(), session_id, current.clone(), 1);
        run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Plan {
                commands: vec!["git --version".to_string()],
                derive: false,
            },
        )
        .expect("register plan before dispatch race");
        let artifacts_before = verification_artifact_bytes(dir.path());
        let marker = dir.path().join("capability-race-command-ran");
        let session_for_hook = session_id.to_string();
        crate::cli::trusted_store::set_write_lease_acquired_hook(move || {
            advance_generation_scoped_session_binding(&session_for_hook, current);
        });

        let error = run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Run {
                commands: vec![format!("touch {}", marker.display())],
            },
        )
        .expect_err("capability rotation before dispatch must fail closed");
        assert_verification_authority_denial(&error, session_id);
        assert!(
            !marker.exists(),
            "revoked capability must be rejected before command dispatch"
        );
        assert_eq!(
            verification_artifact_bytes(dir.path()),
            artifacts_before,
            "dispatch denial must leave verification evidence byte-equivalent"
        );
    }

    #[test]
    fn verify_run_rejects_capability_rotation_after_commands_before_commit() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("verify.run capability race repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let session_id = "session-run-capability-race";
        let current = initialize_generation_scoped_execution(dir.path(), session_id);
        persist_generation_scoped_session(dir.path(), session_id, current.clone(), 1);
        let commands = vec!["git --version".to_string()];
        run_verify_cli_as(
            dir.path(),
            session_id,
            VerifyCommand::Plan {
                commands: commands.clone(),
                derive: false,
            },
        )
        .expect("register plan before run race");
        let authority = snapshot_verification_caller_authority(dir.path(), session_id)
            .expect("snapshot verification caller before run");
        let artifacts_before = verification_artifact_bytes(dir.path());
        let session_for_hook = session_id.to_string();

        let error = run_verification_inner(
            dir.path(),
            session_id,
            &commands,
            Some(&authority),
            move || {
                advance_generation_scoped_session_binding(&session_for_hook, current);
            },
        )
        .expect_err("capability rotation before run commit must fail closed");
        assert!(
            error.contains("current verification authority"),
            "run commit denial must be actionable: {error}"
        );
        assert!(
            !error.contains(session_id),
            "run commit denial must not reflect caller identity: {error}"
        );
        assert_eq!(
            verification_artifact_bytes(dir.path()),
            artifacts_before,
            "capability race must leave plan/run evidence byte-equivalent"
        );
    }

    #[test]
    fn generation_scoped_verification_plan_and_run_bind_exact_execution_identity() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("generation-scoped verification repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        let expected = initialize_generation_scoped_execution(dir.path(), "session-generation-a");
        let commands = vec!["git --version".to_string()];
        let (run, _) = plan_and_run(dir.path(), "session-generation-a", &commands);
        let plan = load_plan(dir.path())
            .expect("load generation-scoped plan")
            .expect("generation-scoped plan exists");

        assert_eq!(plan.execution_binding.as_ref(), Some(&expected));
        assert_eq!(run.execution_binding.as_ref(), Some(&expected));
        assert_eq!(
            evaluate_evidence(dir.path(), "session-generation-a", Some(2359)),
            EvidenceStatus::Fresh
        );
    }

    #[test]
    fn generation_scoped_successor_rejects_predecessor_verification_evidence() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("generation-scoped predecessor repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_scoped_owner();

        let predecessor = initialize_generation_scoped_execution(dir.path(), "session-predecessor");
        let commands = vec!["git --version".to_string()];
        let (run, _) = plan_and_run(dir.path(), "session-predecessor", &commands);
        assert_eq!(run.execution_binding.as_ref(), Some(&predecessor));
        assert!(matches!(
            crate::cli::execution_state::settle(
                dir.path(),
                "session-predecessor",
                crate::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("settle predecessor generation"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));

        let request =
            generation_scoped_successor_request("successor-verification", "session-successor");
        crate::cli::execution_state::prepare_successor(dir.path(), owner, &request)
            .expect("prepare successor generation");
        crate::cli::execution_state::activate_successor(dir.path(), owner, &request)
            .expect("activate successor generation");
        let successor = crate::cli::execution_state::current_execution_binding(dir.path(), owner)
            .expect("load successor binding")
            .expect("successor binding exists");
        assert_ne!(successor, predecessor);

        assert_eq!(
            evaluate_evidence(dir.path(), "session-predecessor", Some(2359)),
            EvidenceStatus::WrongGeneration,
            "same owner/session/fingerprint predecessor evidence must not satisfy the successor"
        );
    }

    #[test]
    fn generation_scoped_legacy_evidence_becomes_audit_only_after_ledger_import() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().expect("legacy evidence repository");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let owner = generation_scoped_owner();

        crate::cli::execution_state::materialize_at_launch(
            dir.path(),
            owner.kind,
            owner.number,
            "session-legacy-evidence",
            "gwt-execute",
            false,
        )
        .expect("materialize legacy execution");
        let commands = vec!["git --version".to_string()];
        let (legacy_run, _) = plan_and_run(dir.path(), "session-legacy-evidence", &commands);
        assert!(
            legacy_run.execution_binding.is_none(),
            "pre-ledger evidence remains readable as legacy audit history"
        );
        assert_eq!(
            evaluate_evidence(dir.path(), "session-legacy-evidence", Some(2359)),
            EvidenceStatus::Fresh
        );
        assert!(matches!(
            crate::cli::execution_state::settle(
                dir.path(),
                "session-legacy-evidence",
                crate::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("settle legacy execution"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        crate::cli::execution_state::ensure_generation_ledger(
            dir.path(),
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Unknown,
        )
        .expect("import verified legacy terminal generation");

        assert_eq!(
            evaluate_evidence(dir.path(), "session-legacy-evidence", Some(2359)),
            EvidenceStatus::WrongGeneration,
            "legacy unbound evidence must not authorize a generation-aware completion gate"
        );
    }

    #[test]
    fn generation_scoped_work_settlement_rejects_predecessor_receipt() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = WorkEventGitFixture::tracked();
        let owner = generation_scoped_owner();

        initialize_generation_scoped_execution(&fixture.repo, "session-settlement-predecessor");
        assert!(matches!(
            crate::cli::execution_state::settle(
                &fixture.repo,
                "session-settlement-predecessor",
                crate::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("settle predecessor before its delivery receipt"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        let predecessor =
            crate::cli::execution_state::current_execution_binding(&fixture.repo, owner)
                .expect("load terminal predecessor binding")
                .expect("terminal predecessor binding exists");
        let receipt = save_work_event_settlement_record(
            &fixture.repo,
            "session-settlement-predecessor",
            true,
        )
        .expect("persist predecessor Work settlement");
        assert!(receipt.status.is_settled());
        assert_eq!(receipt.execution_binding.as_ref(), Some(&predecessor));

        let request =
            generation_scoped_successor_request("successor-settlement", "session-settlement-next");
        crate::cli::execution_state::prepare_successor(&fixture.repo, owner, &request)
            .expect("prepare settlement successor");
        crate::cli::execution_state::activate_successor(&fixture.repo, owner, &request)
            .expect("activate settlement successor");

        let refusal = work_event_settlement_refusal(&fixture.repo)
            .expect("predecessor settlement must not authorize successor completion");
        assert!(
            refusal.contains("generation") && refusal.contains("predecessor"),
            "generation mismatch must be actionable without leaking secrets: {refusal}"
        );
    }

    fn assert_path_dirty(fixture: &WorkEventGitFixture, states: Vec<WorkEventPathState>) {
        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::PathDirty { states })
        );
    }

    #[test]
    fn work_event_settlement_rejects_every_exact_path_dirty_state() {
        let staged = WorkEventGitFixture::tracked();
        staged.append_event("staged");
        staged.stage_events();
        assert_path_dirty(&staged, vec![WorkEventPathState::Staged]);

        let unstaged = WorkEventGitFixture::tracked();
        unstaged.append_event("unstaged");
        assert_path_dirty(&unstaged, vec![WorkEventPathState::Unstaged]);

        let untracked = WorkEventGitFixture::without_tracked_event_log();
        untracked.write_event_log("untracked");
        assert_path_dirty(&untracked, vec![WorkEventPathState::Untracked]);

        let deleted = WorkEventGitFixture::tracked();
        fs::remove_file(deleted.event_path()).expect("delete tracked Work event log");
        assert_path_dirty(&deleted, vec![WorkEventPathState::Deleted]);
    }

    #[test]
    fn work_event_settlement_sorts_and_deduplicates_multiple_path_states() {
        let fixture = WorkEventGitFixture::tracked();
        fixture.append_event("staged-version");
        fixture.stage_events();
        fixture.append_event("unstaged-version");

        assert_path_dirty(
            &fixture,
            vec![WorkEventPathState::Staged, WorkEventPathState::Unstaged],
        );
    }

    #[test]
    fn work_event_settlement_rejects_commit_that_has_not_reached_upstream() {
        let fixture = WorkEventGitFixture::tracked();
        fixture.append_event("committed-not-pushed");
        fixture.stage_events();
        fixture.commit("chore(work): commit final Work event");

        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::CommitNotPushed)
        );
    }

    #[test]
    fn work_event_settlement_requires_head_containment_after_event_commit() {
        let fixture = WorkEventGitFixture::tracked();
        fixture.append_event("pushed-event");
        fixture.stage_events();
        fixture.commit("chore(work): push final Work event");
        fixture.push();

        fs::write(fixture.repo.join("src.txt"), "local source after event\n")
            .expect("write source-only local change");
        fixture.git_ok(&["add", "--", "src.txt"]);
        fixture.commit("fix: source after final Work event");
        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::CommitNotPushed),
            "a pushed event commit cannot settle a later unpushed HEAD"
        );

        fixture.advance_remote_from_peer();
        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::RemoteDiverged),
            "remote divergence after the event receipt must also block settlement"
        );
    }

    #[test]
    fn work_event_settlement_accepts_clean_commit_after_remote_readback() {
        let fixture = WorkEventGitFixture::tracked();
        fixture.append_event("committed-and-pushed");
        fixture.stage_events();
        fixture.commit("chore(work): deliver final Work event");
        fixture.push();

        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            fixture.settled_status()
        );
    }

    #[test]
    fn work_event_settlement_rejects_missing_configured_upstream() {
        let fixture = WorkEventGitFixture::tracked();
        fixture.git_ok(&["branch", "--unset-upstream"]);

        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::MissingUpstream)
        );

        let detached = WorkEventGitFixture::tracked();
        detached.git_ok(&["checkout", "--detach", "-q"]);
        assert_eq!(
            evaluate_work_event_settlement(&detached.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::MissingUpstream)
        );
    }

    #[test]
    fn work_event_settlement_distinguishes_remote_divergence_from_unpushed_commit() {
        let fixture = WorkEventGitFixture::tracked();
        fixture.append_event("local-divergent-event");
        fs::write(fixture.repo.join("src.txt"), "local divergent source\n")
            .expect("write local divergent source");
        fixture.git_ok(&["add", "--", WORK_EVENTS_PATH, "src.txt"]);
        fixture.commit("fix: local source and Work event");
        fixture.advance_remote_from_peer();
        fixture.git_ok(&["fetch", "-q", "origin", "main"]);

        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::RemoteDiverged)
        );
    }

    #[test]
    fn work_event_settlement_fails_closed_when_git_status_is_unreadable() {
        let fixture = WorkEventGitFixture::tracked();
        fs::write(fixture.repo.join(".git/index"), b"corrupt-index")
            .expect("corrupt fixture index");

        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::GitStatusError)
        );
    }

    #[test]
    fn work_event_settlement_fails_closed_when_remote_readback_fails() {
        let fixture = WorkEventGitFixture::tracked();
        let missing_remote = fixture._root.path().join("missing-upstream.git");
        fixture.git_ok(&[
            "remote",
            "set-url",
            "origin",
            missing_remote.to_str().expect("UTF-8 missing remote path"),
        ]);

        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::RemoteReadbackError)
        );
    }

    #[test]
    fn work_event_settlement_commit_policy_distinguishes_source_and_bookkeeping_only_changes() {
        let with_source = WorkEventGitFixture::tracked();
        with_source.append_event("with-source");
        fs::write(with_source.repo.join("src.txt"), "implemented behavior\n")
            .expect("write changed source");
        with_source.git_ok(&["add", "--", WORK_EVENTS_PATH, "src.txt"]);
        with_source.commit("fix: implement behavior with Work event");
        with_source.push();
        assert_eq!(
            evaluate_work_event_settlement(&with_source.repo),
            with_source.settled_status(),
            "a normal commit may carry the Work event when it also contains a source change"
        );

        let work_only_chore = WorkEventGitFixture::tracked();
        work_only_chore.append_event("work-only-chore");
        work_only_chore.stage_events();
        work_only_chore.commit("chore(work): repair final Work event");
        work_only_chore.push();
        assert_eq!(
            evaluate_work_event_settlement(&work_only_chore.repo),
            work_only_chore.settled_status(),
            "an exact chore(work): prefix is the explicit no-source exception"
        );

        let invalid_work_only = WorkEventGitFixture::tracked();
        invalid_work_only.append_event("work-only-invalid");
        invalid_work_only.stage_events();
        invalid_work_only.commit("fix: claim source delivery without source");
        invalid_work_only.push();
        assert_eq!(
            evaluate_work_event_settlement(&invalid_work_only.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::InvalidWorkOnlyCommit),
            "an event-only commit without exact chore(work): prefix must fail closed"
        );
    }

    #[test]
    fn work_event_settlement_accepts_pushed_merge_commit_with_event_and_source_changes() {
        let fixture = WorkEventGitFixture::tracked();
        fixture.git_ok(&["checkout", "-qb", "settlement-feature"]);
        fixture.append_event("feature-terminal-event");
        fs::write(fixture.repo.join("src.txt"), "feature source delivery\n")
            .expect("write feature source change");
        fixture.git_ok(&["add", "--", WORK_EVENTS_PATH, "src.txt"]);
        fixture.commit("fix: implement feature with Work event");

        fixture.git_ok(&["checkout", "-q", "main"]);
        fixture.append_event("main-side-event");
        fixture.stage_events();
        fixture.commit("chore(work): record concurrent main Work event");

        let merge = fixture.git_output(&["merge", "--no-ff", "--no-commit", "settlement-feature"]);
        assert!(
            !merge.status.success(),
            "the fixture must require an explicit event-log merge resolution"
        );
        fs::write(
            fixture.event_path(),
            concat!(
                "{\"id\":\"base\"}\n",
                "{\"id\":\"main-side-event\"}\n",
                "{\"id\":\"feature-terminal-event\"}\n",
                "{\"id\":\"merge-resolution-event\"}\n",
            ),
        )
        .expect("resolve merged Work event log");
        fixture.git_ok(&["add", "--", WORK_EVENTS_PATH, "src.txt"]);
        fixture.commit("fix: merge source and Work event delivery");
        let merge_commit = fixture.git_stdout(&["rev-parse", "HEAD"]);
        assert_eq!(
            fixture.latest_event_commit(),
            merge_commit,
            "the path-limited event commit must be the merge itself"
        );
        fixture.push();

        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            fixture.settled_status(),
            "a pushed merge carrying both source and the Work event must satisfy commit policy"
        );
    }

    #[test]
    fn work_event_settlement_rejects_bookkeeping_only_merge_when_other_parent_contains_source() {
        let fixture = WorkEventGitFixture::tracked();

        fixture.git_ok(&["checkout", "-qb", "settlement-topic"]);
        fixture.append_event("topic-terminal-event");
        fixture.stage_events();
        fixture.commit("chore(work): record topic Work event");

        fixture.git_ok(&["checkout", "-q", "main"]);
        fixture.append_event("main-side-event");
        fs::write(fixture.repo.join("src.txt"), "main source delivery\n")
            .expect("write main-side source change");
        fixture.git_ok(&["add", "--", WORK_EVENTS_PATH, "src.txt"]);
        fixture.commit("fix: advance main source and Work event");

        let merge = fixture.git_output(&["merge", "--no-ff", "--no-commit", "settlement-topic"]);
        assert!(
            !merge.status.success(),
            "the fixture must require an explicit event-log merge resolution"
        );
        fs::write(
            fixture.event_path(),
            concat!(
                "{\"id\":\"base\"}\n",
                "{\"id\":\"main-side-event\"}\n",
                "{\"id\":\"topic-terminal-event\"}\n",
                "{\"id\":\"merge-resolution-event\"}\n",
            ),
        )
        .expect("resolve merged Work event log");
        fixture.git_ok(&["add", "--", WORK_EVENTS_PATH]);
        fixture.commit("fix: merge Work event delivery");
        assert_eq!(
            fixture.latest_event_commit(),
            fixture.git_stdout(&["rev-parse", "HEAD"]),
            "the path-limited event commit must be the merge itself"
        );
        fixture.push();

        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::InvalidWorkOnlyCommit),
            "source inherited from another parent must not satisfy the merge commit policy"
        );
    }

    #[test]
    fn work_event_settlement_checks_only_the_exact_tracked_event_path_for_dirtiness() {
        let fixture = WorkEventGitFixture::tracked();
        fs::write(
            fixture.repo.join("unrelated-source.txt"),
            "untracked source\n",
        )
        .expect("write unrelated dirty path");
        fs::write(fixture.repo.join(".gwt/work/other.json"), "{}\n")
            .expect("write unrelated bookkeeping path");

        assert_eq!(
            evaluate_work_event_settlement(&fixture.repo),
            fixture.settled_status(),
            "unrelated dirtiness must not be misreported as exact Work event path dirtiness"
        );
    }

    #[test]
    fn work_event_settlement_record_is_mirrorless_and_sticky_until_settled() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = WorkEventGitFixture::tracked();
        fixture.append_event("pending-terminal-update");
        let fingerprint_before = worktree_fingerprint(&fixture.repo);

        let opened = save_work_event_settlement_record(&fixture.repo, "session-a", true)
            .expect("persist open settlement obligation");
        assert!(opened.obligation_open);
        assert_eq!(
            opened.status,
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::PathDirty {
                states: vec![WorkEventPathState::Unstaged],
            })
        );
        assert_eq!(
            load_work_event_settlement_record(&fixture.repo)
                .expect("read settlement record")
                .expect("settlement record exists"),
            opened
        );
        assert_eq!(
            worktree_fingerprint(&fixture.repo),
            fingerprint_before,
            "machine-local settlement state must not alter verification freshness"
        );

        let duplicate_open = save_work_event_settlement_record(&fixture.repo, "session-b", true)
            .expect("record duplicate settlement obligation");
        assert!(duplicate_open.obligation_open);
        assert_eq!(
            duplicate_open.session_id, "session-a",
            "a duplicate open must preserve the originating session provenance"
        );

        let refreshed = save_work_event_settlement_record(&fixture.repo, "session-c", false)
            .expect("refresh blocked settlement obligation");
        assert!(
            refreshed.obligation_open,
            "an existing obligation cannot auto-close while settlement is blocked"
        );
        assert_eq!(
            refreshed.session_id, "session-a",
            "a foreign-session refresh must not replace the originating session provenance"
        );

        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(&fixture.repo)
            .expect("fixture has a repo-scoped trusted store");
        assert!(trusted_dir
            .join(WORK_EVENT_SETTLEMENT_RECORD_FILE)
            .is_file());
        assert!(
            !fixture
                .repo
                .join(".gwt/skill-state")
                .join(WORK_EVENT_SETTLEMENT_RECORD_FILE)
                .exists(),
            "settlement obligations must never be mirrored into the worktree"
        );

        fixture.stage_events();
        fixture.commit("chore(work): settle terminal update");
        fixture.push();
        let settled = save_work_event_settlement_record(&fixture.repo, "session-d", false)
            .expect("persist settled status");
        assert!(!settled.obligation_open);
        assert_eq!(
            settled.session_id, "session-a",
            "the settled receipt must retain the obligation's originating session"
        );
        assert_eq!(settled.status, fixture.settled_status());
        assert_eq!(
            load_work_event_settlement_record(&fixture.repo)
                .expect("read settled record")
                .expect("settled record exists"),
            settled
        );

        let foreign_refresh = save_work_event_settlement_record(&fixture.repo, "session-e", false)
            .expect("refresh settled receipt from another session");
        assert!(!foreign_refresh.obligation_open);
        assert_eq!(
            foreign_refresh.session_id, "session-a",
            "ordinary refreshes must retain the settled generation's author provenance"
        );

        fixture.append_event("next-terminal-update");
        let next_generation = save_work_event_settlement_record(&fixture.repo, "session-e", true)
            .expect("open the next settlement generation");
        assert!(next_generation.obligation_open);
        assert_eq!(
            next_generation.session_id, "session-e",
            "an explicit new obligation must establish a new author generation"
        );
        assert_eq!(
            next_generation.status,
            WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::PathDirty {
                states: vec![WorkEventPathState::Unstaged],
            })
        );

        let duplicate_next_generation =
            save_work_event_settlement_record(&fixture.repo, "session-f", true)
                .expect("record duplicate obligation in the next generation");
        assert_eq!(
            duplicate_next_generation.session_id, "session-e",
            "duplicate opens must preserve the current generation's author provenance"
        );
    }

    #[test]
    fn pending_work_event_settlement_stays_open_until_the_exact_event_is_persisted_and_pushed() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = WorkEventGitFixture::tracked();
        let updated_at = Utc::now();
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            "work-pending-terminal",
            updated_at,
        );
        event.agent_session_id = Some("session-pending-terminal".to_string());
        let journal_entry = gwt_core::workspace_projection::WorkspaceJournalEntry {
            id: "journal-pending-terminal".to_string(),
            project_root: fixture.repo.clone(),
            title: None,
            status_category: Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done),
            status_text: None,
            owner: None,
            next_action: None,
            summary: None,
            progress_summary: None,
            agent_session_id: Some("session-pending-terminal".to_string()),
            agent_current_focus: None,
            agent_title_summary: None,
            updated_at,
        };

        let prepared = prepare_work_event_settlement_record(
            &fixture.repo,
            "session-pending-terminal",
            &event,
            &journal_entry,
        )
        .expect("prepare write-ahead settlement receipt");
        assert!(prepared.obligation_open);
        assert_eq!(
            prepared.status,
            WorkEventSettlementStatus::PendingMutation {
                event_id: event.id.clone(),
                work_id: event.work_item_id.clone(),
                journal_entry_id: journal_entry.id.clone(),
                event_session_id: "session-pending-terminal".to_string(),
            }
        );

        let pre_persist_refresh =
            save_work_event_settlement_record(&fixture.repo, "session-pending-terminal", false)
                .expect("refresh before event persistence");
        assert_eq!(pre_persist_refresh.status, prepared.status);
        assert!(pre_persist_refresh.obligation_open);
        assert!(
            work_event_settlement_refusal(&fixture.repo)
                .is_some_and(|message| message.contains("not been persisted")),
            "Stop refresh must fail closed while the prepared event is absent"
        );

        fixture.append_typed_event(&event);
        assert!(
            work_event_settlement_refusal(&fixture.repo)
                .is_some_and(|message| message.contains("dirty")),
            "the pending receipt must transition to ordinary path settlement without a post-write refresh"
        );

        fixture.stage_events();
        fixture.commit("chore(work): settle prepared terminal event");
        fixture.push();
        assert_eq!(work_event_settlement_refusal(&fixture.repo), None);
        let settled = load_work_event_settlement_record(&fixture.repo)
            .expect("load settled receipt")
            .expect("settlement receipt exists");
        assert!(!settled.obligation_open);
        assert!(matches!(
            settled.status,
            WorkEventSettlementStatus::Settled { .. }
        ));
    }

    #[test]
    fn work_event_settlement_reader_accepts_v1_v2_and_refreshes_to_v3() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = WorkEventGitFixture::tracked();
        let legacy = WorkEventSettlementRecord {
            schema_version: WORK_EVENT_SETTLEMENT_LEGACY_SCHEMA_VERSION,
            session_id: "session-legacy".to_string(),
            execution_binding: None,
            obligation_open: false,
            status: fixture.settled_status(),
            updated_at: Utc::now(),
        };
        persist_work_event_settlement_record(&fixture.repo, &legacy)
            .expect("seed v1 settlement receipt");

        assert_eq!(
            load_work_event_settlement_record(&fixture.repo)
                .expect("load v1 receipt")
                .expect("v1 receipt exists"),
            legacy
        );
        let mut pre_generation = legacy.clone();
        pre_generation.schema_version = WORK_EVENT_SETTLEMENT_PRE_GENERATION_SCHEMA_VERSION;
        persist_work_event_settlement_record(&fixture.repo, &pre_generation)
            .expect("seed v2 settlement receipt");
        assert_eq!(
            load_work_event_settlement_record(&fixture.repo)
                .expect("load v2 receipt")
                .expect("v2 receipt exists"),
            pre_generation
        );
        let refreshed = save_work_event_settlement_record(&fixture.repo, "session-legacy", false)
            .expect("refresh v2 receipt");
        assert_eq!(
            refreshed.schema_version,
            WORK_EVENT_SETTLEMENT_SCHEMA_VERSION
        );
        assert_eq!(refreshed.status, fixture.settled_status());
    }

    #[test]
    fn work_event_settlement_record_refuses_worktree_mirror_fallback() {
        let repo = tempfile::tempdir().expect("repository without trusted-store identity");
        let init = gwt_core::process::hidden_command("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .output()
            .expect("initialize repository without origin");
        assert!(init.status.success());

        let error = save_work_event_settlement_record(repo.path(), "session-a", true)
            .expect_err("mirrorless state must not claim persistence without a trusted store");
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert!(
            !repo.path().join(".gwt/skill-state").exists(),
            "the mirrorless API must not fall back to worktree state"
        );
    }

    #[test]
    fn work_event_settlement_persistence_fails_closed_if_trusted_dir_disappears() {
        let repo = tempfile::tempdir().expect("repository without trusted-store identity");
        let init = gwt_core::process::hidden_command("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .output()
            .expect("initialize repository without origin");
        assert!(init.status.success());
        assert!(
            crate::cli::trusted_store::trusted_dir_for_worktree(repo.path()).is_none(),
            "fixture must model identity disappearing after the outer guard"
        );
        let record = WorkEventSettlementRecord {
            schema_version: WORK_EVENT_SETTLEMENT_SCHEMA_VERSION,
            session_id: "session-required-trusted-write".to_string(),
            execution_binding: None,
            obligation_open: true,
            status: WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::GitStatusError),
            updated_at: Utc::now(),
        };

        let error = persist_work_event_settlement_record(repo.path(), &record)
            .expect_err("settlement persistence must never silently no-op");
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert!(
            !repo.path().join(".gwt/skill-state").exists(),
            "required settlement persistence must not create a mirror fallback"
        );
    }

    #[test]
    fn work_event_settlement_fails_closed_when_trusted_dir_changes_after_lease() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = tempfile::tempdir().expect("repository");
        crate::cli::trusted_store::init_git_repo_with_origin(repo.path());

        let trusted_a = crate::cli::trusted_store::trusted_dir_for_worktree(repo.path())
            .expect("initial trusted directory");
        let initial_record = WorkEventSettlementRecord {
            schema_version: WORK_EVENT_SETTLEMENT_SCHEMA_VERSION,
            session_id: "session-from-leased-directory".to_string(),
            execution_binding: None,
            obligation_open: false,
            status: WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::GitStatusError),
            updated_at: Utc::now(),
        };
        let initial_bytes =
            serde_json::to_vec_pretty(&initial_record).expect("serialize initial settlement");
        gwt_github::cache::write_atomic(
            &trusted_a.join(WORK_EVENT_SETTLEMENT_RECORD_FILE),
            &initial_bytes,
        )
        .expect("seed settlement beneath the directory that will be leased");
        let repo_for_hook = repo.path().to_path_buf();
        crate::cli::trusted_store::set_write_lease_acquired_hook(move || {
            let output = gwt_core::process::hidden_command("git")
                .args([
                    "remote",
                    "set-url",
                    "origin",
                    "https://example.com/t/changed-trusted-store.git",
                ])
                .current_dir(&repo_for_hook)
                .output()
                .expect("change repository identity after lease acquisition");
            assert!(output.status.success());
        });

        let error =
            save_work_event_settlement_record(repo.path(), "session-stable-trusted-dir", false)
                .expect_err("trusted-store identity drift must fail closed");

        let trusted_b = crate::cli::trusted_store::trusted_dir_for_worktree(repo.path())
            .expect("changed trusted directory");
        assert_ne!(
            trusted_a, trusted_b,
            "fixture must change the resolved trusted directory"
        );
        let persisted_a = fs::read_to_string(trusted_a.join(WORK_EVENT_SETTLEMENT_RECORD_FILE))
            .expect("read settlement from leased directory");
        assert_eq!(
            persisted_a,
            String::from_utf8(initial_bytes).expect("initial settlement UTF-8"),
            "fail-closed drift handling must leave the leased directory byte-equivalent"
        );
        assert!(
            error.to_string().contains("trusted store identity changed"),
            "drift rejection must be actionable: {error}"
        );
        assert!(
            !trusted_b.join(WORK_EVENT_SETTLEMENT_RECORD_FILE).exists(),
            "a resolver change must not redirect persistence to an unlocked directory"
        );
    }

    #[test]
    fn work_event_settlement_fails_closed_when_trusted_dir_disappears_after_lease() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = tempfile::tempdir().expect("repository");
        crate::cli::trusted_store::init_git_repo_with_origin(repo.path());

        let trusted = crate::cli::trusted_store::trusted_dir_for_worktree(repo.path())
            .expect("initial trusted directory");
        let initial_record = WorkEventSettlementRecord {
            schema_version: WORK_EVENT_SETTLEMENT_SCHEMA_VERSION,
            session_id: "session-before-identity-loss".to_string(),
            execution_binding: None,
            obligation_open: false,
            status: WorkEventSettlementStatus::Blocked(WorkEventSettlementBlocker::GitStatusError),
            updated_at: Utc::now(),
        };
        let initial_bytes =
            serde_json::to_vec_pretty(&initial_record).expect("serialize initial settlement");
        gwt_github::cache::write_atomic(
            &trusted.join(WORK_EVENT_SETTLEMENT_RECORD_FILE),
            &initial_bytes,
        )
        .expect("seed settlement beneath the directory that will be leased");
        let repo_for_hook = repo.path().to_path_buf();
        crate::cli::trusted_store::set_write_lease_acquired_hook(move || {
            let output = gwt_core::process::hidden_command("git")
                .args(["remote", "remove", "origin"])
                .current_dir(&repo_for_hook)
                .output()
                .expect("remove repository identity after lease acquisition");
            assert!(output.status.success());
        });

        let error =
            save_work_event_settlement_record(repo.path(), "session-after-identity-loss", false)
                .expect_err("trusted-store identity loss must fail closed");

        assert!(
            crate::cli::trusted_store::trusted_dir_for_worktree(repo.path()).is_none(),
            "fixture must remove the trusted-store identity"
        );
        assert!(
            error.to_string().contains("trusted store identity changed"),
            "identity-loss rejection must be actionable: {error}"
        );
        assert_eq!(
            fs::read(trusted.join(WORK_EVENT_SETTLEMENT_RECORD_FILE))
                .expect("read unchanged settlement"),
            initial_bytes,
            "identity loss must leave the leased directory byte-equivalent"
        );
    }
}
