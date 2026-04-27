//! Generate `.claude/settings.local.json` with gwt-managed Claude hooks.

use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};

use serde_json::{json, Map, Value};

const GWT_MANAGED_RUNTIME_MARKER: &str = "GWT_MANAGED_HOOK";
const GWT_HOOK_CLI_PREFIX: &str = "gwtd hook ";
const LEGACY_GWT_HOOK_SCRIPT_SEGMENT: &str = "hooks/scripts/gwt-";
/// SPEC #1942 amendment: distinctive subcommand suffixes that mark a
/// generated managed hook command regardless of which binary path is
/// embedded at the front. Detection by suffix avoids coupling the
/// managed-command recogniser to `current_exe()`'s filename, which
/// may be `gwt`, `gwt.exe`, or even a `cargo test` binary
/// like `gwt_skills-abc123def` during unit tests.
const MANAGED_HOOK_SUBCMD_SUFFIXES: &[&str] = &[
    " hook event ",
    " hook runtime-state ",
    " hook coordination-event ",
    " hook board-reminder ",
    " hook workflow-policy",
    " hook block-bash-policy",
    " hook block-git-branch-ops",
    " hook block-cd-command",
    " hook block-file-ops",
    " hook block-git-dir-override",
    " hook forward",
    " hook skill-discussion-stop-check",
    " hook skill-plan-spec-stop-check",
    " hook skill-build-spec-stop-check",
];
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
/// Existing hook files are merged on every refresh: gwt-managed hook entries
/// are replaced, while user hooks and unrelated top-level settings are kept.
pub fn generate_codex_hooks(worktree: &Path) -> io::Result<()> {
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
    shell: HookShell,
) -> Map<String, Value> {
    let managed_hooks = managed_hooks(shell);
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
    command.contains(LEGACY_GWT_HOOK_SCRIPT_SEGMENT)
        || command.contains(GWT_MANAGED_RUNTIME_MARKER)
        || command.contains(GWT_HOOK_CLI_PREFIX)
        || contains_gwt_hook_subcmd(command)
}

/// Match managed commands by their distinctive hook-subcommand
/// suffix (e.g. ` hook block-git-branch-ops`). This decouples the
/// managed-command recogniser from which binary path is embedded at
/// the front, so it works for:
///
/// - the legacy absolute-path form `'/Users/x/.bun/bin/gwt' hook block-*`
/// - the absolute-path form `'/Users/x/.bun/bin/gwtd' hook block-*`
/// - the PATH-dependent literal `gwtd hook block-*`
/// - even the `cargo test` binary path
///   `'/tmp/.../deps/gwt_skills-abc' hook runtime-state PreToolUse`
///   that unit tests see when they call `current_exe()` indirectly.
///
/// The suffixes in [`MANAGED_HOOK_SUBCMD_SUFFIXES`] are distinctive
/// enough that collision with user-defined commands is vanishingly
/// unlikely.
fn contains_gwt_hook_subcmd(command: &str) -> bool {
    MANAGED_HOOK_SUBCMD_SUFFIXES
        .iter()
        .any(|suffix| command.contains(suffix))
}

fn managed_hooks(shell: HookShell) -> Map<String, Value> {
    let mut hooks = Map::new();
    for event in MANAGED_EVENT_ORDER {
        hooks.insert(
            event.to_string(),
            Value::Array(vec![event_hook(event, shell)]),
        );
    }
    hooks
}

fn event_hook(event: &str, shell: HookShell) -> Value {
    json!({
        "matcher": "*",
        "hooks": [
            {
                "command": event_hook_command(event, shell),
                "type": CLAUDE_HOOK_COMMAND_TYPE,
            }
        ]
    })
}

/// Environment variable that pins the absolute path of the gwt
/// binary that generated hook commands should dispatch to. Takes
/// precedence over `current_exe()`. Used by the
/// `regenerate_hook_settings` example (which would otherwise embed
/// the example's own binary path) and any future out-of-process
/// regenerator.
const GWT_HOOK_BIN_ENV: &str = "GWT_HOOK_BIN";

