//! SPEC-1934 US-7 / T-174 end-to-end smoke for the `--single-branch` Normal
//! Git layout (e.g. the GitHub UI clone shape that produced the llmlb
//! incident: `fetch = +refs/heads/develop:refs/remotes/origin/develop`).
//!
//! This drives `gwt::migration::execute_migration` against an isolated
//! fixture and asserts that the migrated bare repository has its
//! `remote.origin.fetch` normalized to the wildcard form and that branches
//! the original single-branch refspec hid are now visible as
//! `refs/remotes/origin/*`. It is the regression test for the original
//! `Git error: fatal: invalid reference: origin/work/<branch>` failure.
//!
//! The Workspace Start Work materialization path that consumes this state is
//! exercised by `crates/gwt-git/src/worktree.rs` unit tests and the
//! `app_runtime::tests::open_existing_branch_refuses_while_migration_pending`
//! guard test; this E2E focuses on the migration executor itself.

use std::path::Path;

use gwt::migration::execute_migration;
use gwt_core::migration::types::{MigrationOptions, MigrationPhase};
use tempfile::tempdir;

fn run_git(args: &[&str], cwd: &Path) {
    let status = gwt_core::process::hidden_command("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("git command spawn");
    assert!(
        status.success(),
        "git {args:?} in {} must succeed",
        cwd.display()
    );
}

/// Configure a deterministic identity inside a freshly-created Git repo so
/// `git commit` works on CI runners that do not provision a global
/// `user.email` / `user.name` (Linux ubuntu-latest is the canonical case).
fn set_test_identity(repo: &Path) {
    run_git(["config", "user.email", "test@example.com"].as_ref(), repo);
    run_git(["config", "user.name", "Test"].as_ref(), repo);
}

fn read_remote_fetch_refspec(repo: &Path) -> Option<String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["config", "--get", "remote.origin.fetch"])
        .current_dir(repo)
        .output()
        .expect("git config --get remote.origin.fetch");
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn list_remote_origin_refs(repo: &Path) -> String {
    let output = gwt_core::process::hidden_command("git")
        .args([
            "for-each-ref",
            "--format=%(refname)",
            "refs/remotes/origin/",
        ])
        .current_dir(repo)
        .output()
        .expect("git for-each-ref");
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Build the bare upstream that the local clone targets. Two branches are
/// populated so the migration's wildcard fetch can prove it pulls down the
/// branch that the single-branch refspec previously masked.
fn build_upstream(workspace: &Path, upstream_dir: &Path) {
    // `git init --bare <path>` creates `<path>` itself, so run with `cwd =
    // workspace` to avoid the chicken-and-egg of cd'ing into an unborn dir.
    run_git(
        [
            "init",
            "--bare",
            upstream_dir.to_str().expect("upstream path"),
        ]
        .as_ref(),
        workspace,
    );

    // Seed an initial commit + `develop` on a scratch normal repo, then push
    // both branches up. Pushing through a scratch tree keeps the upstream
    // bare (no working tree) while still owning real history.
    let scratch = tempdir().expect("scratch tempdir for upstream seed");
    run_git(["init"].as_ref(), scratch.path());
    set_test_identity(scratch.path());
    run_git(
        [
            "remote",
            "add",
            "origin",
            upstream_dir.to_str().expect("upstream path"),
        ]
        .as_ref(),
        scratch.path(),
    );
    run_git(
        ["commit", "--allow-empty", "-m", "seed"].as_ref(),
        scratch.path(),
    );
    // Force the seed branch to be named `develop` so the single-branch
    // refspec under test is the realistic one.
    run_git(["branch", "-M", "develop"].as_ref(), scratch.path());
    run_git(["push", "-u", "origin", "develop"].as_ref(), scratch.path());
    // Add a second branch (`feature/hidden`) that the single-branch refspec
    // would have masked from the local mirror.
    run_git(["branch", "feature/hidden"].as_ref(), scratch.path());
    run_git(
        ["push", "origin", "feature/hidden"].as_ref(),
        scratch.path(),
    );
}

/// Reproduce the local layout that triggered the llmlb failure: a Normal
/// Git working tree with an `origin` whose `fetch` refspec is restricted to
/// `develop` only.
fn build_local_single_branch_clone(workspace: &Path, local_dir: &Path, upstream_dir: &Path) {
    // `--single-branch -b develop` is the exact shape GitHub UI clones use.
    // `git clone` creates `<local>` itself so we run with `cwd = workspace`.
    run_git(
        [
            "clone",
            "--single-branch",
            "-b",
            "develop",
            upstream_dir.to_str().expect("upstream path"),
            local_dir.to_str().expect("local path"),
        ]
        .as_ref(),
        workspace,
    );
    // Sanity: confirm the cloned config has the single-branch refspec.
    let initial = read_remote_fetch_refspec(local_dir);
    assert_eq!(
        initial.as_deref(),
        Some("+refs/heads/develop:refs/remotes/origin/develop"),
        "fixture must reproduce the single-branch fetch refspec exactly"
    );
}

#[test]
fn t174_single_branch_normal_repo_migrates_and_normalizes_fetch_refspec() {
    // Build upstream + local Normal layout outside the target dir so the
    // migration runs over a clean fixture without sibling noise.
    let workspace = tempdir().expect("workspace tempdir");
    let upstream_dir = workspace.path().join("upstream.git");
    let local_dir = workspace.path().join("local");
    build_upstream(workspace.path(), &upstream_dir);
    build_local_single_branch_clone(workspace.path(), &local_dir, &upstream_dir);

    // Sanity: the single-branch clone only knows about `origin/develop`.
    let before_refs = list_remote_origin_refs(&local_dir);
    assert!(
        before_refs.contains("refs/remotes/origin/develop"),
        "fixture must have origin/develop locally: {before_refs}"
    );
    assert!(
        !before_refs.contains("refs/remotes/origin/feature/hidden"),
        "fixture must hide origin/feature/hidden behind the single-branch refspec: {before_refs}"
    );

    let mut last_phase: Option<MigrationPhase> = None;
    let outcome = execute_migration(
        &local_dir,
        MigrationOptions::default(),
        |phase, _percent| {
            last_phase = Some(phase);
        },
    )
    .expect("migration must succeed end-to-end");

    assert_eq!(
        last_phase,
        Some(MigrationPhase::Done),
        "executor must reach the Done phase on success"
    );

    // The migrated bare repo lives under the original project_root with the
    // derived `<repo>.git` directory. Its `remote.origin.fetch` must now be
    // the wildcard form so future `git fetch origin --prune` syncs every
    // branch into `refs/remotes/origin/*`.
    assert!(
        outcome.bare_repo_path.is_dir(),
        "bare repo dir must exist after migration: {}",
        outcome.bare_repo_path.display()
    );
    assert_eq!(
        read_remote_fetch_refspec(&outcome.bare_repo_path).as_deref(),
        Some("+refs/heads/*:refs/remotes/origin/*"),
        "SPEC-1934 FR-033: migrated origin must use the wildcard fetch refspec"
    );

    // The hidden branch must now be present in the local mirror — this is
    // the exact pre-condition that the broken Workspace Start Work flow was
    // failing because it depended on `refs/remotes/origin/work/<branch>`
    // arriving via the wildcard fetch.
    let after_refs = list_remote_origin_refs(&outcome.bare_repo_path);
    assert!(
        after_refs.contains("refs/remotes/origin/develop"),
        "develop must remain visible after migration: {after_refs}"
    );
    assert!(
        after_refs.contains("refs/remotes/origin/feature/hidden"),
        "feature/hidden must arrive once the wildcard refspec replaces the single-branch one: {after_refs}"
    );

    // Branch worktree must materialize alongside the bare repo so the
    // Workspace Home / Start Work path has a checkout to operate on.
    assert!(
        outcome.branch_worktree_path.is_dir(),
        "branch worktree must exist after migration: {}",
        outcome.branch_worktree_path.display()
    );
}
