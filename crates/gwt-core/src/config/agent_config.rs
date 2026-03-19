//! Agent-specific configuration (global, not per-profile)
//!
//! Currently used for storing Claude Code provider settings (e.g. GLM/z.ai manual config).

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use super::settings::Settings;
use crate::error::Result;

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
    pub fn load() -> Result<Self> {
        debug!(category = "config", "Loading agent config from config.toml");
        Ok(Settings::load_global()?.agent_config)
    }

    pub fn save(&self) -> Result<()> {
        Settings::update_global(|settings| {
            settings.agent_config = self.clone();
            Ok(())
        })?;

        info!(
            category = "config",
            path = %Settings::global_config_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<unknown>".to_string()),
            "Saved agent config in config.toml"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn agent_config_default_roundtrip() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let cfg = AgentConfig::default();
        cfg.save().unwrap();

        let path = Settings::global_config_path().unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().ends_with("config.toml"));

        let loaded = AgentConfig::load().unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.claude.provider, ClaudeAgentProvider::Anthropic);
        assert_eq!(loaded.claude.glm.base_url, default_glm_base_url());
    }
}
