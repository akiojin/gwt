//! Settings management commands

use crate::state::AppState;
use gwt_core::config::Settings;
use serde::{Deserialize, Serialize};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use tauri::State;
use tracing::error;

fn with_panic_guard<T>(context: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            error!(
                category = "tauri",
                operation = context,
                "Unexpected panic while handling settings command"
            );
            Err(format!("Unexpected error while {}", context))
        }
    }
}

fn normalize_app_language(value: Option<&str>) -> String {
    match value.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
        "ja" => "ja".to_string(),
        "en" => "en".to_string(),
        _ => "auto".to_string(),
    }
}

/// Serializable settings data for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceInputSettingsData {
    pub enabled: bool,
    pub hotkey: String,
    pub language: String,
    pub model: String,
}

impl Default for VoiceInputSettingsData {
    fn default() -> Self {
        Self {
            enabled: false,
            hotkey: "Mod+Shift+M".to_string(),
            language: "auto".to_string(),
            model: "base".to_string(),
        }
    }
}

/// Serializable settings data for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsData {
    pub protected_branches: Vec<String>,
    pub default_base_branch: String,
    pub worktree_root: String,
    pub debug: bool,
    pub log_dir: Option<String>,
    pub log_retention_days: u32,
    pub agent_default: Option<String>,
    pub agent_claude_path: Option<String>,
    pub agent_codex_path: Option<String>,
    pub agent_gemini_path: Option<String>,
    pub agent_auto_install_deps: bool,
    pub agent_github_project_id: Option<String>,
    pub docker_force_host: bool,
    pub ui_font_size: u32,
    pub terminal_font_size: u32,
    #[serde(default = "default_app_language")]
    pub app_language: String,
    #[serde(default)]
    pub voice_input: VoiceInputSettingsData,
    #[serde(default)]
    pub default_shell: Option<String>,
}

fn default_app_language() -> String {
    "auto".to_string()
}

impl From<&Settings> for SettingsData {
    fn from(s: &Settings) -> Self {
        SettingsData {
            protected_branches: s.protected_branches.clone(),
            default_base_branch: s.default_base_branch.clone(),
            worktree_root: s.worktree_root.clone(),
            debug: s.debug,
            log_dir: s.log_dir.as_ref().map(|p| p.to_string_lossy().to_string()),
            log_retention_days: s.log_retention_days,
            agent_default: s.agent.default_agent.clone(),
            agent_claude_path: s
                .agent
                .claude_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            agent_codex_path: s
                .agent
                .codex_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            agent_gemini_path: s
                .agent
                .gemini_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            agent_auto_install_deps: s.agent.auto_install_deps,
            agent_github_project_id: s.agent.github_project_id.clone(),
            docker_force_host: s.docker.force_host,
            ui_font_size: s.appearance.ui_font_size,
            terminal_font_size: s.appearance.terminal_font_size,
            app_language: normalize_app_language(Some(&s.app_language)),
            voice_input: VoiceInputSettingsData {
                enabled: s.voice_input.enabled,
                hotkey: s.voice_input.hotkey.clone(),
                language: s.voice_input.language.clone(),
                model: s.voice_input.model.clone(),
            },
            default_shell: s.terminal.default_shell.clone(),
        }
    }
}

