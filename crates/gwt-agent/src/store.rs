//! TOML persistence for custom coding agents.
//!
//! Custom agents live under `[tools.customCodingAgents.<id>]` in the gwt
//! global config TOML (`~/.gwt/config.toml` by default). The load/save
//! surface preserves unknown sibling tables (e.g. `models`) so third-party
//! additions are not silently dropped on round-trip (SPEC-1921 FR-059).

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use gwt_config::atomic::write_atomic as write_atomic_shared;
use serde::{Deserialize, Serialize};
use toml::{Table, Value};
use tracing::warn;

use crate::custom::{CustomAgentType, CustomCodingAgent, ModeArgs};

/// Env var that disables loading custom agents from the global config (for
/// tests and isolated runs).
pub const DISABLE_GLOBAL_CUSTOM_AGENTS_ENV: &str = "GWT_DISABLE_GLOBAL_CUSTOM_AGENTS";

/// A custom agent as loaded from TOML, paired with its raw sibling-field table
/// so sibling keys such as `models` survive a load/save round-trip.
#[derive(Debug, Clone)]
pub struct StoredCustomAgent {
    /// Parsed custom agent.
    pub agent: CustomCodingAgent,
    /// Raw TOML table as read from disk. Used to preserve unknown keys
    /// (e.g. `models`, future sibling fields) when the entry is re-serialized.
    raw: Table,
}

impl StoredCustomAgent {
    /// Wrap a freshly-built `CustomCodingAgent` with an empty raw table.
    pub fn new(agent: CustomCodingAgent) -> Self {
        Self {
            agent,
            raw: Table::new(),
        }
    }
}

/// Canonical TOML shape for a single custom agent entry. Accepts both
/// camelCase (preferred, SPEC-1921 Custom Agent Schema) and snake_case forms
/// for backwards-compatibility with older configs.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CustomAgentToml {
    #[serde(default)]
    id: String,
    #[serde(rename = "displayName", alias = "display_name")]
    display_name: String,
    #[serde(rename = "agentType", alias = "type", alias = "agent_type", default)]
    agent_type: CustomAgentType,
    command: String,
    #[serde(
        default,
        rename = "defaultArgs",
        alias = "default_args",
        skip_serializing_if = "Vec::is_empty"
    )]
    default_args: Vec<String>,
    #[serde(
        default,
        rename = "skipPermissionsArgs",
        alias = "skip_permissions_args",
        skip_serializing_if = "Vec::is_empty"
    )]
    skip_permissions_args: Vec<String>,
    #[serde(
        default,
        rename = "modeArgs",
        alias = "mode_args",
        skip_serializing_if = "Option::is_none"
    )]
    mode_args: Option<ModeArgs>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, String>,
}

impl CustomAgentToml {
    fn into_custom_agent(self, key: &str) -> Option<CustomCodingAgent> {
        let agent = CustomCodingAgent {
            id: if self.id.trim().is_empty() {
                key.to_string()
            } else {
                self.id
            },
            display_name: self.display_name,
            agent_type: self.agent_type,
            command: self.command,
            default_args: self.default_args,
            skip_permissions_args: self.skip_permissions_args,
            mode_args: self.mode_args,
            env: self.env,
        };

        agent.validate().then_some(agent)
    }
}

impl From<&CustomCodingAgent> for CustomAgentToml {
    fn from(agent: &CustomCodingAgent) -> Self {
        Self {
            id: agent.id.clone(),
            display_name: agent.display_name.clone(),
            agent_type: agent.agent_type,
            command: agent.command.clone(),
            default_args: agent.default_args.clone(),
            skip_permissions_args: agent.skip_permissions_args.clone(),
            mode_args: agent.mode_args.clone(),
            env: agent.env.clone(),
        }
    }
}

/// Load plain custom agents (no sibling-preservation bookkeeping) from the
/// given config path. Returns an empty `Vec` when the path does not exist.
pub fn load_custom_agents_from_path(path: &Path) -> Result<Vec<CustomCodingAgent>, String> {
    Ok(load_stored_custom_agents_from_path(path)?
        .into_iter()
        .map(|entry| entry.agent)
        .collect())
}

