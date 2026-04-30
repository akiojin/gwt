//! Integration tests for the SPEC-1934 US-6 migration entry points.
//!
//! These tests target the public API of `gwt_git::repository::RepoType` and
//! `gwt_git::migration::*`. Each test corresponds to a task in the
//! SPEC-1934 `tasks` section.

use gwt_git::migration::{
    add_worktree_clean, add_worktree_no_checkout, bareify_local, clone_bare_from_normal,
    copy_hooks_to_bare, evacuate_dirty_files, init_submodules, restore_evacuated_files,
    set_upstream,
};
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
fn t050_add_worktree_clean_checks_out_branch_into_target() {
    // FR-024: clean worktree migration uses plain `git worktree add` so the
    // branch contents land in `<target>` immediately.
    let upstream = tempfile::tempdir().unwrap();
    init_normal_repo(upstream.path());
    commit_initial(upstream.path());

    let workspace = tempfile::tempdir().unwrap();
    let bare = workspace.path().join("repo.git");
    clone_bare_from_normal(upstream.path().to_str().unwrap(), &bare).unwrap();

    // Find the default branch name (init.defaultBranch may differ across hosts).
    let head_output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(&bare)
        .output()
        .unwrap();
    let branch = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();

    let target = workspace.path().join(&branch);
    add_worktree_clean(&bare, &target, &branch).expect("add_worktree_clean");

    assert!(target.is_dir(), "worktree dir must exist");
    assert!(
        target.join(".git").exists(),
        "worktree must contain a .git marker"
    );
}

#[test]
fn t052_dirty_worktree_evacuate_no_checkout_restore_round_trip() {
    // FR-023: dirty file changes must survive the migration.
    // Workflow: evacuate → add_worktree_no_checkout → restore → git reset.
    let upstream = tempfile::tempdir().unwrap();
    init_normal_repo(upstream.path());
    commit_initial(upstream.path());

    let workspace = tempfile::tempdir().unwrap();
    let bare = workspace.path().join("repo.git");
    clone_bare_from_normal(upstream.path().to_str().unwrap(), &bare).unwrap();

    // Build a "dirty Normal worktree" simulation: a directory holding both
    // pre-existing tracked content (clean files committed elsewhere are not
    // available in this isolated fixture, so we limit ourselves to untracked
    // files which round-trip the same way).
    let dirty_root = workspace.path().join("dirty-source");
    std::fs::create_dir_all(&dirty_root).unwrap();
    std::fs::write(dirty_root.join("untracked.txt"), "kept").unwrap();
    std::fs::create_dir_all(dirty_root.join("nested")).unwrap();
    std::fs::write(dirty_root.join("nested").join("note.md"), "still here").unwrap();

    // Step 1: evacuate dirty files away.
    let evacuation = workspace.path().join("evacuation");
    let evacuated = evacuate_dirty_files(&dirty_root, &evacuation).expect("evacuate");
    assert!(
        evacuated.join("untracked.txt").is_file(),
        "untracked file must move into evacuation dir"
    );
    assert!(
        !dirty_root.join("untracked.txt").exists(),
        "original location must be empty after evacuation"
    );

    // Step 2: create the new worktree without checkout.
    let head_output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(&bare)
        .output()
        .unwrap();
    let branch = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();
    let new_worktree = workspace.path().join(&branch);
    add_worktree_no_checkout(&bare, &new_worktree, &branch).expect("add_worktree_no_checkout");
    assert!(new_worktree.is_dir());

    // Step 3: restore the evacuated tree into the new worktree.
    restore_evacuated_files(&evacuated, &new_worktree).expect("restore_evacuated_files");

    assert_eq!(
        std::fs::read_to_string(new_worktree.join("untracked.txt")).unwrap(),
        "kept",
        "evacuated file must be present in the new worktree"
    );
    assert_eq!(
        std::fs::read_to_string(new_worktree.join("nested").join("note.md")).unwrap(),
        "still here"
    );
}

#[test]
fn t060_init_submodules_succeeds_on_repo_without_submodules() {
    // FR-025: submodule init must be best-effort. A repo without `.gitmodules`
    // should not fail validation here (`git submodule update` exits 0).
    let upstream = tempfile::tempdir().unwrap();
    init_normal_repo(upstream.path());
    commit_initial(upstream.path());

    let workspace = tempfile::tempdir().unwrap();
    let bare = workspace.path().join("repo.git");
    clone_bare_from_normal(upstream.path().to_str().unwrap(), &bare).unwrap();

    let head_output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(&bare)
        .output()
        .unwrap();
    let branch = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();
    let target = workspace.path().join(&branch);
    add_worktree_clean(&bare, &target, &branch).unwrap();

    init_submodules(&target).expect("init_submodules must be best-effort Ok");
}

#[test]
fn t062_set_upstream_skips_when_origin_branch_is_absent() {
    // FR-026: When `origin/<branch>` is missing, set_upstream silently
    // succeeds rather than aborting the migration.
    let upstream = tempfile::tempdir().unwrap();
    init_normal_repo(upstream.path());
    commit_initial(upstream.path());

    let workspace = tempfile::tempdir().unwrap();
    let bare = workspace.path().join("repo.git");
    clone_bare_from_normal(upstream.path().to_str().unwrap(), &bare).unwrap();

    let head_output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(&bare)
        .output()
        .unwrap();
    let branch = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();
    let target = workspace.path().join(&branch);
    add_worktree_clean(&bare, &target, &branch).unwrap();

    // The bare clone has no `origin` configured (it was cloned from a local
    // path inside this test), so `origin/<branch>` does not exist. The call
    // must succeed without error.
    set_upstream(&target, &branch).expect("set_upstream must skip missing upstream gracefully");
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
