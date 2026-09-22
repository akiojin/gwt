//! Permission-readiness gate records (Issue #4544).
//!
//! Issue #4543 answers "does this launch run without permission prompts?" and
//! records the verdict. It deliberately stops there: an `unsupported_provider`
//! / `missing_custom_skip_mapping` launch still starts, it just stops being
//! invisible. This module is the other half — the durable record a gate writes
//! when it *refuses*, and that every later gate reads to stay refused.
//!
//! Two things are recorded here, because they are the same fact observed at
//! two different moments:
//!
//! * [`PermissionReadinessKind::PreLaunchBlock`] — the launch was refused
//!   before it produced anything, and
//! * [`PermissionReadinessKind::PromptRegression`] — the launch was accepted,
//!   and then a provider permission prompt appeared anyway. The contract said
//!   this session runs unattended; a prompt proves it does not.
//!
//! # Why this is not a field on the Execution Control Record
//!
//! `execution_state::compute_content_hash` hashes the *deserialized struct*
//! re-serialized, and `execution_state::load` parses with plain serde. A
//! binary that does not know a field drops it, recomputes a different hash,
//! and reports the record `corrupt` with an empty `available_recoveries`.
//! Measured against the shipped gwtd 9.101.1 while designing this module:
//! injecting one unknown field (with the hash a newer writer would have
//! stamped) turned `ecr_status: active` into `ecr_status: corrupt`,
//! `binding_cause: execution_control_integrity_failure`, and left no recovery
//! at all. That is the Issue #4590 failure class.
//!
//! The `recoveries` field carries its own hash chain and is folded into
//! `transfers` by `recovery_storage_projection` for exactly this reason. Rather
//! than grow a third exception inside that record, permission-readiness lives
//! in its own trusted-store file with its own integrity hash, and the Execution
//! Control Record's body hash does not move by a single bit.

use std::{
    io::{self, ErrorKind},
    path::Path,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Trusted-store file name. Mirrored into the worktree like the obligation
/// state so a hook process that cannot resolve the trusted dir still sees it.
const PERMISSION_READINESS_FILE: &str = "permission-readiness.json";

/// Worktree-relative mirror path.
pub const PERMISSION_READINESS_STATE_RELATIVE: &str = ".gwt/skill-state/permission-readiness.json";

/// Which moment the gate refused at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionReadinessKind {
    /// Refused before the launch produced anything observable.
    PreLaunchBlock,
    /// The launch was accepted and then prompted anyway.
    ///
    /// Named in full on the wire: `permission_prompt_regression` is the
    /// vocabulary Issue #4544 AC-3 defines, and a bare `prompt_regression`
    /// in `execution.status` would not match what anyone is looking for.
    #[serde(rename = "permission_prompt_regression")]
    PromptRegression,
}

impl PermissionReadinessKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreLaunchBlock => "pre_launch_block",
            Self::PromptRegression => "permission_prompt_regression",
        }
    }
}

/// One permission-readiness gate decision.
///
/// Every field here exists because Issue #4544 AC-4 names it: the gate id, the
/// owner and session it is about, the provider, the permission mode that was
/// expected against the one actually observed, and a recovery action an
/// operator can carry out. Nothing here describes progress — a record of this
/// kind must never read as though implementation started or tests ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionReadinessRecord {
    pub schema_version: u32,
    /// Stable identifier for this gate decision, safe to quote in a report.
    pub gate_id: String,
    pub kind: PermissionReadinessKind,
    pub owner_kind: String,
    pub owner_number: u64,
    pub session_id: String,
    pub provider: String,
    /// The permission mode the producing-work contract required.
    pub expected_permission_mode: String,
    /// The permission mode actually in force.
    pub observed_permission_mode: String,
    pub reason: String,
    pub recovery_action: String,
    /// What was observed, verbatim enough to argue with. For a prompt
    /// regression this is the provider prompt's fingerprint, not its text —
    /// pane contents can carry secrets.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub evidence: String,
    pub recorded_at: DateTime<Utc>,
    /// Integrity hash over this record with the field emptied.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
}

impl PermissionReadinessRecord {
    /// Whether the stored hash still matches the content.
    #[must_use]
    pub fn integrity_ok(&self) -> bool {
        !self.content_hash.is_empty() && self.content_hash == compute_content_hash(self)
    }

    /// The operator-facing line. Deliberately states only what was observed
    /// and what to do; it never implies that any work was attempted.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "{} [{}] provider {} — expected permission mode `{}`, observed `{}`: {} Recovery: {}",
            self.kind.as_str(),
            self.gate_id,
            self.provider,
            self.expected_permission_mode,
            self.observed_permission_mode,
            self.reason,
            self.recovery_action,
        )
    }
}

#[must_use]
fn compute_content_hash(record: &PermissionReadinessRecord) -> String {
    let mut canonical = record.clone();
    canonical.content_hash = String::new();
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&canonical).unwrap_or_default())
    )
}

