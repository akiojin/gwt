//! Settings management (gwt-spec issue)
//!
//! Manages application settings.
//!
//! Global config: `~/.gwt/config.toml`

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{
    de::{self, Deserializer},
    Deserialize, Serialize,
};
use tracing::{debug, error, info, instrument};

use super::{
    agent_config::AgentConfig,
    migration::{ensure_config_dir, write_atomic},
    profile::{Profile, ProfilesConfig},
    recent_projects::RecentProjectsConfig,
    tools::ToolsConfig,
};
use crate::error::{GwtError, Result};

static GLOBAL_SETTINGS_UPDATE_LOCK: Mutex<()> = Mutex::new(());

/// Runtime application settings assembled from config files and env overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Protected branches that cannot be deleted
    pub protected_branches: Vec<String>,
    /// Default base branch for new worktrees
    pub default_base_branch: String,
    /// Worktree root directory (relative to repo root)
    pub worktree_root: String,
    /// Enable debug logging
    pub debug: bool,
    /// Enable performance profiling (Chrome Trace output)
    #[serde(default)]
    pub profiling: bool,
    /// Log directory path
    pub log_dir: Option<PathBuf>,
    /// Log retention days
    pub log_retention_days: u32,
    /// Agent settings
    pub agent: AgentSettings,
    /// Docker settings
    pub docker: DockerSettings,
    /// Appearance settings
    pub appearance: AppearanceSettings,
    /// Preferred summary language ("auto" | "ja" | "en")
    pub app_language: String,
    /// Voice input settings
    pub voice_input: VoiceInputSettings,
    /// Terminal settings
    pub terminal: TerminalSettings,
    /// Global profiles configuration.
    #[serde(default)]
    pub profiles: ProfilesConfig,
    /// Agent-specific runtime preferences stored in config.toml.
    #[serde(default)]
    pub agent_config: AgentConfig,
    /// Custom coding agent definitions stored in config.toml.
    #[serde(default)]
    pub tools: ToolsConfig,
    /// Recent project history stored in config.toml.
    #[serde(default)]
    pub recent_projects: RecentProjectsConfig,
}

/// TOML DTO for the `[profiles]` section inside `config.toml`.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(default)]
struct ProfilesSectionToml {
    version: u8,
    active: Option<String>,
    #[serde(flatten)]
    profiles: std::collections::HashMap<String, Profile>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ProfilesSectionTomlRaw {
    version: u8,
    active: Option<String>,
    #[serde(flatten)]
    entries: toml::value::Table,
}

impl<'de> Deserialize<'de> for ProfilesSectionToml {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = ProfilesSectionTomlRaw::deserialize(deserializer)?;
        let mut profiles = std::collections::HashMap::new();

        for (key, value) in raw.entries {
            if key == "profiles" {
                let legacy_profiles = value.as_table().ok_or_else(|| {
                    de::Error::custom(
                        "profiles.profiles must be a table when loading legacy config",
                    )
                })?;

                for (legacy_key, legacy_value) in legacy_profiles {
                    insert_profile_entry::<D::Error>(
                        &mut profiles,
                        legacy_key.to_string(),
                        legacy_value.clone(),
                    )?;
                }
                continue;
            }

            insert_profile_entry::<D::Error>(&mut profiles, key, value)?;
        }

        Ok(Self {
            version: raw.version,
            active: raw.active,
            profiles,
        })
    }
}

fn insert_profile_entry<E>(
    profiles: &mut std::collections::HashMap<String, Profile>,
    key: String,
    value: toml::Value,
) -> std::result::Result<(), E>
where
    E: de::Error,
{
    if profiles.contains_key(&key) {
        return Err(E::custom(format!(
            "duplicate profile entry found while loading config.toml: {key}"
        )));
    }

    let profile = value
        .try_into()
        .map_err(|err| E::custom(format!("invalid profile entry `{key}`: {err}")))?;
    profiles.insert(key, profile);
    Ok(())
}

