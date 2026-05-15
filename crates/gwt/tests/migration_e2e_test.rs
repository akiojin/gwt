//! End-to-end tests for the SPEC-1934 US-6 migration orchestrator.

use std::{
    path::Path,
    time::{Duration, Instant},
};

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

fn git_stdout(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_string()
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
fn t143_e2e_remote_ahead_migration_preserves_local_head() {
    // A real copied Workbench smoke exposed that cloning origin during
    // migration can move the migrated worktree to a newer remote HEAD. The
    // migration must preserve the user's local branch HEAD and restore dirty
    // files on top of that exact commit.
    let sandbox = tempfile::tempdir().unwrap();
    let remote = sandbox.path().join("remote.git");
    let seed = sandbox.path().join("seed");
    let project = sandbox.path().join("project");

    std::fs::create_dir_all(&seed).unwrap();
    run_git(&seed, &["init", "-b", "develop", "."]);
    run_git(&seed, &["config", "user.email", "test@example.com"]);
    run_git(&seed, &["config", "user.name", "Test"]);
    std::fs::write(seed.join("README.md"), "v1\n").unwrap();
    run_git(&seed, &["add", "README.md"]);
    run_git(&seed, &["commit", "-m", "v1"]);
    run_git(
        sandbox.path(),
        &["init", "--bare", remote.to_str().unwrap()],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run_git(&seed, &["push", "-u", "origin", "develop"]);
    run_git(&remote, &["symbolic-ref", "HEAD", "refs/heads/develop"]);

    run_git(
        sandbox.path(),
        &["clone", remote.to_str().unwrap(), project.to_str().unwrap()],
    );
    run_git(&project, &["config", "user.email", "test@example.com"]);
    run_git(&project, &["config", "user.name", "Test"]);
    let local_head_before = git_stdout(&project, &["rev-parse", "HEAD"]);

    std::fs::write(seed.join("README.md"), "v2 from remote\n").unwrap();
    run_git(&seed, &["add", "README.md"]);
    run_git(&seed, &["commit", "-m", "v2"]);
    run_git(&seed, &["push", "origin", "develop"]);

    std::fs::write(project.join("README.md"), "local dirty\n").unwrap();

    execute_migration(&project, MigrationOptions::default(), |_phase, _pct| {})
        .expect("execute_migration");

    let worktree = project.join("develop");
    assert_eq!(
        git_stdout(&worktree, &["rev-parse", "HEAD"]),
        local_head_before,
        "migration must preserve the local branch HEAD, not advance to the remote HEAD"
    );
    assert_eq!(
        std::fs::read_to_string(worktree.join("README.md")).unwrap(),
        "local dirty\n",
        "dirty file content must be restored on top of the preserved local HEAD"
    );
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
fn t102_e2e_multi_worktree_migration_preserves_each_branch_worktree() {
    // SPEC-1934 US-6.4: a Normal Git repo with multiple linked worktrees must
    // migrate each worktree into the nested bare layout, not fold linked
    // worktree directories into the main branch worktree.
    let project = tempfile::tempdir().unwrap();
    init_repo_with_commit(project.path());
    let main_branch = current_branch(project.path());
    let project_dir_name = project
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo")
        .to_string();

    let clean_path = project.path().join("feature").join("clean");
    run_git(
        project.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feature/clean",
            clean_path.to_str().unwrap(),
        ],
    );
    std::fs::write(clean_path.join("feature.txt"), "clean branch\n").unwrap();
    run_git(&clean_path, &["add", "feature.txt"]);
    run_git(&clean_path, &["commit", "-m", "feature clean"]);

    let dirty_path = project.path().join("bugfix").join("dirty");
    run_git(
        project.path(),
        &[
            "worktree",
            "add",
            "-b",
            "bugfix/dirty",
            dirty_path.to_str().unwrap(),
        ],
    );
    std::fs::write(dirty_path.join("README.md"), "# sample dirty branch\n").unwrap();
    std::fs::write(dirty_path.join("scratch.txt"), "untracked dirty").unwrap();

    let outcome = execute_migration(
        project.path(),
        MigrationOptions::default(),
        |_phase, _pct| {},
    )
    .expect("execute_migration");

    let bare = project.path().join(format!("{project_dir_name}.git"));
    let main_target = project.path().join(&main_branch);
    let clean_target = project.path().join("feature").join("clean");
    let dirty_target = project.path().join("bugfix").join("dirty");

    assert!(bare.is_dir(), "bare repo must exist");
    assert!(main_target.join("README.md").is_file());
    assert_eq!(
        std::fs::read_to_string(clean_target.join("feature.txt")).unwrap(),
        "clean branch\n",
        "clean linked worktree content must stay with its branch"
    );
    assert_eq!(
        std::fs::read_to_string(dirty_target.join("README.md")).unwrap(),
        "# sample dirty branch\n",
        "dirty linked worktree modifications must survive"
    );
    assert_eq!(
        std::fs::read_to_string(dirty_target.join("scratch.txt")).unwrap(),
        "untracked dirty",
        "dirty linked worktree untracked files must survive"
    );
    assert!(
        !main_target.join("feature").exists(),
        "linked worktree directories must not be restored inside the main branch worktree"
    );

    let mut migrated = outcome.migrated_worktrees.clone();
    migrated.sort();
    let mut expected = vec![
        main_target.clone(),
        clean_target.clone(),
        dirty_target.clone(),
    ];
    expected.sort();
    assert_eq!(migrated, expected);

    for target in [&main_target, &clean_target, &dirty_target] {
        let output = gwt_core::process::hidden_command("git")
            .args(["status", "--short"])
            .current_dir(target)
            .output()
            .expect("git status");
        assert!(
            output.status.success(),
            "{} must be a valid migrated worktree: {}",
            target.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
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
fn t103_git_file_worktree_marker_aborts_before_backup() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join(".git"),
        "gitdir: ../repo.git/worktrees/feature\n",
    )
    .unwrap();
    std::fs::write(project.path().join("README.md"), "# sample\n").unwrap();

    let result = execute_migration(
        project.path(),
        MigrationOptions::default(),
        |_phase, _pct| {},
    );

    let err = result.expect_err("linked worktree marker must not be migrated as Normal Git");
    assert_eq!(err.phase, MigrationPhase::Validate);
    assert_eq!(err.recovery, RecoveryState::Untouched);
    assert!(
        err.message
            .contains("not a normal Git repository with a .git directory"),
        "unexpected migration error: {err}"
    );
    assert!(
        !project
            .path()
            .join(gwt_core::migration::backup::BACKUP_DIR_NAME)
            .exists(),
        "validation must fail before creating .gwt-migration-backup"
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

    let started = Instant::now();
    let result = execute_migration(
        project.path(),
        MigrationOptions::default(),
        |_phase, _pct| {},
    );
    let elapsed = started.elapsed();

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
    assert!(
        elapsed <= Duration::from_secs(30),
        "rollback should complete within 30 seconds for the typical E2E fixture; took {elapsed:?}"
    );
}

#[test]
fn t108_e2e_worktree_failure_rolls_back_external_linked_worktree() {
    // SPEC-1934 US-6.6 / FR-028: rollback must restore linked worktrees that
    // live outside the project root. The branch name intentionally maps to the
    // same path as the new bare repository, forcing the worktree phase to fail
    // after the old external worktree has been evacuated and removed.
    let sandbox = tempfile::tempdir().unwrap();
    let project = sandbox.path().join("repo");
    let external_parent = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(&project).unwrap();
    init_repo_with_commit(&project);

    let project_dir_name = project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap()
        .to_string();
    let conflicting_branch = format!("{project_dir_name}.git");
    let external_path = external_parent.path().join("external-worktree");

    run_git(
        &project,
        &[
            "worktree",
            "add",
            "-b",
            &conflicting_branch,
            external_path.to_str().unwrap(),
        ],
    );
    std::fs::write(
        external_path.join("README.md"),
        "# sample external dirty branch\n",
    )
    .unwrap();
    std::fs::write(external_path.join("scratch.txt"), "external untracked").unwrap();

    let result = execute_migration(&project, MigrationOptions::default(), |_phase, _pct| {});

    let err = result.expect_err("conflicting worktree target must fail migration");
    assert_eq!(err.phase, MigrationPhase::Worktrees);
    assert_eq!(err.recovery, RecoveryState::RolledBack);

    assert!(
        project.join(".git").is_dir(),
        "original project .git directory must be restored"
    );
    assert!(
        !project.join(".gwt/project.toml").exists(),
        "failed migration must not leave project.toml"
    );
    assert!(
        external_path.is_dir(),
        "external linked worktree must be restored after rollback"
    );
    assert!(
        external_path.join(".git").is_file(),
        "external linked worktree git marker must be restored"
    );
    assert_eq!(
        std::fs::read_to_string(external_path.join("README.md")).unwrap(),
        "# sample external dirty branch\n",
        "external tracked modifications must survive rollback"
    );
    assert_eq!(
        std::fs::read_to_string(external_path.join("scratch.txt")).unwrap(),
        "external untracked",
        "external untracked files must survive rollback"
    );

    let status = gwt_core::process::hidden_command("git")
        .args(["status", "--short"])
        .current_dir(&external_path)
        .output()
        .expect("git status");
    assert!(
        status.status.success(),
        "external worktree must remain a valid Git worktree: {}",
        String::from_utf8_lossy(&status.stderr)
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
