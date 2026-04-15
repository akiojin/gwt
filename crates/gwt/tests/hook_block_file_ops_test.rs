//! T-050 (SPEC #1942) — block-file-ops golden tests.

use std::path::PathBuf;

use gwt::cli::hook::block_file_ops;

fn root() -> PathBuf {
    PathBuf::from("/tmp/gwt-test-worktree")
}

#[test]
fn rm_rf_root_slash_is_blocked() {
    let decision = block_file_ops::evaluate_bash_command("rm -rf /", &root());
    assert!(decision.is_some(), "rm -rf / must be blocked");
}

#[test]
fn rm_rf_home_shortcut_is_blocked() {
    let decision = block_file_ops::evaluate_bash_command("rm -rf ~", &root());
    assert!(decision.is_some(), "rm -rf ~ must be blocked");
}

#[test]
fn rm_inside_worktree_relative_path_is_allowed() {
    // Relative paths resolve against the current process cwd, which
    // during test execution is the gwt repo. Because our synthetic
    // `/tmp/gwt-test-worktree` root does NOT match that cwd, relative
    // paths will be considered *outside* — so for this test we use an
    // absolute path under the synthetic root.
    let decision =
        block_file_ops::evaluate_bash_command("rm -rf /tmp/gwt-test-worktree/target", &root());
    assert!(
        decision.is_none(),
        "rm -rf <path-under-root> must be allowed, got {decision:?}"
    );
}

#[test]
fn mkdir_inside_worktree_is_allowed() {
    let decision =
        block_file_ops::evaluate_bash_command("mkdir /tmp/gwt-test-worktree/new-dir", &root());
    assert!(decision.is_none(), "mkdir under the root must be allowed");
}

#[test]
fn non_file_op_command_is_ignored() {
    let decision = block_file_ops::evaluate_bash_command("echo rm /etc", &root());
    assert!(
        decision.is_none(),
        "echo rm is not a file-op segment start, must not be blocked"
    );
}

#[test]
fn cp_to_path_outside_worktree_is_blocked() {
    let decision = block_file_ops::evaluate_bash_command(
        "cp /tmp/gwt-test-worktree/foo.txt /etc/foo.txt",
        &root(),
    );
    assert!(
        decision.is_some(),
        "cp targeting /etc must be blocked even if the source is inside"
    );
}

#[test]
fn flags_are_not_treated_as_file_paths() {
    // `rm -rf --no-preserve-root /tmp/gwt-test-worktree/target` has only
    // one path arg (`/tmp/...`) — the flags must not trigger a
    // false-positive block.
    let decision = block_file_ops::evaluate_bash_command(
        "rm -rf --no-preserve-root /tmp/gwt-test-worktree/target",
        &root(),
    );
    assert!(decision.is_none(), "flags must not be treated as paths");
}
