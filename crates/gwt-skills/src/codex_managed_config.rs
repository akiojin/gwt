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
    codex_hook_trust::{ensure_child_table, read_codex_config, with_codex_config_lock},
    settings_local::write_text_atomically,
};

/// Dotted path of the managed key, for logs and ledger rows.
pub const CODEX_CONTEXT_MANAGEMENT_EXPERIMENTAL_MODE_KEY: &str =
    "features.context_management.experimental_mode";

/// First codex CLI release that loads `[features.context_management]` as a
/// table (Issue #4229). 0.148.0 through 0.152.0 refuse the whole config with
/// `invalid type: map, expected a boolean`.
pub const CODEX_FEATURE_TABLE_MIN_VERSION: &str = "0.153.0";

/// How the codex CLI on `PATH` reads `features.<name>` entries (Issue #4229).
///
/// The host config is shared by every codex on the machine: the one gwt
/// launches (often `bunx @openai/codex@latest`) and the one the user types
/// (`codex` on `PATH`). They are different binaries, so a managed key is safe
/// only when the `PATH` codex can load it too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexFeaturesSchema {
    /// `features.<name>` may be a table: codex >= 0.153.0, or no codex on
    /// `PATH` at all.
    AcceptsTables,
    /// Every `features.<name>` must be a boolean and a single table makes the
    /// whole config unloadable: codex <= 0.152.x, or a version gwt could not
    /// read.
    BooleansOnly,
}

impl CodexFeaturesSchema {
    /// Whether a codex reading with this schema loads `value` as a
    /// `features.<name>` entry.
    pub fn accepts_feature_value(self, value: &toml::Value) -> bool {
        self == Self::AcceptsTables || value.is_bool()
    }
}

/// What the managed-key pass did to the config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexManagedConfigOutcome {
    /// The key was absent; gwt wrote the recommended value.
    Written,
    /// The key already had a value (any value); the file was not touched.
    Preserved,
    /// The key was absent, but the `PATH` codex cannot load it; nothing was
    /// written.
    Skipped,
    /// A `features.context_management` table the `PATH` codex cannot load was
    /// removed so that codex can read the config again.
    Repaired,
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
///
/// `schema` is how the codex on `PATH` reads `features` (Issue #4229). Under
/// [`CodexFeaturesSchema::BooleansOnly`] the table is never written, and one
/// already present is removed because it makes that codex unable to load any
/// of the config.
pub fn ensure_codex_context_management_experimental_mode(
    config_path: &Path,
    schema: CodexFeaturesSchema,
) -> io::Result<CodexManagedConfigReport> {
    // Issue #4071: the hook trust registration mutates this same shared file,
    // so both writers take the same lock or one of them loses its update.
    let outcome = with_codex_config_lock(config_path, || {
        let mut root = read_codex_config(config_path)?;
        let root_table = root.as_table_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Codex config root must be a TOML table",
            )
        })?;
        let features = ensure_child_table(root_table, "features")?;
        if schema == CodexFeaturesSchema::BooleansOnly {
            let entry_loads = features
                .get("context_management")
                .map(|entry| schema.accepts_feature_value(entry));
            return match entry_loads {
                None => Ok(CodexManagedConfigOutcome::Skipped),
                Some(true) => Ok(CodexManagedConfigOutcome::Preserved),
                Some(false) => {
                    features.remove("context_management");
                    write_codex_config(config_path, &root)?;
                    Ok(CodexManagedConfigOutcome::Repaired)
                }
            };
        }
        let context_management = ensure_child_table(features, "context_management")?;
        if context_management.contains_key("experimental_mode") {
            return Ok(CodexManagedConfigOutcome::Preserved);
        }
        context_management.insert("experimental_mode".to_string(), toml::Value::Boolean(true));

        write_codex_config(config_path, &root)?;
        Ok(CodexManagedConfigOutcome::Written)
    })?;

    Ok(CodexManagedConfigReport {
        config_path: config_path.to_path_buf(),
        outcome,
    })
}

fn write_codex_config(config_path: &Path, root: &toml::Value) -> io::Result<()> {
    let rendered = toml::to_string_pretty(root)
        .map_err(|err| io::Error::other(format!("Codex config TOML serialize failed: {err}")))?;
    write_text_atomically(config_path, &rendered)
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

        let report = ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::AcceptsTables,
        )
        .unwrap();

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

        let report = ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::AcceptsTables,
        )
        .unwrap();

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

        let report = ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::AcceptsTables,
        )
        .unwrap();

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

        let report = ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::AcceptsTables,
        )
        .unwrap();

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

        ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::AcceptsTables,
        )
        .unwrap();

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

        ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::AcceptsTables,
        )
        .unwrap();
        let written = fs::read_to_string(&path).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();

        let report = ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::AcceptsTables,
        )
        .unwrap();

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

        let error = ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::AcceptsTables,
        )
        .unwrap_err();

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

        let error = ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::AcceptsTables,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }

    /// Issue #4229 AC-4: what codex 0.148.0–0.152.x accepts — every
    /// `features.<name>` is a boolean, or the whole config fails to load.
    fn loads_under_codex_0_148_schema(value: &toml::Value) -> bool {
        value
            .get("features")
            .and_then(toml::Value::as_table)
            .is_none_or(|features| {
                features
                    .values()
                    .all(|entry| CodexFeaturesSchema::BooleansOnly.accepts_feature_value(entry))
            })
    }

    // Issue #4229 AC-2 / AC-4
    #[test]
    fn booleans_only_path_codex_never_gets_the_table_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, EXISTING_CONFIG).unwrap();

        let report = ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::BooleansOnly,
        )
        .unwrap();

        assert_eq!(report.outcome, CodexManagedConfigOutcome::Skipped);
        assert_eq!(fs::read_to_string(&path).unwrap(), EXISTING_CONFIG);
        assert!(loads_under_codex_0_148_schema(&parsed(&path)));
    }

    // Issue #4229 AC-3 / AC-4
    #[test]
    fn booleans_only_path_codex_gets_an_existing_table_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let broken =
            format!("{EXISTING_CONFIG}\n[features.context_management]\nexperimental_mode = true\n");
        fs::write(&path, &broken).unwrap();
        let before: toml::Value = toml::from_str(&broken).unwrap();
        assert!(!loads_under_codex_0_148_schema(&before));

        let report = ensure_codex_context_management_experimental_mode(
            &path,
            CodexFeaturesSchema::BooleansOnly,
        )
        .unwrap();

        assert_eq!(report.outcome, CodexManagedConfigOutcome::Repaired);
        let after = parsed(&path);
        assert!(loads_under_codex_0_148_schema(&after));
        assert_eq!(experimental_mode(&after), None);
        assert_eq!(
            after.get("features").and_then(|f| f.get("web_search")),
            Some(&toml::Value::Boolean(true))
        );
        assert_eq!(after.get("hooks"), before.get("hooks"));
        assert_eq!(after.get("projects"), before.get("projects"));
    }
}
