//! Settings management commands

use crate::state::AppState;
use gwt_core::config::{Settings, SkillRegistrationPreferences, SkillRegistrationScope};
use gwt_core::StructuredError;
use serde::{Deserialize, Serialize};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use tauri::State;
use tracing::error;

fn with_panic_guard<T>(
    context: &str,
    command: &str,
    f: impl FnOnce() -> Result<T, StructuredError>,
) -> Result<T, StructuredError> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            error!(
                category = "tauri",
                operation = context,
                "Unexpected panic while handling settings command"
            );
            Err(StructuredError::internal(
                &format!("Unexpected error while {}", context),
                command,
            ))
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

fn parse_scope_field(
    value: Option<&str>,
    field_name: &str,
) -> Result<Option<SkillRegistrationScope>, String> {
    let normalized = value.unwrap_or("").trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Ok(None);
    }

    match normalized.as_str() {
        "user" => Ok(Some(SkillRegistrationScope::User)),
        "project" => Ok(Some(SkillRegistrationScope::Project)),
        "local" => Ok(Some(SkillRegistrationScope::Local)),
        _ => Err(format!("{field_name} must be one of: user, project, local")),
    }
}

fn scope_to_string(scope: SkillRegistrationScope) -> String {
    match scope {
        SkillRegistrationScope::User => "user".to_string(),
        SkillRegistrationScope::Project => "project".to_string(),
        SkillRegistrationScope::Local => "local".to_string(),
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
    pub agent_github_project_id: Option<String>,
    #[serde(default)]
    pub agent_skill_registration_default_scope: Option<String>,
    #[serde(default)]
    pub agent_skill_registration_codex_scope: Option<String>,
    #[serde(default)]
    pub agent_skill_registration_claude_scope: Option<String>,
    #[serde(default)]
    pub agent_skill_registration_gemini_scope: Option<String>,
    pub docker_force_host: bool,
    pub ui_font_size: u32,
    pub terminal_font_size: u32,
    #[serde(default = "default_ui_font_family")]
    pub ui_font_family: String,
    #[serde(default = "default_terminal_font_family")]
    pub terminal_font_family: String,
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

fn default_ui_font_family() -> String {
    "system-ui, -apple-system, \"Segoe UI\", Roboto, Ubuntu, sans-serif".to_string()
}

fn default_terminal_font_family() -> String {
    "\"JetBrains Mono\", \"Fira Code\", \"SF Mono\", Menlo, Consolas, monospace".to_string()
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
            agent_skill_registration_default_scope: s
                .agent
                .skill_registration
                .as_ref()
                .map(|prefs| scope_to_string(prefs.default_scope)),
            agent_skill_registration_codex_scope: s
                .agent
                .skill_registration
                .as_ref()
                .and_then(|prefs| prefs.codex_scope.map(scope_to_string)),
            agent_skill_registration_claude_scope: s
                .agent
                .skill_registration
                .as_ref()
                .and_then(|prefs| prefs.claude_scope.map(scope_to_string)),
            agent_skill_registration_gemini_scope: s
                .agent
                .skill_registration
                .as_ref()
                .and_then(|prefs| prefs.gemini_scope.map(scope_to_string)),
            docker_force_host: s.docker.force_host,
            ui_font_size: s.appearance.ui_font_size,
            terminal_font_size: s.appearance.terminal_font_size,
            // Keep these fields for frontend backward compatibility.
            // gwt-core no longer persists font family settings.
            ui_font_family: default_ui_font_family(),
            terminal_font_family: default_terminal_font_family(),
            app_language: normalize_app_language(Some(&s.app_language)),
            voice_input: VoiceInputSettingsData {
                enabled: s.voice_input.enabled,
                engine: s.voice_input.engine.clone(),
                hotkey: s.voice_input.hotkey.clone(),
                ptt_hotkey: s.voice_input.ptt_hotkey.clone(),
                language: s.voice_input.language.clone(),
                quality: s.voice_input.quality.clone(),
                model: s.voice_input.model.clone(),
            },
            default_shell: s.terminal.default_shell.clone(),
        }
    }
}

