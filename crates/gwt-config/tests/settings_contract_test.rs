//! Public API contract tests for `gwt_config::Settings`.
//!
//! Other crates (gwt, gwt-agent) load and persist settings exclusively
//! through this surface, so these tests pin the externally observable
//! behavior: save/load roundtrip, tolerant parsing for config evolution,
//! and the global config path layout. The real `~/.gwt/config.toml` is
//! never touched.

use std::{num::NonZeroU16, path::Path};

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
