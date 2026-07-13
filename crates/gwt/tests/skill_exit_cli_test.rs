//! Integration tests for the SPEC-1935 Phase 10 LLM-facing exit operations:
//! `discuss.*`, `plan.*`, and `build.*` JSON envelopes.
//!
//! These tests drive the real `dispatch` entry point over `TestEnv` so
//! the end-to-end parse → run path stays covered. The underlying state
//! file semantics are exhaustively tested at the unit level in
//! `gwt_core::skill_state` and `crate::discussion_resume`.

use gwt::cli::{dispatch, TestEnv};
use gwt_core::skill_state::{self, SkillState};
use gwt_core::test_support::{env_lock, ScopedEnvVar, ScopedGwtHome};
use tempfile::TempDir;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(std::string::ToString::to_string).collect()
}

fn new_env() -> (TestEnv, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (TestEnv::new(dir.path().to_path_buf()), dir)
}

fn legacy_discussion_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join(".gwt/discussion.md")
}

fn canonical_discussions_path(dir: &TempDir) -> std::path::PathBuf {
    // SPEC-3214 (FR-007): discussion mutations canonicalize into the
    // machine-local home work-notes file.
    gwt_core::paths::gwt_work_notes_discussions_path(dir.path())
}

fn dispatch_json(env: &mut TestEnv, operation: &str, params: serde_json::Value) -> i32 {
    env.stdin = serde_json::json!({
        "schema_version": 1,
        "operation": operation,
        "params": params,
    })
    .to_string();
    dispatch(env, &argv(&["gwt"]))
}

#[test]
fn exit_operations_dispatch_through_json_envelopes() {
    let (mut env, _dir) = new_env();
    let code = dispatch_json(
        &mut env,
        "plan.phase",
        serde_json::json!({"spec": 1935, "label": "verify"}),
    );
    assert_eq!(code, 0);
}

#[test]
fn discuss_resolve_flips_active_proposal_to_chosen() {
    let (mut env, dir) = new_env();
    let discussion_path = legacy_discussion_path(&dir);
    std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
    std::fs::write(
        &discussion_path,
        "### Proposal A - Hook-driven resume [active]\n\
         - Implementation Proof: crates/gwt/src/discussion_resume.rs inspected\n\
         - SPEC/Issue Proof: SPEC-1935 checked\n\
         - Gap Check Proof: scope/integration/failure/migration/verification checked\n\
         - Official Docs Proof: Claude Code hooks docs checked\n\
         - External Research Proof: not-applicable: local-only behavior\n\
         - Exit Blockers: none\n\
         - Depth Mode: normal\n\
         - Question Ledger: scope boundary, integration, failure, migration, verification checked\n\
         - Depth Gate: complete\n\
         - Next Question: Should we block on Stop?\n\
         - Evidence Gate: complete\n",
    )
    .unwrap();

    let code = dispatch_json(
        &mut env,
        "discuss.resolve",
        serde_json::json!({"proposal": "Proposal A"}),
    );
    assert_eq!(code, 0);
    let updated = std::fs::read_to_string(canonical_discussions_path(&dir)).unwrap();
    assert!(updated.contains("[chosen]"));
    assert!(!updated.contains("[active]"));
}

#[test]
fn discuss_resolve_rejects_incomplete_evidence_gate() {
    let (mut env, dir) = new_env();
    let discussion_path = legacy_discussion_path(&dir);
    std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
    std::fs::write(
        &discussion_path,
        "### Proposal A - Evidence gap [active]\n\
         - Summary: Missing proof.\n\
         - Exit Blockers: none\n\
         - Next Question:\n\
         - Evidence Gate: complete\n",
    )
    .unwrap();

    let code = dispatch_json(
        &mut env,
        "discuss.resolve",
        serde_json::json!({"proposal": "Proposal A"}),
    );
    assert_eq!(code, 2);
    let updated = std::fs::read_to_string(&discussion_path).unwrap();
    assert!(updated.contains("[active]"));
    assert!(!updated.contains("[chosen]"));
}

