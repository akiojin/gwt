//! Worktree-local intake outcome state (SPEC-3248 P7A).
//!
//! Intake (Curate) sessions must leave a durable Issue/SPEC outcome — or an
//! explicit, reasoned No Action — before Stop. This module owns the state
//! that backs that contract:
//!
//! - **FR-012**: `intake.outcome.record` persists the current session's
//!   outcome (`issue_created` / `issue_updated` / `spec_created` /
//!   `spec_updated` / `no_action`). Issue/SPEC outcomes require the owner
//!   `number`; `no_action` requires a non-empty `reason`.
//! - **FR-013**: successful `issue.create` / `issue.comment` /
//!   `issue.spec.create` / `issue.spec.edit` auto-record outcomes for intake
//!   sessions. Board posts never write this state.
//! - **FR-016**: every intake `UserPromptSubmit` marks the artifact
//!   requirement dirty by storing `required_since`; Stop passes only when a
//!   valid outcome has `recorded_at >= required_since`.
//!
//! The state lives at `.gwt/skill-state/intake-outcome.json`, keeping the
//! worktree-local state-file convention of [`gwt_core::skill_state`]. Load
//! errors are surfaced so hook callers can fail open (never mis-block) while
//! explicit writers stay strict (FR-012 validation).
//!
//! Concurrency: every save is an atomic file replacement, but the
//! read-modify-write cycle is not serialized across processes (the
//! UserPromptSubmit hook and gwtd CLI operations are separate processes).
//! The sub-millisecond lost-update window this leaves is a known, accepted
//! bound for P7A — SPEC-3248 P9's Artifact Write Transaction / write-lease
//! machinery owns real cross-process serialization as a dependent follow-up.

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use gwt_github::{client::ApiError, SpecOpsError};
use serde::{Deserialize, Serialize};

use super::CliEnv;

/// Worktree-relative path of the intake outcome state file.
pub const INTAKE_OUTCOME_STATE_RELATIVE: &str = ".gwt/skill-state/intake-outcome.json";

/// The durable outcome kinds an intake session can settle on (FR-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntakeOutcomeKind {
    IssueCreated,
    IssueUpdated,
    SpecCreated,
    SpecUpdated,
    NoAction,
}

impl IntakeOutcomeKind {
    /// Parse the `params.kind` string of `intake.outcome.record`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "issue_created" => Some(Self::IssueCreated),
            "issue_updated" => Some(Self::IssueUpdated),
            "spec_created" => Some(Self::SpecCreated),
            "spec_updated" => Some(Self::SpecUpdated),
            "no_action" => Some(Self::NoAction),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IssueCreated => "issue_created",
            Self::IssueUpdated => "issue_updated",
            Self::SpecCreated => "spec_created",
            Self::SpecUpdated => "spec_updated",
            Self::NoAction => "no_action",
        }
    }

    /// Issue/SPEC outcomes must name the owner they created or updated.
    #[must_use]
    pub fn requires_number(self) -> bool {
        !matches!(self, Self::NoAction)
    }
}

/// One recorded intake outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakeOutcome {
    pub kind: IntakeOutcomeKind,
    /// Issue/SPEC number for `issue_*` / `spec_*` kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    /// Mandatory non-empty reason for `no_action`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The operation that produced this outcome (`issue.create`,
    /// `issue.spec.edit`, `intake.outcome.record`, …) — audit context for the
    /// Stop gate message and later inspection.
    pub source_operation: String,
    pub recorded_at: DateTime<Utc>,
}

impl IntakeOutcome {
    /// FR-012 validation. Strict for explicit writes; the Stop gate also
    /// refuses to accept an invalid persisted outcome (AS-10).
    pub fn validate(&self) -> Result<(), String> {
        if self.kind.requires_number() && self.number.is_none() {
            return Err(format!(
                "outcome kind '{}' requires params.number (the Issue/SPEC number)",
                self.kind.as_str()
            ));
        }
        if self.kind == IntakeOutcomeKind::NoAction
            && self
                .reason
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
        {
            return Err("outcome kind 'no_action' requires a non-empty params.reason".to_string());
        }
        Ok(())
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }
}

