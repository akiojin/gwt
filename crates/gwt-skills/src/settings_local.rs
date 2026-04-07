//! Generate `.claude/settings.local.json` with gwt-managed Claude hooks.

use serde_json::{json, Map, Value};
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const GWT_FORWARD_SCRIPT: &str = "gwt-forward-hook.mjs";
const GWT_BLOCK_SCRIPT_PREFIX: &str = "gwt-block-";
const GWT_MANAGED_RUNTIME_MARKER: &str = "GWT_MANAGED_HOOK";
const GWT_MANAGED_RUNTIME_KIND: &str = "runtime-state";
const CLAUDE_HOOK_COMMAND_TYPE: &str = "command";
const MANAGED_EVENT_ORDER: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
];
const CODEX_HOOKS_PATH: &str = ".codex/hooks.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookShell {
    Posix,
    PowerShell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedHookTarget {
    Claude,
    Codex,
}

impl ManagedHookTarget {
    fn config_path(self, worktree: &Path) -> PathBuf {
        match self {
            Self::Claude => worktree.join(".claude/settings.local.json"),
            Self::Codex => worktree.join(CODEX_HOOKS_PATH),
        }
    }

    fn script_root(self) -> &'static str {
        match self {
            Self::Claude => ".claude/hooks/scripts",
            Self::Codex => ".codex/hooks/scripts",
        }
    }
}

/// Generate `.claude/settings.local.json` in the target worktree.
///
/// Replaces gwt-managed Claude hook entries while preserving user-defined
/// hook entries and unrelated top-level Claude settings.
pub fn generate_settings_local(worktree: &Path) -> io::Result<()> {
    generate_hook_config(worktree, ManagedHookTarget::Claude)
}

/// Generate `.codex/hooks.json` in the target worktree.
///
/// Tracked Codex hook files are normally preserved, but tracked files that
/// still contain gwt's legacy runtime forward-hook commands are migrated to the
/// current no-Node runtime-hook shape so launched worktrees do not stay pinned
/// to stale hook behavior forever.
pub fn generate_codex_hooks(worktree: &Path) -> io::Result<()> {
    let settings_path = worktree.join(CODEX_HOOKS_PATH);
    if path_is_git_tracked(worktree, Path::new(CODEX_HOOKS_PATH))?
        && !tracked_codex_hooks_need_runtime_migration(&settings_path)?
    {
        return Ok(());
    }
    generate_hook_config(worktree, ManagedHookTarget::Codex)
}

fn generate_hook_config(worktree: &Path, target: ManagedHookTarget) -> io::Result<()> {
    let settings_path = target.config_path(worktree);

    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut root = read_existing_settings(&settings_path)?;
    root.remove("managed_hooks");
    root.remove("user_hooks");

    let user_hooks = existing_user_hooks(root.get("hooks"));
    root.insert(
        "hooks".to_string(),
        Value::Object(merge_managed_and_user_hooks(
            user_hooks,
            target,
            managed_hook_shell(),
        )),
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

    if cfg!(windows) && path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn merge_managed_and_user_hooks(
    user_hooks: Map<String, Value>,
    target: ManagedHookTarget,
    shell: HookShell,
) -> Map<String, Value> {
    let managed_hooks = managed_hooks(target, shell);
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
    command.contains(GWT_FORWARD_SCRIPT)
        || command.contains(GWT_BLOCK_SCRIPT_PREFIX)
        || command.contains(GWT_MANAGED_RUNTIME_MARKER)
}

fn tracked_codex_hooks_need_runtime_migration(path: &Path) -> io::Result<bool> {
    let root = read_existing_settings(path)?;
    let hooks = root.get("hooks");
    Ok(contains_legacy_runtime_forwarder(hooks)
        || contains_managed_runtime_shell_mismatch(hooks, managed_hook_shell()))
}

fn contains_legacy_runtime_forwarder(existing: Option<&Value>) -> bool {
    let Some(Value::Object(events)) = existing else {
        return false;
    };

    MANAGED_EVENT_ORDER.iter().any(|event| {
        events
            .get(*event)
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry
                        .as_object()
                        .and_then(|obj| obj.get("hooks"))
                        .and_then(Value::as_array)
                        .is_some_and(|hooks| {
                            hooks.iter().any(|hook| {
                                hook.as_object()
                                    .and_then(|obj| obj.get("command"))
                                    .and_then(Value::as_str)
                                    .is_some_and(|command| command.contains(GWT_FORWARD_SCRIPT))
                            })
                        })
                })
            })
    })
}

