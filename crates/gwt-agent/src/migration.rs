//! Idempotent silent migration of legacy `ClaudeCodeOpenaiCompat` rows
//! (SPEC-1921 FR-101, FR-102 persistence side).
//!
//! Before the 2026-05-18 amendment, Claude Code backend overrides were
//! persisted as ordinary Custom Coding Agents under
//! `[tools.customCodingAgents.<id>]` with `command =
//! "@anthropic-ai/claude-code@latest"` and the upstream URL / API key carried
//! through the `[...env]` sub-table. After the amendment, Backend Override
//! is a built-in attribute under `[builtinAgents.claudeCode.backends.<id>]`.
//!
//! This module performs a single scan that moves legacy rows to the new
//! schema and removes them from `customCodingAgents`. The scan:
//!
//! - is safe to run repeatedly (returns an empty report when nothing matches);
//! - disambiguates id collisions deterministically with `-N` suffix when the
//!   target builtin section already has a profile of the same id;
//! - leaves non-backend custom agents untouched;
//! - never aborts startup on parse errors — invalid rows are reported via
//!   structured logs and skipped.
//!
//! Callers also use [`resolve_legacy_backend_remap`] at relaunch time to
//! re-map a persisted `AgentId::Custom("<old-id>")` reference onto the new
//! `(AgentId::ClaudeCode, backend_id="<old-id>")` representation.

use std::path::Path;

use gwt_config::atomic::write_atomic as write_atomic_shared;
use toml::{Table, Value};
use tracing::warn;

use crate::backend::{AgentBackendProfile, BuiltinAgentId};

/// Summary of what the migration touched. Useful in tests and Workspace
/// telemetry; production callers can ignore the contents.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// New backend profile ids that were created under
    /// `[builtinAgents.claudeCode.backends.*]` during this scan. Empty when
    /// nothing matched.
    pub migrated_claude_code_ids: Vec<String>,
    /// Legacy `[tools.customCodingAgents.*]` entry ids that were removed
    /// because they were detected as Claude Code backend overrides.
    pub removed_custom_agent_ids: Vec<String>,
    /// Renames applied to resolve id collisions, as `(original_id,
    /// chosen_id)` pairs.
    pub renamed: Vec<(String, String)>,
}

impl MigrationReport {
    /// `true` when the scan produced any change.
    pub fn changed(&self) -> bool {
        !self.migrated_claude_code_ids.is_empty()
    }
}