/// Load stored custom agents (with raw sibling-field tables) from the given
/// path. Used by callers that later want to re-save with
/// [`save_stored_custom_agents_to_path`].
pub fn load_stored_custom_agents_from_path(path: &Path) -> Result<Vec<StoredCustomAgent>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read config {}: {err}", path.display()))?;
    let root: Value = toml::from_str(&content)
        .map_err(|err| format!("failed to parse custom agents in {}: {err}", path.display()))?;
    let Some(custom_table) = custom_agents_table(&root) else {
        return Ok(Vec::new());
    };

    let mut agents = Vec::new();
    for (key, raw_value) in custom_table {
        let Some(raw_table) = raw_value.as_table() else {
            warn!(custom_agent = %key, "skipping non-table custom agent entry");
            continue;
        };
        let parsed: CustomAgentToml = match raw_value.clone().try_into() {
            Ok(parsed) => parsed,
            Err(err) => {
                warn!(custom_agent = %key, error = %err, "skipping unparsable custom agent");
                continue;
            }
        };
        if let Some(agent) = parsed.into_custom_agent(key) {
            agents.push(StoredCustomAgent {
                agent,
                raw: raw_table.clone(),
            });
        } else {
            warn!(custom_agent = %key, "skipping invalid custom agent");
        }
    }

    Ok(agents)
}