impl SettingsData {
    /// Apply frontend-editable fields onto an existing Settings struct.
    fn apply_to_settings(&self, s: &mut Settings) -> Result<(), String> {
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

        let default_scope = parse_scope_field(
            self.agent_skill_registration_default_scope.as_deref(),
            "agent_skill_registration_default_scope",
        )?;
        let codex_scope = parse_scope_field(
            self.agent_skill_registration_codex_scope.as_deref(),
            "agent_skill_registration_codex_scope",
        )?;
        let claude_scope = parse_scope_field(
            self.agent_skill_registration_claude_scope.as_deref(),
            "agent_skill_registration_claude_scope",
        )?;
        let gemini_scope = parse_scope_field(
            self.agent_skill_registration_gemini_scope.as_deref(),
            "agent_skill_registration_gemini_scope",
        )?;

        if default_scope.is_none()
            && (codex_scope.is_some() || claude_scope.is_some() || gemini_scope.is_some())
        {
            return Err(
                "agent_skill_registration_default_scope is required when agent overrides are set"
                    .to_string(),
            );
        }

        s.agent.skill_registration =
            default_scope.map(|default_scope| SkillRegistrationPreferences {
                default_scope,
                codex_scope,
                claude_scope,
                gemini_scope,
            });

        s.docker.force_host = self.docker_force_host;
        s.appearance.ui_font_size = self.ui_font_size;
        s.appearance.terminal_font_size = self.terminal_font_size;
        s.app_language = normalize_app_language(Some(self.app_language.as_str()));
        let voice = normalize_voice_input(&self.voice_input)?;
        s.voice_input.enabled = self.voice_input.enabled;
        s.voice_input.engine = voice.engine;
        s.voice_input.hotkey = voice.hotkey;
        s.voice_input.ptt_hotkey = voice.ptt_hotkey;
        s.voice_input.language = voice.language;
        s.voice_input.quality = voice.quality;
        s.voice_input.model = voice.model;
        s.terminal.default_shell = self
            .default_shell
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        Ok(())
    }