/// Run the idempotent silent migration against the gwt config TOML at
/// `path`. Returns a report describing what changed; an empty report when
/// nothing matched or the file does not exist.
pub fn migrate_legacy_backend_rows(path: &Path) -> Result<MigrationReport, String> {
    if !path.exists() {
        return Ok(MigrationReport::default());
    }

    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read config {}: {err}", path.display()))?;
    let mut root: Value = toml::from_str(&content)
        .map_err(|err| format!("failed to parse config {}: {err}", path.display()))?;

    let root_table = root
        .as_table_mut()
        .ok_or_else(|| format!("config {} must contain a TOML table root", path.display()))?;

    let mut report = MigrationReport::default();

    let detected = detect_legacy_claude_code_rows(root_table, &mut report);
    if detected.is_empty() {
        return Ok(report);
    }

    let existing_backend_ids = existing_backend_ids(root_table, BuiltinAgentId::ClaudeCode);
    let mut reserved = existing_backend_ids;
    let mut new_profiles: Vec<AgentBackendProfile> = Vec::with_capacity(detected.len());

    for (legacy_id, profile_candidate) in detected {
        let chosen_id = pick_unique_id(&legacy_id, &reserved);
        if chosen_id != legacy_id {
            report.renamed.push((legacy_id.clone(), chosen_id.clone()));
        }
        reserved.push(chosen_id.clone());

        let mut profile = profile_candidate;
        profile.id = chosen_id.clone();

        if let Err(err) = profile.validate(BuiltinAgentId::ClaudeCode) {
            warn!(
                legacy_id = %legacy_id,
                error = err,
                "skipping legacy Claude Code backend row that fails validation"
            );
            // Keep the original custom-agent row in place rather than dropping
            // user data on the floor.
            continue;
        }

        report.migrated_claude_code_ids.push(profile.id.clone());
        report.removed_custom_agent_ids.push(legacy_id);
        new_profiles.push(profile);
    }

    if new_profiles.is_empty() {
        return Ok(report);
    }

    // Remove the legacy custom-agent rows from the in-memory TOML.
    if let Some(tools_value) = root_table.get_mut("tools") {
        if let Some(tools_table) = tools_value.as_table_mut() {
            for id in &report.removed_custom_agent_ids {
                if let Some(custom_table) = tools_table
                    .get_mut("customCodingAgents")
                    .and_then(Value::as_table_mut)
                {
                    custom_table.remove(id);
                }
                if let Some(custom_table) = tools_table
                    .get_mut("custom_coding_agents")
                    .and_then(Value::as_table_mut)
                {
                    custom_table.remove(id);
                }
            }
            // Drop empty `customCodingAgents` / `custom_coding_agents`
            // sub-tables so a fully migrated config does not leave behind
            // empty sections.
            if let Some(custom_table) = tools_table
                .get("customCodingAgents")
                .and_then(Value::as_table)
            {
                if custom_table.is_empty() {
                    tools_table.remove("customCodingAgents");
                }
            }
            if let Some(custom_table) = tools_table
                .get("custom_coding_agents")
                .and_then(Value::as_table)
            {
                if custom_table.is_empty() {
                    tools_table.remove("custom_coding_agents");
                }
            }
            if tools_table.is_empty() {
                root_table.remove("tools");
            }
        }
    }

    // Insert the new backend profiles under [builtinAgents.claudeCode.backends.*].
    let builtin_entry = root_table
        .entry("builtinAgents".to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    let builtin_table = builtin_entry.as_table_mut().ok_or_else(|| {
        format!(
            "config {} has a non-table [builtinAgents] section",
            path.display()
        )
    })?;
    let agent_entry = builtin_table
        .entry(BuiltinAgentId::ClaudeCode.as_str().to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    let agent_table = agent_entry.as_table_mut().ok_or_else(|| {
        format!(
            "config {} has a non-table [builtinAgents.claudeCode] section",
            path.display()
        )
    })?;
    let backends_entry = agent_table
        .entry("backends".to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    let backends_table = backends_entry.as_table_mut().ok_or_else(|| {
        format!(
            "config {} has a non-table [builtinAgents.claudeCode.backends] section",
            path.display()
        )
    })?;

    for profile in &new_profiles {
        backends_table.insert(
            profile.id.clone(),
            Value::try_from(profile).map_err(|err| {
                format!("failed to serialize migrated backend {}: {err}", profile.id)
            })?,
        );
    }

    let serialized = toml::to_string_pretty(&root)
        .map_err(|err| format!("failed to serialize config {}: {err}", path.display()))?;
    write_atomic_shared(path, &serialized)
        .map_err(|err| format!("failed to write config {}: {err}", path.display()))?;

    Ok(report)
}

/// Resolve a relaunch-time `AgentId::Custom("<id>")` reference to a
/// migrated Claude Code backend id if one exists in the current config.
///
/// Returns `Some(backend_id)` when `[builtinAgents.claudeCode.backends.<id>]`
/// is present in `config_path`, otherwise `None`.
pub fn resolve_legacy_backend_remap(
    agent_id: &crate::types::AgentId,
    config_path: &Path,
) -> Option<String> {
    let crate::types::AgentId::Custom(raw) = agent_id else {
        return None;
    };
    let id = raw.trim();
    if id.is_empty() {
        return None;
    }
    let backends =
        crate::backend_store::load_backends_for_agent(config_path, BuiltinAgentId::ClaudeCode)
            .ok()?;
    backends
        .into_iter()
        .find(|profile| profile.id == id)
        .map(|profile| profile.id)
}

fn detect_legacy_claude_code_rows(
    root_table: &Table,
    report: &mut MigrationReport,
) -> Vec<(String, AgentBackendProfile)> {
    let Some(tools_table) = root_table.get("tools").and_then(Value::as_table) else {
        return Vec::new();
    };
    let mut detected = Vec::new();
    for section_name in ["customCodingAgents", "custom_coding_agents"] {
        let Some(custom_table) = tools_table.get(section_name).and_then(Value::as_table) else {
            continue;
        };
        for (key, raw_value) in custom_table {
            let Some(row) = raw_value.as_table() else {
                continue;
            };
            let Some(command) = row.get("command").and_then(Value::as_str) else {
                continue;
            };
            if command != "@anthropic-ai/claude-code@latest" {
                continue;
            }
            let Some(env_table) = row.get("env").and_then(Value::as_table) else {
                continue;
            };
            let Some(base_url) = env_table.get("ANTHROPIC_BASE_URL").and_then(Value::as_str) else {
                continue;
            };

            let api_key = env_table
                .get("ANTHROPIC_API_KEY")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // FR-101: derive `model` from the most authoritative env var.
            let model = env_table
                .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
                .and_then(Value::as_str)
                .or_else(|| {
                    env_table
                        .get("CLAUDE_CODE_SUBAGENT_MODEL")
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    env_table
                        .get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    env_table
                        .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
                        .and_then(Value::as_str)
                })
                .unwrap_or("")
                .to_string();

            let display_name = row
                .get("displayName")
                .and_then(Value::as_str)
                .or_else(|| row.get("display_name").and_then(Value::as_str))
                .unwrap_or(key)
                .to_string();

            if model.trim().is_empty() {
                // Cannot derive a usable model — keep the legacy row in
                // place and warn so the user notices.
                warn!(
                    legacy_id = %key,
                    "legacy Claude Code backend row has no resolvable model env var; skipping migration"
                );
                let _ = report; // silence unused if no further logic runs
                continue;
            }

            let profile = AgentBackendProfile {
                id: key.clone(),
                display_name,
                base_url: base_url.to_string(),
                api_key,
                model,
                ..Default::default()
            };
            detected.push((key.clone(), profile));
        }
    }
    detected
}

fn existing_backend_ids(root_table: &Table, agent: BuiltinAgentId) -> Vec<String> {
    root_table
        .get("builtinAgents")
        .and_then(Value::as_table)
        .and_then(|t| t.get(agent.as_str()))
        .and_then(Value::as_table)
        .and_then(|t| t.get("backends"))
        .and_then(Value::as_table)
        .map(|backends| backends.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default()
}

fn pick_unique_id(preferred: &str, reserved: &[String]) -> String {
    if !reserved.iter().any(|r| r == preferred) {
        return preferred.to_string();
    }
    for n in 2..=999 {
        let candidate = format!("{preferred}-{n}");
        if !reserved.iter().any(|r| r == &candidate) {
            return candidate;
        }
    }
    // Fallback that should never realistically trigger.
    format!("{preferred}-collision")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_store::load_backends_for_agent;

    const LEGACY_CONFIG: &str = r#"
[tools.customCodingAgents.claude-code-openai]
id = "claude-code-openai"
displayName = "Claude Code (OpenAI-compat)"
agentType = "bunx"
command = "@anthropic-ai/claude-code@latest"
defaultArgs = []
skipPermissionsArgs = ["--dangerously-skip-permissions"]

[tools.customCodingAgents.claude-code-openai.env]
ANTHROPIC_API_KEY = "sk-legacy"
ANTHROPIC_BASE_URL = "http://192.168.100.166:32768"
ANTHROPIC_DEFAULT_HAIKU_MODEL = "openai/gpt-oss-20b"
ANTHROPIC_DEFAULT_OPUS_MODEL = "openai/gpt-oss-20b"
ANTHROPIC_DEFAULT_SONNET_MODEL = "openai/gpt-oss-20b"
CLAUDE_CODE_SUBAGENT_MODEL = "openai/gpt-oss-20b"
"#;

    #[test]
    fn migration_returns_empty_report_when_file_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing.toml");
        let report = migrate_legacy_backend_rows(&path).expect("ok");
        assert!(!report.changed());
        assert!(report.migrated_claude_code_ids.is_empty());
    }

    #[test]
    fn migration_returns_empty_report_when_no_legacy_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[tools.customCodingAgents.aider]
id = "aider"
displayName = "Aider"
agentType = "command"
command = "aider"
"#,
        )
        .expect("write");
        let report = migrate_legacy_backend_rows(&path).expect("ok");
        assert!(!report.changed());
        // External Agent must remain in place.
        let content = std::fs::read_to_string(&path).expect("read");
        assert!(content.contains("[tools.customCodingAgents.aider]"));
    }

    #[test]
    fn migration_moves_legacy_claude_code_row_to_backends_section() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, LEGACY_CONFIG).expect("write");

        let report = migrate_legacy_backend_rows(&path).expect("ok");
        assert!(report.changed());
        assert_eq!(
            report.migrated_claude_code_ids,
            vec!["claude-code-openai".to_string()]
        );
        assert_eq!(
            report.removed_custom_agent_ids,
            vec!["claude-code-openai".to_string()]
        );
        assert!(report.renamed.is_empty());

        let content = std::fs::read_to_string(&path).expect("read");
        // Legacy row is gone.
        assert!(!content.contains("[tools.customCodingAgents.claude-code-openai]"));
        // New row is present.
        assert!(content.contains("[builtinAgents.claudeCode.backends.claude-code-openai]"));

        let loaded = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "claude-code-openai");
        assert_eq!(loaded[0].base_url, "http://192.168.100.166:32768");
        assert_eq!(loaded[0].api_key, "sk-legacy");
        assert_eq!(loaded[0].model, "openai/gpt-oss-20b");
        assert_eq!(loaded[0].display_name, "Claude Code (OpenAI-compat)");
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, LEGACY_CONFIG).expect("write");

        let first = migrate_legacy_backend_rows(&path).expect("first");
        assert!(first.changed());

        let second = migrate_legacy_backend_rows(&path).expect("second");
        assert!(!second.changed());
        assert!(second.migrated_claude_code_ids.is_empty());

        let loaded = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load");
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn migration_disambiguates_id_when_backend_already_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let mut combined = String::from(LEGACY_CONFIG);
        combined.push_str(
            r#"
[builtinAgents.claudeCode.backends.claude-code-openai]
id = "claude-code-openai"
displayName = "Preexisting"
baseUrl = "http://existing.example.com"
apiKey = "sk-existing"
model = "existing-model"
"#,
        );
        std::fs::write(&path, combined).expect("write");

        let report = migrate_legacy_backend_rows(&path).expect("ok");
        assert!(report.changed());
        assert_eq!(report.renamed.len(), 1);
        assert_eq!(report.renamed[0].0, "claude-code-openai");
        assert_eq!(report.renamed[0].1, "claude-code-openai-2");
        assert_eq!(
            report.migrated_claude_code_ids,
            vec!["claude-code-openai-2".to_string()]
        );

        let loaded = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load");
        assert_eq!(loaded.len(), 2);
        // Both ids end up alphabetically sorted.
        let ids: Vec<&String> = loaded.iter().map(|p| &p.id).collect();
        assert!(ids.contains(&&"claude-code-openai".to_string()));
        assert!(ids.contains(&&"claude-code-openai-2".to_string()));
    }

    #[test]
    fn migration_leaves_non_backend_custom_agents_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let mut combined = String::from(LEGACY_CONFIG);
        combined.push_str(
            r#"
[tools.customCodingAgents.aider]
id = "aider"
displayName = "Aider"
agentType = "command"
command = "aider"
"#,
        );
        std::fs::write(&path, combined).expect("write");

        let report = migrate_legacy_backend_rows(&path).expect("ok");
        assert!(report.changed());

        let content = std::fs::read_to_string(&path).expect("read");
        // Legacy Claude Code row migrated away.
        assert!(!content.contains("[tools.customCodingAgents.claude-code-openai]"));
        // Aider stays as an External Agent.
        assert!(content.contains("[tools.customCodingAgents.aider]"));
        assert!(content.contains("command = \"aider\""));
    }

    #[test]
    fn migration_skips_claude_code_command_without_anthropic_base_url() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[tools.customCodingAgents.claude-without-backend]