impl From<ProfilesConfig> for ProfilesSectionToml {
    fn from(value: ProfilesConfig) -> Self {
        Self {
            version: value.version,
            active: value.active,
            profiles: value.profiles,
        }
    }
}

impl From<ProfilesSectionToml> for ProfilesConfig {
    fn from(value: ProfilesSectionToml) -> Self {
        Self {
            version: value.version,
            active: value.active,
            profiles: value.profiles,
        }
    }
}

/// TOML DTO for the canonical `~/.gwt/config.toml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct ConfigToml {
    protected_branches: Vec<String>,
    default_base_branch: String,
    worktree_root: String,
    debug: bool,
    #[serde(default)]
    profiling: bool,
    log_dir: Option<PathBuf>,
    log_retention_days: u32,
    agent: AgentSettings,
    docker: DockerSettings,
    appearance: AppearanceSettings,
    app_language: String,
    voice_input: VoiceInputSettings,
    terminal: TerminalSettings,
    profiles: ProfilesSectionToml,
    agent_config: AgentConfig,
    tools: ToolsConfig,
    recent_projects: RecentProjectsConfig,
}

impl Default for ConfigToml {
    fn default() -> Self {
        Settings::default().into()
    }
}

/// TOML DTO for repo-local config files that must not include global-only sections.
#[derive(Debug, Clone, Serialize)]
struct LocalConfigToml {
    protected_branches: Vec<String>,
    default_base_branch: String,
    worktree_root: String,
    debug: bool,
    profiling: bool,
    log_dir: Option<PathBuf>,
    log_retention_days: u32,
    agent: AgentSettings,
    docker: DockerSettings,
    appearance: AppearanceSettings,
    app_language: String,
    voice_input: VoiceInputSettings,
    terminal: TerminalSettings,
}

impl From<Settings> for ConfigToml {
    fn from(value: Settings) -> Self {
        Self {
            protected_branches: value.protected_branches,
            default_base_branch: value.default_base_branch,
            worktree_root: value.worktree_root,
            debug: value.debug,
            profiling: value.profiling,
            log_dir: value.log_dir,
            log_retention_days: value.log_retention_days,
            agent: value.agent,
            docker: value.docker,
            appearance: value.appearance,
            app_language: value.app_language,
            voice_input: value.voice_input,
            terminal: value.terminal,
            profiles: value.profiles.into(),
            agent_config: value.agent_config,
            tools: value.tools,
            recent_projects: value.recent_projects,
        }
    }
}

impl From<Settings> for LocalConfigToml {
    fn from(value: Settings) -> Self {
        Self {
            protected_branches: value.protected_branches,
            default_base_branch: value.default_base_branch,
            worktree_root: value.worktree_root,
            debug: value.debug,
            profiling: value.profiling,
            log_dir: value.log_dir,
            log_retention_days: value.log_retention_days,
            agent: value.agent,
            docker: value.docker,
            appearance: value.appearance,
            app_language: value.app_language,
            voice_input: value.voice_input,
            terminal: value.terminal,
        }
    }
}

impl From<ConfigToml> for Settings {
    fn from(value: ConfigToml) -> Self {
        Self {
            protected_branches: value.protected_branches,
            default_base_branch: value.default_base_branch,
            worktree_root: value.worktree_root,
            debug: value.debug,
            profiling: value.profiling,
            log_dir: value.log_dir,
            log_retention_days: value.log_retention_days,
            agent: value.agent,
            docker: value.docker,
            appearance: value.appearance,
            app_language: value.app_language,
            voice_input: value.voice_input,
            terminal: value.terminal,
            profiles: value.profiles.into(),
            agent_config: value.agent_config,
            tools: value.tools,
            recent_projects: value.recent_projects,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            protected_branches: vec![
                "main".to_string(),
                "master".to_string(),
                "develop".to_string(),
            ],
            default_base_branch: "main".to_string(),
            worktree_root: ".worktrees".to_string(),
            debug: false,
            profiling: false,
            log_dir: None,
            log_retention_days: 7,
            agent: AgentSettings::default(),
            docker: DockerSettings::default(),
            appearance: AppearanceSettings::default(),
            app_language: "auto".to_string(),
            voice_input: VoiceInputSettings::default(),
            terminal: TerminalSettings::default(),
            profiles: ProfilesConfig::default(),
            agent_config: AgentConfig::default(),
            tools: ToolsConfig::default(),
            recent_projects: RecentProjectsConfig::default(),
        }
    }
}

