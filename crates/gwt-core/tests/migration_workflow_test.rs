//! Workflow-level tests for the SPEC-1934 US-6 migration support.

use gwt_core::config::BareProjectConfig;
use gwt_core::migration::backup::{self, BACKUP_DIR_NAME};
use gwt_core::migration::executor;
use gwt_core::migration::rollback;
use gwt_core::migration::validator::{
    self, check_disk_space, check_locked_worktrees, check_write_permission, evaluate_disk_space,
    ValidationError,
};
use gwt_core::migration::MigrationOptions;
use gwt_core::migration::{MigrationError, MigrationPhase, RecoveryState};

#[test]
fn t013_bare_project_config_round_trip() {
    let tmp = tempfile::tempdir().unwrap();

    let original = BareProjectConfig {
        bare_repo_name: "llmlb.git".into(),
        remote_url: Some("https://github.com/akiojin/llmlb".into()),
        created_at: "2026-04-30T12:34:56Z".into(),
        migrated_from: Some("normal".into()),
    };

    original.save(tmp.path()).expect("save project.toml");

    let path = BareProjectConfig::config_path(tmp.path());
    assert!(path.exists(), "project.toml must exist after save");

    let loaded = BareProjectConfig::load(tmp.path())
        .expect("load project.toml")
        .expect("project.toml exists");
    assert_eq!(loaded, original);
}

#[test]
fn t013_bare_project_config_load_returns_none_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let result = BareProjectConfig::load(tmp.path()).expect("load absent project.toml");
    assert!(result.is_none(), "absent config must yield None");
}

#[test]
fn t013_bare_project_config_save_creates_dot_gwt_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = BareProjectConfig {
        bare_repo_name: "x.git".into(),
        remote_url: None,
        created_at: "2026-04-30T00:00:00Z".into(),
        migrated_from: None,
    };
    cfg.save(tmp.path()).expect("save project.toml");
    assert!(tmp.path().join(".gwt").is_dir());
    assert!(tmp.path().join(".gwt/project.toml").is_file());
}

#[test]
fn t020_evaluate_disk_space_rejects_when_required_exceeds_available() {
    // FR-020 / Edge Case: マイグレーションは backup を作成するので空き容量が
    // 元 repo の倍以上必要。required > available で
    // ValidationError::InsufficientDiskSpace が返らなければならない。
    let result = evaluate_disk_space(10_000, 5_000);
    match result {
        Err(ValidationError::InsufficientDiskSpace {
            required,
            available,
        }) => {
            assert_eq!(required, 10_000);
            assert_eq!(available, 5_000);
        }
        other => panic!("expected InsufficientDiskSpace, got {other:?}"),
    }
}

#[test]
fn t020_evaluate_disk_space_accepts_when_available_meets_required() {
    assert!(matches!(evaluate_disk_space(1_000, 1_000), Ok(())));
    assert!(matches!(evaluate_disk_space(1_000, 1_500), Ok(())));
    assert!(matches!(evaluate_disk_space(0, 0), Ok(())));
}

#[test]
fn t020_check_disk_space_passes_for_small_tempdir() {
    // Live filesystem check. Tempdirs created by this test fixture are kilobyte
    // sized so unless the test host is full this must succeed.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("hello.txt"), "world").unwrap();
    check_disk_space(tmp.path()).expect("clean tempdir must pass disk check");
}

