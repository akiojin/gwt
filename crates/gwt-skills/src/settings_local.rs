//! Generate `.claude/settings.local.json` with gwt-managed Claude hooks.

use serde_json::{json, Map, Value};
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;

const GWT_FORWARD_SCRIPT: &str = "gwt-forward-hook.mjs";
const GWT_BLOCK_SCRIPT_PREFIX: &str = "gwt-block-";
const CLAUDE_HOOK_COMMAND_TYPE: &str = "command";
const MANAGED_EVENT_ORDER: &[&str] = &[
    "Notification",
    "PostToolUse",
    "PreToolUse",
    "Stop",
    "UserPromptSubmit",
];

/// Generate `.claude/settings.local.json` in the target worktree.
///
/// Replaces gwt-managed Claude hook entries while preserving user-defined
/// hook entries and unrelated top-level Claude settings.
pub fn generate_settings_local(worktree: &Path) -> io::Result<()> {
    let settings_path = worktree.join(".claude/settings.local.json");

    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut root = read_existing_settings(&settings_path)?;
    root.remove("managed_hooks");
    root.remove("user_hooks");

    let user_hooks = existing_user_hooks(root.get("hooks"));
    root.insert(
        "hooks".to_string(),
        Value::Object(merge_managed_and_user_hooks(user_hooks)),
    );

    write_settings_atomically(&settings_path, &Value::Object(root))
}

fn read_existing_settings(path: &Path) -> io::Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }

    let content = fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(Map::new());
    }

    match serde_json::from_str::<Value>(&content) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) | Err(_) => Ok(Map::new()),
    }
}

fn write_settings_atomically(path: &Path, value: &Value) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_path = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("settings.local.json"),
        std::process::id()
    ));
    let json = serde_json::to_string_pretty(value)
        .map_err(|err| io::Error::other(format!("settings.local.json serialize failed: {err}")))?;

    {
        let mut tmp = fs::File::create(&tmp_path)?;
        tmp.write_all(json.as_bytes())?;
        tmp.write_all(b"\n")?;
        tmp.sync_all()?;
    }

    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn merge_managed_and_user_hooks(user_hooks: Map<String, Value>) -> Map<String, Value> {
    let managed_hooks = managed_hooks();
    let mut merged = Map::new();

    for event in MANAGED_EVENT_ORDER {
        let mut entries = managed_hooks
            .get(*event)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(Value::Array(user_entries)) = user_hooks.get(*event) {
            entries.extend(user_entries.clone());
        }
        merged.insert((*event).to_string(), Value::Array(entries));
    }

    for (event, value) in user_hooks {
        if !merged.contains_key(&event) {
            merged.insert(event, value);
        }
    }

    merged
}

fn existing_user_hooks(existing: Option<&Value>) -> Map<String, Value> {
    let Some(Value::Object(events)) = existing else {
        return Map::new();
    };

    let mut sanitized = Map::new();
    for (event, value) in events {
        let Some(entries) = value.as_array() else {
            continue;
        };

        let mut kept_entries = Vec::new();
        for entry in entries {
            let Some(entry_obj) = entry.as_object() else {
                kept_entries.push(entry.clone());
                continue;
            };

            let Some(hooks) = entry_obj.get("hooks").and_then(Value::as_array) else {
                kept_entries.push(entry.clone());
                continue;
            };

            let filtered_hooks: Vec<Value> = hooks
                .iter()
                .filter(|hook| {
                    hook.as_object()
                        .and_then(|obj| obj.get("command"))
                        .and_then(Value::as_str)
                        .is_none_or(|command| !is_gwt_managed_command(command))
                })
                .cloned()
                .collect();

            if filtered_hooks.is_empty() {
                continue;
            }

            let mut filtered_entry = entry_obj.clone();
            filtered_entry.insert("hooks".to_string(), Value::Array(filtered_hooks));
            kept_entries.push(Value::Object(filtered_entry));
        }

        if !kept_entries.is_empty() {
            sanitized.insert(event.clone(), Value::Array(kept_entries));
        }
    }

    sanitized
}

fn is_gwt_managed_command(command: &str) -> bool {
    command.contains(GWT_FORWARD_SCRIPT) || command.contains(GWT_BLOCK_SCRIPT_PREFIX)
}

