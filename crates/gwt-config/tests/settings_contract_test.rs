//! Public API contract tests for `gwt_config::Settings`.
//!
//! Other crates (gwt, gwt-agent) load and persist settings exclusively
//! through this surface, so these tests pin the externally observable
//! behavior: save/load roundtrip, tolerant parsing for config evolution,
//! and the global config path layout. The real `~/.gwt/config.toml` is
//! never touched.

use std::num::NonZeroU16;
#[cfg(unix)]
use std::path::Path;

use gwt_config::settings::WorkflowPolicyTarget;
use gwt_config::{ConfigError, Settings};

#[test]
fn save_then_load_from_path_roundtrips_mutated_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");

    let mut settings = Settings {
        default_base_branch: "develop".to_string(),
        debug: true,
        ..Settings::default()
    };
    settings.protected_branches.push("release".to_string());

    settings.save(&path).expect("save settings");
    let loaded = Settings::load_from_path(&path).expect("load saved settings");

    assert_eq!(loaded.default_base_branch, "develop");
    assert!(
        loaded.protected_branches.contains(&"release".to_string()),
        "protected_branches must survive a save/load roundtrip"
    );
    assert!(loaded.debug);
}

#[test]
fn load_from_path_fills_missing_sections_with_defaults() {
    // Config evolution contract: a file written by an older gwt version
    // (missing newer sections) must still load, with defaults filled in.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "debug = true\n").expect("write minimal config");

    let loaded = Settings::load_from_path(&path).expect("minimal config must load");

    assert!(loaded.debug);
    assert_eq!(loaded.default_base_branch, "main");
    assert!(loaded.protected_branches.contains(&"main".to_string()));
    let serialized = toml::Value::try_from(&loaded).expect("serialize loaded settings");
    assert!(
        serialized
            .get("allowed_workflows")
            .and_then(toml::Value::as_table)
            .is_some_and(|workflows| workflows.is_empty()),
        "older settings must default to an empty workflow allowlist"
    );
}

#[test]
fn load_and_save_preserves_repo_scoped_allowed_workflows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("source.toml");
    let rewritten = dir.path().join("rewritten.toml");
    let content = "[allowed_workflows]\n\"akiojin/gwt\" = [\"prepare-release.yml\"]\n";
    std::fs::write(&source, content).expect("write workflow allowlist");

    let loaded = Settings::load_from_path(&source).expect("workflow allowlist must load");
    loaded.save(&rewritten).expect("save workflow allowlist");
    let restored = Settings::load_from_path(&rewritten).expect("reload workflow allowlist");
    let actual = toml::Value::try_from(&restored).expect("serialize restored settings");
    let expected: toml::Value = toml::from_str(content).expect("parse expected allowlist");
    assert_eq!(
        actual.get("allowed_workflows"),
        expected.get("allowed_workflows"),
        "explicit repository permissions must survive a load/save roundtrip"
    );
}

#[test]
fn load_from_path_ignores_unknown_future_keys() {
    // Forward compatibility contract: a config written by a newer gwt
    // version (containing keys this binary does not know) must not fail.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "default_base_branch = \"develop\"\nfuture_unknown_key = true\n",
    )
    .expect("write forward-compat config");

    let loaded = Settings::load_from_path(&path).expect("unknown keys must be ignored");
    assert_eq!(loaded.default_base_branch, "develop");
}

#[test]
fn load_and_save_preserves_nonzero_embedded_server_port() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("source.toml");
    let rewritten = dir.path().join("rewritten.toml");
    std::fs::write(
        &source,
        "default_base_branch = \"develop\"\ndebug = true\n\n[server]\nembedded_port = 43210\n",
    )
    .expect("write server config");

    let loaded = Settings::load_from_path(&source).expect("server config must load");
    loaded.save(&rewritten).expect("rewrite server config");
    let content = std::fs::read_to_string(&rewritten).expect("read rewritten config");

    assert!(
        content.contains("[server]") && content.contains("embedded_port = 43210"),
        "a recognized non-zero embedded port must survive load/save: {content}"
    );
    assert!(
        content.contains("default_base_branch = \"develop\"") && content.contains("debug = true"),
        "adding server settings must preserve unrelated fields: {content}"
    );
}

