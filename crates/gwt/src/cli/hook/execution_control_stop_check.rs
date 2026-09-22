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

use super::{envelope::stop_hook_active_from, HookOutput};
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
    let record = match execution_state::load(&resolved) {
        Ok(Some(record)) => record,
        // No record: pre-P8a worktree or unlinked launch — unchanged.
        Ok(None) => return HookOutput::Silent,
        // Malformed record fails open for hooks.
        Err(_) => return HookOutput::Silent,
    };
    // P9a (T-122): a record edited outside the canonical operations must not
    // release the gate — block with the repair path instead of trusting the
    // edited status.
    if !execution_state::integrity_ok(&record) {
        let repair = execution_state::integrity_repair_guidance(record.status);
        return HookOutput::stop_block(format!(
            "Execution control record failed integrity validation: it was edited outside the canonical operations. {repair}",
        ));
    }
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
    // SPEC #3248 FR-243: a trusted No Action is a successful non-delivery
    // terminal outcome, so it releases Stop like a settlement does — but it is
    // neither Completed nor Blocked, and the record stays Active. The lookup
    // fails closed: it is bound to this session, requires an integrity-valid
    // audit, and requires that audit to quote this exact predecessor record,
    // so a stale or edited one releases nothing.
    if let Some(audit) =
        crate::cli::delivered_owner::trusted_no_action_for_session(&resolved, current.trim())
    {
        super::diagnostics::record_stop_gate_decision(
            &resolved,
            serde_json::json!({
                "message": "Stop released by a trusted No Action: the owner was already delivered and there was no source work to settle",
                "gate": "execution-control-stop-check",
                "issue": 4545,
                "session_id": current.trim(),
                "owner": format!(
                    "{kind} #{number}",
                    kind = audit.owner_kind.as_str(),
                    number = audit.owner_number
                ),
                "reason": audit.reason,
            }),
        );
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
         - already delivered with nothing to produce: run JSON operation `execution.no_action` with a non-empty `params.reason`. It succeeds only when the base already contains this worktree's whole source state, and it is not Blocked — a delivered owner is not a blocker.\n\
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

    fn mk_worktree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gwt")).unwrap();
        dir
    }

    // T-108: an active launch-written record blocks Stop even though
    // build.start was never called.
    #[test]
    fn active_record_blocks_stop_without_skill_state() {
        let dir = mk_worktree();
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
        let dir = mk_worktree();
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
        let dir = mk_worktree();
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
        let dir = mk_worktree();
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

    // T-122: a tampered record blocks Stop with the repair path. Repeated
    // blocks stay identical — the retired self-improvement capture note
    // (T-124) must not reappear in the reason (AC-R4).
    #[test]
    fn tampered_record_block_reports_repair_guidance_without_capture_note() {
        let dir = mk_worktree();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Spec,
            3248,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();
        let path = crate::cli::execution_state::state_path(dir.path());
        let tampered = std::fs::read_to_string(&path)
            .unwrap()
            .replace("$gwt-execute", "$gwt-forged");
        std::fs::write(&path, tampered).unwrap();

        for _ in 0..2 {
            let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
            let HookOutput::StopBlock { reason } = output else {
                panic!("expected StopBlock, got {output:?}");
            };
            assert!(reason.contains("integrity validation"), "{reason}");
            assert!(reason.contains("execution.repair"), "{reason}");
            assert!(reason.contains("quarantines"), "{reason}");
            assert!(!reason.contains("Self-improvement"), "{reason}");
        }
    }

    #[test]
    fn tampered_terminal_record_routes_to_fresh_launch_not_adopt() {
        let dir = mk_worktree();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            42,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();
        settle(
            dir.path(),
            "sess-1",
            ExecutionSettlement::Blocked {
                reason: "temporary dependency".to_string(),
                missing_verification: None,
            },
        )
        .unwrap();
        let path = crate::cli::execution_state::state_path(dir.path());
        let tampered = std::fs::read_to_string(&path)
            .unwrap()
            .replace("temporary dependency", "forged dependency");
        std::fs::write(&path, tampered).unwrap();

        let HookOutput::StopBlock { reason } = handle_with_input(dir.path(), "{}", Some("sess-1"))
        else {
            panic!("expected StopBlock");
        };
        assert!(reason.contains("execution.repair"), "{reason}");
        assert!(reason.contains("quarantines"), "{reason}");
        assert!(
            !reason.contains("Repair it with JSON operation `execution.adopt`"),
            "{reason}"
        );
    }

    /// A delivered worktree: a git repo whose `origin/develop` already
    /// contains its whole source state, plus a machine-local gwt home so the
    /// No Action audit lands in a trusted store this test owns.
    fn delivered_worktree() -> (
        gwt_core::test_support::ScopedGwtHome,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let home = tempfile::tempdir().unwrap();
        let guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        for args in [
            vec!["update-ref", "refs/remotes/origin/develop", "HEAD"],
            vec!["checkout", "-q", "-b", "work/issue-3290"],
        ] {
            let status = gwt_core::process::hidden_command("git")
                .arg("-C")
                .arg(dir.path())
                .args(&args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        }
        (guard, home, dir)
    }

    // AC-4 / FR-243: a trusted No Action releases Stop as a successful
    // non-delivery. The projection settles so that every reader — including
    // one that predates `execution.no_action` (Issue #4590) — sees a settled
    // execution, while the audit keeps it a non-delivery: no blocker, and no
    // verification evidence claiming a delivery.
    #[test]
    fn trusted_no_action_releases_stop_without_blocking_or_claiming_evidence() {
        let (_home_guard, _home, dir) = delivered_worktree();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            3290,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();
        assert!(
            matches!(
                handle_with_input(dir.path(), "{}", Some("sess-1")),
                HookOutput::StopBlock { .. }
            ),
            "the active record gates Stop before the No Action"
        );

        crate::cli::delivered_owner::record_no_action(
            dir.path(),
            "sess-1",
            "owner #3290 was delivered in PR #3328",
        )
        .unwrap();

        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
            "a trusted No Action is a successful non-delivery terminal outcome"
        );
        let record = crate::cli::execution_state::load(dir.path())
            .unwrap()
            .unwrap();
        assert!(
            record.blocked_reason.is_none(),
            "a delivered owner is never a blocker"
        );
        assert!(
            record.completion_evidence.is_none(),
            "No Action never claims the verification evidence of a delivery"
        );
        assert!(
            crate::cli::delivered_owner::trusted_no_action_for_session(dir.path(), "sess-1")
                .is_some(),
            "the audit is what tells this settlement from a delivery"
        );
    }

    // AC-4: the release is fail-closed. A hand-written audit settles nothing —
    // only the canonical operation, which proves the zero source surface
    // first, can release the gate.
    #[test]
    fn a_hand_written_no_action_audit_does_not_release_the_gate() {
        let (_home_guard, _home, dir) = delivered_worktree();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            3290,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();
        crate::cli::delivered_owner::record_no_action(dir.path(), "sess-1", "already delivered")
            .unwrap();

        // Another session holds no authority over this record, so its own
        // lookup stays empty (FR-014t) — the audit never crosses sessions.
        assert!(
            crate::cli::delivered_owner::trusted_no_action_for_session(dir.path(), "sess-2")
                .is_none()
        );

        // Replay the same audit against a fresh, unsettled generation: the
        // audit is byte-for-byte the one the canonical operation wrote, and it
        // still releases nothing, because it is bound to the generation it
        // settled.
        let bytes = serde_json::to_vec_pretty(
            &crate::cli::delivered_owner::load_audit(dir.path())
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let (_home_guard, _home, fresh) = delivered_worktree();
        materialize_at_launch(
            fresh.path(),
            ExecutionOwnerKind::Issue,
            3290,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();
        crate::cli::trusted_store::write(
            fresh.path(),
            crate::cli::delivered_owner::NO_ACTION_AUDIT_FILE,
            &bytes,
        )
        .unwrap();
        std::fs::write(
            crate::cli::delivered_owner::audit_mirror_path(fresh.path()),
            &bytes,
        )
        .unwrap();

        assert!(
            crate::cli::delivered_owner::trusted_no_action_for_session(fresh.path(), "sess-1")
                .is_none(),
            "an audit from another generation is no audit at all"
        );
        assert!(
            matches!(
                handle_with_input(fresh.path(), "{}", Some("sess-1")),
                HookOutput::StopBlock { .. }
            ),
            "a planted audit must not release the gate"
        );
    }

    // Issue #4590 AC-1/AC-4: a reader that knows nothing about
    // `execution.no_action` must not block an execution that settled as No
    // Action. Such a reader never opens the audit — it only knows the
    // Execution Control Record — so the test evaluates the gate with the
    // audit removed from both of its locations.
    #[test]
    fn a_no_action_blind_reader_does_not_block_a_settled_no_action() {
        let (_home_guard, _home, dir) = delivered_worktree();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            3290,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();
        crate::cli::delivered_owner::record_no_action(
            dir.path(),
            "sess-1",
            "owner #3290 was delivered in PR #3328",
        )
        .unwrap();

        // The shipped generation that raised #4590 has no No Action audit to
        // read: it predates the file entirely.
        let trusted_audit = crate::cli::trusted_store::trusted_dir_for_worktree(dir.path())
            .unwrap()
            .join(crate::cli::delivered_owner::NO_ACTION_AUDIT_FILE);
        std::fs::remove_file(&trusted_audit).unwrap();
        std::fs::remove_file(crate::cli::delivered_owner::audit_mirror_path(dir.path())).unwrap();
        assert!(
            crate::cli::delivered_owner::load_audit(dir.path())
                .unwrap()
                .is_none(),
            "the No Action audit is invisible to this reader"
        );

        let record = crate::cli::execution_state::load(dir.path())
            .unwrap()
            .unwrap();
        assert_ne!(
            record.status,
            ExecutionControlStatus::Active,
            "a settled No Action must not read as an unsettled execution"
        );
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
            "an execution.no_action-blind Stop gate must not block a settled No Action"
        );
    }

    // AC-4: the block names the No Action route, so a session parked on a
    // delivered owner can find the exit that is not `execution.blocked`.
    #[test]
    fn the_block_names_the_no_action_route() {
        let dir = mk_worktree();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            3290,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();
        let HookOutput::StopBlock { reason } = handle_with_input(dir.path(), "{}", Some("sess-1"))
        else {
            panic!("expected StopBlock");
        };
        assert!(reason.contains("execution.no_action"), "{reason}");
    }

    // SPEC #3245 FR-007: the former intake-lane exclusion is gone — a
    // launch-written record gates Stop uniformly in every worktree.
    #[test]
    fn former_intake_worktree_gates_like_any_other() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = mk_worktree();
        materialize_at_launch(
            dir.path(),
            ExecutionOwnerKind::Issue,
            7,
            "sess-1",
            "launch",
            false,
        )
        .unwrap();
        assert!(
            matches!(
                handle_with_input(dir.path(), "{}", Some("sess-1")),
                HookOutput::StopBlock { .. }
            ),
            "the execution control record gates uniformly after the lane removal"
        );
    }
}
