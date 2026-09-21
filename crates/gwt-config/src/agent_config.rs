//! Agent-related configuration.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Default grace, in seconds, between an Issue-linked Agent window's canonical
/// terminal settlement and its automatic close (Issue #3927 / SPEC #3340
/// FR-045).
pub const DEFAULT_TERMINAL_CLOSE_GRACE_SECS: u64 = 60;

/// Agent runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// SPEC #1921 Phase 86 (Issue #3813): scheduling / CPU / cargo budget
    /// applied to every AgentBootstrap process tree. Missing in older config
    /// files, which therefore load the enabled default.
    pub resource: AgentResourceConfig,
    /// Issue #3927 (SPEC #3340 FR-045): seconds the runtime waits after an
    /// Issue-linked Agent window becomes canonically terminal (settled
    /// execution, closed Issue, or revoked Monitor launch) before closing it.
    /// Missing in older config files, which load the 60-second default.
    pub terminal_close_grace_secs: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            default_agent: None,
            agent_paths: HashMap::new(),
            auto_install_deps: false,
            codex_trust_managed_hooks: None,
            resource: AgentResourceConfig::default(),
            terminal_close_grace_secs: DEFAULT_TERMINAL_CLOSE_GRACE_SECS,
        }
    }
}

/// Scheduling priority applied to an agent process tree relative to gwt.
///
/// Serialized names are stable (`normal` / `below-normal` / `idle`). Windows
/// maps them to process priority classes; Unix maps them to nice 0 / 10 / 19.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentProcessPriority {
    Normal,
    #[default]
    BelowNormal,
    Idle,
}

impl AgentProcessPriority {
    /// Every priority in ascending scheduling-weight order for UI selects.
    pub const ALL: [Self; 3] = [Self::Normal, Self::BelowNormal, Self::Idle];

    /// Stable serialized name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::BelowNormal => "below-normal",
            Self::Idle => "idle",
        }
    }

    /// Parse a serialized name, tolerating surrounding whitespace and case.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(Self::Normal),
            "below-normal" => Some(Self::BelowNormal),
            "idle" => Some(Self::Idle),
            _ => None,
        }
    }
}

/// Resource preset selected in Settings > System (SPEC #1921 Phase 86,
/// user feedback 2026-09-02). Presets describe the intent; the numeric budget
/// is derived at launch. `Custom` exposes the explicit fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentResourcePreset {
    /// Below-normal priority, CPU and build parallelism divided by the
    /// repository's max active agents.
    #[default]
    Automatic,
    /// Idle priority and half of the automatic CPU / parallelism budget.
    GuiResponsiveness,
    /// Below-normal priority with no CPU cap and full parallelism.
    BuildSpeed,
    /// Explicit priority / CPU limit / build parallelism.
    Custom,
}

impl AgentResourcePreset {
    /// Every preset in UI order.
    pub const ALL: [Self; 4] = [
        Self::Automatic,
        Self::GuiResponsiveness,
        Self::BuildSpeed,
        Self::Custom,
    ];

    /// Stable serialized name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::GuiResponsiveness => "gui-responsiveness",
            Self::BuildSpeed => "build-speed",
            Self::Custom => "custom",
        }
    }

    /// Parse a serialized name, tolerating surrounding whitespace and case.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "automatic" => Some(Self::Automatic),
            "gui-responsiveness" => Some(Self::GuiResponsiveness),
            "build-speed" => Some(Self::BuildSpeed),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

/// Validation failure for [`AgentResourceConfig`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentResourceConfigError {
    #[error("agent CPU limit must be between 1 and 100 percent, got {0}")]
    CpuLimitOutOfRange(u8),
    #[error("agent build parallelism must be at least 1")]
    BuildJobsZero,
}

/// Agent process-tree resource policy (SPEC #1921 Phase 86 FR-235).
///
/// `priority`, `cpu_limit_percent`, and `build_jobs` only take effect with
/// the `Custom` preset; `None` numeric values there mean automatic derivation
/// from the repository's Issue Monitor `max_active` and the host logical-core
/// count. Zero is rejected by [`Self::validate`] rather than treated as a
/// sentinel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentResourceConfig {
    /// Master switch. Disabled means no priority, CPU cap, or parallelism
    /// change.
    pub enabled: bool,
    /// Intent-level preset; `Custom` enables the explicit fields below.
    pub preset: AgentResourcePreset,
    /// Scheduling priority for the whole agent process tree (Custom).
    pub priority: AgentProcessPriority,
    /// Windows Job CPU hard cap in percent (1..=100); `None` is automatic
    /// (Custom).
    pub cpu_limit_percent: Option<u8>,
    /// Build parallelism per agent, handed to every build tool that reads a
    /// job count from the environment (`CARGO_BUILD_JOBS`, `MAKEFLAGS`, ...);
    /// `None` is automatic (Custom).
    pub build_jobs: Option<u32>,
}

impl Default for AgentResourceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            preset: AgentResourcePreset::Automatic,
            priority: AgentProcessPriority::BelowNormal,
            cpu_limit_percent: None,
            build_jobs: None,
        }
    }
}

impl AgentResourceConfig {
    /// Reject out-of-range explicit values before they are persisted or
    /// applied to a launch.
    pub fn validate(&self) -> Result<(), AgentResourceConfigError> {
        if let Some(percent) = self.cpu_limit_percent {
            if !(1..=100).contains(&percent) {
                return Err(AgentResourceConfigError::CpuLimitOutOfRange(percent));
            }
        }
        if self.build_jobs == Some(0) {
            return Err(AgentResourceConfigError::BuildJobsZero);
        }
        Ok(())
    }
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
            resource: AgentResourceConfig::default(),
            terminal_close_grace_secs: DEFAULT_TERMINAL_CLOSE_GRACE_SECS,
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

