//! Delivered-owner Inspection and Git-free No Action (SPEC #3248 FR-239 /
//! FR-241 / FR-243, AS-216..AS-221).
//!
//! A *delivered owner* is a closed Issue/SPEC whose worktree has nothing left
//! to deliver: the configured integration base already contains its source
//! state. Opening one used to materialize a producing generation anyway, and
//! the session then had no honest way out — there was no source work to
//! verify, commit, or hand to a PR, so the only reachable settlement was
//! `execution.blocked`. An already shipped Issue became terminally Blocked
//! purely because no new PR existed.
//!
//! Two capabilities remove that trap:
//!
//! - [`classify_launch`] decides *before* anything is materialized whether the
//!   launch produces. Closed plus a proven zero source surface is Inspection —
//!   no Execution Control Record, no Work, no obligation. Ambiguity about the
//!   source surface fails to Inspection rather than to producing work.
//! - [`record_no_action`] settles a generation that was materialized anyway.
//!   It proves the zero source surface, writes a machine-local trusted audit
//!   that preserves the predecessor's bytes verbatim, advances the execution
//!   to its terminal No Action state, and touches nothing else: not Git, not
//!   verification, not the PR.
//!
//! The settlement is what keeps the outcome legible to a reader that predates
//! `execution.no_action` (Issue #4590): such a reader knows only the record's
//! three statuses, so an execution left Active after a correct No Action
//! wedges its Stop gate forever. The audit — not the status — is what tells
//! this successful non-delivery from a delivery.
//!
//! Neither path ever mutates GitHub. Reopening a closed Issue is explicitly
//! out of scope — the owner stays closed and the audit stays local.

use std::{
    io,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cli::execution_state::{
    ExecutionControlRecord, ExecutionControlStatus, ExecutionOwnerKind,
};

/// Trusted-store file holding this worktree's latest No Action audit.
pub const NO_ACTION_AUDIT_FILE: &str = "execution-no-action.json";

const NO_ACTION_AUDIT_SCHEMA_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// Owner lifecycle
// ---------------------------------------------------------------------------

/// The owner's GitHub lifecycle as the local Issue cache records it.
///
/// `Unknown` is not "probably closed": an uncached or unreadable owner keeps
/// the launch on its existing producing path, because treating a cache miss as
/// delivered would silently stop ordinary work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerLifecycle {
    Open,
    Closed,
    Unknown,
}

/// Read `state` from the local Issue cache entry for `number`.
#[must_use]
pub fn owner_lifecycle(repo_path: &Path, number: u64) -> OwnerLifecycle {
    let Some(cache_root) = crate::issue_cache::issue_cache_root_for_repo_path(repo_path) else {
        return OwnerLifecycle::Unknown;
    };
    let meta_path = cache_root.join(number.to_string()).join("meta.json");
    let Ok(contents) = std::fs::read_to_string(&meta_path) else {
        return OwnerLifecycle::Unknown;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return OwnerLifecycle::Unknown;
    };
    match value.get("state").and_then(serde_json::Value::as_str) {
        Some(state) if state.eq_ignore_ascii_case("closed") => OwnerLifecycle::Closed,
        Some(state) if state.eq_ignore_ascii_case("open") => OwnerLifecycle::Open,
        _ => OwnerLifecycle::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Source surface
// ---------------------------------------------------------------------------

/// What this worktree still has to deliver relative to the integration base.
///
/// The probe reuses the verification matrix's own changed-path collection, so
/// "source surface" means exactly what the matrix means by it: the committed
/// span against the merge-base, uncommitted changes against HEAD, and
/// untracked files, with `.gwt/` and `tasks/` bookkeeping excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSurface {
    /// The base contains the worktree's source state — nothing to deliver.
    Zero { base: String },
    /// Source paths that the base does not contain yet.
    Present { paths: Vec<String> },
    /// The base or the diff could not be read, so neither answer is proven.
    Unprovable { reason: String },
}

impl SourceSurface {
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Zero { base } => format!("zero source surface against base {base}"),
            Self::Present { paths } => {
                format!(
                    "{} changed source path(s): {}",
                    paths.len(),
                    paths.join(", ")
                )
            }
            Self::Unprovable { reason } => format!("source surface is unprovable: {reason}"),
        }
    }
}

/// Probe the worktree's source surface against the integration base.
#[must_use]
pub fn probe_source_surface(worktree: &Path) -> SourceSurface {
    let Some(base) = crate::cli::verify_derivation::integration_merge_base(worktree) else {
        return SourceSurface::Unprovable {
            reason:
                "no integration merge-base (origin/develop, origin/main, origin/HEAD) is readable"
                    .to_string(),
        };
    };
    match crate::cli::verify_derivation::changed_source_paths_since(worktree, &base) {
        Ok(paths) if paths.is_empty() => SourceSurface::Zero { base },
        Ok(paths) => SourceSurface::Present { paths },
        Err(reason) => SourceSurface::Unprovable { reason },
    }
}

// ---------------------------------------------------------------------------
// Launch classification
// ---------------------------------------------------------------------------

/// Whether the launch was asked to produce new work against this owner.
///
/// `ExplicitFollowUp` is the only thing that creates a fresh producing
/// generation for a delivered owner (FR-240). It never reopens the GitHub
/// Issue; materially different scope belongs to a new owner, which intake
/// routes before the launch happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUpIntent {
    /// Open or continue the owner without asking for new producing work.
    Default,
    /// Explicitly start follow-up work on this same owner.
    ExplicitFollowUp,
}

