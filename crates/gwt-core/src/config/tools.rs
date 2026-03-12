//! Custom coding agent configuration management in `~/.gwt/config.toml`.

use super::settings::Settings;
use crate::config::migration::{ensure_config_dir, write_atomic};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info, warn};

/// Agent execution type
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentType {
    /// Execute via PATH search
    #[default]
    Command,
    /// Execute via absolute path
    Path,
    /// Execute via bunx
    Bunx,
}

/// Mode-specific arguments for different execution modes
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeArgs {
    /// Arguments for normal mode
    pub normal: Vec<String>,
    /// Arguments for continue mode
    #[serde(rename = "continue")]
    pub continue_mode: Vec<String>,
    /// Arguments for resume mode
    pub resume: Vec<String>,
}

/// Model definition for custom agents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDef {
    /// Model identifier
    pub id: String,
    /// Display label
    pub label: String,
    /// Command line argument for this model
    pub arg: String,
}

/// Custom coding agent definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCodingAgent {
    /// Unique identifier (alphanumeric and hyphens)
    pub id: String,
    /// Display name in UI
    #[serde(alias = "displayName")]
    pub display_name: String,
    /// Execution type
    #[serde(rename = "type")]
    pub agent_type: AgentType,
    /// Command or path to execute
    pub command: String,
    /// Default arguments
    #[serde(default, alias = "defaultArgs")]
    pub default_args: Vec<String>,
    /// Mode-specific arguments
    #[serde(default, alias = "modeArgs")]
    pub mode_args: Option<ModeArgs>,
    /// Arguments to skip permissions
    #[serde(default, alias = "permissionSkipArgs")]
    pub permission_skip_args: Vec<String>,
    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Available models
    #[serde(default)]
    pub models: Vec<ModelDef>,
    /// Command to get version
    #[serde(default, alias = "versionCommand")]
    pub version_command: Option<String>,
}

/// Tools configuration stored under `[tools]` in config.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Schema version (required)
    pub version: String,
    /// Custom coding agents
    #[serde(default, alias = "customCodingAgents")]
    pub custom_coding_agents: Vec<CustomCodingAgent>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self::empty()
    }
}

impl ToolsConfig {
    /// Create an empty configuration
    pub fn empty() -> Self {
        Self {
            version: "1.0.0".to_string(),
            custom_coding_agents: Vec::new(),
        }
    }

    /// Load global tools config from `~/.gwt/config.toml`.
    pub fn load_global() -> Option<Self> {
        Settings::load_global().ok().map(|settings| settings.tools)
    }

    /// Project-local custom agent files are no longer supported.
    pub fn load_local(_repo_root: &Path) -> Option<Self> {
        None
    }

