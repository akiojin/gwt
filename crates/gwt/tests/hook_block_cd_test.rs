//! T-040 (SPEC #1942) — block-cd-command golden tests.

use std::path::{Path, PathBuf};

use gwt::cli::hook::block_cd_command;

fn root() -> PathBuf {
    std::env::temp_dir().join("gwt-test-worktree")
}

fn outside_root() -> PathBuf {
    root()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("gwt-test-outside")
}

#[test]
fn cd_to_absolute_path_outside_worktree_is_blocked() {
    let decision = block_cd_command::evaluate_bash_command(
        &format!("cd {}", outside_root().display()),
        &root(),
    );
    assert!(
        decision.is_some(),
        "cd outside the worktree should be blocked"
    );
}

#[test]
fn cd_to_home_shortcut_is_blocked() {
    let decision = block_cd_command::evaluate_bash_command("cd ~", &root());
    assert!(decision.is_some(), "cd ~ should be blocked");
}

#[test]
fn cd_to_absolute_path_inside_worktree_is_allowed() {
    let decision = block_cd_command::evaluate_bash_command(
        &format!("cd {}/subdir", root().display()),
        &root(),
    );
    assert!(
        decision.is_none(),
        "cd into a path strictly under the root should be allowed, got {decision:?}"
    );
}

#[test]
fn cd_to_worktree_root_itself_is_allowed() {
    let decision =
        block_cd_command::evaluate_bash_command(&format!("cd {}", root().display()), &root());
    assert!(
        decision.is_none(),
        "cd to the root itself should be allowed"
    );
}

#[test]
fn non_cd_command_is_not_examined() {
    let decision = block_cd_command::evaluate_bash_command(
        &format!("echo cd {}", outside_root().display()),
        &root(),
    );
    assert!(
        decision.is_none(),
        "echo cd is not a cd command, must not be blocked"
    );
}

#[test]
fn grep_mentioning_cd_is_not_blocked() {
    let decision = block_cd_command::evaluate_bash_command("grep cd foo.txt", &root());
    assert!(
        decision.is_none(),
        "grep containing the literal word 'cd' must not be blocked"
    );
}

#[test]
fn adversarial_segment_after_innocuous_prefix_is_blocked() {
    let decision = block_cd_command::evaluate_bash_command(
        &format!("echo hello && cd {}", outside_root().display()),
        &root(),
    );
    assert!(
        decision.is_some(),
        "cd outside the worktree after an innocuous prefix must still be blocked"
    );
}