fn managed_hooks() -> Map<String, Value> {
    let mut hooks = Map::new();
    hooks.insert(
        "Notification".to_string(),
        Value::Array(vec![forward_hook("Notification")]),
    );
    hooks.insert(
        "PostToolUse".to_string(),
        Value::Array(vec![forward_hook("PostToolUse")]),
    );
    hooks.insert(
        "PreToolUse".to_string(),
        Value::Array(vec![forward_hook("PreToolUse"), bash_blockers_hook()]),
    );
    hooks.insert("Stop".to_string(), Value::Array(vec![forward_hook("Stop")]));
    hooks.insert(
        "UserPromptSubmit".to_string(),
        Value::Array(vec![forward_hook("UserPromptSubmit")]),
    );
    hooks
}

fn forward_hook(event: &str) -> Value {
    json!({
        "matcher": "*",
        "hooks": [
            {
                "command": format!("node .claude/hooks/scripts/{GWT_FORWARD_SCRIPT} {event}"),
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            }
        ]
    })
}

fn bash_blockers_hook() -> Value {
    json!({
        "matcher": "Bash",
        "hooks": [
            {
                "command": "node .claude/hooks/scripts/gwt-block-git-branch-ops.mjs",
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            },
            {
                "command": "node .claude/hooks/scripts/gwt-block-cd-command.mjs",
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            },
            {
                "command": "node .claude/hooks/scripts/gwt-block-file-ops.mjs",
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            },
            {
                "command": "node .claude/hooks/scripts/gwt-block-git-dir-override.mjs",
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_settings_local_with_managed_hooks() {
        let dir = tempfile::tempdir().unwrap();

        generate_settings_local(dir.path()).unwrap();

        let path = dir.path().join(".claude/settings.local.json");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(
            value["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
            Value::String(
                "node .claude/hooks/scripts/gwt-forward-hook.mjs UserPromptSubmit".to_string()
            )
        );
        assert_eq!(
            value["hooks"]["PreToolUse"][1]["matcher"],
            Value::String("Bash".to_string())
        );
    }

    #[test]
    fn preserves_existing_user_hooks_while_replacing_gwt_managed_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude/settings.local.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "hooks": {
                    "PreToolUse": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": "node .claude/hooks/scripts/gwt-forward-hook.mjs PreToolUse",
                                    "type": "command"
                                },
                                {
                                    "command": "my-custom-hook",
                                    "type": "command"
                                }
                            ]
                        }
                    ],
                    "CustomEvent": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": "my-custom-event-hook",
                                    "type": "command"
                                }
                            ]
                        }
                    ]
                },
                "permissions": {
                    "allow": ["Bash"]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        generate_settings_local(dir.path()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();
        let pre_tool = value["hooks"]["PreToolUse"].as_array().unwrap();
        let commands: Vec<&str> = pre_tool
            .iter()
            .flat_map(|entry| entry["hooks"].as_array().unwrap().iter())
            .filter_map(|hook| hook["command"].as_str())
            .collect();

        assert_eq!(
            commands
                .iter()
                .filter(|command| command.contains("gwt-forward-hook.mjs PreToolUse"))
                .count(),
            1
        );
        assert!(commands.contains(&"my-custom-hook"));
        assert_eq!(
            value["hooks"]["CustomEvent"][0]["hooks"][0]["command"],
            Value::String("my-custom-event-hook".to_string())
        );
        assert_eq!(
            value["permissions"]["allow"][0],
            Value::String("Bash".to_string())
        );
    }

    #[test]
    fn normalizes_legacy_wrong_schema_into_claude_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude/settings.local.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "managed_hooks": [],
                "user_hooks": []
            }))
            .unwrap(),
        )
        .unwrap();

        generate_settings_local(dir.path()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();
        assert!(value.get("managed_hooks").is_none());
        assert!(value.get("user_hooks").is_none());
        assert!(value["hooks"]["Stop"].is_array());
    }

    #[test]
    fn creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude/settings.local.json");
        assert!(!path.exists());

        generate_settings_local(dir.path()).unwrap();

        assert!(path.exists());
    }
}
