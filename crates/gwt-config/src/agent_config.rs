//! Agent-related configuration.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Agent runtime configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Default agent identifier (e.g. "claude", "codex", "gemini").
    pub default_agent: Option<String>,
    /// Named agent executable paths.
    pub agent_paths: HashMap<String, PathBuf>,
    /// Auto-install agent dependencies before launch.
    pub auto_install_deps: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_no_agent() {
        let c = AgentConfig::default();
        assert!(c.default_agent.is_none());
        assert!(c.agent_paths.is_empty());
        assert!(!c.auto_install_deps);
    }

    #[test]
    fn roundtrip_toml() {
        let mut paths = HashMap::new();
        paths.insert("claude".to_string(), PathBuf::from("/usr/bin/claude"));
        paths.insert("codex".to_string(), PathBuf::from("/usr/bin/codex"));

        let c = AgentConfig {
            default_agent: Some("claude".to_string()),
            agent_paths: paths,
            auto_install_deps: true,
        };
        let toml_str = toml::to_string_pretty(&c).unwrap();
        let loaded: AgentConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.default_agent, c.default_agent);
        assert_eq!(loaded.agent_paths.len(), 2);
        assert!(loaded.auto_install_deps);
    }
}