    /// Load configuration from a TOML file.
    fn load_from_toml(path: &Path) -> Option<Self> {
        debug!(
            category = "config",
            path = %path.display(),
            "Loading tools config from TOML data"
        );

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    category = "config",
                    path = %path.display(),
                    error = %e,
                    "Failed to read tools config file"
                );
                return None;
            }
        };

        match toml::from_str::<ToolsConfig>(&content) {
            Ok(config) => {
                if config.version.is_empty() {
                    warn!(
                        category = "config",
                        path = %path.display(),
                        "Tools config missing version field, skipping"
                    );
                    return None;
                }
                debug!(
                    category = "config",
                    path = %path.display(),
                    version = %config.version,
                    agent_count = config.custom_coding_agents.len(),
                    "Loaded tools config"
                );
                Some(config)
            }
            Err(e) => {
                warn!(
                    category = "config",
                    path = %path.display(),
                    error = %e,
                    "Failed to parse tools config TOML"
                );
                None
            }
        }
    }

    /// Load configuration from a specific TOML path.
    #[allow(dead_code)]
    fn load_from_path(path: &Path) -> Option<Self> {
        Self::load_from_toml(path)
    }

    /// Load and merge global and local configurations
    /// Local configuration is no longer supported; only config.toml is used.
    pub fn load_merged(repo_root: &Path) -> Self {
        let global = Self::load_global();
        let local = Self::load_local(repo_root);

        Self::merge(global, local)
    }

    /// Merge two configurations, second takes priority for same IDs
    pub fn merge(first: Option<Self>, second: Option<Self>) -> Self {
        let mut agents: HashMap<String, CustomCodingAgent> = HashMap::new();

        // Add first config's agents
        if let Some(config) = first {
            for agent in config.custom_coding_agents {
                if Self::validate_agent(&agent) {
                    agents.insert(agent.id.clone(), agent);
                }
            }
        }

        // Add/override with second config's agents (local priority)
        if let Some(config) = second {
            for agent in config.custom_coding_agents {
                if Self::validate_agent(&agent) {
                    agents.insert(agent.id.clone(), agent);
                }
            }
        }

        Self {
            version: "1.0.0".to_string(),
            custom_coding_agents: agents.into_values().collect(),
        }
    }

    /// Validate a custom agent definition (FR-017)
    pub fn validate_agent(agent: &CustomCodingAgent) -> bool {
        // Check required fields
        if agent.id.is_empty() {
            warn!(category = "config", "Custom agent missing id, skipping");
            return false;
        }

        // Validate id format (alphanumeric and hyphens)
        if !agent.id.chars().all(|c| c.is_alphanumeric() || c == '-') {
            warn!(
                category = "config",
                id = %agent.id,
                "Custom agent id contains invalid characters, skipping"
            );
            return false;
        }

        if agent.display_name.is_empty() {
            warn!(
                category = "config",
                id = %agent.id,
                "Custom agent missing displayName, skipping"
            );
            return false;
        }

        if agent.command.is_empty() {
            warn!(
                category = "config",
                id = %agent.id,
                "Custom agent missing command, skipping"
            );
            return false;
        }

        true
    }

    /// Save configuration to a path in TOML format (gwt-spec issue FR-006)
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            ensure_config_dir(parent).map_err(|e| std::io::Error::other(e.to_string()))?;
        }

        let content = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        write_atomic(path, &content).map_err(|e| std::io::Error::other(e.to_string()))?;

        info!(
            category = "config",
            path = %path.display(),
            "Saved tools config (TOML)"
        );
        Ok(())
    }

    /// Save configuration into `~/.gwt/config.toml`.
    pub fn save_global(&self) -> std::io::Result<()> {
        Settings::update_global(|settings| {
            settings.tools = self.clone();
            Ok(())
        })
        .map_err(|e| std::io::Error::other(e.to_string()))
    }

    /// Project-local custom agent files are no longer supported.
    pub fn save_local(&self, _repo_root: &Path) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "project-local custom agent files are no longer supported; use save_global()",
        ))
    }

    /// Add a new agent
    pub fn add_agent(&mut self, agent: CustomCodingAgent) -> bool {
        if !Self::validate_agent(&agent) {
            return false;
        }

        // Check for duplicate ID
        if self.custom_coding_agents.iter().any(|a| a.id == agent.id) {
            warn!(
                category = "config",
                id = %agent.id,
                "Agent with this ID already exists"
            );
            return false;
        }

        self.custom_coding_agents.push(agent);
        true
    }

    /// Update an existing agent
    pub fn update_agent(&mut self, agent: CustomCodingAgent) -> bool {
        if !Self::validate_agent(&agent) {
            return false;
        }

        if let Some(existing) = self
            .custom_coding_agents
            .iter_mut()
            .find(|a| a.id == agent.id)
        {
            *existing = agent;
            true
        } else {
            warn!(
                category = "config",
                id = %agent.id,
                "Agent not found for update"
            );
            false
        }
    }

    /// Remove an agent by ID
    pub fn remove_agent(&mut self, id: &str) -> bool {
        let initial_len = self.custom_coding_agents.len();
        self.custom_coding_agents.retain(|a| a.id != id);
        self.custom_coding_agents.len() < initial_len
    }

    /// Get an agent by ID
    pub fn get_agent(&self, id: &str) -> Option<&CustomCodingAgent> {
        self.custom_coding_agents.iter().find(|a| a.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_tools_config_parse_toml() {
        let toml_str = r#"
version = "1.0.0"

[[custom_coding_agents]]
id = "test-agent"
display_name = "Test Agent"
type = "command"
command = "test-cmd"
"#;

        let config: ToolsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.version, "1.0.0");
        assert_eq!(config.custom_coding_agents.len(), 1);
        assert_eq!(config.custom_coding_agents[0].id, "test-agent");
        assert_eq!(config.custom_coding_agents[0].display_name, "Test Agent");
        assert_eq!(
            config.custom_coding_agents[0].agent_type,
            AgentType::Command
        );
    }

    #[test]
    fn test_tools_config_parse_all_fields() {
        let toml_str = r#"
version = "1.0.0"

[[custom_coding_agents]]
id = "aider"
display_name = "Aider"
type = "command"
command = "aider"
default_args = ["--no-git"]
permission_skip_args = ["--yes"]
version_command = "aider --version"

[custom_coding_agents.env]
OPENAI_API_KEY = "sk-test"

[custom_coding_agents.mode_args]
normal = []
continue = ["--resume"]
resume = ["--resume"]

[[custom_coding_agents.models]]
id = "gpt-4"
label = "GPT-4"
arg = "--model gpt-4"
"#;

        let config: ToolsConfig = toml::from_str(toml_str).unwrap();
        let agent = &config.custom_coding_agents[0];
        assert_eq!(agent.default_args, vec!["--no-git"]);
        assert_eq!(
            agent.mode_args.as_ref().unwrap().continue_mode,
            vec!["--resume"]
        );
        assert_eq!(agent.permission_skip_args, vec!["--yes"]);
        assert_eq!(
            agent.env.get("OPENAI_API_KEY"),
            Some(&"sk-test".to_string())
        );
        assert_eq!(agent.models.len(), 1);
        assert_eq!(agent.version_command, Some("aider --version".to_string()));
    }

    // T102: CustomCodingAgent validation test
    #[test]
    fn test_custom_agent_validation_valid() {
        let agent = CustomCodingAgent {
            id: "valid-agent".to_string(),
            display_name: "Valid Agent".to_string(),
            agent_type: AgentType::Command,
            command: "test".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        };
        assert!(ToolsConfig::validate_agent(&agent));
    }

    // T102: Validation fails for empty id
    #[test]
    fn test_custom_agent_validation_empty_id() {
        let agent = CustomCodingAgent {
            id: "".to_string(),
            display_name: "Test".to_string(),
            agent_type: AgentType::Command,
            command: "test".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        };
        assert!(!ToolsConfig::validate_agent(&agent));
    }

    // T102: Validation fails for invalid id characters
    #[test]
    fn test_custom_agent_validation_invalid_id() {
        let agent = CustomCodingAgent {
            id: "invalid agent".to_string(), // space is invalid
            display_name: "Test".to_string(),
            agent_type: AgentType::Command,
            command: "test".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        };
        assert!(!ToolsConfig::validate_agent(&agent));
    }

    // T102: Validation fails for empty display name
    #[test]
    fn test_custom_agent_validation_empty_display_name() {
        let agent = CustomCodingAgent {
            id: "test".to_string(),
            display_name: "".to_string(),
            agent_type: AgentType::Command,
            command: "test".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        };
        assert!(!ToolsConfig::validate_agent(&agent));
    }

    // T102: Validation fails for empty command
    #[test]
    fn test_custom_agent_validation_empty_command() {
        let agent = CustomCodingAgent {
            id: "test".to_string(),
            display_name: "Test".to_string(),
            agent_type: AgentType::Command,
            command: "".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        };
        assert!(!ToolsConfig::validate_agent(&agent));
    }

    #[test]
    fn test_merge_global_local() {
        let global = ToolsConfig {
            version: "1.0.0".to_string(),
            custom_coding_agents: vec![
                CustomCodingAgent {
                    id: "global-only".to_string(),
                    display_name: "Global Only".to_string(),
                    agent_type: AgentType::Command,
                    command: "global".to_string(),
                    default_args: vec![],
                    mode_args: None,
                    permission_skip_args: vec![],
                    env: HashMap::new(),
                    models: vec![],
                    version_command: None,
                },
                CustomCodingAgent {
                    id: "shared".to_string(),
                    display_name: "Global Shared".to_string(),
                    agent_type: AgentType::Command,
                    command: "global-shared".to_string(),
                    default_args: vec![],
                    mode_args: None,
                    permission_skip_args: vec![],
                    env: HashMap::new(),
                    models: vec![],
                    version_command: None,
                },
            ],
        };

        let local = ToolsConfig {
            version: "1.0.0".to_string(),
            custom_coding_agents: vec![
                CustomCodingAgent {
                    id: "local-only".to_string(),
                    display_name: "Local Only".to_string(),
                    agent_type: AgentType::Command,
                    command: "local".to_string(),
                    default_args: vec![],
                    mode_args: None,
                    permission_skip_args: vec![],
                    env: HashMap::new(),
                    models: vec![],
                    version_command: None,
                },
                CustomCodingAgent {
                    id: "shared".to_string(),
                    display_name: "Local Shared".to_string(), // Should override global
                    agent_type: AgentType::Path,
                    command: "local-shared".to_string(),
                    default_args: vec![],
                    mode_args: None,
                    permission_skip_args: vec![],
                    env: HashMap::new(),
                    models: vec![],
                    version_command: None,
                },
            ],
        };

        let merged = ToolsConfig::merge(Some(global), Some(local));
        assert_eq!(merged.custom_coding_agents.len(), 3);

        // Check local priority for shared ID
        let shared = merged.get_agent("shared").unwrap();
        assert_eq!(shared.display_name, "Local Shared");
        assert_eq!(shared.agent_type, AgentType::Path);
    }

    #[test]
    fn test_version_undefined_error() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        assert!(ToolsConfig::load_from_path(&path).is_none());
    }

    #[test]
    fn test_empty_version_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");
        std::fs::write(&path, "version = \"\"\n").unwrap();
        assert!(ToolsConfig::load_from_path(&path).is_none());
    }

    // T301: Save test (TOML format)
    #[test]
    fn test_tools_config_save_toml() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        let config = ToolsConfig {
            version: "1.0.0".to_string(),
            custom_coding_agents: vec![CustomCodingAgent {
                id: "test".to_string(),
                display_name: "Test".to_string(),
                agent_type: AgentType::Command,
                command: "test".to_string(),
                default_args: vec![],
                mode_args: None,
                permission_skip_args: vec![],
                env: HashMap::new(),
                models: vec![],
                version_command: None,
            }],
        };

        config.save(&path).unwrap();
        assert!(path.exists());

        // Verify content is TOML
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("version = \"1.0.0\""));
        assert!(content.contains("[[custom_coding_agents]]"));

        // Verify can be loaded
        let loaded = ToolsConfig::load_from_toml(&path).unwrap();
        assert_eq!(loaded.custom_coding_agents.len(), 1);
        assert_eq!(loaded.custom_coding_agents[0].id, "test");
    }

    #[test]
    fn test_save_global_uses_config_toml() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp_dir.path());

        let config = ToolsConfig {
            version: "1.0.0".to_string(),
            custom_coding_agents: vec![CustomCodingAgent {
                id: "global-agent".to_string(),
                display_name: "Global Agent".to_string(),
                agent_type: AgentType::Command,
                command: "global".to_string(),
                default_args: vec![],
                mode_args: None,
                permission_skip_args: vec![],
                env: HashMap::new(),
                models: vec![],
                version_command: None,
            }],
        };
        config.save_global().unwrap();

        let loaded = ToolsConfig::load_global().unwrap();
        assert_eq!(loaded.custom_coding_agents.len(), 1);
        assert_eq!(loaded.custom_coding_agents[0].id, "global-agent");

        let config_path = Settings::global_config_path().unwrap();
        let content = std::fs::read_to_string(config_path).unwrap();
        assert!(content.contains("[tools]"));
        assert!(content.contains("[[tools.custom_coding_agents]]"));
    }

    #[test]
    fn test_save_local_returns_explicit_error() {
        let temp_dir = TempDir::new().unwrap();
        let config = ToolsConfig::empty();

        let err = config.save_local(temp_dir.path()).unwrap_err();
        assert!(err
            .to_string()
            .contains("project-local custom agent files are no longer supported"));
    }

    // T302: Add/update/delete tests
    #[test]
    fn test_add_agent() {
        let mut config = ToolsConfig::empty();
        let agent = CustomCodingAgent {
            id: "new-agent".to_string(),
            display_name: "New Agent".to_string(),
            agent_type: AgentType::Bunx,
            command: "new-cmd".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        };

        assert!(config.add_agent(agent));
        assert_eq!(config.custom_coding_agents.len(), 1);
    }

    #[test]
    fn test_add_duplicate_agent_fails() {
        let mut config = ToolsConfig::empty();
        let agent = CustomCodingAgent {
            id: "dup".to_string(),
            display_name: "Dup".to_string(),
            agent_type: AgentType::Command,
            command: "cmd".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        };

        assert!(config.add_agent(agent.clone()));
        assert!(!config.add_agent(agent)); // Duplicate should fail
    }

    #[test]
    fn test_update_agent() {
        let mut config = ToolsConfig::empty();
        let agent = CustomCodingAgent {
            id: "upd".to_string(),
            display_name: "Original".to_string(),
            agent_type: AgentType::Command,
            command: "cmd".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        };
        config.add_agent(agent);

        let updated = CustomCodingAgent {
            id: "upd".to_string(),
            display_name: "Updated".to_string(),
            agent_type: AgentType::Path,
            command: "/new/path".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        };

        assert!(config.update_agent(updated));
        let agent = config.get_agent("upd").unwrap();
        assert_eq!(agent.display_name, "Updated");
        assert_eq!(agent.agent_type, AgentType::Path);
    }

    #[test]
    fn test_remove_agent() {
        let mut config = ToolsConfig::empty();
        let agent = CustomCodingAgent {
            id: "del".to_string(),
            display_name: "Delete Me".to_string(),
            agent_type: AgentType::Command,
            command: "cmd".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: HashMap::new(),
            models: vec![],
            version_command: None,
        };
        config.add_agent(agent);
        assert_eq!(config.custom_coding_agents.len(), 1);

        assert!(config.remove_agent("del"));
        assert_eq!(config.custom_coding_agents.len(), 0);

        // Remove non-existent should return false
        assert!(!config.remove_agent("nonexistent"));
    }

    // AgentType serialization test
    #[test]
    fn test_agent_type_serialization() {
        assert_eq!(
            serde_json::to_string(&AgentType::Command).unwrap(),
            "\"command\""
        );
        assert_eq!(serde_json::to_string(&AgentType::Path).unwrap(), "\"path\"");
        assert_eq!(serde_json::to_string(&AgentType::Bunx).unwrap(), "\"bunx\"");
    }
}
