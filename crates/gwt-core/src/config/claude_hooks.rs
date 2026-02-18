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

/// Patterns indicating temporary execution environments (bunx, npx, etc.)
/// FR-102i: Detect when gwt is running from a cache directory
const TEMP_EXECUTION_PATTERNS: &[&str] = &[
    ".bun/install/cache/",
    "/tmp/bunx-",
    "/.npm/_npx/",
    "node_modules/.cache/",
];

/// Check if gwt is running from a temporary execution environment (bunx, npx, etc.)
///
/// Returns Some(exe_path) if running from a temporary location, None otherwise.
/// FR-102i: Used to warn users that hooks may not work correctly.
pub fn is_temporary_execution() -> Option<String> {
    let exe_path = get_gwt_executable_path();
    is_temporary_execution_path(&exe_path)
}

/// Check if the given path indicates a temporary execution environment
///
/// This is a separate function for testability.
pub fn is_temporary_execution_path(exe_path: &str) -> Option<String> {
    // Normalize path separators so the pattern checks work on Windows too.
    // `current_exe()` usually returns backslashes on Windows.
    let normalized = exe_path.replace('\\', "/");
    for pattern in TEMP_EXECUTION_PATTERNS {
        if normalized.contains(pattern) {
            return Some(exe_path.to_string());
        }
    }
    None
}

const HOOK_COMMAND_DELIMITER: &str = " hook ";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedGwtHookCommand {
    executable_identity: String,
    event: String,
}

fn strip_wrapping_quotes(value: &str) -> &str {
    let trimmed = value.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(trimmed)
}

fn strip_exe_suffix(value: &str) -> &str {
    let lowercase = value.to_ascii_lowercase();
    if lowercase.ends_with(".exe") {
        &value[..value.len().saturating_sub(4)]
    } else {
        value
    }
}

fn normalize_command_executable_path(executable: &str) -> String {
    let normalized = strip_wrapping_quotes(executable).replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');

    if trimmed.is_empty() {
        return String::new();
    }

    if let Some((dir, file_name)) = trimmed.rsplit_once('/') {
        format!("{dir}/{}", strip_exe_suffix(file_name))
    } else {
        strip_exe_suffix(trimmed).to_string()
    }
}

fn command_executable_name(executable: &str) -> Option<&str> {
    let trimmed = executable.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    trimmed.rsplit('/').next().filter(|name| !name.is_empty())
}

fn is_gwt_executable_name(executable_name: &str) -> bool {
    let normalized = strip_exe_suffix(executable_name);
    let lower = normalized.to_ascii_lowercase();
    lower == "gwt"
        || lower
            .strip_prefix("gwt-")
            .map(|suffix| !suffix.is_empty())
            .unwrap_or(false)
}

fn parse_gwt_hook_command(command: &str) -> Option<ParsedGwtHookCommand> {
    let (executable, event) = command.trim().split_once(HOOK_COMMAND_DELIMITER)?;
    let event = event.trim();
    if event.is_empty() {
        return None;
    }

    let executable_identity = normalize_command_executable_path(executable);
    let executable_name = command_executable_name(&executable_identity)?;
    if !is_gwt_executable_name(executable_name) {
        return None;
    }

    Some(ParsedGwtHookCommand {
        executable_identity,
        event: event.to_string(),
    })
}

fn is_expected_gwt_hook_command(command: &str, event: &str, exe_path: &str) -> bool {
    let Some(parsed) = parse_gwt_hook_command(command) else {
        return false;
    };
    if parsed.event != event {
        return false;
    }
    parsed.executable_identity == normalize_command_executable_path(exe_path)
}