/// Agent settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// Default agent to use
    pub default_agent: Option<String>,
    /// Claude Code path
    pub claude_path: Option<PathBuf>,
    /// Codex CLI path
    pub codex_path: Option<PathBuf>,
    /// Gemini CLI path
    pub gemini_path: Option<PathBuf>,
    /// Auto install dependencies before launching agent
    pub auto_install_deps: bool,
    /// Default GitHub Project V2 ID for issue-first spec sync.
    pub github_project_id: Option<String>,
    /// Skill / plugin registration scope preferences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_registration: Option<SkillRegistrationPreferences>,
}

fn default_skill_registration_enabled() -> bool {
    true
}

/// Preferences for managed skill / plugin registration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SkillRegistrationPreferences {
    /// Whether project-local managed assets are automatically repaired and refreshed.
    #[serde(default = "default_skill_registration_enabled")]
    pub enabled: bool,
    /// Inject managed skills block into CLAUDE.md (default: true).
    #[serde(default = "default_inject_claude_md")]
    pub inject_claude_md: bool,
    /// Inject managed skills block into AGENTS.md (default: false).
    #[serde(default)]
    pub inject_agents_md: bool,
    /// Inject managed skills block into GEMINI.md (default: false).
    #[serde(default)]
    pub inject_gemini_md: bool,
}

fn default_inject_claude_md() -> bool {
    true
}

impl Default for SkillRegistrationPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            inject_claude_md: true,
            inject_agents_md: false,
            inject_gemini_md: false,
        }
    }
}

/// Docker settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DockerSettings {
    /// Force host launch (skip docker) even when Docker files are detected
    pub force_host: bool,
}

/// Terminal settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    /// Default shell program (None = use system default)
    pub default_shell: Option<String>,
}

/// Appearance settings (font sizes)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    /// UI font size in pixels (8-24, default 13)
    pub ui_font_size: u32,
    /// Terminal font size in pixels (8-24, default 13)
    pub terminal_font_size: u32,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            ui_font_size: 13,
            terminal_font_size: 13,
        }
    }
}

/// Voice input settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceInputSettings {
    /// Enable voice input support
    pub enabled: bool,
    /// Voice backend engine (currently "qwen3-asr")
    pub engine: String,
    /// Recognition language ("auto" | "ja" | "en")
    pub language: String,
    /// Quality preset ("fast" | "balanced" | "accurate")
    pub quality: String,
    /// Local STT model tier hint
    pub model: String,
}

impl Default for VoiceInputSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: "qwen3-asr".to_string(),
            language: "auto".to_string(),
            quality: "balanced".to_string(),
            model: "Qwen/Qwen3-ASR-1.7B".to_string(),
        }
    }
}

impl Settings {
    fn is_global_config_path(path: &Path) -> bool {
        Self::global_config_path().as_deref() == Some(path)
    }

    fn strip_global_only_sections(&mut self) {
        self.profiles = ProfilesConfig::default();
        self.agent_config = AgentConfig::default();
        self.tools = ToolsConfig::default();
        self.recent_projects = RecentProjectsConfig::default();
    }

    fn apply_runtime_env_overrides(mut settings: Settings) -> Settings {
        if let Ok(value) = std::env::var("GWT_DEBUG") {
            if let Some(parsed) = parse_env_bool(&value) {
                settings.debug = parsed;
            }
        }

        if let Ok(value) = std::env::var("GWT_AGENT_AUTO_INSTALL_DEPS") {
            if let Some(parsed) = parse_env_bool(&value) {
                settings.agent.auto_install_deps = parsed;
            }
        }

        if let Ok(value) = std::env::var("GWT_DOCKER_FORCE_HOST") {
            if let Some(parsed) = parse_env_bool(&value) {
                settings.docker.force_host = parsed;
            }
        }

        settings
    }