/// Return the binary path that every generated hook command should
/// invoke. SPEC #1942 amendment: instead of relying on a literal `gwt`
/// resolved via `$PATH`, the generator embeds an absolute path so
/// that hooks work even when the user has not installed gwt globally
/// (dev worktrees, CI sandboxes, fresh clones).
///
/// Resolution order:
///
/// 1. `$GWT_HOOK_BIN` environment variable (explicit override, used
///    by the regenerate-settings example when it knows the gwt
///    binary path but its own `current_exe()` points elsewhere).
/// 2. The sibling `gwtd` binary when the GUI `gwt` binary calls
///    `generate_settings_local` at startup.
/// 3. A PATH-resolved `gwtd` when available.
/// 4. Literal `"gwtd"` fallback (PATH-dependent behaviour).
///
/// Platform notes:
///
/// - macOS: `current_exe()` preserves the invocation path, so a
///   `~/.bun/bin/gwt` GUI launch resolves to the sibling
///   `~/.bun/bin/gwtd` path and survives bun upgrades.
/// - Linux: `/proc/self/exe` resolves to the real binary, which may
///   land inside bun's per-version cache. The generator is re-run on
///   every gwt startup so staleness self-heals on the next launch.
fn gwt_hook_bin_path() -> String {
    if let Ok(v) = std::env::var(GWT_HOOK_BIN_ENV) {
        if !v.is_empty() {
            return v;
        }
    }
    let current_exe = std::env::current_exe().ok();
    gwt_hook_bin_path_with(current_exe.as_deref(), path_lookup)
}

fn gwt_hook_bin_path_with(
    current_exe: Option<&Path>,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> String {
    if let Some(path) = current_exe {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return lookup("gwtd")
                .and_then(path_to_string)
                .unwrap_or_else(|| "gwtd".to_string());
        };
        if is_gwtd_exe_name(file_name) {
            return path_to_string(path.to_path_buf()).unwrap_or_else(|| "gwtd".to_string());
        }
        if is_gwt_gui_exe_name(file_name) {
            let gwtd_path = path.with_file_name(gwtd_exe_name_for(file_name));
            return path_to_string(gwtd_path).unwrap_or_else(|| "gwtd".to_string());
        }
    }
    lookup("gwtd")
        .and_then(path_to_string)
        .unwrap_or_else(|| "gwtd".to_string())
}

fn is_gwt_gui_exe_name(file_name: &str) -> bool {
    matches!(file_name, "gwt" | "gwt.exe")
}

fn is_gwtd_exe_name(file_name: &str) -> bool {
    matches!(file_name, "gwtd" | "gwtd.exe")
}

fn gwtd_exe_name_for(gwt_exe_name: &str) -> &'static str {
    if gwt_exe_name.ends_with(".exe") {
        "gwtd.exe"
    } else {
        "gwtd"
    }
}

fn path_to_string(path: PathBuf) -> Option<String> {
    path.into_os_string().into_string().ok()
}

fn path_lookup(command: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(command))
        .find(|candidate| candidate.is_file())
}

/// POSIX shell single-quote quoting. An embedded single quote becomes
/// `'\''` (close, literal, reopen).
fn posix_shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// PowerShell single-quote quoting. An embedded single quote is
/// escaped by doubling it.
fn powershell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn managed_hook_shell() -> HookShell {
    if cfg!(windows) {
        HookShell::PowerShell
    } else {
        HookShell::Posix
    }
}

fn event_hook_command(event: &str, shell: HookShell) -> String {
    event_hook_command_with_bin(&gwt_hook_bin_path(), event, shell)
}

fn event_hook_command_with_bin(bin: &str, event: &str, shell: HookShell) -> String {
    match shell {
        HookShell::Posix => posix_event_hook_command_with_bin(bin, event),
        HookShell::PowerShell => powershell_event_hook_command_with_bin(bin, event),
    }
}

#[cfg(test)]
fn runtime_hook_command(event: &str, shell: HookShell) -> String {
    match shell {
        HookShell::Posix => posix_runtime_hook_command(event),
        HookShell::PowerShell => powershell_runtime_hook_command(event),
    }
}

#[cfg(test)]
fn coordination_hook_command(event: &str, shell: HookShell) -> String {
    match shell {
        HookShell::Posix => posix_coordination_hook_command(event),
        HookShell::PowerShell => powershell_coordination_hook_command(event),
    }
}

#[cfg(test)]
fn forward_hook_command_with_bin(bin: &str, shell: HookShell) -> String {
    match shell {
        HookShell::Posix => posix_forward_hook_command_with_bin(bin),
        HookShell::PowerShell => powershell_forward_hook_command_with_bin(bin),
    }
}

#[cfg(test)]
fn workflow_policy_hook_command(shell: HookShell) -> String {
    workflow_policy_hook_command_with_bin(&gwt_hook_bin_path(), shell)
}

#[cfg(test)]
fn workflow_policy_hook_command_with_bin(bin: &str, shell: HookShell) -> String {
    match shell {
        HookShell::Posix => posix_workflow_policy_hook_command_with_bin(bin),
        HookShell::PowerShell => powershell_workflow_policy_hook_command_with_bin(bin),
    }
}