#[test]
fn t022_check_locked_worktrees_returns_ok_when_no_worktrees_locked() {
    let tmp = tempfile::tempdir().unwrap();
    gwt_core::process::hidden_command("git")
        .args(["init", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();
    gwt_core::process::hidden_command("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    check_locked_worktrees(tmp.path()).expect("unlocked repo must pass");
}

#[test]
fn t022_check_locked_worktrees_detects_locked_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    gwt_core::process::hidden_command("git")
        .args(["init", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();
    gwt_core::process::hidden_command("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let wt_path = tmp.path().join("wt-feature");
    gwt_core::process::hidden_command("git")
        .args([
            "worktree",
            "add",
            "-b",
            "feature/x",
            wt_path.to_str().unwrap(),
        ])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    gwt_core::process::hidden_command("git")
        .args(["worktree", "lock", wt_path.to_str().unwrap()])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let err =
        check_locked_worktrees(tmp.path()).expect_err("locked worktree must surface as error");
    match err {
        ValidationError::LockedWorktrees(paths) => {
            assert!(
                paths.iter().any(|p| {
                    std::fs::canonicalize(p)
                        .map(|c| c == std::fs::canonicalize(&wt_path).unwrap())
                        .unwrap_or(false)
                }),
                "locked worktree path must be reported, got {paths:?}"
            );
        }
        other => panic!("expected LockedWorktrees, got {other:?}"),
    }
}

#[test]
fn t024_check_write_permission_passes_for_writable_dir() {
    let tmp = tempfile::tempdir().unwrap();
    check_write_permission(tmp.path()).expect("writable dir must pass");
}

#[cfg(unix)]
#[test]
fn t024_check_write_permission_fails_for_readonly_dir() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let mut perms = std::fs::metadata(tmp.path()).unwrap().permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(tmp.path(), perms).unwrap();

    let result = check_write_permission(tmp.path());

    // 後始末: パーミッションを書き戻して tempdir が削除できるようにする。
    let mut restore = std::fs::metadata(tmp.path()).unwrap().permissions();
    restore.set_mode(0o755);
    std::fs::set_permissions(tmp.path(), restore).unwrap();

    match result {
        Err(ValidationError::WritePermissionDenied(path)) => {
            assert_eq!(
                std::fs::canonicalize(&path).unwrap(),
                std::fs::canonicalize(tmp.path()).unwrap()
            );
        }
        other => panic!("expected WritePermissionDenied, got {other:?}"),
    }
}

#[test]
fn t026_validate_aggregates_all_checks_for_clean_normal_repo() {
    let tmp = tempfile::tempdir().unwrap();
    gwt_core::process::hidden_command("git")
        .args(["init", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();
    gwt_core::process::hidden_command("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    validator::validate(tmp.path()).expect("clean Normal Git must pass aggregate validate()");
}

#[test]
fn t026_validate_rejects_git_file_worktree_marker_before_backup() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join(".git"),
        "gitdir: ../repo.git/worktrees/feature\n",
    )
    .unwrap();

    let err = validator::validate(tmp.path())
        .expect_err("linked worktree markers must not be valid Normal Git migration roots");

    assert!(
        err.to_string()
            .contains("not a normal Git repository with a .git directory"),
        "unexpected validation error: {err}"
    );
}

#[test]
fn t030_backup_create_copies_tree_into_backup_dir() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "alpha").unwrap();
    std::fs::create_dir_all(tmp.path().join("nested")).unwrap();
    std::fs::write(tmp.path().join("nested").join("b.txt"), "beta").unwrap();

    let snapshot = backup::create(tmp.path()).expect("backup::create");
    assert_eq!(snapshot.project_root, tmp.path());

    let backup_dir = tmp.path().join(BACKUP_DIR_NAME);
    assert!(backup_dir.is_dir(), "backup dir must exist");
    assert_eq!(snapshot.backup_dir, backup_dir);

    assert!(backup_dir.join("a.txt").is_file());
    assert!(backup_dir.join("nested").join("b.txt").is_file());
    assert_eq!(
        std::fs::read_to_string(backup_dir.join("a.txt")).unwrap(),
        "alpha"
    );
}

#[cfg(unix)]
#[test]
fn t030_backup_create_skips_symlinks_in_project_and_nested_dirs() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("target.txt"), "target").unwrap();
    symlink("target.txt", tmp.path().join("link.txt")).unwrap();
    std::fs::create_dir_all(tmp.path().join("nested")).unwrap();
    std::fs::write(tmp.path().join("nested").join("inner.txt"), "inner").unwrap();
    symlink(
        "inner.txt",
        tmp.path().join("nested").join("inner-link.txt"),
    )
    .unwrap();

    let snapshot = backup::create(tmp.path()).expect("backup::create");

    assert!(snapshot.backup_dir.join("target.txt").is_file());
    assert!(
        !snapshot.backup_dir.join("link.txt").exists(),
        "top-level symlinks must not be copied into the backup"
    );
    assert!(snapshot
        .backup_dir
        .join("nested")
        .join("inner.txt")
        .is_file());
    assert!(
        !snapshot
            .backup_dir
            .join("nested")
            .join("inner-link.txt")
            .exists(),
        "nested symlinks must not be copied into the backup"
    );
}

#[test]
fn t030_backup_create_excludes_self() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("real.txt"), "x").unwrap();

    let snapshot = backup::create(tmp.path()).expect("backup::create");

    // The backup directory must not contain a recursive copy of itself.
    assert!(!snapshot.backup_dir.join(BACKUP_DIR_NAME).exists());
    assert!(snapshot.backup_dir.join("real.txt").is_file());
}

#[test]
fn t030_backup_create_with_external_roots_skips_missing_and_sanitizes_names() {
    let project = tempfile::tempdir().unwrap();
    let external_parent = tempfile::tempdir().unwrap();
    let external = external_parent.path().join("linked worktree@one");
    std::fs::create_dir_all(&external).unwrap();
    std::fs::write(external.join("dirty.txt"), "dirty").unwrap();
    let missing = external_parent.path().join("missing-worktree");

    let snapshot =
        backup::create_with_external_roots(project.path(), &[missing.clone(), external.clone()])
            .expect("backup::create_with_external_roots");

    assert_eq!(snapshot.external_roots.len(), 1);
    let external_snapshot = &snapshot.external_roots[0];
    assert_eq!(external_snapshot.original_path, external);
    assert!(external_snapshot
        .backup_path
        .ends_with("1-linked_worktree_one"));
    assert!(external_snapshot.backup_path.join("dirty.txt").is_file());
    assert!(
        !snapshot
            .backup_dir
            .join(".external-worktrees")
            .join("0-missing-worktree")
            .exists(),
        "missing external roots must be ignored instead of creating empty backups"
    );
}

#[test]
fn t032_backup_create_renames_existing_backup() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("seed.txt"), "fresh").unwrap();
    let backup_dir = tmp.path().join(BACKUP_DIR_NAME);
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::write(backup_dir.join("stale.txt"), "stale").unwrap();

    let snapshot = backup::create(tmp.path()).expect("backup::create");

    assert!(snapshot.backup_dir.join("seed.txt").is_file());

    // The legacy backup must be renamed (any sibling starting with the backup
    // dir name + "-" is acceptable; we just ensure stale content exists somewhere).
    let mut found_legacy = false;
    for entry in std::fs::read_dir(tmp.path()).unwrap().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&format!("{BACKUP_DIR_NAME}-")) {
            let stale = entry.path().join("stale.txt");
            if stale.is_file() && std::fs::read_to_string(&stale).unwrap_or_default() == "stale" {
                found_legacy = true;
                break;
            }
        }
    }
    assert!(
        found_legacy,
        "legacy backup must be preserved with timestamp suffix"
    );
}

