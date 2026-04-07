//! Phase 8: integration tests for `gwt_core::index::paths`.

use gwt_core::index::paths::{gwt_index_db_path, gwt_index_repo_dir, gwt_index_root, Scope};
use gwt_core::repo_hash::compute_repo_hash;
use gwt_core::worktree_hash::compute_worktree_hash;

#[test]
fn gwt_index_root_ends_with_index() {
    let root = gwt_index_root();
    assert!(root.ends_with("index"));
    assert!(root.parent().unwrap().ends_with(".gwt"));
}

#[test]
fn issue_db_path_omits_worktree_hash() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let path = gwt_index_db_path(&repo, None, Scope::Issues).unwrap();
    let expected_suffix = format!(".gwt/index/{}/issues", repo.as_str());
    assert!(
        path.to_string_lossy().ends_with(&expected_suffix),
        "got {}",
        path.display()
    );
}

#[test]
fn specs_db_path_includes_worktree_hash() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let tmp = tempfile::tempdir().unwrap();
    let wt = compute_worktree_hash(tmp.path()).unwrap();
    let path = gwt_index_db_path(&repo, Some(&wt), Scope::Specs).unwrap();
    let expected_suffix = format!(
        ".gwt/index/{}/worktrees/{}/specs",
        repo.as_str(),
        wt.as_str()
    );
    assert!(
        path.to_string_lossy().ends_with(&expected_suffix),
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
    assert!(path
        .to_string_lossy()
        .ends_with(&format!("/worktrees/{}/files", wt.as_str())));
}

#[test]
fn files_docs_db_path_under_worktree() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let tmp = tempfile::tempdir().unwrap();
    let wt = compute_worktree_hash(tmp.path()).unwrap();
    let path = gwt_index_db_path(&repo, Some(&wt), Scope::FilesDocs).unwrap();
    assert!(path
        .to_string_lossy()
        .ends_with(&format!("/worktrees/{}/files-docs", wt.as_str())));
}

#[test]
fn worktree_scope_without_worktree_hash_errors() {
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let result = gwt_index_db_path(&repo, None, Scope::Specs);
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
