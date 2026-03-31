//! Persistence for TUI launch dialog defaults.
//!
//! Saves and loads user preferences (last selected agent, model per agent, etc.)
//! to `~/.gwt/tui/launch_defaults.json` so the dialog remembers settings across
//! sessions.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

const DEFAULTS_REL_PATH: &str = ".gwt/tui/launch_defaults.json";

/// Persisted launch dialog preferences.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LaunchDefaults {
    #[serde(default)]
    pub selected_agent: String,
    #[serde(default)]
    pub session_mode: String,
    #[serde(default)]
    pub model_by_agent: HashMap<String, String>,
    #[serde(default)]
    pub version_by_agent: HashMap<String, String>,
    #[serde(default)]
    pub skip_permissions: bool,
    #[serde(default)]
    pub reasoning_level: String,
    #[serde(default)]
    pub fast_mode: bool,
    #[serde(default)]
    pub extra_args: String,
    #[serde(default)]
    pub env_overrides: String,
}

/// Load launch defaults from `~/.gwt/tui/launch_defaults.json`.
///
/// Returns [`LaunchDefaults::default()`] if the file does not exist or cannot be parsed.
pub fn load_defaults() -> LaunchDefaults {
    let path = match dirs::home_dir() {
        Some(home) => home.join(DEFAULTS_REL_PATH),
        None => return LaunchDefaults::default(),
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save launch defaults to `~/.gwt/tui/launch_defaults.json`.
///
/// Silently ignores I/O errors (best-effort persistence).
pub fn save_defaults(defaults: &LaunchDefaults) {
    let path = match dirs::home_dir() {
        Some(home) => home.join(DEFAULTS_REL_PATH),
        None => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = serde_json::to_string_pretty(defaults).map(|s| std::fs::write(&path, s));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_launch_defaults_roundtrip() {
        let mut defaults = LaunchDefaults::default();
        defaults.selected_agent = "claude".to_string();
        defaults
            .model_by_agent
            .insert("claude".to_string(), "opus".to_string());
        defaults.skip_permissions = true;
        defaults.fast_mode = true;
        defaults.reasoning_level = "high".to_string();
        defaults.extra_args = "--verbose".to_string();

        let json = serde_json::to_string_pretty(&defaults).unwrap();
        let parsed: LaunchDefaults = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.selected_agent, "claude");
        assert_eq!(
            parsed.model_by_agent.get("claude"),
            Some(&"opus".to_string())
        );
        assert!(parsed.skip_permissions);
        assert!(parsed.fast_mode);
        assert_eq!(parsed.reasoning_level, "high");
        assert_eq!(parsed.extra_args, "--verbose");
    }

    #[test]
    fn test_launch_defaults_deserialize_empty() {
        let parsed: LaunchDefaults = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed.selected_agent, "");
        assert!(!parsed.skip_permissions);
        assert!(!parsed.fast_mode);
    }
}