    #[cfg(test)]
    #[allow(clippy::field_reassign_with_default)]
    fn to_settings(&self) -> Result<Settings, String> {
        let mut s = Settings::default();
        self.apply_to_settings(&mut s)?;
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

fn is_named_hotkey_key(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "space" | "escape" | "enter" | "tab" | "backspace" | "delete"
    )
}

fn normalize_hotkey(hotkey: &str, field: &str) -> Result<String, String> {
    let trimmed = hotkey.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if !trimmed.contains('+') && trimmed.chars().count() != 1 && !is_named_hotkey_key(trimmed) {
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
pub fn get_settings(
    window: tauri::Window,
    state: State<AppState>,
) -> Result<SettingsData, StructuredError> {
    with_panic_guard("loading settings", "get_settings", || {
        let settings = if let Some(project_path) = state.project_for_window(window.label()) {
            Settings::load(Path::new(&project_path))
                .map_err(|e| StructuredError::from_gwt_error(&e, "get_settings"))?
        } else {
            Settings::load_global()
                .map_err(|e| StructuredError::from_gwt_error(&e, "get_settings"))?
        };
        Ok(SettingsData::from(&settings))
    })
}

/// Save settings
#[tauri::command]
pub fn save_settings(
    window: tauri::Window,
    settings: SettingsData,
    state: State<AppState>,
) -> Result<(), StructuredError> {
    with_panic_guard("saving settings", "save_settings", || {
        let project_path = state.project_for_window(window.label());
        let mut core_settings = match project_path.as_ref() {
            Some(path) => Settings::load(Path::new(path))
                .map_err(|e| StructuredError::from_gwt_error(&e, "save_settings"))?,
            None => Settings::load_global()
                .map_err(|e| StructuredError::from_gwt_error(&e, "save_settings"))?,
        };

        settings
            .apply_to_settings(&mut core_settings)
            .map_err(|e| StructuredError::internal(&e, "save_settings"))?;

        match project_path {
            Some(path) => {
                let config_path = Path::new(&path).join(".gwt.toml");
                core_settings
                    .save(&config_path)
                    .map_err(|e| StructuredError::from_gwt_error(&e, "save_settings"))?;
            }
            None => {
                core_settings
                    .save_global()
                    .map_err(|e| StructuredError::from_gwt_error(&e, "save_settings"))?;
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
        core.app_language = "ja".to_string();
        core.voice_input.enabled = true;
        core.voice_input.engine = "qwen3-asr".to_string();
        core.voice_input.hotkey = "Mod+Shift+V".to_string();
        core.voice_input.ptt_hotkey = "Mod+Shift+Space".to_string();
        core.voice_input.language = "ja".to_string();
        core.voice_input.quality = "accurate".to_string();
        core.voice_input.model = "Qwen/Qwen3-ASR-1.7B".to_string();
        core.terminal.default_shell = Some("powershell".to_string());
        let data = SettingsData::from(&core);
        assert_eq!(data.ui_font_size, 16);
        assert_eq!(data.terminal_font_size, 20);
        assert_eq!(data.ui_font_family, default_ui_font_family());
        assert_eq!(data.terminal_font_family, default_terminal_font_family());
        assert_eq!(data.app_language, "ja");
        assert!(data.voice_input.enabled);
        assert_eq!(data.voice_input.engine, "qwen3-asr");
        assert_eq!(data.voice_input.hotkey, "Mod+Shift+V");
        assert_eq!(data.voice_input.ptt_hotkey, "Mod+Shift+Space");
        assert_eq!(data.voice_input.language, "ja");
        assert_eq!(data.voice_input.quality, "accurate");
        assert_eq!(data.voice_input.model, "Qwen/Qwen3-ASR-1.7B");
        assert_eq!(data.default_shell, Some("powershell".to_string()));
        assert_eq!(data.agent_skill_registration_default_scope, None);
        let back = data.to_settings().unwrap();
        assert_eq!(back.appearance.ui_font_size, 16);
        assert_eq!(back.appearance.terminal_font_size, 20);
        assert_eq!(back.app_language, "ja");
        assert!(back.voice_input.enabled);
        assert_eq!(back.voice_input.engine, "qwen3-asr");
        assert_eq!(back.voice_input.hotkey, "Mod+Shift+V");
        assert_eq!(back.voice_input.ptt_hotkey, "Mod+Shift+Space");
        assert_eq!(back.voice_input.language, "ja");
        assert_eq!(back.voice_input.quality, "accurate");
        assert_eq!(back.voice_input.model, "Qwen/Qwen3-ASR-1.7B");
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

    #[test]
    fn test_settings_data_font_family_empty_uses_default() {
        let mut data = SettingsData::from(&Settings::default());
        data.ui_font_family = "   ".to_string();
        data.terminal_font_family = "".to_string();
        let back = data.to_settings().unwrap();
        let normalized = SettingsData::from(&back);
        assert_eq!(normalized.ui_font_family, default_ui_font_family());
        assert_eq!(
            normalized.terminal_font_family,
            default_terminal_font_family()
        );
    }

    #[test]
    fn test_settings_data_skill_registration_round_trip() {
        let mut core = Settings::default();
        core.agent.skill_registration = Some(SkillRegistrationPreferences {
            default_scope: SkillRegistrationScope::Project,
            codex_scope: Some(SkillRegistrationScope::User),
            claude_scope: Some(SkillRegistrationScope::Project),
            gemini_scope: Some(SkillRegistrationScope::Local),
        });

        let data = SettingsData::from(&core);
        assert_eq!(
            data.agent_skill_registration_default_scope.as_deref(),
            Some("project")
        );
        assert_eq!(
            data.agent_skill_registration_codex_scope.as_deref(),
            Some("user")
        );
        assert_eq!(
            data.agent_skill_registration_claude_scope.as_deref(),
            Some("project")
        );
        assert_eq!(
            data.agent_skill_registration_gemini_scope.as_deref(),
            Some("local")
        );

        let back = data.to_settings().unwrap();
        assert_eq!(back.agent.skill_registration, core.agent.skill_registration);
    }

    #[test]
    fn test_settings_data_skill_registration_override_requires_default_scope() {
        let mut data = SettingsData::from(&Settings::default());
        data.agent_skill_registration_default_scope = None;
        data.agent_skill_registration_codex_scope = Some("user".to_string());

        let err = data.to_settings().unwrap_err();
        assert!(
            err.contains("agent_skill_registration_default_scope is required"),
            "unexpected error: {err}"
        );
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn test_apply_to_settings_preserves_unmanaged_fields() {
        let mut existing = Settings::default();
        existing.app_language = "ja".to_string();
        existing.terminal.default_shell = Some("zsh".to_string());
        existing.agent.github_project_id = Some("12345".to_string());

        let mut data = SettingsData::from(&existing);
        data.ui_font_size = 18;
        data.voice_input.quality = "fast".to_string();
        data.voice_input.model = String::new();

        data.apply_to_settings(&mut existing).unwrap();

        assert_eq!(existing.app_language, "ja");
        assert_eq!(existing.terminal.default_shell.as_deref(), Some("zsh"));
        assert_eq!(existing.agent.github_project_id.as_deref(), Some("12345"));
        assert_eq!(existing.appearance.ui_font_size, 18);
        assert_eq!(existing.voice_input.quality, "fast");
        assert_eq!(existing.voice_input.model, "Qwen/Qwen3-ASR-0.6B");
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
    fn test_voice_hotkey_accepts_named_single_key() {
        let mut data = SettingsData::from(&Settings::default());
        data.voice_input.hotkey = "Space".to_string();
        data.voice_input.ptt_hotkey = "Mod+Shift+Space".to_string();
        let normalized = data.to_settings().unwrap();
        assert_eq!(normalized.voice_input.hotkey, "Space");
    }

    #[test]
    fn test_voice_engine_whisper_is_migrated_to_qwen() {
        let mut data = SettingsData::from(&Settings::default());
        data.voice_input.engine = "whisper".to_string();
        let normalized = data.to_settings().unwrap();
        assert_eq!(normalized.voice_input.engine, "qwen3-asr");
    }
}
