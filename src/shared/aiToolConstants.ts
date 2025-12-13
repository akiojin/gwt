/**
 * Shared constants for AI tool integrations.
 *
 * These values are consumed by both the CLI (Node) runtime and the Web UI so
 * that command previews, permission flags, and default arguments stay in sync.
 */

export const CLAUDE_PERMISSION_SKIP_ARGS = [
  "--dangerously-skip-permissions",
] as const;

export const CODEX_DEFAULT_ARGS = [
  "--enable",
  "web_search_request",
  "--model=gpt-5-codex",
  "--sandbox",
  "workspace-write",
  "-c",
  "model_reasoning_effort=high",
  "-c",
  "model_reasoning_summaries=detailed",
  "-c",
  "sandbox_workspace_write.network_access=true",
  "-c",
  "shell_environment_policy.inherit=all",
  "-c",
  "shell_environment_policy.ignore_default_excludes=true",
  "-c",
  "shell_environment_policy.experimental_use_profile=true",
] as const;
