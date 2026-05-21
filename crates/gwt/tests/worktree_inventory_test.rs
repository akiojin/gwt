use std::path::Path;
use std::process::Command;

use gwt::worktree_inventory::{
    enumerate_worktrees, enumerate_worktrees_with_sessions_dir, WorktreeEntryKind,
};
use tempfile::tempdir;

fn git(args: &[&str], cwd: &Path) {
    let status = Command::new("git")
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
fn enumerate_worktrees_includes_session_ids_for_each_worktree() {
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
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
    for session_id in ["session-b", "session-a"] {
        let mut session =
            gwt_agent::Session::new(&worktree_path, "feature/a", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.save(&sessions_dir).expect("save session");
    }
    let mut main_session = gwt_agent::Session::new(&repo, "main", gwt_agent::AgentId::ClaudeCode);
    main_session.id = "session-main".to_string();
    main_session.save(&sessions_dir).expect("save main session");

    let entries = enumerate_worktrees_with_sessions_dir(&repo, Some(&worktree_path), &sessions_dir)
        .expect("inventory");

    assert_eq!(entries[0].session_ids, vec!["session-main".to_string()]);
    assert_eq!(
        entries[1].session_ids,
        vec!["session-a".to_string(), "session-b".to_string()]
    );
}
