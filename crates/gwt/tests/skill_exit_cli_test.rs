//! Integration tests for the SPEC-1935 Phase 10 LLM-facing exit CLIs:
//! `gwtd discuss <action>`, `gwtd plan <action>`, and `gwtd build <action>`.
//!
//! These tests drive the real `dispatch` entry point over `TestEnv` so
//! the end-to-end parse → run path stays covered. The underlying state
//! file semantics are exhaustively tested at the unit level in
//! `gwt_core::skill_state` and `crate::discussion_resume`.

use gwt::cli::{dispatch, should_dispatch_cli, TestEnv};
use gwt_core::skill_state::{self, SkillState};
use tempfile::TempDir;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

fn new_env() -> (TestEnv, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (TestEnv::new(dir.path().to_path_buf()), dir)
}

#[test]
fn should_dispatch_cli_recognises_the_three_new_verbs() {
    assert!(should_dispatch_cli(&argv(&["gwt", "discuss", "resolve"])));
    assert!(should_dispatch_cli(&argv(&["gwt", "plan", "start"])));
    assert!(should_dispatch_cli(&argv(&["gwt", "build", "complete"])));
}

#[test]
fn discuss_resolve_flips_active_proposal_to_chosen() {
    let (mut env, dir) = new_env();
    let discussion_path = dir.path().join(".gwt/discussion.md");
    std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
    std::fs::write(
        &discussion_path,
        "### Proposal A - Hook-driven resume [active]\n\
         - Next Question: Should we block on Stop?\n",
    )
    .unwrap();

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "discuss", "resolve", "--proposal", "Proposal A"]),
    );
    assert_eq!(code, 0);
    let updated = std::fs::read_to_string(&discussion_path).unwrap();
    assert!(updated.contains("[chosen]"));
    assert!(!updated.contains("[active]"));
}

#[test]
fn discuss_park_and_reject_follow_the_same_pattern() {
    for (action, expected) in &[("park", "[parked]"), ("reject", "[rejected]")] {
        let (mut env, dir) = new_env();
        let path = dir.path().join(".gwt/discussion.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "### Proposal B - WIP [active]\n- Next Question: ???\n",
        )
        .unwrap();

        let code = dispatch(
            &mut env,
            &argv(&["gwt", "discuss", action, "--proposal", "Proposal B"]),
        );
        assert_eq!(code, 0, "action={action} should exit 0");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains(expected), "expected {expected} in {body}");
    }
}

#[test]
fn discuss_clear_next_question_empties_the_field() {
    let (mut env, dir) = new_env();
    let path = dir.path().join(".gwt/discussion.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "### Proposal A - Hook-driven resume [active]\n\
         - Next Question: Should SessionStart surface it?\n",
    )
    .unwrap();

    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "discuss",
            "clear-next-question",
            "--proposal",
            "Proposal A",
        ]),
    );
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("- Next Question:\n"));
    assert!(!body.contains("Should SessionStart surface it?"));
}

#[test]
fn plan_start_creates_active_state_file() {
    let (mut env, dir) = new_env();
    let code = dispatch(&mut env, &argv(&["gwt", "plan", "start", "--spec", "1935"]));
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
    dispatch(&mut env, &argv(&["gwt", "plan", "start", "--spec", "1935"]));
    let code = dispatch(
        &mut env,
        &argv(&["gwt", "plan", "complete", "--spec", "1935"]),
    );
    assert_eq!(code, 0);

    let state: SkillState = skill_state::load(dir.path(), "plan-spec")
        .unwrap()
        .expect("plan-spec state should still exist");
    assert!(!state.active);
}

#[test]
fn plan_complete_with_mismatched_spec_is_rejected() {
    let (mut env, dir) = new_env();
    dispatch(&mut env, &argv(&["gwt", "plan", "start", "--spec", "1935"]));
    let code = dispatch(
        &mut env,
        &argv(&["gwt", "plan", "complete", "--spec", "9999"]),
    );
    assert_eq!(code, 2, "mismatched SPEC must refuse to finalize");
    let state: SkillState = skill_state::load(dir.path(), "plan-spec").unwrap().unwrap();
    assert!(state.active, "state must remain active on rejection");
}

#[test]
fn build_lifecycle_start_phase_complete_sequences_correctly() {
    let (mut env, dir) = new_env();
    assert_eq!(
        dispatch(
            &mut env,
            &argv(&["gwt", "build", "start", "--spec", "1935"])
        ),
        0
    );
    assert_eq!(
        dispatch(
            &mut env,
            &argv(&["gwt", "build", "phase", "--spec", "1935", "--label", "verify"])
        ),
        0
    );
    let state = skill_state::load(dir.path(), "build-spec")
        .unwrap()
        .unwrap();
    assert!(state.active);
    assert_eq!(state.phase.as_deref(), Some("verify"));

    assert_eq!(
        dispatch(
            &mut env,
            &argv(&["gwt", "build", "complete", "--spec", "1935"])
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
    dispatch(
        &mut env,
        &argv(&["gwt", "build", "start", "--spec", "1935"]),
    );
    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "build",
            "abort",
            "--spec",
            "1935",
            "--reason",
            "needs clarification from product",
        ]),
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

#[test]
fn discuss_commands_exit_zero_when_discussion_md_absent() {
    // Idempotent no-op: absent file is not an error; the handler just
    // reports "no change".
    let (mut env, _dir) = new_env();
    let code = dispatch(
        &mut env,
        &argv(&["gwt", "discuss", "resolve", "--proposal", "Proposal X"]),
    );
    assert_eq!(code, 0);
}