#[test]
fn load_and_save_normalizes_zero_embedded_server_port_to_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("source.toml");
    let rewritten = dir.path().join("rewritten.toml");
    std::fs::write(
        &source,
        "default_base_branch = \"develop\"\ndebug = true\n\n[server]\nembedded_port = 0\n",
    )
    .expect("write zero server port");

    let loaded = Settings::load_from_path(&source).expect("zero port config must load");
    loaded.save(&rewritten).expect("rewrite normalized config");
    let content = std::fs::read_to_string(&rewritten).expect("read rewritten config");

    assert!(
        content.contains("[server]"),
        "the server settings schema must be recognized: {content}"
    );
    assert!(
        !content.contains("embedded_port = 0"),
        "zero is invalid persisted state and must normalize to absent: {content}"
    );
    assert!(
        content.contains("default_base_branch = \"develop\"") && content.contains("debug = true"),
        "normalizing the server port must preserve unrelated fields: {content}"
    );
}

#[test]
fn persist_embedded_port_preserves_unknown_keys_comments_and_minimal_shape() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "# operator comment\ndebug = true\nfuture_unknown_key = { mode = \"new\" }\n\n[server]\n# keep this server comment\nfuture_server_key = \"keep\"\nembedded_port = 41000 # keep this port comment\n",
    )
    .expect("write forward-compatible config");

    Settings::persist_embedded_port(
        &path,
        NonZeroU16::new(42000).expect("non-zero fixture port"),
    )
    .expect("persist embedded port");

    let content = std::fs::read_to_string(&path).expect("read updated config");
    assert!(content.contains("# operator comment"));
    assert!(content.contains("future_unknown_key = { mode = \"new\" }"));
    assert!(content.contains("# keep this server comment"));
    assert!(content.contains("future_server_key = \"keep\""));
    assert!(content.contains("embedded_port = 42000 # keep this port comment"));
    assert!(
        Settings::load_from_path(&path)
            .expect("updated config loads")
            .debug
    );

    let minimal_path = dir.path().join("minimal.toml");
    Settings::persist_embedded_port(
        &minimal_path,
        NonZeroU16::new(43000).expect("non-zero fixture port"),
    )
    .expect("persist first embedded port");
    let minimal = std::fs::read_to_string(&minimal_path).expect("read minimal config");
    assert!(
        minimal.contains("[server]") && minimal.contains("embedded_port = 43000"),
        "automatic port persistence must use the documented server table: {minimal}"
    );
    assert!(
        !minimal.contains("default_base_branch") && !minimal.contains("protected_branches"),
        "automatic port persistence must not materialize unrelated defaults: {minimal}"
    );
}

#[cfg(unix)]
#[test]
fn persist_embedded_port_updates_symlink_target_without_replacing_link() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("tempdir");
    let managed_dir = dir.path().join("managed");
    std::fs::create_dir_all(&managed_dir).expect("create managed config directory");
    let target = managed_dir.join("config.toml");
    std::fs::write(
        &target,
        "# managed target\ndebug = true\nfuture_unknown_key = \"keep\"\n",
    )
    .expect("write managed config target");

    let path = dir.path().join("config.toml");
    let relative_target = Path::new("managed").join("config.toml");
    symlink(&relative_target, &path).expect("create relative config symlink");

    Settings::persist_embedded_port(
        &path,
        NonZeroU16::new(44000).expect("non-zero fixture port"),
    )
    .expect("persist embedded port through symlink");

    assert!(
        std::fs::symlink_metadata(&path)
            .expect("read config link metadata")
            .file_type()
            .is_symlink(),
        "automatic port persistence must not replace a user-managed config symlink"
    );
    assert_eq!(
        std::fs::read_link(&path).expect("read config symlink"),
        relative_target,
        "automatic port persistence must keep the original relative link target"
    );

    let target_content = std::fs::read_to_string(&target).expect("read managed config target");
    assert!(target_content.contains("# managed target"));
    assert!(target_content.contains("future_unknown_key = \"keep\""));
    assert!(target_content.contains("embedded_port = 44000"));
    assert_eq!(
        std::fs::read_to_string(&path).expect("read config through symlink"),
        target_content
    );
}

#[test]
fn load_from_path_reports_parse_error_for_invalid_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "this is not [valid toml").expect("write broken config");

    let error = Settings::load_from_path(&path).expect_err("invalid toml must fail");
    assert!(
        matches!(error, ConfigError::ParseError { .. }),
        "expected ParseError, got: {error:?}"
    );
}

#[test]
fn load_from_path_reports_parse_error_for_missing_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("does-not-exist.toml");

    let error = Settings::load_from_path(&path).expect_err("missing file must fail");
    assert!(
        matches!(error, ConfigError::ParseError { .. }),
        "expected ParseError, got: {error:?}"
    );
}