/// Build a gate id that is stable for the same owner, session, and kind, so a
/// repeated observation names the same gate instead of inventing a new one.
#[must_use]
pub fn gate_id(kind: PermissionReadinessKind, owner_number: u64, session_id: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(session_id.as_bytes()));
    format!(
        "permission-readiness:{}:{owner_number}:{}",
        kind.as_str(),
        &digest[..12]
    )
}

/// Stamp the integrity hash and persist the record.
///
/// Recording is last-write-wins on purpose: a gate decision describes the
/// current state of one execution, and an execution that prompts twice is not
/// two blockers. Callers hold the trusted-store write lease.
pub fn save(worktree: &Path, record: &PermissionReadinessRecord) -> io::Result<()> {
    let mut record = record.clone();
    record.content_hash = compute_content_hash(&record);
    let serialized = serde_json::to_vec_pretty(&record)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    crate::cli::trusted_store::write_with_mirror(
        worktree,
        PERMISSION_READINESS_FILE,
        &worktree.join(PERMISSION_READINESS_STATE_RELATIVE),
        &serialized,
    )
}

/// Read the current record, if any.
///
/// A record whose integrity hash does not match is returned as `Ok(None)`
/// rather than an error: this file gates delivery, so a hand-edited or
/// truncated copy must not be able to *create* a blocker either. The
/// Execution Control Record remains the integrity-protected identity; this is
/// an observation about it.
pub fn load(worktree: &Path) -> io::Result<Option<PermissionReadinessRecord>> {
    // Trusted copy authoritative, mirror fallback — the P9b convention the
    // obligation and execution records already follow, so a worktree with no
    // resolvable trusted directory still reports its own gate decision.
    let contents = match crate::cli::trusted_store::read(worktree, PERMISSION_READINESS_FILE)? {
        Some(contents) => contents,
        None if crate::cli::trusted_store::under_trusted_management(worktree) => return Ok(None),
        None => match std::fs::read_to_string(worktree.join(PERMISSION_READINESS_STATE_RELATIVE)) {
            Ok(contents) => contents,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        },
    };
    let Ok(record) = serde_json::from_str::<PermissionReadinessRecord>(&contents) else {
        return Ok(None);
    };
    if !record.integrity_ok() {
        return Ok(None);
    }
    Ok(Some(record))
}

/// Record that a provider permission prompt appeared on a launch that the
/// producing-work contract required to be prompt-free (AC-3).
pub fn record_prompt_regression(
    worktree: &Path,
    owner_kind: &str,
    owner_number: u64,
    session_id: &str,
    provider: &str,
    prompt_fingerprint: u64,
) -> io::Result<PermissionReadinessRecord> {
    let mut record = PermissionReadinessRecord {
        schema_version: 1,
        gate_id: gate_id(
            PermissionReadinessKind::PromptRegression,
            owner_number,
            session_id,
        ),
        kind: PermissionReadinessKind::PromptRegression,
        owner_kind: owner_kind.to_string(),
        owner_number,
        session_id: session_id.to_string(),
        provider: provider.to_string(),
        expected_permission_mode: "skip-permissions (producing-work launch)".to_string(),
        observed_permission_mode: "interactive (provider asked for approval)".to_string(),
        reason: format!(
            "this producing-work launch was accepted as prompt-free, and {provider} asked for approval anyway, so nobody is watching the question it is waiting on"
        ),
        recovery_action:
            "Answer or dismiss the prompt in the agent window, then relaunch this owner with a provider whose skip-permissions mapping is applied. `gwt-execute` on a relaunched window clears this record."
                .to_string(),
        evidence: format!("provider approval prompt fingerprint {prompt_fingerprint:016x}"),
        recorded_at: Utc::now(),
        content_hash: String::new(),
    };
    // Stamped here as well as inside `save` so the value handed back to the
    // caller is the one on disk — a returned record with an empty hash would
    // fail its own `integrity_ok`.
    record.content_hash = compute_content_hash(&record);
    crate::cli::trusted_store::with_write_lease(worktree, || save(worktree, &record))?;
    Ok(record)
}

/// The refusal a settlement gate emits while a permission-readiness record
/// stands, or `None` when the execution is clear.
///
/// Issue #4544 AC-3: verification, completion, PR Ready, Deliver, merge, and
/// autonomous evidence all consume this. One predicate so they cannot drift
/// into disagreeing about whether the same execution may settle.
#[must_use]
pub fn settlement_refusal(worktree: &Path) -> Option<String> {
    let record = load(worktree).ok().flatten()?;
    Some(format!(
        "permission readiness refused this settlement — {}",
        record.describe()
    ))
}