fn contains_managed_runtime_shell_mismatch(existing: Option<&Value>, shell: HookShell) -> bool {
    let Some(Value::Object(events)) = existing else {
        return false;
    };

    MANAGED_EVENT_ORDER.iter().any(|event| {
        events
            .get(*event)
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry
                        .as_object()
                        .and_then(|obj| obj.get("hooks"))
                        .and_then(Value::as_array)
                        .is_some_and(|hooks| {
                            hooks.iter().any(|hook| {
                                hook.as_object()
                                    .and_then(|obj| obj.get("command"))
                                    .and_then(Value::as_str)
                                    .is_some_and(|command| {
                                        command.contains(GWT_MANAGED_RUNTIME_MARKER)
                                            && command_shell_mismatch(command, shell)
                                    })
                            })
                        })
                })
            })
    })
}

fn command_shell_mismatch(command: &str, shell: HookShell) -> bool {
    match shell {
        HookShell::Posix => command.contains("powershell -NoProfile -Command"),
        HookShell::PowerShell => command.contains(" sh -lc '"),
    }
}

fn managed_hooks(target: ManagedHookTarget, shell: HookShell) -> Map<String, Value> {
    let mut hooks = Map::new();
    hooks.insert(
        "SessionStart".to_string(),
        Value::Array(vec![runtime_hook("SessionStart", shell)]),
    );
    hooks.insert(
        "UserPromptSubmit".to_string(),
        Value::Array(vec![runtime_hook("UserPromptSubmit", shell)]),
    );
    hooks.insert(
        "PreToolUse".to_string(),
        Value::Array(vec![
            runtime_hook("PreToolUse", shell),
            bash_blockers_hook(target),
        ]),
    );
    hooks.insert(
        "PostToolUse".to_string(),
        Value::Array(vec![runtime_hook("PostToolUse", shell)]),
    );
    hooks.insert(
        "Stop".to_string(),
        Value::Array(vec![runtime_hook("Stop", shell)]),
    );
    hooks
}

fn runtime_hook(event: &str, shell: HookShell) -> Value {
    json!({
        "matcher": "*",
        "hooks": [
            {
                "command": runtime_hook_command(event, shell),
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            }
        ]
    })
}

fn bash_blockers_hook(target: ManagedHookTarget) -> Value {
    json!({
        "matcher": "Bash",
        "hooks": [
            {
                "command": format!("node {}/gwt-block-git-branch-ops.mjs", target.script_root()),
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            },
            {
                "command": format!("node {}/gwt-block-cd-command.mjs", target.script_root()),
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            },
            {
                "command": format!("node {}/gwt-block-file-ops.mjs", target.script_root()),
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            },
            {
                "command": format!(
                    "node {}/gwt-block-git-dir-override.mjs",
                    target.script_root()
                ),
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            }
        ]
    })
}

fn managed_hook_shell() -> HookShell {
    if cfg!(windows) {
        HookShell::PowerShell
    } else {
        HookShell::Posix
    }
}

fn runtime_hook_command(event: &str, shell: HookShell) -> String {
    let status = runtime_status_for_event(event);
    match shell {
        HookShell::Posix => posix_runtime_hook_command(event, status),
        HookShell::PowerShell => powershell_runtime_hook_command(event, status),
    }
}

fn runtime_status_for_event(event: &str) -> &'static str {
    match event {
        "SessionStart" | "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => "Running",
        "Stop" => "WaitingInput",
        other => panic!("unsupported runtime hook event: {other}"),
    }
}

