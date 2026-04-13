//! T-035 (SPEC #1942 amendment) — block-bash-policy golden tests.

use std::path::PathBuf;

use gwt_tui::cli::hook::block_bash_policy;

fn root() -> PathBuf {
    PathBuf::from("/tmp/gwt-test-worktree")
}

fn block(command: &str) {
    assert!(
        block_bash_policy::evaluate_bash_command(command, &root()).is_some(),
        "expected BLOCK for {command:?}"
    );
}

fn allow(command: &str) {
    assert!(
        block_bash_policy::evaluate_bash_command(command, &root()).is_none(),
        "expected ALLOW for {command:?}"
    );
}

#[test]
fn blocks_branch_policy_commands() {
    block("git rebase -i origin/main");
    block("git checkout main");
}

#[test]
fn blocks_cd_outside_worktree() {
    block("cd /etc");
}

#[test]
fn blocks_file_ops_outside_worktree() {
    block("rm -rf /");
    block("cp /tmp/gwt-test-worktree/foo.txt /etc/foo.txt");
}

#[test]
fn blocks_git_dir_override_env_vars() {
    block("GIT_DIR=/other/.git git status");
    block("export GIT_WORK_TREE=/somewhere");
}

#[test]
fn allows_read_only_and_in_worktree_commands() {
    allow("git branch --list");
    allow("git checkout HEAD -- foo.rs");
    allow("mkdir /tmp/gwt-test-worktree/new-dir");
}
