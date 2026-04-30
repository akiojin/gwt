//! Integration tests for the SPEC-1934 US-6 migration entry points.
//!
//! These tests target the public API of `gwt_git::repository::RepoType` and
//! `gwt_git::migration::*`. Each test corresponds to a task in the
//! SPEC-1934 `tasks` section.

use gwt_git::migration::{bareify_local, clone_bare_from_normal, copy_hooks_to_bare};
use gwt_git::repository::{detect_repo_type, install_develop_protection, RepoType};

fn init_normal_repo(path: &std::path::Path) {
    gwt_core::process::hidden_command("git")
        .args(["init", path.to_str().unwrap()])
        .output()
        .expect("git init");
}

fn commit_initial(path: &std::path::Path) {
    gwt_core::process::hidden_command("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(path)
        .output()
        .expect("git commit");
}

fn is_bare_repo(path: &std::path::Path) -> bool {
    let output = gwt_core::process::hidden_command("git")
        .args(["rev-parse", "--is-bare-repository"])
        .current_dir(path)
        .output()
        .expect("rev-parse");
    output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
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

#[test]
fn t040_clone_bare_from_normal_creates_bare_repository() {
    // FR-021: when the Normal repo has a usable origin URL we clone it as
    // bare into the target path.
    let upstream = tempfile::tempdir().unwrap();
    init_normal_repo(upstream.path());
    commit_initial(upstream.path());

    let workspace = tempfile::tempdir().unwrap();
    let target = workspace.path().join("repo.git");
    let url = upstream.path().to_str().unwrap();

    let resolved = clone_bare_from_normal(url, &target).expect("clone_bare_from_normal");
    assert_eq!(resolved, target);
    assert!(target.exists(), "bare repo dir must exist");
    assert!(is_bare_repo(&target), "target must be a bare repository");
}

#[test]
fn t046_copy_hooks_to_bare_copies_existing_hooks_into_bare_layout() {
    // FR-022: After cloning bare from origin, the original `.git/hooks/`
    // contents must be brought across — `git clone --bare` does not preserve
    // them.
    let upstream = tempfile::tempdir().unwrap();
    init_normal_repo(upstream.path());
    commit_initial(upstream.path());

    // Pretend the user added a custom hook before the migration.
    let src_hooks = upstream.path().join(".git").join("hooks");
    std::fs::write(
        src_hooks.join("pre-push"),
        "#!/bin/sh\necho 'custom pre-push'\n",
    )
    .unwrap();

    let workspace = tempfile::tempdir().unwrap();
    let bare = workspace.path().join("repo.git");
    clone_bare_from_normal(upstream.path().to_str().unwrap(), &bare).unwrap();

    copy_hooks_to_bare(&upstream.path().join(".git"), &bare).expect("copy_hooks_to_bare");

    let copied = bare.join("hooks").join("pre-push");
    assert!(
        copied.is_file(),
        "user hook must be copied into bare layout"
    );
    assert_eq!(
        std::fs::read_to_string(&copied).unwrap(),
        "#!/bin/sh\necho 'custom pre-push'\n"
    );
}

#[test]
fn t047_install_develop_protection_works_against_bare_repo() {
    // FR-007/008: install_develop_protection must work when given a bare
    // repository path (Nested layout uses `<repo>.git/hooks/pre-commit`,
    // not `.git/hooks/...`).
    let upstream = tempfile::tempdir().unwrap();
    init_normal_repo(upstream.path());
    commit_initial(upstream.path());

    let workspace = tempfile::tempdir().unwrap();
    let bare = workspace.path().join("repo.git");
    clone_bare_from_normal(upstream.path().to_str().unwrap(), &bare).unwrap();

    install_develop_protection(&bare).expect("install hook on bare repo");

    let hook = bare.join("hooks").join("pre-commit");
    assert!(hook.is_file(), "pre-commit hook must exist in bare layout");
    let content = std::fs::read_to_string(&hook).unwrap();
    assert!(content.contains("gwt-managed"));
    assert!(content.contains("\"$branch\" = \"main\""));
}

#[test]
fn t042_bareify_local_converts_local_dot_git_when_origin_missing() {
    // Edge case: when the origin URL cannot be cloned (auth required, network
    // off, etc.), we copy the local `.git/` into a bare directory in place.
    let normal = tempfile::tempdir().unwrap();
    init_normal_repo(normal.path());
    commit_initial(normal.path());

    let target = normal.path().join("repo.git");
    let resolved = bareify_local(normal.path(), &target).expect("bareify_local");
    assert_eq!(resolved, target);
    assert!(is_bare_repo(&target), "bareify_local target must be bare");

    // Sanity: the bare clone must still know about the original branch.
    let output = gwt_core::process::hidden_command("git")
        .args(["branch", "--list"])
        .current_dir(&target)
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&output.stdout).is_empty(),
        "bareified repo must list at least one branch"
    );
}