/// Why a launch is non-producing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectionReason {
    /// Closed owner, and the base already contains its source state.
    DeliveredZeroSurface,
    /// Closed owner whose source surface could not be proven either way.
    AmbiguousSourceSurface,
}

/// Why a launch produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProducingReason {
    /// The caller explicitly asked for follow-up work on this owner.
    ExplicitFollowUp,
    /// The owner is not a delivered one (open, or lifecycle unknown).
    OwnerNotDelivered,
    /// The worktree still holds source the base does not contain.
    SourceSurfacePresent,
}

/// What a launch may materialize for this owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchDisposition {
    /// Read history and evidence only: no generation, Work, obligation,
    /// verification requirement, or PR obligation is created.
    Inspection {
        reason: InspectionReason,
    },
    Producing {
        reason: ProducingReason,
    },
}

impl LaunchDisposition {
    #[must_use]
    pub fn is_inspection(self) -> bool {
        matches!(self, Self::Inspection { .. })
    }

    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Inspection {
                reason: InspectionReason::DeliveredZeroSurface,
            } => "inspection: the owner is closed and the base already contains this worktree's source state",
            Self::Inspection {
                reason: InspectionReason::AmbiguousSourceSurface,
            } => "inspection: the owner is closed and its source surface could not be proven",
            Self::Producing {
                reason: ProducingReason::ExplicitFollowUp,
            } => "producing: follow-up work was requested explicitly for this owner",
            Self::Producing {
                reason: ProducingReason::OwnerNotDelivered,
            } => "producing: the owner is not a delivered one",
            Self::Producing {
                reason: ProducingReason::SourceSurfacePresent,
            } => "producing: the worktree holds source the base does not contain",
        }
    }
}

/// Classify a launch from cached owner state, the caller's intent, and — only
/// when the first two leave the question open — the local source surface
/// (FR-239).
///
/// The order is the contract: an explicit follow-up always produces, an owner
/// that is not proven closed always produces, and only then does the source
/// surface decide. A closed owner whose surface cannot be proven falls to
/// Inspection — ambiguity must not materialize producing work.
///
/// `probe` is called at most once, and only on the path that needs it: the
/// probe costs three `git` invocations, and the two cheap answers above settle
/// nearly every launch.
pub fn classify_launch_with(
    lifecycle: OwnerLifecycle,
    intent: FollowUpIntent,
    probe: impl FnOnce() -> SourceSurface,
) -> LaunchDisposition {
    if intent == FollowUpIntent::ExplicitFollowUp {
        return LaunchDisposition::Producing {
            reason: ProducingReason::ExplicitFollowUp,
        };
    }
    if lifecycle != OwnerLifecycle::Closed {
        return LaunchDisposition::Producing {
            reason: ProducingReason::OwnerNotDelivered,
        };
    }
    match probe() {
        SourceSurface::Present { .. } => LaunchDisposition::Producing {
            reason: ProducingReason::SourceSurfacePresent,
        },
        SourceSurface::Zero { .. } => LaunchDisposition::Inspection {
            reason: InspectionReason::DeliveredZeroSurface,
        },
        SourceSurface::Unprovable { .. } => LaunchDisposition::Inspection {
            reason: InspectionReason::AmbiguousSourceSurface,
        },
    }
}

/// [`classify_launch_with`] over an already probed surface.
#[must_use]
pub fn classify_launch(
    lifecycle: OwnerLifecycle,
    surface: &SourceSurface,
    intent: FollowUpIntent,
) -> LaunchDisposition {
    classify_launch_with(lifecycle, intent, || surface.clone())
}

/// Classify a launch for `owner_number`, reading the owner lifecycle from the
/// Issue cache under `repo_path` and the source surface from `worktree`.
#[must_use]
pub fn classify_launch_for_owner(
    repo_path: &Path,
    worktree: &Path,
    owner_number: u64,
    intent: FollowUpIntent,
) -> LaunchDisposition {
    classify_launch_with(owner_lifecycle(repo_path, owner_number), intent, || {
        probe_source_surface(worktree)
    })
}

// ---------------------------------------------------------------------------
// No Action audit
// ---------------------------------------------------------------------------

/// A machine-local trusted record proving that a materialized generation had
/// no source work to deliver (FR-241).
///
/// It stands beside the Execution Control Record rather than inside it: the
/// predecessor's bytes are hashed, quoted, and preserved verbatim, so a
/// legacy, integrity-missing, or otherwise unrepairable predecessor survives a
/// No Action byte-identically (AS-221, Issue #4590 AC-2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoActionAudit {
    pub schema_version: u32,
    pub owner_kind: ExecutionOwnerKind,
    pub owner_number: u64,
    /// The session that recorded the No Action.
    pub session_id: String,
    pub reason: String,
    /// The predecessor record's own integrity hash, empty when it carries
    /// none. Quoted for evidence; never recomputed or written back.
    pub predecessor_content_hash: String,
    /// sha256 over the predecessor record's exact stored bytes.
    pub predecessor_bytes_sha256: String,
    /// The predecessor record's exact stored bytes (Issue #4590 AC-2).
    ///
    /// The canonical projection advances to a terminal state so that readers
    /// which predate `execution.no_action` see a settled execution, so the
    /// byte-identical predecessor lives here instead of in place.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub predecessor_execution_control_json: String,
    /// When the predecessor generation was launched. Immutable across the
    /// settlement, so it — not the record bytes — is what binds this audit to
    /// its own generation after the projection advances.
    pub predecessor_launched_at: DateTime<Utc>,
    pub predecessor_status: ExecutionControlStatus,
    /// The integration base that proved the zero source surface.
    pub source_base: String,
    pub recorded_at: DateTime<Utc>,
    /// Integrity hash over this audit with `content_hash` emptied.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
}

