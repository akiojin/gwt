//! Worktree-local hook bridge assets for providers without Claude/Codex-style hooks.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};

use crate::settings_local::{
    gwt_hook_bin_path, posix_shell_quote, set_executable, write_settings_atomically,
    write_text_atomically,
};

/// Generate OpenCode project-local hook bridge assets under `.gwt/opencode`.
pub fn generate_opencode_hooks(worktree: &Path) -> io::Result<()> {
    let config_dir = worktree.join(".gwt/opencode");
    let plugin_path = config_dir.join("plugins/gwt-hooks.js");
    let config_path = config_dir.join("opencode.json");
    let plugin_content = opencode_plugin_content(&gwt_hook_bin_path());
    let config = json!({
        "plugin": ["./plugins/gwt-hooks.js"],
    });

    write_text_atomically(&plugin_path, &plugin_content)?;
    write_settings_atomically(&config_path, &config)
}

/// Generate Hermes Agent project-local hook config under `.gwt/hermes` and
/// bridge the user's real Hermes setup (credentials + provider config) into
/// the worktree-local HERMES_HOME so the isolated home stays authenticated.
pub fn generate_hermes_hooks(worktree: &Path) -> io::Result<()> {
    let source_home = hermes_source_home(worktree);
    generate_hermes_hooks_with_source(worktree, source_home.as_deref())
}

/// Testable core for [`generate_hermes_hooks`]. `source_home` is the user's
/// real HERMES_HOME to bridge credentials and provider config from; `None`
/// when it cannot be resolved or the user has no global Hermes setup.
pub(crate) fn generate_hermes_hooks_with_source(
    worktree: &Path,
    source_home: Option<&Path>,
) -> io::Result<()> {
    let home = worktree.join(".gwt/hermes");
    let config_path = home.join("config.yaml");
    let script_path = home.join("agent-hooks/gwt-hook.sh");

    write_text_atomically(
        &script_path,
        &hermes_hook_script_content(&gwt_hook_bin_path()),
    )?;
    set_executable(&script_path)?;

    if let Some(src) = source_home {
        bridge_hermes_credentials(src, &home)?;
    }

    let merged = merge_hermes_config(source_home, &script_path)?;
    write_text_atomically(&config_path, &merged)
}

/// `true` when the user's real Hermes home has resolvable credentials
/// (a non-empty `.env` or an `auth.json`). Used by the launch wizard to
/// surface a non-blocking "Hermes is not set up" hint.
pub fn hermes_is_configured(source_home: &Path) -> bool {
    let env_ok = fs::read_to_string(source_home.join(".env"))
        .map(|content| !content.trim().is_empty())
        .unwrap_or(false);
    env_ok || source_home.join("auth.json").exists()
}

/// Enumerate the Hermes providers configured in the user's `config.yaml`:
/// the currently-selected `model.provider` first, then the keys under
/// `providers:`. Used to populate the launch wizard's provider dropdown from
/// real config instead of a stale hardcoded list. Returns an empty vec when
/// the config is absent or unparseable (the wizard then offers only the
/// "use config default" and free-text "Other" entries).
pub fn hermes_provider_choices(source_home: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(source_home.join("config.yaml")) else {
        return Vec::new();
    };
    let Ok(serde_yaml::Value::Mapping(root)) = serde_yaml::from_str::<serde_yaml::Value>(&text)
    else {
        return Vec::new();
    };

    let mut choices: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        let name = name.trim();
        if !name.is_empty() && !choices.iter().any(|p| p == name) {
            choices.push(name.to_string());
        }
    };

    if let Some(serde_yaml::Value::Mapping(model)) = root.get("model") {
        if let Some(serde_yaml::Value::String(provider)) = model.get("provider") {
            push(provider);
        }
    }
    if let Some(serde_yaml::Value::Mapping(providers)) = root.get("providers") {
        for key in providers.keys() {
            if let serde_yaml::Value::String(name) = key {
                push(name);
            }
        }
    }
    choices
}