    // Issue #3927 (SPEC #3340 T-623 / FR-045): the terminal close grace is
    // configurable in Agent settings, defaults to 60 seconds, and older config
    // files without the field load that default.
    #[test]
    fn terminal_close_grace_defaults_to_sixty_seconds() {
        assert_eq!(AgentConfig::default().terminal_close_grace_secs, 60);
        let missing: AgentConfig = toml::from_str("auto_install_deps = false").unwrap();
        assert_eq!(missing.terminal_close_grace_secs, 60);
    }

    #[test]
    fn terminal_close_grace_override_roundtrips() {
        let overridden: AgentConfig = toml::from_str("terminal_close_grace_secs = 5").unwrap();
        assert_eq!(overridden.terminal_close_grace_secs, 5);
        let rendered = toml::to_string_pretty(&overridden).unwrap();
        let reloaded: AgentConfig = toml::from_str(&rendered).unwrap();
        assert_eq!(reloaded.terminal_close_grace_secs, 5);
    }

    #[test]
    fn resource_policy_defaults_to_enabled_automatic_preset() {
        let config = AgentConfig::default();
        assert!(config.resource.enabled);
        assert_eq!(config.resource.preset, AgentResourcePreset::Automatic);
        assert_eq!(config.resource.priority, AgentProcessPriority::BelowNormal);
        assert_eq!(config.resource.cpu_limit_percent, None);
        assert_eq!(config.resource.build_jobs, None);
    }

    #[test]
    fn resource_preset_names_are_stable_and_parse_case_insensitively() {
        assert_eq!(AgentResourcePreset::Automatic.as_str(), "automatic");
        assert_eq!(
            AgentResourcePreset::GuiResponsiveness.as_str(),
            "gui-responsiveness"
        );
        assert_eq!(AgentResourcePreset::BuildSpeed.as_str(), "build-speed");
        assert_eq!(AgentResourcePreset::Custom.as_str(), "custom");
        assert_eq!(
            AgentResourcePreset::parse(" Build-Speed "),
            Some(AgentResourcePreset::BuildSpeed)
        );
        assert_eq!(AgentResourcePreset::parse("turbo"), None);
        let loaded: AgentResourceConfig =
            toml::from_str("preset = \"gui-responsiveness\"\n").unwrap();
        assert_eq!(loaded.preset, AgentResourcePreset::GuiResponsiveness);
    }

    #[test]
    fn old_agent_config_without_resource_section_loads_default_policy() {
        let loaded: AgentConfig = toml::from_str(
            "default_agent = \"claude\"
auto_install_deps = true
",
        )
        .unwrap();
        assert_eq!(loaded.default_agent.as_deref(), Some("claude"));
        assert_eq!(loaded.resource, AgentResourceConfig::default());
    }

    #[test]
    fn resource_policy_roundtrips_priority_names_and_optional_limits() {
        let config = AgentConfig {
            resource: AgentResourceConfig {
                enabled: true,
                preset: AgentResourcePreset::Custom,
                priority: AgentProcessPriority::Idle,
                cpu_limit_percent: Some(40),
                build_jobs: Some(3),
            },
            ..AgentConfig::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("priority = \"idle\""), "{toml_str}");
        assert!(toml_str.contains("preset = \"custom\""), "{toml_str}");
        assert!(toml_str.contains("build_jobs = 3"), "{toml_str}");
        let loaded: AgentConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.resource, config.resource);

        let automatic: AgentConfig = toml::from_str(
            "[resource]
enabled = false
priority = \"normal\"
",
        )
        .unwrap();
        assert!(!automatic.resource.enabled);
        assert_eq!(automatic.resource.preset, AgentResourcePreset::Automatic);
        assert_eq!(automatic.resource.priority, AgentProcessPriority::Normal);
        assert_eq!(automatic.resource.cpu_limit_percent, None);
        assert_eq!(automatic.resource.build_jobs, None);
    }

    #[test]
    fn resource_policy_validation_rejects_zero_and_out_of_range_values() {
        let valid = AgentResourceConfig {
            cpu_limit_percent: Some(1),
            build_jobs: Some(1),
            ..AgentResourceConfig::default()
        };
        assert!(valid.validate().is_ok());
        let full = AgentResourceConfig {
            cpu_limit_percent: Some(100),
            ..AgentResourceConfig::default()
        };
        assert!(full.validate().is_ok());

        for cpu in [0, 101, 255] {
            let invalid = AgentResourceConfig {
                cpu_limit_percent: Some(cpu),
                ..AgentResourceConfig::default()
            };
            assert!(invalid.validate().is_err(), "cpu {cpu} must be rejected");
        }
        let zero_jobs = AgentResourceConfig {
            build_jobs: Some(0),
            ..AgentResourceConfig::default()
        };
        assert!(zero_jobs.validate().is_err());
    }

    #[test]
    fn priority_names_are_stable_and_parse_case_insensitively() {
        assert_eq!(AgentProcessPriority::Normal.as_str(), "normal");
        assert_eq!(AgentProcessPriority::BelowNormal.as_str(), "below-normal");
        assert_eq!(AgentProcessPriority::Idle.as_str(), "idle");
        assert_eq!(
            AgentProcessPriority::parse(" Below-Normal "),
            Some(AgentProcessPriority::BelowNormal)
        );
        assert_eq!(
            AgentProcessPriority::parse("idle"),
            Some(AgentProcessPriority::Idle)
        );
        assert_eq!(AgentProcessPriority::parse("high"), None);
    }
}