#[test]
fn discuss_resolve_rejects_incomplete_depth_gate() {
    let (mut env, dir) = new_env();
    let discussion_path = legacy_discussion_path(&dir);
    std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
    std::fs::write(
        &discussion_path,
        "### Proposal A - Depth gap [active]\n\
         - Implementation Proof: crates/gwt/src/discussion_resume.rs inspected\n\
         - SPEC/Issue Proof: SPEC-1935 checked\n\
         - Gap Check Proof: scope/integration/failure/migration/verification checked\n\
         - Official Docs Proof: not-applicable: local-only behavior\n\
         - External Research Proof: not-applicable: local-only behavior\n\
         - Exit Blockers: none\n\
         - Depth Mode: normal\n\
         - Question Ledger: scope boundary checked only\n\
         - Depth Gate: open\n\
         - Next Question:\n\
         - Evidence Gate: complete\n",
    )
    .unwrap();

    let code = dispatch_json(
        &mut env,
        "discuss.resolve",
        serde_json::json!({"proposal": "Proposal A"}),
    );
    assert_eq!(code, 2);
    let updated = std::fs::read_to_string(&discussion_path).unwrap();
    assert!(updated.contains("[active]"));
    assert!(!updated.contains("[chosen]"));
}

#[test]
fn discuss_park_and_reject_follow_the_same_pattern() {
    for (action, expected) in &[("park", "[parked]"), ("reject", "[rejected]")] {
        let (mut env, dir) = new_env();
        let path = legacy_discussion_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "### Proposal B - WIP [active]\n- Next Question: ???\n",
        )
        .unwrap();

        let code = dispatch_json(
            &mut env,
            &format!("discuss.{action}"),
            serde_json::json!({"proposal": "Proposal B"}),
        );
        assert_eq!(code, 0, "action={action} should exit 0");
        let body = std::fs::read_to_string(canonical_discussions_path(&dir)).unwrap();
        assert!(body.contains(expected), "expected {expected} in {body}");
    }
}

#[test]
fn discuss_clear_next_question_empties_the_field() {
    let (mut env, dir) = new_env();
    let path = legacy_discussion_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "### Proposal A - Hook-driven resume [active]\n\
         - Next Question: Should SessionStart surface it?\n",
    )
    .unwrap();

    let code = dispatch_json(
        &mut env,
        "discuss.clear_next_question",
        serde_json::json!({"proposal": "Proposal A"}),
    );
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(canonical_discussions_path(&dir)).unwrap();
    assert!(body.contains("- Next Question:\n"));
    assert!(!body.contains("Should SessionStart surface it?"));
}

#[test]
fn plan_start_creates_active_state_file() {
    let (mut env, dir) = new_env();
    let code = dispatch_json(&mut env, "plan.start", serde_json::json!({"spec": 1935}));
    assert_eq!(code, 0);

    let state = skill_state::load(dir.path(), "plan-spec")
        .unwrap()
        .expect("plan-spec state should exist");
    assert!(state.active);
    assert_eq!(state.owner_spec, Some(1935));
}

#[test]
fn plan_complete_marks_state_inactive() {
    let (mut env, dir) = new_env();
    dispatch_json(&mut env, "plan.start", serde_json::json!({"spec": 1935}));
    let code = dispatch_json(&mut env, "plan.complete", serde_json::json!({"spec": 1935}));
    assert_eq!(code, 0);

    let state: SkillState = skill_state::load(dir.path(), "plan-spec")
        .unwrap()
        .expect("plan-spec state should still exist");
    assert!(!state.active);
}

#[test]
fn plan_complete_with_mismatched_spec_is_rejected() {
    let (mut env, dir) = new_env();
    dispatch_json(&mut env, "plan.start", serde_json::json!({"spec": 1935}));
    let code = dispatch_json(&mut env, "plan.complete", serde_json::json!({"spec": 9999}));
    assert_eq!(code, 2, "mismatched SPEC must refuse to finalize");
    let state: SkillState = skill_state::load(dir.path(), "plan-spec").unwrap().unwrap();
    assert!(state.active, "state must remain active on rejection");
}