    fn load_from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            crate::logging::log_incident(
                "config",
                "load",
                Some("CONFIG_LOAD_FAILED"),
                &format!("path={}: {}", path.display(), e),
            );
            error!(
                category = "config",
                path = %path.display(),
                error = %e,
                "Failed to read config file"
            );
            GwtError::ConfigParseError {
                reason: e.to_string(),
            }
        })?;

        toml::from_str::<ConfigToml>(&content)
            .map(Into::into)
            .map_err(|e| {
                crate::logging::log_incident(
                    "config",
                    "load",
                    Some("CONFIG_PARSE_FAILED"),
                    &format!("path={}: {}", path.display(), e),
                );
                error!(
                    category = "config",
                    path = %path.display(),
                    error = %e,
                    "Failed to parse config"
                );
                GwtError::ConfigParseError {
                    reason: e.to_string(),
                }
            })
    }

    fn load_global_internal(apply_env_overrides: bool) -> Result<Self> {
        debug!(
            category = "config",
            apply_env_overrides, "Loading global settings"
        );

        let config_path = Self::global_config_path().filter(|p| p.exists());
        let mut settings = if let Some(ref path) = config_path {
            debug!(
                category = "config",
                config_path = %path.display(),
                "Loading global config file"
            );
            Self::load_from_path(path)?
        } else {
            Settings::default()
        };

        if apply_env_overrides {
            settings = Self::apply_runtime_env_overrides(settings);
        }
        settings.profiles.normalize_loaded()?;

        info!(
            category = "config",
            operation = if apply_env_overrides {
                "load_global"
            } else {
                "load_global_raw"
            },
            config_path = config_path.as_ref().map(|p| p.display().to_string()).as_deref(),
            debug = settings.debug,
            worktree_root = %settings.worktree_root,
            "Global settings loaded"
        );

        Ok(settings)
    }

    pub fn load_global_raw() -> Result<Self> {
        Self::load_global_internal(false)
    }

    /// Load settings from configuration files and environment
    #[instrument(skip_all)]
    pub fn load(repo_root: &Path) -> Result<Self> {
        debug!(
            category = "config",
            repo_root = %repo_root.display(),
            "Loading settings"
        );

        let config_path = Self::find_config_file(repo_root);
        let mut settings = if let Some(ref path) = config_path {
            debug!(
                category = "config",
                config_path = %path.display(),
                "Loading config file"
            );
            Self::load_from_path(path)?
        } else {
            Settings::default()
        };

        settings = Self::apply_runtime_env_overrides(settings);
        if let Some(path) = config_path.as_ref() {
            if !Self::is_global_config_path(path) {
                let global_settings = Self::load_global().unwrap_or_default();
                settings.profiles = global_settings.profiles;
                settings.agent_config = global_settings.agent_config;
                settings.tools = global_settings.tools;
                settings.recent_projects = global_settings.recent_projects;
            }
        }
        settings.profiles.normalize_loaded()?;

        info!(
            category = "config",
            operation = "load",
            config_path = config_path.as_ref().map(|p| p.display().to_string()).as_deref(),
            debug = settings.debug,
            worktree_root = %settings.worktree_root,
            "Settings loaded"
        );

        Ok(settings)
    }

    /// Load settings from global configuration file and environment.
    ///
    /// Reads `~/.gwt/config.toml`. Falls back to defaults when the file does not exist.
    #[instrument(skip_all)]
    pub fn load_global() -> Result<Self> {
        Self::load_global_internal(true)
    }

    /// Find the configuration file (gwt-spec issue FR-013)
    ///
    /// Priority (highest to lowest):
    /// 1. .gwt.toml (local, highest priority)
    /// 2. .gwt/config.toml (local)
    /// 3. ~/.gwt/config.toml (global)
    pub fn find_config_file(repo_root: &Path) -> Option<PathBuf> {
        debug!(
            category = "config",
            repo_root = %repo_root.display(),
            "Searching for config file"
        );

        // Local config candidates
        let local_candidates = [
            repo_root.join(".gwt.toml"),
            repo_root.join(".gwt/config.toml"),
        ];

        for path in local_candidates {
            if path.exists() {
                debug!(
                    category = "config",
                    config_path = %path.display(),
                    "Found local config file"
                );
                return Some(path);
            }
        }

        // Check global config (~/.gwt/config.toml)
        if let Some(global) = Self::global_config_path() {
            if global.exists() {
                debug!(
                    category = "config",
                    config_path = %global.display(),
                    "Found global config file"
                );
                return Some(global);
            }
        }

        debug!(
            category = "config",
            repo_root = %repo_root.display(),
            "No config file found, using defaults"
        );
        None
    }

    /// Get the new global config path (~/.gwt/config.toml)
    pub fn global_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|home| home.join(".gwt").join("config.toml"))
    }

    /// Get log directory path
    pub fn log_dir(&self, repo_root: &Path) -> PathBuf {
        if let Some(ref log_dir) = self.log_dir {
            if log_dir.is_absolute() {
                return log_dir.clone();
            }
            return repo_root.join(log_dir);
        }

        // Default: ~/.gwt/logs/{workspace_name}
        if let Some(home) = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()) {
            let workspace_name = repo_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "default".to_string());
            return home.join(".gwt").join("logs").join(workspace_name);
        }

        repo_root.join(".gwt").join("logs")
    }

    /// Save settings to file (gwt-spec issue FR-008)
    ///
    /// Uses atomic write (temp file + rename) for data safety.
    #[instrument(skip(self))]
    pub fn save(&self, path: &Path) -> Result<()> {
        debug!(
            category = "config",
            path = %path.display(),
            "Saving settings"
        );

        let mut persisted = self.clone();
        if !Self::is_global_config_path(path) {
            persisted.strip_global_only_sections();
        }

        let content = if Self::is_global_config_path(path) {
            toml::to_string_pretty(&ConfigToml::from(persisted))
        } else {
            toml::to_string_pretty(&LocalConfigToml::from(persisted))
        }
        .map_err(|e| {
            crate::logging::log_incident(
                "config",
                "save",
                Some("CONFIG_SERIALIZE_FAILED"),
                &format!("path={}: {}", path.display(), e),
            );
            error!(
                category = "config",
                path = %path.display(),
                error = %e,
                "Failed to serialize settings"
            );
            GwtError::ConfigWriteError {
                reason: e.to_string(),
            }
        })?;

        if let Some(parent) = path.parent() {
            ensure_config_dir(parent)?;
        }

        write_atomic(path, &content)?;

        info!(
            category = "config",
            operation = "save",
            path = %path.display(),
            "Settings saved"
        );
        Ok(())
    }

    /// Save settings to the new global config path (~/.gwt/config.toml)
    #[instrument(skip(self))]
    pub fn save_global(&self) -> Result<()> {
        let _guard = GLOBAL_SETTINGS_UPDATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = Self::global_config_path().ok_or_else(|| GwtError::ConfigWriteError {
            reason: "Could not determine global config path".to_string(),
        })?;
        self.save(&path)
    }

    pub fn update_global<F>(mutate: F) -> Result<()>
    where
        F: FnOnce(&mut Self) -> Result<()>,
    {
        let _guard = GLOBAL_SETTINGS_UPDATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = Self::global_config_path().ok_or_else(|| GwtError::ConfigWriteError {
            reason: "Could not determine global config path".to_string(),
        })?;
        let mut settings = Self::load_global_raw()?;
        mutate(&mut settings)?;
        settings.save(&path)
    }

    /// Create default config file
    pub fn create_default(path: &Path) -> Result<Self> {
        debug!(
            category = "config",
            path = %path.display(),
            "Creating default config"
        );

        let settings = Self::default();
        settings.save(path)?;

        info!(
            category = "config",
            operation = "create_default",
            path = %path.display(),
            "Default config created"
        );
        Ok(settings)
    }

    /// Check if a branch is protected
    pub fn is_branch_protected(&self, branch: &str) -> bool {
        self.protected_branches.iter().any(|p| p == branch)
    }
}