fn posix_runtime_hook_command(event: &str, status: &str) -> String {
    format!(
        "{GWT_MANAGED_RUNTIME_MARKER}={GWT_MANAGED_RUNTIME_KIND} sh -lc 'runtime_path=\"${{GWT_SESSION_RUNTIME_PATH:-}}\"; [ -n \"$runtime_path\" ] || exit 0; runtime_dir=$(dirname \"$runtime_path\"); mkdir -p \"$runtime_dir\" || exit 0; now=$(date -u +\"%Y-%m-%dT%H:%M:%SZ\"); tmp=\"${{runtime_path}}.tmp.$$\"; printf \"{{\\\"status\\\":\\\"%s\\\",\\\"updated_at\\\":\\\"%s\\\",\\\"last_activity_at\\\":\\\"%s\\\",\\\"source_event\\\":\\\"%s\\\"}}\" \"{status}\" \"$now\" \"$now\" \"{event}\" > \"$tmp\" && mv \"$tmp\" \"$runtime_path\"' || true"
    )
}

fn powershell_runtime_hook_command(event: &str, status: &str) -> String {
    format!(
        "powershell -NoProfile -Command \"& {{ $env:{GWT_MANAGED_RUNTIME_MARKER} = '{GWT_MANAGED_RUNTIME_KIND}'; if ($env:GWT_SESSION_RUNTIME_PATH) {{ $runtimePath = $env:GWT_SESSION_RUNTIME_PATH; $runtimeDir = Split-Path -Parent $runtimePath; New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null; $now = (Get-Date).ToUniversalTime().ToString('o'); $tmp = \\\"$runtimePath.tmp.$PID\\\"; $payload = @{{ status = '{status}'; updated_at = $now; last_activity_at = $now; source_event = '{event}' }} | ConvertTo-Json -Compress; Set-Content -Path $tmp -Value $payload -NoNewline; Move-Item -Force $tmp $runtimePath }} }}\""
    )
}

