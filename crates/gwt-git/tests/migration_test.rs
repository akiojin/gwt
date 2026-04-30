//! Integration tests for the SPEC-1934 US-6 migration entry points.
//!
//! These tests target the public API of `gwt_git::repository::RepoType` and
//! `gwt_git::migration::*`. Each test corresponds to a task in the
//! SPEC-1934 `tasks` section.

use gwt_git::repository::{detect_repo_type, RepoType};

fn init_normal_repo(path: &std::path::Path) {
    gwt_core::process::hidden_command("git")
        .args(["init", path.to_str().unwrap()])
        .output()
        .expect("git init");
}

#[test]
fn t010_normal_repo_reports_needs_migration_true() {
    // SPEC-1934 US-6 / FR-019: a Normal Git repo must be flagged so the
    // launcher can show the Migration confirmation modal.
    let tmp = tempfile::tempdir().unwrap();
    init_normal_repo(tmp.path());

    match detect_repo_type(tmp.path()) {
        RepoType::Normal {
            needs_migration, ..
        } => {
            assert!(
                needs_migration,
                "every Normal Git layout must be flagged for migration"
            );
        }
        other => panic!("expected RepoType::Normal {{..}}, got {other:?}"),
    }
}

#[test]
fn t010_normal_repo_path_matches_input() {
    // The `Normal` variant must still expose the resolved repository path so
    // downstream layers (workspace, runtime_support) can locate the working
    // tree.
    let tmp = tempfile::tempdir().unwrap();
    init_normal_repo(tmp.path());

    let resolved = match detect_repo_type(tmp.path()) {
        RepoType::Normal { path, .. } => path,
        other => panic!("expected RepoType::Normal {{..}}, got {other:?}"),
    };

    // Both paths point at the same directory; canonicalise to defeat
    // /private/var ↔ /var symlinks on macOS without forcing the rest of the
    // codebase to canonicalise everything.
    let resolved_canonical =
        std::fs::canonicalize(&resolved).expect("canonicalise resolved repo path");
    let tmp_canonical = std::fs::canonicalize(tmp.path()).expect("canonicalise tmp path");
    assert_eq!(resolved_canonical, tmp_canonical);
}
