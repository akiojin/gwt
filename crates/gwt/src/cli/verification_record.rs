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
    pub commands: Vec<String>,
    /// Full T-130: the matrix was derived from changed surfaces instead of
    /// hand-picked (`verify.plan` with `params.derive:true`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub derived: bool,
    /// Content-level worktree fingerprint at plan registration. Derived
    /// matrices are valid only for this exact change set; a later surface
    /// change requires deriving and registering a new plan.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub worktree_fingerprint: String,
    /// Present only for the canonical bookkeeping-only recovery derivation.
    /// The plan hash binds classifier/base/requirement provenance in addition
    /// to the derivation-time worktree fingerprint above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_change_evidence: Option<crate::cli::verify_derivation::NoChangeDerivationEvidence>,
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
pub fn save_plan(worktree: &Path, plan: &VerificationPlanRecord) -> io::Result<()> {
    crate::cli::trusted_store::with_write_lease(worktree, || {
        let mut plan = plan.clone();
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

/// Register a canonical plan with its current execution owner and integrity
/// hash captured under the same owner write lease.
fn register_plan(
    worktree: &Path,
    session_id: &str,
    commands: Vec<String>,
    derived: bool,
) -> io::Result<VerificationPlanRecord> {
    crate::cli::trusted_store::with_write_lease(worktree, || {
        let fingerprint = worktree_fingerprint(worktree);
        register_plan_unleased(worktree, session_id, commands, derived, fingerprint, None)
    })
}

fn register_plan_unleased(
    worktree: &Path,
    session_id: &str,
    commands: Vec<String>,
    derived: bool,
    worktree_fingerprint: String,
    no_change_evidence: Option<crate::cli::verify_derivation::NoChangeDerivationEvidence>,
) -> io::Result<VerificationPlanRecord> {
    let owner_number = execution_state::load(worktree)?.map(|record| record.owner_number);
    let mut plan = VerificationPlanRecord {
        session_id: session_id.to_string(),
        owner_number,
        commands,
        derived,
        worktree_fingerprint,
        no_change_evidence,
        created_at: Utc::now(),
        content_hash: String::new(),
    };
    plan.content_hash = compute_plan_hash(&plan);
    save_plan_unleased(worktree, &plan)?;
    Ok(plan)
}

pub(crate) fn derive_and_register_plan(
    worktree: &Path,
    session_id: &str,
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
        let plan = register_plan_unleased(
            worktree,
            session_id,
            derived.commands.clone(),
            true,
            fingerprint_after,
            derived.no_change_evidence.clone(),
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
pub fn run_verification(
    worktree: &Path,
    session_id: &str,
    commands: &[String],
) -> Result<(VerificationRunRecord, String), String> {
    if commands.is_empty() {
        return Err("verify.run requires at least one command".to_string());
    }
    // Snapshot owner, plan, and worktree together. Commands deliberately run
    // outside the lease; the final commit reacquires it and rejects any
    // interleaving writer by invalidating the evidence snapshot.
    let (owner_number, plan_snapshot, fingerprint_before) =
        crate::cli::trusted_store::with_write_lease(worktree, || {
            let owner_number = execution_state::load(worktree)?.map(|record| record.owner_number);
            let plan = load_plan(worktree)?;
            Ok((owner_number, plan, worktree_fingerprint(worktree)))
        })
        .map_err(|err| format!("failed to snapshot verification state: {err}"))?;
    let plan_provenance_valid_before = plan_snapshot.as_ref().is_none_or(|plan| {
        crate::cli::verify_derivation::validate_no_change_plan(worktree, plan).is_ok()
    });
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
                    && plan_provenance_valid_before =>
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
        let current_owner = execution_state::load(worktree)?
            .map(|execution| execution.owner_number);
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
        if current_plan.as_ref().is_some_and(|plan| {
            crate::cli::verify_derivation::validate_no_change_plan(worktree, plan).is_err()
        }) {
            transcript.push_str(
                "warning: the no-change derivation provenance is stale or invalid — derive and run a fresh plan\n",
            );
            record.plan_covered = false;
        }
        record.created_at = Utc::now();
        record.content_hash = compute_content_hash(&record);
        save(worktree, &record)
    })
    .map_err(|err| format!("failed to save verification record: {err}"))?;

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
            || plan.worktree_fingerprint.is_empty()
            || plan.worktree_fingerprint != record.worktree_fingerprint
            || record.verification_plan_hash != plan.content_hash
            || record.plan_derived != plan.derived
        {
            return EvidenceStatus::PlanChanged;
        }
        if crate::cli::verify_derivation::validate_no_change_plan(worktree, plan).is_err() {
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
    match command {
        VerifyCommand::Plan { commands, derive } => {
            let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
            let (commands, plan) = if derive {
                if !commands.is_empty() {
                    return Err(SpecOpsError::from(ApiError::Unexpected(
                        "verify.plan takes either params.derive:true or explicit params.commands, not both"
                            .to_string(),
                    )));
                }
                let (derived_plan, plan) = derive_and_register_plan(&worktree, &session_id)
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
                let plan = register_plan(&worktree, &session_id, commands.clone(), false)
                    .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
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
            let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
            let (record, transcript) = run_verification(&worktree, &session_id, &commands)
                .map_err(|err| SpecOpsError::from(ApiError::Unexpected(err)))?;
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
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    fn git(worktree: &Path, args: &[&str]) {
        let status = gwt_core::process::hidden_command("git")
            .arg("-C")
            .arg(worktree)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn write(worktree: &Path, rel: &str, contents: &str) {
        let path = worktree.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn blocked_recovery_fixture(
        worktree: &Path,
        session: &str,
        required_commands: Vec<execution_state::RequiredRecoveryCommand>,
        tracked_event: bool,
    ) {
        crate::cli::trusted_store::init_git_repo_with_origin(worktree);
        if tracked_event {
            write(worktree, ".gwt/work/events.jsonl", "{\"event\":\"base\"}\n");
            write(
                worktree,
                ".gwt/work/board-remote-roots.jsonl",
                "{\"root\":\"base\"}\n",
            );
            git(
                worktree,
                &[
                    "add",
                    "-f",
                    ".gwt/work/events.jsonl",
                    ".gwt/work/board-remote-roots.jsonl",
                ],
            );
            git(worktree, &["commit", "-qm", "chore(work): seed event"]);
        }
        git(
            worktree,
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        git(worktree, &["checkout", "-q", "-b", "work/recovery"]);
        execution_state::materialize_at_launch(
            worktree,
            execution_state::ExecutionOwnerKind::Spec,
            3248,
            session,
            "$gwt-execute",
            false,
        )
        .unwrap();
        let mut env = crate::cli::TestEnv::new(worktree.to_path_buf());
        let (code, output) = crate::cli::run_collect(
            &mut env,
            crate::cli::CliCommand::Execution(execution_state::ExecutionCommand::Blocked {
                reason: "external verifier unavailable".to_string(),
                missing_verification: Some("required recovery verifier".to_string()),
                required_recovery_commands: Some(required_commands),
            }),
        )
        .unwrap();
        assert_eq!(code, 0, "{output}");
    }

    fn worktree_command(command: &str) -> execution_state::RequiredRecoveryCommand {
        execution_state::RequiredRecoveryCommand {
            execution_root: execution_state::RecoveryExecutionRoot::Worktree,
            command: command.to_string(),
        }
    }

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
                commands: commands.to_vec(),
                derived: false,
                worktree_fingerprint: String::new(),
                no_change_evidence: None,
                created_at: Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        run_verification(worktree, session, commands).unwrap()
    }

    // T-327 / AS-180: a clean blocked worktree derives a real floor first,
    // then the immutable blocker requirements. The exact plan executes and
    // remains covered; a run may add commands without changing membership.
    #[test]
    fn clean_recovery_derives_exact_non_vacuous_floor_and_requirements() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-recovery");
        let dir = tempfile::tempdir().unwrap();
        blocked_recovery_fixture(
            dir.path(),
            "sess-recovery",
            vec![
                worktree_command("git --version"),
                worktree_command("git --version"),
                worktree_command("git --exec-path"),
            ],
            false,
        );

        let (derived, plan) = derive_and_register_plan(dir.path(), "sess-recovery").unwrap();
        let evidence = plan
            .no_change_evidence
            .as_ref()
            .expect("clean recovery must bind no-change evidence");
        assert_eq!(
            evidence.classifier_version,
            crate::cli::verify_derivation::BOOKKEEPING_CLASSIFIER_VERSION
        );
        assert!(!evidence.integration_base.is_empty());
        assert_eq!(plan.commands, derived.commands);
        assert_eq!(plan.commands.len(), 3, "floor + two exact requirements");
        assert_eq!(
            plan.commands[0],
            crate::cli::verify_derivation::no_change_floor_command(&evidence.integration_base)
        );
        assert_eq!(&plan.commands[1..], ["git --version", "git --exec-path"]);

        let mut ran = plan.commands.clone();
        ran.push("git --html-path".to_string());
        let (run, transcript) = run_verification(dir.path(), "sess-recovery", &ran).unwrap();
        assert!(run.all_passed, "{transcript}");
        assert!(run.plan_covered, "{transcript}");
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-recovery", Some(3248)),
            EvidenceStatus::Fresh
        );
    }

    // FR-200 / AS-172: adding an ordinary source surface must not bypass the
    // blocker-specific verifier. Canonical derivation appends every missing
    // required command, and reopen validation rejects an omitted command.
    #[test]
    fn ordinary_recovery_matrix_still_requires_blocker_commands() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-ordinary");
        let dir = tempfile::tempdir().unwrap();
        blocked_recovery_fixture(
            dir.path(),
            "sess-ordinary",
            vec![worktree_command("git --version")],
            false,
        );
        write(
            dir.path(),
            "crates/gwt-core/src/lib.rs",
            "pub fn changed() {}\n",
        );

        let (_, plan) = derive_and_register_plan(dir.path(), "sess-ordinary").unwrap();
        assert!(plan.no_change_evidence.is_none());
        assert_eq!(
            plan.commands.last().map(String::as_str),
            Some("git --version")
        );

        let record = execution_state::load(dir.path()).unwrap().unwrap();
        let mut omitted = plan;
        omitted
            .commands
            .retain(|command| command != "git --version");
        let error =
            crate::cli::verify_derivation::validate_recovery_plan(dir.path(), &record, &omitted)
                .unwrap_err();
        assert!(error.contains("Required Recovery Command Set"), "{error}");
    }

    // T-328 / AS-180: a blocker-specific verifier is not replaceable by a
    // passing generic command. The same immutable plan fails before the
    // external state changes, passes afterward, and alone authorizes reopen.
    #[test]
    fn external_blocker_must_really_clear_before_same_session_reopen() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-external");
        let dir = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let sentinel = external.path().join("runner-ready");
        let required = format!("test -f {}", sentinel.display());
        blocked_recovery_fixture(
            dir.path(),
            "sess-external",
            vec![worktree_command(&required)],
            false,
        );

        let (_, exact_plan) = derive_and_register_plan(dir.path(), "sess-external").unwrap();
        let blocked_before_substitution = execution_state::load(dir.path()).unwrap().unwrap();
        let mut substituted = exact_plan.clone();
        substituted.commands = vec![exact_plan.commands[0].clone(), "git --version".to_string()];
        save_plan(dir.path(), &substituted).unwrap();
        let (substituted_run, substituted_transcript) =
            run_verification(dir.path(), "sess-external", &substituted.commands).unwrap();
        assert!(substituted_run.all_passed, "{substituted_transcript}");
        assert!(!substituted_run.plan_covered, "{substituted_transcript}");
        let mut env = crate::cli::TestEnv::new(dir.path().to_path_buf());
        let (code, output) = crate::cli::run_collect(
            &mut env,
            crate::cli::CliCommand::Execution(execution_state::ExecutionCommand::Reopen {
                reason: "always-green substitute passed".to_string(),
            }),
        )
        .unwrap();
        assert_eq!(code, 2, "{output}");
        assert_eq!(
            execution_state::load(dir.path()).unwrap().unwrap(),
            blocked_before_substitution,
            "substituted evidence must leave the Blocked record unchanged"
        );

        let (_, plan) = derive_and_register_plan(dir.path(), "sess-external").unwrap();
        let (before, before_transcript) =
            run_verification(dir.path(), "sess-external", &plan.commands).unwrap();
        assert!(!before.all_passed, "{before_transcript}");
        assert!(before.plan_covered, "{before_transcript}");

        std::fs::write(&sentinel, "ready\n").unwrap();
        let (after, after_transcript) =
            run_verification(dir.path(), "sess-external", &plan.commands).unwrap();
        assert!(after.all_passed, "{after_transcript}");
        assert!(after.plan_covered, "{after_transcript}");

        let (code, output) = crate::cli::run_collect(
            &mut env,
            crate::cli::CliCommand::Execution(execution_state::ExecutionCommand::Reopen {
                reason: "external runner became available".to_string(),
            }),
        )
        .unwrap();
        assert_eq!(code, 0, "{output}");
        let record = execution_state::load(dir.path()).unwrap().unwrap();
        assert_eq!(
            record.status,
            execution_state::ExecutionControlStatus::Active
        );
        assert_eq!(record.recoveries.len(), 1);
        assert_eq!(record.recoveries[0].verification_record_id, after.record_id);

        // A later Active -> Blocked transition starts a new blocked lifetime:
        // the prior lifetime stayed immutable, but the new blocker must bind
        // its own verifier instead of being forced to reuse stale semantics.
        let (code, output) = crate::cli::run_collect(
            &mut env,
            crate::cli::CliCommand::Execution(execution_state::ExecutionCommand::Blocked {
                reason: "a different verifier became unavailable".to_string(),
                missing_verification: None,
                required_recovery_commands: Some(vec![worktree_command("git --exec-path")]),
            }),
        )
        .unwrap();
        assert_eq!(code, 0, "{output}");
        let record = execution_state::load(dir.path()).unwrap().unwrap();
        assert_eq!(
            record.status,
            execution_state::ExecutionControlStatus::Blocked
        );
        assert_eq!(
            record.required_recovery_commands,
            vec![worktree_command("git --exec-path")]
        );
    }

    // T-327: both working-tree and committed work journals are classified
    // before filtering and still produce the same no-change floor.
    #[test]
    fn tracked_events_only_states_derive_no_change_evidence() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-events");

        for committed in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            blocked_recovery_fixture(
                dir.path(),
                "sess-events",
                vec![worktree_command("git --version")],
                true,
            );
            write(
                dir.path(),
                ".gwt/work/events.jsonl",
                "{\"event\":\"base\"}\n{\"event\":\"settled\"}\n",
            );
            write(
                dir.path(),
                ".gwt/work/board-remote-roots.jsonl",
                "{\"root\":\"base\"}\n{\"root\":\"settled\"}\n",
            );
            if committed {
                git(
                    dir.path(),
                    &[
                        "add",
                        "-f",
                        ".gwt/work/events.jsonl",
                        ".gwt/work/board-remote-roots.jsonl",
                    ],
                );
                git(dir.path(), &["commit", "-qm", "chore(work): settle event"]);
            }

            let (_, plan) = derive_and_register_plan(dir.path(), "sess-events").unwrap();
            assert!(plan.no_change_evidence.is_some(), "committed={committed}");
            assert_eq!(plan.commands.len(), 2, "committed={committed}");

            let (run, transcript) =
                run_verification(dir.path(), "sess-events", &plan.commands).unwrap();
            assert!(run.all_passed, "committed={committed}: {transcript}");
            assert!(run.plan_covered, "committed={committed}: {transcript}");

            let mut env = crate::cli::TestEnv::new(dir.path().to_path_buf());
            let (code, output) = crate::cli::run_collect(
                &mut env,
                crate::cli::CliCommand::Execution(execution_state::ExecutionCommand::Reopen {
                    reason: "work journals settled".to_string(),
                }),
            )
            .unwrap();
            assert_eq!(code, 0, "committed={committed}: {output}");
            assert_eq!(
                execution_state::load(dir.path()).unwrap().unwrap().status,
                execution_state::ExecutionControlStatus::Active,
                "committed={committed}"
            );
        }
    }

    // T-327 / AS-181: legacy terminal blocks cannot gain requirements at
    // derive time; current no-change recovery must direct a fresh launch.
    #[test]
    fn legacy_requirement_gap_refuses_no_change_derivation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-legacy");
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        git(dir.path(), &["checkout", "-q", "-b", "work/legacy"]);
        execution_state::materialize_at_launch(
            dir.path(),
            execution_state::ExecutionOwnerKind::Spec,
            3248,
            "sess-legacy",
            "$gwt-execute",
            false,
        )
        .unwrap();
        execution_state::settle(
            dir.path(),
            "sess-legacy",
            execution_state::ExecutionSettlement::Blocked {
                reason: "legacy blocker".to_string(),
                missing_verification: None,
            },
        )
        .unwrap();

        let err = derive_and_register_plan(dir.path(), "sess-legacy").unwrap_err();
        assert!(err.contains("Legacy Requirement Gap"), "{err}");
        assert!(err.contains("fresh linked-owner launch"), "{err}");
    }

    // T-327 / FR-201: plan membership is an exact union. A plan-level
    // superset is rejected even though verify.run itself may execute a
    // superset, and integration-base drift invalidates provenance without a
    // worktree-content change.
    #[test]
    fn no_change_plan_rejects_membership_substitution_and_base_drift() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-exact");
        let dir = tempfile::tempdir().unwrap();
        blocked_recovery_fixture(
            dir.path(),
            "sess-exact",
            vec![worktree_command("git --version")],
            false,
        );
        git(
            dir.path(),
            &["commit", "--allow-empty", "-qm", "chore: second base"],
        );
        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        let (_, plan) = derive_and_register_plan(dir.path(), "sess-exact").unwrap();
        let record = execution_state::load(dir.path()).unwrap().unwrap();

        let mut floor_only = plan.clone();
        floor_only.commands.truncate(1);
        let err =
            crate::cli::verify_derivation::validate_recovery_plan(dir.path(), &record, &floor_only)
                .unwrap_err();
        assert!(err.contains("exact canonical"), "{err}");

        let mut requirement_only = plan.clone();
        requirement_only.commands.remove(0);
        assert!(crate::cli::verify_derivation::validate_recovery_plan(
            dir.path(),
            &record,
            &requirement_only,
        )
        .is_err());

        let mut plan_superset = plan.clone();
        plan_superset.commands.push("git --exec-path".to_string());
        assert!(crate::cli::verify_derivation::validate_recovery_plan(
            dir.path(),
            &record,
            &plan_superset,
        )
        .is_err());

        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD^"],
        );
        let err = crate::cli::verify_derivation::validate_recovery_plan(dir.path(), &record, &plan)
            .unwrap_err();
        assert!(err.contains("integration-base identity changed"), "{err}");
    }

    #[test]
    fn no_change_plan_rejects_post_derivation_non_bookkeeping_untracked_file() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-drift");
        let dir = tempfile::tempdir().unwrap();
        blocked_recovery_fixture(
            dir.path(),
            "sess-drift",
            vec![worktree_command("git --version")],
            false,
        );
        let (_, plan) = derive_and_register_plan(dir.path(), "sess-drift").unwrap();
        write(dir.path(), "post-plan.rs", "fn drift() {}\n");

        let (run, transcript) = run_verification(dir.path(), "sess-drift", &plan.commands).unwrap();
        assert!(run.all_passed, "the floor itself may pass: {transcript}");
        assert!(
            !run.plan_covered,
            "provenance drift must invalidate coverage"
        );
        assert_eq!(
            evaluate_evidence(dir.path(), "sess-drift", Some(3248)),
            EvidenceStatus::PlanNotCovered
        );
    }

    fn passing_record(session: &str, fingerprint: &str) -> VerificationRunRecord {
        VerificationRunRecord {
            record_id: "vr-test".to_string(),
            session_id: session.to_string(),
            owner_number: Some(3248),
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
            commands: vec!["cargo test -p gwt --lib".to_string()],
            derived: false,
            worktree_fingerprint: String::new(),
            no_change_evidence: None,
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
            commands: vec!["git --version".to_string()],
            derived: false,
            worktree_fingerprint: String::new(),
            no_change_evidence: None,
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
                commands: vec!["git --version".to_string()],
                derived: true,
                worktree_fingerprint: String::new(),
                no_change_evidence: None,
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
                commands: vec!["git --version".to_string(), "git --exec-path".to_string()],
                derived: false,
                worktree_fingerprint: String::new(),
                no_change_evidence: None,
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
                commands: vec!["git --version".to_string()],
                derived: false,
                worktree_fingerprint: String::new(),
                no_change_evidence: None,
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
                commands: commands.clone(),
                derived: true,
                worktree_fingerprint: String::new(),
                no_change_evidence: None,
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
                commands: vec!["git --exec-path".to_string()],
                derived: true,
                worktree_fingerprint: String::new(),
                no_change_evidence: None,
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
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let commands = vec!["git --version".to_string()];
        save_plan(
            dir.path(),
            &VerificationPlanRecord {
                session_id: "sess-1".to_string(),
                owner_number: None,
                commands: commands.clone(),
                derived: true,
                worktree_fingerprint: String::new(),
                no_change_evidence: None,
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
}
