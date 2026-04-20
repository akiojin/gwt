//! T-035 (SPEC #1942 amendment) — block-bash-policy golden tests.

use std::path::{Path, PathBuf};

use gwt::cli::hook::block_bash_policy;

fn root() -> PathBuf {
    std::env::temp_dir().join("gwt-test-worktree")
}

fn outside_root() -> PathBuf {
    root()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("gwt-test-outside")
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
    block(&format!("cd {}", outside_root().display()));
}

#[test]
fn blocks_file_ops_outside_worktree() {
    block("rm -rf /");
    block(&format!(
        "cp {}/foo.txt {}/foo.txt",
        root().display(),
        outside_root().display()
    ));
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
    // All guidance must land inside `permissionDecisionReason` because the
    // legacy `stopReason` field is ignored on PreToolUse hooks. If the
    // canonical `gwt` alternatives are not in that single visible field,
    // the LLM/user sees only the short summary and has no idea how to
    // recover.
    let decision = block_bash_policy::evaluate_bash_command("gh pr view 1949", &root())
        .expect("workflow gh command must block");
    let visible = decision.permission_decision_reason();
    assert!(
        visible.contains("GitHub workflow CLI"),
        "summary must appear in permissionDecisionReason, got: {visible}"
    );
    assert!(
        visible.contains("gwt issue view"),
        "gwt issue view alternative missing from visible reason: {visible}"
    );
    assert!(
        visible.contains("gwt pr view"),
        "gwt pr view alternative missing from visible reason: {visible}"
    );
    assert!(
        visible.contains("gwt actions logs"),
        "gwt actions logs alternative missing from visible reason: {visible}"
    );
    assert!(
        visible.contains("gwt-search"),
        "gwt-search alternative missing from visible reason: {visible}"
    );
    assert!(
        visible.contains("Blocked command: gh pr view 1949"),
        "the blocked command must be echoed in the visible reason: {visible}"
    );
}

#[test]
fn allows_read_only_and_in_worktree_commands() {
    allow("git branch --list");
    allow("git checkout HEAD -- foo.rs");
    allow(&format!("mkdir {}/new-dir", root().display()));
}

#[test]
fn allows_non_workflow_github_cli_commands() {
    allow("gh auth status");
    allow("gh repo view");
    allow("gh release list");
    allow("gh api user");
    allow("gh api graphql -f query='query { viewer { login } }'");
}