#[test]
fn global_config_path_for_home_is_dot_gwt_config_toml() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path();
    assert_eq!(
        Settings::global_config_path_for_home(home),
        home.join(".gwt").join("config.toml")
    );
}

#[test]
fn settings_writers_reject_an_unusable_shared_lock_without_changing_config() {
    for operation in ["save", "update", "port", "auto_update"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "debug = false\n").unwrap();
        std::fs::create_dir(path.with_extension("settings.lock")).unwrap();
        let result = match operation {
            "save" => Settings::default().save(&path),
            "update" => Settings::update_at_path(&path, |settings| {
                settings.debug = true;
                Ok(())
            }),
            "port" => Settings::persist_embedded_port(&path, NonZeroU16::new(44000).unwrap()),
            _ => Settings::persist_auto_apply_updates(&path, |_| Ok(true)).map(|_| ()),
        };
        assert!(
            result.is_err(),
            "{operation} bypassed the unusable shared lock"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "debug = false\n");
    }
}

#[test]
fn release_policy_merge_preserves_other_settings_and_allows_a_checked_rollback() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let original = "# operator comment\ndebug = true\nfuture_unknown_key = { mode = \"new\" }\n\n[allowed_workflows]\n# keep this policy comment\n\"akiojin/gwt\" = [\"build.yml\"] # existing workflow\n\"other/repo\" = [\"other.yml\"]\n";
    std::fs::write(&path, original).unwrap();
    let before = Settings::release_policy_snapshot(&path, "akiojin/gwt").unwrap();
    assert!(!before.allowed);
    assert_eq!(before.content, original);
    assert!(
        !path.with_extension("settings.lock").exists(),
        "snapshot stays read-only"
    );

    let mut lock_probe = None;
    let (receipt, after) =
        Settings::with_locked_release_policy(&path, "akiojin/gwt", true, |actual, proposed| {
            let probe = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path.with_extension("settings.lock"))
                .unwrap();
            let contention = fs2::FileExt::try_lock_exclusive(&probe)
                .expect_err("the actual Settings file lock must cover the precommit callback");
            assert_eq!(
                contention.raw_os_error(),
                fs2::lock_contended_error().raw_os_error()
            );
            lock_probe = Some(probe);
            assert_eq!(actual.content, before.content);
            assert!(!actual.allowed && proposed.allowed);
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                original,
                "callback precedes the effect"
            );
            Ok(17)
        })
        .unwrap();
    let probe = lock_probe.expect("precommit callback ran");
    fs2::FileExt::try_lock_exclusive(&probe)
        .expect("return releases the actual Settings file lock");
    fs2::FileExt::unlock(&probe).unwrap();
    assert_eq!(receipt, 17);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), after.content);
    assert!(after.content.contains("# operator comment"));
    assert!(after
        .content
        .contains("future_unknown_key = { mode = \"new\" }"));
    assert!(after.content.contains("# keep this policy comment"));
    assert!(after.content.contains("# existing workflow"));
    let settings = Settings::load_from_path(&path).unwrap();
    assert_eq!(
        settings.allowed_workflows["akiojin/gwt"],
        ["build.yml", "prepare-release.yml"]
    );
    assert_eq!(settings.allowed_workflows["other/repo"], ["other.yml"]);

    let (called, unchanged) =
        Settings::with_locked_release_policy(&path, "akiojin/gwt", true, |actual, proposed| {
            assert_eq!(actual.content, proposed.content);
            Ok(true)
        })
        .unwrap();
    assert!(
        called,
        "an unchanged add still requires the caller's authority check"
    );
    assert_eq!(after.content, unchanged.content);
    let (_, rolled_back) =
        Settings::with_locked_release_policy(&path, "akiojin/gwt", false, |actual, proposed| {
            assert_eq!(
                actual.content, after.content,
                "rollback CAS checks the current post-state"
            );
            assert!(!proposed.allowed);
            Ok(())
        })
        .unwrap();
    assert!(!rolled_back.allowed);
    let settings = Settings::load_from_path(&path).unwrap();
    assert_eq!(settings.allowed_workflows["akiojin/gwt"], ["build.yml"]);
    assert_eq!(settings.allowed_workflows["other/repo"], ["other.yml"]);
}