fn parse_env_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert!(!settings.protected_branches.is_empty());
        assert!(settings.protected_branches.contains(&"main".to_string()));
        assert!(!settings.debug);
        assert!(!settings.voice_input.enabled);
        assert_eq!(settings.voice_input.engine, "qwen3-asr");
        assert_eq!(settings.voice_input.language, "auto");
        assert_eq!(settings.voice_input.quality, "balanced");
        assert_eq!(settings.voice_input.model, "Qwen/Qwen3-ASR-1.7B");
    }

    #[test]
    fn test_load_ignores_unrelated_legacy_file() {
        let temp = TempDir::new().unwrap();
        let legacy_path = temp.path().join(".gwt.json");

        std::fs::write(
            &legacy_path,
            r#"{"default_base_branch":"develop","worktree_root":".worktrees"}"#,
        )
        .unwrap();

        let settings = Settings::load(temp.path()).unwrap();
        assert_ne!(settings.default_base_branch, "develop");
        assert!(!temp.path().join(".gwt.toml").exists());
    }

    #[test]
    fn test_save_and_load() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join(".gwt.toml");

        let settings = Settings {
            protected_branches: vec!["main".to_string(), "release".to_string()],
            debug: true,
            ..Default::default()
        };

        settings.save(&config_path).unwrap();

        let loaded = Settings::load(temp.path()).unwrap();
        assert!(loaded.protected_branches.contains(&"main".to_string()));
        assert!(loaded.protected_branches.contains(&"release".to_string()));
        assert!(loaded.debug);
    }

    #[test]
    fn test_is_branch_protected() {
        let settings = Settings::default();
        assert!(settings.is_branch_protected("main"));
        assert!(settings.is_branch_protected("master"));
        assert!(settings.is_branch_protected("develop"));
        assert!(!settings.is_branch_protected("feature/foo"));
    }

    #[test]
    fn test_env_override() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        // Set environment variable
        std::env::set_var("GWT_DEBUG", "true");

        let settings = Settings::load(temp.path()).unwrap();

        // Clean up
        std::env::remove_var("GWT_DEBUG");

        assert!(settings.debug);
    }

    #[test]
    fn test_env_override_docker_force_host_accepts_numeric_bool() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        std::env::set_var("GWT_DOCKER_FORCE_HOST", "1");
        let settings = Settings::load(temp.path()).unwrap();
        std::env::remove_var("GWT_DOCKER_FORCE_HOST");

        assert!(settings.docker.force_host);
    }

    #[test]
    fn test_env_override_auto_install_deps() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        std::env::set_var("GWT_AGENT_AUTO_INSTALL_DEPS", "true");
        let settings = Settings::load(temp.path()).unwrap();
        std::env::remove_var("GWT_AGENT_AUTO_INSTALL_DEPS");

        assert!(settings.agent.auto_install_deps);
    }

    #[test]
    fn test_global_config_path() {
        let path = Settings::global_config_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains(".gwt"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn test_new_global_config_priority() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        // Create new global config
        let new_gwt = temp.path().join(".gwt");
        std::fs::create_dir_all(&new_gwt).unwrap();
        std::fs::write(
            new_gwt.join("config.toml"),
            r#"
debug = true
default_base_branch = "new-global"
"#,
        )
        .unwrap();

        // Create a repo without local config
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        let settings = Settings::load(&repo).unwrap();
        assert!(settings.debug);
        assert_eq!(settings.default_base_branch, "new-global");
    }

    #[test]
    fn test_appearance_default() {
        let settings = Settings::default();
        assert_eq!(settings.appearance.ui_font_size, 13);
        assert_eq!(settings.appearance.terminal_font_size, 13);
    }

    #[test]
    fn test_appearance_backward_compat() {
        // Config without [appearance]/[voice_input] sections should deserialize with defaults
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join(".gwt.toml");
        std::fs::write(&config_path, "debug = true\n").unwrap();
        let settings = Settings::load(temp.path()).unwrap();
        assert!(settings.debug);
        assert_eq!(settings.appearance.ui_font_size, 13);
        assert_eq!(settings.appearance.terminal_font_size, 13);
        assert!(!settings.voice_input.enabled);
        assert_eq!(settings.voice_input.engine, "qwen3-asr");
        assert_eq!(settings.voice_input.language, "auto");
        assert_eq!(settings.voice_input.quality, "balanced");
        assert_eq!(settings.voice_input.model, "Qwen/Qwen3-ASR-1.7B");
    }

    #[test]
    fn test_appearance_save_load() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join(".gwt.toml");
        let mut settings = Settings::default();
        settings.appearance.ui_font_size = 16;
        settings.appearance.terminal_font_size = 18;
        settings.save(&config_path).unwrap();
        let loaded = Settings::load(temp.path()).unwrap();
        assert_eq!(loaded.appearance.ui_font_size, 16);
        assert_eq!(loaded.appearance.terminal_font_size, 18);
    }

    #[test]
    fn test_save_global() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let settings = Settings {
            debug: true,
            default_base_branch: "save-global-test".to_string(),
            ..Default::default()
        };

        settings.save_global().unwrap();

        // Should be saved to new location
        let new_path = temp.path().join(".gwt").join("config.toml");
        assert!(new_path.exists());

        let content = std::fs::read_to_string(&new_path).unwrap();
        assert!(content.contains("debug = true"));
        assert!(content.contains("save-global-test"));
    }

    #[test]
    fn test_local_save_strips_global_only_profiles_and_load_uses_global_profiles() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let global_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::write(
            global_dir.join("config.toml"),
            r#"
[profiles]
version = 1
active = "default"

[profiles.default.env]
OPENAI_API_KEY = "global-key"
"#,
        )
        .unwrap();

        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let local_path = repo.join(".gwt.toml");

        let mut local_settings = Settings::default();
        local_settings.profiles.profiles.insert(
            "dev".to_string(),
            Profile::new("dev").with_env("LOCAL_ONLY", "1"),
        );
        local_settings.save(&local_path).unwrap();

        let local_content = std::fs::read_to_string(&local_path).unwrap();
        assert!(!local_content.contains("[profiles]"));
        assert!(!local_content.contains("[agent_config]"));
        assert!(!local_content.contains("[tools]"));
        assert!(!local_content.contains("[recent_projects]"));

        let loaded = Settings::load(&repo).unwrap();
        assert_eq!(loaded.profiles.active.as_deref(), Some("default"));
        assert_eq!(
            loaded
                .profiles
                .profiles
                .get("default")
                .and_then(|profile| profile.env.get("OPENAI_API_KEY"))
                .map(String::as_str),
            Some("global-key")
        );
        assert!(!loaded.profiles.profiles.contains_key("dev"));
    }

    #[test]
    fn test_load_global() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());
        let prev_gwt_agent = std::env::var_os("GWT_AGENT");
        std::env::set_var("GWT_AGENT", "Codex");

        let global_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::write(
            global_dir.join("config.toml"),
            r#"
debug = true
default_base_branch = "global-main"
[appearance]
ui_font_size = 17
terminal_font_size = 19
[voice_input]
enabled = true
engine = "qwen3-asr"
hotkey = "Mod+Shift+V"
ptt_hotkey = "Mod+Shift+Space"
language = "ja"
quality = "accurate"
model = "Qwen/Qwen3-ASR-1.7B"
"#,
        )
        .unwrap();

        let loaded = Settings::load_global().unwrap();
        assert!(loaded.debug);
        assert_eq!(loaded.default_base_branch, "global-main");
        assert_eq!(loaded.appearance.ui_font_size, 17);
        assert_eq!(loaded.appearance.terminal_font_size, 19);
        assert!(loaded.voice_input.enabled);
        assert_eq!(loaded.voice_input.engine, "qwen3-asr");
        assert_eq!(loaded.voice_input.language, "ja");
        assert_eq!(loaded.voice_input.quality, "accurate");
        assert_eq!(loaded.voice_input.model, "Qwen/Qwen3-ASR-1.7B");

        match prev_gwt_agent {
            Some(value) => std::env::set_var("GWT_AGENT", value),
            None => std::env::remove_var("GWT_AGENT"),
        }
    }

    #[test]
    fn test_load_global_accepts_legacy_nested_profiles_schema() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let global_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&global_dir).unwrap();
        let config_path = global_dir.join("config.toml");
        std::fs::write(
            &config_path,
            r#"
[profiles]
version = 1
active = "default"

[profiles.profiles.default]
name = "default"
disabled_env = []
description = ""

[profiles.profiles.default.env]
OPENAI_API_KEY = "legacy-key"

[agent.skill_registration]
enabled = true
"#,
        )
        .unwrap();

        let loaded = Settings::load_global().unwrap();
        assert_eq!(loaded.profiles.active.as_deref(), Some("default"));
        assert_eq!(
            loaded
                .profiles
                .profiles
                .get("default")
                .and_then(|profile| profile.env.get("OPENAI_API_KEY"))
                .map(String::as_str),
            Some("legacy-key")
        );
        assert!(!loaded.profiles.profiles.contains_key("profiles"));

        loaded.save_global().unwrap();
        let rewritten = std::fs::read_to_string(config_path).unwrap();
        assert!(rewritten.contains("[profiles.default]"));
        assert!(!rewritten.contains("[profiles.profiles.default]"));
    }

    #[test]
    fn test_load_global_raw_ignores_env_overrides() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let prev_debug = std::env::var_os("GWT_DEBUG");
        let prev_docker_force_host = std::env::var_os("GWT_DOCKER_FORCE_HOST");
        let prev_auto_install = std::env::var_os("GWT_AGENT_AUTO_INSTALL_DEPS");

        let global_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::write(
            global_dir.join("config.toml"),
            r#"
debug = false

[agent]
auto_install_deps = false

[docker]
force_host = false
"#,
        )
        .unwrap();

        std::env::set_var("GWT_DEBUG", "true");
        std::env::set_var("GWT_DOCKER_FORCE_HOST", "1");
        std::env::set_var("GWT_AGENT_AUTO_INSTALL_DEPS", "true");

        let raw = Settings::load_global_raw().unwrap();
        let merged = Settings::load_global().unwrap();

        match prev_debug {
            Some(value) => std::env::set_var("GWT_DEBUG", value),
            None => std::env::remove_var("GWT_DEBUG"),
        }
        match prev_docker_force_host {
            Some(value) => std::env::set_var("GWT_DOCKER_FORCE_HOST", value),
            None => std::env::remove_var("GWT_DOCKER_FORCE_HOST"),
        }
        match prev_auto_install {
            Some(value) => std::env::set_var("GWT_AGENT_AUTO_INSTALL_DEPS", value),
            None => std::env::remove_var("GWT_AGENT_AUTO_INSTALL_DEPS"),
        }

        assert!(!raw.debug);
        assert!(!raw.docker.force_host);
        assert!(!raw.agent.auto_install_deps);

        assert!(merged.debug);
        assert!(merged.docker.force_host);
        assert!(merged.agent.auto_install_deps);
    }

    #[test]
    fn test_load_ignores_runtime_gwt_agent_env() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());
        let config_path = temp.path().join(".gwt.toml");
        std::fs::write(&config_path, "debug = true\n").unwrap();

        let prev_gwt_agent = std::env::var_os("GWT_AGENT");
        std::env::set_var("GWT_AGENT", "Codex");

        let loaded = Settings::load(temp.path()).unwrap();
        assert!(loaded.debug);

        match prev_gwt_agent {
            Some(value) => std::env::set_var("GWT_AGENT", value),
            None => std::env::remove_var("GWT_AGENT"),
        }
    }
}
