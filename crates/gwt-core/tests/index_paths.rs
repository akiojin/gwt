//! Phase 8: integration tests for `gwt_core::index::paths`.

use gwt_core::{
    index::paths::{gwt_index_db_path, gwt_index_repo_dir, gwt_index_root, Scope},
    repo_hash::compute_repo_hash,
    worktree_hash::compute_worktree_hash,
};

#[test]
fn gwt_index_root_ends_with_index() {
    let root = gwt_index_root();
    assert!(root.ends_with("index"));
    assert!(root.parent().unwrap().file_name().and_then(|s| s.to_str()) == Some(".gwt"));
}

#[test]
fn issue_db_path_omits_worktree_hash() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let path = gwt_index_db_path(&repo, None, Scope::Issues).unwrap();
    assert_eq!(path.file_name().and_then(|s| s.to_str()), Some("issues"));
    assert_eq!(
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|s| s.to_str()),
        Some(repo.as_str()),
        "got {}",
        path.display()
    );
}

#[test]
fn specs_db_path_is_repo_scoped() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let path = gwt_index_db_path(&repo, None, Scope::Specs).unwrap();
    assert_eq!(path.file_name().and_then(|s| s.to_str()), Some("specs"));
    assert_eq!(
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|s| s.to_str()),
        Some(repo.as_str()),
        "got {}",
        path.display()
    );
}

#[test]
fn files_code_db_path_under_worktree() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let tmp = tempfile::tempdir().unwrap();
    let wt = compute_worktree_hash(tmp.path()).unwrap();
    let path = gwt_index_db_path(&repo, Some(&wt), Scope::FilesCode).unwrap();
    assert_eq!(path.file_name().and_then(|s| s.to_str()), Some("files"));
    assert_eq!(
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|s| s.to_str()),
        Some(wt.as_str())
    );
    assert_eq!(
        path.parent()
            .and_then(|parent| parent.parent())
            .and_then(|parent| parent.file_name())
            .and_then(|s| s.to_str()),
        Some("worktrees")
    );
}

#[test]
fn files_docs_db_path_under_worktree() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let tmp = tempfile::tempdir().unwrap();
    let wt = compute_worktree_hash(tmp.path()).unwrap();
    let path = gwt_index_db_path(&repo, Some(&wt), Scope::FilesDocs).unwrap();
    assert_eq!(
        path.file_name().and_then(|s| s.to_str()),
        Some("files-docs")
    );
    assert_eq!(
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|s| s.to_str()),
        Some(wt.as_str())
    );
    assert_eq!(
        path.parent()
            .and_then(|parent| parent.parent())
            .and_then(|parent| parent.file_name())
            .and_then(|s| s.to_str()),
        Some("worktrees")
    );
}

#[test]
fn files_scope_without_worktree_hash_errors() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let result = gwt_index_db_path(&repo, None, Scope::FilesCode);
    assert!(result.is_err());
}

#[test]
fn issue_scope_with_worktree_hash_ignores_or_errors() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let tmp = tempfile::tempdir().unwrap();
    let wt = compute_worktree_hash(tmp.path()).unwrap();
    let path = gwt_index_db_path(&repo, Some(&wt), Scope::Issues).unwrap();
    // Issue scope must not contain worktree segment.
    assert!(!path.to_string_lossy().contains("worktrees"));
}

#[test]
fn gwt_index_repo_dir_layout() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let dir = gwt_index_repo_dir(&repo);
    assert!(dir.ends_with(repo.as_str()));
    assert!(dir.parent().unwrap().ends_with("index"));
}
