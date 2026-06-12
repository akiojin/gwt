//! Public API contract tests for `gwt_config::Settings`.
//!
//! Other crates (gwt, gwt-agent) load and persist settings exclusively
//! through this surface, so these tests pin the externally observable
//! behavior: save/load roundtrip, tolerant parsing for config evolution,
//! and the global config path layout. The real `~/.gwt/config.toml` is
//! never touched.

use std::path::Path;

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
    let home = Path::new("home-fixture");
    assert_eq!(
        Settings::global_config_path_for_home(home),
        Path::new("home-fixture").join(".gwt").join("config.toml")
    );
}
