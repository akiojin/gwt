//! Claude Code Hook settings management (SPEC-861d8cdf T-102)
//!
//! This module provides functionality to register gwt hooks in Claude Code's settings.json
//! for agent status tracking.
//!
//! New hooks format (2026+):
//! ```json
//! {
//!   "hooks": {
//!     "PostToolUse": [
//!       {
//!         "matcher": "",
//!         "hooks": [{"type": "command", "command": "gwt hook PostToolUse"}]
//!       }
//!     ]
//!   }
//! }
//! ```

use crate::error::GwtError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Get the absolute path of the gwt executable
///
/// Uses std::env::current_exe() to get the path of the running binary.
/// Falls back to "gwt" if the path cannot be determined.
fn get_gwt_executable_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "gwt".to_string())
}

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
/// Events that require matcher (tool-based): PreToolUse, PostToolUse
/// Events without matcher: UserPromptSubmit, Notification, Stop
pub const HOOK_EVENTS_WITH_MATCHER: &[&str] = &["PreToolUse", "PostToolUse"];
pub const HOOK_EVENTS_WITHOUT_MATCHER: &[&str] = &["UserPromptSubmit", "Notification", "Stop"];

/// Get all hook event types
pub fn all_hook_events() -> impl Iterator<Item = &'static str> {
    HOOK_EVENTS_WITH_MATCHER
        .iter()
        .chain(HOOK_EVENTS_WITHOUT_MATCHER.iter())
        .copied()
}

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

    settings_has_gwt_hooks(&settings)
}

fn settings_has_gwt_hooks(settings: &ClaudeSettings) -> bool {
    settings
        .hooks
        .values()
        .any(|value| value_has_gwt_hook(value))
}

fn value_has_gwt_hook(value: &serde_json::Value) -> bool {
    // New format: array of {matcher, hooks}
    if let Some(arr) = value.as_array() {
        arr.iter().any(|entry| {
            if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
                hooks.iter().any(|hook| {
                    hook.get("command")
                        .and_then(|c| c.as_str())
                        .map(|s| s.contains("gwt hook"))
                        .unwrap_or(false)
                })
            } else {
                // Legacy: array of strings
                entry
                    .as_str()
                    .map(|s| s.contains("gwt hook"))
                    .unwrap_or(false)
            }
        })
    } else if let Some(cmd) = value.as_str() {
        // Legacy: single string
        cmd.contains("gwt hook")
    } else {
        false
    }
}

/// Create a gwt hook entry with matcher (for PreToolUse, PostToolUse)
fn create_gwt_hook_entry_with_matcher(event: &str, exe_path: &str) -> serde_json::Value {
    serde_json::json!({
        "matcher": "*",
        "hooks": [{
            "type": "command",
            "command": format!("{} hook {}", exe_path, event)
        }]
    })
}

/// Create a gwt hook entry without matcher (for UserPromptSubmit, Notification, Stop)
fn create_gwt_hook_entry_without_matcher(event: &str, exe_path: &str) -> serde_json::Value {
    serde_json::json!({
        "hooks": [{
            "type": "command",
            "command": format!("{} hook {}", exe_path, event)
        }]
    })
}

