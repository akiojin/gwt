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
fn blocks_issue_focused_github_cli_commands() {
    block("gh issue view 1942");
    block("gh issue create --title \"fix: issue\" --body \"details\"");
    block("gh issue comment 1942 --body \"done\"");
    block("env GH_TOKEN=test gh issue view 1942");
    block("gh api repos/akiojin/gwt/issues/1942");
    block("gh api /repos/akiojin/gwt/issues/1942/comments");
    block("gh api graphql -f query='query { repository(owner:\"akiojin\", name:\"gwt\") { issue(number:1942) { id } } }'");
}

#[test]
fn allows_read_only_and_in_worktree_commands() {
    allow("git branch --list");
    allow("git checkout HEAD -- foo.rs");
    allow("mkdir /tmp/gwt-test-worktree/new-dir");
    allow("gh auth status");
    allow("gh repo view");
    allow("gh release list");
    allow("gh pr create --base main --head feature/x --title test --body body");
    allow("gh pr checks 1949");
    allow("gh api repos/akiojin/gwt/pulls/1949");
    allow("gh api graphql -f query='query { repository(owner:\"akiojin\", name:\"gwt\") { pullRequest(number:1949) { id } } }'");
    allow("gh run view 123456789");
}
