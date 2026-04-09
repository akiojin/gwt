//! Phase 8: integration tests for `gwt_core::worktree_hash`.

use std::fs;

use gwt_core::worktree_hash::{compute_worktree_hash, WorktreeHash};

#[test]
fn compute_worktree_hash_returns_16_hex_chars() {
    let tmp = tempfile::tempdir().unwrap();
    let h = compute_worktree_hash(tmp.path()).unwrap();
    assert_eq!(h.as_str().len(), 16);
    assert!(h.as_str().chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn compute_worktree_hash_is_deterministic_for_same_path() {
    let tmp = tempfile::tempdir().unwrap();
    let a = compute_worktree_hash(tmp.path()).unwrap();
    let b = compute_worktree_hash(tmp.path()).unwrap();
    assert_eq!(a.as_str(), b.as_str());
}

#[test]
fn compute_worktree_hash_canonicalizes_symlinks() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    fs::create_dir(&real).unwrap();
    let link = tmp.path().join("link");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&real, &link).unwrap();

    let real_hash = compute_worktree_hash(&real).unwrap();
    let link_hash = compute_worktree_hash(&link).unwrap();
    assert_eq!(real_hash.as_str(), link_hash.as_str());
}

#[test]
fn compute_worktree_hash_rejects_relative_path() {
    let result = compute_worktree_hash(std::path::Path::new("relative/path"));
    assert!(result.is_err(), "relative paths must be rejected");
}

#[test]
fn different_paths_produce_different_hashes() {
    let tmp = tempfile::tempdir().unwrap();
    let a_dir = tmp.path().join("a");
    let b_dir = tmp.path().join("b");
    fs::create_dir(&a_dir).unwrap();
    fs::create_dir(&b_dir).unwrap();
    let a = compute_worktree_hash(&a_dir).unwrap();
    let b = compute_worktree_hash(&b_dir).unwrap();
    assert_ne!(a.as_str(), b.as_str());
}

#[test]
fn worktree_hash_display() {
    let tmp = tempfile::tempdir().unwrap();
    let h: WorktreeHash = compute_worktree_hash(tmp.path()).unwrap();
    assert_eq!(format!("{h}"), h.as_str());
}
