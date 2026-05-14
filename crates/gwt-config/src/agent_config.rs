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
    /// Explicit opt-in for pre-registering trust of gwt-generated Codex hooks.
    pub codex_trust_managed_hooks: Option<bool>,
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
        assert_eq!(c.codex_trust_managed_hooks, None);
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
            codex_trust_managed_hooks: Some(true),
        };
        let toml_str = toml::to_string_pretty(&c).unwrap();
        let loaded: AgentConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.default_agent, c.default_agent);
        assert_eq!(loaded.agent_paths.len(), 2);
        assert!(loaded.auto_install_deps);
        assert_eq!(loaded.codex_trust_managed_hooks, Some(true));
    }

    #[test]
    fn codex_trust_managed_hooks_is_explicit_opt_in() {
        let default_config = AgentConfig::default();
        assert_eq!(default_config.codex_trust_managed_hooks, None);

        let disabled: AgentConfig = toml::from_str("codex_trust_managed_hooks = false").unwrap();
        assert_eq!(disabled.codex_trust_managed_hooks, Some(false));

        let enabled: AgentConfig = toml::from_str("codex_trust_managed_hooks = true").unwrap();
        assert_eq!(enabled.codex_trust_managed_hooks, Some(true));
    }
}