id = "claude-without-backend"
displayName = "Bare Claude"
agentType = "bunx"
command = "@anthropic-ai/claude-code@latest"

[tools.customCodingAgents.claude-without-backend.env]
SOMETHING_ELSE = "x"
"#,
        )
        .expect("write");

        let report = migrate_legacy_backend_rows(&path).expect("ok");
        assert!(!report.changed());

        let content = std::fs::read_to_string(&path).expect("read");
        // Untouched: the row stays under customCodingAgents.
        assert!(content.contains("[tools.customCodingAgents.claude-without-backend]"));
    }

    #[test]
    fn migration_preserves_sibling_root_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let mut combined = String::from("debug = true\n\n");
        combined.push_str(LEGACY_CONFIG);
        std::fs::write(&path, combined).expect("write");

        migrate_legacy_backend_rows(&path).expect("migrate");

        let content = std::fs::read_to_string(&path).expect("read");
        assert!(content.contains("debug = true"));
    }

    #[test]
    fn migration_skips_legacy_row_when_no_model_env_var_resolves() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[tools.customCodingAgents.no-model]
id = "no-model"
displayName = "No Model"
agentType = "bunx"
command = "@anthropic-ai/claude-code@latest"

[tools.customCodingAgents.no-model.env]
ANTHROPIC_API_KEY = "sk"
ANTHROPIC_BASE_URL = "http://x"
"#,
        )
        .expect("write");

        let report = migrate_legacy_backend_rows(&path).expect("migrate");
        assert!(!report.changed());

        let content = std::fs::read_to_string(&path).expect("read");
        assert!(content.contains("[tools.customCodingAgents.no-model]"));
    }

    #[test]
    fn resolve_legacy_backend_remap_returns_some_when_backend_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, LEGACY_CONFIG).expect("write");
        migrate_legacy_backend_rows(&path).expect("migrate");

        let resolved = resolve_legacy_backend_remap(
            &crate::types::AgentId::Custom("claude-code-openai".into()),
            &path,
        );
        assert_eq!(resolved, Some("claude-code-openai".to_string()));
    }

    #[test]
    fn resolve_legacy_backend_remap_returns_none_when_backend_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let resolved = resolve_legacy_backend_remap(
            &crate::types::AgentId::Custom("never-existed".into()),
            &path,
        );
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_legacy_backend_remap_returns_none_for_non_custom_agent_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, LEGACY_CONFIG).expect("write");
        migrate_legacy_backend_rows(&path).expect("migrate");

        assert!(resolve_legacy_backend_remap(&crate::types::AgentId::ClaudeCode, &path).is_none());
        assert!(resolve_legacy_backend_remap(&crate::types::AgentId::Codex, &path).is_none());
    }
}