/// Emit the POSIX-shell form of the runtime-state hook. The previous
/// inline `sh -lc '...'` one-liner that wrote JSON directly is replaced
/// by a single `gwtd hook runtime-state <event>` dispatch.
fn posix_event_hook_command_with_bin(bin: &str, event: &str) -> String {
    let bin = posix_shell_quote(bin);
    format!("{bin} hook event {event}")
}

#[cfg(test)]
fn posix_runtime_hook_command(event: &str) -> String {
    let bin = posix_shell_quote(&gwt_hook_bin_path());
    format!("{bin} hook runtime-state {event}")
}

#[cfg(test)]
fn posix_workflow_policy_hook_command_with_bin(bin: &str) -> String {
    let bin = posix_shell_quote(bin);
    format!("{bin} hook workflow-policy")
}

#[cfg(test)]
fn posix_forward_hook_command_with_bin(bin: &str) -> String {
    let bin = posix_shell_quote(bin);
    format!("{bin} hook forward")
}

#[cfg(test)]
fn posix_coordination_hook_command(event: &str) -> String {
    let bin = posix_shell_quote(&gwt_hook_bin_path());
    format!("{bin} hook coordination-event {event}")
}

/// Emit the PowerShell form of the runtime-state hook. Windows Claude
/// Code runs the hook through `powershell -NoProfile -Command`, so we
/// keep that wrapper, then invoke the gwtd binary via `& '...'` call
/// operator.
fn powershell_event_hook_command_with_bin(bin: &str, event: &str) -> String {
    let bin = powershell_quote(bin);
    format!("powershell -NoProfile -Command \"& {{ & {bin} hook event {event} }}\"")
}

#[cfg(test)]
fn powershell_runtime_hook_command(event: &str) -> String {
    let bin = powershell_quote(&gwt_hook_bin_path());
    format!("powershell -NoProfile -Command \"& {{ & {bin} hook runtime-state {event} }}\"")
}

#[cfg(test)]
fn powershell_workflow_policy_hook_command_with_bin(bin: &str) -> String {
    let bin = powershell_quote(bin);
    format!("powershell -NoProfile -Command \"& {{ & {bin} hook workflow-policy }}\"")
}

#[cfg(test)]
fn powershell_forward_hook_command_with_bin(bin: &str) -> String {
    let bin = powershell_quote(bin);
    format!("powershell -NoProfile -Command \"& {{ & {bin} hook forward }}\"")
}

