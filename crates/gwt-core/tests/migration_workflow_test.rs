//! Workflow-level tests for the SPEC-1934 US-6 migration support.

use gwt_core::config::BareProjectConfig;
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
