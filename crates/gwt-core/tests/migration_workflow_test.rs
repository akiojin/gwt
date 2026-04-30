//! Workflow-level tests for the SPEC-1934 US-6 migration support.

use gwt_core::config::BareProjectConfig;
use gwt_core::migration::validator::{
    self, check_disk_space, check_locked_worktrees, check_write_permission, evaluate_disk_space,
    ValidationError,
};
use gwt_core::migration::MigrationPhase;

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