#[test]
fn t034_backup_restore_restores_external_roots_and_removes_added_dirs() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("root.txt"), "v1").unwrap();
    let external_parent = tempfile::tempdir().unwrap();
    let external = external_parent.path().join("linked-worktree");
    std::fs::create_dir_all(external.join("dir")).unwrap();
    std::fs::write(external.join("dir").join("tracked.txt"), "before").unwrap();

    let snapshot =
        backup::create_with_external_roots(project.path(), std::slice::from_ref(&external))
            .expect("backup::create_with_external_roots");

    std::fs::remove_file(project.path().join("root.txt")).unwrap();
    std::fs::write(project.path().join("migration-junk.txt"), "junk").unwrap();
    std::fs::remove_dir_all(&external).unwrap();
    std::fs::create_dir_all(external.join("new-dir")).unwrap();
    std::fs::write(external.join("new-dir").join("junk.txt"), "junk").unwrap();

    backup::restore(&snapshot).expect("backup::restore");

    assert_eq!(
        std::fs::read_to_string(project.path().join("root.txt")).unwrap(),
        "v1"
    );
    assert!(!project.path().join("migration-junk.txt").exists());
    assert_eq!(
        std::fs::read_to_string(external.join("dir").join("tracked.txt")).unwrap(),
        "before"
    );
    assert!(!external.join("new-dir").exists());
}

#[test]
fn t034_backup_restore_returns_files_to_project_root() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("keep.txt"), "v1").unwrap();
    std::fs::create_dir_all(tmp.path().join("dir")).unwrap();
    std::fs::write(tmp.path().join("dir").join("nested.txt"), "n1").unwrap();

    let snapshot = backup::create(tmp.path()).expect("backup::create");

    // Mutate the live tree to simulate a partial migration before restore.
    std::fs::remove_file(tmp.path().join("keep.txt")).unwrap();
    std::fs::write(tmp.path().join("dir").join("nested.txt"), "tampered").unwrap();
    std::fs::write(tmp.path().join("new.txt"), "added during migration").unwrap();

    backup::restore(&snapshot).expect("backup::restore");

    assert_eq!(
        std::fs::read_to_string(tmp.path().join("keep.txt")).unwrap(),
        "v1",
        "removed file must be brought back"
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("dir").join("nested.txt")).unwrap(),
        "n1",
        "tampered file must be reverted"
    );
    assert!(
        !tmp.path().join("new.txt").exists(),
        "files added after backup must be cleared on restore"
    );
}

#[test]
fn t035_backup_discard_removes_existing_snapshot_and_ignores_missing_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "alpha").unwrap();
    let snapshot = backup::create(tmp.path()).expect("backup::create");
    let backup_dir = snapshot.backup_dir.clone();

    backup::discard(snapshot).expect("discard existing backup");
    assert!(!backup_dir.exists());

    let missing_snapshot = backup::BackupSnapshot {
        project_root: tmp.path().to_path_buf(),
        backup_dir,
        external_roots: Vec::new(),
        pre_normalize_fetch_refspec: None,
    };
    backup::discard(missing_snapshot).expect("discard missing backup is a no-op");
}

#[test]
fn t036_rollback_uses_backup_to_restore_partial_changes() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a"), "1").unwrap();
    let snapshot = backup::create(tmp.path()).expect("backup::create");

    // Pretend a phase failed and left junk.
    std::fs::write(tmp.path().join("partial.bin"), "garbage").unwrap();
    std::fs::remove_file(tmp.path().join("a")).unwrap();

    rollback::rollback_migration(&snapshot).expect("rollback");

    assert_eq!(
        std::fs::read_to_string(tmp.path().join("a")).unwrap(),
        "1",
        "rollback must restore deleted files"
    );
    assert!(
        !tmp.path().join("partial.bin").exists(),
        "rollback must clear partial files"
    );
}

#[test]
fn t036_rollback_surfaces_backup_restore_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let snapshot = backup::BackupSnapshot {
        project_root: tmp.path().join("missing-project-root"),
        backup_dir: tmp.path().join("missing-backup"),
        external_roots: Vec::new(),
        pre_normalize_fetch_refspec: None,
    };

    let err = rollback::rollback_migration(&snapshot).expect_err("rollback must fail");
    let message = err.to_string();
    assert!(message.contains("rollback failed: backup io error:"));
}

/// Helper: initialize a Normal Git repo with an `origin` remote whose
/// `remote.origin.fetch` is the single-branch shape a GitHub-UI clone leaves.
fn init_repo_with_single_branch_origin(repo: &std::path::Path) {
    gwt_core::process::hidden_command("git")
        .args(["init", repo.to_str().unwrap()])
        .output()
        .unwrap();
    gwt_core::process::hidden_command("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ])
        .current_dir(repo)
        .output()
        .unwrap();
    gwt_core::process::hidden_command("git")
        .args([
            "config",
            "remote.origin.fetch",
            "+refs/heads/develop:refs/remotes/origin/develop",
        ])
        .current_dir(repo)
        .output()
        .unwrap();
}

fn read_origin_fetch_refspec(repo: &std::path::Path) -> Option<String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["config", "--get", "remote.origin.fetch"])
        .current_dir(repo)
        .output()
        .unwrap();
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

