//! T-035 (SPEC #1942 amendment) — block-bash-policy golden tests.

use std::path::PathBuf;

use gwt::cli::hook::block_bash_policy;

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
fn blocks_workflow_focused_github_cli_commands() {
    block("gh issue view 1942");
    block("gh issue create --title \"fix: issue\" --body \"details\"");
    block("gh issue comment 1942 --body \"done\"");
    block("gh pr view 1949");
    block("gh pr create --base main --head feature/x --title test --body body");
    block("gh pr checks 1949");
    block("gh run view 123456789");
    block("env GH_TOKEN=test gh issue view 1942");
    block("gh api repos/akiojin/gwt/issues/1942");
    block("gh api /repos/akiojin/gwt/issues/1942/comments");
    block("gh api repos/akiojin/gwt/pulls/1949");
    block("gh api repos/akiojin/gwt/actions/runs/123456789");
    block("gh api graphql -f query='query { repository(owner:\"akiojin\", name:\"gwt\") { issue(number:1942) { id } } }'");
    block("gh api graphql -f query='query { repository(owner:\"akiojin\", name:\"gwt\") { pullRequest(number:1949) { id } } }'");
}

#[test]
fn github_workflow_block_message_points_to_canonical_gwt_surfaces() {
    let decision = block_bash_policy::evaluate_bash_command("gh pr view 1949", &root())
        .expect("workflow gh command must block");
    assert!(decision.reason.contains("GitHub workflow CLI"));
    assert!(decision.stop_reason.contains("gwt issue view"));
    assert!(decision.stop_reason.contains("gwt pr view"));
    assert!(decision.stop_reason.contains("gwt actions logs"));
    assert!(decision.stop_reason.contains("gwt-search"));
}

#[test]
fn allows_read_only_and_in_worktree_commands() {
    allow("git branch --list");
    allow("git checkout HEAD -- foo.rs");
    allow("mkdir /tmp/gwt-test-worktree/new-dir");
}

#[test]
fn allows_non_workflow_github_cli_commands() {
    allow("gh auth status");
    allow("gh repo view");
    allow("gh release list");
    allow("gh api user");
    allow("gh api graphql -f query='query { viewer { login } }'");
}