/// Why `execution.no_action` refused. Every variant names what the caller must
/// change; none of them mutates anything (AS-219).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoActionRefusal {
    /// No Execution Control Record exists — there is no generation to settle.
    NoRecord,
    /// The record belongs to another session.
    ForeignSession { holder_session_id: String },
    /// The record was edited outside the canonical operations.
    Tampered,
    /// The record already carries a terminal state. Blocked is not a
    /// successful No Action, and Completed is immutable.
    TerminalPredecessor { status: ExecutionControlStatus },
    /// Real source work exists, so the non-delivery claim is false.
    SourceChanges { paths: Vec<String> },
    /// The source surface could not be proven, so zero cannot be claimed.
    AmbiguousSourceSurface { reason: String },
    /// A stored audit describes a different owner or predecessor: the
    /// execution this audit belongs to was replaced.
    ReplacedExecution { stored_owner: String },
}

impl NoActionRefusal {
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::NoRecord => {
                "no execution control record exists for this worktree, so there is no generation to settle as No Action"
                    .to_string()
            }
            Self::ForeignSession { holder_session_id } => format!(
                "the execution control record belongs to session {holder_session_id}; take it over with `execution.adopt` before settling it"
            ),
            Self::Tampered => {
                "the execution control record failed integrity validation; repair it with `execution.repair` before settling it"
                    .to_string()
            }
            Self::TerminalPredecessor { status } => format!(
                "the execution is already {}; No Action settles an active generation only, and a Blocked execution is never a successful non-delivery",
                match status {
                    ExecutionControlStatus::Completed => "completed",
                    ExecutionControlStatus::Blocked => "blocked",
                    ExecutionControlStatus::Active => "active",
                }
            ),
            Self::SourceChanges { paths } => format!(
                "this worktree still holds {} source path(s) the base does not contain ({}); deliver them instead of recording No Action",
                paths.len(),
                paths.join(", ")
            ),
            Self::AmbiguousSourceSurface { reason } => format!(
                "the zero source surface could not be proven: {reason}"
            ),
            Self::ReplacedExecution { stored_owner } => format!(
                "a stored No Action audit describes {stored_owner}, which is not this execution; the execution was replaced"
            ),
        }
    }
}

/// Outcome of [`record_no_action`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoActionOutcome {
    Recorded(Box<NoActionAudit>),
    /// An identical audit is already on record; nothing was rewritten.
    AlreadyRecorded(Box<NoActionAudit>),
    Refused(NoActionRefusal),
}

fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

#[must_use]
pub fn compute_audit_hash(audit: &NoActionAudit) -> String {
    let mut probe = audit.clone();
    probe.content_hash = String::new();
    sha256_hex(serde_json::to_vec(&probe).unwrap_or_default())
}

#[must_use]
pub fn audit_integrity_ok(audit: &NoActionAudit) -> bool {
    !audit.content_hash.is_empty() && audit.content_hash == compute_audit_hash(audit)
}

/// Worktree mirror of the audit, for degenerate (non-git) stores.
#[must_use]
pub fn audit_mirror_path(worktree: &Path) -> PathBuf {
    worktree.join(".gwt/skill-state").join(NO_ACTION_AUDIT_FILE)
}

/// Read this worktree's stored No Action audit, if any.
pub fn load_audit(worktree: &Path) -> io::Result<Option<NoActionAudit>> {
    let contents = match crate::cli::trusted_store::read(worktree, NO_ACTION_AUDIT_FILE)? {
        Some(contents) => Some(contents),
        None => match std::fs::read_to_string(audit_mirror_path(worktree)) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        },
    };
    let Some(contents) = contents else {
        return Ok(None);
    };
    // An audit from a superseded schema reads as absent rather than as an
    // error: the only writer is `record_no_action`, which re-proves the zero
    // source surface and records a current audit, so refusing the whole
    // operation over a stale file would strand the very sessions Issue #4590
    // is about.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) {
        if value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(u64::from(NO_ACTION_AUDIT_SCHEMA_VERSION))
        {
            return Ok(None);
        }
    }
    match serde_json::from_str::<NoActionAudit>(&contents) {
        Ok(audit) => Ok(Some(audit)),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    }
}

fn owner_label(kind: ExecutionOwnerKind, number: u64) -> String {
    format!("{} #{number}", kind.as_str())
}

/// Whether `audit` was recorded for exactly this generation.
///
/// The binding is the predecessor's launch timestamp rather than its stored
/// bytes: the settlement advances the canonical projection (Issue #4590), so
/// the bytes stop matching the moment the No Action succeeds, while
/// `launched_at` is immutable for the lifetime of a generation. A replacement
/// generation for the same owner and session carries a different one, so the
/// lookup still fails closed.
fn audit_matches_execution(
    audit: &NoActionAudit,
    record: &ExecutionControlRecord,
    session_id: &str,
) -> bool {
    audit.owner_kind == record.owner_kind
        && audit.owner_number == record.owner_number
        && audit.session_id == session_id
        && audit.predecessor_launched_at == record.launched_at
}

/// Advance the canonical projection to its terminal No Action state.
///
/// Idempotent: an execution that is already terminal stays as it is.
fn settle_as_no_action(worktree: &Path, session_id: &str, reason: &str) -> io::Result<()> {
    use crate::cli::execution_state::{ExecutionSettlement, SettleResult};

    match crate::cli::execution_state::settle(
        worktree,
        session_id,
        ExecutionSettlement::NoAction {
            reason: reason.to_string(),
        },
    )? {
        SettleResult::Settled(_) | SettleResult::AlreadySettled(_) | SettleResult::NoRecord => {
            Ok(())
        }
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "execution.no_action recorded its audit but could not settle the execution: {other:?}"
            ),
        )),
    }
}

