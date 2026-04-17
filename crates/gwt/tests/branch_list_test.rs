use std::collections::HashSet;

use gwt::{
    list_branch_entries, list_branch_entries_with_active_sessions, BranchCleanupAvailability,
    BranchCleanupBlockedReason, BranchCleanupRisk, BranchScope,
};
use tempfile::tempdir;

#[test]
fn list_branch_entries_marks_head_and_returns_local_branches() {
    let dir = tempdir().expect("tempdir");

    run_git(dir.path(), &["init", "-q"]);
    run_git(dir.path(), &["config", "user.name", "PoC Tester"]);
    run_git(dir.path(), &["config", "user.email", "poc@example.com"]);
    std::fs::write(dir.path().join("README.md"), "# demo\n").expect("write readme");
    run_git(dir.path(), &["add", "README.md"]);
    run_git(dir.path(), &["commit", "-qm", "init"]);
    run_git(dir.path(), &["branch", "-M", "main"]);
    run_git(dir.path(), &["branch", "feature/alpha"]);

    let branches = list_branch_entries(dir.path()).expect("branch entries");

    assert!(branches.iter().any(|branch| {
        branch.name == "main" && branch.is_head && branch.scope == BranchScope::Local
    }));
    assert!(branches.iter().any(|branch| {
        branch.name == "feature/alpha" && !branch.is_head && branch.scope == BranchScope::Local
    }));
}

#[test]
fn list_branch_entries_marks_unmerged_local_branch_as_risky_cleanup_candidate() {
    let dir = tempdir().expect("tempdir");

    run_git(dir.path(), &["init", "-q"]);
    init_repo(dir.path());
    run_git(dir.path(), &["checkout", "-qb", "feature/unmerged"]);
    std::fs::write(dir.path().join("feature.txt"), "work\n").expect("write feature");
    run_git(dir.path(), &["add", "feature.txt"]);
    run_git(dir.path(), &["commit", "-qm", "feature work"]);
    run_git(dir.path(), &["checkout", "main"]);

    let branches =
        list_branch_entries_with_active_sessions(dir.path(), &HashSet::new()).expect("entries");
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature/unmerged")
        .expect("feature branch");

    assert_eq!(feature.scope, BranchScope::Local);
    assert_eq!(
        feature.cleanup.availability,
        BranchCleanupAvailability::Risky
    );
    assert_eq!(
        feature.cleanup.execution_branch.as_deref(),
        Some("feature/unmerged")
    );
    assert_eq!(feature.cleanup.blocked_reason, None);
    assert!(feature.cleanup.risks.contains(&BranchCleanupRisk::Unmerged));
}

#[test]
fn list_branch_entries_marks_remote_tracking_row_with_local_counterpart_as_risky() {
    let temp = tempdir().expect("tempdir");
    let remote = temp.path().join("origin.git");
    let repo = temp.path().join("repo");

    run_git(
        temp.path(),
        &["init", "--bare", remote.to_str().expect("remote path")],
    );
    run_git(
        temp.path(),
        &["init", "-q", repo.to_str().expect("repo path")],
    );
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["checkout", "-qb", "feature/alpha"]);
    std::fs::write(repo.join("alpha.txt"), "alpha\n").expect("write alpha");
    run_git(&repo, &["add", "alpha.txt"]);
    run_git(&repo, &["commit", "-qm", "alpha"]);
    run_git(&repo, &["push", "-u", "origin", "feature/alpha"]);
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["fetch", "origin", "--prune"]);

    let branches =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let remote_entry = branches
        .iter()
        .find(|branch| branch.name == "origin/feature/alpha")
        .expect("remote branch");

    assert_eq!(remote_entry.scope, BranchScope::Remote);
    assert_eq!(
        remote_entry.cleanup.availability,
        BranchCleanupAvailability::Risky
    );
    assert_eq!(
        remote_entry.cleanup.execution_branch.as_deref(),
        Some("feature/alpha")
    );
    assert_eq!(remote_entry.cleanup.blocked_reason, None);
    assert!(remote_entry
        .cleanup
        .risks
        .contains(&BranchCleanupRisk::RemoteTracking));
}

#[test]
fn list_branch_entries_blocks_active_session_branch_from_cleanup() {
    let dir = tempdir().expect("tempdir");

    run_git(dir.path(), &["init", "-q"]);
    init_repo(dir.path());
    run_git(dir.path(), &["checkout", "-qb", "feature/live"]);
    std::fs::write(dir.path().join("live.txt"), "live\n").expect("write live");
    run_git(dir.path(), &["add", "live.txt"]);
    run_git(dir.path(), &["commit", "-qm", "live"]);
    run_git(dir.path(), &["checkout", "main"]);

    let branches = list_branch_entries_with_active_sessions(
        dir.path(),
        &HashSet::from([String::from("feature/live")]),
    )
    .expect("entries");
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature/live")
        .expect("feature branch");

    assert_eq!(
        feature.cleanup.availability,
        BranchCleanupAvailability::Blocked
    );
    assert_eq!(
        feature.cleanup.blocked_reason,
        Some(BranchCleanupBlockedReason::ActiveSession)
    );
}

fn init_repo(repo: &std::path::Path) {
    run_git(repo, &["config", "user.name", "PoC Tester"]);
    run_git(repo, &["config", "user.email", "poc@example.com"]);
    std::fs::write(repo.join("README.md"), "# demo\n").expect("write readme");
    run_git(repo, &["add", "README.md"]);
    run_git(repo, &["commit", "-qm", "init"]);
    run_git(repo, &["branch", "-M", "main"]);
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