/// Check if an array contains a gwt hook (new format)
fn has_gwt_hook_in_array(arr: &[serde_json::Value]) -> bool {
    arr.iter().any(|entry| {
        if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(|c| c.as_str())
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
/// 3. Adds gwt hook commands for all hook events (new format)
/// 4. Preserves existing hook configurations
pub fn register_gwt_hooks(settings_path: &Path) -> Result<(), GwtError> {
    let exe_path = get_gwt_executable_path();
    register_gwt_hooks_with_exe_path(settings_path, &exe_path)
}

/// Internal function to register hooks with a specified executable path
fn register_gwt_hooks_with_exe_path(settings_path: &Path, exe_path: &str) -> Result<(), GwtError> {
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

    // Helper to register a single event
    let register_event =
        |settings: &mut ClaudeSettings, event: &str, gwt_entry: serde_json::Value| {
            if let Some(existing) = settings.hooks.get_mut(event) {
                if let Some(arr) = existing.as_array_mut() {
                    if !has_gwt_hook_in_array(arr) {
                        arr.push(gwt_entry);
                    }
                } else {
                    settings
                        .hooks
                        .insert(event.to_string(), serde_json::json!([gwt_entry]));
                }
            } else {
                settings
                    .hooks
                    .insert(event.to_string(), serde_json::json!([gwt_entry]));
            }
        };

    // Register hooks with matcher (PreToolUse, PostToolUse)
    for event in HOOK_EVENTS_WITH_MATCHER {
        let gwt_entry = create_gwt_hook_entry_with_matcher(event, exe_path);
        register_event(&mut settings, event, gwt_entry);
    }

    // Register hooks without matcher (UserPromptSubmit, Notification, Stop)
    for event in HOOK_EVENTS_WITHOUT_MATCHER {
        let gwt_entry = create_gwt_hook_entry_without_matcher(event, exe_path);
        register_event(&mut settings, event, gwt_entry);
    }

    // Write settings back
    let content =
        serde_json::to_string_pretty(&settings).map_err(|e| GwtError::ConfigWriteError {
            reason: e.to_string(),
        })?;
    std::fs::write(settings_path, content)?;

    Ok(())
}

/// Re-register gwt hooks to update the executable path.
/// Returns true when hooks were re-registered.
pub fn reregister_gwt_hooks(settings_path: &Path) -> Result<bool, GwtError> {
    let exe_path = get_gwt_executable_path();
    reregister_gwt_hooks_with_exe_path(settings_path, &exe_path)
}

fn reregister_gwt_hooks_with_exe_path(
    settings_path: &Path,
    exe_path: &str,
) -> Result<bool, GwtError> {
    if !settings_path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(settings_path)?;
    let settings: ClaudeSettings =
        serde_json::from_str(&content).map_err(|e| GwtError::ConfigParseError {
            reason: e.to_string(),
        })?;

    if !settings_has_gwt_hooks(&settings) {
        return Ok(false);
    }

    unregister_gwt_hooks(settings_path)?;
    register_gwt_hooks_with_exe_path(settings_path, exe_path)?;
    Ok(true)
}

/// Check if an entry is a gwt hook (new format)
fn is_gwt_hook_entry(entry: &serde_json::Value) -> bool {
    if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
        hooks.iter().any(|hook| {
            hook.get("command")
                .and_then(|c| c.as_str())
                .map(|s| s.contains("gwt hook"))
                .unwrap_or(false)
        })
    } else {
        false
    }
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
    for event in all_hook_events() {
        if let Some(existing) = settings.hooks.get_mut(event) {
            if let Some(arr) = existing.as_array() {
                // New format: filter out gwt hook entries
                let filtered: Vec<_> = arr
                    .iter()
                    .filter(|entry| !is_gwt_hook_entry(entry))
                    .cloned()
                    .collect();

                if filtered.is_empty() {
                    settings.hooks.remove(event);
                } else {
                    settings
                        .hooks
                        .insert(event.to_string(), serde_json::json!(filtered));
                }
            } else if let Some(cmd) = existing.as_str() {
                // Legacy format: single string
                if cmd.contains("gwt hook") {
                    settings.hooks.remove(event);
                }
            }
        }
    }

    // Write settings back
    let content =
        serde_json::to_string_pretty(&settings).map_err(|e| GwtError::ConfigWriteError {
            reason: e.to_string(),
        })?;
    std::fs::write(settings_path, content)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Fixed executable path for tests
    const TEST_EXE_PATH: &str = "gwt";

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
    fn test_detect_existing_gwt_hooks_new_format() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        let content = r#"{"hooks": {"UserPromptSubmit": [{"matcher": "", "hooks": [{"type": "command", "command": "gwt hook UserPromptSubmit"}]}]}}"#;
        std::fs::write(&settings_path, content).unwrap();

        let result = is_gwt_hooks_registered(&settings_path);

        assert!(result);
    }

    #[test]
    fn test_detect_existing_gwt_hooks_legacy_format() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        // Legacy format (string) should still be detected
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

        // Existing hook for UserPromptSubmit in new format
        let existing_content = r#"{"hooks": {"UserPromptSubmit": [{"matcher": "", "hooks": [{"type": "command", "command": "echo 'user prompt received'"}]}]}}"#;
        std::fs::write(&settings_path, existing_content).unwrap();

        let result = register_gwt_hooks_with_exe_path(&settings_path, TEST_EXE_PATH);

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
        register_gwt_hooks_with_exe_path(&settings_path, TEST_EXE_PATH).unwrap();
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
        register_gwt_hooks_with_exe_path(&settings_path, TEST_EXE_PATH).unwrap();
        register_gwt_hooks_with_exe_path(&settings_path, TEST_EXE_PATH).unwrap();

        // Should only have one gwt hook per event
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: ClaudeSettings = serde_json::from_str(&content).unwrap();

        // Check UserPromptSubmit is not duplicated (new format: array with one entry)
        let user_prompt_hook = settings.hooks.get("UserPromptSubmit").unwrap();
        let arr = user_prompt_hook.as_array().expect("Expected array format");
        // Should have exactly one entry (not duplicated)
        assert_eq!(arr.len(), 1);
        // Verify it's the gwt hook
        assert!(has_gwt_hook_in_array(arr));
    }

    #[test]
    fn test_new_format_structure_with_matcher() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        register_gwt_hooks_with_exe_path(&settings_path, TEST_EXE_PATH).unwrap();

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: ClaudeSettings = serde_json::from_str(&content).unwrap();

        // PreToolUse should have matcher
        let pre_tool_hook = settings.hooks.get("PreToolUse").unwrap();
        let arr = pre_tool_hook.as_array().expect("Should be array");
        let entry = &arr[0];

        // Check matcher exists and is "*"
        let matcher = entry
            .get("matcher")
            .expect("PreToolUse should have matcher");
        assert_eq!(matcher.as_str().unwrap(), "*");

        // Check hooks array exists with command entry
        let hooks = entry.get("hooks").unwrap().as_array().unwrap();
        let hook = &hooks[0];
        assert_eq!(hook.get("type").unwrap().as_str().unwrap(), "command");
        assert!(hook
            .get("command")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("gwt hook PreToolUse"));
    }

    #[test]
    fn test_new_format_structure_without_matcher() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        register_gwt_hooks_with_exe_path(&settings_path, TEST_EXE_PATH).unwrap();

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: ClaudeSettings = serde_json::from_str(&content).unwrap();

        // UserPromptSubmit should NOT have matcher
        let user_prompt_hook = settings.hooks.get("UserPromptSubmit").unwrap();
        let arr = user_prompt_hook.as_array().expect("Should be array");
        let entry = &arr[0];

        // Check matcher does NOT exist
        assert!(
            entry.get("matcher").is_none(),
            "UserPromptSubmit should not have matcher"
        );

        // Check hooks array exists with command entry
        let hooks = entry.get("hooks").unwrap().as_array().unwrap();
        let hook = &hooks[0];
        assert_eq!(hook.get("type").unwrap().as_str().unwrap(), "command");
        assert!(hook
            .get("command")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("gwt hook UserPromptSubmit"));
    }

    #[test]
    fn test_reregister_updates_exe_path_and_preserves_custom_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        register_gwt_hooks_with_exe_path(&settings_path, "old-gwt").unwrap();

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let mut settings: ClaudeSettings = serde_json::from_str(&content).unwrap();
        let user_prompt_hook = settings.hooks.get_mut("UserPromptSubmit").unwrap();
        let arr = user_prompt_hook
            .as_array_mut()
            .expect("Expected array format");
        arr.push(serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": "echo custom"
            }]
        }));
        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&settings).unwrap(),
        )
        .unwrap();

        let changed = reregister_gwt_hooks_with_exe_path(&settings_path, "new-gwt").unwrap();
        assert!(changed);

        let updated = std::fs::read_to_string(&settings_path).unwrap();
        assert!(updated.contains("new-gwt hook UserPromptSubmit"));
        assert!(!updated.contains("old-gwt hook UserPromptSubmit"));
        assert!(updated.contains("new-gwt hook PreToolUse"));
        assert!(updated.contains("new-gwt hook Stop"));
        assert!(updated.contains("echo custom"));
    }

    #[test]
    fn test_reregister_skips_when_no_gwt_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        let content = r#"{"hooks": {"CustomHook": "custom-command"}}"#;
        std::fs::write(&settings_path, content).unwrap();
        let before = std::fs::read_to_string(&settings_path).unwrap();

        let changed = reregister_gwt_hooks_with_exe_path(&settings_path, "new-gwt").unwrap();
        assert!(!changed);

        let after = std::fs::read_to_string(&settings_path).unwrap();
        assert_eq!(after, before);
    }
}
