//! Phase 8: integration tests for `gwt_core::index::runtime::reconcile_repo`.

use std::fs;

use gwt_core::index::runtime::{reconcile_repo, ReconcileOptions};
use gwt_core::repo_hash::compute_repo_hash;
use gwt_core::worktree_hash::compute_worktree_hash;

#[test]
fn orphan_worktree_directory_is_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");

    let live_wt = tmp.path().join("live");
    fs::create_dir(&live_wt).unwrap();
    let live_hash = compute_worktree_hash(&live_wt).unwrap();

    let orphan_dir = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join("deadbeefdeadbeef");
    fs::create_dir_all(&orphan_dir).unwrap();
    fs::write(orphan_dir.join("manifest.json"), "[]").unwrap();

    let live_dir = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(live_hash.as_str());
    fs::create_dir_all(&live_dir).unwrap();

    let opts = ReconcileOptions {
        index_root: index_root.clone(),
        repo_hash: repo.clone(),
        active_worktree_paths: vec![live_wt.clone()],
        legacy_worktree_dirs: Vec::new(),
    };
    reconcile_repo(&opts).unwrap();

    assert!(!orphan_dir.exists(), "orphan dir should be removed");
    assert!(live_dir.exists(), "live dir must be preserved");
}

#[test]
fn legacy_dotgwt_index_directory_is_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let worktree = tmp.path().join("wt");
    fs::create_dir(&worktree).unwrap();
    let legacy = worktree.join(".gwt").join("index");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("dummy"), "data").unwrap();

    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let opts = ReconcileOptions {
        index_root: tmp.path().join("index"),
        repo_hash: repo,
        active_worktree_paths: vec![worktree.clone()],
        legacy_worktree_dirs: vec![worktree.clone()],
    };
    reconcile_repo(&opts).unwrap();

    assert!(
        !legacy.exists(),
        "legacy $WORKTREE/.gwt/index/ must be removed"
    );
}

#[test]
fn reconcile_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let opts = ReconcileOptions {
        index_root: tmp.path().join("index"),
        repo_hash: repo,
        active_worktree_paths: Vec::new(),
        legacy_worktree_dirs: Vec::new(),
    };
    reconcile_repo(&opts).unwrap();
    reconcile_repo(&opts).unwrap();
}