impl SettingsData {
    /// Convert back to gwt_core Settings by creating a default and updating fields.
    ///
    /// Uses serde round-trip since the sub-types (AgentSettings, DockerSettings)
    /// are not re-exported from gwt_core::config.
    #[allow(clippy::field_reassign_with_default)]
    fn to_settings(&self) -> Result<Settings, String> {
        let mut s = Settings::default();
        s.protected_branches = self.protected_branches.clone();
        s.default_base_branch = self.default_base_branch.clone();
        s.worktree_root = self.worktree_root.clone();
        s.debug = self.debug;
        s.log_dir = self.log_dir.as_ref().map(PathBuf::from);
        s.log_retention_days = self.log_retention_days;
        s.agent.default_agent = self.agent_default.clone();
        s.agent.claude_path = self.agent_claude_path.as_ref().map(PathBuf::from);
        s.agent.codex_path = self.agent_codex_path.as_ref().map(PathBuf::from);
        s.agent.gemini_path = self.agent_gemini_path.as_ref().map(PathBuf::from);
        s.agent.auto_install_deps = self.agent_auto_install_deps;
        s.agent.github_project_id = self
            .agent_github_project_id
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        s.docker.force_host = self.docker_force_host;
        s.appearance.ui_font_size = self.ui_font_size;
        s.appearance.terminal_font_size = self.terminal_font_size;
        s.app_language = normalize_app_language(Some(self.app_language.as_str()));
        s.voice_input.enabled = self.voice_input.enabled;
        s.voice_input.hotkey = self.voice_input.hotkey.trim().to_string();
        s.voice_input.language = self.voice_input.language.trim().to_string();
        s.voice_input.model = self.voice_input.model.trim().to_string();
        s.terminal.default_shell = self
            .default_shell
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        Ok(s)
    }
}

/// Get current settings
#[tauri::command]
pub fn get_settings(
    _window: tauri::Window,
    _state: State<AppState>,
) -> Result<SettingsData, String> {
    with_panic_guard("loading settings", || {
        let settings = Settings::load_global().map_err(|e| e.to_string())?;
        Ok(SettingsData::from(&settings))
    })
}

/// Save settings
#[tauri::command]
pub fn save_settings(
    _window: tauri::Window,
    settings: SettingsData,
    _state: State<AppState>,
) -> Result<(), String> {
    with_panic_guard("saving settings", || {
        let core_settings = settings.to_settings()?;
        core_settings.save_global().map_err(|e| e.to_string())?;

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_data_round_trip() {
        let mut core = Settings::default();
        core.appearance.ui_font_size = 16;
        core.appearance.terminal_font_size = 20;
        core.app_language = "ja".to_string();
        core.voice_input.enabled = true;
        core.voice_input.hotkey = "Mod+Shift+V".to_string();
        core.voice_input.language = "ja".to_string();
        core.voice_input.model = "base".to_string();
        core.terminal.default_shell = Some("powershell".to_string());
        let data = SettingsData::from(&core);
        assert_eq!(data.ui_font_size, 16);
        assert_eq!(data.terminal_font_size, 20);
        assert_eq!(data.app_language, "ja");
        assert!(data.voice_input.enabled);
        assert_eq!(data.voice_input.hotkey, "Mod+Shift+V");
        assert_eq!(data.voice_input.language, "ja");
        assert_eq!(data.voice_input.model, "base");
        assert_eq!(data.default_shell, Some("powershell".to_string()));
        let back = data.to_settings().unwrap();
        assert_eq!(back.appearance.ui_font_size, 16);
        assert_eq!(back.appearance.terminal_font_size, 20);
        assert_eq!(back.app_language, "ja");
        assert!(back.voice_input.enabled);
        assert_eq!(back.voice_input.hotkey, "Mod+Shift+V");
        assert_eq!(back.voice_input.language, "ja");
        assert_eq!(back.voice_input.model, "base");
        assert_eq!(back.terminal.default_shell, Some("powershell".to_string()));
    }

    #[test]
    fn test_settings_data_default_shell_none() {
        let core = Settings::default();
        let data = SettingsData::from(&core);
        assert!(data.default_shell.is_none());
        let back = data.to_settings().unwrap();
        assert!(back.terminal.default_shell.is_none());
    }

    #[test]
    fn test_settings_data_default_shell_trims_whitespace() {
        let mut data = SettingsData::from(&Settings::default());
        data.default_shell = Some("  wsl  ".to_string());
        let back = data.to_settings().unwrap();
        assert_eq!(back.terminal.default_shell, Some("wsl".to_string()));
    }

    #[test]
    fn test_settings_data_default_shell_empty_becomes_none() {
        let mut data = SettingsData::from(&Settings::default());
        data.default_shell = Some("   ".to_string());
        let back = data.to_settings().unwrap();
        assert!(back.terminal.default_shell.is_none());
    }
}
