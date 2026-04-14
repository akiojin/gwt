use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gwt_agent::{
    custom::{CustomAgentType, ModeArgs},
    CustomCodingAgent,
};
use gwt_config::Settings;
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

pub(crate) const DISABLE_GLOBAL_CUSTOM_AGENTS_ENV: &str = "GWT_TUI_DISABLE_GLOBAL_CUSTOM_AGENTS";

#[derive(Debug, Clone)]
pub(crate) struct StoredCustomAgent {
    pub(crate) agent: CustomCodingAgent,
    raw: Table,
}

impl StoredCustomAgent {
    pub(crate) fn new(agent: CustomCodingAgent) -> Self {
        Self {
            agent,
            raw: Table::new(),
        }
    }
}

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

pub(crate) fn load_custom_agents() -> Vec<CustomCodingAgent> {
    if std::env::var_os(DISABLE_GLOBAL_CUSTOM_AGENTS_ENV).is_some() {
        return Vec::new();
    }

    let Some(path) = Settings::global_config_path() else {
        return Vec::new();
    };

    load_custom_agents_from_path(&path).unwrap_or_else(|err| {
        tracing::warn!(path = %path.display(), error = %err, "failed to load custom agents");
        Vec::new()
    })
}

pub(crate) fn load_custom_agents_from_path(path: &Path) -> Result<Vec<CustomCodingAgent>, String> {
    Ok(load_stored_custom_agents_from_path(path)?
        .into_iter()
        .map(|entry| entry.agent)
        .collect())
}

pub(crate) fn load_stored_custom_agents_from_path(
    path: &Path,
) -> Result<Vec<StoredCustomAgent>, String> {
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
            tracing::warn!(custom_agent = %key, "skipping non-table custom agent entry");
            continue;
        };
        let parsed: CustomAgentToml = match raw_value.clone().try_into() {
            Ok(parsed) => parsed,
            Err(err) => {
                tracing::warn!(custom_agent = %key, error = %err, "skipping unparsable custom agent");
                continue;
            }
        };
        if let Some(agent) = parsed.into_custom_agent(key) {
            agents.push(StoredCustomAgent {
                agent,
                raw: raw_table.clone(),
            });
        } else {
            tracing::warn!(custom_agent = %key, "skipping invalid custom agent");
        }
    }

    Ok(agents)
}

pub(crate) fn save_stored_custom_agents_to_path(
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
    write_atomic(path, &content)
}

fn validate_unique_custom_agents(agents: &[StoredCustomAgent]) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
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

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create config dir {}: {err}", parent.display()))?;
    }

    let temp_path = temp_path_for(path);
    std::fs::write(&temp_path, content)
        .map_err(|err| format!("failed to write temp config {}: {err}", temp_path.display()))?;
    std::fs::rename(&temp_path, path).map_err(|err| {
        format!(
            "failed to replace config {} with {}: {err}",
            path.display(),
            temp_path.display()
        )
    })
}

fn temp_path_for(path: &Path) -> PathBuf {
    let suffix = format!(
        "{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    );
    path.with_file_name(format!(
        ".{}.tmp.{suffix}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_custom_agents_from_path_parses_spec_schema() {
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
    fn save_stored_custom_agents_to_path_preserves_models_and_other_settings() {
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
}