#[test]
fn t154_backup_snapshot_records_pre_normalize_fetch_refspec() {
    // RED: the migration backup snapshot must be able to carry the project's
    // pre-normalize `remote.origin.fetch` value so rollback can restore it
    // (SPEC-1934 US-7 / FR-033). A fresh `backup::create` defaults to `None`;
    // the executor populates it before the refspec is normalized.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("seed.txt"), "x").unwrap();

    let mut snapshot = backup::create(tmp.path()).expect("backup::create");
    assert_eq!(
        snapshot.pre_normalize_fetch_refspec, None,
        "a fresh backup snapshot must default the pre-normalize refspec to None"
    );

    snapshot.pre_normalize_fetch_refspec =
        Some("+refs/heads/develop:refs/remotes/origin/develop".to_string());
    assert_eq!(
        snapshot.pre_normalize_fetch_refspec.as_deref(),
        Some("+refs/heads/develop:refs/remotes/origin/develop"),
        "the snapshot must record the single-branch refspec captured before normalize"
    );
}

#[test]
fn t155_rollback_restores_pre_normalize_fetch_refspec() {
    // RED: when the snapshot carries a pre-normalize refspec, rollback must
    // write it back to `remote.origin.fetch` in the restored project so a
    // partially-migrated repo returns to its original single-branch fetch
    // configuration (SPEC-1934 US-7, T-155 / T-158).
    let tmp = tempfile::tempdir().unwrap();
    init_repo_with_single_branch_origin(tmp.path());

    let mut snapshot = backup::create(tmp.path()).expect("backup::create");
    snapshot.pre_normalize_fetch_refspec = read_origin_fetch_refspec(tmp.path());
    assert_eq!(
        snapshot.pre_normalize_fetch_refspec.as_deref(),
        Some("+refs/heads/develop:refs/remotes/origin/develop"),
        "fixture must capture the single-branch refspec before normalize"
    );

    // Simulate a normalize step that already rewrote the live config to the
    // wildcard form before a later phase failed.
    gwt_core::process::hidden_command("git")
        .args([
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert_eq!(
        read_origin_fetch_refspec(tmp.path()).as_deref(),
        Some("+refs/heads/*:refs/remotes/origin/*"),
        "precondition: live config is normalized to the wildcard form"
    );

    rollback::rollback_migration(&snapshot).expect("rollback");

    assert_eq!(
        read_origin_fetch_refspec(tmp.path()).as_deref(),
        Some("+refs/heads/develop:refs/remotes/origin/develop"),
        "rollback must restore the pre-normalize single-branch refspec to .git/config"
    );
}

#[test]
fn t155_rollback_without_recorded_refspec_leaves_restored_config_untouched() {
    // When no pre-normalize refspec was recorded (idempotent / wildcard
    // origin), rollback must not invent or strip a refspec beyond what the
    // file-tree restore brings back.
    let tmp = tempfile::tempdir().unwrap();
    gwt_core::process::hidden_command("git")
        .args(["init", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();
    gwt_core::process::hidden_command("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    gwt_core::process::hidden_command("git")
        .args([
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let mut snapshot = backup::create(tmp.path()).expect("backup::create");
    snapshot.pre_normalize_fetch_refspec = None;

    rollback::rollback_migration(&snapshot).expect("rollback");

    assert_eq!(
        read_origin_fetch_refspec(tmp.path()).as_deref(),
        Some("+refs/heads/*:refs/remotes/origin/*"),
        "rollback with no recorded refspec must leave the restored config as-is"
    );
}

#[test]
fn t040_executor_stub_returns_untouched_confirm_error() {
    let tmp = tempfile::tempdir().unwrap();

    let err = executor::execute_migration(tmp.path(), MigrationOptions::default(), |_, _| {
        panic!("stub executor must not emit progress")
    })
    .expect_err("stub executor must return the explicit not-implemented error");

    assert_eq!(err.phase, MigrationPhase::Confirm);
    assert_eq!(err.recovery, RecoveryState::Untouched);
    assert!(err.message.contains("execute_migration is not implemented"));
    assert!(err
        .to_string()
        .contains("migration failed at phase confirm"));
}

#[test]
fn t040_migration_error_display_includes_phase_recovery_and_message() {
    let err = MigrationError {
        phase: MigrationPhase::Backup,
        message: "disk failed".to_string(),
        recovery: RecoveryState::Partial,
    };

    assert_eq!(
        err.to_string(),
        "migration failed at phase backup (recovery: Partial): disk failed"
    );
}

#[test]
fn t020_validation_error_display_and_io_conversion_are_stable() {
    let disk = ValidationError::InsufficientDiskSpace {
        required: 20,
        available: 10,
    };
    assert_eq!(
        disk.to_string(),
        "insufficient disk space: required 20 bytes, available 10 bytes"
    );

    let locked = ValidationError::LockedWorktrees(vec!["/tmp/wt".into()]);
    assert!(locked.to_string().contains("locked worktrees:"));

    let denied = ValidationError::WritePermissionDenied("/tmp/project".into());
    assert_eq!(denied.to_string(), "write permission denied: /tmp/project");

    let io_error: ValidationError = std::io::Error::other("boom").into();
    assert_eq!(io_error.to_string(), "validator io error: boom");
}

#[test]
fn t015_migration_phase_has_stable_string_keys() {
    // FR-019/FR-029 expose phases over the WebSocket API. Their string
    // representation must stay stable so the WebView can dispatch on it.
    let cases = [
        (MigrationPhase::Confirm, "confirm"),
        (MigrationPhase::Validate, "validate"),
        (MigrationPhase::Backup, "backup"),
        (MigrationPhase::Bareify, "bareify"),
        (MigrationPhase::Worktrees, "worktrees"),
        (MigrationPhase::Submodules, "submodules"),
        (MigrationPhase::Tracking, "tracking"),
        (MigrationPhase::Cleanup, "cleanup"),
        (MigrationPhase::Done, "done"),
        (MigrationPhase::Error, "error"),
        (MigrationPhase::RolledBack, "rolled_back"),
    ];
    for (phase, expected) in cases {
        assert_eq!(phase.as_str(), expected, "as_str for {phase:?}");
        assert_eq!(format!("{phase}"), expected, "Display for {phase:?}");
    }
}

// ---------------------------------------------------------------------------
// Issue #3466 / SPEC-3431 T-172 (FR-078, AS-PM-LIFECYCLE-MIGRATE): stores that
// a pre-#3466 build split apart are folded back in by quarantining them and
// rebuilding the canonical projection from durable events. The migration never
// writes to the store it is absorbing, and every refusal is fail-closed.
// ---------------------------------------------------------------------------

mod store_consolidation {
    use std::path::{Path, PathBuf};

    use gwt_core::paths::{
        gwt_project_dir, gwt_project_state_work_events_path, gwt_project_state_works_path,
    };
    use gwt_core::repo_hash::{compute_path_hash, detect_repo_identity};
    use gwt_core::workspace_projection::store_migration::{
        apply_store_consolidation, issue_store_consolidation_plan, plan_store_consolidation,
        QuarantineManifest, StoreConsolidationOutcome, StoreConsolidationPlan,
        StoreConsolidationRefusal, QUARANTINE_MANIFEST_VERSION,
    };
    use gwt_core::workspace_projection::{
        save_workspace_work_items_projection_to_path, WorkEvent, WorkEventKind, WorkItem,
        WorkItemsProjection, WorkspaceStatusCategory,
    };

    /// The Session that reviews and applies every plan in these tests.
    const TEST_SESSION: Option<&str> = Some("session-store-consolidation");

    /// Review a dry run the way an operator does, and return the manifest hash
    /// that authorizes applying it.
    fn review(plan: &StoreConsolidationPlan) -> String {
        issue_store_consolidation_plan(plan, TEST_SESSION).expect("issue reviewed plan");
        plan.manifest_hash.clone()
    }

    /// The lock a live Work writer takes for one project store's projection.
    fn works_lock_path(store_dir: &Path) -> PathBuf {
        store_dir.join("project-state").join("works.lock")
    }

    fn hold_exclusive(path: &Path) -> std::fs::File {
        std::fs::create_dir_all(path.parent().expect("lock parent")).expect("lock parent");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .expect("open lock");
        fs2::FileExt::lock_exclusive(&file).expect("hold lock");
        file
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let mut cmd = gwt_core::process::hidden_command("git");
        cmd.args(args).current_dir(cwd);
        gwt_core::process::scrub_git_env(&mut cmd);
        let output = cmd.output().expect("git command");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A Nested Bare + Worktree layout plus the orphaned, path-keyed store that
    /// a pre-#3466 build left behind for its layout root.
    struct SplitStoreFixture {
        _temp: tempfile::TempDir,
        layout_root: PathBuf,
        worktree: PathBuf,
        canonical_works: PathBuf,
        orphan_store: PathBuf,
    }

    impl SplitStoreFixture {
        fn new(origin: &str) -> Self {
            let temp = tempfile::tempdir().expect("tempdir");
            let layout_root = temp.path().join("workbench");
            let bare = layout_root.join("gwt.git");
            let bootstrap = layout_root.join(".bootstrap");
            let worktree = layout_root.join("work").join("develop");
            std::fs::create_dir_all(worktree.parent().expect("work dir")).expect("work dir");

            run_git(
                temp.path(),
                &["init", "--bare", bare.to_str().expect("bare")],
            );
            run_git(&bare, &["remote", "add", "origin", origin]);
            run_git(
                &layout_root,
                &["clone", bare.to_str().expect("bare"), ".bootstrap"],
            );
            run_git(&bootstrap, &["checkout", "-b", "develop"]);
            run_git(
                &bootstrap,
                &[
                    "-c",
                    "user.name=gwt-test",
                    "-c",
                    "user.email=gwt-test@example.com",
                    "commit",
                    "--allow-empty",
                    "-m",
                    "init",
                ],
            );
            run_git(&bootstrap, &["push", "origin", "develop"]);
            std::fs::remove_dir_all(&bootstrap).expect("remove bootstrap");
            run_git(
                &bare,
                &[
                    "worktree",
                    "add",
                    worktree.to_str().expect("worktree"),
                    "develop",
                ],
            );

            let canonical_hash = detect_repo_identity(&layout_root)
                .expect("canonical identity")
                .hash;
            let canonical_works = gwt_project_state_works_path(&canonical_hash);

            // The pre-#3466 store: keyed by the layout root's *path*.
            let orphan_hash = compute_path_hash(&layout_root);
            assert_ne!(orphan_hash.as_str(), canonical_hash.as_str());
            let orphan_store = gwt_project_dir(&orphan_hash);
            let orphan_event = start_event("work-orphaned", "Work only the split store knows");
            let mut orphan_projection = WorkItemsProjection::empty(orphan_event.updated_at);
            orphan_projection.apply_event(orphan_event.clone());
            save_workspace_work_items_projection_to_path(
                &gwt_project_state_works_path(&orphan_hash),
                &orphan_projection,
            )
            .expect("seed orphaned WorkItems");
            write_events(
                &gwt_project_state_work_events_path(&orphan_hash),
                &[orphan_event],
            );

            Self {
                _temp: temp,
                layout_root,
                worktree,
                canonical_works,
                orphan_store,
            }
        }

        fn seed_canonical_event(&self, work_id: &str, title: &str) {
            let canonical_hash = detect_repo_identity(&self.layout_root)
                .expect("canonical identity")
                .hash;
            write_events(
                &gwt_project_state_work_events_path(&canonical_hash),
                &[start_event(work_id, title)],
            );
        }

        /// A Work row that exists only in the store's projection, with no line
        /// in any durable event log. The real split store held 796 such rows
        /// against 240 events, so this is the common case, not an edge case.
        fn seed_orphan_eventless_work(&self, work_id: &str, title: &str) {
            let orphan_hash = compute_path_hash(&self.layout_root);
            let works_path = gwt_project_state_works_path(&orphan_hash);
            let mut projection =
                gwt_core::workspace_projection::load_workspace_work_items_from_path(&works_path)
                    .expect("load orphan WorkItems")
                    .expect("orphan WorkItems present");
            let at = chrono::Utc::now();
            projection.work_items.push(WorkItem {
                id: work_id.to_string(),
                title: title.to_string(),
                intent: None,
                summary: None,
                progress_summary: None,
                status_category: WorkspaceStatusCategory::Idle,
                owner: None,
                created_at: at,
                updated_at: at,
                completed_at: None,
                agents: Vec::new(),
                execution_containers: Vec::new(),
                board_refs: Vec::new(),
                related_work_item_ids: Vec::new(),
                events: Vec::new(),
                legacy_metadata_snapshot: None,
                legacy_metadata_authoritative: false,
                legacy_metadata_snapshot_at: None,
                duplicate_event_containers: Default::default(),
                discarded: false,
                discarded_at: None,
            });
            save_workspace_work_items_projection_to_path(&works_path, &projection)
                .expect("seed eventless orphan Work");
        }

        fn quarantine_manifest(&self) -> PathBuf {
            let orphan_hash = compute_path_hash(&self.layout_root);
            self.canonical_store()
                .join("quarantine")
                .join(format!("{orphan_hash}.json"))
        }

        fn canonical_store(&self) -> PathBuf {
            gwt_project_dir(
                &detect_repo_identity(&self.layout_root)
                    .expect("canonical identity")
                    .hash,
            )
        }

        fn canonical_work_ids(&self) -> Vec<String> {
            let Some(projection) =
                gwt_core::workspace_projection::load_workspace_work_items_from_path(
                    &self.canonical_works,
                )
                .expect("load canonical WorkItems")
            else {
                return Vec::new();
            };
            let mut ids: Vec<String> = projection
                .work_items
                .iter()
                .map(|item| item.id.clone())
                .collect();
            ids.sort();
            ids
        }
    }

    fn start_event(work_id: &str, title: &str) -> WorkEvent {
        let mut event = WorkEvent::new(WorkEventKind::Start, work_id, chrono::Utc::now());
        event.id = format!("event-{work_id}");
        event.title = Some(title.to_string());
        event
    }

    fn write_events(path: &Path, events: &[WorkEvent]) {
        std::fs::create_dir_all(path.parent().expect("event log parent")).expect("event parent");
        let mut content = String::new();
        for event in events {
            content.push_str(&serde_json::to_string(event).expect("serialize event"));
            content.push('\n');
        }
        std::fs::write(path, content).expect("write event log");
    }

    /// AC-8 dry run: planning names the orphan and writes nothing.
    #[test]
    fn plan_is_a_dry_run_that_names_the_orphaned_store() {
        let fixture = SplitStoreFixture::new("https://example.invalid/acme/consolidate-plan.git");
        let before = fixture.canonical_store().exists();

        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");

        assert_eq!(plan.orphans.len(), 1, "the split store must be reported");
        let orphan = &plan.orphans[0];
        assert_eq!(orphan.source_root, fixture.layout_root);
        assert_eq!(orphan.store_dir, fixture.orphan_store);
        assert_eq!(orphan.work_item_count, 1);
        assert_eq!(orphan.work_event_count, 1);
        assert!(
            !orphan.revision.is_empty(),
            "the manifest must pin a revision"
        );
        assert!(!plan.manifest_hash.is_empty());

        assert!(
            fixture.orphan_store.is_dir(),
            "a dry run must leave the source store in place"
        );
        assert_eq!(
            fixture.canonical_store().exists(),
            before,
            "a dry run must not materialize the canonical store"
        );
    }

    /// AC-7: the source is quarantined behind a read-only manifest, never
    /// patched in place, and the canonical projection is rebuilt from events.
    #[test]
    fn apply_quarantines_the_source_and_rebuilds_from_durable_events() {
        let fixture = SplitStoreFixture::new("https://example.invalid/acme/consolidate-apply.git");
        fixture.seed_canonical_event("work-canonical", "Work the canonical store already had");
        let source_bytes =
            std::fs::read(fixture.orphan_store.join("project-state/works.json")).expect("source");
        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");

        let outcome = apply_store_consolidation(&fixture.layout_root, &review(&plan), TEST_SESSION)
            .expect("consolidate");

        let StoreConsolidationOutcome::Consolidated {
            quarantined,
            work_item_count,
        } = outcome
        else {
            panic!("expected a consolidation, got {outcome:?}");
        };
        assert_eq!(quarantined.len(), 1);
        assert!(
            !fixture.orphan_store.exists(),
            "the split store must move out of the project store namespace"
        );
        assert_eq!(
            std::fs::read(quarantined[0].join("project-state/works.json")).expect("quarantined"),
            source_bytes,
            "quarantine must preserve the source bytes verbatim"
        );

        let manifest_path = fixture.quarantine_manifest();
        let manifest: QuarantineManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("manifest bytes"))
                .expect("manifest json");
        assert_eq!(manifest.version, QUARANTINE_MANIFEST_VERSION);
        assert_eq!(manifest.source.source_root, fixture.layout_root);
        assert_eq!(manifest.source.work_item_count, 1);
        assert!(
            std::fs::metadata(&manifest_path)
                .expect("manifest metadata")
                .permissions()
                .readonly(),
            "the quarantine manifest must be read-only"
        );

        assert_eq!(
            fixture.canonical_work_ids(),
            vec!["work-canonical".to_string(), "work-orphaned".to_string()],
            "the rebuild must carry both stores' durable Work forward"
        );
        assert_eq!(work_item_count, 2);
    }

    /// AC-8 idempotence: re-applying an approved plan changes nothing.
    #[test]
    fn apply_is_idempotent() {
        let fixture =
            SplitStoreFixture::new("https://example.invalid/acme/consolidate-idempotent.git");
        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");
        apply_store_consolidation(&fixture.layout_root, &review(&plan), TEST_SESSION)
            .expect("first apply");
        let after_first = std::fs::read(&fixture.canonical_works).expect("canonical works");

        let replan = plan_store_consolidation(&fixture.layout_root).expect("re-plan");
        assert!(
            replan.is_empty(),
            "the quarantined store must not be replanned"
        );
        let outcome =
            apply_store_consolidation(&fixture.layout_root, &review(&replan), TEST_SESSION)
                .expect("re-apply");

        assert_eq!(outcome, StoreConsolidationOutcome::NothingToDo);
        assert_eq!(
            std::fs::read(&fixture.canonical_works).expect("canonical works"),
            after_first,
            "a second apply must not rewrite the canonical projection"
        );
    }

    /// AC-8 history conflict: an approved plan that no longer matches the disk
    /// is refused before anything moves.
    #[test]
    fn apply_refuses_a_stale_manifest_hash_without_mutation() {
        let fixture = SplitStoreFixture::new("https://example.invalid/acme/consolidate-stale.git");
        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");
        let before = std::fs::read(fixture.orphan_store.join("project-state/works.json"))
            .expect("source before");

        let error = apply_store_consolidation(&fixture.layout_root, "0000deadbeef", TEST_SESSION)
            .expect_err("a stale manifest must fail closed");

        assert_eq!(error.refusal, StoreConsolidationRefusal::ManifestChanged);
        assert!(plan.manifest_hash != "0000deadbeef");
        assert_eq!(
            std::fs::read(fixture.orphan_store.join("project-state/works.json"))
                .expect("source after"),
            before,
            "a refusal must be zero-mutation"
        );
        assert!(!fixture.quarantine_manifest().exists());
    }

    /// Issue #3524 (folded into #3606): the migration used to contend on a
    /// private `store-consolidation.lock` that no Work writer knows about, so a
    /// live writer holding the real `works.lock` could be mid-write while its
    /// entire store was renamed away underneath it. The migration must take the
    /// same lock the Work persistence takes — on the store it is absorbing, not
    /// only on the one it is writing.
    #[test]
    fn apply_refuses_while_a_work_writer_holds_the_orphan_store_lock() {
        let fixture =
            SplitStoreFixture::new("https://example.invalid/acme/consolidate-orphan-lock.git");
        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");
        let held = hold_exclusive(&works_lock_path(&fixture.orphan_store));

        let error = apply_store_consolidation(&fixture.layout_root, &review(&plan), TEST_SESSION)
            .expect_err("a live Work writer in the split store must fail the migration closed");

        assert_eq!(error.refusal, StoreConsolidationRefusal::WriterBusy);
        assert!(
            fixture.orphan_store.is_dir(),
            "a refusal must be zero-mutation"
        );
        fs2::FileExt::unlock(&held).expect("release lock");
    }

    /// Issue #3524 (folded into #3606): contention on the canonical store must
    /// be a bounded refusal too. Blocking on `works.lock` would hang the
    /// migration behind whatever writer happens to be running, with the source
    /// store already quarantined.
    #[test]
    fn apply_refuses_while_a_work_writer_holds_the_canonical_store_lock() {
        let fixture =
            SplitStoreFixture::new("https://example.invalid/acme/consolidate-canonical-lock.git");
        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");
        let canonical_store = fixture.canonical_store();
        let held = hold_exclusive(&works_lock_path(&canonical_store));

        let layout_root = fixture.layout_root.clone();
        let manifest_hash = review(&plan);
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(
                apply_store_consolidation(&layout_root, &manifest_hash, TEST_SESSION).is_err(),
            );
        });
        let refused = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the migration must refuse instead of blocking on a live writer");

        assert!(refused, "a held canonical lock must fail the migration");
        assert!(
            fixture.orphan_store.is_dir(),
            "a refusal must be zero-mutation"
        );
        fs2::FileExt::unlock(&held).expect("release lock");
    }

    /// Issue #3524 (folded into #3606): the review gate used to be a bare hash
    /// comparison, so a caller that never read the plan could still apply it.
    /// A matching hash is not evidence of review — an issued dry run is.
    #[test]
    fn apply_refuses_a_manifest_hash_that_no_dry_run_issued() {
        let fixture =
            SplitStoreFixture::new("https://example.invalid/acme/consolidate-unreviewed.git");
        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");

        let error = apply_store_consolidation(&fixture.layout_root, &plan.manifest_hash, None)
            .expect_err("an unreviewed plan must fail closed");

        assert_eq!(error.refusal, StoreConsolidationRefusal::PlanNotIssued);
        assert!(error.refusal.retryable(), "reviewing the plan clears this");
        assert!(
            fixture.orphan_store.is_dir(),
            "a refusal must be zero-mutation"
        );
    }

    /// Issue #3524 (folded into #3606): the refusal text used to quote the
    /// current manifest hash, which turned "apply without review" into a
    /// two-call loop. No refusal may hand out a hash that authorizes an apply.
    #[test]
    fn a_stale_manifest_refusal_never_leaks_the_current_hash() {
        let fixture =
            SplitStoreFixture::new("https://example.invalid/acme/consolidate-no-leak.git");
        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");

        let error = apply_store_consolidation(&fixture.layout_root, "0000deadbeef", TEST_SESSION)
            .expect_err("a stale manifest must fail closed");

        assert_eq!(error.refusal, StoreConsolidationRefusal::ManifestChanged);
        assert!(
            !error.detail.contains(&plan.manifest_hash),
            "the refusal must not disclose the hash that would authorize an apply: {}",
            error.detail
        );
    }

    /// Issue #3524 (folded into #3606): a plan reviewed in one Session does not
    /// authorize an apply from another.
    #[test]
    fn apply_refuses_a_plan_reviewed_by_a_different_session() {
        let fixture =
            SplitStoreFixture::new("https://example.invalid/acme/consolidate-other-session.git");
        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");
        let manifest_hash = review(&plan);

        let error = apply_store_consolidation(
            &fixture.layout_root,
            &manifest_hash,
            Some("session-somebody-else"),
        )
        .expect_err("another Session's review must not authorize this apply");

        assert_eq!(error.refusal, StoreConsolidationRefusal::PlanNotIssued);
        assert!(
            fixture.orphan_store.is_dir(),
            "a refusal must be zero-mutation"
        );
    }

    /// Issue #3524 (folded into #3606): the real split store held 796 Work rows
    /// against 240 events, so rebuilding the canonical projection from the
    /// durable event logs alone drops every legacy row recorded before event
    /// logging existed. Consolidation must not lose a single Work.
    #[test]
    fn apply_keeps_orphan_work_that_has_no_durable_event() {
        let fixture =
            SplitStoreFixture::new("https://example.invalid/acme/consolidate-eventless.git");
        fixture.seed_canonical_event("work-canonical", "Work the canonical store already had");
        fixture.seed_orphan_eventless_work("work-eventless", "Legacy Work with no event");
        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");

        apply_store_consolidation(&fixture.layout_root, &review(&plan), TEST_SESSION)
            .expect("consolidate");

        assert_eq!(
            fixture.canonical_work_ids(),
            vec![
                "work-canonical".to_string(),
                "work-eventless".to_string(),
                "work-orphaned".to_string(),
            ],
            "consolidation must carry a Work with no durable event forward"
        );
    }

    /// AC-8 corrupt input: an unparseable source store stops the plan.
    #[test]
    fn plan_refuses_a_corrupt_source_store() {
        let fixture =
            SplitStoreFixture::new("https://example.invalid/acme/consolidate-corrupt.git");
        std::fs::write(
            fixture.orphan_store.join("project-state/works.json"),
            b"{ not json",
        )
        .expect("corrupt the source");

        let error =
            plan_store_consolidation(&fixture.layout_root).expect_err("corrupt input must refuse");

        assert_eq!(error.refusal, StoreConsolidationRefusal::CorruptInput);
    }

    /// AC-8 unknown remote: without a repository identity there is no
    /// canonical store to consolidate into.
    #[test]
    fn plan_refuses_an_unknown_remote() {
        let temp = tempfile::tempdir().expect("tempdir");
        let plain = temp.path().join("not-a-repository");
        std::fs::create_dir_all(&plain).expect("plain dir");

        let error = plan_store_consolidation(&plain).expect_err("unknown remote must refuse");

        assert_eq!(error.refusal, StoreConsolidationRefusal::UnknownRemote);
    }

    /// The GUI opens the layout root, but agents run `gwtd` from a linked
    /// worktree — planning from either must find the layout root's split store.
    /// A linked worktree reaches its git dir through a relative `commondir`, so
    /// this is where a naive parent lookup silently misses the orphan.
    #[test]
    fn plan_from_a_linked_worktree_finds_the_layout_root_store() {
        let fixture =
            SplitStoreFixture::new("https://example.invalid/acme/consolidate-from-worktree.git");

        let plan = plan_store_consolidation(&fixture.worktree).expect("plan from the worktree");

        assert!(
            plan.orphans
                .iter()
                .any(|orphan| orphan.store_dir == fixture.orphan_store),
            "the layout root's split store must be found from a worktree too: {:?}",
            plan.orphans
        );
        assert_eq!(
            plan.canonical_hash.as_str(),
            plan_store_consolidation(&fixture.layout_root)
                .expect("plan from the layout root")
                .canonical_hash
                .as_str(),
            "both entry points must target one canonical store"
        );
    }

    /// AC-9 negative: consolidation only ever reaches the repository's own
    /// roots, never an unrelated directory that happens to have a store.
    #[test]
    fn plan_ignores_stores_belonging_to_unrelated_directories() {
        let fixture =
            SplitStoreFixture::new("https://example.invalid/acme/consolidate-unrelated.git");
        let unrelated = fixture
            .layout_root
            .parent()
            .expect("temp root")
            .join("elsewhere");
        std::fs::create_dir_all(&unrelated).expect("unrelated dir");
        let unrelated_store = gwt_project_dir(&compute_path_hash(&unrelated));
        std::fs::create_dir_all(unrelated_store.join("project-state")).expect("unrelated store");

        let plan = plan_store_consolidation(&fixture.layout_root).expect("plan");

        assert!(
            plan.orphans
                .iter()
                .all(|orphan| orphan.store_dir != unrelated_store),
            "an unrelated directory's store must never be absorbed"
        );
        assert!(unrelated_store.is_dir(), "and it must be left alone");
    }
}