#[test]
fn build_lifecycle_start_phase_complete_sequences_correctly() {
    let (mut env, dir) = new_env();
    assert_eq!(
        dispatch_json(&mut env, "build.start", serde_json::json!({"spec": 1935})),
        0
    );
    assert_eq!(
        dispatch_json(
            &mut env,
            "build.phase",
            serde_json::json!({"spec": 1935, "label": "verify"})
        ),
        0
    );
    let state = skill_state::load(dir.path(), "build-spec")
        .unwrap()
        .unwrap();
    assert!(state.active);
    assert_eq!(state.phase.as_deref(), Some("verify"));

    assert_eq!(
        dispatch_json(
            &mut env,
            "build.complete",
            serde_json::json!({"spec": 1935})
        ),
        0
    );
    let state = skill_state::load(dir.path(), "build-spec")
        .unwrap()
        .unwrap();
    assert!(!state.active);
}

#[test]
fn build_abort_records_reason_in_phase_field() {
    let (mut env, dir) = new_env();
    dispatch_json(&mut env, "build.start", serde_json::json!({"spec": 1935}));
    let code = dispatch_json(
        &mut env,
        "build.abort",
        serde_json::json!({
            "spec": 1935,
            "reason": "needs clarification from product",
        }),
    );
    assert_eq!(code, 0);
    let state = skill_state::load(dir.path(), "build-spec")
        .unwrap()
        .unwrap();
    assert!(!state.active);
    assert!(state
        .phase
        .as_deref()
        .unwrap_or("")
        .starts_with("aborted: "));
}

fn seed_session_work(repo: &std::path::Path, session_id: &str) {
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        repo,
        &format!("work-session-{session_id}"),
        Some("Build lifecycle Work"),
        None,
        Some("SPEC-2359"),
        &[],
        None,
        Some(session_id),
        chrono::Utc::now(),
    )
    .expect("seed session Work");
}

#[test]
fn build_complete_marks_current_session_work_done_idempotently() {
    let _lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut env, dir) = new_env();
    let _home = ScopedGwtHome::set(dir.path().join("home"));
    let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-build-done");
    seed_session_work(dir.path(), "session-build-done");

    dispatch_json(&mut env, "build.start", serde_json::json!({"spec": 2359}));
    assert_eq!(
        dispatch_json(
            &mut env,
            "build.complete",
            serde_json::json!({"spec": 2359})
        ),
        0
    );
    assert_eq!(
        dispatch_json(
            &mut env,
            "build.complete",
            serde_json::json!({"spec": 2359})
        ),
        0,
        "retry remains successful"
    );

    let projection = gwt_core::workspace_projection::load_workspace_work_items(dir.path())
        .expect("load Work items")
        .expect("Work items");
    let work = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-build-done")
        .expect("current session Work");
    assert_eq!(
        work.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Done
    );
    assert_eq!(
        work.events
            .iter()
            .filter(|event| { event.kind == gwt_core::workspace_projection::WorkEventKind::Done })
            .count(),
        1,
        "build.complete retry must not duplicate Done"
    );
}

#[test]
fn build_complete_retry_with_inactive_state_does_not_close_later_work() {
    let _lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut env, dir) = new_env();
    let _home = ScopedGwtHome::set(dir.path().join("home"));
    let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-stale-retry");

    dispatch_json(&mut env, "build.start", serde_json::json!({"spec": 2359}));
    assert_eq!(
        dispatch_json(
            &mut env,
            "build.complete",
            serde_json::json!({"spec": 2359})
        ),
        0
    );
    seed_session_work(dir.path(), "session-stale-retry");

    assert_eq!(
        dispatch_json(
            &mut env,
            "build.complete",
            serde_json::json!({"spec": 2359})
        ),
        0,
        "stale completion retry remains successful"
    );

    let projection = gwt_core::workspace_projection::load_workspace_work_items(dir.path())
        .expect("load Work items")
        .expect("Work items");
    let work = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-stale-retry")
        .expect("later current Work");
    assert!(
        !work.is_terminal(),
        "inactive build state must not close Work created after completion"
    );
}

#[test]
fn build_complete_from_another_session_does_not_close_current_work() {
    let _lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut env, dir) = new_env();
    let _home = ScopedGwtHome::set(dir.path().join("home"));

    {
        let _owner_session =
            ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-build-owner");
        dispatch_json(&mut env, "build.start", serde_json::json!({"spec": 2359}));
    }
    {
        let _current_session =
            ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-build-observer");
        seed_session_work(dir.path(), "session-build-observer");
        assert_eq!(
            dispatch_json(
                &mut env,
                "build.complete",
                serde_json::json!({"spec": 2359})
            ),
            0
        );
    }

    let projection = gwt_core::workspace_projection::load_workspace_work_items(dir.path())
        .expect("load Work items")
        .expect("Work items");
    let work = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-build-observer")
        .expect("observer Work");
    assert!(
        !work.is_terminal(),
        "a build state owned by another session must not close this session's Work"
    );
}