/// Enumerate Hermes providers from the user's global HERMES_HOME
/// (`$HERMES_HOME` or `~/.hermes`). Convenience wrapper over
/// [`hermes_provider_choices`] for callers (e.g. the launch wizard) that only
/// have the global home, not a worktree.
pub fn hermes_provider_choices_global() -> Vec<String> {
    let home = match env::var_os("HERMES_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => match home_dir() {
            Some(home) => home.join(".hermes"),
            None => return Vec::new(),
        },
    };
    hermes_provider_choices(&home)
}

/// Resolve the user's real HERMES_HOME to bridge from. Honors an explicit
/// `HERMES_HOME` env var, else falls back to `~/.hermes`. Returns `None`
/// when it cannot be resolved or would point inside the worktree's managed
/// `.gwt/` tree (self-reference guard against a leaked redirect).
pub fn hermes_source_home(worktree: &Path) -> Option<PathBuf> {
    let candidate = match env::var_os("HERMES_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => home_dir()?.join(".hermes"),
    };
    if candidate.starts_with(worktree.join(".gwt")) {
        return None;
    }
    Some(candidate)
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Symlink the user's Hermes credentials (`.env`, `auth.json`) into the
/// worktree-local HERMES_HOME. Symlinks let OAuth token refreshes flow back
/// to the real home; on platforms/filesystems where symlinks fail we fall
/// back to a copy. Existing entries are replaced so refresh stays idempotent.
fn bridge_hermes_credentials(source_home: &Path, dest_home: &Path) -> io::Result<()> {
    fs::create_dir_all(dest_home)?;
    for name in [".env", "auth.json"] {
        let src = source_home.join(name);
        if !src.exists() {
            continue;
        }
        let dst = dest_home.join(name);
        if fs::symlink_metadata(&dst).is_ok() {
            let _ = fs::remove_file(&dst);
        }
        if symlink_file(&src, &dst).is_err() {
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink_file(src: &Path, dst: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(windows)]
fn symlink_file(src: &Path, dst: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(src, dst)
}

#[cfg(not(any(unix, windows)))]
fn symlink_file(_src: &Path, _dst: &Path) -> io::Result<()> {
    Err(io::Error::other("symlinks unsupported on this platform"))
}

/// Build the worktree-local `config.yaml` by merging the user's existing
/// Hermes config (model / provider / terminal / etc.) with gwt's managed
/// hook block. The user's source config is parsed but never modified; the
/// verbatim gwt hook block is appended so its exact shape is preserved.
fn merge_hermes_config(source_home: Option<&Path>, script_path: &Path) -> io::Result<String> {
    let hooks_block = hermes_config_content(script_path);
    let Some(source_home) = source_home else {
        return Ok(hooks_block);
    };
    let Ok(text) = fs::read_to_string(source_home.join("config.yaml")) else {
        return Ok(hooks_block);
    };
    let mut mapping = match serde_yaml::from_str::<serde_yaml::Value>(&text) {
        Ok(serde_yaml::Value::Mapping(mapping)) => mapping,
        _ => return Ok(hooks_block),
    };
    mapping.remove(serde_yaml::Value::from("hooks"));
    mapping.remove(serde_yaml::Value::from("hooks_auto_accept"));
    if mapping.is_empty() {
        return Ok(hooks_block);
    }
    let user_part =
        serde_yaml::to_string(&serde_yaml::Value::Mapping(mapping)).map_err(io::Error::other)?;
    Ok(format!("{user_part}{hooks_block}"))
}

/// Generate OpenClaw project-local hook bridge assets under `.gwt/openclaw`.
pub fn generate_openclaw_hooks(worktree: &Path) -> io::Result<()> {
    let config_dir = worktree.join(".gwt/openclaw");
    let plugin_dir = config_dir.join("plugins/gwt-hook-bridge");
    let config_path = config_dir.join("openclaw.json");

    write_settings_atomically(&config_path, &openclaw_config(&plugin_dir))?;
    write_text_atomically(
        &plugin_dir.join("package.json"),
        &openclaw_package_content(),
    )?;
    write_settings_atomically(
        &plugin_dir.join("openclaw.plugin.json"),
        &openclaw_manifest(),
    )?;
    write_text_atomically(
        &plugin_dir.join("plugin.ts"),
        &openclaw_plugin_content(&gwt_hook_bin_path()),
    )
}

fn js_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"gwtd\"".to_string())
}

fn opencode_plugin_content(bin: &str) -> String {
    let bin = js_string_literal(bin);
    format!(
        r#"import {{ spawnSync }} from "node:child_process";

const GWT_HOOK_BIN = {bin};

function canonicalPayload(nativeEvent, input = {{}}, output = {{}}, context = {{}}) {{
  const toolName = input.tool ?? input.toolName ?? output.tool ?? output.toolName ?? input.name;
  const toolInput = output.args ?? input.args ?? output.params ?? input.params ?? output.input ?? input.input ?? {{}};
  return {{
    provider: "opencode",
    native_event: nativeEvent,
    tool_name: toolName,
    tool_input: toolInput,
    session_id: input.sessionID ?? input.sessionId ?? input.session_id ?? context.sessionID ?? context.sessionId,
    cwd: input.cwd ?? context.directory ?? context.worktree ?? context.project?.root,
    input,
    output,
  }};
}}

function dispatch(nativeEvent, input, output, context) {{
  const result = spawnSync(
    GWT_HOOK_BIN,
    ["hook", "provider-event", "opencode", nativeEvent],
    {{
      input: JSON.stringify(canonicalPayload(nativeEvent, input, output, context)),
      encoding: "utf8",
      stdio: ["pipe", "pipe", "ignore"],
    }},
  );
  try {{
    return result.stdout ? JSON.parse(result.stdout) : {{}};
  }} catch {{
    return {{}};
  }}
}}

function blockReason(result) {{
  return result.hookSpecificOutput?.permissionDecisionReason ?? result.reason;
}}

export const GwtHooks = async (context) => ({{
  "session.created": async (input, output) => dispatch("session.created", input, output, context),
  "message.updated": async (input, output) => dispatch("message.updated", input, output, context),
  "tool.execute.before": async (input, output) => {{
    const reason = blockReason(dispatch("tool.execute.before", input, output, context));
    if (reason) throw new Error(reason);
  }},
  "tool.execute.after": async (input, output) => dispatch("tool.execute.after", input, output, context),
  "session.idle": async (input, output) => dispatch("session.idle", input, output, context),
}});
"#
    )
}

fn hermes_hook_command(script_path: &Path, event: &str) -> String {
    js_string_literal(&format!(
        "{} {event}",
        posix_shell_quote(script_path.to_string_lossy().as_ref())
    ))
}

fn hermes_config_content(script_path: &Path) -> String {
    let on_session_start = hermes_hook_command(script_path, "on_session_start");
    let pre_llm_call = hermes_hook_command(script_path, "pre_llm_call");
    let pre_tool_call = hermes_hook_command(script_path, "pre_tool_call");
    let post_tool_call = hermes_hook_command(script_path, "post_tool_call");
    let on_session_end = hermes_hook_command(script_path, "on_session_end");
    format!(
        r#"hooks:
  on_session_start:
    - command: {on_session_start}
      timeout: 10
  pre_llm_call:
    - command: {pre_llm_call}
      timeout: 10
  pre_tool_call:
    - matcher: ".*"
      command: {pre_tool_call}
      timeout: 10
  post_tool_call:
    - matcher: ".*"
      command: {post_tool_call}
      timeout: 10
  on_session_end:
    - command: {on_session_end}
      timeout: 10
hooks_auto_accept: true
"#
    )
}

fn hermes_hook_script_content(bin: &str) -> String {
    let bin = posix_shell_quote(bin);
    r#"#!/bin/sh
set -eu

event="${1:-}"
if [ -z "$event" ]; then
  exit 0
fi

payload="$(cat)"
set +e
output="$(printf '%s' "$payload" | __GWT_HOOK_BIN__ hook provider-event hermes "$event")"
set -e
if [ -n "$output" ]; then
  printf '%s\n' "$output"
fi
exit 0
"#
    .replace("__GWT_HOOK_BIN__", &bin)
}

fn openclaw_config(plugin_dir: &Path) -> Value {
    json!({
        "commands": {
            "native": "auto",
            "nativeSkills": "auto",
            "restart": true,
            "ownerDisplay": "raw",
        },
        "plugins": {
            "enabled": true,
            "load": {
                "paths": [plugin_dir.to_string_lossy().to_string()],
            },
            "entries": {
                "gwt-hook-bridge": {
                    "enabled": true,
                    "hooks": {
                        "allowPromptInjection": true,
                        "allowConversationAccess": false,
                    },
                },
            },
        },
    })
}

fn openclaw_manifest() -> Value {
    json!({
        "id": "gwt-hook-bridge",
        "name": "gwt Hook Bridge",
        "description": "Routes OpenClaw plugin hook events into gwtd hook provider-event.",
        "configSchema": {
            "type": "object",
            "additionalProperties": false,
        },
    })
}

fn openclaw_package_content() -> String {
    r#"{
  "name": "gwt-hook-bridge",
  "version": "0.0.0",
  "type": "module",
  "private": true,
  "openclaw": {
    "extensions": ["./plugin.ts"]
  }
}
"#
    .to_string()
}

fn openclaw_plugin_content(bin: &str) -> String {
    let bin = js_string_literal(bin);
    format!(
        r#"import {{ spawnSync }} from "node:child_process";
import {{ definePluginEntry }} from "openclaw/plugin-sdk/plugin-entry";

const GWT_HOOK_BIN = {bin};

function dispatch(nativeEvent, event = {{}}, ctx = {{}}) {{
  const payload = {{
    provider: "openclaw",
    native_event: nativeEvent,
    tool_name: event.toolName ?? event.tool_name,
    tool_input: event.params ?? event.args ?? event.toolInput ?? event.tool_input ?? {{}},
    session_id: event.sessionId ?? event.session_id ?? ctx.sessionId ?? ctx.sessionKey,
    cwd: event.cwd ?? ctx.cwd ?? ctx.workspaceRoot,
    event,
    ctx,
  }};
  const result = spawnSync(
    GWT_HOOK_BIN,
    ["hook", "provider-event", "openclaw", nativeEvent],
    {{
      input: JSON.stringify(payload),
      encoding: "utf8",
      stdio: ["pipe", "pipe", "ignore"],
    }},
  );
  try {{
    return result.stdout ? JSON.parse(result.stdout) : {{}};
  }} catch {{
    return {{}};
  }}
}}

function blockResult(result) {{
  const reason = result.hookSpecificOutput?.permissionDecisionReason ?? result.reason;
  if (!reason) return undefined;
  return {{ block: true, blockReason: reason }};
}}

function promptContextResult(result) {{
  const text = result.hookSpecificOutput?.additionalContext ?? result.context;
  if (!text) return undefined;
  return {{ prependContext: text }};
}}

export default definePluginEntry({{
  id: "gwt-hook-bridge",
  name: "gwt Hook Bridge",
  description: "Routes OpenClaw hook events into gwtd.",
  register(api) {{
    api.on("session_start", async (event, ctx) => dispatch("session_start", event, ctx));
    api.on("before_prompt_build", async (event, ctx) => promptContextResult(dispatch("before_prompt_build", event, ctx)));
    api.on("before_tool_call", async (event, ctx) => blockResult(dispatch("before_tool_call", event, ctx)));
    api.on("after_tool_call", async (event, ctx) => dispatch("after_tool_call", event, ctx));
    api.on("session_end", async (event, ctx) => dispatch("session_end", event, ctx));
  }},
}});
"#
    )
}

#[cfg(test)]
mod hermes_tests {
    use super::*;
    use std::fs;

    fn write_file(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn bridges_env_and_auth_via_symlink_when_present() {
        let wt = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        write_file(&src.path().join(".env"), "HERMES_API_KEY=secret\n");
        write_file(&src.path().join("auth.json"), "{\"nous\":\"tok\"}\n");

        generate_hermes_hooks_with_source(wt.path(), Some(src.path())).unwrap();

        let dest = wt.path().join(".gwt/hermes");
        let env_link = dest.join(".env");
        let auth_link = dest.join("auth.json");
        assert!(env_link.exists(), ".env must be bridged");
        assert!(auth_link.exists(), "auth.json must be bridged");
        assert_eq!(
            fs::read_to_string(&env_link).unwrap(),
            "HERMES_API_KEY=secret\n"
        );
        #[cfg(unix)]
        {
            assert!(fs::symlink_metadata(&env_link)
                .unwrap()
                .file_type()
                .is_symlink());
            assert!(fs::symlink_metadata(&auth_link)
                .unwrap()
                .file_type()
                .is_symlink());
        }
    }

    #[test]
    fn skips_missing_credentials_when_source_unconfigured() {
        let wt = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();

        generate_hermes_hooks_with_source(wt.path(), Some(src.path())).unwrap();

        let dest = wt.path().join(".gwt/hermes");
        assert!(!dest.join(".env").exists());
        assert!(!dest.join("auth.json").exists());
        assert!(dest.join("config.yaml").exists());
    }

    #[test]
    fn merges_user_model_with_gwt_hooks_without_touching_source() {
        let wt = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let src_config = src.path().join("config.yaml");
        let original = "model: openrouter/anthropic/claude-sonnet-4\nterminal:\n  backend: pty\n";
        write_file(&src_config, original);

        generate_hermes_hooks_with_source(wt.path(), Some(src.path())).unwrap();

        let dest_config = fs::read_to_string(wt.path().join(".gwt/hermes/config.yaml")).unwrap();
        assert!(dest_config.contains("model:"));
        assert!(dest_config.contains("openrouter/anthropic/claude-sonnet-4"));
        assert!(dest_config.contains("hooks_auto_accept: true"));
        assert!(dest_config.contains("on_session_start"));
        // The user's global config must never be mutated by the merge.
        assert_eq!(fs::read_to_string(&src_config).unwrap(), original);
    }

    #[test]
    fn merge_is_idempotent_and_migrates_old_hooks_only_config() {
        let wt = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        write_file(&src.path().join(".env"), "K=v\n");
        write_file(&src.path().join("config.yaml"), "model: nous/hermes-4\n");

        // Simulate a pre-existing hooks-only dest config (older gwt output).
        let dest_config = wt.path().join(".gwt/hermes/config.yaml");
        write_file(
            &dest_config,
            "hooks:\n  on_session_start: []\nhooks_auto_accept: true\n",
        );

        generate_hermes_hooks_with_source(wt.path(), Some(src.path())).unwrap();
        let first = fs::read_to_string(&dest_config).unwrap();
        assert!(first.contains("model: nous/hermes-4"));
        assert!(first.contains("on_session_start"));

        generate_hermes_hooks_with_source(wt.path(), Some(src.path())).unwrap();
        let second = fs::read_to_string(&dest_config).unwrap();
        assert_eq!(first, second, "merge must be byte-identical on refresh");
        assert!(wt.path().join(".gwt/hermes/.env").exists());
    }

    #[test]
    fn provider_choices_reads_model_provider_then_providers_keys() {
        let src = tempfile::tempdir().unwrap();
        fs::write(
            src.path().join("config.yaml"),
            "model:\n  provider: zai\n  default: glm-5.2\nproviders:\n  ollama-launch:\n    base_url: http://x\n  myvault:\n    base_url: http://y\n",
        )
        .unwrap();

        let choices = hermes_provider_choices(src.path());
        // The currently-selected provider comes first, then user-defined keys.
        assert_eq!(choices.first().map(String::as_str), Some("zai"));
        assert!(choices.iter().any(|p| p == "ollama-launch"));
        assert!(choices.iter().any(|p| p == "myvault"));
    }

    #[test]
    fn provider_choices_empty_without_config() {
        let src = tempfile::tempdir().unwrap();
        assert!(hermes_provider_choices(src.path()).is_empty());
    }

    #[test]
    fn none_source_writes_hooks_only_config() {
        let wt = tempfile::tempdir().unwrap();

        generate_hermes_hooks_with_source(wt.path(), None).unwrap();

        let dest = wt.path().join(".gwt/hermes");
        assert!(dest.join("config.yaml").exists());
        assert!(!dest.join(".env").exists());
        let config = fs::read_to_string(dest.join("config.yaml")).unwrap();
        assert!(config.contains("hooks_auto_accept: true"));
    }
}
