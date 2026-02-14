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
pub struct VoiceInputSettingsData {
    pub enabled: bool,
    pub engine: String,
    pub hotkey: String,
    pub ptt_hotkey: String,
    pub language: String,
    pub quality: String,
    pub model: String,
}

impl Default for VoiceInputSettingsData {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: "qwen3-asr".to_string(),
            hotkey: "Mod+Shift+M".to_string(),
            ptt_hotkey: "Mod+Shift+Space".to_string(),
            language: "auto".to_string(),
            quality: "balanced".to_string(),
            model: "Qwen/Qwen3-ASR-1.7B".to_string(),
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
    pub docker_force_host: bool,
    pub ui_font_size: u32,
    pub terminal_font_size: u32,
    #[serde(default)]
    pub voice_input: VoiceInputSettingsData,
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
            docker_force_host: s.docker.force_host,
            ui_font_size: s.appearance.ui_font_size,
            terminal_font_size: s.appearance.terminal_font_size,
            voice_input: VoiceInputSettingsData {
                enabled: s.voice_input.enabled,
                engine: s.voice_input.engine.clone(),
                hotkey: s.voice_input.hotkey.clone(),
                ptt_hotkey: s.voice_input.ptt_hotkey.clone(),
                language: s.voice_input.language.clone(),
                quality: s.voice_input.quality.clone(),
                model: s.voice_input.model.clone(),
            },
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
        s.docker.force_host = self.docker_force_host;
        s.appearance.ui_font_size = self.ui_font_size;
        s.appearance.terminal_font_size = self.terminal_font_size;
        let voice = normalize_voice_input(&self.voice_input)?;
        s.voice_input.enabled = self.voice_input.enabled;
        s.voice_input.engine = voice.engine;
        s.voice_input.hotkey = voice.hotkey;
        s.voice_input.ptt_hotkey = voice.ptt_hotkey;
        s.voice_input.language = voice.language;
        s.voice_input.quality = voice.quality;
        s.voice_input.model = voice.model;
        Ok(s)
    }
}

#[derive(Debug, Clone)]
struct NormalizedVoiceInput {
    engine: String,
    hotkey: String,
    ptt_hotkey: String,
    language: String,
    quality: String,
    model: String,
}

fn qwen_model_for_quality(quality: &str) -> &'static str {
    match quality {
        "fast" => "Qwen/Qwen3-ASR-0.6B",
        "accurate" => "Qwen/Qwen3-ASR-1.7B",
        _ => "Qwen/Qwen3-ASR-1.7B",
    }
}

fn normalize_hotkey(hotkey: &str, field: &str) -> Result<String, String> {
    let trimmed = hotkey.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if !trimmed.contains('+') && trimmed.chars().count() != 1 {
        return Err(format!("{field} must include modifiers or a single key"));
    }
    Ok(trimmed.to_string())
}

fn normalize_voice_input(value: &VoiceInputSettingsData) -> Result<NormalizedVoiceInput, String> {
    let engine = match value.engine.trim().to_lowercase().as_str() {
        "" | "qwen3-asr" | "qwen" | "whisper" => "qwen3-asr".to_string(),
        _ => return Err("voice_input.engine must be \"qwen3-asr\"".to_string()),
    };

    let hotkey = normalize_hotkey(&value.hotkey, "voice_input.hotkey")?;
    let ptt_hotkey = normalize_hotkey(&value.ptt_hotkey, "voice_input.ptt_hotkey")?;
    if hotkey.eq_ignore_ascii_case(&ptt_hotkey) {
        return Err("voice_input.hotkey and voice_input.ptt_hotkey must differ".to_string());
    }

    let language = value.language.trim().to_lowercase();
    let language = match language.as_str() {
        "" | "auto" => "auto".to_string(),
        "ja" => "ja".to_string(),
        "en" => "en".to_string(),
        _ => return Err("voice_input.language must be one of auto|ja|en".to_string()),
    };

    let quality = value.quality.trim().to_lowercase();
    let quality = match quality.as_str() {
        "" | "balanced" => "balanced".to_string(),
        "fast" => "fast".to_string(),
        "accurate" => "accurate".to_string(),
        _ => return Err("voice_input.quality must be one of fast|balanced|accurate".to_string()),
    };

    let model = value.model.trim();
    let model = if model.is_empty() {
        qwen_model_for_quality(&quality).to_string()
    } else {
        model.to_string()
    };

    Ok(NormalizedVoiceInput {
        engine,
        hotkey,
        ptt_hotkey,
        language,
        quality,
        model,
    })
}

/// Get current settings
#[tauri::command]
pub fn get_settings(window: tauri::Window, state: State<AppState>) -> Result<SettingsData, String> {
    with_panic_guard("loading settings", || {
        let repo_root = match state.project_for_window(window.label()) {
            Some(p) => PathBuf::from(p),
            None => {
                let settings = Settings::load_global().map_err(|e| e.to_string())?;
                return Ok(SettingsData::from(&settings));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_data_round_trip() {
        let mut core = Settings::default();
        core.appearance.ui_font_size = 16;
        core.appearance.terminal_font_size = 20;
        core.voice_input.enabled = true;
        core.voice_input.engine = "qwen3-asr".to_string();
        core.voice_input.hotkey = "Mod+Shift+V".to_string();
        core.voice_input.ptt_hotkey = "Mod+Shift+Space".to_string();
        core.voice_input.language = "ja".to_string();
        core.voice_input.quality = "accurate".to_string();
        core.voice_input.model = "Qwen/Qwen3-ASR-1.7B".to_string();
        let data = SettingsData::from(&core);
        assert_eq!(data.ui_font_size, 16);
        assert_eq!(data.terminal_font_size, 20);
        assert!(data.voice_input.enabled);
        assert_eq!(data.voice_input.engine, "qwen3-asr");
        assert_eq!(data.voice_input.hotkey, "Mod+Shift+V");
        assert_eq!(data.voice_input.ptt_hotkey, "Mod+Shift+Space");
        assert_eq!(data.voice_input.language, "ja");
        assert_eq!(data.voice_input.quality, "accurate");
        assert_eq!(data.voice_input.model, "Qwen/Qwen3-ASR-1.7B");
        let back = data.to_settings().unwrap();
        assert_eq!(back.appearance.ui_font_size, 16);
        assert_eq!(back.appearance.terminal_font_size, 20);
        assert!(back.voice_input.enabled);
        assert_eq!(back.voice_input.engine, "qwen3-asr");
        assert_eq!(back.voice_input.hotkey, "Mod+Shift+V");
        assert_eq!(back.voice_input.ptt_hotkey, "Mod+Shift+Space");
        assert_eq!(back.voice_input.language, "ja");
        assert_eq!(back.voice_input.quality, "accurate");
        assert_eq!(back.voice_input.model, "Qwen/Qwen3-ASR-1.7B");
    }

    #[test]
    fn test_voice_hotkeys_must_not_conflict() {
        let mut data = SettingsData::from(&Settings::default());
        data.voice_input.enabled = true;
        data.voice_input.hotkey = "Mod+Shift+M".to_string();
        data.voice_input.ptt_hotkey = "Mod+Shift+M".to_string();
        let err = data.to_settings().unwrap_err();
        assert!(err.contains("must differ"));
    }

    #[test]
    fn test_voice_engine_whisper_is_migrated_to_qwen() {
        let mut data = SettingsData::from(&Settings::default());
        data.voice_input.engine = "whisper".to_string();
        let normalized = data.to_settings().unwrap();
        assert_eq!(normalized.voice_input.engine, "qwen3-asr");
    }
}
