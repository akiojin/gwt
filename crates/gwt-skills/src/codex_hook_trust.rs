//! Codex hook trust-state registration for gwt-managed project hooks.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::settings_local::{
    codex_event_hook_commands, codex_event_hook_commands_with_bin,
    codex_hooks_paths_for_codex_discovery, write_text_atomically, CodexHookDiscoveryMode,
};

const CODEX_DEFAULT_COMMAND_TIMEOUT_SECONDS: u64 = 600;
const MANAGED_EVENTS: &[(&str, &str)] = &[
    ("SessionStart", "session_start"),
    ("UserPromptSubmit", "user_prompt_submit"),
    ("PreToolUse", "pre_tool_use"),
    ("PostToolUse", "post_tool_use"),
    ("Stop", "stop"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexHookTrustEntry {
    pub key: String,
    pub trusted_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexHookTrustReport {
    pub config_path: PathBuf,
    pub trusted_entries: Vec<CodexHookTrustEntry>,
}

pub fn collect_codex_managed_hook_trust_entries(
    worktree: &Path,
) -> io::Result<Vec<CodexHookTrustEntry>> {
    collect_codex_managed_hook_trust_entries_for_mode(
        worktree,
        CodexHookDiscoveryMode::WorkspaceHome,
    )
}

pub fn collect_codex_managed_hook_trust_entries_for_mode(
    worktree: &Path,
    mode: CodexHookDiscoveryMode,
) -> io::Result<Vec<CodexHookTrustEntry>> {
    collect_codex_managed_hook_trust_entries_for_mode_with_expected_bin(worktree, mode, None)
}

#[cfg(test)]
fn collect_codex_managed_hook_trust_entries_with_expected_bin(
    worktree: &Path,
    expected_gwt_bin: Option<&str>,
) -> io::Result<Vec<CodexHookTrustEntry>> {
    collect_codex_managed_hook_trust_entries_for_mode_with_expected_bin(
        worktree,
        CodexHookDiscoveryMode::WorkspaceHome,
        expected_gwt_bin,
    )
}

fn collect_codex_managed_hook_trust_entries_for_mode_with_expected_bin(
    worktree: &Path,
    mode: CodexHookDiscoveryMode,
    expected_gwt_bin: Option<&str>,
) -> io::Result<Vec<CodexHookTrustEntry>> {
    let mut entries = Vec::new();
    for hooks_path in codex_hooks_paths_for_codex_discovery(worktree, mode) {
        entries.extend(collect_codex_managed_hook_trust_entries_from_path(
            &hooks_path,
            expected_gwt_bin,
        )?);
    }
    Ok(entries)
}

fn collect_codex_managed_hook_trust_entries_from_path(
    hooks_path: &Path,
    expected_gwt_bin: Option<&str>,
) -> io::Result<Vec<CodexHookTrustEntry>> {
    if !hooks_path.exists() {
        return Ok(Vec::new());
    }

    let key_source = fs::canonicalize(hooks_path)?;
    let content = fs::read_to_string(hooks_path)?;
    let root: Value = serde_json::from_str(&content).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Codex hooks JSON parse failed: {err}"),
        )
    })?;

    let Some(hooks_by_event) = root.get("hooks").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };

    let mut entries = Vec::new();
    for (event_json_name, event_snake_name) in MANAGED_EVENTS {
        let Some(groups) = hooks_by_event
            .get(*event_json_name)
            .and_then(Value::as_array)
        else {
            continue;
        };
        let Some(group) = groups.first().and_then(Value::as_object) else {
            continue;
        };
        let matcher = group
            .get("matcher")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if matcher != "*" {
            continue;
        }
        let Some(hook) = group
            .get("hooks")
            .and_then(Value::as_array)
            .and_then(|hooks| hooks.first())
            .and_then(Value::as_object)
        else {
            continue;
        };
        let Some(command) = hook.get("command").and_then(Value::as_str) else {
            continue;
        };
        if hook.get("type").and_then(Value::as_str) != Some("command")
            || !is_generated_gwt_event_command(command, event_json_name, expected_gwt_bin)
        {
            continue;
        }

        entries.push(CodexHookTrustEntry {
            key: hook_key(&key_source, event_snake_name, 0, 0),
            trusted_hash: command_hook_trusted_hash(event_snake_name, matcher, command),
        });
    }

    Ok(entries)
}