fn gwt_hook_commands_from_value(value: &serde_json::Value) -> Vec<String> {
    let mut commands = Vec::new();

    if let Some(arr) = value.as_array() {
        for entry in arr {
            if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
                for hook in hooks {
                    if let Some(command) = hook.get("command").and_then(|c| c.as_str()) {
                        if is_gwt_hook_command(command) {
                            commands.push(command.to_string());
                        }
                    }
                }
            } else if let Some(command) = entry.as_str() {
                if is_gwt_hook_command(command) {
                    commands.push(command.to_string());
                }
            }
        }
    } else if let Some(command) = value.as_str() {
        if is_gwt_hook_command(command) {
            commands.push(command.to_string());
        }
    }

    commands
}

fn event_has_expected_gwt_hooks(value: &serde_json::Value, event: &str, exe_path: &str) -> bool {
    let commands = gwt_hook_commands_from_value(value);
    !commands.is_empty()
        && commands
            .iter()
            .all(|command| is_expected_gwt_hook_command(command, event, exe_path))
}

fn settings_has_expected_gwt_hooks(settings: &ClaudeSettings, exe_path: &str) -> bool {
    all_hook_events().all(|event| {
        settings
            .hooks
            .get(event)
            .map(|value| event_has_expected_gwt_hooks(value, event, exe_path))
            .unwrap_or(false)
    })
}

/// Check if a command string is a gwt hook command (FR-102j)
///
/// Matches standard format ("gwt hook"), Windows ".exe" paths, and build binary
/// format (".../gwt-<hash> hook <Event>").
fn is_gwt_hook_command(command: &str) -> bool {
    parse_gwt_hook_command(command).is_some()
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
    settings.hooks.values().any(value_has_gwt_hook)
}

