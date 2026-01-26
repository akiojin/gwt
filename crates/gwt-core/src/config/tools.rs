//! Custom coding agent configuration management
//!
//! This module handles loading, validating, and managing custom coding agents
//! defined in tools.json files (global ~/.gwt/tools.json and local .gwt/tools.json).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

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
    #[serde(rename = "displayName")]
    pub display_name: String,
    /// Execution type
    #[serde(rename = "type")]
    pub agent_type: AgentType,
    /// Command or path to execute
    pub command: String,
    /// Default arguments
    #[serde(default, rename = "defaultArgs")]
    pub default_args: Vec<String>,
    /// Mode-specific arguments
    #[serde(default, rename = "modeArgs")]
    pub mode_args: Option<ModeArgs>,
    /// Arguments to skip permissions
    #[serde(default, rename = "permissionSkipArgs")]
    pub permission_skip_args: Vec<String>,
    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Available models
    #[serde(default)]
    pub models: Vec<ModelDef>,
    /// Command to get version
    #[serde(default, rename = "versionCommand")]
    pub version_command: Option<String>,
}

/// Tools configuration (tools.json structure)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Schema version (required)
    pub version: String,
    /// Custom coding agents
    #[serde(default, rename = "customCodingAgents")]
    pub custom_coding_agents: Vec<CustomCodingAgent>,
}

impl ToolsConfig {
    /// Create an empty configuration
    pub fn empty() -> Self {
        Self {
            version: "1.0.0".to_string(),
            custom_coding_agents: Vec::new(),
        }
    }

    /// Get global tools.json path (~/.gwt/tools.json)
    pub fn global_path() -> Option<PathBuf> {
        dirs::home_dir().map(|home| home.join(".gwt").join("tools.json"))
    }

    /// Get local tools.json path (.gwt/tools.json)
    pub fn local_path(repo_root: &Path) -> PathBuf {
        repo_root.join(".gwt").join("tools.json")
    }

    /// Load global tools.json
    pub fn load_global() -> Option<Self> {
        let path = Self::global_path()?;
        Self::load_from_path(&path)
    }

    /// Load local tools.json from repository root
    pub fn load_local(repo_root: &Path) -> Option<Self> {
        let path = Self::local_path(repo_root);
        Self::load_from_path(&path)
    }

    /// Load configuration from a specific path
    fn load_from_path(path: &Path) -> Option<Self> {
        if !path.exists() {
            debug!(
                category = "config",
                path = %path.display(),
                "tools.json not found"
            );
            return None;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    category = "config",
                    path = %path.display(),
                    error = %e,
                    "Failed to read tools.json"
                );
                return None;
            }
        };

        match serde_json::from_str::<ToolsConfig>(&content) {
            Ok(config) => {
                // Validate version field is present (FR-018)
                if config.version.is_empty() {
                    warn!(
                        category = "config",
                        path = %path.display(),
                        "tools.json missing version field, skipping"
                    );
                    return None;
                }
                debug!(
                    category = "config",
                    path = %path.display(),
                    version = %config.version,
                    agent_count = config.custom_coding_agents.len(),
                    "Loaded tools.json"
                );
                Some(config)
            }
            Err(e) => {
                warn!(
                    category = "config",
                    path = %path.display(),
                    error = %e,
                    "Failed to parse tools.json"
                );
                None
            }
        }
    }

    /// Load and merge global and local configurations
    /// Local configuration takes priority for same IDs (FR-003)
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

    /// Save configuration to a path
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;

        // Set file permissions to 600 on Unix (security for env vars)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms)?;
        }

        debug!(
            category = "config",
            path = %path.display(),
            "Saved tools.json"
        );
        Ok(())
    }

    /// Save configuration to global path (~/.gwt/tools.json)
    pub fn save_global(&self) -> std::io::Result<()> {
        if let Some(path) = Self::global_path() {
            self.save(&path)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine global tools.json path",
            ))
        }
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

    // T101: ToolsConfig parse test
    #[test]
    fn test_tools_config_parse() {
        let json = r#"{
            "version": "1.0.0",
            "customCodingAgents": [
                {
                    "id": "test-agent",
                    "displayName": "Test Agent",
                    "type": "command",
                    "command": "test-cmd"
                }
            ]
        }"#;

        let config: ToolsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.version, "1.0.0");
        assert_eq!(config.custom_coding_agents.len(), 1);
        assert_eq!(config.custom_coding_agents[0].id, "test-agent");
        assert_eq!(config.custom_coding_agents[0].display_name, "Test Agent");
        assert_eq!(
            config.custom_coding_agents[0].agent_type,
            AgentType::Command
        );
    }

    // T101: Parse with all fields
    #[test]
    fn test_tools_config_parse_all_fields() {
        let json = r#"{
            "version": "1.0.0",
            "customCodingAgents": [
                {
                    "id": "aider",
                    "displayName": "Aider",
                    "type": "command",
                    "command": "aider",
                    "defaultArgs": ["--no-git"],
                    "modeArgs": {
                        "normal": [],
                        "continue": ["--resume"],
                        "resume": ["--resume"]
                    },
                    "permissionSkipArgs": ["--yes"],
                    "env": {
                        "OPENAI_API_KEY": "sk-test"
                    },
                    "models": [
                        {"id": "gpt-4", "label": "GPT-4", "arg": "--model gpt-4"}
                    ],
                    "versionCommand": "aider --version"
                }
            ]
        }"#;

        let config: ToolsConfig = serde_json::from_str(json).unwrap();
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

    // T103: Global/local merge test
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

    // T104: Version undefined error test
    #[test]
    fn test_version_undefined_error() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("tools.json");

        // Write JSON without version field (simulated by empty version)
        let json = r#"{
            "customCodingAgents": []
        }"#;
        std::fs::write(&path, json).unwrap();

        // This should fail to parse because version is required
        let result = ToolsConfig::load_from_path(&path);
        // serde will fail because version is required (not Option)
        assert!(result.is_none());
    }

    // T104: Empty version should be rejected
    #[test]
    fn test_empty_version_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("tools.json");

        let json = r#"{
            "version": "",
            "customCodingAgents": []
        }"#;
        std::fs::write(&path, json).unwrap();

        let result = ToolsConfig::load_from_path(&path);
        assert!(result.is_none());
    }

    // T301: Save test
    #[test]
    fn test_tools_config_save() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("tools.json");

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

        // Verify content
        let loaded = ToolsConfig::load_from_path(&path).unwrap();
        assert_eq!(loaded.custom_coding_agents.len(), 1);
        assert_eq!(loaded.custom_coding_agents[0].id, "test");
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