#[test]
fn build_abort_discards_only_current_session_work() {
    let _lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut env, dir) = new_env();
    let _home = ScopedGwtHome::set(dir.path().join("home"));
    let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-build-abort");
    seed_session_work(dir.path(), "session-build-abort");
    seed_session_work(dir.path(), "session-other");

    dispatch_json(&mut env, "build.start", serde_json::json!({"spec": 2359}));
    assert_eq!(
        dispatch_json(
            &mut env,
            "build.abort",
            serde_json::json!({"spec": 2359, "reason": "cancelled"})
        ),
        0
    );

    let projection = gwt_core::workspace_projection::load_workspace_work_items(dir.path())
        .expect("load Work items")
        .expect("Work items");
    let current = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-build-abort")
        .expect("current Work");
    assert!(current.discarded);
    let other = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-other")
        .expect("other Work");
    assert!(!other.is_terminal(), "other Work must remain untouched");
}

#[test]
fn build_complete_without_registered_work_is_successful_noop() {
    let _lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut env, dir) = new_env();
    let _home = ScopedGwtHome::set(dir.path().join("home"));
    let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "standalone-session");

    dispatch_json(&mut env, "build.start", serde_json::json!({"spec": 2359}));
    assert_eq!(
        dispatch_json(
            &mut env,
            "build.complete",
            serde_json::json!({"spec": 2359})
        ),
        0
    );
    assert!(
        gwt_core::workspace_projection::load_workspace_work_items(dir.path())
            .expect("load Work items")
            .is_none(),
        "standalone completion must not invent a Work"
    );
}

#[test]
fn plan_phase_without_active_state_exits_zero() {
    let (mut env, _dir) = new_env();
    let code = dispatch_json(
        &mut env,
        "plan.phase",
        serde_json::json!({"spec": 1935, "label": "verify"}),
    );
    assert_eq!(code, 0);
}

#[test]
fn plan_abort_without_active_state_exits_zero() {
    let (mut env, _dir) = new_env();
    let code = dispatch_json(
        &mut env,
        "plan.abort",
        serde_json::json!({"spec": 1935, "reason": "cancelled"}),
    );
    assert_eq!(code, 0);
}

#[test]
fn build_phase_with_mismatched_spec_is_rejected() {
    let (mut env, _dir) = new_env();
    dispatch_json(&mut env, "build.start", serde_json::json!({"spec": 1935}));
    let code = dispatch_json(
        &mut env,
        "build.phase",
        serde_json::json!({"spec": 9999, "label": "verify"}),
    );
    assert_eq!(code, 2, "mismatched SPEC must refuse to update phase");
    let state = skill_state::load(_dir.path(), "build-spec")
        .unwrap()
        .unwrap();
    assert!(state.active, "state must remain active on rejection");
    assert!(state.phase.is_none(), "phase must not be updated");
}

#[test]
fn build_abort_with_mismatched_spec_is_rejected() {
    let (mut env, dir) = new_env();
    dispatch_json(&mut env, "build.start", serde_json::json!({"spec": 1935}));
    let code = dispatch_json(
        &mut env,
        "build.abort",
        serde_json::json!({"spec": 9999, "reason": "wrong spec"}),
    );
    assert_eq!(code, 2, "mismatched SPEC must refuse to abort");
    let state = skill_state::load(dir.path(), "build-spec")
        .unwrap()
        .unwrap();
    assert!(state.active, "state must remain active on rejection");
}

#[test]
fn discuss_commands_exit_zero_when_discussion_md_absent() {
    // Idempotent no-op: absent file is not an error; the handler just
    // reports "no change".
    let (mut env, _dir) = new_env();
    let code = dispatch_json(
        &mut env,
        "discuss.resolve",
        serde_json::json!({"proposal": "Proposal X"}),
    );
    assert_eq!(code, 0);
}