/// Persisted per-worktree intake outcome state. One intake session owns a
/// worktree at a time; a new session taking over replaces the state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakeOutcomeState {
    pub session_id: String,
    /// FR-016 dirty marker: the latest user prompt time. Stop passes only
    /// when a valid outcome was recorded at or after this instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_since: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<IntakeOutcome>,
}

impl IntakeOutcomeState {
    /// FR-016: does this state hold a valid outcome fresh enough for the
    /// latest prompt? (`required_since` absent means no prompt marked the
    /// requirement dirty yet — any valid outcome passes.)
    #[must_use]
    pub fn has_fresh_valid_outcome(&self) -> bool {
        let Some(outcome) = &self.outcome else {
            return false;
        };
        if !outcome.is_valid() {
            return false;
        }
        match self.required_since {
            Some(required_since) => outcome.recorded_at >= required_since,
            None => true,
        }
    }
}

/// Resolve the state-file path for a worktree.
#[must_use]
pub fn state_path(worktree: &Path) -> PathBuf {
    worktree.join(INTAKE_OUTCOME_STATE_RELATIVE)
}

/// Load the intake outcome state. `Ok(None)` when the file is missing;
/// malformed JSON and I/O failures propagate so hook callers can fail open
/// while explicit writers surface the error.
pub fn load(worktree: &Path) -> io::Result<Option<IntakeOutcomeState>> {
    let path = state_path(worktree);
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let state = serde_json::from_str::<IntakeOutcomeState>(&contents)
                .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
            Ok(Some(state))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Persist the intake outcome state atomically (the Stop gate reads this
/// file concurrently with CLI writers, so partial writes must be impossible).
pub fn save(worktree: &Path, state: &IntakeOutcomeState) -> io::Result<()> {
    let path = state_path(worktree);
    let serialized = serde_json::to_vec_pretty(state)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    gwt_github::cache::write_atomic(&path, &serialized)
}

/// T-079 / FR-016: mark the artifact requirement dirty for `session_id` at
/// `at` (the user-prompt boundary). A same-session outcome is preserved — it
/// simply becomes stale relative to the new `required_since` — while a state
/// owned by a different (previous) session is replaced wholesale. A
/// malformed state file is replaced too: restoring the dirty marker restores
/// the gate.
pub fn mark_required_since(worktree: &Path, session_id: &str, at: DateTime<Utc>) -> io::Result<()> {
    let state = match load(worktree) {
        Ok(Some(existing)) if existing.session_id == session_id => IntakeOutcomeState {
            required_since: Some(at),
            ..existing
        },
        _ => IntakeOutcomeState {
            session_id: session_id.to_string(),
            required_since: Some(at),
            outcome: None,
        },
    };
    save(worktree, &state)
}

/// Record an outcome for `session_id` (explicit `intake.outcome.record` and
/// the FR-013 auto-record hooks). Validation is strict (FR-012); the
/// existing `required_since` for the same session is preserved so freshness
/// evaluation stays correct.
pub fn record_outcome(
    worktree: &Path,
    session_id: &str,
    outcome: IntakeOutcome,
) -> Result<(), String> {
    outcome.validate()?;
    let state = match load(worktree) {
        Ok(Some(existing)) if existing.session_id == session_id => IntakeOutcomeState {
            outcome: Some(outcome),
            ..existing
        },
        _ => IntakeOutcomeState {
            session_id: session_id.to_string(),
            required_since: None,
            outcome: Some(outcome),
        },
    };
    save(worktree, &state).map_err(|err| format!("failed to save intake outcome state: {err}"))
}

// ---------------------------------------------------------------------------
// CLI command surface (`intake.outcome.record`, FR-012)
// ---------------------------------------------------------------------------

/// Commands of the `intake.*` JSON operation family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntakeCommand {
    OutcomeRecord {
        kind: String,
        number: Option<u64>,
        reason: Option<String>,
    },
}

/// Run an `intake.*` command. `intake.outcome.record` binds the outcome to
/// the current `GWT_SESSION_ID` (an outcome that cannot be attributed to a
/// session is useless to the Stop gate, so a missing session id is an error
/// for this explicit write path).
pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: IntakeCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match command {
        IntakeCommand::OutcomeRecord {
            kind,
            number,
            reason,
        } => {
            let Some(kind) = IntakeOutcomeKind::parse(&kind) else {
                return Err(SpecOpsError::from(ApiError::Unexpected(format!(
                    "unknown outcome kind '{kind}' — expected issue_created, issue_updated, spec_created, spec_updated, or no_action"
                ))));
            };
            let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    SpecOpsError::from(ApiError::Unexpected(
                        "intake.outcome.record requires GWT_SESSION_ID to bind the outcome to the current session".to_string(),
                    ))
                })?;
            let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
            let outcome = IntakeOutcome {
                kind,
                number,
                reason,
                source_operation: "intake.outcome.record".to_string(),
                recorded_at: Utc::now(),
            };
            record_outcome(&worktree, &session_id, outcome)
                .map_err(|err| SpecOpsError::from(ApiError::Unexpected(err)))?;
            out.push_str(&format!(
                "recorded intake outcome '{}' for session {session_id}\n",
                kind.as_str()
            ));
            Ok(0)
        }
    }
}

