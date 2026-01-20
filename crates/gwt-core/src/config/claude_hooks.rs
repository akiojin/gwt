//! Claude Code Hook settings management (SPEC-861d8cdf T-102)
//!
//! This module provides functionality to register gwt hooks in Claude Code's settings.json
//! for agent status tracking.

use crate::error::GwtError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Claude Code settings.json structure (partial)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaudeSettings {
    #[serde(default)]
    pub hooks: HashMap<String, serde_json::Value>,

    /// Preserve other fields
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Hook event types supported by Claude Code
pub const HOOK_EVENTS: &[&str] = &[
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
];

/// Get the path to Claude Code settings.json
pub fn get_claude_settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".claude").join("settings.json"))
}

/// Check if gwt hooks are already registered in settings.json
pub fn is_gwt_hooks_registered(settings_path: &Path) -> bool {
    if !settings_path.exists() {
        return false;
    }

    let content = match std::fs::read_to_string(settings_path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let settings: ClaudeSettings = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Check if at least one gwt hook is registered
    settings.hooks.values().any(|v| {
        if let Some(cmd) = v.as_str() {
            cmd.contains("gwt hook")
        } else if let Some(arr) = v.as_array() {
            arr.iter().any(|item| {
                item.as_str()
                    .map(|s| s.contains("gwt hook"))
                    .unwrap_or(false)
            })
        } else {
            false
        }
    })
}

/// Register gwt hooks in Claude Code settings.json
///
/// This function:
/// 1. Creates ~/.claude directory if it doesn't exist
/// 2. Creates or updates settings.json
/// 3. Adds gwt hook commands for all hook events
/// 4. Preserves existing hook configurations
pub fn register_gwt_hooks(settings_path: &Path) -> Result<(), GwtError> {
    // Create parent directory if needed
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Load existing settings or create new
    let mut settings: ClaudeSettings = if settings_path.exists() {
        let content = std::fs::read_to_string(settings_path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        ClaudeSettings::default()
    };

    // Register hooks for each event
    for event in HOOK_EVENTS {
        let hook_command = format!("gwt hook {}", event);

        // Check if this event already has hooks
        if let Some(existing) = settings.hooks.get_mut(*event) {
            // If existing is a string, convert to array and add gwt hook
            if let Some(cmd) = existing.as_str() {
                if !cmd.contains("gwt hook") {
                    *existing = serde_json::json!([cmd, hook_command]);
                }
            } else if let Some(arr) = existing.as_array_mut() {
                // If array, check if gwt hook already exists
                let has_gwt = arr.iter().any(|v| {
                    v.as_str()
                        .map(|s| s.contains("gwt hook"))
                        .unwrap_or(false)
                });
                if !has_gwt {
                    arr.push(serde_json::json!(hook_command));
                }
            }
        } else {
            // No existing hook for this event, add new
            settings
                .hooks
                .insert(event.to_string(), serde_json::json!(hook_command));
        }
    }

    // Write settings back
    let content = serde_json::to_string_pretty(&settings).map_err(|e| GwtError::ConfigWriteError {
        reason: e.to_string(),
    })?;
    std::fs::write(settings_path, content)?;

    Ok(())
}

/// Unregister gwt hooks from Claude Code settings.json
pub fn unregister_gwt_hooks(settings_path: &Path) -> Result<(), GwtError> {
    if !settings_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(settings_path)?;
    let mut settings: ClaudeSettings =
        serde_json::from_str(&content).map_err(|e| GwtError::ConfigParseError {
            reason: e.to_string(),
        })?;

    // Remove gwt hooks from each event
    for event in HOOK_EVENTS {
        if let Some(existing) = settings.hooks.get_mut(*event) {
            if let Some(cmd) = existing.as_str() {
                if cmd.contains("gwt hook") {
                    settings.hooks.remove(*event);
                    continue;
                }
            } else if let Some(arr) = existing.as_array() {
                let filtered: Vec<_> = arr
                    .iter()
                    .filter(|v| {
                        !v.as_str()
                            .map(|s| s.contains("gwt hook"))
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect();

                if filtered.is_empty() {
                    settings.hooks.remove(*event);
                } else if filtered.len() == 1 {
                    // Convert single-item array back to string
                    settings.hooks.insert(event.to_string(), filtered[0].clone());
                } else {
                    settings
                        .hooks
                        .insert(event.to_string(), serde_json::json!(filtered));
                }
            }
        }
    }

    // Write settings back
    let content = serde_json::to_string_pretty(&settings).map_err(|e| GwtError::ConfigWriteError {
        reason: e.to_string(),
    })?;
    std::fs::write(settings_path, content)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_create_claude_settings_if_not_exists() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        let result = register_gwt_hooks(&settings_path);

        assert!(result.is_ok());
        assert!(settings_path.exists());
    }

    #[test]
    fn test_preserve_existing_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        let existing_content = r#"{"hooks": {"CustomHook": "custom-command"}}"#;
        std::fs::write(&settings_path, existing_content).unwrap();

        let result = register_gwt_hooks(&settings_path);

        assert!(result.is_ok());
        let content = std::fs::read_to_string(&settings_path).unwrap();
        assert!(content.contains("CustomHook"));
        assert!(content.contains("UserPromptSubmit"));
    }

    #[test]
    fn test_register_all_five_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        let result = register_gwt_hooks(&settings_path);

        assert!(result.is_ok());
        let content = std::fs::read_to_string(&settings_path).unwrap();
        assert!(content.contains("UserPromptSubmit"));
        assert!(content.contains("PreToolUse"));
        assert!(content.contains("PostToolUse"));
        assert!(content.contains("Notification"));
        assert!(content.contains("Stop"));
    }

    #[test]
    fn test_detect_missing_gwt_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        let result = is_gwt_hooks_registered(&settings_path);

        assert!(!result);
    }

    #[test]
    fn test_detect_existing_gwt_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        let content = r#"{"hooks": {"UserPromptSubmit": "gwt hook UserPromptSubmit"}}"#;
        std::fs::write(&settings_path, content).unwrap();

        let result = is_gwt_hooks_registered(&settings_path);

        assert!(result);
    }

    #[test]
    fn test_preserve_existing_event_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        // Existing hook for UserPromptSubmit
        let existing_content =
            r#"{"hooks": {"UserPromptSubmit": "echo 'user prompt received'"}}"#;
        std::fs::write(&settings_path, existing_content).unwrap();

        let result = register_gwt_hooks(&settings_path);

        assert!(result.is_ok());
        let content = std::fs::read_to_string(&settings_path).unwrap();
        // Both the existing hook and gwt hook should be present
        assert!(content.contains("echo"));
        assert!(content.contains("gwt hook UserPromptSubmit"));
    }

    #[test]
    fn test_unregister_gwt_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        // First register hooks
        register_gwt_hooks(&settings_path).unwrap();
        assert!(is_gwt_hooks_registered(&settings_path));

        // Then unregister
        unregister_gwt_hooks(&settings_path).unwrap();
        assert!(!is_gwt_hooks_registered(&settings_path));
    }

    #[test]
    fn test_idempotent_registration() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        // Register twice
        register_gwt_hooks(&settings_path).unwrap();
        register_gwt_hooks(&settings_path).unwrap();

        // Should only have one gwt hook per event
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: ClaudeSettings = serde_json::from_str(&content).unwrap();

        // Check UserPromptSubmit is not duplicated
        let user_prompt_hook = settings.hooks.get("UserPromptSubmit").unwrap();
        if let Some(cmd) = user_prompt_hook.as_str() {
            assert_eq!(cmd, "gwt hook UserPromptSubmit");
        } else {
            panic!("Expected string hook, got array");
        }
    }
}
