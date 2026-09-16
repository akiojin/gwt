use std::path::Path;

use gwt::worktree_inventory::{enumerate_worktrees, WorktreeEntryKind};
use gwt_core::process::hidden_command;
use tempfile::tempdir;

fn git(args: &[&str], cwd: &Path) {
    let status = hidden_command("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed in {cwd:?}");
}

fn init_repo(repo: &Path) {
    std::fs::create_dir_all(repo).expect("create repo dir");
    git(&["init", "--initial-branch=main"], repo);
    git(&["config", "user.email", "test@example.com"], repo);
    git(&["config", "user.name", "tester"], repo);
    std::fs::write(repo.join("README.md"), "hello\n").expect("write readme");
    git(&["add", "README.md"], repo);
    git(&["commit", "-m", "init"], repo);
}

#[test]
fn enumerate_worktrees_returns_main_only_for_fresh_repo() {
    let dir = tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);

    let entries = enumerate_worktrees(&repo, Some(&repo)).expect("inventory");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, WorktreeEntryKind::BareMain);
    assert!(
        entries[0].is_active,
        "active flag should follow active_root"
    );
    assert_eq!(entries[0].label, "main repository");
}

#[test]
fn enumerate_worktrees_lists_main_and_workspace_entries() {
    let dir = tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);

    let worktree_path = dir.path().join("worktrees").join("feature-a");
    git(
        &[
            "worktree",
            "add",
            "-b",
            "feature/a",
            worktree_path.to_str().expect("path str"),
        ],
        &repo,
    );

    let entries = enumerate_worktrees(&repo, Some(&worktree_path)).expect("inventory");
    assert_eq!(entries.len(), 2);

    // BareMain should be first per ordering rule.
    assert_eq!(entries[0].kind, WorktreeEntryKind::BareMain);
    assert!(!entries[0].is_active);
    assert_eq!(entries[1].kind, WorktreeEntryKind::Workspace);
    assert!(entries[1].is_active, "active flag should mark feature/a");
    assert_eq!(entries[1].branch.as_deref(), Some("feature/a"));
    assert_eq!(entries[1].label, "feature/a");

    // IDs are stable and differ between entries.
    assert_ne!(entries[0].id, entries[1].id);
    assert_eq!(entries[0].id.len(), 16);
}

#[test]
fn enumerate_worktrees_skips_prunable_entries() {
    let dir = tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);

    let worktree_path = dir.path().join("worktrees").join("ephemeral");
    git(
        &[
            "worktree",
            "add",
            "-b",
            "feature/ephemeral",
            worktree_path.to_str().expect("path str"),
        ],
        &repo,
    );

    // Delete the worktree directory on disk without invoking `git worktree
    // remove`, then run `git worktree prune` to mark it prunable.
    std::fs::remove_dir_all(&worktree_path).expect("remove worktree dir");
    git(&["worktree", "prune"], &repo);

    let entries = enumerate_worktrees(&repo, None).expect("inventory");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, WorktreeEntryKind::BareMain);
}

#[test]
fn enumerate_worktrees_lists_entries_from_workspace_home_with_child_bare_repo() {
    // Regression: the File Tree "SELECT WORKTREE" picker passes the project
    // root (which may be a "workspace home" directory that contains a child
    // bare repo but is not itself a git work tree) to `enumerate_worktrees`.
    // Running `git worktree list` directly in that home dir fails with
    // "not a git repository (or any of the parent directories): .git". The
    // inventory must instead resolve the bare repo and enumerate its worktrees.
    let dir = tempdir().expect("tempdir");
    let home = dir.path(); // workspace home — NOT a git work tree itself
    let bare = home.join("project.git");
    let bare_str = bare.to_str().expect("bare path str");

    git(&["init", "--bare", "--initial-branch=main", bare_str], home);

    // Bootstrap an initial commit so the bare repo has a ref to base worktrees on.
    let bootstrap = home.join("bootstrap");
    git(
        &[
            "clone",
            bare_str,
            bootstrap.to_str().expect("bootstrap str"),
        ],
        home,
    );
    git(&["config", "user.email", "test@example.com"], &bootstrap);
    git(&["config", "user.name", "tester"], &bootstrap);
    std::fs::write(bootstrap.join("README.md"), "hello\n").expect("write readme");
    git(&["add", "README.md"], &bootstrap);
    git(&["commit", "-m", "init"], &bootstrap);
    git(&["push", "origin", "HEAD:refs/heads/main"], &bootstrap);

    // Add a linked worktree directly from the bare repo.
    let worktree_path = home.join("feature-a");
    git(
        &[
            "worktree",
            "add",
            "-b",
            "feature/a",
            worktree_path.to_str().expect("worktree str"),
            "main",
        ],
        &bare,
    );

    let entries = enumerate_worktrees(home, Some(home))
        .expect("inventory should succeed for a workspace home with a child bare repo");

    assert!(
        entries
            .iter()
            .any(|entry| entry.kind == WorktreeEntryKind::BareMain),
        "the bare repo should be listed as the main entry: {entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.branch.as_deref() == Some("feature/a")),
        "the linked worktree feature/a should be listed: {entries:?}"
    );
}

#[test]
fn worktree_entries_carry_no_session_ids() {
    // Issue #4377: the inventory used to parse every Session TOML under
    // `~/.gwt/sessions` to attach session ids that nothing read, once per
    // project tab at startup. The picker contract has no such field.
    let dir = tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);

    let worktree_path = dir.path().join("worktrees").join("feature-a");
    git(
        &[
            "worktree",
            "add",
            "-b",
            "feature/a",
            worktree_path.to_str().expect("path str"),
        ],
        &repo,
    );

    let entries = enumerate_worktrees(&repo, Some(&worktree_path)).expect("inventory");

    assert_eq!(entries.len(), 2);
    let json = serde_json::to_value(&entries).expect("serialize entries");
    assert!(
        json.as_array()
            .expect("entries array")
            .iter()
            .all(|entry| entry.get("session_ids").is_none()),
        "worktree entries must not carry session ids: {json}"
    );
}
