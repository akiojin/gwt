//! Legacy Claude Code Hook detection and cleanup (SPEC-861d8cdf)
//!
//! Before the gwt-integration plugin migration, gwt registered hooks directly
//! into `~/.claude/settings.json`.  This module retains only the detection
//! (`is_gwt_hooks_registered`) and removal (`unregister_gwt_hooks`) helpers so
//! that the plugin setup path can automatically clean up leftover legacy entries.

use crate::error::GwtError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

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
    fn test_unregister_gwt_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        // Write a fixture with gwt hooks and a custom hook directly
        let fixture = serde_json::json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "*",
                        "hooks": [{"type": "command", "command": "gwt hook PreToolUse"}]
                    }
                ],
                "PostToolUse": [
                    {
                        "matcher": "*",
                        "hooks": [{"type": "command", "command": "gwt hook PostToolUse"}]
                    }
                ],
                "UserPromptSubmit": [
                    {
                        "hooks": [{"type": "command", "command": "gwt hook UserPromptSubmit"}]
                    },
                    {
                        "hooks": [{"type": "command", "command": "echo custom"}]
                    }
                ],
                "Notification": [
                    {
                        "hooks": [{"type": "command", "command": "gwt hook Notification"}]
                    }
                ],
                "Stop": [
                    {
                        "hooks": [{"type": "command", "command": "gwt hook Stop"}]
                    }
                ]
            }
        });
        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&fixture).unwrap(),
        )
        .unwrap();

        assert!(is_gwt_hooks_registered(&settings_path));

        // Unregister
        unregister_gwt_hooks(&settings_path).unwrap();
        assert!(!is_gwt_hooks_registered(&settings_path));

        // The custom hook should be preserved
        let content = std::fs::read_to_string(&settings_path).unwrap();
        assert!(content.contains("echo custom"));
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
}
