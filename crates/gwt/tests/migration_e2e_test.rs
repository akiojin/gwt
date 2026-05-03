//! End-to-end tests for the SPEC-1934 US-6 migration orchestrator.

use std::path::Path;

use gwt::migration::execute_migration;
use gwt_core::config::BareProjectConfig;
use gwt_core::migration::{MigrationOptions, MigrationPhase, RecoveryState};

fn run_git(dir: &Path, args: &[&str]) {
    let output = gwt_core::process::hidden_command("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo_with_commit(path: &Path) {
    run_git(path, &["init", "."]);
    // Configure a deterministic identity so commits succeed in CI sandboxes.
    run_git(path, &["config", "user.email", "test@example.com"]);
    run_git(path, &["config", "user.name", "Test"]);
    std::fs::write(path.join("README.md"), "# sample\n").unwrap();
    run_git(path, &["add", "README.md"]);
    run_git(path, &["commit", "-m", "init"]);
}

fn current_branch(path: &Path) -> String {
    let output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(path)
        .output()
        .expect("symbolic-ref");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn t100_e2e_normal_to_nested_bare_worktree_layout() {
    let project = tempfile::tempdir().unwrap();
    init_repo_with_commit(project.path());
    let branch = current_branch(project.path());
    let project_dir_name = project
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo")
        .to_string();

    let outcome = execute_migration(
        project.path(),
        MigrationOptions::default(),
        |_phase, _pct| {},
    )
    .expect("execute_migration");

    let bare = project.path().join(format!("{project_dir_name}.git"));
    let worktree = project.path().join(&branch);

    assert!(bare.is_dir(), "bare repo must exist at {}", bare.display());
    assert!(
        worktree.is_dir(),
        "branch worktree must exist at {}",
        worktree.display()
    );
    assert!(
        worktree.join(".git").exists(),
        "worktree must carry a .git marker"
    );
    assert!(
        worktree.join("README.md").is_file(),
        "tracked content must land in the worktree"
    );

    // Old `.git` directory is gone, replaced by the bare repo.
    assert!(
        !project.path().join(".git").exists(),
        "old .git directory must be removed"
    );

    // .gwt/project.toml is written.
    let cfg = BareProjectConfig::load(project.path())
        .expect("load project.toml")
        .expect("project.toml exists");
    assert_eq!(cfg.bare_repo_name, format!("{project_dir_name}.git"));
    assert_eq!(cfg.migrated_from.as_deref(), Some("normal"));

    // Outcome reflects the layout.
    assert_eq!(outcome.bare_repo_path, bare);
    assert_eq!(outcome.branch_worktree_path, worktree);
}

#[test]
fn t101_dirty_normal_repo_preserves_uncommitted_changes_after_migration() {
    // SPEC-1934 US-6.3: ファイルが modified / untracked の状態で migration し
    // ても、worktree の中身は同じファイル内容で残っている。
    let project = tempfile::tempdir().unwrap();
    init_repo_with_commit(project.path());
    let branch = current_branch(project.path());

    // Modify a tracked file and add an untracked one before the migration.
    std::fs::write(project.path().join("README.md"), "# sample (dirty)\n").unwrap();
    std::fs::write(project.path().join("scratch.txt"), "untracked").unwrap();

    execute_migration(
        project.path(),
        MigrationOptions::default(),
        |_phase, _pct| {},
    )
    .expect("execute_migration");

    let worktree = project.path().join(&branch);
    assert!(worktree.is_dir());
    assert_eq!(
        std::fs::read_to_string(worktree.join("README.md")).unwrap(),
        "# sample (dirty)\n",
        "modified content must survive the migration"
    );
    assert_eq!(
        std::fs::read_to_string(worktree.join("scratch.txt")).unwrap(),
        "untracked",
        "untracked file must be preserved"
    );
}

#[test]
fn t103_e2e_locked_worktree_blocks_migration_with_no_changes() {
    // SPEC-1934 US-6.5: locked worktree が見つかったらマイグレーションを中止
    // し、リポジトリレイアウトは未変更で残らなければならない。
    let project = tempfile::tempdir().unwrap();
    init_repo_with_commit(project.path());

    let wt_path = project.path().join("locked-wt");
    run_git(
        project.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feature/locked",
            wt_path.to_str().unwrap(),
        ],
    );
    run_git(
        project.path(),
        &["worktree", "lock", wt_path.to_str().unwrap()],
    );

    let result = execute_migration(
        project.path(),
        MigrationOptions::default(),
        |_phase, _pct| {},
    );

    let err = result.expect_err("locked worktree must abort migration");
    assert_eq!(err.phase, MigrationPhase::Validate);
    assert_eq!(err.recovery, RecoveryState::Untouched);

    // Project layout must be untouched: no bare repo, no .gwt config, .git
    // directory still present.
    assert!(
        project.path().join(".git").is_dir(),
        "original .git must remain"
    );
    assert!(
        !project.path().join(".gwt").exists(),
        ".gwt config must not be written when validation aborts"
    );
}

#[test]
fn t104_e2e_failure_injected_during_bareify_rolls_back_to_original_layout() {
    // SPEC-1934 US-6.6: when a phase after Backup fails, rollback must
    // restore the original Normal Git layout. We provoke the failure by
    // pre-creating the bare target directory so `bareify_local` refuses to
    // overwrite it; the executor then triggers `rollback_migration` which
    // re-applies the backup snapshot.
    let project = tempfile::tempdir().unwrap();
    init_repo_with_commit(project.path());

    // Capture pre-migration snapshot of the project tree we care about.
    let project_dir_name = project
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap()
        .to_string();
    let bare_target = project.path().join(format!("{project_dir_name}.git"));

    // Pre-create the bare target so bareify_local fails on the
    // "target already exists" guard. clone_bare_from_normal would fail too
    // since there is no origin remote on this fixture.
    std::fs::create_dir_all(&bare_target).unwrap();
    // Drop a marker file so we can detect that the pre-existing dir was not
    // wiped by the rollback (it should be preserved as it pre-existed).
    std::fs::write(bare_target.join("preexisting.txt"), "marker").unwrap();

    let result = execute_migration(
        project.path(),
        MigrationOptions::default(),
        |_phase, _pct| {},
    );

    let err = result.expect_err("migration must fail when bare target exists");
    assert_eq!(err.phase, MigrationPhase::Bareify);
    // Recovery should reflect that rollback ran. Either RolledBack or
    // Partial is acceptable for this assertion; the important invariant is
    // that the original .git/ and tracked files are still in place.
    assert!(matches!(
        err.recovery,
        RecoveryState::RolledBack | RecoveryState::Partial
    ));

    // Original Normal Git layout must remain.
    assert!(
        project.path().join(".git").is_dir(),
        "original .git directory must survive rollback"
    );
    assert!(
        project.path().join("README.md").is_file(),
        "tracked file must survive rollback"
    );
    // The pre-existing bare-name directory we planted should still be there
    // (it was the trigger, not something we created).
    assert!(
        bare_target.join("preexisting.txt").is_file(),
        "pre-existing collision artifact must remain after rollback"
    );
    // No project.toml should have been written for a failed migration.
    assert!(
        !project.path().join(".gwt/project.toml").exists(),
        "no project.toml must be written when migration fails"
    );
}

#[test]
fn t107_repo_hash_remains_stable_across_migration() {
    // SPEC-2021 / SC-019: project_scope_hash is computed from the origin URL
    // (or the path when no origin exists). Migration must not move the
    // project root, so the hash a downstream caller would compute remains
    // identical before and after.
    let project = tempfile::tempdir().unwrap();
    init_repo_with_commit(project.path());

    let before = gwt_core::paths::project_scope_hash(project.path());

    execute_migration(
        project.path(),
        MigrationOptions::default(),
        |_phase, _pct| {},
    )
    .expect("execute_migration");

    let after = gwt_core::paths::project_scope_hash(project.path());
    assert_eq!(
        before.as_str(),
        after.as_str(),
        "project_scope_hash must remain stable across migration"
    );
}

#[test]
fn t100_e2e_progress_callback_walks_phases() {
    let project = tempfile::tempdir().unwrap();
    init_repo_with_commit(project.path());

    let mut seen: Vec<MigrationPhase> = Vec::new();
    execute_migration(project.path(), MigrationOptions::default(), |phase, _| {
        seen.push(phase);
    })
    .expect("execute_migration");

    // The orchestrator must advertise at minimum these milestones.
    for required in [
        MigrationPhase::Validate,
        MigrationPhase::Backup,
        MigrationPhase::Bareify,
        MigrationPhase::Worktrees,
        MigrationPhase::Submodules,
        MigrationPhase::Tracking,
        MigrationPhase::Cleanup,
        MigrationPhase::Done,
    ] {
        assert!(
            seen.contains(&required),
            "progress callback missing phase {required:?}, saw {seen:?}"
        );
    }
}