pub fn register_codex_managed_hook_trust(
    worktree: &Path,
    config_path: &Path,
) -> io::Result<CodexHookTrustReport> {
    register_codex_managed_hook_trust_for_mode(
        worktree,
        config_path,
        CodexHookDiscoveryMode::WorkspaceHome,
    )
}

pub fn register_codex_managed_hook_trust_for_mode(
    worktree: &Path,
    config_path: &Path,
    mode: CodexHookDiscoveryMode,
) -> io::Result<CodexHookTrustReport> {
    let trusted_entries = collect_codex_managed_hook_trust_entries_for_mode(worktree, mode)?;
    if trusted_entries.is_empty() {
        return Ok(CodexHookTrustReport {
            config_path: config_path.to_path_buf(),
            trusted_entries,
        });
    }

    let mut root = read_codex_config(config_path)?;
    let root_table = root.as_table_mut().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Codex config root must be a TOML table",
        )
    })?;
    let hooks_table = ensure_child_table(root_table, "hooks")?;
    let state_table = ensure_child_table(hooks_table, "state")?;

    for entry in &trusted_entries {
        let hook_state = ensure_child_table(state_table, &entry.key)?;
        enable_hook_unless_explicitly_disabled(hook_state);
        hook_state.insert(
            "trusted_hash".to_string(),
            toml::Value::String(entry.trusted_hash.clone()),
        );
    }

    let rendered = toml::to_string_pretty(&root)
        .map_err(|err| io::Error::other(format!("Codex config TOML serialize failed: {err}")))?;
    write_text_atomically(config_path, &rendered)?;

    Ok(CodexHookTrustReport {
        config_path: config_path.to_path_buf(),
        trusted_entries,
    })
}

#[cfg(test)]
fn command_hook_trusted_hash_for_test(
    event_name_snake: &str,
    matcher: &str,
    command: &str,
) -> String {
    command_hook_trusted_hash(event_name_snake, matcher, command)
}

fn read_codex_config(path: &Path) -> io::Result<toml::Value> {
    if !path.exists() {
        return Ok(toml::Value::Table(toml::Table::new()));
    }

    let content = fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(toml::Value::Table(toml::Table::new()));
    }

    toml::from_str::<toml::Value>(&content).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Codex config TOML parse failed: {err}"),
        )
    })
}

fn ensure_child_table<'a>(
    table: &'a mut toml::Table,
    key: &str,
) -> io::Result<&'a mut toml::Table> {
    if !table.contains_key(key) {
        table.insert(key.to_string(), toml::Value::Table(toml::Table::new()));
    }

    table
        .get_mut(key)
        .and_then(toml::Value::as_table_mut)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Codex config key `{key}` must be a TOML table"),
            )
        })
}

fn enable_hook_unless_explicitly_disabled(hook_state: &mut toml::Table) {
    if hook_state.get("enabled").and_then(toml::Value::as_bool) == Some(false) {
        return;
    }
    hook_state.insert("enabled".to_string(), toml::Value::Boolean(true));
}

fn hook_key(
    key_source: &Path,
    event_name: &str,
    group_index: usize,
    handler_index: usize,
) -> String {
    format!(
        "{}:{event_name}:{group_index}:{handler_index}",
        key_source.display()
    )
}

