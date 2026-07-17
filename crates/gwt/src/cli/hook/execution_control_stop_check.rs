//! Execution control Stop gate (SPEC-3248 P8a, T-108/T-109, AS-30).
//!
//! When an Execution launch materialized an active Execution Control Record
//! for the current session, Stop stays blocked until the session settles the
//! execution — `execution.complete`, `execution.blocked`, or (for build-spec
//! flows) `build.complete`. The gate keys off the launch-written record, not
//! skill state, so a plain-Issue `$gwt-fix-issue` session that never called
//! `build.start` is covered by the same lifecycle as `$gwt-build-spec`
//! (FR-034).
//!
//! Existing Stop contracts hold: `stop_hook_active` short-circuits (one
//! forced continuation per cycle), parse/IO errors and missing records fail
//! open (pre-P8a worktrees and unlinked launches are unchanged), another
//! session's record stays silent (FR-014t), and intake lanes are excluded —
//! they own no execution and have their own completion gate.

use std::path::Path;

use super::{context::HookContext, envelope::stop_hook_active_from, HookOutput};
use crate::cli::execution_state::{self, ExecutionControlStatus};

pub fn handle_with_input(
    worktree: &Path,
    input: &str,
    current_session: Option<&str>,
) -> HookOutput {
    if stop_hook_active_from(input) {
        return HookOutput::Silent;
    }
    let resolved = gwt_core::paths::resolve_current_worktree_root(worktree);
    let lane = HookContext::for_worktree(&resolved).lane;
    if lane.policy_flags.completion_gate {
        // Intake lanes settle through the intake artifact gate instead.
        return HookOutput::Silent;
    }
    let record = match execution_state::load(&resolved) {
        Ok(Some(record)) => record,
        // No record: pre-P8a worktree or unlinked launch — unchanged.
        Ok(None) => return HookOutput::Silent,
        // Malformed record fails open for hooks.
        Err(_) => return HookOutput::Silent,
    };
    // Settlement requires GWT_SESSION_ID; a session without one (a bare,
    // non-gwt-launched agent in the worktree) could never satisfy the gate,
    // so blocking it would be an unsatisfiable trap — stay silent.
    let Some(current) = current_session else {
        return HookOutput::Silent;
    };
    if current.trim() != record.primary_session_id {
        return HookOutput::Silent;
    }
    if record.status != ExecutionControlStatus::Active {
        return HookOutput::Silent;
    }

    let owner = format!(
        "{kind} #{number}",
        kind = record.owner_kind.as_str(),
        number = record.owner_number
    );
    HookOutput::stop_block(format!(
        "Execution for {owner} is still active (execution control record, entrypoint {entrypoint}).\n\
         Continue the execution workflow until the owner's scope is implemented, verified, and handed off. Settle the execution before stopping:\n\
         - done and verified: run JSON operation `execution.complete` (a successful `build.complete` with `params.spec:<n>` also settles it for gwt-build-spec flows), or\n\
         - blocked by the environment or missing verification: run JSON operation `execution.blocked` with a non-empty `params.reason` and optional `params.missing_verification`. Blocked is not done — report the blocker.\n\
         Do not settle as complete without the verification evidence the owner requires.",
        entrypoint = record.entrypoint,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::execution_state::{
        materialize_at_launch, settle, ExecutionOwnerKind, ExecutionSettlement,
    };
    use gwt_core::test_support::ScopedEnvVar;
    use gwt_skills::{write_lane_file, EXECUTION_PROFILE, INTAKE_PROFILE};

    fn mk_worktree(profile: Option<&gwt_skills::LaneProfile>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gwt")).unwrap();
        if let Some(profile) = profile {
            write_lane_file(dir.path(), profile).unwrap();
        }
        dir
    }

    // T-108: an active launch-written record blocks Stop even though
    // build.start was never called.
    #[test]
    fn active_record_blocks_stop_without_skill_state() {
        let dir = mk_worktree(Some(&EXECUTION_PROFILE));
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            42,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();

        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        let HookOutput::StopBlock { reason } = output else {
            panic!("expected StopBlock, got {output:?}");
        };
        assert!(reason.contains("issue #42"), "{reason}");
        assert!(reason.contains("execution.complete"), "{reason}");
        assert!(reason.contains("execution.blocked"), "{reason}");
        assert!(reason.contains("build.complete"), "{reason}");
    }

    // Settlement (completed or blocked) passes Stop.
    #[test]
    fn settled_record_passes_stop() {
        let dir = mk_worktree(Some(&EXECUTION_PROFILE));
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
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent
        );

        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-1",
            "launch",
            false,
        )
        .unwrap();
        settle(
            dir.path(),
            "sess-1",
            ExecutionSettlement::Blocked {
                reason: "runner unavailable".to_string(),
                missing_verification: None,
            },
        )
        .unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
            "terminal blocked settlement must pass Stop (blocked is reported, not looped)"
        );
    }

    // FR-015 analog: no record (pre-P8a worktrees / unlinked launches) and
    // malformed records fail open.
    #[test]
    fn missing_or_malformed_record_fails_open() {
        let dir = mk_worktree(Some(&EXECUTION_PROFILE));
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent
        );
        let path = crate::cli::execution_state::state_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent
        );
    }

    #[test]
    fn stop_hook_active_and_session_mismatch_stay_silent() {
        let dir = mk_worktree(Some(&EXECUTION_PROFILE));
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            7,
            "sess-1",
            "launch",
            false,
        )
        .unwrap();
        assert_eq!(
            handle_with_input(dir.path(), r#"{"stop_hook_active":true}"#, Some("sess-1")),
            HookOutput::Silent
        );
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("another-session")),
            HookOutput::Silent
        );
        // Review follow-up: a session without GWT_SESSION_ID can never settle
        // the record — blocking it would be an unsatisfiable trap.
        assert_eq!(
            handle_with_input(dir.path(), "{}", None),
            HookOutput::Silent
        );
    }

    // Intake lanes are excluded — the intake artifact gate owns them.
    #[test]
    fn intake_lane_stays_silent_even_with_record() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _kind = ScopedEnvVar::unset(gwt_skills::GWT_SESSION_KIND_ENV);
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            7,
            "sess-1",
            "launch",
            false,
        )
        .unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent
        );
    }
}