#[test]
fn release_policy_missing_snapshot_and_rejected_precommit_do_not_write_config() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing/config.toml");
    let before = Settings::release_policy_snapshot(&path, "akiojin/gwt").unwrap();
    assert!(before.content.is_empty() && !before.allowed);
    assert!(
        !path.parent().unwrap().exists(),
        "read must not create the config directory"
    );
    let result = Settings::with_locked_release_policy(
        &path,
        "akiojin/gwt",
        true,
        |actual, proposed| -> Result<(), ConfigError> {
            assert!(actual.content.is_empty() && proposed.allowed);
            Err(ConfigError::ValidationError {
                reason: "stale CAS token".into(),
            })
        },
    );
    assert!(result.is_err());
    assert!(
        !path.exists(),
        "rejected CAS/authority must have zero configuration effects"
    );
    for repository in [
        "",
        "foreign/repo/extra",
        "https://github.com/other/repo",
        "../repo",
    ] {
        assert!(Settings::release_policy_snapshot(&path, repository).is_err());
    }
}

#[test]
fn diagnostic_workflow_policy_add_replay_and_remove_preserve_release_and_other_settings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let original = "# operator comment\ndebug = true\nfuture_unknown_key = { mode = \"new\" }\n\n[allowed_workflows]\n\"akiojin/gwt\" = [\"prepare-release.yml\", \"build.yml\"] # existing workflows\n\"other/repo\" = [\"other.yml\"]\n";
    std::fs::write(&path, original).unwrap();
    let target = WorkflowPolicyTarget::WindowsDiagnostic;
    assert_eq!(target.filename(), "windows-diagnostic.yml");
    assert_eq!(
        WorkflowPolicyTarget::Release.filename(),
        "prepare-release.yml"
    );
    let before = Settings::workflow_policy_snapshot(&path, "akiojin/gwt", target).unwrap();
    assert!(!before.allowed);
    let (receipt, added) = Settings::with_locked_workflow_policy(
        &path,
        "akiojin/gwt",
        target,
        true,
        |actual, proposed| {
            assert_eq!(actual, &before);
            assert!(!actual.allowed && proposed.allowed);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            Ok(17)
        },
    )
    .unwrap();
    assert_eq!(receipt, 17);
    assert!(added.allowed);
    assert_eq!(
        Settings::workflow_policy_snapshot(&path, "akiojin/gwt", target).unwrap(),
        added
    );
    let (_, replayed) = Settings::with_locked_workflow_policy(
        &path,
        "akiojin/gwt",
        target,
        true,
        |actual, proposed| {
            assert_eq!(actual, &added);
            assert_eq!(actual, proposed);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(replayed, added);
    assert_eq!(
        Settings::load_from_path(&path).unwrap().allowed_workflows["akiojin/gwt"],
        ["prepare-release.yml", "build.yml", "windows-diagnostic.yml"]
    );
    let (_, removed) = Settings::with_locked_workflow_policy(
        &path,
        "akiojin/gwt",
        target,
        false,
        |actual, proposed| {
            assert_eq!(actual, &added);
            assert!(actual.allowed && !proposed.allowed);
            Ok(())
        },
    )
    .unwrap();
    assert!(!removed.allowed);
    assert!(
        Settings::release_policy_snapshot(&path, "akiojin/gwt")
            .unwrap()
            .allowed
    );
    let settings = Settings::load_from_path(&path).unwrap();
    assert_eq!(
        settings.allowed_workflows["akiojin/gwt"],
        ["prepare-release.yml", "build.yml"]
    );
    assert_eq!(settings.allowed_workflows["other/repo"], ["other.yml"]);
    assert!(settings.debug);
    for preserved in [
        "# operator comment",
        "# existing workflows",
        "future_unknown_key = { mode = \"new\" }",
    ] {
        assert!(removed.content.contains(preserved));
    }
}

#[test]
fn diagnostic_workflow_policy_refuses_foreign_repositories_and_rejected_callbacks() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing/config.toml");
    let target = WorkflowPolicyTarget::WindowsDiagnostic;
    assert!(Settings::workflow_policy_snapshot(&path, "other/repo", target).is_err());
    assert!(Settings::with_locked_workflow_policy(
        &path,
        "other/repo",
        target,
        true,
        |_, _| -> Result<(), ConfigError> {
            panic!("foreign policy must be rejected before the callback")
        }
    )
    .is_err());
    assert!(
        !path.parent().unwrap().exists(),
        "foreign target must not create a lock or directory"
    );
    assert!(Settings::with_locked_workflow_policy(
        &path,
        "akiojin/gwt",
        target,
        true,
        |before, after| -> Result<(), ConfigError> {
            assert!(!before.allowed && after.allowed);
            Err(ConfigError::ValidationError {
                reason: "stale CAS token".into(),
            })
        }
    )
    .is_err());
    assert!(
        !path.exists(),
        "rejected callback must not write configuration"
    );
}
