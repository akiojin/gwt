//! Settings management commands

use crate::state::AppState;
use gwt_core::config::Settings;
use serde::{Deserialize, Serialize};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
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

/// Serializable settings data for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsData {
    pub protected_branches: Vec<String>,
    pub default_base_branch: String,
    pub worktree_root: String,
    pub debug: bool,
    pub log_dir: Option<String>,
    pub log_retention_days: u32,
    pub web_port: u16,
    pub web_address: String,
    pub web_cors: bool,
    pub agent_default: Option<String>,
    pub agent_claude_path: Option<String>,
    pub agent_codex_path: Option<String>,
    pub agent_gemini_path: Option<String>,
    pub agent_auto_install_deps: bool,
    pub docker_force_host: bool,
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
            web_port: s.web.port,
            web_address: s.web.address.clone(),
            web_cors: s.web.cors,
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
            docker_force_host: s.docker.force_host,
        }
    }
}

impl SettingsData {
    /// Convert back to gwt_core Settings by creating a default and updating fields.
    ///
    /// Uses serde round-trip since the sub-types (WebSettings, AgentSettings, DockerSettings)
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
        s.web.port = self.web_port;
        s.web.address = self.web_address.clone();
        s.web.cors = self.web_cors;
        s.agent.default_agent = self.agent_default.clone();
        s.agent.claude_path = self.agent_claude_path.as_ref().map(PathBuf::from);
        s.agent.codex_path = self.agent_codex_path.as_ref().map(PathBuf::from);
        s.agent.gemini_path = self.agent_gemini_path.as_ref().map(PathBuf::from);
        s.agent.auto_install_deps = self.agent_auto_install_deps;
        s.docker.force_host = self.docker_force_host;
        Ok(s)
    }
}

/// Get current settings
#[tauri::command]
pub fn get_settings(window: tauri::Window, state: State<AppState>) -> Result<SettingsData, String> {
    with_panic_guard("loading settings", || {
        let repo_root = match state.project_for_window(window.label()) {
            Some(p) => PathBuf::from(p),
            None => {
                // Return default settings if no project is opened in this window
                return Ok(SettingsData::from(&Settings::default()));
            }
        };

        let settings = Settings::load(&repo_root).map_err(|e| e.to_string())?;
        Ok(SettingsData::from(&settings))
    })
}

/// Save settings
#[tauri::command]
pub fn save_settings(
    window: tauri::Window,
    settings: SettingsData,
    state: State<AppState>,
) -> Result<(), String> {
    with_panic_guard("saving settings", || {
        let core_settings = settings.to_settings()?;

        match state.project_for_window(window.label()) {
            Some(p) => {
                let config_path = Path::new(&p).join(".gwt.toml");
                core_settings
                    .save(&config_path)
                    .map_err(|e| e.to_string())?;
            }
            None => {
                // Save to global config if no project is opened
                core_settings.save_global().map_err(|e| e.to_string())?;
            }
        }

        Ok(())
    })
}
