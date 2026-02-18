//! Agent-specific configuration (global, not per-profile)
//!
//! Currently used for storing Claude Code provider settings (e.g. GLM/z.ai manual config).

use crate::config::migration::{backup_broken_file, ensure_config_dir, write_atomic};
use crate::error::{GwtError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

fn default_glm_base_url() -> String {
    // z.ai DevPack manual configuration
    "https://api.z.ai/api/anthropic".to_string()
}

fn default_glm_timeout_ms() -> String {
    // Example value from the z.ai guide. Users can override or clear it.
    "3000000".to_string()
}

fn default_glm_opus_model() -> String {
    "glm-4.7".to_string()
}

fn default_glm_sonnet_model() -> String {
    "glm-4.7".to_string()
}

fn default_glm_haiku_model() -> String {
    "glm-4.5-air".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaudeAgentProvider {
    Anthropic,
    Glm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeGlmConfig {
    #[serde(default = "default_glm_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub auth_token: String,
    /// Passed as `API_TIMEOUT_MS` when non-empty.
    #[serde(default = "default_glm_timeout_ms")]
    pub api_timeout_ms: String,
    #[serde(default = "default_glm_opus_model")]
    pub default_opus_model: String,
    #[serde(default = "default_glm_sonnet_model")]
    pub default_sonnet_model: String,
    #[serde(default = "default_glm_haiku_model")]
    pub default_haiku_model: String,
}

impl Default for ClaudeGlmConfig {
    fn default() -> Self {
        Self {
            base_url: default_glm_base_url(),
            auth_token: String::new(),
            api_timeout_ms: default_glm_timeout_ms(),
            default_opus_model: default_glm_opus_model(),
            default_sonnet_model: default_glm_sonnet_model(),
            default_haiku_model: default_glm_haiku_model(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeAgentConfig {
    pub provider: ClaudeAgentProvider,
    pub glm: ClaudeGlmConfig,
}

impl Default for ClaudeAgentConfig {
    fn default() -> Self {
        Self {
            provider: ClaudeAgentProvider::Anthropic,
            glm: ClaudeGlmConfig::default(),
        }
    }
}

/// Agent configuration stored on disk
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Schema version
    pub version: u8,
    pub claude: ClaudeAgentConfig,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            version: 1,
            claude: ClaudeAgentConfig::default(),
        }
    }
}

impl AgentConfig {
    /// TOML config file path (~/.gwt/agents.toml)
    pub fn toml_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".gwt").join("agents.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::toml_path();

        if !path.exists() {
            return Ok(Self::default());
        }

        debug!(
            category = "config",
            path = %path.display(),
            "Loading agent config (TOML)"
        );

        match Self::load_toml(&path) {
            Ok(config) => Ok(config),
            Err(e) => {
                warn!(
                    category = "config",
                    path = %path.display(),
                    error = %e,
                    "Failed to load agent config; falling back to defaults"
                );
                let _ = backup_broken_file(&path);
                Ok(Self::default())
            }
        }
    }

    fn load_toml(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let cfg: AgentConfig =
            toml::from_str(&content).map_err(|e| GwtError::ConfigParseError {
                reason: format!("Failed to parse TOML: {}", e),
            })?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::toml_path();

        if let Some(parent) = path.parent() {
            ensure_config_dir(parent)?;
        }

        let content = toml::to_string_pretty(self).map_err(|e| GwtError::ConfigWriteError {
            reason: format!("Failed to serialize to TOML: {}", e),
        })?;

        write_atomic(&path, &content)?;

        info!(
            category = "config",
            path = %path.display(),
            "Saved agent config (TOML)"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn agent_config_default_roundtrip() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let cfg = AgentConfig::default();
        cfg.save().unwrap();

        let path = AgentConfig::toml_path();
        assert!(path.exists());
        assert!(path.to_string_lossy().ends_with("agents.toml"));

        let loaded = AgentConfig::load().unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.claude.provider, ClaudeAgentProvider::Anthropic);
        assert_eq!(loaded.claude.glm.base_url, default_glm_base_url());
    }

    #[test]
    fn agent_config_backs_up_broken_file() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let path = AgentConfig::toml_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, "this is not toml ==").unwrap();

        let loaded = AgentConfig::load().unwrap();
        assert_eq!(loaded.version, 1);

        let broken = path.with_extension("broken");
        assert!(broken.exists());
        assert!(!path.exists());
    }
}