// ---------------------------------------------------------------------------
// FR-013 auto-record (issue.create / issue.comment / issue.spec.create /
// issue.spec.edit success paths)
// ---------------------------------------------------------------------------

/// Best-effort auto-record after a successful Issue/SPEC operation (FR-013).
///
/// The gate is self-contained: it fires only when the resolved lane for the
/// worktree is intake and `GWT_SESSION_ID` is set, so GUI/argv invocations
/// and execution sessions never write outcome state. Failures are logged and
/// swallowed — the GitHub operation already succeeded and its exit code must
/// not change (Board posts never reach this path, FR-013/FR-017).
pub(crate) fn auto_record_issue_operation(
    repo_path: &Path,
    source_operation: &str,
    kind: IntakeOutcomeKind,
    number: u64,
) {
    let worktree = gwt_core::paths::resolve_current_worktree_root(repo_path);
    let profile = gwt_skills::resolve_lane_for_worktree(&worktree);
    if profile.id != "intake" {
        return;
    }
    let Some(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let outcome = IntakeOutcome {
        kind,
        number: Some(number),
        reason: None,
        source_operation: source_operation.to_string(),
        recorded_at: Utc::now(),
    };
    if let Err(error) = record_outcome(&worktree, &session_id, outcome) {
        tracing::warn!(%error, source_operation, "intake outcome auto-record failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 16, hour, minute, 0).unwrap()
    }

    fn issue_outcome(recorded_at: DateTime<Utc>) -> IntakeOutcome {
        IntakeOutcome {
            kind: IntakeOutcomeKind::IssueUpdated,
            number: Some(3248),
            reason: None,
            source_operation: "issue.comment".to_string(),
            recorded_at,
        }
    }

    // T-070: roundtrip with session_id, kind, number, reason, source
    // operation, and timestamps.
    #[test]
    fn state_roundtrips_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let state = IntakeOutcomeState {
            session_id: "sess-1".to_string(),
            required_since: Some(t(9, 0)),
            outcome: Some(IntakeOutcome {
                kind: IntakeOutcomeKind::NoAction,
                number: None,
                reason: Some("duplicate of #3248".to_string()),
                source_operation: "intake.outcome.record".to_string(),
                recorded_at: t(9, 5),
            }),
        };
        save(dir.path(), &state).unwrap();
        assert_eq!(load(dir.path()).unwrap(), Some(state));
    }

    #[test]
    fn load_returns_none_when_absent_and_invalid_data_when_malformed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), None);
        let path = state_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not json").unwrap();
        let err = load(dir.path()).expect_err("expected InvalidData");
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    // FR-012: no_action requires a non-empty reason.
    #[test]
    fn no_action_without_reason_is_invalid() {
        for reason in [None, Some(String::new()), Some("   ".to_string())] {
            let outcome = IntakeOutcome {
                kind: IntakeOutcomeKind::NoAction,
                number: None,
                reason,
                source_operation: "intake.outcome.record".to_string(),
                recorded_at: t(9, 0),
            };
            assert!(outcome.validate().is_err(), "{outcome:?}");
        }
    }

    // FR-012: Issue/SPEC outcomes require the owner number.
    #[test]
    fn issue_and_spec_kinds_require_number() {
        for kind in [
            IntakeOutcomeKind::IssueCreated,
            IntakeOutcomeKind::IssueUpdated,
            IntakeOutcomeKind::SpecCreated,
            IntakeOutcomeKind::SpecUpdated,
        ] {
            let outcome = IntakeOutcome {
                kind,
                number: None,
                reason: None,
                source_operation: "test".to_string(),
                recorded_at: t(9, 0),
            };
            assert!(outcome.validate().is_err(), "{kind:?} must require number");
        }
    }

    #[test]
    fn kind_parse_roundtrips_all_kinds_and_rejects_unknown() {
        for kind in [
            IntakeOutcomeKind::IssueCreated,
            IntakeOutcomeKind::IssueUpdated,
            IntakeOutcomeKind::SpecCreated,
            IntakeOutcomeKind::SpecUpdated,
            IntakeOutcomeKind::NoAction,
        ] {
            assert_eq!(IntakeOutcomeKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(IntakeOutcomeKind::parse("board_posted"), None);
        assert_eq!(IntakeOutcomeKind::parse(""), None);
    }

    // T-079: marking dirty preserves the same-session outcome (it becomes
    // stale), replaces another session's state, and replaces malformed state.
    #[test]
    fn mark_required_since_preserves_same_session_outcome() {
        let dir = tempfile::tempdir().unwrap();
        record_outcome(dir.path(), "sess-1", issue_outcome(t(9, 0))).unwrap();
        mark_required_since(dir.path(), "sess-1", t(10, 0)).unwrap();
        let state = load(dir.path()).unwrap().unwrap();
        assert_eq!(state.session_id, "sess-1");
        assert_eq!(state.required_since, Some(t(10, 0)));
        assert!(state.outcome.is_some(), "same-session outcome preserved");
        assert!(
            !state.has_fresh_valid_outcome(),
            "outcome recorded before the new prompt must be stale"
        );
    }

    #[test]
    fn mark_required_since_replaces_other_session_state() {
        let dir = tempfile::tempdir().unwrap();
        record_outcome(dir.path(), "sess-old", issue_outcome(t(9, 0))).unwrap();
        mark_required_since(dir.path(), "sess-new", t(10, 0)).unwrap();
        let state = load(dir.path()).unwrap().unwrap();
        assert_eq!(state.session_id, "sess-new");
        assert_eq!(state.outcome, None, "old session's outcome must not leak");
    }

    #[test]
    fn mark_required_since_replaces_malformed_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not json").unwrap();
        mark_required_since(dir.path(), "sess-1", t(10, 0)).unwrap();
        let state = load(dir.path()).unwrap().unwrap();
        assert_eq!(state.session_id, "sess-1");
        assert_eq!(state.required_since, Some(t(10, 0)));
    }

    // FR-016 freshness evaluation (AS-9 / AS-11).
    #[test]
    fn fresh_valid_outcome_passes_and_stale_or_missing_fails() {
        // No outcome → not fresh.
        let state = IntakeOutcomeState {
            session_id: "sess-1".to_string(),
            required_since: Some(t(10, 0)),
            outcome: None,
        };
        assert!(!state.has_fresh_valid_outcome());

        // Outcome recorded before the prompt → stale.
        let stale = IntakeOutcomeState {
            outcome: Some(issue_outcome(t(9, 59))),
            ..state.clone()
        };
        assert!(!stale.has_fresh_valid_outcome());

        // Outcome recorded at/after the prompt → fresh.
        let fresh = IntakeOutcomeState {
            outcome: Some(issue_outcome(t(10, 0))),
            ..state.clone()
        };
        assert!(fresh.has_fresh_valid_outcome());

        // Invalid persisted outcome never passes (AS-10).
        let invalid = IntakeOutcomeState {
            outcome: Some(IntakeOutcome {
                kind: IntakeOutcomeKind::NoAction,
                number: None,
                reason: Some(String::new()),
                source_operation: "intake.outcome.record".to_string(),
                recorded_at: t(10, 5),
            }),
            ..state
        };
        assert!(!invalid.has_fresh_valid_outcome());
    }

    // record_outcome preserves the same-session dirty marker so a
    // subsequent Stop evaluates freshness against the latest prompt.
    #[test]
    fn record_outcome_preserves_required_since_for_same_session() {
        let dir = tempfile::tempdir().unwrap();
        mark_required_since(dir.path(), "sess-1", t(10, 0)).unwrap();
        record_outcome(dir.path(), "sess-1", issue_outcome(t(10, 5))).unwrap();
        let state = load(dir.path()).unwrap().unwrap();
        assert_eq!(state.required_since, Some(t(10, 0)));
        assert!(state.has_fresh_valid_outcome());
    }

    #[test]
    fn record_outcome_rejects_invalid_outcomes_strictly() {
        let dir = tempfile::tempdir().unwrap();
        let err = record_outcome(
            dir.path(),
            "sess-1",
            IntakeOutcome {
                kind: IntakeOutcomeKind::NoAction,
                number: None,
                reason: None,
                source_operation: "intake.outcome.record".to_string(),
                recorded_at: t(9, 0),
            },
        )
        .expect_err("no_action without reason must be rejected");
        assert!(err.contains("reason"), "{err}");
        assert_eq!(load(dir.path()).unwrap(), None, "state must stay untouched");
    }

    // ------------------------------------------------------------------
    // T-072: `intake.outcome.record` command behavior
    // ------------------------------------------------------------------

    mod command {
        use super::*;
        use crate::cli::{run_collect, CliCommand, TestEnv};
        use gwt_core::test_support::ScopedEnvVar;

        fn run_record(
            repo: &Path,
            kind: &str,
            number: Option<u64>,
            reason: Option<&str>,
        ) -> Result<(i32, String), gwt_github::SpecOpsError> {
            let mut env = TestEnv::new(repo.to_path_buf());
            run_collect(
                &mut env,
                CliCommand::Intake(IntakeCommand::OutcomeRecord {
                    kind: kind.to_string(),
                    number,
                    reason: reason.map(str::to_string),
                }),
            )
        }

        #[test]
        fn record_op_persists_outcome_for_current_session() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-op");
            let dir = tempfile::tempdir().unwrap();

            let (code, out) =
                run_record(dir.path(), "no_action", None, Some("duplicate of #42")).unwrap();
            assert_eq!(code, 0);
            assert!(out.contains("no_action"), "{out}");

            let state = load(dir.path()).unwrap().unwrap();
            assert_eq!(state.session_id, "sess-op");
            let outcome = state.outcome.unwrap();
            assert_eq!(outcome.kind, IntakeOutcomeKind::NoAction);
            assert_eq!(outcome.reason.as_deref(), Some("duplicate of #42"));
            assert_eq!(outcome.source_operation, "intake.outcome.record");
        }

        // FR-012: strict validation for explicit writes.
        #[test]
        fn record_op_rejects_invalid_payloads() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-op");
            let dir = tempfile::tempdir().unwrap();

            // no_action without reason.
            assert!(run_record(dir.path(), "no_action", None, None).is_err());
            // Issue/SPEC kinds without number.
            assert!(run_record(dir.path(), "issue_updated", None, None).is_err());
            // Unknown kind.
            assert!(run_record(dir.path(), "board_posted", None, None).is_err());
            assert_eq!(load(dir.path()).unwrap(), None);
        }

        #[test]
        fn record_op_requires_session_id() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
            let dir = tempfile::tempdir().unwrap();

            let err = run_record(dir.path(), "no_action", None, Some("reason"))
                .expect_err("missing GWT_SESSION_ID must fail");
            assert!(err.to_string().contains("GWT_SESSION_ID"), "{err}");
        }
    }

    // ------------------------------------------------------------------
    // T-073: FR-013 auto-record from Issue/SPEC operation success paths
    // ------------------------------------------------------------------

    mod auto_record {
        use super::*;
        use gwt_core::test_support::ScopedEnvVar;
        use gwt_skills::{write_lane_file, EXECUTION_PROFILE, INTAKE_PROFILE};

        fn mk_worktree(profile: &gwt_skills::LaneProfile) -> tempfile::TempDir {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join(".gwt")).unwrap();
            write_lane_file(dir.path(), profile).unwrap();
            dir
        }

        #[test]
        fn records_for_intake_lane_with_session() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-auto");
            let dir = mk_worktree(&INTAKE_PROFILE);

            auto_record_issue_operation(
                dir.path(),
                "issue.comment",
                IntakeOutcomeKind::IssueUpdated,
                3248,
            );

            let state = load(dir.path()).unwrap().unwrap();
            assert_eq!(state.session_id, "sess-auto");
            let outcome = state.outcome.unwrap();
            assert_eq!(outcome.kind, IntakeOutcomeKind::IssueUpdated);
            assert_eq!(outcome.number, Some(3248));
            assert_eq!(outcome.source_operation, "issue.comment");
        }

        // FR-015: execution lane must not write outcome state.
        #[test]
        fn skips_execution_lane() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-auto");
            let dir = mk_worktree(&EXECUTION_PROFILE);

            auto_record_issue_operation(
                dir.path(),
                "issue.create",
                IntakeOutcomeKind::IssueCreated,
                7,
            );
            assert_eq!(load(dir.path()).unwrap(), None);
        }

        // Without a session id the outcome cannot be attributed — skip.
        #[test]
        fn skips_without_session_id() {
            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
            let dir = mk_worktree(&INTAKE_PROFILE);

            auto_record_issue_operation(
                dir.path(),
                "issue.create",
                IntakeOutcomeKind::IssueCreated,
                7,
            );
            assert_eq!(load(dir.path()).unwrap(), None);
        }

        // FR-013 end-to-end: a successful issue.comment through the CLI
        // records the outcome for the intake session.
        #[test]
        fn issue_comment_success_auto_records_through_cli() {
            use crate::cli::{run_collect, CliCommand, IssueCommand, TestEnv};
            use gwt_github::{IssueNumber, IssueSnapshot, IssueState, UpdatedAt};

            let _env_lock = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-cli");
            let dir = mk_worktree(&INTAKE_PROFILE);

            let mut env = TestEnv::new(dir.path().to_path_buf());
            env.client.seed(IssueSnapshot {
                number: IssueNumber(42),
                title: "T".to_string(),
                body: String::new(),
                labels: vec![],
                state: IssueState::Open,
                updated_at: UpdatedAt::new("seed"),
                comments: Vec::new(),
            });
            let (code, _) = run_collect(
                &mut env,
                CliCommand::Issue(IssueCommand::CommentBody {
                    number: 42,
                    body: "progress note".to_string(),
                }),
            )
            .unwrap();
            assert_eq!(code, 0);

            let state = load(dir.path()).unwrap().unwrap();
            assert_eq!(state.session_id, "sess-cli");
            let outcome = state.outcome.unwrap();
            assert_eq!(outcome.kind, IntakeOutcomeKind::IssueUpdated);
            assert_eq!(outcome.number, Some(42));
        }
    }
}
