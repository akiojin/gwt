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
/// SPEC #1942 (CORE-CLI): every managed hook dispatched through the
/// new `gwt hook ...` CLI surface carries this substring. Used by
/// [`is_gwt_managed_command`] to recognise new-form entries as
/// managed so that regeneration replaces them in place instead of
/// preserving them as "user hooks" and appending fresh duplicates.
const GWT_HOOK_CLI_PREFIX: &str = "gwt hook ";
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
        || command.contains(GWT_HOOK_CLI_PREFIX)
}

fn tracked_codex_hooks_need_runtime_migration(path: &Path) -> io::Result<bool> {
    let root = read_existing_settings(path)?;
    let hooks = root.get("hooks");
    Ok(contains_legacy_runtime_forwarder(hooks)
        || contains_managed_runtime_shell_mismatch(hooks, managed_hook_shell())
        || contains_legacy_node_bash_blockers(hooks)
        || contains_inline_shell_runtime_hook(hooks))
}

/// SPEC #1942: tracked Codex / Claude hook files that still dispatch
/// bash blockers through Node scripts (`node .../gwt-block-*.mjs`)
/// must be migrated to the new `gwt hook block-*` form on the next
/// regeneration pass. Without this, tracking the file causes the
/// generator to short-circuit and the migration never completes.
fn contains_legacy_node_bash_blockers(existing: Option<&Value>) -> bool {
    any_managed_command(existing, |command| {
        command.contains(GWT_BLOCK_SCRIPT_PREFIX)
    })
}

/// SPEC #1942: detect tracked hook files that still carry the old
/// `GWT_MANAGED_HOOK=runtime-state sh -lc '...'` inline-shell runtime
/// hook form. The new form is
/// `GWT_MANAGED_HOOK=runtime-state gwt hook runtime-state <event>`, so
/// we trigger migration whenever a managed runtime command contains
/// `sh -lc` (POSIX) or `ConvertTo-Json` (PowerShell), both of which
/// were exclusive to the legacy inline writer.
fn contains_inline_shell_runtime_hook(existing: Option<&Value>) -> bool {
    any_managed_command(existing, |command| {
        command.contains(GWT_MANAGED_RUNTIME_MARKER)
            && (command.contains("sh -lc") || command.contains("ConvertTo-Json"))
    })
}

/// Iterate every managed hook command under `events` and return true
/// if any of them satisfies `predicate`. Shared body for the two
/// detectors above.
fn any_managed_command(existing: Option<&Value>, predicate: impl Fn(&str) -> bool) -> bool {
    let Some(Value::Object(events)) = existing else {
        return false;
    };
    events.values().any(|events_value| {
        events_value.as_array().is_some_and(|entries| {
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
                                .is_some_and(&predicate)
                        })
                    })
            })
        })
    })
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

