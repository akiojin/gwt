//! gwt-recommended Codex config keys (Issue #4075).
//!
//! gwt keeps a small set of managed keys in the host Codex `config.toml`
//! (`$CODEX_HOME/config.toml`, default `~/.codex/config.toml`). A managed key
//! is written only when the user has not set it: an explicit value, even one
//! that disagrees with gwt's recommendation, is user configuration and is
//! left untouched. The reader / writer is shared with the Codex hook trust
//! registration path so every other table in the file survives the rewrite.

use std::{
    io,
    path::{Path, PathBuf},
};

use crate::{
    codex_hook_trust::{ensure_child_table, read_codex_config},
    settings_local::write_text_atomically,
};

/// Dotted path of the managed key, for logs and ledger rows.
pub const CODEX_CONTEXT_MANAGEMENT_EXPERIMENTAL_MODE_KEY: &str =
    "features.context_management.experimental_mode";

/// What the managed-key pass did to the config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexManagedConfigOutcome {
    /// The key was absent; gwt wrote the recommended value.
    Written,
    /// The key already had a value (any value); the file was not touched.
    Preserved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexManagedConfigReport {
    pub config_path: PathBuf,
    pub outcome: CodexManagedConfigOutcome,
}

/// Ensure `features.context_management.experimental_mode` is set in the Codex
/// config at `config_path`, writing `true` only when the key is absent.
///
/// Idempotent: a config that already carries the key is never rewritten, so a
/// second pass leaves the file bytes and mtime alone. A missing file or
/// missing parent tables are created. A config that cannot be parsed, or whose
/// `features` / `features.context_management` entries are not tables, is an
/// `InvalidData` error and the file is left as-is.
pub fn ensure_codex_context_management_experimental_mode(
    config_path: &Path,
) -> io::Result<CodexManagedConfigReport> {
    let mut root = read_codex_config(config_path)?;
    let root_table = root.as_table_mut().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Codex config root must be a TOML table",
        )
    })?;
    let features = ensure_child_table(root_table, "features")?;
    let context_management = ensure_child_table(features, "context_management")?;
    if context_management.contains_key("experimental_mode") {
        return Ok(CodexManagedConfigReport {
            config_path: config_path.to_path_buf(),
            outcome: CodexManagedConfigOutcome::Preserved,
        });
    }
    context_management.insert("experimental_mode".to_string(), toml::Value::Boolean(true));

    let rendered = toml::to_string_pretty(&root)
        .map_err(|err| io::Error::other(format!("Codex config TOML serialize failed: {err}")))?;
    write_text_atomically(config_path, &rendered)?;

    Ok(CodexManagedConfigReport {
        config_path: config_path.to_path_buf(),
        outcome: CodexManagedConfigOutcome::Written,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const EXISTING_CONFIG: &str = r#"model = "gpt-6-astra"

[features]
web_search = true

[hooks.state."/repo/.codex/hooks.json:session_start:0:0"]
enabled = true
trusted_hash = "sha256:abc"

[model_providers.gwt-anthropic]
name = "Anthropic"
base_url = "http://127.0.0.1:1234/v1"

[projects."/repo/develop"]
trust_level = "trusted"
"#;

    fn parsed(path: &Path) -> toml::Value {
        toml::from_str(&fs::read_to_string(path).unwrap()).expect("config.toml must parse")
    }

    fn experimental_mode(value: &toml::Value) -> Option<&toml::Value> {
        value
            .get("features")?
            .get("context_management")?
            .get("experimental_mode")
    }

    // AC-1
    #[test]
    fn writes_experimental_mode_true_when_table_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/config.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, EXISTING_CONFIG).unwrap();

        let report = ensure_codex_context_management_experimental_mode(&path).unwrap();

        assert_eq!(report.outcome, CodexManagedConfigOutcome::Written);
        assert_eq!(report.config_path, path);
        assert_eq!(
            experimental_mode(&parsed(&path)),
            Some(&toml::Value::Boolean(true))
        );
    }

    // AC-1 (fresh machine: no ~/.codex yet)
    #[test]
    fn creates_config_when_file_and_parent_directory_are_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/config.toml");

        let report = ensure_codex_context_management_experimental_mode(&path).unwrap();

        assert_eq!(report.outcome, CodexManagedConfigOutcome::Written);
        assert_eq!(
            experimental_mode(&parsed(&path)),
            Some(&toml::Value::Boolean(true))
        );
    }

    // AC-2 / AC-4
    #[test]
    fn preserves_explicit_false_and_does_not_rewrite_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let content = "[features.context_management]\nexperimental_mode = false\n";
        fs::write(&path, content).unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();

        let report = ensure_codex_context_management_experimental_mode(&path).unwrap();

        assert_eq!(report.outcome, CodexManagedConfigOutcome::Preserved);
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }

    // AC-2
    #[test]
    fn preserves_explicit_true_line_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let content =
            "# user comment\n[features.context_management]\nexperimental_mode = true # keep\n";
        fs::write(&path, content).unwrap();

        let report = ensure_codex_context_management_experimental_mode(&path).unwrap();

        assert_eq!(report.outcome, CodexManagedConfigOutcome::Preserved);
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }

    // AC-3
    #[test]
    fn roundtrip_keeps_features_hooks_state_model_providers_and_projects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, EXISTING_CONFIG).unwrap();
        let before: toml::Value = toml::from_str(EXISTING_CONFIG).unwrap();

        ensure_codex_context_management_experimental_mode(&path).unwrap();

        let after = parsed(&path);
        assert_eq!(after.get("model"), before.get("model"));
        assert_eq!(
            after.get("features").and_then(|f| f.get("web_search")),
            before.get("features").and_then(|f| f.get("web_search"))
        );
        assert_eq!(after.get("hooks"), before.get("hooks"));
        assert_eq!(after.get("model_providers"), before.get("model_providers"));
        assert_eq!(after.get("projects"), before.get("projects"));
    }

    // AC-4
    #[test]
    fn second_pass_after_write_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, EXISTING_CONFIG).unwrap();

        ensure_codex_context_management_experimental_mode(&path).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();

        let report = ensure_codex_context_management_experimental_mode(&path).unwrap();

        assert_eq!(report.outcome, CodexManagedConfigOutcome::Preserved);
        assert_eq!(fs::read_to_string(&path).unwrap(), written);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }

    // AC-5
    #[test]
    fn unparseable_config_is_reported_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let content = "[features\nthis is not toml";
        fs::write(&path, content).unwrap();

        let error = ensure_codex_context_management_experimental_mode(&path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(
            error.to_string().contains("parse failed"),
            "unexpected error: {error}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }

    // AC-5 (key exists but is not a table)
    #[test]
    fn non_table_features_entry_is_reported_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let content = "features = \"oops\"\n";
        fs::write(&path, content).unwrap();

        let error = ensure_codex_context_management_experimental_mode(&path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }
}