/// A trusted No Action audit for the session currently holding `worktree`'s
/// execution, or `None`.
///
/// The gates use this to tell a successful non-delivery from an unsettled
/// Active record. It fails closed: an unreadable, integrity-failed, or
/// mismatched audit is no audit at all.
#[must_use]
pub fn trusted_no_action_for_session(worktree: &Path, session_id: &str) -> Option<NoActionAudit> {
    let record = crate::cli::execution_state::load(worktree).ok().flatten()?;
    if !crate::cli::execution_state::integrity_ok(&record) {
        return None;
    }
    if record.primary_session_id != session_id {
        return None;
    }
    let audit = load_audit(worktree).ok().flatten()?;
    if !audit_integrity_ok(&audit) {
        return None;
    }
    audit_matches_execution(&audit, &record, session_id).then_some(audit)
}

/// Record a Git-free No Action for the current session's active generation
/// (FR-241, AS-218/AS-219).
///
/// On success this writes exactly one machine-local trusted audit, preserving
/// the predecessor Execution Control Record's exact bytes inside it, settles
/// this session's own open action obligations, and settles the execution
/// itself. It performs no Git commit or push, requests no verification record,
/// and creates and mutates no PR.
pub fn record_no_action(
    worktree: &Path,
    session_id: &str,
    reason: &str,
) -> io::Result<NoActionOutcome> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "execution.no_action requires a non-empty params.reason",
        ));
    }
    let Some(record) = crate::cli::execution_state::load(worktree)? else {
        return Ok(NoActionOutcome::Refused(NoActionRefusal::NoRecord));
    };
    if !crate::cli::execution_state::integrity_ok(&record) {
        return Ok(NoActionOutcome::Refused(NoActionRefusal::Tampered));
    }
    if record.primary_session_id != session_id {
        return Ok(NoActionOutcome::Refused(NoActionRefusal::ForeignSession {
            holder_session_id: record.primary_session_id,
        }));
    }
    // Idempotency comes before the terminal check and before the probe: a No
    // Action already on record is a settled outcome, the settlement itself is
    // what made the record terminal, and re-proving it would let an unrelated
    // later edit in the worktree turn a successful settlement into a refusal.
    if let Some(stored) = load_audit(worktree)? {
        if audit_integrity_ok(&stored) {
            if audit_matches_execution(&stored, &record, session_id) {
                // Converge a No Action whose audit landed but whose
                // settlement did not: the audit alone leaves the execution
                // Active, which is exactly the wedge of Issue #4590.
                settle_as_no_action(worktree, session_id, &stored.reason)?;
                return Ok(NoActionOutcome::AlreadyRecorded(Box::new(stored)));
            }
            if stored.owner_kind == record.owner_kind
                && stored.owner_number == record.owner_number
                && stored.session_id == session_id
            {
                return Ok(NoActionOutcome::Refused(
                    NoActionRefusal::ReplacedExecution {
                        stored_owner: format!(
                            "{} recorded against a different predecessor record",
                            owner_label(stored.owner_kind, stored.owner_number)
                        ),
                    },
                ));
            }
        }
    }

    if record.status != ExecutionControlStatus::Active {
        return Ok(NoActionOutcome::Refused(
            NoActionRefusal::TerminalPredecessor {
                status: record.status,
            },
        ));
    }
    let Some(predecessor_bytes) = crate::cli::execution_state::record_contents(worktree)? else {
        return Ok(NoActionOutcome::Refused(NoActionRefusal::NoRecord));
    };
    let predecessor_bytes_sha256 = sha256_hex(predecessor_bytes.as_bytes());

    let source_base = match probe_source_surface(worktree) {
        SourceSurface::Zero { base } => base,
        SourceSurface::Present { paths } => {
            return Ok(NoActionOutcome::Refused(NoActionRefusal::SourceChanges {
                paths,
            }))
        }
        SourceSurface::Unprovable { reason } => {
            return Ok(NoActionOutcome::Refused(
                NoActionRefusal::AmbiguousSourceSurface { reason },
            ))
        }
    };

    let mut audit = NoActionAudit {
        schema_version: NO_ACTION_AUDIT_SCHEMA_VERSION,
        owner_kind: record.owner_kind,
        owner_number: record.owner_number,
        session_id: session_id.to_string(),
        reason: reason.to_string(),
        predecessor_content_hash: record.content_hash.clone(),
        predecessor_bytes_sha256,
        predecessor_execution_control_json: predecessor_bytes,
        predecessor_launched_at: record.launched_at,
        predecessor_status: record.status,
        source_base,
        recorded_at: Utc::now(),
        content_hash: String::new(),
    };
    audit.content_hash = compute_audit_hash(&audit);
    let bytes = serde_json::to_vec_pretty(&audit)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    crate::cli::trusted_store::with_write_lease(worktree, || {
        crate::cli::trusted_store::write(worktree, NO_ACTION_AUDIT_FILE, &bytes)?;
        let mirror = audit_mirror_path(worktree);
        if let Some(parent) = mirror.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&mirror, &bytes)
    })?;

    // Issue #4590 AC-1: the audit is written first so a failure here is
    // retryable, then the canonical projection advances to its terminal No
    // Action state. Readers that predate `execution.no_action` know only the
    // record's three statuses, so leaving it Active would wedge their Stop
    // gate on an execution that settled correctly. The predecessor's exact
    // bytes are preserved in the audit above, and the owner ledger records
    // the settlement as a non-delivery.
    settle_as_no_action(worktree, session_id, reason)?;

    // AS-218: the obligations this action armed are settled as No Action. The
    // settlement helper is already scoped to this session's own state, so no
    // other session's obligations are touched.
    crate::cli::action_obligation::settle_kinds_best_effort(
        worktree,
        session_id,
        &[
            crate::cli::action_obligation::ObligationKind::IssueUpdate,
            crate::cli::action_obligation::ObligationKind::Implementation,
            crate::cli::action_obligation::ObligationKind::Verification,
            crate::cli::action_obligation::ObligationKind::Pr,
        ],
        &format!("execution.no_action ({reason})"),
    );

    Ok(NoActionOutcome::Recorded(Box::new(audit)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::execution_state::{
        materialize_at_launch, settle, ExecutionOwnerKind, ExecutionSettlement,
    };
    use gwt_core::process::hidden_command;

    fn git(worktree: &Path, args: &[&str]) {
        let status = hidden_command("git")
            .arg("-C")
            .arg(worktree)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn write_file(worktree: &Path, rel: &str, contents: &str) {
        let path = worktree.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    /// A worktree whose integration base already contains everything it has:
    /// the shape a delivered owner leaves behind.
    fn delivered_worktree(worktree: &Path) {
        crate::cli::trusted_store::init_git_repo_with_origin(worktree);
        git(
            worktree,
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        git(worktree, &["checkout", "-q", "-b", "work/issue-3290"]);
    }

    fn zero() -> SourceSurface {
        SourceSurface::Zero {
            base: "abc123".to_string(),
        }
    }

    fn present() -> SourceSurface {
        SourceSurface::Present {
            paths: vec!["crates/gwt/src/lib.rs".to_string()],
        }
    }

    fn unprovable() -> SourceSurface {
        SourceSurface::Unprovable {
            reason: "no base".to_string(),
        }
    }

    // AC-1 / FR-239: closed plus a proven zero source surface is the only
    // default route into Inspection; every other input keeps producing.
    #[test]
    fn closed_zero_surface_owner_defaults_to_inspection() {
        assert_eq!(
            classify_launch(OwnerLifecycle::Closed, &zero(), FollowUpIntent::Default),
            LaunchDisposition::Inspection {
                reason: InspectionReason::DeliveredZeroSurface
            }
        );
    }

    // AC-1: an open owner is never a delivered one, whatever its worktree
    // holds. This is the route that must stay byte-identical to today.
    #[test]
    fn open_owner_keeps_producing() {
        for surface in [zero(), present(), unprovable()] {
            assert_eq!(
                classify_launch(OwnerLifecycle::Open, &surface, FollowUpIntent::Default),
                LaunchDisposition::Producing {
                    reason: ProducingReason::OwnerNotDelivered
                },
                "{surface:?}"
            );
        }
        // An uncached owner is not evidence of delivery either.
        assert_eq!(
            classify_launch(OwnerLifecycle::Unknown, &zero(), FollowUpIntent::Default),
            LaunchDisposition::Producing {
                reason: ProducingReason::OwnerNotDelivered
            }
        );
    }

    // AC-1: real source work outranks the closed state — there is something
    // to deliver, so the launch produces.
    #[test]
    fn closed_owner_with_source_changes_keeps_producing() {
        assert_eq!(
            classify_launch(OwnerLifecycle::Closed, &present(), FollowUpIntent::Default),
            LaunchDisposition::Producing {
                reason: ProducingReason::SourceSurfacePresent
            }
        );
    }

    // AC-1 / FR-239: ambiguity fails to Inspection, never to producing work.
    #[test]
    fn closed_owner_with_unprovable_surface_falls_to_inspection() {
        assert_eq!(
            classify_launch(
                OwnerLifecycle::Closed,
                &unprovable(),
                FollowUpIntent::Default
            ),
            LaunchDisposition::Inspection {
                reason: InspectionReason::AmbiguousSourceSurface
            }
        );
    }

    // AC-1 / AS-217: explicit follow-up is the one intent that creates a fresh
    // producing generation for a delivered owner — and it does so without any
    // GitHub state change, because classification never touches GitHub.
    #[test]
    fn explicit_follow_up_produces_for_a_delivered_owner() {
        assert_eq!(
            classify_launch(
                OwnerLifecycle::Closed,
                &zero(),
                FollowUpIntent::ExplicitFollowUp
            ),
            LaunchDisposition::Producing {
                reason: ProducingReason::ExplicitFollowUp
            }
        );
    }

    // FR-239: the probe reads the same changed-path set the verification
    // matrix does, so bookkeeping never counts as something to deliver.
    #[test]
    fn source_surface_probe_separates_delivery_from_bookkeeping() {
        let dir = tempfile::tempdir().unwrap();
        delivered_worktree(dir.path());
        assert!(
            matches!(probe_source_surface(dir.path()), SourceSurface::Zero { .. }),
            "a delivered worktree has nothing to deliver"
        );

        write_file(dir.path(), ".gwt/work/events.jsonl", "{}\n");
        write_file(dir.path(), "tasks/todo.md", "- [ ] x\n");
        assert!(
            matches!(probe_source_surface(dir.path()), SourceSurface::Zero { .. }),
            "gwt bookkeeping is not deliverable source"
        );

        write_file(dir.path(), "crates/gwt/src/new.rs", "pub fn x() {}\n");
        let SourceSurface::Present { paths } = probe_source_surface(dir.path()) else {
            panic!("an untracked source file is a source surface");
        };
        assert_eq!(paths, vec!["crates/gwt/src/new.rs".to_string()]);

        let bare = tempfile::tempdir().unwrap();
        assert!(matches!(
            probe_source_surface(bare.path()),
            SourceSurface::Unprovable { .. }
        ));
    }

    struct NoActionFixture {
        _home: gwt_core::test_support::ScopedGwtHome,
        home: tempfile::TempDir,
        worktree: tempfile::TempDir,
    }

    fn fixture() -> NoActionFixture {
        let home = tempfile::tempdir().unwrap();
        let guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let worktree = tempfile::tempdir().unwrap();
        delivered_worktree(worktree.path());
        NoActionFixture {
            _home: guard,
            home,
            worktree,
        }
    }

    impl NoActionFixture {
        fn path(&self) -> &Path {
            self.worktree.path()
        }

        fn launch(&self, session: &str) {
            materialize_at_launch(
                self.path(),
                ExecutionOwnerKind::Issue,
                3290,
                session,
                "$gwt-execute",
                false,
            )
            .unwrap();
        }

        fn record_bytes(&self) -> String {
            crate::cli::execution_state::record_contents(self.path())
                .unwrap()
                .unwrap()
        }
    }

    // AC-2 / AS-218: a zero-diff No Action succeeds, is idempotent, and
    // preserves the predecessor byte-identically — in the audit, because the
    // canonical projection settles (Issue #4590).
    #[test]
    fn no_action_succeeds_idempotently_and_preserves_predecessor_bytes() {
        let fx = fixture();
        fx.launch("sess-1");
        let before = fx.record_bytes();

        let NoActionOutcome::Recorded(audit) =
            record_no_action(fx.path(), "sess-1", "owner #3290 shipped in PR #3328").unwrap()
        else {
            panic!("a delivered owner records No Action");
        };
        assert_eq!(audit.owner_number, 3290);
        assert_eq!(audit.reason, "owner #3290 shipped in PR #3328");
        assert!(audit_integrity_ok(&audit), "the audit is integrity hashed");
        assert_eq!(
            audit.predecessor_bytes_sha256,
            sha256_hex(before.as_bytes()),
            "the audit quotes the predecessor bytes"
        );
        assert_eq!(
            audit.predecessor_execution_control_json, before,
            "No Action preserves the predecessor's bytes verbatim"
        );
        let settled = fx.record_bytes();

        let NoActionOutcome::AlreadyRecorded(again) =
            record_no_action(fx.path(), "sess-1", "owner #3290 shipped in PR #3328").unwrap()
        else {
            panic!("a second No Action is idempotent");
        };
        assert_eq!(*again, *audit, "the stored audit is returned unchanged");
        assert_eq!(
            fx.record_bytes(),
            settled,
            "a repeated No Action rewrites nothing"
        );

        // The audit is machine-local: it lives in the trusted store, not in
        // the repository.
        assert!(
            crate::cli::trusted_store::read(fx.path(), NO_ACTION_AUDIT_FILE)
                .unwrap()
                .is_some(),
            "the audit is persisted in the machine-local trusted store"
        );
        assert!(
            fx.home.path().exists(),
            "the trusted store lives under the machine-local gwt home"
        );
    }

    // AC-2 / AS-218: the audit records the settlement without asking for
    // verification evidence or a PR, and settles this session's obligations.
    #[test]
    fn no_action_settles_obligations_without_verification_or_pr() {
        let fx = fixture();
        fx.launch("sess-1");
        crate::cli::action_obligation::mark_from_prompt(
            fx.path(),
            "sess-1",
            "Issue #3290 の残作業を実装してください",
        )
        .unwrap();
        assert!(
            !crate::cli::action_obligation::open_kinds(fx.path(), "sess-1").is_empty(),
            "the prompt armed a producing obligation"
        );

        record_no_action(fx.path(), "sess-1", "already delivered").unwrap();

        assert!(
            crate::cli::action_obligation::open_kinds(fx.path(), "sess-1").is_empty(),
            "No Action settles this session's own obligations"
        );
        assert!(
            crate::cli::verification_record::load(fx.path())
                .unwrap()
                .is_none(),
            "No Action creates no verification record"
        );
    }

    // AC-2 / AS-219: real source work makes the non-delivery claim false.
    #[test]
    fn no_action_refuses_a_dirty_worktree_without_changing_anything() {
        let fx = fixture();
        fx.launch("sess-1");
        let before = fx.record_bytes();
        write_file(fx.path(), "crates/gwt/src/new.rs", "pub fn x() {}\n");

        let NoActionOutcome::Refused(NoActionRefusal::SourceChanges { paths }) =
            record_no_action(fx.path(), "sess-1", "nothing to do").unwrap()
        else {
            panic!("source changes refuse No Action");
        };
        assert_eq!(paths, vec!["crates/gwt/src/new.rs".to_string()]);
        assert_eq!(fx.record_bytes(), before);
        assert!(
            load_audit(fx.path()).unwrap().is_none(),
            "no audit is written"
        );
    }

    // AC-2 / AS-219: a foreign session cannot settle someone else's execution.
    #[test]
    fn no_action_refuses_a_foreign_session() {
        let fx = fixture();
        fx.launch("sess-1");
        let before = fx.record_bytes();

        let NoActionOutcome::Refused(NoActionRefusal::ForeignSession { holder_session_id }) =
            record_no_action(fx.path(), "sess-2", "not mine").unwrap()
        else {
            panic!("a foreign session refuses No Action");
        };
        assert_eq!(holder_session_id, "sess-1");
        assert_eq!(fx.record_bytes(), before);
        assert!(load_audit(fx.path()).unwrap().is_none());
    }

    // AC-2 / Out of scope: terminal Blocked is never a successful No Action.
    #[test]
    fn no_action_refuses_a_terminal_predecessor() {
        let fx = fixture();
        fx.launch("sess-1");
        settle(
            fx.path(),
            "sess-1",
            ExecutionSettlement::Blocked {
                reason: "runner unavailable".to_string(),
                missing_verification: None,
            },
        )
        .unwrap();
        let before = fx.record_bytes();

        let NoActionOutcome::Refused(NoActionRefusal::TerminalPredecessor { status }) =
            record_no_action(fx.path(), "sess-1", "already delivered").unwrap()
        else {
            panic!("a terminal predecessor refuses No Action");
        };
        assert_eq!(status, ExecutionControlStatus::Blocked);
        assert_eq!(fx.record_bytes(), before);
        assert!(load_audit(fx.path()).unwrap().is_none());
    }

    // AC-2 / AS-219: an ambiguous owner state proves nothing, so it refuses.
    #[test]
    fn no_action_refuses_an_unprovable_source_surface() {
        let home = tempfile::tempdir().unwrap();
        let _guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        // No origin/develop ref: the base the claim would rest on is missing.
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            3290,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();

        assert!(matches!(
            record_no_action(dir.path(), "sess-1", "already delivered").unwrap(),
            NoActionOutcome::Refused(NoActionRefusal::AmbiguousSourceSurface { .. })
        ));
        assert!(load_audit(dir.path()).unwrap().is_none());
    }

    // AC-2: no record at all is a refusal, not a silent success.
    #[test]
    fn no_action_refuses_without_a_record() {
        let fx = fixture();
        assert_eq!(
            record_no_action(fx.path(), "sess-1", "nothing here").unwrap(),
            NoActionOutcome::Refused(NoActionRefusal::NoRecord)
        );
    }

    // AC-2: an empty reason is rejected before anything is read.
    #[test]
    fn no_action_requires_a_non_empty_reason() {
        let fx = fixture();
        fx.launch("sess-1");
        let error = record_no_action(fx.path(), "sess-1", "   ").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    // AC-2 / AC-3 / AS-221: a legacy predecessor carrying no integrity hash is
    // still a valid record (pre-P9a launches wrote them), so No Action accepts
    // it — and preserves it. The audit quotes the missing hash as missing, the
    // byte hash separately, and the legacy bytes themselves verbatim.
    #[test]
    fn no_action_preserves_a_legacy_integrity_missing_predecessor() {
        let fx = fixture();
        fx.launch("sess-1");
        let mut record = crate::cli::execution_state::load(fx.path())
            .unwrap()
            .unwrap();
        record.content_hash = String::new();
        let legacy = serde_json::to_vec_pretty(&record).unwrap();
        crate::cli::trusted_store::write(fx.path(), "execution-control.json", &legacy).unwrap();
        let before = fx.record_bytes();

        let NoActionOutcome::Recorded(audit) =
            record_no_action(fx.path(), "sess-1", "already delivered").unwrap()
        else {
            panic!("a legacy predecessor still settles as No Action");
        };
        assert!(
            audit.predecessor_content_hash.is_empty(),
            "the audit records the predecessor's missing hash as missing"
        );
        assert_eq!(
            audit.predecessor_bytes_sha256,
            sha256_hex(before.as_bytes())
        );
        assert_eq!(
            audit.predecessor_execution_control_json, before,
            "the legacy predecessor is preserved exactly as it was found"
        );
        assert_ne!(
            fx.record_bytes(),
            before,
            "the projection still settles, so a no_action-blind reader is released"
        );
    }

    // AC-2 / AS-221: a predecessor edited outside the canonical operations is
    // refused and left exactly as it was found.
    #[test]
    fn no_action_refuses_a_tampered_predecessor() {
        let fx = fixture();
        fx.launch("sess-1");
        let mut record = crate::cli::execution_state::load(fx.path())
            .unwrap()
            .unwrap();
        record.entrypoint = "edited-by-hand".to_string();
        let tampered = serde_json::to_vec_pretty(&record).unwrap();
        crate::cli::trusted_store::write(fx.path(), "execution-control.json", &tampered).unwrap();
        let before = fx.record_bytes();

        assert_eq!(
            record_no_action(fx.path(), "sess-1", "already delivered").unwrap(),
            NoActionOutcome::Refused(NoActionRefusal::Tampered)
        );
        assert_eq!(
            fx.record_bytes(),
            before,
            "a tampered predecessor is never rewritten in place"
        );
        assert!(load_audit(fx.path()).unwrap().is_none());
    }

    // AC-4: the gate helper tells a trusted No Action from an unsettled
    // record, and fails closed on a tampered audit or a foreign session.
    #[test]
    fn trusted_no_action_lookup_fails_closed() {
        let fx = fixture();
        fx.launch("sess-1");
        assert!(trusted_no_action_for_session(fx.path(), "sess-1").is_none());

        record_no_action(fx.path(), "sess-1", "already delivered").unwrap();
        assert!(trusted_no_action_for_session(fx.path(), "sess-1").is_some());
        assert!(
            trusted_no_action_for_session(fx.path(), "sess-2").is_none(),
            "another session's No Action never releases this one"
        );

        let mut audit = load_audit(fx.path()).unwrap().unwrap();
        audit.reason = "edited outside the canonical operation".to_string();
        let bytes = serde_json::to_vec_pretty(&audit).unwrap();
        crate::cli::trusted_store::write(fx.path(), NO_ACTION_AUDIT_FILE, &bytes).unwrap();
        std::fs::write(audit_mirror_path(fx.path()), &bytes).unwrap();
        assert!(
            trusted_no_action_for_session(fx.path(), "sess-1").is_none(),
            "an edited audit is no audit at all"
        );
    }

    // Issue #4590 AC-1: the settled trace has to be legible to a reader that
    // knows nothing about `execution.no_action`. Such a reader only knows the
    // three Execution Control Record statuses, so No Action leaves the
    // execution terminal rather than Active.
    #[test]
    fn no_action_leaves_the_execution_terminal_for_a_status_only_reader() {
        let fx = fixture();
        fx.launch("sess-1");

        record_no_action(fx.path(), "sess-1", "owner #3290 shipped in PR #3328").unwrap();

        let record = crate::cli::execution_state::load(fx.path())
            .unwrap()
            .unwrap();
        assert_ne!(
            record.status,
            ExecutionControlStatus::Active,
            "a status-only reader must see a settled execution"
        );
        assert!(
            record.settled_at.is_some(),
            "the settlement carries its timestamp"
        );
        assert!(
            record.blocked_reason.is_none(),
            "a delivered owner is never a blocker"
        );
        assert!(
            record.completion_evidence.is_none(),
            "No Action consumes no verification evidence"
        );
        assert!(
            crate::cli::execution_state::integrity_ok(&record),
            "the settled record stays integrity-valid"
        );
    }

    // Issue #4590: an audit written by the schema that shipped before this fix
    // must not strand the session. It reads as absent, so the operation
    // re-proves the zero source surface and settles the execution properly.
    #[test]
    fn a_superseded_audit_schema_reads_as_absent_instead_of_stranding_the_session() {
        let fx = fixture();
        fx.launch("sess-1");
        let stale = serde_json::json!({
            "schema_version": 1,
            "owner_kind": "issue",
            "owner_number": 3290,
            "session_id": "sess-1",
            "reason": "already delivered",
            "predecessor_content_hash": "",
            "predecessor_bytes_sha256": "deadbeef",
            "predecessor_status": "active",
            "source_base": "origin/develop",
            "recorded_at": "2026-09-22T00:00:00Z",
            "content_hash": "unverifiable",
        });
        let bytes = serde_json::to_vec_pretty(&stale).unwrap();
        crate::cli::trusted_store::write(fx.path(), NO_ACTION_AUDIT_FILE, &bytes).unwrap();
        std::fs::write(audit_mirror_path(fx.path()), &bytes).unwrap();

        assert!(
            load_audit(fx.path()).unwrap().is_none(),
            "a superseded audit schema is not an error and not an audit"
        );
        assert!(
            matches!(
                record_no_action(fx.path(), "sess-1", "already delivered").unwrap(),
                NoActionOutcome::Recorded(_)
            ),
            "the operation records a current audit instead of refusing"
        );
        assert_ne!(
            crate::cli::execution_state::load(fx.path())
                .unwrap()
                .unwrap()
                .status,
            ExecutionControlStatus::Active,
            "and the execution ends up settled"
        );
    }

    // Issue #4590 AC-2: the predecessor is preserved byte-identically. It
    // moves into the settled trace verbatim instead of staying in place, so
    // the guarantee survives the terminal projection.
    #[test]
    fn no_action_preserves_the_predecessor_bytes_verbatim_in_the_audit() {
        let fx = fixture();
        fx.launch("sess-1");
        let before = fx.record_bytes();

        let NoActionOutcome::Recorded(audit) =
            record_no_action(fx.path(), "sess-1", "already delivered").unwrap()
        else {
            panic!("a delivered owner records No Action");
        };

        assert_eq!(
            audit.predecessor_execution_control_json, before,
            "the audit keeps the predecessor record byte-identical"
        );
        assert_eq!(
            audit.predecessor_bytes_sha256,
            sha256_hex(before.as_bytes()),
            "the preserved bytes are the ones the audit quotes"
        );
        assert_eq!(audit.predecessor_status, ExecutionControlStatus::Active);
    }

    // Issue #4590 AC-2 (PM condition 1): the verbatim predecessor bytes are
    // covered by the audit's integrity hash. Without that they would be the
    // one unprotected part of the evidence that the predecessor was preserved,
    // so editing them would forge exactly the guarantee AC-2 asks for.
    #[test]
    fn the_preserved_predecessor_bytes_are_covered_by_the_audit_integrity_hash() {
        let fx = fixture();
        fx.launch("sess-1");
        record_no_action(fx.path(), "sess-1", "already delivered").unwrap();

        let mut audit = load_audit(fx.path()).unwrap().unwrap();
        assert!(audit_integrity_ok(&audit));

        audit
            .predecessor_execution_control_json
            .push_str("\n<!-- forged -->");
        assert!(
            !audit_integrity_ok(&audit),
            "editing the preserved bytes must break the audit's integrity hash"
        );

        let bytes = serde_json::to_vec_pretty(&audit).unwrap();
        crate::cli::trusted_store::write(fx.path(), NO_ACTION_AUDIT_FILE, &bytes).unwrap();
        std::fs::write(audit_mirror_path(fx.path()), &bytes).unwrap();
        assert!(
            trusted_no_action_for_session(fx.path(), "sess-1").is_none(),
            "and an audit with forged predecessor bytes is no audit at all"
        );
    }
}
