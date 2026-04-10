//! T-060 (SPEC #1942) — block-git-dir-override golden tests.

use gwt_tui::cli::hook::block_git_dir_override;

fn block(cmd: &str) {
    assert!(
        block_git_dir_override::evaluate_bash_command(cmd).is_some(),
        "expected BLOCK for {cmd:?}"
    );
}

fn allow(cmd: &str) {
    assert!(
        block_git_dir_override::evaluate_bash_command(cmd).is_none(),
        "expected ALLOW for {cmd:?}"
    );
}

#[test]
fn git_dir_flag_on_git_invocation_is_allowed() {
    // The original MJS helper actually keys off the ENV override forms
    // (`GIT_DIR=...`, `env GIT_DIR=...`, `export GIT_DIR=...`,
    // `declare -x GIT_DIR=...`). A bare `git --git-dir=...` flag is
    // allowed. This test pins that behaviour so we do not
    // accidentally widen the rule.
    allow("git --git-dir=/other/.git status");
}

#[test]
fn env_var_prefix_git_dir_is_blocked() {
    block("GIT_DIR=/other/.git git status");
}

#[test]
fn env_command_setting_git_dir_is_blocked() {
    block("env GIT_DIR=/other/.git git status");
}

#[test]
fn export_git_dir_is_blocked() {
    block("export GIT_DIR=/other/.git");
}

#[test]
fn declare_x_git_dir_is_blocked() {
    block("declare -x GIT_DIR=/other/.git");
}

#[test]
fn git_work_tree_override_is_blocked() {
    block("GIT_WORK_TREE=/somewhere git status");
    block("export GIT_WORK_TREE=/somewhere");
}

#[test]
fn plain_git_status_is_allowed() {
    allow("git status");
}

#[test]
fn innocuous_var_with_similar_prefix_is_allowed() {
    // `GIT_DIRECTORY` (not a real git var) must not false-positive.
    allow("GIT_DIRECTORY=/tmp echo hi");
}