/// The same refusal, addressed by owner rather than by worktree.
///
/// The Issue Monitor scans from the project root, so it never holds the work
/// worktree's path. The owner generation ledger does — `diagnose_owner` is
/// repository-scoped and reports the holder worktree — and that is the only
/// way Deliver and merge can consult a record written in a different
/// directory by a different process (Issue #4544 AC-3).
#[must_use]
pub fn owner_settlement_refusal(project_root: &Path, owner_number: u64) -> Option<String> {
    let diagnosis = crate::cli::execution_state::diagnose_owner(
        project_root,
        crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: owner_number,
        },
    );
    let holder = diagnosis.holder_worktree?;
    settlement_refusal(Path::new(&holder))
}

/// Clear the record. Used when a relaunch re-establishes the contract.
pub fn clear(worktree: &Path) -> io::Result<()> {
    let Some(trusted_dir) = crate::cli::trusted_store::trusted_dir_for_worktree(worktree) else {
        return Ok(());
    };
    let path = trusted_dir.join(PERMISSION_READINESS_FILE);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mirror = worktree.join(PERMISSION_READINESS_STATE_RELATIVE);
    match std::fs::remove_file(&mirror) {
        Ok(()) | Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        dir
    }

    /// AC-3: a prompt on a producing-work launch is recorded, with a gate id,
    /// the provider, both permission modes, and a recovery action.
    #[test]
    fn a_prompt_regression_is_recorded_with_everything_ac4_names() {
        let dir = fixture();

        let recorded =
            record_prompt_regression(dir.path(), "issue", 4544, "sess-4544", "codex", 0xdead_beef)
                .expect("record the regression");

        let loaded = load(dir.path()).expect("load").expect("a record stands");
        assert_eq!(loaded, recorded);
        assert!(loaded.integrity_ok());
        assert_eq!(loaded.kind, PermissionReadinessKind::PromptRegression);
        assert_eq!(loaded.owner_number, 4544);
        assert_eq!(loaded.session_id, "sess-4544");
        assert_eq!(loaded.provider, "codex");
        assert!(loaded.gate_id.starts_with("permission-readiness:permission_prompt_regression:4544:"));
        assert!(!loaded.expected_permission_mode.is_empty());
        assert!(!loaded.observed_permission_mode.is_empty());
        assert!(!loaded.recovery_action.is_empty());
        assert!(
            loaded.evidence.contains("00000000deadbeef"),
            "the fingerprint is the evidence, not the pane text: {}",
            loaded.evidence
        );
    }

    /// AC-4: the rendered line must not read as though work happened.
    #[test]
    fn the_rendered_gate_line_never_claims_work_was_attempted() {
        let dir = fixture();
        let record =
            record_prompt_regression(dir.path(), "issue", 4544, "sess-4544", "codex", 1).unwrap();

        let described = record.describe();

        assert!(described.contains(&record.gate_id));
        assert!(described.contains("codex"));
        assert!(described.contains("Recovery:"));
        for misleading in [
            "implementation started",
            "tests ran",
            "tests passed",
            "verification passed",
            "in progress",
        ] {
            assert!(
                !described.to_ascii_lowercase().contains(misleading),
                "the gate line must not imply {misleading}: {described}"
            );
        }
    }

    /// AC-3: every settlement consumes one predicate, and it refuses while the
    /// record stands.
    #[test]
    fn settlement_refusal_stands_until_the_record_is_cleared() {
        let dir = fixture();
        assert!(settlement_refusal(dir.path()).is_none());

        record_prompt_regression(dir.path(), "issue", 4544, "sess-4544", "codex", 1).unwrap();
        let refusal = settlement_refusal(dir.path()).expect("the settlement is refused");
        assert!(refusal.contains("permission readiness refused this settlement"));
        assert!(refusal.contains("permission_prompt_regression"));

        clear(dir.path()).expect("clear");
        assert!(settlement_refusal(dir.path()).is_none());
    }

    /// A hand-edited record must not be able to invent a blocker either.
    #[test]
    fn a_record_whose_hash_does_not_match_creates_no_blocker() {
        let dir = fixture();
        record_prompt_regression(dir.path(), "issue", 4544, "sess-4544", "codex", 1).unwrap();

        let trusted = crate::cli::trusted_store::trusted_dir_for_worktree(dir.path())
            .expect("trusted dir");
        let path = trusted.join(PERMISSION_READINESS_FILE);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        value["owner_number"] = serde_json::json!(9999);
        std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        assert!(load(dir.path()).expect("load").is_none());
        assert!(settlement_refusal(dir.path()).is_none());
    }

    /// The same owner and session name the same gate across observations.
    #[test]
    fn the_gate_id_is_stable_for_the_same_owner_and_session() {
        let first = gate_id(PermissionReadinessKind::PromptRegression, 4544, "sess-a");
        let again = gate_id(PermissionReadinessKind::PromptRegression, 4544, "sess-a");
        let other_session = gate_id(PermissionReadinessKind::PromptRegression, 4544, "sess-b");
        let other_kind = gate_id(PermissionReadinessKind::PreLaunchBlock, 4544, "sess-a");

        assert_eq!(first, again);
        assert_ne!(first, other_session);
        assert_ne!(first, other_kind);
    }
}
