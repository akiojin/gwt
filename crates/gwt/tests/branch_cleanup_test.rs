use std::collections::HashSet;

use gwt::{
    cleanup_selected_branches, list_branch_entries_with_active_sessions, BranchCleanupResultStatus,
};
use tempfile::tempdir;

#[test]
fn cleanup_selected_branches_deletes_local_and_remote_branch() {
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
    run_git(&repo, &["checkout", "-qb", "work/prune-me"]);
    std::fs::write(repo.join("work.txt"), "work\n").expect("write work");
    run_git(&repo, &["add", "work.txt"]);
    run_git(&repo, &["commit", "-qm", "feature"]);
    run_git(&repo, &["push", "-u", "origin", "work/prune-me"]);
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["fetch", "origin", "--prune"]);

    let entries =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let results =
        cleanup_selected_branches(&repo, &entries, &[String::from("work/prune-me")], true);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, BranchCleanupResultStatus::Success);
    assert!(
        !branch_exists(&repo, "refs/heads/work/prune-me"),
        "local branch should be deleted"
    );
    assert!(
        !branch_exists(&repo, "refs/remotes/origin/work/prune-me"),
        "remote-tracking branch should be deleted"
    );
}

#[test]
fn cleanup_selected_branches_from_workspace_home_resolves_child_bare_repo() {
    let temp = tempdir().expect("tempdir");
    let source = temp.path().join("source");
    let workspace_home = temp.path().join("workspace-home");
    let bare = workspace_home.join("repo.git");

    run_git(
        temp.path(),
        &["init", "-q", source.to_str().expect("source path")],
    );
    init_repo(&source);
    run_git(&source, &["checkout", "-qb", "work/prune-me"]);
    std::fs::write(source.join("work.txt"), "work\n").expect("write work");
    run_git(&source, &["add", "work.txt"]);
    run_git(&source, &["commit", "-qm", "feature"]);
    run_git(&source, &["checkout", "main"]);

    std::fs::create_dir_all(&workspace_home).expect("workspace home");
    run_git(
        temp.path(),
        &[
            "clone",
            "--bare",
            source.to_str().expect("source path"),
            bare.to_str().expect("bare path"),
        ],
    );

    let entries = list_branch_entries_with_active_sessions(&workspace_home, &HashSet::new())
        .expect("entries");
    let results = cleanup_selected_branches(
        &workspace_home,
        &entries,
        &[String::from("work/prune-me")],
        false,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, BranchCleanupResultStatus::Success);
    assert!(
        !branch_exists(&bare, "refs/heads/work/prune-me"),
        "workspace cleanup should execute against the child bare repository"
    );
}

#[test]
fn cleanup_selected_branches_defaults_to_local_only_when_remote_delete_is_false() {
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
    run_git(&repo, &["checkout", "-qb", "work/local-only"]);
    std::fs::write(repo.join("work.txt"), "work\n").expect("write work");
    run_git(&repo, &["add", "work.txt"]);
    run_git(&repo, &["commit", "-qm", "feature"]);
    run_git(&repo, &["push", "-u", "origin", "work/local-only"]);
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["fetch", "origin", "--prune"]);

    let entries =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let results =
        cleanup_selected_branches(&repo, &entries, &[String::from("work/local-only")], false);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, BranchCleanupResultStatus::Success);
    assert!(
        !branch_exists(&repo, "refs/heads/work/local-only"),
        "local branch should be deleted"
    );
    assert!(
        branch_exists(&repo, "refs/remotes/origin/work/local-only"),
        "remote-tracking branch should remain without explicit remote delete"
    );
}

#[test]
fn cleanup_selected_branches_rejects_blocked_branch() {
    let repo = tempdir().expect("tempdir");

    run_git(repo.path(), &["init", "-q"]);
    init_repo(repo.path());

    let entries =
        list_branch_entries_with_active_sessions(repo.path(), &HashSet::new()).expect("entries");
    let results = cleanup_selected_branches(repo.path(), &entries, &[String::from("main")], true);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, BranchCleanupResultStatus::Failed);
    assert!(
        branch_exists(repo.path(), "refs/heads/main"),
        "protected branch should remain"
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

fn branch_exists(repo: &std::path::Path, refname: &str) -> bool {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", refname])
        .current_dir(repo)
        .output()
        .expect("run git rev-parse");
    output.status.success()
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
