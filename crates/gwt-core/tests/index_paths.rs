//! Phase 8: integration tests for `gwt_core::index::paths`.

use gwt_core::{
    index::paths::{
        gwt_file_index_v2_base_dir, gwt_file_index_v2_cas_dir, gwt_file_index_v2_head_path,
        gwt_file_index_v2_overlay_dir, gwt_file_index_v2_root, gwt_file_index_v2_view_dir,
        gwt_index_db_path, gwt_index_repo_dir, gwt_index_root, gwt_index_worktree_dir, Scope,
    },
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

#[test]
fn file_index_v2_path_helpers_enforce_additive_layout_and_safe_artifact_ids() {
    let fixture = tempfile::tempdir().unwrap();
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let worktree = compute_worktree_hash(fixture.path()).unwrap();
    let legacy_worktree = gwt_index_worktree_dir(&repo, &worktree);
    let root = gwt_file_index_v2_root(&repo);

    assert!(root.ends_with(format!("{}/file-index-v2", repo.as_str())));
    assert_eq!(
        root.parent(),
        legacy_worktree.parent().and_then(|p| p.parent())
    );
    assert!(!root.starts_with(legacy_worktree.parent().unwrap()));

    let base = gwt_file_index_v2_base_dir(&repo, "base-123").unwrap();
    let cas = gwt_file_index_v2_cas_dir(&repo);
    let overlay = gwt_file_index_v2_overlay_dir(&repo, &worktree, "overlay-123").unwrap();
    let view = gwt_file_index_v2_view_dir(&repo, &worktree, "view-123").unwrap();
    let head = gwt_file_index_v2_head_path(&repo, &worktree);

    assert!(base.starts_with(&root));
    assert!(base.ends_with("base-123"));
    assert!(cas.starts_with(&root));
    assert!(!base
        .components()
        .any(|part| part.as_os_str() == worktree.as_str()));
    assert!(!cas
        .components()
        .any(|part| part.as_os_str() == worktree.as_str()));

    for worktree_path in [&overlay, &view, &head] {
        assert!(worktree_path.starts_with(&root));
        assert!(worktree_path
            .components()
            .any(|part| part.as_os_str() == worktree.as_str()));
    }
    assert!(overlay.ends_with("overlay-123"));
    assert!(view.ends_with("view-123"));
    assert_ne!(base, cas);
    assert_ne!(overlay, view);
    assert_ne!(view, head);

    for invalid in [
        "",
        ".",
        "..",
        "/",
        "/absolute",
        "a/b",
        r"a\b",
        "C:artifact",
        r"C:\absolute",
        r"\\server\share",
    ] {
        assert!(gwt_file_index_v2_base_dir(&repo, invalid).is_err());
        assert!(gwt_file_index_v2_overlay_dir(&repo, &worktree, invalid).is_err());
        assert!(gwt_file_index_v2_view_dir(&repo, &worktree, invalid).is_err());
    }
}