fn bash_blockers_hook(_target: ManagedHookTarget) -> Value {
    // SPEC #1942 (CORE-CLI): dispatch every bash blocker through the
    // `gwt hook ...` CLI surface. `target` is retained as a parameter
    // only to keep the call sites consistent across Claude and Codex;
    // the emitted commands are identical for both.
    json!({
        "matcher": "Bash",
        "hooks": [
            {
                "command": "gwt hook block-git-branch-ops",
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            },
            {
                "command": "gwt hook block-cd-command",
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            },
            {
                "command": "gwt hook block-file-ops",
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            },
            {
                "command": "gwt hook block-git-dir-override",
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
    match shell {
        HookShell::Posix => posix_runtime_hook_command(event),
        HookShell::PowerShell => powershell_runtime_hook_command(event),
    }
}

/// Emit the POSIX-shell form of the runtime-state hook. The previous
/// inline `sh -lc '...'` one-liner that wrote JSON directly is replaced
/// by a single `gwt hook runtime-state <event>` dispatch. The
/// `GWT_MANAGED_HOOK=runtime-state` env-var prefix is retained so that
/// [`is_gwt_managed_command`] continues to identify managed entries for
/// idempotent replace on regeneration.
fn posix_runtime_hook_command(event: &str) -> String {
    format!(
        "{GWT_MANAGED_RUNTIME_MARKER}={GWT_MANAGED_RUNTIME_KIND} gwt hook runtime-state {event}"
    )
}

/// Emit the PowerShell form of the runtime-state hook. Windows Claude
/// Code runs the hook through `powershell -NoProfile -Command`, so we
/// keep that wrapper to be able to set the detection env-var, then
/// invoke the same `gwt hook runtime-state` CLI.
fn powershell_runtime_hook_command(event: &str) -> String {
    format!(
        "powershell -NoProfile -Command \"& {{ $env:{GWT_MANAGED_RUNTIME_MARKER} = '{GWT_MANAGED_RUNTIME_KIND}'; gwt hook runtime-state {event} }}\""
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

    // T-080 / T-082 (SPEC #1942): the Claude settings.local.json must
    // dispatch every managed hook through the `gwt hook ...` CLI surface,
    // not through `node .../gwt-*.mjs`. The runtime hook keeps the
    // `GWT_MANAGED_HOOK=runtime-state` env-var prefix so the detection
    // logic in `is_gwt_managed_command` still recognises it as managed.
    #[test]
    fn managed_hooks_dispatch_through_gwt_hook_cli_not_node_scripts() {
        let dir = tempfile::tempdir().unwrap();

        generate_settings_local(dir.path()).unwrap();

        let path = dir.path().join(".claude/settings.local.json");
        let content = fs::read_to_string(&path).unwrap();

        // No leftover references to the old Node scripts.
        assert!(
            !content.contains("gwt-block-"),
            "settings.local.json must not reference Node block scripts, got: {content}"
        );
        assert!(
            !content.contains(".mjs"),
            "settings.local.json must not reference any .mjs file, got: {content}"
        );

        let value: Value = serde_json::from_str(&content).unwrap();

        // T-082: runtime hooks now invoke `gwt hook runtime-state <event>`
        // and still carry the GWT_MANAGED_HOOK marker for replace
        // detection. The inline POSIX shell one-liner is gone.
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
        ] {
            let cmd = value["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("runtime command missing for event {event}"));
            assert!(
                cmd.contains("gwt hook runtime-state"),
                "runtime hook for {event} must call `gwt hook runtime-state`, got: {cmd}"
            );
            assert!(
                cmd.contains(event),
                "runtime hook for {event} must pass the event name, got: {cmd}"
            );
            assert!(
                cmd.contains("GWT_MANAGED_HOOK"),
                "runtime hook must carry the GWT_MANAGED_HOOK marker, got: {cmd}"
            );
            assert!(
                !cmd.contains("mkdir"),
                "runtime hook must not shell out to mkdir anymore, got: {cmd}"
            );
            assert!(
                !cmd.contains("printf"),
                "runtime hook must not shell out to printf anymore, got: {cmd}"
            );
        }

        // T-080: bash-blocker hooks now dispatch through `gwt hook block-*`.
        let pre_tool_block_hooks = value["hooks"]["PreToolUse"][1]["hooks"]
            .as_array()
            .expect("bash blockers array");
        let expected_commands = [
            "gwt hook block-git-branch-ops",
            "gwt hook block-cd-command",
            "gwt hook block-file-ops",
            "gwt hook block-git-dir-override",
        ];
        let actual: Vec<&str> = pre_tool_block_hooks
            .iter()
            .map(|h| h["command"].as_str().unwrap_or(""))
            .collect();
        for expected in expected_commands {
            assert!(
                actual.contains(&expected),
                "bash blocker hooks must include {expected:?}, got: {actual:?}"
            );
        }
    }

    // Regression for PR #1943 review feedback ("settings.local.json
    // was not actually regenerated"). Three independent bugs were
    // shipped at once and this test locks all three:
    //
    // 1. Running the generator twice against a repo whose existing
    //    settings file already contains new-form `gwt hook block-*`
    //    commands must NOT duplicate them. `is_gwt_managed_command`
    //    has to recognise the new form as managed so the generator
    //    replaces instead of appending.
    // 2. A tracked `.codex/hooks.json` that still has the legacy
    //    `node .../gwt-block-*.mjs` entries must get migrated on the
    //    next regeneration pass — the migration gate has to include
    //    the node-bash-blocker detector.
    // 3. A tracked `.codex/hooks.json` with the legacy
    //    `GWT_MANAGED_HOOK=runtime-state sh -lc '...'` inline runtime
    //    hook must also trigger migration.
    #[test]
    fn regenerating_twice_does_not_duplicate_new_form_managed_entries() {
        let dir = tempfile::tempdir().unwrap();
        generate_settings_local(dir.path()).unwrap();
        let first = fs::read_to_string(dir.path().join(".claude/settings.local.json")).unwrap();
        generate_settings_local(dir.path()).unwrap();
        let second = fs::read_to_string(dir.path().join(".claude/settings.local.json")).unwrap();
        assert_eq!(
            first, second,
            "idempotent regeneration must produce byte-identical output"
        );

        let value: Value = serde_json::from_str(&second).unwrap();
        let pre_tool = value["hooks"]["PreToolUse"].as_array().unwrap();
        let bash_entries: Vec<_> = pre_tool
            .iter()
            .filter(|entry| entry["matcher"] == "Bash")
            .collect();
        assert_eq!(
            bash_entries.len(),
            1,
            "exactly one Bash matcher entry expected, got {}: {:?}",
            bash_entries.len(),
            bash_entries
        );
    }

    #[test]
    fn tracked_legacy_node_bash_blockers_trigger_migration() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/hooks.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "hooks": {
                    "PreToolUse": [
                        {
                            "matcher": "Bash",
                            "hooks": [
                                {
                                    "command": "node .codex/hooks/scripts/gwt-block-git-branch-ops.mjs",
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
        assert!(
            !content.contains("gwt-block-git-branch-ops.mjs"),
            "tracked legacy node bash blocker must be migrated away, got: {content}"
        );
        assert!(
            content.contains("gwt hook block-git-branch-ops"),
            "tracked file must be migrated to the new CLI form, got: {content}"
        );
    }

    #[test]
    fn tracked_legacy_inline_shell_runtime_hook_triggers_migration() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/hooks.json");
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
                                    "command": "GWT_MANAGED_HOOK=runtime-state sh -lc 'echo legacy'",
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
        assert!(
            !content.contains("sh -lc"),
            "tracked legacy inline shell runtime hook must be migrated away, got: {content}"
        );
        assert!(
            content.contains("gwt hook runtime-state"),
            "tracked file must carry the new CLI form, got: {content}"
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
            HookShell::Posix => powershell_runtime_hook_command("SessionStart"),
            HookShell::PowerShell => posix_runtime_hook_command("SessionStart"),
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

    // SPEC #1942 T-083: the inline POSIX shell JSON writer has been
    // replaced by a single `gwt hook runtime-state <event>` CLI call.
    // The sidecar-write behaviour is now covered end-to-end by
    // `crates/gwt-tui/tests/hook_runtime_state_test.rs`, which exercises
    // the Rust implementation directly without requiring `gwt` to be on
    // PATH at test time.
    #[test]
    fn posix_runtime_hook_command_dispatches_through_gwt_hook_cli() {
        let command = posix_runtime_hook_command("SessionStart");
        assert!(
            command.starts_with("GWT_MANAGED_HOOK=runtime-state"),
            "posix runtime hook must keep the managed marker prefix, got: {command}"
        );
        assert!(
            command.contains("gwt hook runtime-state SessionStart"),
            "posix runtime hook must invoke the gwt CLI with the event name, got: {command}"
        );
        assert!(
            !command.contains("sh -lc"),
            "posix runtime hook must no longer wrap the call in an inline shell, got: {command}"
        );
        assert!(
            !command.contains("printf"),
            "posix runtime hook must no longer shell out to printf, got: {command}"
        );
    }

    #[test]
    fn powershell_runtime_hook_command_dispatches_through_gwt_hook_cli() {
        let command = powershell_runtime_hook_command("Stop");
        assert!(
            command.contains("$env:GWT_MANAGED_HOOK = 'runtime-state'"),
            "powershell runtime hook must set the managed env var, got: {command}"
        );
        assert!(
            command.contains("gwt hook runtime-state Stop"),
            "powershell runtime hook must invoke the gwt CLI with the event name, got: {command}"
        );
        assert!(
            !command.contains("ConvertTo-Json"),
            "powershell runtime hook must no longer format JSON inline, got: {command}"
        );
    }
}
