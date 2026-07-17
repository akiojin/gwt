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
//! P8a scope notes (dependent follow-ups, phase contract T-263):
//! - Evidence manifests / provenance validation (T-110..T-114) are not here;
//!   settlement is session-bound state only.
//! - The record is worktree-local (`.gwt/skill-state/execution-control.json`);
//!   the repo-scoped trusted store (T-172+) and authorized ownership transfer
//!   / concurrent-owner rejection (T-117/T-118) arrive with P9. A fresh
//!   relaunch takes over with a fresh active record; a resume preserves an
//!   existing settled record for the same owner.
//! - Like the P7A intake state, saves are atomic file replacements but the
//!   read-modify-write cycle is not serialized across processes; P9's write
//!   lease owns real serialization.

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use gwt_github::{client::ApiError, SpecOpsError};
use serde::{Deserialize, Serialize};

use super::CliEnv;

/// Worktree-relative path of the Execution Control Record.
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
}

/// Resolve the record path for a worktree.
#[must_use]
pub fn state_path(worktree: &Path) -> PathBuf {
    worktree.join(EXECUTION_CONTROL_STATE_RELATIVE)
}

/// Load the record. `Ok(None)` when missing; malformed JSON and I/O failures
/// propagate so hook readers can fail open while writers surface the error.
pub fn load(worktree: &Path) -> io::Result<Option<ExecutionControlRecord>> {
    let path = state_path(worktree);
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let record = serde_json::from_str::<ExecutionControlRecord>(&contents)
                .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
            Ok(Some(record))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Persist the record atomically (hooks read this file concurrently).
pub fn save(worktree: &Path, record: &ExecutionControlRecord) -> io::Result<()> {
    let path = state_path(worktree);
    let serialized = serde_json::to_vec_pretty(record)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    gwt_github::cache::write_atomic(&path, &serialized)
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
    if resume {
        if let Ok(Some(existing)) = load(worktree) {
            if existing.owner_number == owner_number
                && existing.status != ExecutionControlStatus::Active
            {
                return Ok(());
            }
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
    let settlement = match command {
        ExecutionCommand::Complete => ExecutionSettlement::Completed,
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
                "execution: settlement refused — record belongs to session {record_session_id}, not the current session\n",
            ));
            Ok(2)
        }
    }
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
        }
    }

    // T-106: roundtrip with owner kind/number, primary session id,
    // entrypoint, bundled-required owners, state, and timestamps.
    #[test]
    fn record_roundtrips_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let record = active_record("sess-1");
        save(dir.path(), &record).unwrap();
        assert_eq!(load(dir.path()).unwrap(), Some(record));
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

            let (code, out) = run_cmd(dir.path(), ExecutionCommand::Complete).unwrap();
            assert_eq!(code, 0, "{out}");
            assert!(out.contains("completed"), "{out}");
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Completed
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

            // Real finalize for the SAME owner — settles.
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
            assert_eq!(
                load(dir.path()).unwrap().unwrap().status,
                ExecutionControlStatus::Completed,
                "a real matching finalize must settle the execution"
            );
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