/// Save the provided custom agents into the given config path. Preserves
/// sibling root-level tables (e.g. top-level `debug = true`) and
/// sibling tables under each custom agent (e.g. `models`). Rejects
/// duplicate IDs.
pub fn save_stored_custom_agents_to_path(
    path: &Path,
    agents: &[StoredCustomAgent],
) -> Result<(), String> {
    validate_unique_custom_agents(agents)?;

    let mut root = load_root_document(path)?;
    let root_table = root
        .as_table_mut()
        .ok_or_else(|| format!("config {} must contain a TOML table root", path.display()))?;

    let tools_entry = root_table
        .entry("tools".to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    let tools_table = tools_entry
        .as_table_mut()
        .ok_or_else(|| format!("config {} has a non-table [tools] section", path.display()))?;

    tools_table.remove("customCodingAgents");
    tools_table.remove("custom_coding_agents");

    if !agents.is_empty() {
        let mut custom_table = Table::new();
        for entry in agents {
            custom_table.insert(
                entry.agent.id.clone(),
                Value::Table(normalized_custom_agent_table(entry)?),
            );
        }
        tools_table.insert("customCodingAgents".to_string(), Value::Table(custom_table));
    }

    if tools_table.is_empty() {
        root_table.remove("tools");
    }

    let content = toml::to_string_pretty(&root)
        .map_err(|err| format!("failed to serialize config {}: {err}", path.display()))?;
    // Delegate to the shared atomic writer so custom-agent configs pick up
    // the same `0o600` permissions hardening that `gwt-config` applies to
    // other secret-bearing files on Unix. SPEC-1921 FR-063: api_key values
    // are persisted here.
    write_atomic_shared(path, &content)
        .map_err(|err| format!("failed to write config {}: {err}", path.display()))
}

fn validate_unique_custom_agents(agents: &[StoredCustomAgent]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for entry in agents {
        if !entry.agent.validate() {
            return Err(format!("invalid custom agent: {}", entry.agent.id));
        }
        if !seen.insert(entry.agent.id.clone()) {
            return Err(format!("duplicate custom agent id: {}", entry.agent.id));
        }
    }
    Ok(())
}

/// Canonicalize the TOML representation of a single agent entry, preserving
/// any unknown sibling keys (e.g. `models`).
fn normalized_custom_agent_table(entry: &StoredCustomAgent) -> Result<Table, String> {
    let mut raw = entry.raw.clone();
    for key in [
        "id",
        "displayName",
        "display_name",
        "agentType",
        "agent_type",
        "type",
        "command",
        "defaultArgs",
        "default_args",
        "skipPermissionsArgs",
        "skip_permissions_args",
        "modeArgs",
        "mode_args",
        "env",
    ] {
        raw.remove(key);
    }

    let canonical: Value = Value::try_from(CustomAgentToml::from(&entry.agent))
        .map_err(|err| format!("failed to serialize custom agent {}: {err}", entry.agent.id))?;
    let canonical_table = canonical.as_table().ok_or_else(|| {
        format!(
            "custom agent {} did not serialize as a table",
            entry.agent.id
        )
    })?;
    for (key, value) in canonical_table {
        raw.insert(key.clone(), value.clone());
    }

    Ok(raw)
}

fn custom_agents_table(root: &Value) -> Option<&Table> {
    let tools = root.get("tools")?.as_table()?;
    tools
        .get("customCodingAgents")
        .and_then(Value::as_table)
        .or_else(|| tools.get("custom_coding_agents").and_then(Value::as_table))
}

fn load_root_document(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Table(Table::new()));
    }

    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read config {}: {err}", path.display()))?;
    toml::from_str(&content)
        .map_err(|err| format!("failed to parse config {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::claude_code_openai_compat_preset;

    #[test]
    fn load_custom_agents_parses_camelcase_schema() {
        let dir = tempfile::tempdir().expect("temp config dir");
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
[tools.customCodingAgents.my-agent]
id = "my-agent"
displayName = "My Agent"
agentType = "command"
command = "my-agent-cli"
defaultArgs = ["--flag"]
skipPermissionsArgs = ["--yolo"]

[tools.customCodingAgents.my-agent.modeArgs]
normal = ["--normal"]
continue = ["--continue"]
resume = ["--resume"]

[tools.customCodingAgents.my-agent.env]
CUSTOM_ENV = "enabled"
"#,
        )
        .expect("write config");

        let agents = load_custom_agents_from_path(&config_path).expect("load custom agents");

        assert_eq!(agents.len(), 1);
        let agent = &agents[0];
        assert_eq!(agent.id, "my-agent");
        assert_eq!(agent.display_name, "My Agent");
        assert_eq!(agent.agent_type, CustomAgentType::Command);
        assert_eq!(agent.command, "my-agent-cli");
        assert_eq!(agent.default_args, vec!["--flag"]);
        assert_eq!(agent.skip_permissions_args, vec!["--yolo"]);
        assert_eq!(
            agent
                .mode_args
                .as_ref()
                .map(|args| args.continue_mode.clone()),
            Some(vec!["--continue".to_string()])
        );
        assert_eq!(
            agent.env.get("CUSTOM_ENV").map(String::as_str),
            Some("enabled")
        );
    }

    #[test]
    fn load_custom_agents_accepts_snake_case_for_backwards_compat() {
        let dir = tempfile::tempdir().expect("temp config dir");
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
[tools.custom_coding_agents.legacy]
id = "legacy"
display_name = "Legacy Agent"
type = "bunx"
command = "@legacy/cli"
default_args = ["--x"]
"#,
        )
        .expect("write config");

        let agents = load_custom_agents_from_path(&config_path).expect("load");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "legacy");
        assert_eq!(agents[0].agent_type, CustomAgentType::Bunx);
        assert_eq!(agents[0].command, "@legacy/cli");
    }

    #[test]
    fn load_returns_empty_when_no_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("missing.toml");
        let agents = load_custom_agents_from_path(&path).expect("load");
        assert!(agents.is_empty());
    }

    #[test]
    fn load_returns_empty_when_no_section() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "debug = true\n").expect("write");
        let agents = load_custom_agents_from_path(&path).expect("load");
        assert!(agents.is_empty());
    }

    #[test]
    fn save_preserves_sibling_root_fields_and_custom_agent_subtables() {
        let dir = tempfile::tempdir().expect("temp config dir");
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
debug = true

[tools.customCodingAgents.my-agent]
id = "my-agent"
displayName = "My Agent"
agentType = "command"
command = "my-agent-cli"

[tools.customCodingAgents.my-agent.models]
default = { id = "default", label = "Default", arg = "" }
"#,
        )
        .expect("write config");

        let mut agents = load_stored_custom_agents_from_path(&config_path).expect("load stored");
        assert_eq!(agents.len(), 1);
        agents[0].agent.display_name = "Updated Agent".to_string();

        save_stored_custom_agents_to_path(&config_path, &agents).expect("save stored");

        let content = std::fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("debug = true"));
        assert!(content.contains("displayName = \"Updated Agent\""));
        let parsed: Value = toml::from_str(&content).expect("parse saved config");
        assert_eq!(
            parsed["tools"]["customCodingAgents"]["my-agent"]["models"]["default"]["label"]
                .as_str(),
            Some("Default")
        );
    }

    #[test]
    fn save_rejects_duplicate_ids() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        let a1 = StoredCustomAgent::new(CustomCodingAgent {
            id: "dup".to_string(),
            display_name: "D1".to_string(),
            agent_type: CustomAgentType::Command,
            command: "c1".to_string(),
            default_args: vec![],
            mode_args: None,
            skip_permissions_args: vec![],
            env: HashMap::new(),
        });
        let a2 = StoredCustomAgent::new(CustomCodingAgent {
            id: "dup".to_string(),
            display_name: "D2".to_string(),
            agent_type: CustomAgentType::Command,
            command: "c2".to_string(),
            default_args: vec![],
            mode_args: None,
            skip_permissions_args: vec![],
            env: HashMap::new(),
        });
        let err = save_stored_custom_agents_to_path(&path, &[a1, a2]).unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn save_rejects_invalid_agent_id() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        let entry = StoredCustomAgent::new(CustomCodingAgent {
            id: "has spaces".to_string(),
            display_name: "X".to_string(),
            agent_type: CustomAgentType::Command,
            command: "cmd".to_string(),
            default_args: vec![],
            mode_args: None,
            skip_permissions_args: vec![],
            env: HashMap::new(),
        });
        let err = save_stored_custom_agents_to_path(&path, &[entry]).unwrap_err();
        assert!(err.contains("invalid"));
    }

    #[test]
    fn preset_roundtrip_preserves_all_thirteen_env_entries() {
        let dir = tempfile::tempdir().expect("temp config dir");
        let config_path = dir.path().join("config.toml");

        let preset = claude_code_openai_compat_preset(
            "claude-code-openai",
            "Claude Code (OpenAI-compat)",
            "http://192.168.100.166:32768",
            "sk-test-key",
            "openai/gpt-oss-20b",
        );
        let entries = vec![StoredCustomAgent::new(preset)];
        save_stored_custom_agents_to_path(&config_path, &entries).expect("save preset");

        let reloaded = load_custom_agents_from_path(&config_path).expect("reload");
        assert_eq!(reloaded.len(), 1);
        let agent = &reloaded[0];
        assert_eq!(agent.id, "claude-code-openai");
        assert_eq!(agent.agent_type, CustomAgentType::Bunx);
        assert_eq!(agent.command, "@anthropic-ai/claude-code@latest");
        assert_eq!(
            agent.skip_permissions_args,
            vec!["--dangerously-skip-permissions".to_string()]
        );
        assert_eq!(
            agent.env.len(),
            13,
            "all 13 preset env entries survive TOML round-trip"
        );
        assert_eq!(
            agent.env.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("sk-test-key")
        );
        assert_eq!(
            agent.env.get("ANTHROPIC_BASE_URL").map(String::as_str),
            Some("http://192.168.100.166:32768")
        );
        assert_eq!(
            agent
                .env
                .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
                .map(String::as_str),
            Some("openai/gpt-oss-20b")
        );
        assert_eq!(
            agent
                .env
                .get("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC")
                .map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn save_then_load_produces_camelcase_keys_on_disk() {
        let dir = tempfile::tempdir().expect("temp config dir");
        let config_path = dir.path().join("config.toml");

        let entry = StoredCustomAgent::new(CustomCodingAgent {
            id: "agent-x".to_string(),
            display_name: "Agent X".to_string(),
            agent_type: CustomAgentType::Bunx,
            command: "@foo/bar@latest".to_string(),
            default_args: vec!["--debug".to_string()],
            mode_args: None,
            skip_permissions_args: vec!["--yolo".to_string()],
            env: HashMap::from([("FOO".to_string(), "BAR".to_string())]),
        });
        save_stored_custom_agents_to_path(&config_path, &[entry]).expect("save");

        let content = std::fs::read_to_string(&config_path).expect("read");
        assert!(content.contains("displayName = \"Agent X\""));
        assert!(content.contains("agentType = \"bunx\""));
        assert!(content.contains("defaultArgs = [\"--debug\"]"));
        assert!(content.contains("skipPermissionsArgs = [\"--yolo\"]"));
    }

    #[test]
    fn save_with_empty_vec_removes_section_but_keeps_other_root_keys() {
        let dir = tempfile::tempdir().expect("temp config dir");
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
debug = true

[tools.customCodingAgents.to-remove]
id = "to-remove"
displayName = "Old"
agentType = "command"
command = "old-cli"
"#,
        )
        .expect("write");

        save_stored_custom_agents_to_path(&config_path, &[]).expect("save empty");

        let content = std::fs::read_to_string(&config_path).expect("read");
        assert!(content.contains("debug = true"));
        assert!(!content.contains("to-remove"));
        assert!(!content.contains("customCodingAgents"));
    }

    #[test]
    fn disable_env_is_documented() {
        // Not a behavior test per se; fails if the const is renamed or removed
        // since callers in the UI will rely on it verbatim.
        assert_eq!(
            DISABLE_GLOBAL_CUSTOM_AGENTS_ENV,
            "GWT_DISABLE_GLOBAL_CUSTOM_AGENTS"
        );
    }
}