fn path_is_git_tracked(worktree: &Path, relative_path: &Path) -> io::Result<bool> {
    match Command::new("git")
        .arg("-C")
        .arg(worktree)
        .arg("ls-files")
        .arg("--error-unmatch")
        .arg(relative_path)
        .output()
    {
        Ok(output) => Ok(output.status.success()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn creates_settings_local_with_managed_hooks() {
        let dir = tempfile::tempdir().unwrap();

        generate_settings_local(dir.path()).unwrap();

        let path = dir.path().join(".claude/settings.local.json");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();

        let command = value["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
            .as_str()
            .expect("command string");
        assert!(command.contains("GWT_MANAGED_HOOK"));
        assert!(!command.contains("node"));
        assert!(value["hooks"]["SessionStart"].is_array());
        assert!(value["hooks"].get("Notification").is_none());
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
                                    "command": "GWT_MANAGED_HOOK=runtime-state sh -lc 'echo old-managed'",
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
                .filter(|command| command.contains("GWT_MANAGED_HOOK"))
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

    #[test]
    fn generate_codex_hooks_creates_hooks_json_without_node_runtime_hooks() {
        let dir = tempfile::tempdir().unwrap();

        generate_codex_hooks(dir.path()).unwrap();

        let path = dir.path().join(".codex/hooks.json");
        let content = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();
        let command = value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("command string");

        assert!(command.contains("GWT_MANAGED_HOOK"));
        assert!(!command.contains("node"));
        assert_eq!(
            value["hooks"]["PreToolUse"][1]["matcher"],
            Value::String("Bash".to_string())
        );
    }

    #[test]
    fn generate_codex_hooks_preserves_user_hooks_while_replacing_managed_runtime_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/hooks.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "hooks": {
                    "SessionStart": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": "GWT_MANAGED_HOOK=runtime-state sh -lc 'echo old-managed'",
                                    "type": "command"
                                },
                                {
                                    "command": "my-custom-hook",
                                    "type": "command"
                                }
                            ]
                        }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        generate_codex_hooks(dir.path()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();
        let commands: Vec<&str> = value["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|entry| entry["hooks"].as_array().unwrap().iter())
            .filter_map(|hook| hook["command"].as_str())
            .collect();

        assert_eq!(
            commands
                .iter()
                .filter(|command| command.contains("GWT_MANAGED_HOOK"))
                .count(),
            1
        );
        assert!(commands.contains(&"my-custom-hook"));
    }

    #[test]
    fn generate_codex_hooks_skips_tracked_hooks_json_without_legacy_runtime_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/hooks.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "hooks": {
                    "SessionStart": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": "tracked-command",
                                    "type": "command"
                                }
                            ]
                        }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        assert!(Command::new("git")
            .arg("init")
            .arg(dir.path())
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .arg("add")
            .arg(".codex/hooks.json")
            .status()
            .unwrap()
            .success());

        generate_codex_hooks(dir.path()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("tracked-command"));
        assert!(!content.contains("GWT_MANAGED_HOOK"));
    }

    #[test]
    fn generate_codex_hooks_migrates_tracked_legacy_runtime_hooks_without_node() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/hooks.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "hooks": {
                    "SessionStart": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": "node \"$(git rev-parse --show-toplevel)/.codex/hooks/scripts/gwt-forward-hook.mjs\" SessionStart",
                                    "type": "command"
                                }
                            ]
                        }
                    ],
                    "PreToolUse": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": "node \"$(git rev-parse --show-toplevel)/.codex/hooks/scripts/gwt-forward-hook.mjs\" PreToolUse",
                                    "type": "command"
                                },
                                {
                                    "command": "my-custom-hook",
                                    "type": "command"
                                }
                            ]
                        }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        assert!(Command::new("git")
            .arg("init")
            .arg(dir.path())
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .arg("add")
            .arg(".codex/hooks.json")
            .status()
            .unwrap()
            .success());

        generate_codex_hooks(dir.path()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();
        let session_start_command = value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("session start command");
        let pre_tool_commands: Vec<&str> = value["hooks"]["PreToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|entry| entry["hooks"].as_array().unwrap().iter())
            .filter_map(|hook| hook["command"].as_str())
            .collect();

        assert!(session_start_command.contains("GWT_MANAGED_HOOK"));
        assert!(!content.contains("gwt-forward-hook.mjs"));
        assert!(!session_start_command.contains("node"));
        assert!(pre_tool_commands.contains(&"my-custom-hook"));
    }

    #[test]
    fn generate_codex_hooks_migrates_tracked_runtime_hooks_when_shell_shape_mismatches_host() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/hooks.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let foreign_managed_command = match managed_hook_shell() {
            HookShell::Posix => powershell_runtime_hook_command("SessionStart", "Running"),
            HookShell::PowerShell => posix_runtime_hook_command("SessionStart", "Running"),
        };
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "hooks": {
                    "SessionStart": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": foreign_managed_command,
                                    "type": "command"
                                }
                            ]
                        }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        assert!(Command::new("git")
            .arg("init")
            .arg(dir.path())
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .arg("add")
            .arg(".codex/hooks.json")
            .status()
            .unwrap()
            .success());

        generate_codex_hooks(dir.path()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();
        let session_start_command = value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("session start command");
        let expected = runtime_hook_command("SessionStart", managed_hook_shell());
        assert_eq!(session_start_command, expected);
    }

    #[cfg(not(windows))]
    #[test]
    fn posix_runtime_hook_command_writes_runtime_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let runtime_path = dir
            .path()
            .join("runtime")
            .join("999")
            .join("session-123.json");
        let command = posix_runtime_hook_command("SessionStart", "Running");

        assert!(Command::new("sh")
            .arg("-lc")
            .arg(&command)
            .env("GWT_SESSION_RUNTIME_PATH", &runtime_path)
            .status()
            .unwrap()
            .success());

        let content = fs::read_to_string(&runtime_path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["status"], Value::String("Running".to_string()));
        assert_eq!(
            value["source_event"],
            Value::String("SessionStart".to_string())
        );
    }
}