#[cfg(test)]
fn powershell_coordination_hook_command(event: &str) -> String {
    let bin = powershell_quote(&gwt_hook_bin_path());
    format!("powershell -NoProfile -Command \"& {{ & {bin} hook coordination-event {event} }}\"")
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn commands_for_event<'a>(value: &'a Value, event: &str) -> Vec<&'a str> {
        value["hooks"][event]
            .as_array()
            .unwrap_or_else(|| panic!("hooks missing for event {event}"))
            .iter()
            .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
            .filter_map(|hook| hook["command"].as_str())
            .collect()
    }

    #[test]
    fn managed_hooks_use_one_event_dispatcher_command_per_event() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".codex/hooks.json")).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();

        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
        ] {
            let commands = commands_for_event(&value, event);
            assert_eq!(
                commands.len(),
                1,
                "managed {event} hook must be a single dispatcher command, got: {commands:?}"
            );
            assert!(
                commands[0].contains(&format!(" hook event {event}")),
                "managed {event} command must dispatch through `hook event {event}`, got: {}",
                commands[0]
            );
        }
    }

    #[test]
    fn board_reminder_registered_only_on_intent_boundary_events() {
        let dir = tempfile::tempdir().unwrap();
        generate_settings_local(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".claude/settings.local.json")).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();

        for event in MANAGED_EVENT_ORDER {
            let commands = commands_for_event(&value, event);
            assert_eq!(commands.len(), 1, "{event} must use one dispatcher");
            assert!(
                commands[0].contains(&format!(" hook event {event}")),
                "{event} must route through the event dispatcher; commands: {commands:?}"
            );
        }
    }

    #[test]
    fn managed_stop_chain_includes_three_skill_check_handlers_after_board_reminder() {
        let dir = tempfile::tempdir().unwrap();
        generate_settings_local(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".claude/settings.local.json")).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();
        let stop_commands = commands_for_event(&value, "Stop");
        assert_eq!(
            stop_commands,
            vec!["'gwtd' hook event Stop"],
            "Stop chain must collapse to the event dispatcher; got: {stop_commands:?}"
        );
    }

    #[test]
    fn managed_stop_chain_does_not_register_skill_checks_on_non_stop_events() {
        let dir = tempfile::tempdir().unwrap();
        generate_settings_local(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".claude/settings.local.json")).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
        ] {
            let commands = commands_for_event(&value, event);
            for suffix in [
                " hook skill-discussion-stop-check",
                " hook skill-plan-spec-stop-check",
                " hook skill-build-spec-stop-check",
            ] {
                assert!(
                    commands.iter().all(|c| !c.contains(suffix)),
                    "{suffix} must NOT be registered on {event}; commands: {commands:?}"
                );
            }
        }
    }

    #[test]
    fn managed_stop_chain_does_not_register_short_lived_skill_handlers() {
        // Regression guard for FR-014s: gwt-register-issue / gwt-fix-issue /
        // gwt-issue-search / gwt-search must never have a Stop-check handler
        // registered. We assert those hook names are not emitted anywhere.
        let dir = tempfile::tempdir().unwrap();
        generate_settings_local(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".claude/settings.local.json")).unwrap();
        for forbidden in [
            " hook skill-register-issue-stop-check",
            " hook skill-fix-issue-stop-check",
            " hook skill-issue-search-stop-check",
            " hook skill-search-stop-check",
        ] {
            assert!(
                !content.contains(forbidden),
                "short-lived skill {forbidden} must not appear in managed hooks"
            );
        }
    }

    #[test]
    fn board_reminder_regeneration_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        generate_settings_local(dir.path()).unwrap();
        let first = fs::read_to_string(dir.path().join(".claude/settings.local.json")).unwrap();
        generate_settings_local(dir.path()).unwrap();
        let second = fs::read_to_string(dir.path().join(".claude/settings.local.json")).unwrap();
        assert_eq!(
            first, second,
            "settings.local.json must be byte-identical after regeneration"
        );
        assert!(first.contains(" hook event SessionStart"));
        assert!(first.contains(" hook event UserPromptSubmit"));
        assert!(first.contains(" hook event Stop"));
    }

    #[test]
    fn board_reminder_migration_dedupes_legacy_settings_without_board_reminder() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join(".claude/settings.local.json");
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        // Legacy settings carry every older gwt-managed hook but none of
        // them know about board-reminder yet. The regeneration must add
        // board-reminder on the intent-boundary events without duplicating
        // any pre-existing managed entries.
        let legacy = serde_json::json!({
            "hooks": {
                "SessionStart": [
                    {
                        "matcher": "*",
                        "hooks": [
                            {
                                "command": "'/old/gwt' hook runtime-state SessionStart",
                                "type": "command"
                            }
                        ]
                    }
                ],
                "UserPromptSubmit": [],
                "PreToolUse": [],
                "PostToolUse": [],
                "Stop": []
            }
        });
        fs::write(
            &settings_path,
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();

        generate_settings_local(dir.path()).unwrap();

        let content = fs::read_to_string(&settings_path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();

        // Every event now has exactly one dispatcher entry, and the legacy
        // runtime-state entry has been replaced rather than duplicated.
        for event in MANAGED_EVENT_ORDER {
            let commands = commands_for_event(&value, event);
            assert_eq!(
                commands.len(),
                1,
                "expected exactly one dispatcher entry on {event}, got: {commands:?}"
            );
            assert!(
                commands[0].contains(&format!(" hook event {event}")),
                "event must dispatch through hook event, got: {commands:?}"
            );
        }

        let session_start = commands_for_event(&value, "SessionStart");
        assert!(
            session_start
                .iter()
                .all(|c| !c.contains(" hook runtime-state ")),
            "legacy runtime-state entry must be replaced, not duplicated; got: {session_start:?}"
        );
    }

    #[test]
    fn creates_settings_local_with_managed_hooks() {
        let dir = tempfile::tempdir().unwrap();

        generate_settings_local(dir.path()).unwrap();

        let path = dir.path().join(".claude/settings.local.json");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&content).unwrap();

        let user_prompt_commands = commands_for_event(&value, "UserPromptSubmit");
        assert_eq!(user_prompt_commands.len(), 1);
        assert!(user_prompt_commands[0].contains(" hook event UserPromptSubmit"));
        assert!(user_prompt_commands[0].contains("gwtd"));
        assert!(!user_prompt_commands[0].contains("node"));
        assert!(value["hooks"]["SessionStart"].is_array());
        assert!(value["hooks"].get("Notification").is_none());
        let pre_tool_commands = commands_for_event(&value, "PreToolUse");
        assert_eq!(pre_tool_commands.len(), 1);
        assert!(pre_tool_commands[0].contains(" hook event PreToolUse"));
    }

    // T-080 / T-082 (SPEC #1942): the Claude settings.local.json must
    // dispatch every managed hook through the `gwtd hook ...` CLI surface,
    // not through retired Node hook scripts under `hooks/scripts/gwt-*`.
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
            !content.contains("hooks/scripts/gwt-"),
            "settings.local.json must not reference retired hook scripts, got: {content}"
        );

        let value: Value = serde_json::from_str(&content).unwrap();

        // T-082 / event dispatcher update: managed hooks invoke
        // `gwtd hook event <event>` exactly once per hook event.
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
        ] {
            let commands = commands_for_event(&value, event);
            assert_eq!(
                commands.len(),
                1,
                "managed event {event} must collapse to one command, got: {commands:?}"
            );
            let cmd = commands[0];
            assert!(
                cmd.contains(&format!(" hook event {event}")),
                "managed hook for {event} must dispatch to `hook event {event}`, got: {cmd}"
            );
            assert!(
                !cmd.contains("GWT_MANAGED_HOOK"),
                "event hook must not carry the managed marker anymore, got: {cmd}"
            );
            assert!(
                !cmd.contains("mkdir"),
                "event hook must not shell out to mkdir anymore, got: {cmd}"
            );
            assert!(
                !cmd.contains("printf"),
                "event hook must not shell out to printf anymore, got: {cmd}"
            );
            assert!(
                !cmd.starts_with("gwtd hook "),
                "event hook must not use the PATH-dependent literal `gwtd`, got: {cmd}"
            );
            assert!(
                !cmd.contains("/internal/hook-live"),
                "event hook must not expose daemon endpoints, got: {cmd}"
            );
            assert!(
                !cmd.contains("GWT_HOOK_FORWARD_URL"),
                "event hook must not expose transport env names, got: {cmd}"
            );
            assert!(
                !cmd.contains("GWT_HOOK_FORWARD_TOKEN"),
                "event hook must not expose transport env names, got: {cmd}"
            );
        }
    }

    // Regression for PR #1943 review feedback ("settings.local.json
    // was not actually regenerated"). Three independent bugs were
    // shipped at once and this test locks all three:
    //
    // 1. Running the generator twice against a repo whose existing
    //    settings file already contains new-form `gwtd hook block-*`
    //    commands must NOT duplicate them. `is_gwt_managed_command`
    //    has to recognise the new form as managed so the generator
    //    replaces instead of appending.
    // 2. An existing `.codex/hooks.json` that still has legacy
    //    `node .../hooks/scripts/gwt-*` entries must be merged into the
    //    new hook CLI form on the next regeneration pass.
    // 3. An existing `.codex/hooks.json` with the legacy
    //    `GWT_MANAGED_HOOK=runtime-state sh -lc '...'` inline runtime
    //    hook must also be rewritten into the current managed form.
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
        let event_entries: Vec<_> = pre_tool
            .iter()
            .filter(|entry| {
                entry["hooks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|hook| hook["command"].as_str())
                    .any(|command| command.contains(" hook event PreToolUse"))
            })
            .collect();
        assert_eq!(
            event_entries.len(),
            1,
            "exactly one event dispatcher entry expected, got {}: {:?}",
            event_entries.len(),
            event_entries
        );
        assert_eq!(commands_for_event(&value, "PreToolUse").len(), 1);
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
                                    "command": "node .codex/hooks/scripts/gwt-block-git-branch-ops.js",
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
            !content.contains("hooks/scripts/gwt-"),
            "tracked legacy node bash blocker must be migrated away, got: {content}"
        );
        assert!(
            content.contains("hook event PreToolUse"),
            "tracked file must be migrated to the consolidated event dispatcher form, got: {content}"
        );
    }

    #[test]
    fn tracked_block_bash_policy_hook_triggers_migration() {
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
                                    "command": "'/tmp/gwt' hook block-bash-policy",
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
            !content.contains("hook block-bash-policy"),
            "tracked block-bash-policy hook must be migrated away, got: {content}"
        );
        assert!(
            content.contains("hook event PreToolUse"),
            "tracked file must dispatch to the event dispatcher after migration, got: {content}"
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
            content.contains("hook event PreToolUse"),
            "tracked file must carry the event dispatcher CLI form, got: {content}"
        );
        assert!(
            !content.contains("GWT_MANAGED_HOOK"),
            "tracked file must drop the managed marker, got: {content}"
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

        assert!(commands
            .iter()
            .any(|command| command.contains(" hook event PreToolUse")));
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
        let session_start_commands = commands_for_event(&value, "SessionStart");
        let pre_tool_commands = commands_for_event(&value, "PreToolUse");

        assert_eq!(session_start_commands.len(), 1);
        assert!(session_start_commands
            .iter()
            .any(|command| command.contains(" hook event SessionStart")));
        assert!(session_start_commands
            .iter()
            .all(|command| !command.contains("GWT_MANAGED_HOOK")));
        assert!(session_start_commands
            .iter()
            .all(|command| !command.contains("node")));
        assert_eq!(pre_tool_commands.len(), 1);
        assert!(pre_tool_commands
            .iter()
            .any(|command| command.contains(" hook event PreToolUse")));
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

        assert!(commands
            .iter()
            .any(|command| command.contains(" hook event SessionStart")));
        assert!(commands.contains(&"my-custom-hook"));
    }

    #[test]
    fn generate_codex_hooks_merges_existing_tracked_hooks_json_without_legacy_runtime_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/hooks.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "custom_setting": true,
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
        let value: Value = serde_json::from_str(&content).unwrap();
        let session_start_commands: Vec<&str> = value["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|entry| entry["hooks"].as_array().unwrap().iter())
            .filter_map(|hook| hook["command"].as_str())
            .collect();
        let pre_tool_commands: Vec<&str> = value["hooks"]["PreToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|entry| entry["hooks"].as_array().unwrap().iter())
            .filter_map(|hook| hook["command"].as_str())
            .collect();

        assert!(session_start_commands.contains(&"tracked-command"));
        assert!(session_start_commands
            .iter()
            .any(|command| command.contains(" hook event SessionStart")));
        assert!(pre_tool_commands
            .iter()
            .any(|command| command.contains(" hook event PreToolUse")));
        assert_eq!(value["custom_setting"], Value::Bool(true));
    }

    fn legacy_node_forward_hook_command(event: &str) -> String {
        format!(
            "node \"$(git rev-parse --show-toplevel)/.codex/hooks/scripts/gwt-legacy-forward.js\" {event}"
        )
    }

    #[test]
    fn generate_codex_hooks_migrates_tracked_legacy_forward_hooks_without_node() {
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
                                    "command": legacy_node_forward_hook_command("SessionStart"),
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
                                    "command": legacy_node_forward_hook_command("PreToolUse"),
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
        let session_start_commands = commands_for_event(&value, "SessionStart");
        let pre_tool_commands = commands_for_event(&value, "PreToolUse");

        assert!(session_start_commands
            .iter()
            .any(|command| command.contains(" hook event SessionStart")));
        assert!(session_start_commands
            .iter()
            .all(|command| !command.contains("GWT_MANAGED_HOOK")));
        assert!(!content.contains("hooks/scripts/gwt-"));
        assert!(session_start_commands
            .iter()
            .all(|command| !command.contains("node")));
        assert!(pre_tool_commands.contains(&"my-custom-hook"));
        assert!(pre_tool_commands
            .iter()
            .any(|command| command.contains(" hook event PreToolUse")));
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
        let expected = event_hook_command("SessionStart", managed_hook_shell());
        assert_eq!(session_start_command, expected);
    }

    #[test]
    fn generate_codex_hooks_migrates_tracked_hooks_missing_coordination_entries() {
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
                                    "command": runtime_hook_command("SessionStart", managed_hook_shell()),
                                    "type": "command"
                                }
                            ]
                        }
                    ],
                    "UserPromptSubmit": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": runtime_hook_command("UserPromptSubmit", managed_hook_shell()),
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
                                    "command": runtime_hook_command("PreToolUse", managed_hook_shell()),
                                    "type": "command"
                                }
                            ]
                        },
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": workflow_policy_hook_command(managed_hook_shell()),
                                    "type": "command"
                                }
                            ]
                        }
                    ],
                    "PostToolUse": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": runtime_hook_command("PostToolUse", managed_hook_shell()),
                                    "type": "command"
                                }
                            ]
                        }
                    ],
                    "Stop": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": runtime_hook_command("Stop", managed_hook_shell()),
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
        let session_start_commands: Vec<&str> = value["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|entry| entry["hooks"].as_array().unwrap().iter())
            .filter_map(|hook| hook["command"].as_str())
            .collect();
        let stop_commands: Vec<&str> = value["hooks"]["Stop"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|entry| entry["hooks"].as_array().unwrap().iter())
            .filter_map(|hook| hook["command"].as_str())
            .collect();

        assert!(session_start_commands
            .iter()
            .any(|command| command.contains(" hook event SessionStart")));
        assert!(stop_commands
            .iter()
            .any(|command| command.contains(" hook event Stop")));
        assert!(stop_commands.contains(&"my-custom-hook"));
    }

    #[test]
    fn generate_codex_hooks_migrates_tracked_hooks_missing_forward_entries() {
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
                                    "command": runtime_hook_command("SessionStart", managed_hook_shell()),
                                    "type": "command"
                                },
                                {
                                    "command": coordination_hook_command("SessionStart", managed_hook_shell()),
                                    "type": "command"
                                }
                            ]
                        }
                    ],
                    "UserPromptSubmit": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": runtime_hook_command("UserPromptSubmit", managed_hook_shell()),
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
                                    "command": runtime_hook_command("PreToolUse", managed_hook_shell()),
                                    "type": "command"
                                }
                            ]
                        },
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": workflow_policy_hook_command(managed_hook_shell()),
                                    "type": "command"
                                },
                                {
                                    "command": "my-custom-hook",
                                    "type": "command"
                                }
                            ]
                        }
                    ],
                    "PostToolUse": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": runtime_hook_command("PostToolUse", managed_hook_shell()),
                                    "type": "command"
                                }
                            ]
                        }
                    ],
                    "Stop": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": runtime_hook_command("Stop", managed_hook_shell()),
                                    "type": "command"
                                },
                                {
                                    "command": coordination_hook_command("Stop", managed_hook_shell()),
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
        for event in MANAGED_EVENT_ORDER {
            let commands = commands_for_event(&value, event);
            assert_eq!(
                commands
                    .iter()
                    .filter(|command| command.contains(&format!(" hook event {event}")))
                    .count(),
                1,
                "migration must restore exactly one event dispatcher for {event}, got: {commands:?}"
            );
        }
        assert!(commands_for_event(&value, "PreToolUse").contains(&"my-custom-hook"));
        assert!(content.contains("gwtd"));
        assert!(!content.contains("\"gwt hook"));
    }

    // SPEC #1942 T-083: the inline POSIX shell JSON writer has been
    // replaced by a single `gwtd hook runtime-state <event>` CLI call.
    // The sidecar-write behaviour is now covered end-to-end by
    // `crates/gwt/tests/hook_runtime_state_test.rs`, which exercises
    // the Rust implementation directly without requiring `gwtd` to be on
    // PATH at test time.
    #[test]
    fn posix_runtime_hook_command_dispatches_through_gwtd_hook_cli() {
        let command = posix_runtime_hook_command("SessionStart");
        assert!(
            !command.contains("GWT_MANAGED_HOOK"),
            "posix runtime hook must not keep the managed marker, got: {command}"
        );
        // SPEC #1942 amendment: the gwtd binary is dispatched via a
        // quoted command. In unit tests this may be the literal fallback
        // because the test process is not a release binary.
        assert!(
            command.contains(" hook runtime-state SessionStart"),
            "posix runtime hook must invoke `hook runtime-state <event>`, got: {command}"
        );
        assert!(
            command.contains("'gwtd'") || command.contains("/gwtd") || command.contains("\\gwtd"),
            "posix runtime hook must dispatch through gwtd, got: {command}"
        );
        assert!(
            !command.contains("sh -lc"),
            "posix runtime hook must no longer wrap the call in an inline shell, got: {command}"
        );
        assert!(
            !command.contains("printf"),
            "posix runtime hook must no longer shell out to printf, got: {command}"
        );
        // Regression: the PATH-less literal form is forbidden.
        assert!(
            !command.contains(" gwtd hook "),
            "posix runtime hook must not use the PATH-dependent literal `gwt`, got: {command}"
        );
    }

    #[test]
    fn powershell_runtime_hook_command_dispatches_through_gwt_hook_cli() {
        let command = powershell_runtime_hook_command("Stop");
        assert!(
            !command.contains("GWT_MANAGED_HOOK"),
            "powershell runtime hook must not set the managed env var, got: {command}"
        );
        assert!(
            command.contains(" hook runtime-state Stop"),
            "powershell runtime hook must invoke `hook runtime-state <event>`, got: {command}"
        );
        // SPEC #1942 amendment: absolute path embedded via PowerShell's
        // `& 'path' arg` call operator.
        assert!(
            command.contains("& '"),
            "powershell runtime hook must dispatch via the & call operator with a quoted path, got: {command}"
        );
        assert!(
            !command.contains("ConvertTo-Json"),
            "powershell runtime hook must no longer format JSON inline, got: {command}"
        );
    }

    // SPEC #1942 amendment regression tests
    #[test]
    fn gwt_hook_bin_path_returns_absolute_or_gwtd_fallback() {
        let path = gwt_hook_bin_path();
        // Either an absolute path from current_exe/PATH or the literal
        // fallback "gwtd". Must never be empty.
        assert!(!path.is_empty());
        assert_ne!(path, "gwt");
        if path != "gwtd" {
            assert!(
                std::path::Path::new(&path).is_absolute(),
                "gwt_hook_bin_path must return an absolute path or the literal gwtd fallback, got: {path}"
            );
        }
    }

    #[test]
    fn gwt_hook_bin_path_resolves_gui_binary_to_gwtd_sibling() {
        assert_eq!(
            gwt_hook_bin_path_with(Some(Path::new("/opt/gwt/bin/gwt")), |_| None),
            "/opt/gwt/bin/gwtd"
        );
        assert_eq!(
            gwt_hook_bin_path_with(Some(Path::new("C:/tools/gwt.exe")), |_| None),
            "C:/tools/gwtd.exe"
        );
    }

    #[test]
    fn gwt_hook_bin_path_uses_gwtd_fallback_for_non_release_binaries() {
        assert_eq!(
            gwt_hook_bin_path_with(
                Some(Path::new("/tmp/target/debug/deps/gwt_skills-abc123")),
                |_| None
            ),
            "gwtd"
        );
        assert_eq!(
            gwt_hook_bin_path_with(
                Some(Path::new("/tmp/target/debug/deps/gwt_skills-abc123")),
                |command| {
                    assert_eq!(command, "gwtd");
                    Some(PathBuf::from("/usr/local/bin/gwtd"))
                }
            ),
            "/usr/local/bin/gwtd"
        );
    }

    #[test]
    fn posix_shell_quote_escapes_single_quotes() {
        assert_eq!(posix_shell_quote("simple"), "'simple'");
        assert_eq!(posix_shell_quote("with space"), "'with space'");
        assert_eq!(posix_shell_quote("a'b"), r"'a'\''b'");
        assert_eq!(posix_shell_quote(""), "''");
    }

    #[test]
    fn powershell_quote_escapes_single_quotes() {
        assert_eq!(powershell_quote("simple"), "'simple'");
        assert_eq!(powershell_quote("a'b"), "'a''b'");
    }

    #[test]
    fn workflow_policy_hook_command_matches_shell_shape() {
        assert_eq!(
            workflow_policy_hook_command_with_bin("gwtd", HookShell::Posix),
            "'gwtd' hook workflow-policy"
        );
        assert_eq!(
            workflow_policy_hook_command_with_bin("gwtd", HookShell::PowerShell),
            "powershell -NoProfile -Command \"& { & 'gwtd' hook workflow-policy }\""
        );
    }

    #[test]
    fn forward_hook_command_matches_shell_shape() {
        assert_eq!(
            forward_hook_command_with_bin("gwtd", HookShell::Posix),
            "'gwtd' hook forward"
        );
        assert_eq!(
            forward_hook_command_with_bin("gwtd", HookShell::PowerShell),
            "powershell -NoProfile -Command \"& { & 'gwtd' hook forward }\""
        );
    }

    #[test]
    fn is_gwt_managed_command_recognizes_absolute_path_form() {
        assert!(is_gwt_managed_command(
            "'/Users/x/.bun/bin/gwtd' hook workflow-policy"
        ));
        assert!(is_gwt_managed_command(
            "'/Users/x/.bun/bin/gwtd' hook event PreToolUse"
        ));
        assert!(is_gwt_managed_command(
            "'/Users/x/.bun/bin/gwt' hook workflow-policy"
        ));
        assert!(is_gwt_managed_command(
            "'/Users/x/.bun/bin/gwt' hook runtime-state PreToolUse"
        ));
        assert!(is_gwt_managed_command(
            "'/Users/x/.bun/bin/gwt' hook forward"
        ));
        // Negative: unrelated `hook` substring must not match.
        assert!(!is_gwt_managed_command("echo 'fish hook ornament'"));
        assert!(!is_gwt_managed_command("grep hook foo.txt"));
    }

    #[test]
    fn tracked_pathless_gwt_hook_literal_triggers_migration() {
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
                                    "command": "gwtd hook block-git-branch-ops",
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
            !content.contains("\"gwtd hook block-git-branch-ops\""),
            "tracked PATH-less literal must be migrated away, got: {content}"
        );
        assert!(
            content.contains("hook event PreToolUse"),
            "migrated file must dispatch through the event dispatcher, got: {content}"
        );
    }
}