fn command_hook_trusted_hash(event_name_snake: &str, matcher: &str, command: &str) -> String {
    let mut identity = json!({
        "event_name": event_name_snake,
        "hooks": [
            {
                "async": false,
                "command": command,
                "timeout": CODEX_DEFAULT_COMMAND_TIMEOUT_SECONDS,
                "type": "command"
            }
        ]
    });
    if codex_trust_identity_uses_matcher(event_name_snake) {
        identity
            .as_object_mut()
            .expect("Codex hook trust identity must be an object")
            .insert("matcher".to_string(), Value::String(matcher.to_string()));
    }
    sort_json_objects(&mut identity);
    let bytes = serde_json::to_vec(&identity).expect("serialize Codex hook trust identity");
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

fn codex_trust_identity_uses_matcher(event_name_snake: &str) -> bool {
    matches!(
        event_name_snake,
        "pre_tool_use"
            | "permission_request"
            | "post_tool_use"
            | "pre_compact"
            | "post_compact"
            | "session_start"
    )
}

fn sort_json_objects(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                sort_json_objects(item);
            }
        }
        Value::Object(map) => {
            let mut sorted = std::mem::take(map).into_iter().collect::<Vec<_>>();
            sorted.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (key, mut child) in sorted {
                sort_json_objects(&mut child);
                map.insert(key, child);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_generated_gwt_event_command(
    command: &str,
    event_json_name: &str,
    expected_gwt_bin: Option<&str>,
) -> bool {
    expected_generated_gwt_event_commands(event_json_name, expected_gwt_bin)
        .iter()
        .any(|expected| expected == command)
}

fn expected_generated_gwt_event_commands(
    event_json_name: &str,
    expected_gwt_bin: Option<&str>,
) -> Vec<String> {
    expected_gwt_bin.map_or_else(
        || codex_event_hook_commands(event_json_name),
        |bin| codex_event_hook_commands_with_bin(bin, event_json_name),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::{json, Value};

    use super::*;
    use crate::{
        generate_codex_hooks, generate_codex_hooks_for_mode,
        settings_local::codex_event_hook_commands_with_bin, CodexHookDiscoveryMode,
    };

    #[test]
    fn command_hook_hash_matches_codex_for_known_post_tool_use_fixture() {
        let command = "'/Applications/GWT.app/Contents/MacOS/gwtd' hook event PostToolUse";

        let trusted_hash = command_hook_trusted_hash_for_test("post_tool_use", "*", command);

        assert_eq!(
            trusted_hash,
            "sha256:9c3ce103f03f0b27a28bc4a30883f7e98a80b5df566b4572fcbb2955ebf5ba62"
        );
    }

    #[test]
    fn command_hook_hash_omits_codex_ignored_matchers_for_prompt_and_stop() {
        let user_prompt_command =
            "gwt_bin=\"${GWT_BIN_PATH:-/Applications/GWT.app/Contents/MacOS/gwtd}\"; \"$gwt_bin\" hook event UserPromptSubmit";
        let stop_command =
            "gwt_bin=\"${GWT_BIN_PATH:-/Applications/GWT.app/Contents/MacOS/gwtd}\"; \"$gwt_bin\" hook event Stop";

        assert_eq!(
            command_hook_trusted_hash_for_test("user_prompt_submit", "*", user_prompt_command),
            "sha256:1a86ba6796c5b5bf1601fd1af1d6094846287ec85e9f1ad4d39335c6b306e2fa"
        );
        assert_eq!(
            command_hook_trusted_hash_for_test("stop", "*", stop_command),
            "sha256:984e12cd30ef54cf4c63af8aabce1849705e5de09c70d039367ba68de9760389"
        );
    }

    #[test]
    fn generated_codex_hooks_produce_five_trust_entries() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = fs::canonicalize(dir.path().join(".codex/hooks.json")).unwrap();

        let entries = collect_codex_managed_hook_trust_entries(dir.path()).unwrap();

        assert_eq!(
            entries.len(),
            5,
            "expected one trust entry per managed event"
        );
        for event_name in [
            "session_start",
            "user_prompt_submit",
            "pre_tool_use",
            "post_tool_use",
            "stop",
        ] {
            let expected_key = format!("{}:{event_name}:0:0", hooks_path.display());
            let entry = entries
                .iter()
                .find(|entry| entry.key == expected_key)
                .unwrap_or_else(|| panic!("missing trust key {expected_key}; got {entries:?}"));
            assert!(
                entry.trusted_hash.starts_with("sha256:"),
                "trusted hash must use Codex sha256 prefix"
            );
        }
    }

    #[test]
    fn linked_worktree_trust_entries_use_root_checkout_hook_path() {
        let dir = tempfile::tempdir().unwrap();
        let root_checkout = dir.path().join("project");
        let common_git_dir = root_checkout.join("project.git");
        let worktree = root_checkout.join("work/20260524-0545");
        fs::create_dir_all(common_git_dir.join("worktrees/20260524-0545")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!(
                "gitdir: {}\n",
                common_git_dir.join("worktrees/20260524-0545").display()
            ),
        )
        .unwrap();
        generate_codex_hooks(&worktree).unwrap();
        let root_hooks_path = fs::canonicalize(root_checkout.join(".codex/hooks.json")).unwrap();
        let worktree_hooks_prefix = worktree.join(".codex/hooks.json").display().to_string();

        let entries = collect_codex_managed_hook_trust_entries(&worktree).unwrap();

        assert_eq!(entries.len(), 5);
        assert!(
            entries
                .iter()
                .all(|entry| entry.key.starts_with(&root_hooks_path.display().to_string())),
            "Codex 0.133 linked-worktree trust keys must use root checkout hook path {root_hooks_path:?}; got {entries:?}"
        );
        assert!(
            entries.iter().all(|entry| {
                !entry
                    .key
                    .starts_with(&worktree_hooks_prefix)
            }),
            "worktree-local hook keys are ignored by Codex linked-worktree discovery; got {entries:?}"
        );
    }

    #[test]
    fn linked_worktree_trust_entries_can_use_worktree_hook_path_for_older_codex() {
        let dir = tempfile::tempdir().unwrap();
        let root_checkout = dir.path().join("project");
        let common_git_dir = root_checkout.join("project.git");
        let worktree = root_checkout.join("work/20260524-0545");
        fs::create_dir_all(common_git_dir.join("worktrees/20260524-0545")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!(
                "gitdir: {}\n",
                common_git_dir.join("worktrees/20260524-0545").display()
            ),
        )
        .unwrap();
        generate_codex_hooks_for_mode(&worktree, CodexHookDiscoveryMode::WorktreeLocal).unwrap();
        let worktree_hooks_path = fs::canonicalize(worktree.join(".codex/hooks.json")).unwrap();

        let entries = collect_codex_managed_hook_trust_entries_for_mode(
            &worktree,
            CodexHookDiscoveryMode::WorktreeLocal,
        )
        .unwrap();

        assert_eq!(entries.len(), 5);
        assert!(
            entries
                .iter()
                .all(|entry| entry.key.starts_with(&worktree_hooks_path.display().to_string())),
            "Codex < 0.131.0-alpha.21 trust keys must use worktree hook path {worktree_hooks_path:?}; got {entries:?}"
        );
    }

    #[test]
    fn linked_worktree_trust_entries_can_register_both_paths_for_unknown_codex() {
        let dir = tempfile::tempdir().unwrap();
        let root_checkout = dir.path().join("project");
        let common_git_dir = root_checkout.join("project.git");
        let worktree = root_checkout.join("work/20260524-0545");
        fs::create_dir_all(common_git_dir.join("worktrees/20260524-0545")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!(
                "gitdir: {}\n",
                common_git_dir.join("worktrees/20260524-0545").display()
            ),
        )
        .unwrap();
        generate_codex_hooks_for_mode(&worktree, CodexHookDiscoveryMode::Both).unwrap();
        let root_hooks_path = fs::canonicalize(root_checkout.join(".codex/hooks.json")).unwrap();
        let worktree_hooks_path = fs::canonicalize(worktree.join(".codex/hooks.json")).unwrap();

        let entries = collect_codex_managed_hook_trust_entries_for_mode(
            &worktree,
            CodexHookDiscoveryMode::Both,
        )
        .unwrap();

        assert_eq!(entries.len(), 10);
        assert!(entries.iter().any(|entry| entry
            .key
            .starts_with(&root_hooks_path.display().to_string())));
        assert!(entries.iter().any(|entry| entry
            .key
            .starts_with(&worktree_hooks_path.display().to_string())));
    }

    #[test]
    fn portable_generated_hooks_are_trusted_for_explicit_expected_fallback_path() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let expected_fallback = "/host/gwt/bin/gwtd";
        let mut hooks = serde_json::Map::new();
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
        ] {
            hooks.insert(
                event.to_string(),
                json!([
                    {
                        "matcher": "*",
                        "hooks": [
                            {
                                "command": codex_event_hook_commands_with_bin(expected_fallback, event)
                                    .into_iter()
                                    .next()
                                    .unwrap(),
                                "type": "command"
                            }
                        ]
                    }
                ]),
            );
        }
        fs::write(
            codex_dir.join("hooks.json"),
            serde_json::to_string_pretty(&json!({ "hooks": hooks })).unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries_with_expected_bin(
            dir.path(),
            Some(expected_fallback),
        )
        .unwrap();

        assert_eq!(
            entries.len(),
            5,
            "container-local registration must accept the exact host-generated fallback path"
        );
    }

    #[test]
    fn powershell_generated_hook_with_expected_fallback_is_trusted_on_posix_registration() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let expected_fallback = "C:/Program Files/GWT/gwtd.exe";
        let powershell_stop_command = format!(
            "powershell -NoProfile -Command \"& {{ $gwtBin = if ($env:GWT_BIN_PATH) {{ $env:GWT_BIN_PATH }} else {{ '{expected_fallback}' }}; & $gwtBin hook event Stop }}\""
        );
        fs::write(
            codex_dir.join("hooks.json"),
            serde_json::to_string_pretty(&json!({
                "hooks": {
                    "Stop": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": powershell_stop_command,
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

        let entries = collect_codex_managed_hook_trust_entries_with_expected_bin(
            dir.path(),
            Some(expected_fallback),
        )
        .unwrap();

        assert_eq!(
            entries.len(),
            1,
            "Linux container registration must trust exact PowerShell-generated Codex hooks"
        );
    }

    #[test]
    fn portable_generated_hook_with_unexpected_fallback_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let hooks_content = fs::read_to_string(&hooks_path).unwrap();
        let mut hooks_json: Value = serde_json::from_str(&hooks_content).unwrap();
        hooks_json["hooks"]["Stop"][0]["hooks"][0]["command"] = Value::String(
            codex_event_hook_commands_with_bin("/tmp/attacker/gwtd", "Stop")
                .into_iter()
                .next()
                .unwrap(),
        );
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries(dir.path()).unwrap();

        assert_eq!(
            entries.len(),
            4,
            "unexpected portable fallback path must be left for Codex /hooks review"
        );
        assert!(
            entries.iter().all(|entry| !entry.key.contains(":stop:")),
            "unexpected fallback Stop hook must not be trusted; got {entries:?}"
        );
    }

    #[test]
    fn registration_preserves_unrelated_config_and_skips_user_hooks() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let hooks_content = fs::read_to_string(&hooks_path).unwrap();
        let mut hooks_json: Value = serde_json::from_str(&hooks_content).unwrap();
        hooks_json["hooks"]["PreToolUse"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "matcher": "Bash",
                "hooks": [
                    {
                        "command": "echo user-hook",
                        "type": "command"
                    }
                ]
            }));
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();

        let config_path = dir.path().join("codex-config.toml");
        fs::write(
            &config_path,
            r#"
[profiles.default]
model = "gpt-5.2"

[hooks.state."custom:pre_tool_use:0:0"]
enabled = false
"#,
        )
        .unwrap();

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(report.trusted_entries.len(), 5);
        let config = fs::read_to_string(&config_path).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        assert_eq!(
            parsed["profiles"]["default"]["model"].as_str(),
            Some("gpt-5.2")
        );
        assert_eq!(
            parsed["hooks"]["state"]["custom:pre_tool_use:0:0"]["enabled"].as_bool(),
            Some(false)
        );
        assert!(
            parsed["hooks"]["state"]["custom:pre_tool_use:0:0"]
                .get("trusted_hash")
                .is_none(),
            "unrelated hook state must not receive a trusted hash"
        );
        let hooks_path = fs::canonicalize(&hooks_path).unwrap();
        assert!(
            parsed["hooks"]["state"]
                .get(format!("{}:pre_tool_use:1:0", hooks_path.display()))
                .is_none(),
            "user hook entry must not be trusted"
        );
    }

    #[test]
    fn registration_enables_generated_managed_hooks() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = fs::canonicalize(dir.path().join(".codex/hooks.json")).unwrap();
        let config_path = dir.path().join("codex-config.toml");

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(report.trusted_entries.len(), 5);
        let config = fs::read_to_string(&config_path).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        for event_name in [
            "session_start",
            "user_prompt_submit",
            "pre_tool_use",
            "post_tool_use",
            "stop",
        ] {
            let key = format!("{}:{event_name}:0:0", hooks_path.display());
            let state = parsed["hooks"]["state"]
                .get(&key)
                .unwrap_or_else(|| panic!("missing managed Codex hook state entry: {key}"));
            assert_eq!(
                state.get("enabled").and_then(toml::Value::as_bool),
                Some(true),
                "managed Codex hook must be enabled: {key}"
            );
            assert!(
                state["trusted_hash"].as_str().is_some(),
                "managed Codex hook must still carry trusted_hash: {key}"
            );
        }
    }

    #[test]
    fn registration_preserves_explicit_managed_hook_opt_out() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = fs::canonicalize(dir.path().join(".codex/hooks.json")).unwrap();
        let pre_tool_key = format!("{}:pre_tool_use:0:0", hooks_path.display());
        let pre_tool_key_toml = pre_tool_key.replace('\\', "\\\\").replace('"', "\\\"");
        let config_path = dir.path().join("codex-config.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[hooks.state."{pre_tool_key_toml}"]
enabled = false
"#
            ),
        )
        .unwrap();

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(report.trusted_entries.len(), 5);
        let config = fs::read_to_string(&config_path).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        let state = &parsed["hooks"]["state"][&pre_tool_key];
        assert_eq!(
            state["enabled"].as_bool(),
            Some(false),
            "explicit managed hook opt-out must not be overwritten"
        );
        assert!(
            state["trusted_hash"].as_str().is_some(),
            "explicitly disabled managed hook should still receive the current trusted hash"
        );
    }

    #[test]
    fn registration_does_not_enable_user_or_modified_hooks() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let hooks_content = fs::read_to_string(&hooks_path).unwrap();
        let mut hooks_json: Value = serde_json::from_str(&hooks_content).unwrap();
        hooks_json["hooks"]["PreToolUse"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "matcher": "Bash",
                "hooks": [
                    {
                        "command": "echo user-hook",
                        "type": "command"
                    }
                ]
            }));
        hooks_json["hooks"]["Stop"][0]["hooks"][0]["command"] =
            Value::String("'gwtd' hook event Stop --unexpected".to_string());
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();
        let hooks_path = fs::canonicalize(&hooks_path).unwrap();
        let config_path = dir.path().join("codex-config.toml");

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(
            report.trusted_entries.len(),
            4,
            "only unchanged generated hooks should be trusted"
        );
        let config = fs::read_to_string(&config_path).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        assert!(
            parsed["hooks"]["state"]
                .get(format!("{}:pre_tool_use:1:0", hooks_path.display()))
                .is_none(),
            "user hook entry must not be enabled or trusted"
        );
        assert!(
            parsed["hooks"]["state"]
                .get(format!("{}:stop:0:0", hooks_path.display()))
                .is_none(),
            "modified generated hook must not be enabled or trusted"
        );
    }

    #[test]
    fn modified_gwt_command_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let hooks_content = fs::read_to_string(&hooks_path).unwrap();
        let mut hooks_json: Value = serde_json::from_str(&hooks_content).unwrap();
        hooks_json["hooks"]["Stop"][0]["hooks"][0]["command"] =
            Value::String("'gwtd' hook event Stop --unexpected".to_string());
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries(dir.path()).unwrap();

        assert_eq!(
            entries.len(),
            4,
            "modified gwt command must be left for Codex /hooks review"
        );
        assert!(
            entries.iter().all(|entry| !entry.key.contains(":stop:")),
            "modified Stop hook must not be trusted; got {entries:?}"
        );
    }

    #[test]
    fn gwt_command_with_modified_binary_path_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let hooks_content = fs::read_to_string(&hooks_path).unwrap();
        let mut hooks_json: Value = serde_json::from_str(&hooks_content).unwrap();
        hooks_json["hooks"]["Stop"][0]["hooks"][0]["command"] =
            Value::String("'/tmp/gwtd' hook event Stop".to_string());
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries(dir.path()).unwrap();

        assert_eq!(
            entries.len(),
            4,
            "path-modified gwt command must be left for Codex /hooks review"
        );
        assert!(
            entries.iter().all(|entry| !entry.key.contains(":stop:")),
            "path-modified Stop hook must not be trusted; got {entries:?}"
        );
    }
}
