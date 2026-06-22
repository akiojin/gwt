//! Worktree-local hook bridge assets for providers without Claude/Codex-style hooks.

use std::{io, path::Path};

use serde_json::{json, Value};

use crate::settings_local::{
    gwt_hook_bin_path, posix_shell_quote, set_executable, write_settings_atomically,
    write_text_atomically,
};

/// Generate OpenCode project-local hook bridge assets under `.gwt/opencode`.
///
/// Writes three artifacts:
/// - `plugins/gwt-hooks.js` — the hook bridge plugin (auto-loaded from
///   `OPENCODE_CONFIG_DIR/plugins/`).
/// - `opencode.json` — the project config that also references the plugin.
/// - `skip-permissions.json` — a permissive `permission` overlay layered in at
///   launch time via `OPENCODE_CONFIG` when a launch opts into skip_permissions
///   (SPEC-3151 FR-005). OpenCode has no skip-permissions CLI flag, so this
///   config overlay is the parity mechanism for `--yolo` /
///   `--dangerously-skip-permissions`.
pub fn generate_opencode_hooks(worktree: &Path) -> io::Result<()> {
    let config_dir = worktree.join(".gwt/opencode");
    let plugin_path = config_dir.join("plugins/gwt-hooks.js");
    let config_path = config_dir.join("opencode.json");
    let skip_permissions_path = config_dir.join("skip-permissions.json");
    let plugin_content = opencode_plugin_content(&gwt_hook_bin_path());
    let config = json!({
        "plugin": ["./plugins/gwt-hooks.js"],
    });
    let skip_permissions_config = json!({
        "permission": "allow",
    });

    write_text_atomically(&plugin_path, &plugin_content)?;
    write_settings_atomically(&config_path, &config)?;
    write_settings_atomically(&skip_permissions_path, &skip_permissions_config)
}

/// Generate Hermes Agent project-local hook config under `.gwt/hermes`.
pub fn generate_hermes_hooks(worktree: &Path) -> io::Result<()> {
    let home = worktree.join(".gwt/hermes");
    let config_path = home.join("config.yaml");
    let script_path = home.join("agent-hooks/gwt-hook.sh");

    write_text_atomically(
        &script_path,
        &hermes_hook_script_content(&gwt_hook_bin_path()),
    )?;
    set_executable(&script_path)?;
    write_text_atomically(&config_path, &hermes_config_content(&script_path))
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
