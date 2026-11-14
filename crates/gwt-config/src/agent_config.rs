//! Agent-related configuration.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Cross-platform scheduling class applied to an agent process tree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProcessPriority {
    Normal,
    #[default]
    BelowNormal,
    Idle,
}

/// Resource limits applied to agent process trees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentResourceConfig {
    /// Apply scheduling, CPU, and cargo limits to agent launches.
    pub enabled: bool,
    /// Scheduling class inherited by agent descendants.
    pub priority: AgentProcessPriority,
    /// Windows Job Object CPU hard cap. `None` derives from `max_active`.
    pub cpu_limit_percent: Option<u8>,
    /// Cargo parallelism. `None` derives from logical cores and `max_active`.
    pub cargo_build_jobs: Option<usize>,
}

impl Default for AgentResourceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            priority: AgentProcessPriority::BelowNormal,
            cpu_limit_percent: None,
            cargo_build_jobs: None,
        }
    }
}

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
    /// Optional override for pre-registering trust of gwt-generated Codex hooks.
    /// `None` and `Some(true)` enable trust; `Some(false)` is the opt-out.
    pub codex_trust_managed_hooks: Option<bool>,
    /// Scheduling and CPU policy for agent process trees.
    pub resources: AgentResourceConfig,
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
            resources: AgentResourceConfig::default(),
        };
        let toml_str = toml::to_string_pretty(&c).unwrap();
        let loaded: AgentConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.default_agent, c.default_agent);
        assert_eq!(loaded.agent_paths.len(), 2);
        assert!(loaded.auto_install_deps);
        assert_eq!(loaded.codex_trust_managed_hooks, Some(true));
    }

    #[test]
    fn codex_trust_managed_hooks_is_false_only_opt_out() {
        let default_config = AgentConfig::default();
        assert_eq!(default_config.codex_trust_managed_hooks, None);

        let disabled: AgentConfig = toml::from_str("codex_trust_managed_hooks = false").unwrap();
        assert_eq!(disabled.codex_trust_managed_hooks, Some(false));

        let enabled: AgentConfig = toml::from_str("codex_trust_managed_hooks = true").unwrap();
        assert_eq!(enabled.codex_trust_managed_hooks, Some(true));
    }

    #[test]
    fn resource_policy_defaults_enabled_and_roundtrips_explicit_values() {
        let default_config = AgentConfig::default();
        assert!(default_config.resources.enabled);
        assert_eq!(
            default_config.resources.priority,
            AgentProcessPriority::BelowNormal
        );
        assert_eq!(default_config.resources.cpu_limit_percent, None);
        assert_eq!(default_config.resources.cargo_build_jobs, None);

        let configured: AgentConfig = toml::from_str(
            r#"
[resources]
enabled = true
priority = "idle"
cpu_limit_percent = 40
cargo_build_jobs = 3
"#,
        )
        .expect("explicit resource policy parses");
        assert_eq!(configured.resources.priority, AgentProcessPriority::Idle);
        assert_eq!(configured.resources.cpu_limit_percent, Some(40));
        assert_eq!(configured.resources.cargo_build_jobs, Some(3));

        let serialized = toml::to_string_pretty(&configured).expect("serialize resource policy");
        let roundtrip: AgentConfig =
            toml::from_str(&serialized).expect("roundtrip resource policy");
        assert_eq!(roundtrip.resources, configured.resources);
    }
}