fn value_has_gwt_hook(value: &serde_json::Value) -> bool {
    !gwt_hook_commands_from_value(value).is_empty()
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
#[cfg(test)]
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
/// Always overwrites existing gwt hooks with the new executable path
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

    // Helper to register a single event (always overwrite existing gwt hooks)
    let register_event =
        |settings: &mut ClaudeSettings, event: &str, gwt_entry: serde_json::Value| {
            if let Some(existing) = settings.hooks.get_mut(event) {
                if let Some(arr) = existing.as_array_mut() {
                    // Remove all existing gwt hooks, then add the new one
                    arr.retain(|entry| !is_gwt_hook_entry(entry));
                    arr.push(gwt_entry);
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

    if settings_has_expected_gwt_hooks(&settings, exe_path) {
        return Ok(false);
    }

    unregister_gwt_hooks(settings_path)?;
    register_gwt_hooks_with_exe_path(settings_path, exe_path)?;
    Ok(true)
}

/// Check if an entry is a gwt hook (new format) (FR-102j)
fn is_gwt_hook_entry(entry: &serde_json::Value) -> bool {
    if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
        hooks.iter().any(|hook| {
            hook.get("command")
                .and_then(|c| c.as_str())
                .map(is_gwt_hook_command)
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
    fn test_detect_existing_gwt_hooks_windows_exe_format() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        let content = serde_json::json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": r"C:\Users\user\AppData\Local\gwt\gwt.exe hook PreToolUse"
                    }]
                }]
            }
        });
        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&content).unwrap(),
        )
        .unwrap();

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

        register_gwt_hooks_with_exe_path(&settings_path, "gwt-old").unwrap();

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

        let changed = reregister_gwt_hooks_with_exe_path(&settings_path, "gwt-new").unwrap();
        assert!(changed);

        let updated = std::fs::read_to_string(&settings_path).unwrap();
        assert!(updated.contains("gwt-new hook UserPromptSubmit"));
        assert!(!updated.contains("gwt-old hook UserPromptSubmit"));
        assert!(updated.contains("gwt-new hook PreToolUse"));
        assert!(updated.contains("gwt-new hook Stop"));
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

        let changed = reregister_gwt_hooks_with_exe_path(&settings_path, "gwt-new").unwrap();
        assert!(!changed);

        let after = std::fs::read_to_string(&settings_path).unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn test_reregister_skips_when_hooks_already_match_exe_path() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        register_gwt_hooks_with_exe_path(&settings_path, r"C:\Program Files\gwt\gwt.exe").unwrap();
        let before = std::fs::read_to_string(&settings_path).unwrap();

        let changed =
            reregister_gwt_hooks_with_exe_path(&settings_path, r"C:/Program Files/gwt/gwt")
                .unwrap();
        assert!(!changed);

        let after = std::fs::read_to_string(&settings_path).unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn test_overwrite_registration_with_different_paths() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        // Register with first path
        register_gwt_hooks_with_exe_path(&settings_path, "/path/to/gwt-old").unwrap();

        // Register with different path - should OVERWRITE (not add duplicate)
        register_gwt_hooks_with_exe_path(&settings_path, "/path/to/gwt-new").unwrap();

        // Should only have one gwt hook per event (not duplicated)
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: ClaudeSettings = serde_json::from_str(&content).unwrap();

        // Check UserPromptSubmit is not duplicated
        let user_prompt_hook = settings.hooks.get("UserPromptSubmit").unwrap();
        let arr = user_prompt_hook.as_array().expect("Expected array format");
        assert_eq!(
            arr.len(),
            1,
            "Should have exactly one entry, not duplicated"
        );

        // Check PreToolUse is not duplicated
        let pre_tool_hook = settings.hooks.get("PreToolUse").unwrap();
        let arr = pre_tool_hook.as_array().expect("Expected array format");
        assert_eq!(
            arr.len(),
            1,
            "Should have exactly one entry, not duplicated"
        );

        // Verify the new path overwrites the old one
        assert!(!content.contains("/path/to/gwt-old"));
        assert!(content.contains("/path/to/gwt-new"));
    }

    // T-102-05: Temporary execution detection tests (FR-102i)

    #[test]
    fn test_is_temporary_execution_bunx() {
        // bunx cache path should be detected
        let exe_path = "/home/user/.bun/install/cache/@akiojin/gwt@1.0.0/gwt";
        assert!(is_temporary_execution_path(exe_path).is_some());
    }

    #[test]
    fn test_is_temporary_execution_npx() {
        // npx cache path should be detected
        let exe_path = "/home/user/.npm/_npx/12345/node_modules/@akiojin/gwt/gwt";
        assert!(is_temporary_execution_path(exe_path).is_some());
    }

    #[test]
    fn test_is_temporary_execution_tmp_bunx() {
        // /tmp/bunx- path should be detected
        let exe_path = "/tmp/bunx-abc123/gwt";
        assert!(is_temporary_execution_path(exe_path).is_some());
    }

    #[test]
    fn test_is_temporary_execution_node_modules_cache() {
        // node_modules/.cache path should be detected
        let exe_path = "/project/node_modules/.cache/gwt/gwt";
        assert!(is_temporary_execution_path(exe_path).is_some());
    }

    #[test]
    fn test_is_temporary_execution_windows_path_separators() {
        // Windows paths typically use backslashes; normalize before matching.
        let exe_path = r"C:\Users\user\.bun\install\cache\@akiojin\gwt@1.0.0\gwt.exe";
        assert!(is_temporary_execution_path(exe_path).is_some());
    }

    #[test]
    fn test_is_temporary_execution_global_install() {
        // Global install should NOT be detected as temporary
        let exe_path = "/usr/local/bin/gwt";
        assert!(is_temporary_execution_path(exe_path).is_none());
    }

    #[test]
    fn test_is_temporary_execution_local_dev() {
        // Local development build should NOT be detected as temporary
        let exe_path = "/home/user/projects/gwt/target/release/gwt";
        assert!(is_temporary_execution_path(exe_path).is_none());
    }

    #[test]
    fn test_is_temporary_execution_returns_path() {
        // Should return the executable path when detected
        let exe_path = "/home/user/.bun/install/cache/@akiojin/gwt@1.0.0/gwt";
        let result = is_temporary_execution_path(exe_path);
        assert_eq!(result, Some(exe_path.to_string()));
    }

    // T-102-06: gwt hook detection pattern tests (FR-102j)

    #[test]
    fn test_is_gwt_hook_command_standard() {
        // Standard format: "gwt hook <event>"
        assert!(is_gwt_hook_command("gwt hook PreToolUse"));
        assert!(is_gwt_hook_command("/usr/bin/gwt hook UserPromptSubmit"));
    }

    #[test]
    fn test_is_gwt_hook_command_build_binary() {
        // Build binary format: gwt-HASH (from cargo target directory)
        assert!(is_gwt_hook_command(
            "/gwt/target/release/deps/gwt-614ba193345891eb hook PreToolUse"
        ));
        assert!(is_gwt_hook_command(
            "/home/user/gwt/target/debug/gwt-abc123 hook Stop"
        ));
    }

    #[test]
    fn test_is_gwt_hook_command_windows_exe_path() {
        assert!(is_gwt_hook_command(
            r"C:\Users\user\AppData\Local\gwt\gwt.exe hook PreToolUse"
        ));
    }

    #[test]
    fn test_is_gwt_hook_command_windows_quoted_exe_path() {
        assert!(is_gwt_hook_command(
            r#""C:\Program Files\gwt\gwt.exe" hook Stop"#
        ));
    }

    #[test]
    fn test_is_gwt_hook_command_windows_build_binary_exe() {
        assert!(is_gwt_hook_command(
            r"C:\gwt\target\release\deps\gwt-abc123def456.exe hook Notification"
        ));
    }

    #[test]
    fn test_is_gwt_hook_command_not_gwt() {
        // Should not match non-gwt commands
        assert!(!is_gwt_hook_command("echo hello"));
        assert!(!is_gwt_hook_command("other-tool hook PreToolUse"));
        // "hook" without "/gwt" path should not match
        assert!(!is_gwt_hook_command("/some/path hook something"));
    }

    #[test]
    fn test_is_gwt_hook_entry_standard_format() {
        let entry = serde_json::json!({
            "hooks": [{"type": "command", "command": "gwt hook PreToolUse"}]
        });
        assert!(is_gwt_hook_entry(&entry));
    }

    #[test]
    fn test_is_gwt_hook_entry_build_binary_format() {
        // Should detect build binary format (FR-102j)
        let entry = serde_json::json!({
            "hooks": [{"type": "command", "command": "/gwt/target/release/deps/gwt-614ba193345891eb hook PreToolUse"}]
        });
        assert!(is_gwt_hook_entry(&entry));
    }

    #[test]
    fn test_is_gwt_hook_entry_non_gwt() {
        let entry = serde_json::json!({
            "hooks": [{"type": "command", "command": "echo hello"}]
        });
        assert!(!is_gwt_hook_entry(&entry));
    }

    #[test]
    fn test_no_duplicate_when_registering_build_binary_path() {
        // FR-102j: Build binary paths should be detected to prevent duplicate registration
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");

        // Register with build binary path (gwt-HASH format)
        register_gwt_hooks_with_exe_path(
            &settings_path,
            "/gwt/target/release/deps/gwt-614ba193345891eb",
        )
        .unwrap();

        // Register again with different hash - should overwrite, not duplicate
        register_gwt_hooks_with_exe_path(
            &settings_path,
            "/gwt/target/release/deps/gwt-abc123def456",
        )
        .unwrap();

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: ClaudeSettings = serde_json::from_str(&content).unwrap();

        // Count PreToolUse entries - should be exactly 1
        let pre_tool_use = settings
            .hooks
            .get("PreToolUse")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            pre_tool_use.len(),
            1,
            "Should have exactly one entry, not duplicated"
        );

        // Verify the new hash is present, not the old one
        assert!(!content.contains("gwt-614ba193345891eb"));
        assert!(content.contains("gwt-abc123def456"));
    }
}
