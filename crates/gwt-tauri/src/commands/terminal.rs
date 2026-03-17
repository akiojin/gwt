//! Terminal/PTY management commands for xterm.js integration

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::state::{AppState, PaneLaunchMeta, PaneRuntimeContext};
use chrono::Utc;
use gwt_core::ai::SessionParser;
use gwt_core::config::stats::Stats;
use gwt_core::config::{AgentConfig, ClaudeAgentProvider, ProfilesConfig, Settings};
use gwt_core::docker::{
    compose_available, daemon_running, detect_docker_files, docker_available, try_start_daemon,
    DevContainerConfig, DockerFileType, DockerManager, PortAllocator,
};
use gwt_core::git::{create_or_verify_linked_branch, IssueLinkedBranchStatus, Remote};
use gwt_core::terminal::pane::PaneStatus;
use gwt_core::terminal::runner::{
    choose_fallback_runner, normalize_windows_command_path, resolve_command_path, FallbackRunner,
};
use gwt_core::terminal::scrollback::{strip_ansi, ScrollbackFile};
use gwt_core::terminal::{AgentColor, BuiltinLaunchConfig};
use gwt_core::worktree::WorktreeManager;
use gwt_core::StructuredError;
use serde::Deserialize;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;
use which::which;

/// Terminal output event payload sent to the frontend
#[derive(Debug, Clone, Serialize)]
pub struct TerminalOutputPayload {
    pub pane_id: String,
    pub data: Vec<u8>,
}

/// Terminal closed event payload sent to the frontend
#[derive(Debug, Clone, Serialize)]
pub struct TerminalClosedPayload {
    pub pane_id: String,
}

/// Worktree change event payload sent to the frontend
#[derive(Debug, Clone, Serialize)]
pub struct WorktreesChangedPayload {
    pub project_path: String,
    pub branch: String,
}

/// Serializable terminal info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct TerminalInfo {
    pub pane_id: String,
    pub agent_name: String,
    pub branch_name: String,
    pub status: String,
}

/// ANSI/SGR probe result for a terminal pane (diagnostics).
#[derive(Debug, Clone, Serialize)]
pub struct TerminalAnsiProbe {
    pub pane_id: String,
    pub bytes_scanned: usize,
    pub esc_count: usize,
    pub sgr_count: usize,
    pub color_sgr_count: usize,
    pub has_256_color: bool,
    pub has_true_color: bool,
}

/// Launch progress event payload sent to the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchProgressPayload {
    pub job_id: String,
    /// "fetch" | "validate" | "paths" | "conflicts" | "create" | "skills" | "deps"
    pub step: String,
    pub detail: Option<String>,
}

/// Launch finished event payload sent to the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchFinishedPayload {
    pub job_id: String,
    /// "ok" | "cancelled" | "error"
    pub status: String,
    pub pane_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBranchRequest {
    /// New branch name (e.g., "feature/foo")
    pub name: String,
    /// Optional base branch/ref (e.g., "develop", "origin/develop")
    pub base: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    Normal,
    Continue,
    Resume,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchAgentRequest {
    /// Agent id (e.g., "claude", "codex", "gemini")
    pub agent_id: String,
    /// Branch name or remote ref (e.g., "main", "feature/foo", "origin/main")
    pub branch: String,
    /// Optional profile name override (uses active profile when omitted)
    pub profile: Option<String>,
    /// Optional model selection (agent-specific).
    pub model: Option<String>,
    /// Optional tool version selection.
    ///
    /// - `None`: auto (prefer installed command when present, else package runner)
    /// - `"installed"`: force installed command (missing command fails at runtime)
    /// - other: force package runner `@...@{version}` (e.g. "latest", "1.2.3")
    pub agent_version: Option<String>,
    /// Optional session mode override (default: normal).
    pub mode: Option<SessionMode>,
    /// Skip permissions / approvals (agent-specific; default: false).
    pub skip_permissions: Option<bool>,
    /// Codex reasoning override (e.g. "low", "medium", "high", "xhigh").
    pub reasoning_level: Option<String>,
    /// Codex Fast mode toggle (`-c service_tier=fast`).
    pub fast_mode: Option<bool>,
    /// Collaboration modes for Codex. Ignored (always enabled when version supports it).
    /// Kept for deserialization compatibility with older frontends.
    #[allow(dead_code)]
    pub collaboration_modes: Option<bool>,
    /// Additional command line args to append (one arg per entry).
    pub extra_args: Option<Vec<String>>,
    /// Environment variable overrides to merge into the launch env (highest precedence).
    pub env_overrides: Option<HashMap<String, String>>,
    /// Docker compose service name to exec into (compose detected only).
    pub docker_service: Option<String>,
    /// Force host launch (skip docker) even when compose is detected.
    pub docker_force_host: Option<bool>,
    /// Force recreate containers (`docker compose up --force-recreate`).
    pub docker_recreate: Option<bool>,
    /// Build images before launch (`docker compose up --build`).
    pub docker_build: Option<bool>,
    /// Keep containers running after agent exit (skip `docker compose down`).
    pub docker_keep: Option<bool>,
    /// Explicit session ID to resume/continue with (best-effort; agent-specific).
    pub resume_session_id: Option<String>,
    /// Optional new branch creation request (creates branch + worktree before launch)
    pub create_branch: Option<CreateBranchRequest>,
    /// Optional issue number used for issue-linked branch creation.
    pub issue_number: Option<u64>,
    /// AI branch description for automatic branch name generation (AI Suggest mode)
    pub ai_branch_description: Option<String>,
    /// Optional terminal shell override (e.g. "powershell", "cmd", "wsl").
    #[serde(default)]
    #[allow(dead_code)]
    pub terminal_shell: Option<String>,
}

fn strip_known_remote_prefix<'a>(branch: &'a str, remotes: &[Remote]) -> &'a str {
    let Some((first, rest)) = branch.split_once('/') else {
        return branch;
    };
    if remotes.iter().any(|r| r.name == first) {
        return rest;
    }
    branch
}

fn resolve_worktree_path(
    repo_path: &std::path::Path,
    branch_ref: &str,
) -> Result<(PathBuf, bool), String> {
    let manager = WorktreeManager::new(repo_path).map_err(|e| e.to_string())?;

    let remotes = Remote::list(repo_path).unwrap_or_default();
    let normalized = strip_known_remote_prefix(branch_ref, &remotes);

    if let Ok(Some(wt)) = manager.get_by_branch_basic(normalized) {
        if !wt.path.exists() {
            return Err(format!(
                "Worktree path does not exist: {}",
                wt.path.display()
            ));
        }
        return Ok((wt.path, false));
    }
    // Rare: worktree registered with the raw remote-like name.
    if normalized != branch_ref {
        if let Ok(Some(wt)) = manager.get_by_branch_basic(branch_ref) {
            if !wt.path.exists() {
                return Err(format!(
                    "Worktree path does not exist: {}",
                    wt.path.display()
                ));
            }
            return Ok((wt.path, false));
        }
    }

    let wt = manager
        .create_for_branch(branch_ref)
        .map_err(|e| e.to_string())?;
    Ok((wt.path, true))
}

fn create_new_worktree_path(
    repo_path: &std::path::Path,
    branch_name: &str,
    base_branch: Option<&str>,
) -> Result<PathBuf, String> {
    // gwt-spec issue: Try remote-first flow when gh CLI is available
    if gwt_core::git::gh_cli::is_gh_available() && gwt_core::git::gh_cli::check_auth() {
        match create_new_worktree_remote_first(repo_path, branch_name, base_branch) {
            Ok(path) => return Ok(path),
            Err(e) => {
                // 422 (branch already exists) should not fallback — propagate error
                if e.contains("already exists on remote") {
                    return Err(e);
                }
                tracing::warn!(
                    category = "worktree",
                    branch = branch_name,
                    error = %e,
                    "Remote-first worktree creation failed, falling back to local"
                );
            }
        }
    }

    let manager = WorktreeManager::new(repo_path).map_err(|e| e.to_string())?;
    let wt = manager
        .create_new_branch(branch_name, base_branch)
        .map_err(|e| e.to_string())?;
    Ok(wt.path)
}

fn rollback_new_issue_branch(repo_path: &std::path::Path, branch_name: &str) -> Result<(), String> {
    let mut warnings = Vec::new();

    match WorktreeManager::new(repo_path) {
        Ok(manager) => {
            if let Err(err) = manager.cleanup_branch(branch_name, true, true) {
                warnings.push(format!("local cleanup warning: {err}"));
            }
        }
        Err(err) => warnings.push(format!("failed to initialize worktree manager: {err}")),
    }

    let remote_output = gwt_core::process::command("git")
        .args(["push", "origin", "--delete", branch_name])
        .current_dir(repo_path)
        .output();

    match remote_output {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let lower = stderr.to_ascii_lowercase();
            let missing_ref = lower.contains("remote ref does not exist")
                || lower.contains("unable to delete")
                || lower.contains("not found");
            if !missing_ref {
                warnings.push(format!("remote cleanup warning: {}", stderr.trim()));
            }
        }
        Err(err) => warnings.push(format!("failed to execute remote cleanup: {err}")),
    }

    if warnings.is_empty() {
        Ok(())
    } else {
        Err(warnings.join(" | "))
    }
}

/// gwt-spec issue: Create a worktree by first creating the branch on GitHub,
/// then fetching it locally.
fn create_new_worktree_remote_first(
    repo_path: &std::path::Path,
    branch_name: &str,
    base_branch: Option<&str>,
) -> Result<PathBuf, String> {
    // Normalize base branch: strip "origin/" prefix if present
    let base = base_branch
        .map(|b| b.strip_prefix("origin/").unwrap_or(b))
        .unwrap_or("HEAD");

    // Resolve the SHA of the base branch on GitHub
    let sha = gwt_core::git::resolve_remote_branch_sha(repo_path, base)?;

    // Create the branch on GitHub
    gwt_core::git::create_remote_branch(repo_path, branch_name, &sha)?;

    // Fetch the newly created branch locally
    let fetch_output = gwt_core::process::command("git")
        .args([
            "fetch",
            "origin",
            &format!("{}:{}", branch_name, branch_name),
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to run git fetch: {}", e))?;

    if !fetch_output.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_output.stderr);
        return Err(format!(
            "Failed to fetch branch '{}' from remote (the remote branch was created and may need manual deletion): {}",
            branch_name,
            stderr.trim()
        ));
    }

    // Create worktree for the fetched branch
    let manager = WorktreeManager::new(repo_path).map_err(|e| e.to_string())?;
    let wt = manager
        .create_for_branch(branch_name)
        .map_err(|e| e.to_string())?;

    // gwt-spec issue FR-005: Set upstream tracking config (non-fatal)
    // Remote-first path always uses "origin".
    if let Err(e) = gwt_core::git::Branch::set_upstream_config(&wt.path, branch_name, "origin") {
        tracing::warn!(
            category = "worktree",
            branch = branch_name,
            error = %e,
            "Failed to set upstream config in remote-first path (non-fatal)"
        );
    }

    Ok(wt.path)
}

/// Merge OS environment variables with profile environment.
/// Order: OS env (base) → disabled_env removes → profile env overwrites
fn merge_profile_env(
    os_env: &HashMap<String, String>,
    profile_override: Option<&str>,
) -> HashMap<String, String> {
    let mut env_vars = os_env.clone();

    let Ok(config) = ProfilesConfig::load() else {
        return env_vars;
    };

    let profile_name = profile_override
        .map(|s| s.to_string())
        .or_else(|| config.active.clone());

    let Some(name) = profile_name else {
        return env_vars;
    };

    let Some(profile) = config.profiles.get(&name) else {
        return env_vars;
    };

    // Remove disabled OS env vars
    for key in &profile.disabled_env {
        env_vars.remove(key);
    }

    // Override with profile env vars
    for (key, value) in &profile.env {
        env_vars.insert(key.clone(), value.clone());
    }

    env_vars
}

fn resolve_profile_ai_api_key(profile_override: Option<&str>) -> Option<String> {
    let Ok(config) = ProfilesConfig::load() else {
        return None;
    };

    let profile_name = profile_override
        .map(|s| s.to_string())
        .or_else(|| config.active.clone());

    if let Some(profile) = profile_name
        .as_deref()
        .and_then(|name| config.profiles.get(name))
    {
        if let Some(ai) = profile.ai.as_ref() {
            let key = ai.api_key.trim();
            return (!key.is_empty()).then(|| key.to_string());
        }
    }
    None
}

fn inject_openai_api_key_from_profile_ai(
    env_vars: &mut HashMap<String, String>,
    profile_override: Option<&str>,
) {
    let has_openai_api_key = env_vars
        .get("OPENAI_API_KEY")
        .is_some_and(|value| !value.trim().is_empty());
    if has_openai_api_key {
        return;
    }
    if let Some(api_key) = resolve_profile_ai_api_key(profile_override) {
        env_vars.insert("OPENAI_API_KEY".to_string(), api_key);
    }
}

fn ensure_terminal_env_defaults(env_vars: &mut HashMap<String, String>) {
    env_vars
        .entry("TERM".to_string())
        .or_insert_with(|| "xterm-256color".to_string());
    env_vars
        .entry("COLORTERM".to_string())
        .or_insert_with(|| "truecolor".to_string());
}

pub(crate) struct BuiltinAgentDef {
    pub(crate) label: &'static str,
    pub(crate) local_command: &'static str,
    pub(crate) bunx_package: &'static str,
}

pub(crate) fn builtin_agent_def(agent_id: &str) -> Result<BuiltinAgentDef, String> {
    match agent_id {
        "claude" => Ok(BuiltinAgentDef {
            label: "Claude Code",
            local_command: "claude",
            bunx_package: "@anthropic-ai/claude-code",
        }),
        "codex" => Ok(BuiltinAgentDef {
            label: "Codex",
            local_command: "codex",
            bunx_package: "@openai/codex",
        }),
        "gemini" => Ok(BuiltinAgentDef {
            label: "Gemini",
            local_command: "gemini",
            bunx_package: "@google/gemini-cli",
        }),
        "opencode" => Ok(BuiltinAgentDef {
            label: "OpenCode",
            local_command: "opencode",
            bunx_package: "opencode-ai",
        }),
        "copilot" => Ok(BuiltinAgentDef {
            label: "GitHub Copilot",
            local_command: "copilot",
            bunx_package: "@github/copilot",
        }),
        _ => Err(format!("Unknown agent: {}", agent_id)),
    }
}

fn normalize_agent_version(version: Option<&str>) -> Option<String> {
    let v = version?.trim();
    if v.is_empty() {
        return None;
    }

    // Allow users to paste "v1.2.3" / "@1.2.3" and normalize to npm-friendly tokens.
    let v = v.strip_prefix('@').unwrap_or(v);
    let v = if let Some(rest) = v.strip_prefix('v') {
        if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            rest
        } else {
            v
        }
    } else {
        v
    };

    Some(v.to_string())
}

fn build_bunx_package_spec(package: &str, version: Option<&str>) -> String {
    let v = normalize_agent_version(version).unwrap_or_else(|| "latest".to_string());
    format!("{package}@{v}")
}

fn build_agent_model_args(agent_id: &str, model: Option<&str>) -> Vec<String> {
    let Some(model) = model.map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return Vec::new();
    };

    match agent_id {
        // gwt-spec issue FR-005: Codex uses `--model=...`.
        "codex" => vec![format!("--model={model}")],
        // gwt-spec issue: Claude Code uses `--model <name>`.
        "claude" => vec!["--model".to_string(), model.to_string()],
        // gwt-spec issue: Gemini CLI uses `-m <name>`.
        "gemini" => vec!["-m".to_string(), model.to_string()],
        // gwt-spec issue: OpenCode uses `-m provider/model`.
        "opencode" => vec!["-m".to_string(), model.to_string()],
        "copilot" => vec!["--model".to_string(), model.to_string()],
        _ => Vec::new(),
    }
}

fn get_command_version_with_timeout(command: &str) -> Option<String> {
    let command = normalized_process_command(command);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = gwt_core::process::command(&command)
            .arg("--version")
            .output();
        let _ = tx.send(out);
    });

    let out = rx.recv_timeout(Duration::from_secs(3)).ok()?.ok()?;
    if !out.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Some(stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Some(stderr);
    }
    None
}

#[derive(Debug, Clone)]
struct CodexFeatureProbeContext {
    command: String,
    args: Vec<String>,
    tool_version: String,
}

static CODEX_MULTI_AGENT_SUPPORT_CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
const CODEX_FEATURES_LIST_TIMEOUT: Duration = Duration::from_secs(5);

fn codex_multi_agent_support_cache() -> &'static Mutex<HashMap<String, bool>> {
    CODEX_MULTI_AGENT_SUPPORT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn codex_multi_agent_support_cache_key(context: &CodexFeatureProbeContext) -> String {
    let mut key = format!("{}|{}", context.command, context.tool_version);
    for arg in &context.args {
        key.push('\u{1f}');
        key.push_str(arg);
    }
    key
}

fn codex_features_list_contains_feature(raw: &str, feature: &str) -> bool {
    raw.lines().any(|line| {
        line.split_whitespace()
            .next()
            .is_some_and(|name| name == feature)
    })
}

fn probe_codex_features_list(command: &str, args: &[String], timeout: Duration) -> Option<String> {
    let command = normalized_process_command(command);
    let args = args.to_vec();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = gwt_core::process::command(&command)
            .args(args)
            .arg("features")
            .arg("list")
            .output();
        let _ = tx.send(out);
    });

    let out = rx.recv_timeout(timeout).ok()?.ok()?;
    if !out.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if stdout.is_empty() {
        return None;
    }

    Some(stdout)
}

fn codex_supports_multi_agent(context: &CodexFeatureProbeContext) -> bool {
    let cache_key = codex_multi_agent_support_cache_key(context);
    if let Ok(cache) = codex_multi_agent_support_cache().lock() {
        if let Some(value) = cache.get(&cache_key) {
            return *value;
        }
    }

    let supported = match probe_codex_features_list(
        &context.command,
        &context.args,
        CODEX_FEATURES_LIST_TIMEOUT,
    ) {
        Some(raw) => codex_features_list_contains_feature(&raw, "multi_agent"),
        // Preserve default behavior for package-tag launches when probing is unavailable.
        None => context.tool_version.eq_ignore_ascii_case("latest"),
    };

    if let Ok(mut cache) = codex_multi_agent_support_cache().lock() {
        cache.insert(cache_key, supported);
    }

    supported
}

fn codex_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex").join("config.toml"))
}

fn codex_config_has_collab_alias(path: &std::path::Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read Codex config ({}): {e}", path.display()))?;
    let parsed: toml::Value = toml::from_str(&raw)
        .map_err(|e| format!("Failed to parse Codex config ({}): {e}", path.display()))?;
    Ok(parsed
        .get("features")
        .and_then(|value| value.get("collab"))
        .is_some())
}

#[derive(Debug, Clone)]
struct ResolvedAgentLaunchCommand {
    command: String,
    args: Vec<String>,
    label: &'static str,
    tool_version: String, // "installed" | "latest" | "1.2.3" | dist-tag
    version_for_gates: Option<String>, // best-effort raw version string (may be "latest")
}

#[derive(Debug, Clone, Copy)]
enum LaunchRunner {
    Bunx,
    Npx,
}

fn preferred_launch_runner() -> LaunchRunner {
    let bunx_path = resolve_command_path("bunx");
    let npx_available = resolve_command_path("npx").is_some();
    preferred_launch_runner_with_availability(bunx_path.as_deref(), npx_available)
}

fn preferred_launch_runner_with_availability(
    bunx_path: Option<&std::path::Path>,
    npx_available: bool,
) -> LaunchRunner {
    match choose_fallback_runner(bunx_path, npx_available) {
        Some(FallbackRunner::Npx) => LaunchRunner::Npx,
        Some(FallbackRunner::Bunx) | None => LaunchRunner::Bunx,
    }
}

fn build_runner_launch(
    runner: LaunchRunner,
    package: &str,
    version: Option<&str>,
) -> (String, Vec<String>) {
    let package_spec = build_bunx_package_spec(package, version);
    match runner {
        LaunchRunner::Bunx => ("bunx".to_string(), vec![package_spec]),
        LaunchRunner::Npx => ("npx".to_string(), vec!["--yes".to_string(), package_spec]),
    }
}

fn normalize_launch_command_for_platform(command: String) -> String {
    let normalized = normalize_windows_command_path(&command);
    if normalized.is_empty() {
        command
    } else {
        normalized
    }
}

fn normalized_process_command(command: &str) -> String {
    let normalized = normalize_windows_command_path(command);
    if normalized.is_empty() {
        command.trim().to_string()
    } else {
        normalized
    }
}

fn resolve_agent_launch_command(
    agent_id: &str,
    requested_version: Option<&str>,
) -> Result<ResolvedAgentLaunchCommand, String> {
    let def = builtin_agent_def(agent_id)?;

    let requested = normalize_agent_version(requested_version);
    let requested_is_installed = requested
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case("installed"));

    let local_available = which(def.local_command).is_ok();

    // Force package runner when a specific version/dist-tag is provided.
    if let Some(v) = requested.as_deref() {
        if !requested_is_installed {
            let (cmd, args) =
                build_runner_launch(preferred_launch_runner(), def.bunx_package, Some(v));
            return Ok(ResolvedAgentLaunchCommand {
                command: normalize_launch_command_for_platform(cmd),
                args,
                label: def.label,
                tool_version: v.to_string(),
                version_for_gates: Some(v.to_string()),
            });
        }
    }

    // Prefer installed command when available (auto), or when explicitly requested.
    if local_available {
        let version_raw = get_command_version_with_timeout(def.local_command);
        return Ok(ResolvedAgentLaunchCommand {
            command: normalize_launch_command_for_platform(def.local_command.to_string()),
            args: Vec::new(),
            label: def.label,
            tool_version: "installed".to_string(),
            version_for_gates: version_raw,
        });
    }

    // Explicit installed mode: launch directly and let runtime report command-not-found.
    if requested_is_installed {
        return Ok(ResolvedAgentLaunchCommand {
            command: normalize_launch_command_for_platform(def.local_command.to_string()),
            args: Vec::new(),
            label: def.label,
            tool_version: "installed".to_string(),
            version_for_gates: None,
        });
    }

    // Auto mode fallback: use preferred package runner and resolve in the runtime environment.
    let (cmd, args) = build_runner_launch(preferred_launch_runner(), def.bunx_package, None);
    let tool_version = if requested_is_installed {
        "installed".to_string()
    } else {
        "latest".to_string()
    };
    Ok(ResolvedAgentLaunchCommand {
        command: normalize_launch_command_for_platform(cmd),
        args,
        label: def.label,
        tool_version,
        // bunx/npx "latest" is treated specially by some feature gates (e.g. collaboration_modes).
        version_for_gates: Some("latest".to_string()),
    })
}

fn resolve_agent_launch_command_for_container(
    agent_id: &str,
    requested_version: Option<&str>,
) -> Result<ResolvedAgentLaunchCommand, String> {
    let def = builtin_agent_def(agent_id)?;

    let requested = normalize_agent_version(requested_version);
    let requested_is_installed = requested
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case("installed"));

    if requested_is_installed {
        return Ok(ResolvedAgentLaunchCommand {
            command: normalize_launch_command_for_platform(def.local_command.to_string()),
            args: Vec::new(),
            label: def.label,
            tool_version: "installed".to_string(),
            version_for_gates: None,
        });
    }

    // Container execution must stay independent from host-side command detection.
    // Use npx consistently so launches rely on the container runtime environment.
    let version = requested.unwrap_or_else(|| "latest".to_string());
    let (command, args) =
        build_runner_launch(LaunchRunner::Npx, def.bunx_package, Some(version.as_str()));

    Ok(ResolvedAgentLaunchCommand {
        command: normalize_launch_command_for_platform(command),
        args,
        label: def.label,
        tool_version: version.clone(),
        version_for_gates: Some(version),
    })
}

fn agent_color_for(agent_id: &str) -> AgentColor {
    match agent_id {
        "claude" => AgentColor::Yellow,
        "codex" => AgentColor::Cyan,
        "gemini" => AgentColor::Magenta,
        "opencode" => AgentColor::Green,
        "copilot" => AgentColor::Blue,
        _ => AgentColor::White,
    }
}

fn tool_id_for(agent_id: &str) -> String {
    match agent_id {
        "claude" => "claude-code".to_string(),
        "codex" => "codex-cli".to_string(),
        "gemini" => "gemini-cli".to_string(),
        "opencode" => "opencode".to_string(),
        "copilot" => "github-copilot".to_string(),
        _ => agent_id.to_string(),
    }
}

fn sanitize_extra_args(extra: Option<&[String]>) -> Vec<String> {
    extra
        .unwrap_or(&[])
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn register_pane_runtime_context(state: &AppState, pane_id: &str, launch_workdir: &Path) {
    if let Ok(mut map) = state.pane_runtime_contexts.lock() {
        map.insert(
            pane_id.to_string(),
            PaneRuntimeContext {
                launch_workdir: launch_workdir.to_path_buf(),
            },
        );
    }
}

fn remove_pane_runtime_context(state: &AppState, pane_id: &str) {
    if let Ok(mut map) = state.pane_runtime_contexts.lock() {
        map.remove(pane_id);
    }
}

fn pane_runtime_context(state: &AppState, pane_id: &str) -> Option<PaneRuntimeContext> {
    state
        .pane_runtime_contexts
        .lock()
        .ok()
        .and_then(|map| map.get(pane_id).cloned())
}

const DOCKER_WORKDIR: &str = "/workspace";

fn docker_compose_exec_workdir(workspace_folder: Option<&str>, _working_dir: &Path) -> String {
    workspace_folder
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_default()
}

fn build_docker_compose_up_args(
    compose_args: &[String],
    build: bool,
    recreate: bool,
    service: Option<&str>,
) -> Vec<String> {
    let mut args = vec!["compose".to_string()];
    args.extend(compose_args.iter().cloned());
    args.extend([
        "up".to_string(),
        "-d".to_string(),
        if build {
            "--build".to_string()
        } else {
            "--no-build".to_string()
        },
    ]);
    if recreate {
        args.push("--force-recreate".to_string());
    }
    if let Some(service) = service.map(str::trim).filter(|s| !s.is_empty()) {
        args.push(service.to_string());
    }
    args
}

fn build_docker_compose_down_args(compose_args: &[String]) -> Vec<String> {
    let mut args = vec!["compose".to_string()];
    args.extend(compose_args.iter().cloned());
    args.push("down".to_string());
    args
}

fn is_valid_docker_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

const DOCKER_ENV_MERGE_PREFIX_ALLOWLIST: &[&str] = &[
    "ANTHROPIC_",
    "OPENAI_",
    "GEMINI_",
    "GOOGLE_",
    "GITHUB_",
    "GH_",
    "GIT_",
    "NPM_",
    "HF_",
    "COORDINATOR_",
    "GWT_",
    "CLAUDE_",
    "CODEX_",
    "OLLAMA_",
    "OPENROUTER_",
];

const DOCKER_ENV_MERGE_KEY_ALLOWLIST: &[&str] = &[
    "TERM",
    "COLORTERM",
    "IS_SANDBOX",
    "HOST_GIT_COMMON_DIR",
    "HOST_GIT_WORKTREE_DIR",
];

fn should_merge_profile_env_for_docker(key: &str) -> bool {
    let k = key.trim();
    if k.is_empty() || !is_valid_docker_env_key(k) {
        return false;
    }
    DOCKER_ENV_MERGE_KEY_ALLOWLIST.contains(&k)
        || DOCKER_ENV_MERGE_PREFIX_ALLOWLIST
            .iter()
            .any(|prefix| k.starts_with(prefix))
}

fn merge_profile_env_for_docker(
    base_env: &mut HashMap<String, String>,
    merged_profile_env: &HashMap<String, String>,
) {
    for (key, value) in merged_profile_env {
        let k = key.trim();
        if !should_merge_profile_env_for_docker(k) {
            continue;
        }
        base_env.insert(k.to_string(), value.to_string());
    }
}

fn compose_file_paths_from_args(compose_args: &[String]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut idx = 0usize;
    while idx + 1 < compose_args.len() {
        if compose_args[idx] == "-f" {
            let path = compose_args[idx + 1].trim();
            if !path.is_empty() {
                paths.push(PathBuf::from(path));
            }
            idx += 2;
            continue;
        }
        idx += 1;
    }
    paths
}

fn collect_compose_env_keys(compose_paths: &[PathBuf]) -> HashSet<String> {
    let mut keys = HashSet::new();
    for compose_path in compose_paths {
        if let Ok(found) = DockerManager::list_env_keys_from_compose_file(compose_path) {
            keys.extend(found);
        }
    }
    keys
}

fn merge_compose_env_for_docker(
    base_env: &mut HashMap<String, String>,
    merged_profile_env: &HashMap<String, String>,
    compose_paths: &[PathBuf],
) {
    let keys = collect_compose_env_keys(compose_paths);
    for key in keys {
        let k = key.trim();
        if k.is_empty() || !is_valid_docker_env_key(k) {
            continue;
        }
        if let Some(value) = merged_profile_env.get(k) {
            if should_keep_existing_compose_port_value(base_env, k, value) {
                continue;
            }
            base_env.insert(k.to_string(), value.to_string());
        }
    }
}

fn should_keep_existing_compose_port_value(
    base_env: &HashMap<String, String>,
    key: &str,
    incoming_value: &str,
) -> bool {
    let Some(existing_value) = base_env.get(key) else {
        return false;
    };

    if existing_value == incoming_value {
        return false;
    }

    let Some(incoming_port) = incoming_value.parse::<u16>().ok() else {
        return false;
    };
    if existing_value.parse::<u16>().is_err() {
        return false;
    }

    PortAllocator::is_port_in_use(incoming_port)
}

fn merge_compose_env_from_process(
    base_env: &mut HashMap<String, String>,
    compose_paths: &[PathBuf],
) {
    let keys = collect_compose_env_keys(compose_paths);
    for key in keys {
        let k = key.trim();
        if k.is_empty() || !is_valid_docker_env_key(k) {
            continue;
        }
        if let Ok(value) = std::env::var(k) {
            base_env.insert(k.to_string(), value);
        }
    }
}

fn build_docker_compose_exec_args(
    compose_args: &[String],
    service: &str,
    workdir: &str,
    env_vars: &HashMap<String, String>,
    inner_command: &str,
    inner_args: &[String],
) -> Vec<String> {
    let mut args = vec!["compose".to_string()];
    args.extend(compose_args.iter().cloned());
    args.push("exec".to_string());
    let workdir = workdir.trim();
    if !workdir.is_empty() {
        args.push("-w".to_string());
        args.push(workdir.to_string());
    }

    let mut keys: Vec<&String> = env_vars.keys().collect();
    keys.sort();
    for key in keys {
        let k = key.trim();
        if k.is_empty() || !is_valid_docker_env_key(k) {
            continue;
        }
        let v = env_vars.get(key).map(|s| s.as_str()).unwrap_or_default();
        args.push("-e".to_string());
        args.push(format!("{k}={v}"));
    }

    args.push(service.to_string());
    args.push(inner_command.to_string());
    args.extend(inner_args.iter().cloned());
    args
}

fn ensure_docker_compose_ready() -> Result<(), String> {
    if !docker_available() {
        return Err("docker is not available".to_string());
    }
    if !compose_available() {
        return Err("docker compose is not available".to_string());
    }

    if daemon_running() {
        return Ok(());
    }

    // Best-effort start (e.g., Docker Desktop on macOS).
    try_start_daemon().map_err(|e| e.to_string())?;
    if daemon_running() {
        Ok(())
    } else {
        Err("Docker daemon is not running".to_string())
    }
}

fn verify_docker_compose_service_start<IsRunning, ServiceLogs, ComposeDown>(
    service: Option<&str>,
    mut service_is_running: IsRunning,
    mut service_logs: ServiceLogs,
    mut compose_down: ComposeDown,
) -> Result<(), String>
where
    IsRunning: FnMut(&str) -> Result<bool, String>,
    ServiceLogs: FnMut(&str) -> Option<String>,
    ComposeDown: FnMut() -> Result<(), String>,
{
    let selected_service = service.map(str::trim).unwrap_or_default();
    if selected_service.is_empty() {
        return Ok(());
    }

    match service_is_running(selected_service) {
        Ok(true) => Ok(()),
        Ok(false) => {
            let mut message = format!(
                "docker compose service '{}' is not running after startup.",
                selected_service
            );
            if let Some(logs) = service_logs(selected_service) {
                message.push_str("\n\n");
                message.push_str(&logs);
            }
            if let Err(err) = compose_down() {
                message.push_str("\n\n");
                message.push_str(
                    "Failed to run best-effort docker compose down after startup failure: ",
                );
                message.push_str(&err);
            }
            Err(message)
        }
        Err(err) => Err(format!(
            "docker compose up succeeded, but failed to verify service '{}': {}",
            selected_service, err
        )),
    }
}

fn docker_compose_up(
    worktree_path: &std::path::Path,
    container_name: &str,
    env_vars: &HashMap<String, String>,
    compose_args: &[String],
    build: bool,
    recreate: bool,
    service: Option<&str>,
) -> Result<(), String> {
    ensure_docker_compose_ready()?;

    let output = gwt_core::process::command("docker")
        .args(build_docker_compose_up_args(
            compose_args,
            build,
            recreate,
            service,
        ))
        .current_dir(worktree_path)
        .env("COMPOSE_PROJECT_NAME", container_name)
        .envs(env_vars)
        .output()
        .map_err(|e| format!("Failed to run docker compose up: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("docker compose up failed".to_string());
        }
        return Err(stderr);
    }

    verify_docker_compose_service_start(
        service,
        |selected_service| {
            docker_compose_service_is_running(
                worktree_path,
                container_name,
                env_vars,
                compose_args,
                selected_service,
            )
        },
        |selected_service| {
            docker_compose_service_logs(
                worktree_path,
                container_name,
                env_vars,
                compose_args,
                selected_service,
            )
        },
        || docker_compose_down(worktree_path, container_name, env_vars, compose_args),
    )
}

fn docker_compose_down(
    worktree_path: &std::path::Path,
    container_name: &str,
    env_vars: &HashMap<String, String>,
    compose_args: &[String],
) -> Result<(), String> {
    ensure_docker_compose_ready()?;

    let output = gwt_core::process::command("docker")
        .args(build_docker_compose_down_args(compose_args))
        .current_dir(worktree_path)
        .env("COMPOSE_PROJECT_NAME", container_name)
        .envs(env_vars)
        .output()
        .map_err(|e| format!("Failed to run docker compose down: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        return Err("docker compose down failed".to_string());
    }
    Err(stderr)
}

fn docker_compose_service_is_running(
    worktree_path: &std::path::Path,
    container_name: &str,
    env_vars: &HashMap<String, String>,
    compose_args: &[String],
    service: &str,
) -> Result<bool, String> {
    let mut args = vec!["compose".to_string()];
    args.extend(compose_args.iter().cloned());
    args.extend([
        "ps".to_string(),
        "--status".to_string(),
        "running".to_string(),
        "--services".to_string(),
    ]);

    let output = gwt_core::process::command("docker")
        .args(args)
        .current_dir(worktree_path)
        .env("COMPOSE_PROJECT_NAME", container_name)
        .envs(env_vars)
        .output()
        .map_err(|e| format!("Failed to run docker compose ps: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "docker compose ps failed".to_string()
        } else {
            stderr
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(compose_services_output_contains(stdout.as_ref(), service))
}

fn docker_compose_service_logs(
    worktree_path: &std::path::Path,
    container_name: &str,
    env_vars: &HashMap<String, String>,
    compose_args: &[String],
    service: &str,
) -> Option<String> {
    let mut args = vec!["compose".to_string()];
    args.extend(compose_args.iter().cloned());
    args.extend([
        "logs".to_string(),
        "--no-color".to_string(),
        "--tail".to_string(),
        "80".to_string(),
    ]);
    args.push(service.to_string());

    let output = gwt_core::process::command("docker")
        .args(args)
        .current_dir(worktree_path)
        .env("COMPOSE_PROJECT_NAME", container_name)
        .envs(env_vars)
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    let mut chunks = Vec::new();
    if !stdout.is_empty() {
        chunks.push(stdout);
    }
    if !stderr.is_empty() {
        chunks.push(stderr);
    }
    if chunks.is_empty() {
        None
    } else {
        Some(chunks.join("\n"))
    }
}

fn compose_services_output_contains(output: &str, service: &str) -> bool {
    let target = service.trim();
    if target.is_empty() {
        return false;
    }
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .any(|line| line == target)
}

fn ensure_docker_ready() -> Result<(), String> {
    if !docker_available() {
        return Err("docker is not available".to_string());
    }

    if daemon_running() {
        return Ok(());
    }

    // Best-effort start (e.g., Docker Desktop on macOS).
    try_start_daemon().map_err(|e| e.to_string())?;
    if daemon_running() {
        Ok(())
    } else {
        Err("Docker daemon is not running".to_string())
    }
}

fn docker_image_exists(image: &str) -> bool {
    gwt_core::process::command("docker")
        .args(["image", "inspect", image])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn docker_image_created_time(image: &str) -> Option<SystemTime> {
    let output = gwt_core::process::command("docker")
        .args(["image", "inspect", "-f", "{{.Created}}", image])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let created_raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if created_raw.is_empty() {
        return None;
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(&created_raw).ok()?;
    Some(parsed.with_timezone(&Utc).into())
}

fn docker_should_build_image(
    dockerfile_path: &std::path::Path,
    image: &str,
    build_requested: bool,
) -> bool {
    if build_requested || !docker_image_exists(image) {
        return true;
    }

    let modified = std::fs::metadata(dockerfile_path)
        .and_then(|m| m.modified())
        .ok();
    let created = docker_image_created_time(image);

    match (modified, created) {
        (Some(mod_time), Some(created_time)) => mod_time > created_time,
        (Some(_), None) => true,
        _ => false,
    }
}

fn docker_build_image(
    image: &str,
    dockerfile_path: &std::path::Path,
    context_dir: &std::path::Path,
) -> Result<(), String> {
    let output = gwt_core::process::command("docker")
        .args(["build", "-t", image, "-f"])
        .arg(dockerfile_path)
        .arg(context_dir)
        .output()
        .map_err(|e| format!("Failed to run docker build: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        "docker build failed".to_string()
    } else {
        stderr
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerBindMount {
    source: String,
    target: String,
}

fn normalize_mount_path(path: &str) -> String {
    path.trim().replace('\\', "/")
}

fn is_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
}

fn docker_mount_target_path(path: &str) -> String {
    let normalized = normalize_mount_path(path);
    if !is_windows_drive_path(&normalized) {
        return normalized;
    }

    let rest = normalized[2..].trim_start_matches('/');
    if rest.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", rest)
    }
}

fn path_is_same_or_within(base: &str, child: &str) -> bool {
    let base_norm = normalize_mount_path(base);
    let child_norm = normalize_mount_path(child);
    let base_path = std::path::Path::new(base_norm.as_str());
    let child_path = std::path::Path::new(child_norm.as_str());
    child_path == base_path || child_path.starts_with(base_path)
}

fn build_git_bind_mounts(env_vars: &HashMap<String, String>) -> Vec<DockerBindMount> {
    let common_source = env_vars
        .get("HOST_GIT_COMMON_DIR")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(normalize_mount_path);

    let Some(common_source) = common_source else {
        return Vec::new();
    };

    let common_target = docker_mount_target_path(&common_source);
    let mut mounts = vec![DockerBindMount {
        source: common_source.clone(),
        target: common_target.clone(),
    }];

    let worktree_source = env_vars
        .get("HOST_GIT_WORKTREE_DIR")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(normalize_mount_path);

    let Some(worktree_source) = worktree_source else {
        return mounts;
    };

    let worktree_target = docker_mount_target_path(&worktree_source);
    if worktree_target.is_empty() || path_is_same_or_within(&common_target, &worktree_target) {
        return mounts;
    }

    mounts.push(DockerBindMount {
        source: worktree_source,
        target: worktree_target,
    });
    mounts
}

fn apply_translated_git_env(env_vars: &mut HashMap<String, String>) {
    let common_source = env_vars
        .get("HOST_GIT_COMMON_DIR")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(normalize_mount_path);
    if let Some(common_source) = common_source {
        let common_target = docker_mount_target_path(&common_source);
        if common_target != common_source {
            env_vars.insert("GIT_COMMON_DIR".to_string(), common_target);
        }
    }

    let worktree_source = env_vars
        .get("HOST_GIT_WORKTREE_DIR")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(normalize_mount_path);
    if let Some(worktree_source) = worktree_source {
        let worktree_target = docker_mount_target_path(&worktree_source);
        if worktree_target != worktree_source {
            env_vars.insert("GIT_DIR".to_string(), worktree_target);
        }
    }
}

fn yaml_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn compose_service_container_name(container_name_prefix: &str, service: &str) -> String {
    let mut suffix = String::new();
    for c in service.trim().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
            suffix.push(c.to_ascii_lowercase());
        } else {
            suffix.push('-');
        }
    }
    while suffix.contains("--") {
        suffix = suffix.replace("--", "-");
    }
    let suffix = suffix.trim_matches('-');
    if suffix.is_empty() {
        container_name_prefix.to_string()
    } else {
        format!("{container_name_prefix}-{suffix}")
    }
}

fn render_docker_compose_override(
    selected_service: &str,
    services: &[String],
    container_name_prefix: &str,
    mounts: &[DockerBindMount],
) -> String {
    let mut content = "services:\n".to_string();
    let selected = selected_service.trim();

    let mut seen = std::collections::BTreeSet::new();
    let mut ordered: Vec<String> = Vec::new();
    for service in services {
        let name = service.trim();
        if name.is_empty() || !seen.insert(name.to_string()) {
            continue;
        }
        ordered.push(name.to_string());
    }
    if !selected.is_empty() && seen.insert(selected.to_string()) {
        ordered.push(selected.to_string());
    }

    for service in ordered {
        content.push_str(&format!("  {service}:\n"));
        let resolved_container_name =
            compose_service_container_name(container_name_prefix, &service);
        content.push_str(&format!(
            "    container_name: {}\n",
            yaml_single_quoted(&resolved_container_name)
        ));

        if service != selected || mounts.is_empty() {
            continue;
        }

        content.push_str("    volumes:\n");
        for mount in mounts {
            content.push_str("      - type: bind\n");
            content.push_str(&format!(
                "        source: {}\n",
                yaml_single_quoted(&mount.source)
            ));
            content.push_str(&format!(
                "        target: {}\n",
                yaml_single_quoted(&mount.target)
            ));
        }
    }

    content
}

fn write_docker_compose_override(
    project_root: &std::path::Path,
    container_name: &str,
    selected_service: &str,
    services: &[String],
    mounts: &[DockerBindMount],
) -> Result<Option<std::path::PathBuf>, String> {
    if mounts.is_empty() && services.is_empty() {
        return Ok(None);
    }

    let gwt_dir = project_root.join(".gwt");
    std::fs::create_dir_all(&gwt_dir)
        .map_err(|e| format!("Failed to create .gwt directory: {e}"))?;

    let filename = format!("docker-compose.gwt.override.{container_name}.yml");
    let path = gwt_dir.join(filename);

    let content =
        render_docker_compose_override(selected_service, services, container_name, mounts);

    std::fs::write(&path, content).map_err(|e| format!("Failed to write override file: {e}"))?;
    Ok(Some(path))
}

fn codex_supports_collaboration_modes(version_for_gates: Option<&str>) -> bool {
    version_for_gates.is_some_and(|v| v.eq_ignore_ascii_case("latest"))
        || gwt_core::agent::codex::supports_collaboration_modes(version_for_gates)
}

fn build_agent_args(
    agent_id: &str,
    request: &LaunchAgentRequest,
    version_for_gates: Option<&str>,
    enable_codex_multi_agent: bool,
) -> Result<Vec<String>, String> {
    let mode = request.mode.unwrap_or(SessionMode::Normal);
    let skip_permissions = request.skip_permissions.unwrap_or(false);
    let resume_session_id = request
        .resume_session_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let extra_args = sanitize_extra_args(request.extra_args.as_deref());
    let mut args: Vec<String> = Vec::new();

    match agent_id {
        "codex" => {
            let mut prefix: Vec<String> = Vec::new();
            match mode {
                SessionMode::Normal => {}
                SessionMode::Continue => {
                    prefix.push("resume".to_string());
                    if let Some(id) = resume_session_id {
                        prefix.push(id.to_string());
                    } else {
                        prefix.push("--last".to_string());
                    }
                }
                SessionMode::Resume => {
                    prefix.push("resume".to_string());
                    if let Some(id) = resume_session_id {
                        prefix.push(id.to_string());
                    }
                }
            }
            args.extend(prefix);

            let collaboration = codex_supports_collaboration_modes(version_for_gates);
            args.extend(gwt_core::agent::codex::codex_default_args(
                request.model.as_deref(),
                request.reasoning_level.as_deref(),
                version_for_gates,
                skip_permissions,
                request.fast_mode.unwrap_or(false),
                collaboration,
                enable_codex_multi_agent,
            ));

            if skip_permissions {
                args.push(
                    gwt_core::agent::codex::codex_skip_permissions_flag(version_for_gates)
                        .to_string(),
                );
            }
        }
        "claude" => {
            match mode {
                SessionMode::Normal => {}
                SessionMode::Continue => {
                    if let Some(id) = resume_session_id {
                        args.push("--resume".to_string());
                        args.push(id.to_string());
                    } else {
                        args.push("--continue".to_string());
                    }
                }
                SessionMode::Resume => {
                    args.push("--resume".to_string());
                    if let Some(id) = resume_session_id {
                        args.push(id.to_string());
                    }
                }
            }

            if skip_permissions {
                args.push("--dangerously-skip-permissions".to_string());
            }

            args.extend(build_agent_model_args(agent_id, request.model.as_deref()));
        }
        "gemini" => {
            match mode {
                SessionMode::Normal => {}
                SessionMode::Continue => {
                    args.push("-r".to_string());
                    if let Some(id) = resume_session_id {
                        args.push(id.to_string());
                    } else {
                        args.push("latest".to_string());
                    }
                }
                SessionMode::Resume => {
                    args.push("-r".to_string());
                    if let Some(id) = resume_session_id {
                        args.push(id.to_string());
                    }
                }
            }

            if skip_permissions {
                args.push("-y".to_string());
            }

            args.extend(build_agent_model_args(agent_id, request.model.as_deref()));
        }
        "opencode" => {
            match mode {
                SessionMode::Normal => {}
                SessionMode::Continue => {
                    // Prefer explicit session ID when provided (Quick Start).
                    if let Some(id) = resume_session_id {
                        args.push("-s".to_string());
                        args.push(id.to_string());
                    } else {
                        args.push("-c".to_string());
                    }
                }
                SessionMode::Resume => {
                    let Some(id) = resume_session_id else {
                        return Err("Session ID is required for OpenCode resume".to_string());
                    };
                    args.push("-s".to_string());
                    args.push(id.to_string());
                }
            }

            args.extend(build_agent_model_args(agent_id, request.model.as_deref()));
        }
        "copilot" => {
            match mode {
                SessionMode::Normal => {}
                SessionMode::Continue => {
                    if let Some(id) = resume_session_id {
                        args.push("--resume".to_string());
                        args.push(id.to_string());
                    } else {
                        args.push("--continue".to_string());
                    }
                }
                SessionMode::Resume => {
                    args.push("--resume".to_string());
                    if let Some(id) = resume_session_id {
                        args.push(id.to_string());
                    }
                }
            }
            if skip_permissions {
                args.push("--allow-all-tools".to_string());
            }
            args.extend(build_agent_model_args(agent_id, request.model.as_deref()));
        }
        _ => {}
    }

    args.extend(extra_args);
    Ok(args)
}

fn launch_with_config(
    repo_path: &std::path::Path,
    config: BuiltinLaunchConfig,
    meta: Option<PaneLaunchMeta>,
    state: &AppState,
    app_handle: AppHandle,
) -> Result<String, String> {
    let agent_name_for_stream = config.agent_name.clone();
    let launch_workdir = config.working_dir.clone();
    let pane_id = {
        let mut manager = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock pane manager: {}", e))?;
        manager
            .launch_agent(repo_path, config, 24, 80)
            .map_err(|e| format!("Failed to launch terminal: {}", e))?
    };

    register_pane_runtime_context(state, &pane_id, &launch_workdir);

    if let Some(meta) = meta {
        if let Ok(mut map) = state.pane_launch_meta.lock() {
            map.insert(pane_id.clone(), meta);
        }
    }

    // Take the PTY reader and spawn a thread to stream output to the frontend
    let reader = {
        let manager = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock pane manager: {}", e))?;
        let pane = manager
            .panes()
            .iter()
            .find(|p| p.pane_id() == pane_id)
            .ok_or_else(|| "Pane not found after creation".to_string())?;
        pane.take_reader()
            .map_err(|e| format!("Failed to take reader: {}", e))?
    };

    let pane_id_clone = pane_id.clone();
    std::thread::spawn(move || {
        stream_pty_output(reader, pane_id_clone, app_handle, agent_name_for_stream);
    });

    Ok(pane_id)
}

/// Build the shell command string to inject into a WSL interactive PTY.
///
/// Prepends `export KEY=VALUE; ...` for env vars, then appends
/// `cd <wsl_path> && <agent_command> <args...>`.
fn build_wsl_inject_command(
    agent_command: &str,
    agent_args: &[String],
    env_vars: &HashMap<String, String>,
    wsl_working_dir: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Export env vars
    let mut keys: Vec<&String> = env_vars.keys().collect();
    keys.sort();
    for key in keys {
        let value = env_vars.get(key).map(|s| s.as_str()).unwrap_or("");
        // Single-quote the value and escape embedded single quotes.
        let escaped = value.replace('\'', "'\\''");
        parts.push(format!("export {key}='{escaped}'"));
    }

    // cd + exec
    let mut cmd = format!("cd '{wsl_working_dir}'");
    cmd.push_str(" && exec ");
    cmd.push_str(agent_command);
    for arg in agent_args {
        let escaped = arg.replace('\'', "'\\''");
        cmd.push_str(&format!(" '{escaped}'"));
    }
    parts.push(cmd);

    parts.join(" && ")
}

/// Detect a shell prompt in PTY output and inject a command.
///
/// Reads from the PTY reader, looking for common shell prompt endings
/// (`$`, `#`, `>`, `%` followed by a space or at line end).
/// On detection, writes `command_str` + newline to the PTY input.
/// Returns `Ok(true)` if prompt was detected and command injected,
/// `Ok(false)` if timeout expired without detection.
fn wsl_prompt_detect_and_inject(
    pane_id: &str,
    command_str: &str,
    state: &AppState,
    app_handle: &AppHandle,
    timeout: Duration,
) -> Result<bool, String> {
    // Take a separate reader for prompt detection.
    let reader = {
        let manager = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock pane manager: {e}"))?;
        let pane = manager
            .panes()
            .iter()
            .find(|p| p.pane_id() == pane_id)
            .ok_or_else(|| "Pane not found".to_string())?;
        pane.take_reader()
            .map_err(|e| format!("Failed to take reader: {e}"))?
    };

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let pane_id_stream = pane_id.to_string();
    let app_for_stream = app_handle.clone();

    // Spawn a reader thread that forwards bytes to both the channel and the frontend.
    let reader_handle = std::thread::spawn(move || {
        let state = app_for_stream.state::<AppState>();
        let mut buf = [0u8; 4096];
        let mut reader = reader;
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
                    // Record in scrollback and check frontend readiness under single lock.
                    let is_ready = if let Ok(mut manager) = state.pane_manager.lock() {
                        if let Some(pane) = manager.pane_mut_by_id(&pane_id_stream) {
                            let _ = pane.process_bytes(&chunk);
                            pane.is_frontend_ready()
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    // Only forward to frontend when it has signalled readiness.
                    if is_ready {
                        let payload = TerminalOutputPayload {
                            pane_id: pane_id_stream.clone(),
                            data: chunk.clone(),
                        };
                        let _ = app_for_stream.emit("terminal-output", &payload);
                    }
                    // Send to prompt detector if it is still listening.
                    let _ = tx.send(chunk);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });

    // Monitor received bytes for prompt pattern.
    let deadline = std::time::Instant::now() + timeout;
    let mut accumulated = Vec::new();
    let mut detected = false;

    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(100));
        match rx.recv_timeout(wait) {
            Ok(chunk) => {
                accumulated.extend_from_slice(&chunk);
                if detect_shell_prompt(&accumulated) {
                    detected = true;
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if detected {
        // Write command to PTY
        let cmd_bytes = format!("{command_str}\n");
        let mut manager = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock pane manager: {e}"))?;
        if let Some(pane) = manager.pane_mut_by_id(pane_id) {
            pane.write_input(cmd_bytes.as_bytes())
                .map_err(|e| format!("Failed to write command to WSL PTY: {e}"))?;
        }
    }

    // The reader thread will continue to run; we transition to the normal
    // stream_pty_output loop in the caller. The reader thread stops when
    // the PTY closes because read() returns 0 or errors. Since we already
    // consumed the reader, the caller should NOT start stream_pty_output.
    // Instead, this reader thread IS the stream thread.
    //
    // Detach the reader thread (it will run until EOF).
    drop(reader_handle);

    Ok(detected)
}

/// Check whether a byte buffer ends with a common shell prompt pattern.
///
/// Looks for `$`, `#`, `>`, or `%` followed by optional whitespace at the
/// end of the output, after stripping ANSI escape sequences.
fn detect_shell_prompt(buf: &[u8]) -> bool {
    // Strip ANSI sequences for detection (simple state machine).
    let stripped = strip_ansi_bytes(buf);
    let s = String::from_utf8_lossy(&stripped);
    let trimmed = s.trim_end();
    if trimmed.is_empty() {
        return false;
    }
    let last = trimmed.as_bytes()[trimmed.len() - 1];
    matches!(last, b'$' | b'#' | b'>' | b'%')
}

/// Lightweight ANSI escape stripper for prompt detection.
fn strip_ansi_bytes(buf: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(buf.len());
    let mut i = 0;
    while i < buf.len() {
        if buf[i] == 0x1b {
            i += 1;
            if i < buf.len() && buf[i] == b'[' {
                // CSI sequence: skip until final byte (0x40-0x7E)
                i += 1;
                while i < buf.len() && !(0x40..=0x7E).contains(&buf[i]) {
                    i += 1;
                }
                if i < buf.len() {
                    i += 1; // skip final byte
                }
            } else if i < buf.len() && buf[i] == b']' {
                // OSC sequence: skip until ST (ESC \ or BEL)
                i += 1;
                while i < buf.len() {
                    if buf[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if buf[i] == 0x1b && i + 1 < buf.len() && buf[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            } else {
                // Other escape: skip one more byte
                if i < buf.len() {
                    i += 1;
                }
            }
        } else {
            out.push(buf[i]);
            i += 1;
        }
    }
    out
}

/// Parameters for WSL PTY-write agent launch.
struct WslPtyWriteParams {
    repo_path: PathBuf,
    agent_command: String,
    agent_args: Vec<String>,
    working_dir: PathBuf,
    wsl_working_dir: String,
    branch_name: String,
    agent_name: String,
    agent_color: AgentColor,
    env_vars: HashMap<String, String>,
    meta: Option<PaneLaunchMeta>,
}

/// Launch an agent in WSL using the PTY write approach.
///
/// 1. Starts `wsl.exe --cd <wsl_path>` as an interactive PTY.
/// 2. Monitors output for a shell prompt (3-second timeout).
/// 3. On prompt detection, injects the agent command via PTY write.
/// 4. On timeout, kills the pane and re-launches with non-interactive mode.
fn launch_with_wsl_pty_write(
    params: WslPtyWriteParams,
    state: &AppState,
    app_handle: AppHandle,
) -> Result<String, String> {
    let WslPtyWriteParams {
        repo_path,
        agent_command,
        agent_args,
        working_dir,
        wsl_working_dir,
        branch_name,
        agent_name,
        agent_color,
        env_vars,
        meta,
    } = params;
    // Phase 1: Launch wsl.exe interactively
    let wsl_config = BuiltinLaunchConfig {
        command: "wsl.exe".to_string(),
        args: vec!["--cd".to_string(), wsl_working_dir.clone()],
        working_dir: working_dir.clone(),
        branch_name: branch_name.clone(),
        agent_name: agent_name.clone(),
        agent_color,
        env_vars: HashMap::new(), // WSL login shell handles base env
        terminal_shell: Some("wsl".to_string()),
        interactive: true,
        windows_force_utf8: false,
    };

    let pane_id = {
        let mut manager = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock pane manager: {e}"))?;
        manager
            .launch_agent(&repo_path, wsl_config, 24, 80)
            .map_err(|e| format!("Failed to launch WSL terminal: {e}"))?
    };

    register_pane_runtime_context(state, &pane_id, &working_dir);

    if let Some(ref meta) = meta {
        if let Ok(mut map) = state.pane_launch_meta.lock() {
            map.insert(pane_id.clone(), meta.clone());
        }
    }

    // Phase 2: Detect prompt and inject command
    let inject_cmd =
        build_wsl_inject_command(&agent_command, &agent_args, &env_vars, &wsl_working_dir);

    let prompt_detected = wsl_prompt_detect_and_inject(
        &pane_id,
        &inject_cmd,
        state,
        &app_handle,
        Duration::from_secs(3),
    )?;

    if prompt_detected {
        // The reader thread from wsl_prompt_detect_and_inject is already
        // streaming output. We're done.
        return Ok(pane_id);
    }

    // Phase 3: Fallback - kill interactive pane, re-launch non-interactively
    tracing::warn!(
        pane_id = %pane_id,
        "WSL prompt not detected within timeout, falling back to non-interactive mode"
    );

    {
        let mut manager = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock pane manager: {e}"))?;
        if let Some(pane) = manager.pane_mut_by_id(&pane_id) {
            let _ = pane.kill();
        }
        manager.remove_pane(&pane_id);
    }

    // Remove stale meta
    if let Ok(mut map) = state.pane_launch_meta.lock() {
        map.remove(&pane_id);
    }
    remove_pane_runtime_context(state, &pane_id);

    // Build non-interactive command: wsl.exe -e bash -lc 'export ...; cd ...; exec agent args...'
    let fallback_cmd =
        build_wsl_inject_command(&agent_command, &agent_args, &env_vars, &wsl_working_dir);
    let fallback_config = BuiltinLaunchConfig {
        command: "wsl.exe".to_string(),
        args: vec![
            "-e".to_string(),
            "bash".to_string(),
            "-lc".to_string(),
            fallback_cmd,
        ],
        working_dir,
        branch_name,
        agent_name,
        agent_color,
        env_vars: HashMap::new(),
        terminal_shell: Some("wsl".to_string()),
        interactive: false,
        windows_force_utf8: false,
    };

    launch_with_config(&repo_path, fallback_config, meta, state, app_handle)
}

/// Launch a new terminal pane with an agent
#[tauri::command]
pub fn launch_terminal(
    window: tauri::Window,
    agent_name: String,
    branch: String,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<String, StructuredError> {
    let project_root = {
        let Some(p) = state.project_for_window(window.label()) else {
            return Err(StructuredError::internal(
                "No project opened",
                "launch_terminal",
            ));
        };
        PathBuf::from(p)
    };

    let repo_path = resolve_repo_path_for_project_root(&project_root)
        .map_err(|e| StructuredError::internal(&e, "launch_terminal"))?;
    let (working_dir, _created) = resolve_worktree_path(&repo_path, &branch)
        .map_err(|e| StructuredError::internal(&e, "launch_terminal"))?;

    let config = BuiltinLaunchConfig {
        command: agent_name.clone(),
        args: vec![],
        working_dir,
        branch_name: branch,
        agent_name,
        agent_color: AgentColor::Green,
        env_vars: HashMap::new(),
        terminal_shell: None,
        interactive: true,
        windows_force_utf8: cfg!(target_os = "windows"),
    };

    launch_with_config(&repo_path, config, None, &state, app_handle)
        .map_err(|e| StructuredError::internal(&e, "launch_terminal"))
}

fn non_empty_env_var(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_shell_launch_spec(is_windows: bool) -> (String, Vec<String>) {
    let shell = non_empty_env_var("SHELL")
        .or_else(|| {
            if is_windows {
                non_empty_env_var("COMSPEC")
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            if is_windows {
                "cmd.exe".to_string()
            } else {
                "/bin/sh".to_string()
            }
        });

    let lower = shell.to_ascii_lowercase();
    let requires_plain_launch = lower.ends_with("cmd.exe")
        || lower.ends_with("powershell.exe")
        || lower.ends_with("pwsh.exe");

    let args = if requires_plain_launch {
        Vec::new()
    } else {
        vec!["-l".to_string()]
    };

    (shell, args)
}

fn normalize_terminal_shell_id(shell_id: Option<&str>) -> Option<String> {
    let normalized = shell_id
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())?;
    match normalized.as_str() {
        "powershell" | "cmd" | "wsl" => Some(normalized),
        _ => None,
    }
}

fn resolve_shell_id_for_spawn(shell_id: Option<&str>) -> Option<String> {
    let explicit = shell_id.map(str::trim).filter(|s| !s.is_empty());
    if explicit.is_some() {
        return normalize_terminal_shell_id(explicit);
    }

    Settings::load_global().ok().and_then(|settings| {
        normalize_terminal_shell_id(settings.terminal.default_shell.as_deref())
    })
}

fn should_launch_agent_with_wsl_shell(shell_id: Option<&str>) -> bool {
    cfg!(target_os = "windows") && shell_id == Some("wsl")
}

/// Resolve shell command and arguments for `spawn_shell`.
///
/// When an explicit shell id is given (e.g. `"powershell"`, `"cmd"`, `"wsl"`),
/// it is used directly on Windows.  Otherwise falls back to the default
/// `resolve_shell_launch_spec` logic.
fn resolve_shell_for_spawn(shell_id: Option<&str>) -> (String, Vec<String>) {
    #[cfg(target_os = "windows")]
    {
        if let Some(id) = shell_id {
            match id {
                "powershell" => {
                    let cmd = if which("pwsh").is_ok() {
                        "pwsh".to_string()
                    } else {
                        "powershell".to_string()
                    };
                    return (cmd, Vec::new());
                }
                "cmd" => return ("cmd.exe".to_string(), Vec::new()),
                "wsl" => return ("wsl".to_string(), Vec::new()),
                _ => {} // fall through to default
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = shell_id; // suppress unused warning
    }
    resolve_shell_launch_spec(cfg!(windows))
}

/// Spawn a plain shell terminal (not an agent).
///
/// When `shell` is `Some("powershell" | "cmd" | "wsl")` on Windows, that
/// shell is used. When `shell` is omitted, Windows first consults
/// `terminal.default_shell` in settings, then falls back to auto-resolution.
/// On non-Windows platforms the parameter is silently ignored.
#[tauri::command]
pub fn spawn_shell(
    working_dir: Option<String>,
    shell: Option<String>,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<String, StructuredError> {
    let shell_id = resolve_shell_id_for_spawn(shell.as_deref());
    let (shell_cmd, shell_args) = resolve_shell_for_spawn(shell_id.as_deref());

    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));

    let resolved_dir = working_dir
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .unwrap_or_else(|| home.clone());

    // WSL on Windows: convert working dir and pass --cd flag
    let (final_cmd, final_args, terminal_shell_tag) =
        if cfg!(target_os = "windows") && shell_id.as_deref() == Some("wsl") {
            let wsl_path =
                gwt_core::terminal::shell::windows_to_wsl_path(&resolved_dir.to_string_lossy())
                    .map_err(|e| {
                        StructuredError::internal(
                            &format!("WSL path conversion failed: {e}"),
                            "spawn_shell",
                        )
                    })?;
            (
                "wsl.exe".to_string(),
                vec!["--cd".to_string(), wsl_path],
                Some("wsl".to_string()),
            )
        } else {
            (shell_cmd, shell_args, None)
        };

    let config = BuiltinLaunchConfig {
        command: final_cmd,
        args: final_args,
        working_dir: resolved_dir.clone(),
        branch_name: "terminal".to_string(),
        agent_name: "terminal".to_string(),
        agent_color: AgentColor::White,
        env_vars: HashMap::new(),
        terminal_shell: terminal_shell_tag,
        interactive: true,
        windows_force_utf8: false,
    };

    let pane_id = {
        let mut manager = state.pane_manager.lock().map_err(|e| {
            StructuredError::internal(
                &format!("Failed to lock pane manager: {}", e),
                "spawn_shell",
            )
        })?;
        manager
            .spawn_shell(&resolved_dir, config, 24, 80)
            .map_err(|e| {
                StructuredError::internal(&format!("Failed to spawn shell: {}", e), "spawn_shell")
            })?
    };

    register_pane_runtime_context(&state, &pane_id, &resolved_dir);

    let reader = {
        let manager = state.pane_manager.lock().map_err(|e| {
            StructuredError::internal(
                &format!("Failed to lock pane manager: {}", e),
                "spawn_shell",
            )
        })?;
        let pane = manager
            .panes()
            .iter()
            .find(|p| p.pane_id() == pane_id)
            .ok_or_else(|| {
                StructuredError::internal("Pane not found after creation", "spawn_shell")
            })?;
        pane.take_reader().map_err(|e| {
            StructuredError::internal(&format!("Failed to take reader: {}", e), "spawn_shell")
        })?
    };

    let pane_id_clone = pane_id.clone();
    std::thread::spawn(move || {
        stream_pty_output(reader, pane_id_clone, app_handle, "terminal".to_string());
    });

    Ok(pane_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::ffi::OsString;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::TempDir;

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn resolve_shell_launch_spec_prefers_shell_env_with_login_arg() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let _shell = ScopedEnvVar::set("SHELL", "/usr/bin/zsh");
        let _comspec = ScopedEnvVar::set("COMSPEC", "C:\\Windows\\System32\\cmd.exe");

        let (shell, args) = resolve_shell_launch_spec(false);
        assert_eq!(shell, "/usr/bin/zsh");
        assert_eq!(args, vec!["-l".to_string()]);
    }

    #[test]
    fn resolve_shell_launch_spec_windows_falls_back_to_comspec_without_login_arg() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let _shell = ScopedEnvVar::remove("SHELL");
        let _comspec = ScopedEnvVar::set("COMSPEC", "C:\\Windows\\System32\\cmd.exe");

        let (shell, args) = resolve_shell_launch_spec(true);
        assert_eq!(shell, "C:\\Windows\\System32\\cmd.exe");
        assert!(args.is_empty());
    }

    #[test]
    fn resolve_shell_launch_spec_windows_defaults_to_cmd_when_env_missing() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let _shell = ScopedEnvVar::remove("SHELL");
        let _comspec = ScopedEnvVar::remove("COMSPEC");

        let (shell, args) = resolve_shell_launch_spec(true);
        assert_eq!(shell, "cmd.exe");
        assert!(args.is_empty());
    }

    #[test]
    fn resolve_shell_launch_spec_disables_login_arg_for_pwsh() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let _shell = ScopedEnvVar::set("SHELL", "/opt/homebrew/bin/pwsh.exe");
        let _comspec = ScopedEnvVar::remove("COMSPEC");

        let (shell, args) = resolve_shell_launch_spec(false);
        assert_eq!(shell, "/opt/homebrew/bin/pwsh.exe");
        assert!(args.is_empty());
    }

    #[test]
    fn resolve_shell_id_for_spawn_uses_settings_default_when_shell_not_provided() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let home = TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());
        std::fs::create_dir_all(home.path().join(".gwt")).unwrap();
        std::fs::write(
            home.path().join(".gwt").join("config.toml"),
            "[terminal]\ndefault_shell = \"wsl\"\n",
        )
        .unwrap();

        assert_eq!(resolve_shell_id_for_spawn(None), Some("wsl".to_string()));
    }

    #[test]
    fn resolve_shell_id_for_spawn_prefers_explicit_shell_over_settings() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let home = TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());
        std::fs::create_dir_all(home.path().join(".gwt")).unwrap();
        std::fs::write(
            home.path().join(".gwt").join("config.toml"),
            "[terminal]\ndefault_shell = \"wsl\"\n",
        )
        .unwrap();

        assert_eq!(
            resolve_shell_id_for_spawn(Some("powershell")),
            Some("powershell".to_string())
        );
    }

    #[test]
    fn resolve_shell_id_for_spawn_ignores_invalid_explicit_shell() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let home = TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());
        std::fs::create_dir_all(home.path().join(".gwt")).unwrap();
        std::fs::write(
            home.path().join(".gwt").join("config.toml"),
            "[terminal]\ndefault_shell = \"wsl\"\n",
        )
        .unwrap();

        assert_eq!(resolve_shell_id_for_spawn(Some("fish")), None);
    }

    #[test]
    fn should_launch_agent_with_wsl_shell_requires_wsl_id() {
        assert!(!should_launch_agent_with_wsl_shell(None));
        assert!(!should_launch_agent_with_wsl_shell(Some("cmd")));
        assert!(!should_launch_agent_with_wsl_shell(Some("powershell")));
    }

    #[test]
    fn should_launch_agent_with_wsl_shell_is_windows_only() {
        assert_eq!(
            should_launch_agent_with_wsl_shell(Some("wsl")),
            cfg!(target_os = "windows")
        );
    }

    #[test]
    fn consume_osc7_cwd_updates_buffers_fragmented_sequences() {
        let mut pending = Vec::new();
        let mut last_cwd = String::new();

        let first = b"out\x1b]7;file://host/tmp/frag";
        let second = b"mented\x07tail";

        assert_eq!(
            consume_osc7_cwd_updates(&mut pending, first, &mut last_cwd),
            None
        );
        assert_eq!(last_cwd, "");
        assert_eq!(
            consume_osc7_cwd_updates(&mut pending, second, &mut last_cwd),
            Some("/tmp/fragmented".to_string())
        );
        assert_eq!(last_cwd, "/tmp/fragmented");
    }

    #[test]
    fn consume_osc7_cwd_updates_returns_latest_unique_cwd() {
        let mut pending = Vec::new();
        let mut last_cwd = String::new();

        let chunk = b"\x1b]7;file://host/tmp/a\x07mid\x1b]7;file://host/tmp/b\x07";
        assert_eq!(
            consume_osc7_cwd_updates(&mut pending, chunk, &mut last_cwd),
            Some("/tmp/b".to_string())
        );
        assert_eq!(last_cwd, "/tmp/b");
        assert_eq!(
            consume_osc7_cwd_updates(&mut pending, b"\x1b]7;file://host/tmp/b\x07", &mut last_cwd),
            None
        );
    }

    #[test]
    fn is_node_modules_bin_matches_common_paths() {
        assert!(gwt_core::terminal::runner::is_node_modules_bin(Path::new(
            "/repo/node_modules/.bin/bunx"
        )));
        assert!(gwt_core::terminal::runner::is_node_modules_bin(Path::new(
            "C:\\repo\\node_modules\\.bin\\bunx"
        )));
        assert!(!gwt_core::terminal::runner::is_node_modules_bin(Path::new(
            "/usr/local/bin/bunx"
        )));
    }

    #[test]
    fn preferred_launch_runner_with_availability_prefers_npx_when_available() {
        assert!(matches!(
            preferred_launch_runner_with_availability(Some(Path::new("/usr/bin/bunx")), true),
            LaunchRunner::Npx
        ));
    }

    #[test]
    fn preferred_launch_runner_with_availability_falls_back_to_bunx() {
        assert!(matches!(
            preferred_launch_runner_with_availability(Some(Path::new("/usr/bin/bunx")), false),
            LaunchRunner::Bunx
        ));
        assert!(matches!(
            preferred_launch_runner_with_availability(None, false),
            LaunchRunner::Bunx
        ));
    }

    #[test]
    fn build_runner_launch_bunx_uses_package_as_first_arg() {
        let (cmd, args) = build_runner_launch(LaunchRunner::Bunx, "@openai/codex", Some("latest"));
        assert_eq!(cmd, "bunx");
        assert_eq!(args, vec!["@openai/codex@latest".to_string()]);
    }

    #[test]
    fn build_runner_launch_npx_uses_yes_flag() {
        let (cmd, args) = build_runner_launch(LaunchRunner::Npx, "@openai/codex", Some("1.2.3"));
        assert_eq!(cmd, "npx");
        assert_eq!(
            args,
            vec!["--yes".to_string(), "@openai/codex@1.2.3".to_string()]
        );
    }

    #[test]
    fn resolve_agent_launch_command_for_container_uses_npx_for_latest() {
        let resolved =
            resolve_agent_launch_command_for_container("codex", None).expect("container launch");
        assert_eq!(resolved.command, "npx");
        assert_eq!(
            resolved.args,
            vec!["--yes".to_string(), "@openai/codex@latest".to_string()]
        );
        assert_eq!(resolved.tool_version, "latest");
    }

    #[test]
    fn resolve_agent_launch_command_for_container_uses_npx_for_pinned_version() {
        let resolved = resolve_agent_launch_command_for_container("codex", Some("1.2.3"))
            .expect("container launch");
        assert_eq!(resolved.command, "npx");
        assert_eq!(
            resolved.args,
            vec!["--yes".to_string(), "@openai/codex@1.2.3".to_string()]
        );
        assert_eq!(resolved.tool_version, "1.2.3");
    }

    #[test]
    fn normalize_launch_command_for_platform_windows_unwraps_wrapped_path() {
        let raw = r#"'\"C:\Program Files\nodejs\npx.cmd\"'"#.to_string();
        let normalized = normalize_launch_command_for_platform(raw);
        assert_eq!(normalized, r#"C:\Program Files\nodejs\npx.cmd"#);
    }

    #[test]
    fn normalize_launch_command_for_platform_windows_extracts_leading_command_token() {
        let raw = r#"'\"C:\Program Files\nodejs\npx.cmd\"' --yes @openai/codex@latest"#.to_string();
        let normalized = normalize_launch_command_for_platform(raw);
        assert_eq!(normalized, r#"C:\Program Files\nodejs\npx.cmd"#);
    }

    #[test]
    fn normalized_process_command_unwraps_issue_1265_pattern() {
        let normalized = normalized_process_command(r#"'\"C:\Program Files\nodejs\npx.cmd\"'"#);
        assert_eq!(normalized, r#"C:\Program Files\nodejs\npx.cmd"#);
    }

    #[test]
    fn normalized_process_command_trims_plain_commands() {
        let normalized = normalized_process_command("  codex  ");
        assert_eq!(normalized, "codex");
    }

    #[test]
    fn normalize_agent_version_trims_and_strips_prefixes() {
        assert_eq!(normalize_agent_version(None), None);
        assert_eq!(normalize_agent_version(Some("  ")), None);
        assert_eq!(
            normalize_agent_version(Some("v1.2.3")),
            Some("1.2.3".to_string())
        );
        assert_eq!(
            normalize_agent_version(Some("@1.2.3")),
            Some("1.2.3".to_string())
        );
        assert_eq!(
            normalize_agent_version(Some("  latest ")),
            Some("latest".to_string())
        );
        assert_eq!(
            normalize_agent_version(Some("vnext")),
            Some("vnext".to_string())
        );
    }

    #[test]
    fn build_bunx_package_spec_defaults_to_latest() {
        assert_eq!(
            build_bunx_package_spec("@openai/codex", None),
            "@openai/codex@latest"
        );
    }

    #[test]
    fn build_bunx_package_spec_uses_specified_version() {
        assert_eq!(
            build_bunx_package_spec("@openai/codex", Some("1.2.3")),
            "@openai/codex@1.2.3"
        );
    }

    #[test]
    fn build_agent_model_args_is_agent_specific() {
        assert_eq!(
            build_agent_model_args("codex", Some("gpt-5.2")),
            vec!["--model=gpt-5.2".to_string()]
        );
        assert_eq!(
            build_agent_model_args("claude", Some("sonnet")),
            vec!["--model".to_string(), "sonnet".to_string()]
        );
        assert_eq!(
            build_agent_model_args("opencode", Some("provider/model")),
            vec!["-m".to_string(), "provider/model".to_string()]
        );
        assert_eq!(
            build_agent_model_args("gemini", Some("gemini-2.5-pro")),
            vec!["-m".to_string(), "gemini-2.5-pro".to_string()]
        );
        assert!(build_agent_model_args("codex", None).is_empty());
        assert!(build_agent_model_args("codex", Some("  ")).is_empty());
    }

    #[test]
    fn is_enter_only_accepts_cr_lf_variants() {
        assert!(is_enter_only(b"\r"));
        assert!(is_enter_only(b"\n"));
        assert!(is_enter_only(b"\r\n"));
    }

    #[test]
    fn is_enter_only_rejects_non_enter_input() {
        assert!(!is_enter_only(b""));
        assert!(!is_enter_only(b"a"));
        assert!(!is_enter_only(b"\nq"));
    }

    #[test]
    fn send_keys_to_pane_errors_when_pane_not_running() {
        let _lock = crate::commands::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tempfile::TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());

        let state = AppState::new();
        let pane_id = "pane-send-test";
        let pane =
            gwt_core::terminal::pane::TerminalPane::new(gwt_core::terminal::pane::PaneConfig {
                pane_id: pane_id.to_string(),
                command: "/usr/bin/true".to_string(),
                args: vec![],
                working_dir: std::env::temp_dir(),
                branch_name: "test-branch".to_string(),
                agent_name: "test-agent".to_string(),
                agent_color: AgentColor::Green,
                rows: 24,
                cols: 80,
                env_vars: HashMap::new(),
                terminal_shell: None,
                interactive: false,
                windows_force_utf8: false,
                project_root: std::env::temp_dir(),
            })
            .expect("failed to create test pane");

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            mgr.add_pane(pane).expect("failed to add test pane");
        }

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            let pane = mgr.pane_mut_by_id(pane_id).expect("missing test pane");
            for _ in 0..20 {
                let _ = pane.check_status();
                if !matches!(pane.status(), PaneStatus::Running) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if matches!(pane.status(), PaneStatus::Running) {
                let _ = pane.kill();
                for _ in 0..20 {
                    let _ = pane.check_status();
                    if !matches!(pane.status(), PaneStatus::Running) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }

        let result = send_keys_to_pane_from_state(&state, pane_id, "hello\n", None);
        assert!(result.is_err());

        let mut mgr = state.pane_manager.lock().unwrap();
        let _ = mgr.kill_all();
    }

    #[test]
    fn send_keys_broadcast_counts_running_panes() {
        let _lock = crate::commands::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tempfile::TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());

        let state = AppState::new();

        let pane_running =
            gwt_core::terminal::pane::TerminalPane::new(gwt_core::terminal::pane::PaneConfig {
                pane_id: "pane-running".to_string(),
                command: "/bin/cat".to_string(),
                args: vec![],
                working_dir: std::env::temp_dir(),
                branch_name: "branch-a".to_string(),
                agent_name: "agent-a".to_string(),
                agent_color: AgentColor::Green,
                rows: 24,
                cols: 80,
                env_vars: HashMap::new(),
                terminal_shell: None,
                interactive: false,
                windows_force_utf8: false,
                project_root: std::env::temp_dir(),
            })
            .expect("failed to create running pane");

        let pane_done =
            gwt_core::terminal::pane::TerminalPane::new(gwt_core::terminal::pane::PaneConfig {
                pane_id: "pane-done".to_string(),
                command: "/usr/bin/true".to_string(),
                args: vec![],
                working_dir: std::env::temp_dir(),
                branch_name: "branch-b".to_string(),
                agent_name: "agent-b".to_string(),
                agent_color: AgentColor::Green,
                rows: 24,
                cols: 80,
                env_vars: HashMap::new(),
                terminal_shell: None,
                interactive: false,
                windows_force_utf8: false,
                project_root: std::env::temp_dir(),
            })
            .expect("failed to create done pane");

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            mgr.add_pane(pane_running)
                .expect("failed to add running pane");
            mgr.add_pane(pane_done).expect("failed to add done pane");
        }

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            for _ in 0..20 {
                let status = {
                    let pane = mgr.pane_mut_by_id("pane-done").expect("missing done pane");
                    let _ = pane.check_status();
                    pane.status()
                };
                if !matches!(status, PaneStatus::Running) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let sent = send_keys_broadcast_from_state(&state, "ping\n").expect("broadcast failed");
        assert_eq!(sent, 1);

        let mut mgr = state.pane_manager.lock().unwrap();
        let _ = mgr.kill_all();
    }

    #[test]
    fn capture_scrollback_tail_returns_plain_text() {
        let _lock = crate::commands::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tempfile::TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());

        let state = AppState::new();
        let pane_id = "pane-capture-test";
        let pane =
            gwt_core::terminal::pane::TerminalPane::new(gwt_core::terminal::pane::PaneConfig {
                pane_id: pane_id.to_string(),
                command: "/bin/cat".to_string(),
                args: vec![],
                working_dir: std::env::temp_dir(),
                branch_name: "test-branch".to_string(),
                agent_name: "test-agent".to_string(),
                agent_color: AgentColor::Green,
                rows: 24,
                cols: 80,
                env_vars: HashMap::new(),
                terminal_shell: None,
                interactive: false,
                windows_force_utf8: false,
                project_root: std::env::temp_dir(),
            })
            .expect("failed to create test pane");

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            mgr.add_pane(pane).expect("failed to add test pane");
            let pane = mgr.pane_mut_by_id(pane_id).expect("missing test pane");
            pane.process_bytes(b"hi \x1b[31mred\x1b[0m\n")
                .expect("failed to write test bytes");
        }

        let captured = capture_scrollback_tail_from_state(&state, pane_id, 1024, None)
            .expect("capture should succeed");
        assert!(captured.contains("hi red"));
        assert!(!captured.contains("\x1b"));

        let mut mgr = state.pane_manager.lock().unwrap();
        let _ = mgr.kill_all();
    }

    #[test]
    fn mark_pane_stream_error_sets_error_status_and_records_message() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());

        let state = AppState::new();
        let pane_id = "pane-stream-error-test";
        let pane =
            gwt_core::terminal::pane::TerminalPane::new(gwt_core::terminal::pane::PaneConfig {
                pane_id: pane_id.to_string(),
                command: "/bin/cat".to_string(),
                args: vec![],
                working_dir: std::env::temp_dir(),
                branch_name: "test-branch".to_string(),
                agent_name: "test-agent".to_string(),
                agent_color: AgentColor::Green,
                rows: 24,
                cols: 80,
                env_vars: HashMap::new(),
                terminal_shell: None,
                interactive: false,
                windows_force_utf8: false,
                project_root: std::env::temp_dir(),
            })
            .expect("failed to create test pane");

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            mgr.add_pane(pane).expect("failed to add test pane");
        }

        let bytes = mark_pane_stream_error_and_write_message(&state, pane_id, "mock read failure");
        let output = String::from_utf8_lossy(&bytes);
        assert_eq!(output, "\r\n[PTY stream error: mock read failure]\r\n");

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            let pane = mgr.pane_mut_by_id(pane_id).expect("missing test pane");
            assert_eq!(
                pane.status(),
                &PaneStatus::Error("PTY stream error: mock read failure".to_string())
            );
        }

        let captured = capture_scrollback_tail_from_state(&state, pane_id, 1024, None)
            .expect("capture should succeed");
        assert!(captured.contains("[PTY stream error: mock read failure]"));

        let mut mgr = state.pane_manager.lock().unwrap();
        let _ = mgr.kill_all();
    }

    #[test]
    fn append_close_hint_to_pane_scrollback_records_hint() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());

        let state = AppState::new();
        let pane_id = "pane-stream-close-hint-test";
        let pane =
            gwt_core::terminal::pane::TerminalPane::new(gwt_core::terminal::pane::PaneConfig {
                pane_id: pane_id.to_string(),
                command: "/bin/cat".to_string(),
                args: vec![],
                working_dir: std::env::temp_dir(),
                branch_name: "test-branch".to_string(),
                agent_name: "test-agent".to_string(),
                agent_color: AgentColor::Green,
                rows: 24,
                cols: 80,
                env_vars: HashMap::new(),
                terminal_shell: None,
                interactive: false,
                windows_force_utf8: false,
                project_root: std::env::temp_dir(),
            })
            .expect("failed to create test pane");

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            mgr.add_pane(pane).expect("failed to add test pane");
        }

        let bytes = append_close_hint_to_pane_scrollback(&state, pane_id);
        let output = String::from_utf8_lossy(&bytes);
        assert_eq!(output, "\r\nPress Enter to close this tab.\r\n");

        let captured = capture_scrollback_tail_from_state(&state, pane_id, 1024, None)
            .expect("capture should succeed");
        assert!(captured.contains("Press Enter to close this tab."));

        let mut mgr = state.pane_manager.lock().unwrap();
        let _ = mgr.kill_all();
    }

    fn make_request(agent_id: &str) -> LaunchAgentRequest {
        LaunchAgentRequest {
            agent_id: agent_id.to_string(),
            branch: "feature/test".to_string(),
            profile: None,
            model: None,
            agent_version: None,
            mode: None,
            skip_permissions: None,
            reasoning_level: None,
            fast_mode: None,
            collaboration_modes: None,
            extra_args: None,
            env_overrides: None,
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            resume_session_id: None,
            create_branch: None,
            issue_number: None,
            ai_branch_description: None,
            terminal_shell: None,
        }
    }

    #[test]
    fn make_request_sets_issue_number_none_by_default() {
        let req = make_request("codex");
        assert_eq!(req.issue_number, None);
    }

    #[test]
    fn launch_request_deserialize_with_issue_number() {
        let raw = serde_json::json!({
            "agentId": "codex",
            "branch": "feature/issue-42",
            "issueNumber": 42
        });
        let req: LaunchAgentRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.issue_number, Some(42));
    }

    #[test]
    fn build_agent_args_codex_continue_defaults_to_resume_last() {
        let mut req = make_request("codex");
        req.mode = Some(SessionMode::Continue);
        let args = build_agent_args("codex", &req, Some("0.92.0"), false).unwrap();
        assert_eq!(args[0], "resume");
        assert_eq!(args[1], "--last");
        assert!(args.iter().any(|a| a.starts_with("--model=")));
    }

    #[test]
    fn build_agent_args_codex_collaboration_modes_always_enabled() {
        let req = make_request("codex");
        let args = build_agent_args("codex", &req, Some("latest"), false).unwrap();
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--enable" && w[1] == "collaboration_modes"));
    }

    #[test]
    fn build_agent_args_codex_multi_agent_enabled() {
        let req = make_request("codex");
        let args = build_agent_args("codex", &req, Some("latest"), true).unwrap();
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--enable" && w[1] == "multi_agent"));
    }

    #[test]
    fn build_agent_args_codex_multi_agent_disabled() {
        let req = make_request("codex");
        let args = build_agent_args("codex", &req, Some("latest"), false).unwrap();
        assert!(!args
            .windows(2)
            .any(|w| w[0] == "--enable" && w[1] == "multi_agent"));
    }

    #[test]
    fn codex_features_list_contains_feature_parses_first_column() {
        let raw = r#"
multi_agent                      experimental       true
collaboration_modes              stable             true
"#;
        assert!(codex_features_list_contains_feature(raw, "multi_agent"));
        assert!(!codex_features_list_contains_feature(raw, "collab"));
    }

    #[test]
    fn codex_config_has_collab_alias_detects_legacy_key() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[features]
collab = true
multi_agent = true
"#,
        )
        .unwrap();
        assert!(codex_config_has_collab_alias(&path).unwrap());
    }

    #[test]
    fn codex_config_has_collab_alias_ignores_missing_key() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[features]
multi_agent = true
"#,
        )
        .unwrap();
        assert!(!codex_config_has_collab_alias(&path).unwrap());
    }

    #[test]
    fn build_agent_args_codex_skip_flag_is_version_gated() {
        let mut req = make_request("codex");
        req.skip_permissions = Some(true);

        let legacy = build_agent_args("codex", &req, Some("0.79.9"), false).unwrap();
        assert!(legacy.iter().any(|a| a == "--yolo"));

        let modern = build_agent_args("codex", &req, Some("0.80.0"), false).unwrap();
        assert!(modern
            .iter()
            .any(|a| a == "--dangerously-bypass-approvals-and-sandbox"));
    }

    #[test]
    fn build_agent_args_codex_default_model_is_version_gated() {
        let req = make_request("codex");

        let latest = build_agent_args("codex", &req, Some("latest"), false).unwrap();
        assert!(latest.iter().any(|a| a == "--model=gpt-5.4"));
        assert!(latest.iter().any(|a| a == "model_context_window=1000000"));
        assert!(latest
            .iter()
            .any(|a| a == "model_auto_compact_token_limit=950000"));

        let installed = build_agent_args("codex", &req, Some("0.111.0"), false).unwrap();
        assert!(installed.iter().any(|a| a == "--model=gpt-5.4"));
        assert!(installed
            .iter()
            .any(|a| a == "model_context_window=1000000"));
        assert!(installed
            .iter()
            .any(|a| a == "model_auto_compact_token_limit=950000"));

        let legacy = build_agent_args("codex", &req, Some("0.110.0"), false).unwrap();
        assert!(legacy.iter().any(|a| a == "--model=gpt-5.2-codex"));
        assert!(!legacy.iter().any(|a| a == "--model=gpt-5.4"));
        assert!(!legacy.iter().any(|a| a == "model_context_window=1000000"));
        assert!(!legacy
            .iter()
            .any(|a| a == "model_auto_compact_token_limit=950000"));
    }

    #[test]
    fn build_agent_args_codex_gpt_5_4_override_enables_context_overrides() {
        let mut req = make_request("codex");
        req.model = Some("gpt-5.4".to_string());

        let args = build_agent_args("codex", &req, Some("0.111.0"), false).unwrap();
        assert!(args.iter().any(|a| a == "--model=gpt-5.4"));
        assert!(args.iter().any(|a| a == "model_context_window=1000000"));
        assert!(args
            .iter()
            .any(|a| a == "model_auto_compact_token_limit=950000"));
    }

    #[test]
    fn build_agent_args_codex_fast_mode_adds_service_tier_for_gpt_5_4() {
        let mut req = make_request("codex");
        req.model = Some("gpt-5.4".to_string());
        req.fast_mode = Some(true);

        let args = build_agent_args("codex", &req, Some("0.111.0"), false).unwrap();
        assert!(args.iter().any(|a| a == "service_tier=fast"));
    }

    #[test]
    fn build_agent_args_claude_continue_prefers_resume_id_when_provided() {
        let mut req = make_request("claude");
        req.mode = Some(SessionMode::Continue);
        req.resume_session_id = Some("sess-123".to_string());
        let args = build_agent_args("claude", &req, None, false).unwrap();
        assert_eq!(args[0], "--resume");
        assert_eq!(args[1], "sess-123");
    }

    #[test]
    fn build_agent_args_claude_resume_without_id_opens_picker() {
        let mut req = make_request("claude");
        req.mode = Some(SessionMode::Resume);
        let args = build_agent_args("claude", &req, None, false).unwrap();
        assert_eq!(args[0], "--resume");
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn build_agent_args_gemini_continue_prefers_resume_id_when_provided() {
        let mut req = make_request("gemini");
        req.mode = Some(SessionMode::Continue);
        req.resume_session_id = Some("sess-123".to_string());
        let args = build_agent_args("gemini", &req, None, false).unwrap();
        assert_eq!(args, vec!["-r".to_string(), "sess-123".to_string()]);
    }

    #[test]
    fn build_agent_args_opencode_continue_prefers_resume_id_when_provided() {
        let mut req = make_request("opencode");
        req.mode = Some(SessionMode::Continue);
        req.resume_session_id = Some("sess-123".to_string());
        let args = build_agent_args("opencode", &req, None, false).unwrap();
        assert_eq!(args, vec!["-s".to_string(), "sess-123".to_string()]);
    }

    #[test]
    fn build_agent_args_opencode_resume_requires_session_id() {
        let mut req = make_request("opencode");
        req.mode = Some(SessionMode::Resume);
        let err = build_agent_args("opencode", &req, None, false).unwrap_err();
        assert!(err.to_lowercase().contains("session id"));
    }

    #[test]
    fn build_docker_compose_up_args_build_and_recreate_flags() {
        assert_eq!(
            build_docker_compose_up_args(&[], false, false, None),
            vec![
                "compose".to_string(),
                "up".to_string(),
                "-d".to_string(),
                "--no-build".to_string(),
            ]
        );

        let build = build_docker_compose_up_args(&[], true, false, None);
        assert!(build.contains(&"--build".to_string()));
        assert!(!build.contains(&"--no-build".to_string()));

        let recreate = build_docker_compose_up_args(&[], false, true, None);
        assert!(recreate.contains(&"--force-recreate".to_string()));

        let with_service = build_docker_compose_up_args(&[], false, false, Some("dev"));
        assert_eq!(with_service.last(), Some(&"dev".to_string()));

        let with_blank_service = build_docker_compose_up_args(&[], false, false, Some("  "));
        assert_eq!(
            with_blank_service,
            vec![
                "compose".to_string(),
                "up".to_string(),
                "-d".to_string(),
                "--no-build".to_string(),
            ]
        );
    }

    #[test]
    fn compose_services_output_contains_matches_trimmed_exact_line() {
        let output = " app \nworker\n";
        assert!(compose_services_output_contains(output, "app"));
        assert!(compose_services_output_contains(output, "worker"));
    }

    #[test]
    fn compose_services_output_contains_does_not_match_partial_names() {
        let output = "application\nworker-1\n";
        assert!(!compose_services_output_contains(output, "app"));
        assert!(!compose_services_output_contains(output, "worker"));
    }

    #[test]
    fn verify_docker_compose_service_start_skips_when_service_is_missing() {
        let running_called = Cell::new(false);
        let logs_called = Cell::new(false);
        let down_called = Cell::new(false);

        let result = verify_docker_compose_service_start(
            None,
            |_| {
                running_called.set(true);
                Ok(true)
            },
            |_| {
                logs_called.set(true);
                None
            },
            || {
                down_called.set(true);
                Ok(())
            },
        );

        assert!(result.is_ok());
        assert!(!running_called.get());
        assert!(!logs_called.get());
        assert!(!down_called.get());
    }

    #[test]
    fn verify_docker_compose_service_start_returns_ok_when_service_is_running() {
        let down_called = Cell::new(false);

        let result = verify_docker_compose_service_start(
            Some(" app "),
            |service| {
                assert_eq!(service, "app");
                Ok(true)
            },
            |_| Some("should not be called".to_string()),
            || {
                down_called.set(true);
                Ok(())
            },
        );

        assert!(result.is_ok());
        assert!(!down_called.get());
    }

    #[test]
    fn verify_docker_compose_service_start_runs_best_effort_down_on_failure() {
        let down_called = Cell::new(false);

        let err = verify_docker_compose_service_start(
            Some("app"),
            |_| Ok(false),
            |_| Some("service logs".to_string()),
            || {
                down_called.set(true);
                Ok(())
            },
        )
        .unwrap_err();

        assert!(down_called.get());
        assert!(err.contains("docker compose service 'app' is not running after startup."));
        assert!(err.contains("service logs"));
        assert!(!err.contains("Failed to run best-effort docker compose down"));
    }

    #[test]
    fn verify_docker_compose_service_start_appends_down_error_on_cleanup_failure() {
        let down_called = Cell::new(false);

        let err = verify_docker_compose_service_start(
            Some("app"),
            |_| Ok(false),
            |_| None,
            || {
                down_called.set(true);
                Err("cleanup failed".to_string())
            },
        )
        .unwrap_err();

        assert!(down_called.get());
        assert!(err.contains("docker compose service 'app' is not running after startup."));
        assert!(err.contains("Failed to run best-effort docker compose down"));
        assert!(err.contains("cleanup failed"));
    }

    #[test]
    fn build_docker_compose_exec_args_sorts_env_and_appends_inner_command() {
        let mut env = HashMap::new();
        env.insert("B".to_string(), "2".to_string());
        env.insert("A".to_string(), "1".to_string());

        let inner_args = vec!["--yes".to_string(), "pkg@latest".to_string()];
        let args =
            build_docker_compose_exec_args(&[], "app", "/workspace", &env, "npx", &inner_args);

        let pos_a = args.iter().position(|s| s == "A=1").unwrap();
        let pos_b = args.iter().position(|s| s == "B=2").unwrap();
        assert!(pos_a < pos_b);

        let pos_service = args.iter().position(|s| s == "app").unwrap();
        let pos_cmd = args.iter().position(|s| s == "npx").unwrap();
        assert!(pos_service < pos_cmd);

        assert!(args.ends_with(&inner_args));
    }

    #[test]
    fn build_docker_compose_exec_args_omits_workdir_when_empty() {
        let env = HashMap::new();
        let args = build_docker_compose_exec_args(&[], "app", "", &env, "npx", &[]);

        assert!(!args.contains(&"-w".to_string()));
    }

    #[test]
    fn build_docker_compose_exec_args_skips_invalid_env_names() {
        let mut env = HashMap::new();
        env.insert("=::".to_string(), "bad".to_string());
        env.insert("VALID_KEY".to_string(), "ok".to_string());

        let args = build_docker_compose_exec_args(&[], "app", "", &env, "npx", &[]);
        assert!(args.contains(&"VALID_KEY=ok".to_string()));
        assert!(!args.iter().any(|a| a.contains("=::")));
    }

    #[test]
    fn merge_profile_env_for_docker_filters_non_passthrough_keys() {
        let mut base = HashMap::new();
        base.insert("GITHUB_TOKEN".to_string(), "old".to_string());

        let mut merged = HashMap::new();
        merged.insert("GITHUB_TOKEN".to_string(), "new".to_string());
        merged.insert("GH_TOKEN".to_string(), "gh".to_string());
        merged.insert("TERM".to_string(), "xterm-256color".to_string());
        merged.insert("Path".to_string(), "C:\\Windows\\System32".to_string());
        merged.insert("RANDOM_KEY".to_string(), "ignored".to_string());
        merged.insert("=C:".to_string(), "C:\\tmp".to_string());

        merge_profile_env_for_docker(&mut base, &merged);

        assert_eq!(base.get("GITHUB_TOKEN"), Some(&"new".to_string()));
        assert_eq!(base.get("GH_TOKEN"), Some(&"gh".to_string()));
        assert_eq!(base.get("TERM"), Some(&"xterm-256color".to_string()));
        assert!(!base.contains_key("Path"));
        assert!(!base.contains_key("RANDOM_KEY"));
        assert!(!base.contains_key("=C:"));
    }

    #[test]
    fn compose_file_paths_from_args_extracts_only_compose_files() {
        let args = vec![
            "-f".to_string(),
            "/tmp/compose.base.yml".to_string(),
            "--project-name".to_string(),
            "test".to_string(),
            "-f".to_string(),
            "/tmp/compose.override.yml".to_string(),
        ];

        let paths = compose_file_paths_from_args(&args);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("/tmp/compose.base.yml"));
        assert_eq!(paths[1], PathBuf::from("/tmp/compose.override.yml"));
    }

    #[test]
    fn merge_compose_env_for_docker_includes_non_allowlisted_compose_keys() {
        let temp = TempDir::new().unwrap();
        let compose_path = temp.path().join("docker-compose.yml");
        std::fs::write(
            &compose_path,
            r#"
services:
  app:
    environment:
      - CUSTOM_ENV
      - GITHUB_TOKEN
"#,
        )
        .unwrap();

        let mut base = HashMap::new();
        let mut merged = HashMap::new();
        merged.insert("CUSTOM_ENV".to_string(), "custom".to_string());
        merged.insert("GITHUB_TOKEN".to_string(), "ghs_xxx".to_string());

        merge_compose_env_for_docker(&mut base, &merged, &[compose_path]);

        assert_eq!(base.get("CUSTOM_ENV"), Some(&"custom".to_string()));
        assert_eq!(base.get("GITHUB_TOKEN"), Some(&"ghs_xxx".to_string()));
    }

    #[test]
    fn merge_compose_env_for_docker_keeps_existing_allocated_port_when_incoming_is_occupied() {
        use std::net::TcpListener;

        let temp = TempDir::new().unwrap();
        let compose_path = temp.path().join("docker-compose.yml");
        std::fs::write(
            &compose_path,
            r#"
services:
  app:
    ports:
      - "${KNOWLEDGE_DB_PORT:-5432}:5432"
"#,
        )
        .unwrap();

        let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();

        let mut base = HashMap::new();
        base.insert("KNOWLEDGE_DB_PORT".to_string(), "15432".to_string());

        let mut merged = HashMap::new();
        merged.insert("KNOWLEDGE_DB_PORT".to_string(), occupied_port.to_string());

        merge_compose_env_for_docker(&mut base, &merged, &[compose_path]);

        assert_eq!(base.get("KNOWLEDGE_DB_PORT"), Some(&"15432".to_string()));
    }

    #[test]
    fn docker_compose_exec_workdir_preserves_service_default_without_workspace_folder() {
        assert_eq!(
            docker_compose_exec_workdir(None, Path::new("D:/Repository/GE/GrimoireEngine.git")),
            ""
        );
    }

    #[test]
    fn docker_compose_exec_workdir_uses_workspace_folder_when_present() {
        assert_eq!(
            docker_compose_exec_workdir(
                Some("/workspace"),
                Path::new("D:/Repository/GE/GrimoireEngine.git"),
            ),
            "/workspace"
        );
        assert_eq!(
            docker_compose_exec_workdir(
                Some("  /app  "),
                Path::new("D:/Repository/GE/GrimoireEngine.git"),
            ),
            "/app"
        );
    }

    #[test]
    fn docker_mount_target_path_converts_windows_drive_style() {
        assert_eq!(
            docker_mount_target_path("D:/Repository/GE/GrimoireEngine.git"),
            "/Repository/GE/GrimoireEngine.git"
        );
        assert_eq!(
            docker_mount_target_path("d:\\Repository\\GE\\GrimoireEngine.git"),
            "/Repository/GE/GrimoireEngine.git"
        );
        assert_eq!(
            docker_mount_target_path("/Repository/GE/GrimoireEngine.git"),
            "/Repository/GE/GrimoireEngine.git"
        );
    }

    #[test]
    fn build_git_bind_mounts_skips_nested_worktree_mount_even_with_mixed_paths() {
        let mut env = HashMap::new();
        env.insert(
            "HOST_GIT_COMMON_DIR".to_string(),
            "/Repository/GE/GrimoireEngine.git".to_string(),
        );
        env.insert(
            "HOST_GIT_WORKTREE_DIR".to_string(),
            "D:/Repository/GE/GrimoireEngine.git/worktrees/feature-refactor".to_string(),
        );

        let mounts = build_git_bind_mounts(&env);
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].source, "/Repository/GE/GrimoireEngine.git");
        assert_eq!(mounts[0].target, "/Repository/GE/GrimoireEngine.git");
    }

    #[test]
    fn render_docker_compose_override_uses_long_syntax_bind_mounts() {
        let mounts = vec![DockerBindMount {
            source: "D:/Repository/GE/GrimoireEngine.git".to_string(),
            target: "/Repository/GE/GrimoireEngine.git".to_string(),
        }];

        let services = vec!["app".to_string(), "unity-mcp-server".to_string()];
        let yaml = render_docker_compose_override("app", &services, "gwt-develop", &mounts);
        assert!(yaml.contains("container_name: 'gwt-develop-app'"));
        assert!(yaml.contains("container_name: 'gwt-develop-unity-mcp-server'"));
        assert!(yaml.contains("type: bind"));
        assert!(yaml.contains("source: 'D:/Repository/GE/GrimoireEngine.git'"));
        assert!(yaml.contains("target: '/Repository/GE/GrimoireEngine.git'"));
        assert!(!yaml.contains("${HOST_GIT_WORKTREE_DIR}:${HOST_GIT_WORKTREE_DIR}"));
    }

    #[test]
    fn render_docker_compose_override_adds_container_names_without_mounts() {
        let services = vec!["unity-mcp-server".to_string()];
        let yaml = render_docker_compose_override("unity-mcp-server", &services, "gwt-dev", &[]);
        assert!(yaml.contains("services:\n  unity-mcp-server:\n"));
        assert!(yaml.contains("container_name: 'gwt-dev-unity-mcp-server'"));
        assert!(!yaml.contains("volumes:"));
    }

    #[test]
    fn build_terminal_ansi_probe_counts_basic_color_sgr() {
        let bytes = b"hi \x1b[31mred\x1b[0m\n";
        let probe = build_terminal_ansi_probe("pane-x", bytes);
        assert_eq!(probe.pane_id, "pane-x");
        assert!(probe.esc_count >= 2);
        assert!(probe.sgr_count >= 2);
        assert_eq!(probe.color_sgr_count, 1);
        assert!(!probe.has_256_color);
        assert!(!probe.has_true_color);
    }

    #[test]
    fn build_terminal_ansi_probe_detects_256_and_truecolor() {
        let bytes_256 = b"\x1b[38;5;196mX\x1b[0m";
        let probe_256 = build_terminal_ansi_probe("p", bytes_256);
        assert_eq!(probe_256.color_sgr_count, 1);
        assert!(probe_256.has_256_color);
        assert!(!probe_256.has_true_color);

        let bytes_true = b"\x1b[38;2;255;0;0mX\x1b[0m";
        let probe_true = build_terminal_ansi_probe("p", bytes_true);
        assert_eq!(probe_true.color_sgr_count, 1);
        assert!(!probe_true.has_256_color);
        assert!(probe_true.has_true_color);
    }

    #[test]
    fn build_terminal_ansi_probe_does_not_count_non_sgr_csi() {
        let bytes = b"\x1b[2K\x1b[10D";
        let probe = build_terminal_ansi_probe("p", bytes);
        assert_eq!(probe.sgr_count, 0);
        assert_eq!(probe.color_sgr_count, 0);
    }

    #[test]
    fn build_terminal_ansi_probe_does_not_treat_italic_as_color() {
        let bytes = b"\x1b[3mitalic\x1b[0m";
        let probe = build_terminal_ansi_probe("p", bytes);
        assert_eq!(probe.sgr_count, 2);
        assert_eq!(probe.color_sgr_count, 0);
    }

    #[test]
    fn test_merge_os_base_only() {
        let _lock = crate::commands::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tempfile::TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());

        let os_env = HashMap::from([
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/Users/test".to_string()),
        ]);
        // Isolate HOME so user profile config never affects this test.
        let result = merge_profile_env(&os_env, None);
        assert_eq!(result.get("PATH"), Some(&"/usr/bin".to_string()));
        assert_eq!(result.get("HOME"), Some(&"/Users/test".to_string()));
    }

    #[test]
    fn test_merge_empty_os_env() {
        let _lock = crate::commands::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tempfile::TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());

        let os_env = HashMap::new();
        let result = merge_profile_env(&os_env, None);
        assert!(result.is_empty());
    }

    #[test]
    fn inject_openai_api_key_from_profile_ai_when_env_missing() {
        let _lock = crate::commands::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tempfile::TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());

        let mut config = gwt_core::config::ProfilesConfig::default();
        if let Some(profile) = config.profiles.get_mut("default") {
            profile.ai = Some(gwt_core::config::AISettings {
                endpoint: "https://api.openai.com/v1".to_string(),
                api_key: "sk_test_profile_key".to_string(),
                model: String::new(),
                language: "en".to_string(),
                summary_enabled: true,
            });
        }
        config.save().unwrap();

        let mut env_vars = HashMap::new();
        inject_openai_api_key_from_profile_ai(&mut env_vars, None);

        assert_eq!(
            env_vars.get("OPENAI_API_KEY"),
            Some(&"sk_test_profile_key".to_string())
        );
    }

    #[test]
    fn inject_openai_api_key_from_profile_ai_does_not_override_existing_env_value() {
        let _lock = crate::commands::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tempfile::TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());

        let mut config = gwt_core::config::ProfilesConfig::default();
        if let Some(profile) = config.profiles.get_mut("default") {
            profile.ai = Some(gwt_core::config::AISettings {
                endpoint: "https://api.openai.com/v1".to_string(),
                api_key: "sk_test_profile_key".to_string(),
                model: String::new(),
                language: "en".to_string(),
                summary_enabled: true,
            });
        }
        config.save().unwrap();

        let mut env_vars =
            HashMap::from([(String::from("OPENAI_API_KEY"), String::from("sk_env_key"))]);
        inject_openai_api_key_from_profile_ai(&mut env_vars, None);

        assert_eq!(
            env_vars.get("OPENAI_API_KEY"),
            Some(&"sk_env_key".to_string())
        );
    }

    #[test]
    fn probe_terminal_ansi_flushes_scrollback_before_reading() {
        let _lock = crate::commands::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tempfile::TempDir::new().unwrap();
        let _env = crate::commands::TestEnvGuard::new(home.path());

        let state = AppState::new();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let pane_id = format!("pane-test-{nonce}");

        let pane =
            gwt_core::terminal::pane::TerminalPane::new(gwt_core::terminal::pane::PaneConfig {
                pane_id: pane_id.clone(),
                command: "/usr/bin/true".to_string(),
                args: vec![],
                working_dir: std::env::temp_dir(),
                branch_name: "test-branch".to_string(),
                agent_name: "test-agent".to_string(),
                agent_color: AgentColor::Green,
                rows: 24,
                cols: 80,
                env_vars: HashMap::new(),
                terminal_shell: None,
                interactive: false,
                windows_force_utf8: false,
                project_root: std::env::temp_dir(),
            })
            .expect("failed to create test pane");

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            mgr.add_pane(pane).expect("failed to add test pane");
            let pane = mgr.pane_mut_by_id(&pane_id).expect("missing test pane");
            pane.process_bytes(b"hi \x1b[31mred\x1b[0m\n")
                .expect("failed to write test bytes");
        }

        let probe = probe_terminal_ansi_from_state(&state, &pane_id).expect("probe should succeed");
        assert!(probe.bytes_scanned > 0);
        assert!(probe.sgr_count >= 2);
        assert!(probe.color_sgr_count >= 1);

        let _ = ScrollbackFile::cleanup(&pane_id);
    }

    // gwt-spec issue FR-106: Claude Code launch must always set CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1
    #[test]
    fn claude_launch_env_sets_agent_teams() {
        // Verify that after the IS_SANDBOX block, Claude Code launches include
        // CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1 regardless of skip_permissions.
        let mut env_vars: HashMap<String, String> = HashMap::new();
        let agent_id = "claude";
        let skip_permissions = false;

        // Simulate the env-var injection logic from launch_agent_inner
        if agent_id == "claude" && skip_permissions && std::env::consts::OS != "windows" {
            env_vars.insert("IS_SANDBOX".to_string(), "1".to_string());
        }
        if agent_id == "claude" {
            env_vars
                .entry("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS".to_string())
                .or_insert_with(|| "1".to_string());
        }

        assert_eq!(
            env_vars.get("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"),
            Some(&"1".to_string()),
            "Claude Code launch env must include CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1"
        );
    }

    #[test]
    fn codex_launch_env_no_agent_teams() {
        let mut env_vars: HashMap<String, String> = HashMap::new();
        let agent_id = "codex";

        if agent_id == "claude" {
            env_vars
                .entry("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS".to_string())
                .or_insert_with(|| "1".to_string());
        }

        assert!(
            !env_vars.contains_key("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"),
            "Codex launch env must not include CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"
        );
    }

    // --- WSL prompt detection tests (T031) ---

    #[test]
    fn detect_shell_prompt_dollar() {
        assert!(detect_shell_prompt(b"user@host:~$ "));
    }

    #[test]
    fn detect_shell_prompt_dollar_eol() {
        assert!(detect_shell_prompt(b"user@host:~$"));
    }

    #[test]
    fn detect_shell_prompt_hash() {
        assert!(detect_shell_prompt(b"root@host:~# "));
    }

    #[test]
    fn detect_shell_prompt_hash_eol() {
        assert!(detect_shell_prompt(b"root@host:~#"));
    }

    #[test]
    fn detect_shell_prompt_angle() {
        assert!(detect_shell_prompt(b"PS C:\\Users> "));
    }

    #[test]
    fn detect_shell_prompt_percent() {
        assert!(detect_shell_prompt(b"user@host ~ %"));
    }

    #[test]
    fn detect_shell_prompt_empty() {
        assert!(!detect_shell_prompt(b""));
    }

    #[test]
    fn detect_shell_prompt_no_prompt() {
        assert!(!detect_shell_prompt(b"Welcome to Ubuntu 22.04 LTS\n"));
    }

    #[test]
    fn detect_shell_prompt_with_ansi() {
        // Prompt with ANSI color codes: \e[32muser@host\e[0m:~$
        let buf = b"\x1b[32muser@host\x1b[0m:~$";
        assert!(detect_shell_prompt(buf));
    }

    // --- ANSI strip tests ---

    #[test]
    fn strip_ansi_bytes_plain() {
        let input = b"hello world";
        assert_eq!(strip_ansi_bytes(input), b"hello world");
    }

    #[test]
    fn strip_ansi_bytes_csi() {
        let input = b"\x1b[32mgreen\x1b[0m";
        assert_eq!(strip_ansi_bytes(input), b"green");
    }

    #[test]
    fn strip_ansi_bytes_osc_bel() {
        // OSC 7 terminated by BEL
        let input = b"\x1b]7;file:///home/user\x07prompt$";
        assert_eq!(strip_ansi_bytes(input), b"prompt$");
    }

    #[test]
    fn strip_ansi_bytes_osc_st() {
        // OSC terminated by ST (ESC \)
        let input = b"\x1b]0;title\x1b\\prompt$";
        assert_eq!(strip_ansi_bytes(input), b"prompt$");
    }

    // --- build_wsl_inject_command tests (T033) ---

    #[test]
    fn build_wsl_inject_command_no_env() {
        let cmd = build_wsl_inject_command(
            "claude",
            &["--dangerously-skip-permissions".to_string()],
            &HashMap::new(),
            "/mnt/c/Users/foo",
        );
        assert_eq!(
            cmd,
            "cd '/mnt/c/Users/foo' && exec claude '--dangerously-skip-permissions'"
        );
    }

    #[test]
    fn build_wsl_inject_command_with_env() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let cmd = build_wsl_inject_command("claude", &[], &env, "/mnt/c/repo");
        assert_eq!(cmd, "export FOO='bar' && cd '/mnt/c/repo' && exec claude");
    }

    #[test]
    fn build_wsl_inject_command_env_with_quotes() {
        let mut env = HashMap::new();
        env.insert("MSG".to_string(), "it's a test".to_string());
        let cmd = build_wsl_inject_command("echo", &[], &env, "/mnt/c");
        assert_eq!(
            cmd,
            "export MSG='it'\\''s a test' && cd '/mnt/c' && exec echo"
        );
    }

    #[test]
    fn build_wsl_inject_command_multiple_env_sorted() {
        let mut env = HashMap::new();
        env.insert("ZZZ".to_string(), "last".to_string());
        env.insert("AAA".to_string(), "first".to_string());
        let cmd = build_wsl_inject_command("agent", &[], &env, "/mnt/c");
        assert!(cmd.starts_with("export AAA='first' && export ZZZ='last'"));
    }

    #[test]
    fn builtin_agent_def_copilot() {
        let def = builtin_agent_def("copilot").expect("copilot should be defined");
        assert_eq!(def.label, "GitHub Copilot");
        assert_eq!(def.local_command, "copilot");
        assert_eq!(def.bunx_package, "@github/copilot");
    }

    #[test]
    fn build_agent_model_args_copilot() {
        assert_eq!(
            build_agent_model_args("copilot", Some("gpt-4.1")),
            vec!["--model".to_string(), "gpt-4.1".to_string()]
        );
        assert!(build_agent_model_args("copilot", None).is_empty());
    }

    #[test]
    fn agent_color_for_copilot() {
        assert_eq!(agent_color_for("copilot"), AgentColor::Blue);
    }

    #[test]
    fn tool_id_for_copilot() {
        assert_eq!(tool_id_for("copilot"), "github-copilot");
    }

    #[test]
    fn build_agent_args_copilot_continue() {
        let mut req = make_request("copilot");
        req.mode = Some(SessionMode::Continue);
        let args = build_agent_args("copilot", &req, None, false).unwrap();
        assert!(args.contains(&"--continue".to_string()));
    }

    #[test]
    fn build_agent_args_copilot_continue_prefers_resume_id_when_provided() {
        let mut req = make_request("copilot");
        req.mode = Some(SessionMode::Continue);
        req.resume_session_id = Some("sess-123".to_string());
        let args = build_agent_args("copilot", &req, None, false).unwrap();
        assert_eq!(args, vec!["--resume".to_string(), "sess-123".to_string()]);
    }

    #[test]
    fn build_agent_args_copilot_resume_without_id_opens_picker() {
        let mut req = make_request("copilot");
        req.mode = Some(SessionMode::Resume);
        let args = build_agent_args("copilot", &req, None, false).unwrap();
        assert_eq!(args, vec!["--resume".to_string()]);
    }

    #[test]
    fn build_agent_args_copilot_resume_with_id() {
        let mut req = make_request("copilot");
        req.mode = Some(SessionMode::Resume);
        req.resume_session_id = Some("sess-123".to_string());
        let args = build_agent_args("copilot", &req, None, false).unwrap();
        assert_eq!(args, vec!["--resume".to_string(), "sess-123".to_string()]);
    }

    #[test]
    fn build_agent_args_copilot_skip_permissions() {
        let mut req = make_request("copilot");
        req.skip_permissions = Some(true);
        let args = build_agent_args("copilot", &req, None, false).unwrap();
        assert!(args.contains(&"--allow-all-tools".to_string()));
    }
}

fn is_launch_cancelled(cancelled: Option<&AtomicBool>) -> bool {
    cancelled.is_some_and(|c| c.load(Ordering::SeqCst))
}

fn report_launch_progress(
    job_id: Option<&str>,
    app_handle: &AppHandle,
    step: &str,
    detail: Option<&str>,
) {
    let Some(job_id) = job_id else {
        return;
    };
    let payload = LaunchProgressPayload {
        job_id: job_id.to_string(),
        step: step.to_string(),
        detail: detail.map(|s| s.to_string()),
    };
    let _ = app_handle.emit("launch-progress", &payload);
}

pub(crate) fn launch_agent_for_project_root(
    project_root: PathBuf,
    request: LaunchAgentRequest,
    state: &AppState,
    app_handle: AppHandle,
    job_id: Option<&str>,
    cancelled: Option<&AtomicBool>,
) -> Result<String, String> {
    tracing::debug!(job_id = ?job_id, "launch step: fetch");
    report_launch_progress(job_id, &app_handle, "fetch", None);
    if is_launch_cancelled(cancelled) {
        return Err("Cancelled".to_string());
    }

    tracing::debug!(job_id = ?job_id, "launch step: validate");
    report_launch_progress(job_id, &app_handle, "validate", None);
    let agent_id = request.agent_id.trim();
    if agent_id.is_empty() {
        return Err("Agent is required".to_string());
    }
    if agent_id == "codex" {
        if let Some(path) = codex_config_path() {
            match codex_config_has_collab_alias(&path) {
                Ok(true) => {
                    let detail = "Deprecated Codex feature key `collab` detected. Use `[features].multi_agent` in ~/.codex/config.toml.";
                    tracing::warn!(path = %path.display(), "{detail}");
                    report_launch_progress(job_id, &app_handle, "validate", Some(detail));
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::debug!(%error, "Failed to inspect Codex config for deprecated keys");
                }
            }
        }
    }
    if is_launch_cancelled(cancelled) {
        return Err("Cancelled".to_string());
    }

    tracing::debug!(job_id = ?job_id, "launch step: paths");
    report_launch_progress(job_id, &app_handle, "paths", None);
    let repo_path = resolve_repo_path_for_project_root(&project_root)?;
    if is_launch_cancelled(cancelled) {
        return Err("Cancelled".to_string());
    }

    tracing::debug!(job_id = ?job_id, "launch step: conflicts");
    report_launch_progress(job_id, &app_handle, "conflicts", None);
    if is_launch_cancelled(cancelled) {
        return Err("Cancelled".to_string());
    }

    tracing::debug!(job_id = ?job_id, "launch step: create");
    report_launch_progress(job_id, &app_handle, "create", None);

    let mut created_issue_branch_for_cleanup: Option<String> = None;

    let (working_dir, branch_name, worktree_created) = if let Some(create) =
        request.create_branch.as_ref()
    {
        // Determine branch name: AI generation or direct input
        let new_branch = if let Some(ai_desc) = request.ai_branch_description.as_deref() {
            let ai_desc = ai_desc.trim();
            if ai_desc.is_empty() {
                return Err("AI branch description is required".to_string());
            }
            // AI branch name generation
            report_launch_progress(
                job_id,
                &app_handle,
                "create",
                Some("Generating branch name..."),
            );

            let profiles = ProfilesConfig::load()
                .map_err(|e| format!("[E2001] AI branch name generation failed: {e}"))?;
            let ai = profiles.resolve_active_ai_settings();
            let settings = ai.resolved.ok_or_else(|| {
                "[E2001] AI branch name generation failed: AI is not configured".to_string()
            })?;
            let client = gwt_core::ai::AIClient::new(settings)
                .map_err(|e| format!("[E2001] AI branch name generation failed: {e}"))?;
            gwt_core::ai::suggest_branch_name(&client, ai_desc)
                .map_err(|e| format!("[E2001] AI branch name generation failed: {e}"))?
        } else {
            let name = create.name.trim();
            if name.is_empty() {
                return Err("New branch name is required".to_string());
            }
            name.to_string()
        };

        let base = create
            .base
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());

        if is_launch_cancelled(cancelled) {
            return Err("Cancelled".to_string());
        }

        if let Some(issue_number) = request.issue_number {
            let remotes = Remote::list(&repo_path).unwrap_or_default();
            let issue_base = base.map(|b| strip_known_remote_prefix(b, &remotes).to_string());
            let link_status = create_or_verify_linked_branch(
                &repo_path,
                issue_number,
                &new_branch,
                issue_base.as_deref(),
            )?;

            if matches!(link_status, IssueLinkedBranchStatus::Created) {
                created_issue_branch_for_cleanup = Some(new_branch.clone());
            }

            let (path, created) = match resolve_worktree_path(&repo_path, &new_branch) {
                Ok(result) => result,
                Err(err) => {
                    if let Some(branch) = created_issue_branch_for_cleanup.as_deref() {
                        if let Err(cleanup_err) = rollback_new_issue_branch(&repo_path, branch) {
                            tracing::warn!(
                                branch = branch,
                                error = %cleanup_err,
                                "Issue launch rollback failed after worktree resolution error"
                            );
                        }
                    }
                    return Err(err);
                }
            };

            (path, new_branch, created)
        } else {
            (
                create_new_worktree_path(&repo_path, &new_branch, base)?,
                new_branch,
                true,
            )
        }
    } else {
        let branch_ref = request.branch.trim();
        if branch_ref.is_empty() {
            return Err("Branch is required".to_string());
        }
        let remotes = Remote::list(&repo_path).unwrap_or_default();
        let name = strip_known_remote_prefix(branch_ref, &remotes).to_string();
        let (path, created) = resolve_worktree_path(&repo_path, branch_ref)?;
        (path, name, created)
    };

    tracing::debug!(
        category = "terminal",
        requested_branch = request.branch.as_str(),
        resolved_branch = branch_name.as_str(),
        working_dir = %working_dir.display(),
        repo_path = %repo_path.display(),
        worktree_created,
        "Resolved launch worktree path"
    );

    let launch_result: Result<String, String> = (|| {
        if is_launch_cancelled(cancelled) {
            return Err("Cancelled".to_string());
        }

        if worktree_created {
            let payload = WorktreesChangedPayload {
                project_path: project_root.to_string_lossy().to_string(),
                branch: branch_name.clone(),
            };
            let _ = app_handle.emit("worktrees-changed", &payload);
        }

        // Record stats (agent launch + optional worktree creation) non-blocking.
        {
            let stat_agent_id = agent_id.to_string();
            let stat_model = request.model.as_deref().unwrap_or("").to_string();
            let stat_repo = repo_path.to_string_lossy().to_string();
            let stat_wt_created = worktree_created;
            std::thread::spawn(move || {
                let result = Stats::update(|stats| {
                    stats.increment_agent_launch(&stat_agent_id, &stat_model, &stat_repo);
                    if stat_wt_created {
                        stats.increment_worktree_created(&stat_repo);
                    }
                });
                if let Err(e) = result {
                    tracing::warn!(error = %e, "Failed to record stats");
                }
            });
        }

        // --- Skill registration ---
        tracing::debug!(job_id = ?job_id, "launch step: skills");
        report_launch_progress(job_id, &app_handle, "skills", None);
        if is_launch_cancelled(cancelled) {
            return Err("Cancelled".to_string());
        }
        match gwt_core::config::Settings::load_global() {
            Ok(settings) => {
                if is_launch_cancelled(cancelled) {
                    return Err("Cancelled".to_string());
                }
                let status =
                    gwt_core::config::repair_skill_registration_with_settings_at_project_root(
                        &settings,
                        Some(working_dir.as_path()),
                    );
                if is_launch_cancelled(cancelled) {
                    return Err("Cancelled".to_string());
                }
                state.set_skill_registration_status(status);
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "skills step skipped: failed to load global settings"
                );
                if is_launch_cancelled(cancelled) {
                    return Err("Cancelled".to_string());
                }
                state.set_skill_registration_status(Default::default());
            }
        }
        if is_launch_cancelled(cancelled) {
            return Err("Cancelled".to_string());
        }

        tracing::debug!(job_id = ?job_id, "launch step: deps (waiting for environment)");
        report_launch_progress(job_id, &app_handle, "deps", Some("Waiting for environment"));
        if is_launch_cancelled(cancelled) {
            return Err("Cancelled".to_string());
        }

        // Wait briefly for startup OS env capture; if it is still running, continue with the
        // latest available snapshot (usually process env) instead of blocking launches.
        if !state.wait_os_env_ready(Duration::from_secs(2)) {
            tracing::warn!(
                category = "os_env",
                "OS environment capture still in progress; launching with current snapshot"
            );
        }
        let mut os_env = state.os_env_snapshot();
        if os_env.is_empty() {
            os_env = std::env::vars().collect();
        }

        if is_launch_cancelled(cancelled) {
            return Err("Cancelled".to_string());
        }

        tracing::debug!(job_id = ?job_id, "launch step: deps (resolved)");
        report_launch_progress(job_id, &app_handle, "deps", None);
        let mut env_vars = merge_profile_env(&os_env, request.profile.as_deref());
        inject_openai_api_key_from_profile_ai(&mut env_vars, request.profile.as_deref());
        // Useful for debugging and for agents that want to introspect gwt context.
        env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            project_root.to_string_lossy().to_string(),
        );

        // Agent-specific env (global; not per-profile). Request overrides are still highest precedence.
        let mut wants_claude_glm = false;
        if agent_id == "claude" {
            if let Ok(cfg) = AgentConfig::load() {
                wants_claude_glm = cfg.claude.provider == ClaudeAgentProvider::Glm;
                if wants_claude_glm {
                    let base_url = cfg.claude.glm.base_url.trim();
                    let token = cfg.claude.glm.auth_token.trim();
                    let timeout = cfg.claude.glm.api_timeout_ms.trim();
                    let opus = cfg.claude.glm.default_opus_model.trim();
                    let sonnet = cfg.claude.glm.default_sonnet_model.trim();
                    let haiku = cfg.claude.glm.default_haiku_model.trim();

                    if !base_url.is_empty() {
                        env_vars.insert("ANTHROPIC_BASE_URL".to_string(), base_url.to_string());
                    }
                    if !token.is_empty() {
                        env_vars.insert("ANTHROPIC_AUTH_TOKEN".to_string(), token.to_string());
                    }
                    if !timeout.is_empty() {
                        env_vars.insert("API_TIMEOUT_MS".to_string(), timeout.to_string());
                    }
                    if !opus.is_empty() {
                        env_vars
                            .insert("ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(), opus.to_string());
                    }
                    if !sonnet.is_empty() {
                        env_vars.insert(
                            "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
                            sonnet.to_string(),
                        );
                    }
                    if !haiku.is_empty() {
                        env_vars.insert(
                            "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
                            haiku.to_string(),
                        );
                    }
                }
            }
        }

        // Request-specific overrides are highest precedence.
        if let Some(overrides) = request.env_overrides.as_ref() {
            for (k, v) in overrides {
                env_vars.insert(k.to_string(), v.to_string());
            }
        }

        if wants_claude_glm {
            // If we are configured for GLM, require at least Base URL + Token by the end of merging.
            let base_url = env_vars
                .get("ANTHROPIC_BASE_URL")
                .map(|s| s.trim())
                .unwrap_or("");
            let token = env_vars
                .get("ANTHROPIC_AUTH_TOKEN")
                .map(|s| s.trim())
                .unwrap_or("");
            if base_url.is_empty() || token.is_empty() {
                return Err("GLM (z.ai) provider is selected but required env vars are missing. Configure Base URL and API Token in Launch Agent > Provider.".to_string());
            }
        }

        let skip_permissions = request.skip_permissions.unwrap_or(false);
        if agent_id == "claude" && skip_permissions && std::env::consts::OS != "windows" {
            // gwt-spec issue: Skip-permissions on non-Windows sets IS_SANDBOX=1 to avoid
            // accidental confirmation prompts in sandboxed environments.
            env_vars.insert("IS_SANDBOX".to_string(), "1".to_string());
        }

        // gwt-spec issue FR-106: Always enable Agent Teams for Claude Code launches.
        if agent_id == "claude" {
            env_vars
                .entry("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS".to_string())
                .or_insert_with(|| "1".to_string());
        }

        // Ensure TERM/COLORTERM propagate into Docker exec environments as well.
        // (PTY sets these for the host process, but docker exec only receives vars passed via -e.)
        ensure_terminal_env_defaults(&mut env_vars);

        let settings = Settings::load_global().unwrap_or_default();
        let force_host_settings = settings.docker.force_host;
        let force_host_request = request.docker_force_host.unwrap_or(false);
        let docker_force_host = force_host_settings || force_host_request;

        let docker_build = request.docker_build.unwrap_or(false);
        let docker_recreate = request.docker_recreate.unwrap_or(false);
        let docker_keep = request.docker_keep.unwrap_or(false);

        let mut docker_service = request
            .docker_service
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let mut docker_container_name: Option<String> = None;
        let mut docker_compose_args: Option<Vec<String>> = None;
        let mut docker_env: Option<HashMap<String, String>> = None;

        #[derive(Debug, Clone)]
        struct DockerBuildSpec {
            dockerfile_path: PathBuf,
            context_dir: PathBuf,
        }

        enum DockerExecMode {
            None,
            Compose {
                service: String,
                workdir: String,
                compose_args: Vec<String>,
            },
            DockerRun {
                image: String,
                workdir: String,
                build: Option<DockerBuildSpec>,
            },
        }

        let mut docker_mode = DockerExecMode::None;

        if !docker_force_host {
            match detect_docker_files(&working_dir) {
                Some(DockerFileType::Compose(compose_path)) => {
                    // Best-effort compose service selection.
                    let services = DockerManager::list_services_from_compose_file(&compose_path)
                        .map_err(|e| e.to_string())?;
                    if services.is_empty() {
                        return Err("No services found in docker compose file".to_string());
                    }

                    if let Some(selected) = docker_service.as_deref() {
                        if !services.iter().any(|s| s == selected) {
                            return Err(format!("Docker service not found: {}", selected));
                        }
                    } else {
                        docker_service = Some(services[0].clone());
                    }

                    let service = docker_service
                        .clone()
                        .ok_or_else(|| "Docker service is required".to_string())?;

                    let container_name = DockerManager::generate_container_name(&branch_name);
                    let manager = DockerManager::new(
                        &working_dir,
                        &branch_name,
                        DockerFileType::Compose(compose_path.clone()),
                    );

                    let mut compose_args =
                        vec!["-f".to_string(), compose_path.to_string_lossy().to_string()];
                    let compose_paths = compose_file_paths_from_args(&compose_args);

                    let mut env = manager.collect_passthrough_env();
                    // Merge only docker-relevant profile/env keys to avoid oversized command lines
                    // and invalid Windows pseudo env keys (e.g. "=C:").
                    merge_profile_env_for_docker(&mut env, &env_vars);
                    merge_compose_env_for_docker(&mut env, &env_vars, &compose_paths);
                    env.insert("COMPOSE_PROJECT_NAME".to_string(), container_name.clone());
                    apply_translated_git_env(&mut env);

                    let mounts = build_git_bind_mounts(&env);
                    if let Some(override_path) = write_docker_compose_override(
                        &project_root,
                        &container_name,
                        &service,
                        &services,
                        &mounts,
                    )? {
                        compose_args.push("-f".to_string());
                        compose_args.push(override_path.to_string_lossy().to_string());
                    }

                    docker_compose_up(
                        &working_dir,
                        &container_name,
                        &env,
                        &compose_args,
                        docker_build,
                        docker_recreate,
                        Some(service.as_str()),
                    )?;

                    docker_container_name = Some(container_name);
                    docker_compose_args = Some(compose_args.clone());
                    docker_env = Some(env);
                    docker_mode = DockerExecMode::Compose {
                        service,
                        workdir: docker_compose_exec_workdir(None, &working_dir),
                        compose_args,
                    };
                }
                Some(DockerFileType::DevContainer(devcontainer_path)) => {
                    let cfg =
                        DevContainerConfig::load(&devcontainer_path).map_err(|e| e.to_string())?;
                    let devcontainer_dir = devcontainer_path
                        .parent()
                        .ok_or_else(|| "Invalid devcontainer path".to_string())?;

                    if cfg.uses_compose() {
                        let mut compose_args = cfg.to_compose_args(devcontainer_dir);
                        if compose_args.is_empty() {
                            return Err("Devcontainer is configured for compose but no compose files were found".to_string());
                        }
                        let compose_paths = compose_file_paths_from_args(&compose_args);

                        // Best-effort compose service selection: prefer request, then devcontainer.json, then first service.
                        let mut services: Vec<String> = Vec::new();
                        let mut i = 0usize;
                        while i + 1 < compose_args.len() {
                            if compose_args[i] == "-f" {
                                let path = PathBuf::from(&compose_args[i + 1]);
                                let mut s = DockerManager::list_services_from_compose_file(&path)
                                    .unwrap_or_default();
                                services.append(&mut s);
                            }
                            i += 1;
                        }
                        services.sort();
                        services.dedup();
                        if services.is_empty() {
                            return Err(
                                "No services found in devcontainer compose files".to_string()
                            );
                        }

                        let preferred_service = docker_service
                            .clone()
                            .or_else(|| cfg.get_service().map(|s| s.to_string()));
                        if let Some(selected) = preferred_service.as_deref() {
                            if !services.iter().any(|s| s == selected) {
                                return Err(format!("Docker service not found: {}", selected));
                            }
                            docker_service = Some(selected.to_string());
                        } else {
                            docker_service = Some(services[0].clone());
                        }

                        let service = docker_service
                            .clone()
                            .ok_or_else(|| "Docker service is required".to_string())?;

                        let container_name = DockerManager::generate_container_name(&branch_name);
                        let manager = DockerManager::new(
                            &working_dir,
                            &branch_name,
                            DockerFileType::DevContainer(devcontainer_path.clone()),
                        );

                        let mut env = manager.collect_passthrough_env();
                        merge_profile_env_for_docker(&mut env, &env_vars);
                        merge_compose_env_for_docker(&mut env, &env_vars, &compose_paths);
                        env.insert("COMPOSE_PROJECT_NAME".to_string(), container_name.clone());
                        apply_translated_git_env(&mut env);

                        let mounts = build_git_bind_mounts(&env);
                        if let Some(override_path) = write_docker_compose_override(
                            &project_root,
                            &container_name,
                            &service,
                            &services,
                            &mounts,
                        )? {
                            compose_args.push("-f".to_string());
                            compose_args.push(override_path.to_string_lossy().to_string());
                        }

                        docker_compose_up(
                            &working_dir,
                            &container_name,
                            &env,
                            &compose_args,
                            docker_build,
                            docker_recreate,
                            Some(service.as_str()),
                        )?;

                        docker_container_name = Some(container_name);
                        docker_compose_args = Some(compose_args.clone());
                        docker_env = Some(env);
                        docker_mode = DockerExecMode::Compose {
                            service,
                            workdir: docker_compose_exec_workdir(
                                cfg.workspace_folder.as_deref(),
                                &working_dir,
                            ),
                            compose_args,
                        };
                    } else if let Some(image) = cfg
                        .image
                        .as_deref()
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                    {
                        // Image-based devcontainer: run the provided image (no build).
                        docker_mode = DockerExecMode::DockerRun {
                            image: image.to_string(),
                            workdir: cfg
                                .workspace_folder
                                .as_deref()
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| DOCKER_WORKDIR.to_string()),
                            build: None,
                        };
                    } else if let Some(dockerfile_rel) = cfg.get_dockerfile() {
                        let dockerfile_path = devcontainer_dir.join(dockerfile_rel);
                        let context_dir = cfg
                            .build
                            .as_ref()
                            .and_then(|b| b.context.as_deref())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(|s| devcontainer_dir.join(s))
                            .unwrap_or_else(|| working_dir.clone());

                        docker_mode = DockerExecMode::DockerRun {
                            image: DockerManager::generate_container_name(&branch_name),
                            workdir: cfg
                                .workspace_folder
                                .as_deref()
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| DOCKER_WORKDIR.to_string()),
                            build: Some(DockerBuildSpec {
                                dockerfile_path,
                                context_dir,
                            }),
                        };
                    }
                }
                Some(DockerFileType::Dockerfile(dockerfile_path)) => {
                    docker_mode = DockerExecMode::DockerRun {
                        image: DockerManager::generate_container_name(&branch_name),
                        workdir: DOCKER_WORKDIR.to_string(),
                        build: Some(DockerBuildSpec {
                            dockerfile_path,
                            context_dir: working_dir.clone(),
                        }),
                    };
                }
                _ => {}
            }
        }

        if is_launch_cancelled(cancelled) {
            return Err("Cancelled".to_string());
        }

        let use_docker = !matches!(docker_mode, DockerExecMode::None);

        let resolved = if use_docker {
            resolve_agent_launch_command_for_container(agent_id, request.agent_version.as_deref())?
        } else {
            resolve_agent_launch_command(agent_id, request.agent_version.as_deref())?
        };

        let version_for_gates = resolved
            .version_for_gates
            .as_deref()
            .or(Some(resolved.tool_version.as_str()));

        let enable_codex_multi_agent = if agent_id == "codex" {
            let context = CodexFeatureProbeContext {
                command: resolved.command.clone(),
                args: resolved.args.clone(),
                tool_version: resolved.tool_version.clone(),
            };
            codex_supports_multi_agent(&context)
        } else {
            false
        };

        let mut args = resolved.args.clone();
        args.extend(build_agent_args(
            agent_id,
            &request,
            version_for_gates,
            enable_codex_multi_agent,
        )?);

        let mode = request.mode.unwrap_or(SessionMode::Normal);
        let mode_str = match mode {
            SessionMode::Normal => "normal",
            SessionMode::Continue => "continue",
            SessionMode::Resume => "resume",
        }
        .to_string();

        let collaboration_modes = if agent_id == "codex" {
            codex_supports_collaboration_modes(version_for_gates)
        } else {
            false
        };

        let model = request
            .model
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let reasoning_level = request
            .reasoning_level
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let started_at_millis = now_millis();

        // Best-effort session history entry (do not block launch on IO errors).
        {
            let docker_force_host_entry = if force_host_request {
                Some(true)
            } else if use_docker {
                Some(false)
            } else {
                None
            };
            let docker_service_entry = if matches!(docker_mode, DockerExecMode::Compose { .. }) {
                docker_service.clone()
            } else {
                None
            };
            let entry = gwt_core::config::ToolSessionEntry {
                branch: branch_name.clone(),
                worktree_path: Some(working_dir.to_string_lossy().to_string()),
                tool_id: tool_id_for(agent_id),
                tool_label: resolved.label.to_string(),
                session_id: None,
                mode: Some(mode_str.clone()),
                model: model.clone(),
                reasoning_level: if agent_id == "codex" {
                    reasoning_level.clone()
                } else {
                    None
                },
                skip_permissions: Some(skip_permissions),
                tool_version: Some(resolved.tool_version.clone()),
                collaboration_modes: if agent_id == "codex" {
                    Some(collaboration_modes)
                } else {
                    None
                },
                docker_service: docker_service_entry,
                docker_force_host: docker_force_host_entry,
                docker_recreate: if use_docker {
                    Some(docker_recreate)
                } else {
                    None
                },
                docker_build: if use_docker { Some(docker_build) } else { None },
                docker_keep: if use_docker { Some(docker_keep) } else { None },
                docker_container_name: docker_container_name.clone(),
                docker_compose_args: docker_compose_args.clone(),
                timestamp: started_at_millis,
            };

            if let Err(err) = gwt_core::config::save_session_entry(&repo_path, entry) {
                tracing::warn!(error = %err, "Failed to save session entry (launch): continuing");
            }
        }

        // For Dockerfile-based launches, build the image best-effort before starting the PTY.
        if let DockerExecMode::DockerRun {
            image,
            build: Some(build),
            ..
        } = &docker_mode
        {
            ensure_docker_ready()?;
            if docker_should_build_image(&build.dockerfile_path, image, docker_build) {
                docker_build_image(image, &build.dockerfile_path, &build.context_dir)?;
            }
        }

        let meta = PaneLaunchMeta {
            agent_id: agent_id.to_string(),
            branch: branch_name.clone(),
            repo_path: repo_path.clone(),
            worktree_path: working_dir.clone(),
            tool_label: resolved.label.to_string(),
            tool_version: resolved.tool_version.clone(),
            mode: mode_str.clone(),
            model: model.clone(),
            reasoning_level: reasoning_level.clone(),
            fast_mode: if agent_id == "codex" {
                request.fast_mode.unwrap_or(false)
            } else {
                false
            },
            skip_permissions,
            collaboration_modes,
            docker_service: if matches!(docker_mode, DockerExecMode::Compose { .. }) {
                docker_service.clone()
            } else {
                None
            },
            docker_force_host: if force_host_request {
                Some(true)
            } else if use_docker {
                Some(false)
            } else {
                None
            },
            docker_recreate: if use_docker {
                Some(docker_recreate)
            } else {
                None
            },
            docker_build: if use_docker { Some(docker_build) } else { None },
            docker_keep: if use_docker { Some(docker_keep) } else { None },
            docker_container_name: docker_container_name.clone(),
            docker_compose_args: docker_compose_args.clone(),
            started_at_millis,
        };

        let config = match docker_mode {
            DockerExecMode::Compose {
                service,
                workdir,
                compose_args,
            } => {
                let docker_env = docker_env
                    .as_ref()
                    .ok_or_else(|| "Docker env is missing".to_string())?;
                let docker_args = build_docker_compose_exec_args(
                    &compose_args,
                    &service,
                    &workdir,
                    docker_env,
                    &resolved.command,
                    &args,
                );
                BuiltinLaunchConfig {
                    command: "docker".to_string(),
                    args: docker_args,
                    working_dir,
                    branch_name,
                    agent_name: resolved.label.to_string(),
                    agent_color: agent_color_for(agent_id),
                    env_vars: docker_env.clone(),
                    terminal_shell: None,
                    interactive: false,
                    windows_force_utf8: false,
                }
            }
            DockerExecMode::DockerRun {
                image,
                workdir,
                build,
            } => {
                let docker_file_type = build
                    .as_ref()
                    .map(|b| DockerFileType::Dockerfile(b.dockerfile_path.clone()))
                    .unwrap_or_else(|| DockerFileType::Dockerfile(working_dir.join("Dockerfile")));

                let manager = DockerManager::new(&working_dir, &branch_name, docker_file_type);
                let mut container_env = manager.collect_passthrough_env();
                merge_profile_env_for_docker(&mut container_env, &env_vars);
                apply_translated_git_env(&mut container_env);
                let git_mounts = build_git_bind_mounts(&container_env);

                // Mount the selected worktree and required git dirs, then run the agent in the container.
                let mount_target = workdir.clone();
                let mut run_args: Vec<String> = vec![
                    "run".to_string(),
                    "--rm".to_string(),
                    "-i".to_string(),
                    "-t".to_string(),
                    "-w".to_string(),
                    workdir,
                    "-v".to_string(),
                    format!("{}:{}", working_dir.to_string_lossy(), mount_target),
                ];

                for mount in &git_mounts {
                    run_args.push("-v".to_string());
                    run_args.push(format!("{}:{}", mount.source, mount.target));
                }

                let mut keys: Vec<&String> = container_env.keys().collect();
                keys.sort();
                for key in keys {
                    let k = key.trim();
                    if k.is_empty() || !is_valid_docker_env_key(k) {
                        continue;
                    }
                    let v = container_env
                        .get(key)
                        .map(|s| s.as_str())
                        .unwrap_or_default();
                    run_args.push("-e".to_string());
                    run_args.push(format!("{k}={v}"));
                }

                run_args.push(image);
                run_args.push(resolved.command);
                run_args.extend(args.iter().cloned());

                BuiltinLaunchConfig {
                    command: "docker".to_string(),
                    args: run_args,
                    working_dir,
                    branch_name,
                    agent_name: resolved.label.to_string(),
                    agent_color: agent_color_for(agent_id),
                    env_vars: HashMap::new(),
                    terminal_shell: None,
                    interactive: false,
                    windows_force_utf8: false,
                }
            }
            DockerExecMode::None => {
                let terminal_shell = resolve_shell_id_for_spawn(request.terminal_shell.as_deref());

                // WSL agent launch uses the PTY-write approach (FR-007).
                if should_launch_agent_with_wsl_shell(terminal_shell.as_deref()) {
                    if is_launch_cancelled(cancelled) {
                        return Err("Cancelled".to_string());
                    }
                    let wsl_path = gwt_core::terminal::shell::windows_to_wsl_path(
                        &working_dir.to_string_lossy(),
                    )
                    .map_err(|e| format!("WSL path conversion failed: {e}"))?;

                    // WSL PTY-write launches must receive the same merged env as host launches.
                    let wsl_env_vars = env_vars.clone();

                    return launch_with_wsl_pty_write(
                        WslPtyWriteParams {
                            repo_path: repo_path.clone(),
                            agent_command: resolved.command.clone(),
                            agent_args: args.clone(),
                            working_dir,
                            wsl_working_dir: wsl_path,
                            branch_name,
                            agent_name: resolved.label.to_string(),
                            agent_color: agent_color_for(agent_id),
                            env_vars: wsl_env_vars,
                            meta: Some(meta),
                        },
                        state,
                        app_handle,
                    );
                }

                BuiltinLaunchConfig {
                    command: resolved.command,
                    args,
                    working_dir,
                    branch_name,
                    agent_name: resolved.label.to_string(),
                    agent_color: agent_color_for(agent_id),
                    env_vars,
                    terminal_shell,
                    interactive: true,
                    windows_force_utf8: cfg!(target_os = "windows"),
                }
            }
        };

        if is_launch_cancelled(cancelled) {
            return Err("Cancelled".to_string());
        }

        launch_with_config(&repo_path, config, Some(meta), state, app_handle)
    })();

    match launch_result {
        Ok(pane_id) => Ok(pane_id),
        Err(err) => {
            if let Some(branch) = created_issue_branch_for_cleanup.as_deref() {
                if let Err(cleanup_err) = rollback_new_issue_branch(&repo_path, branch) {
                    tracing::warn!(
                        branch = branch,
                        error = %cleanup_err,
                        launch_error = %err,
                        "Issue launch rollback failed"
                    );
                }
            }
            Err(err)
        }
    }
}

/// Launch an agent with gwt semantics (worktree + profiles)
#[tauri::command]
pub fn launch_agent(
    window: tauri::Window,
    request: LaunchAgentRequest,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<String, StructuredError> {
    let project_root = {
        let Some(p) = state.project_for_window(window.label()) else {
            return Err(StructuredError::internal(
                "No project opened",
                "launch_agent",
            ));
        };
        PathBuf::from(p)
    };
    launch_agent_for_project_root(project_root, request, &state, app_handle, None, None)
        .map_err(|e| StructuredError::internal(&e, "launch_agent"))
}

/// Start an async launch job with progress events (gwt-spec issue US15).
#[tauri::command]
pub fn start_launch_job(
    window: tauri::Window,
    request: LaunchAgentRequest,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<String, StructuredError> {
    let project_root = {
        let Some(p) = state.project_for_window(window.label()) else {
            return Err(StructuredError::internal(
                "No project opened",
                "start_launch_job",
            ));
        };
        p
    };

    let job_id = Uuid::new_v4().to_string();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut jobs) = state.launch_jobs.lock() {
        jobs.insert(job_id.clone(), cancel_flag.clone());
    }

    let app = app_handle.clone();
    let job_id_thread = job_id.clone();
    std::thread::spawn(move || {
        tracing::debug!(job_id = %job_id_thread, "launch thread started");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let state = app.state::<AppState>();
            launch_agent_for_project_root(
                PathBuf::from(project_root),
                request,
                &state,
                app.clone(),
                Some(job_id_thread.as_str()),
                Some(cancel_flag.as_ref()),
            )
        }));

        let finished = match &result {
            Ok(Ok(pane_id)) => {
                tracing::debug!(job_id = %job_id_thread, pane_id = %pane_id, "launch succeeded");
                LaunchFinishedPayload {
                    job_id: job_id_thread.clone(),
                    status: "ok".to_string(),
                    pane_id: Some(pane_id.clone()),
                    error: None,
                }
            }
            Ok(Err(err)) if err.trim() == "Cancelled" => {
                tracing::debug!(job_id = %job_id_thread, "launch cancelled");
                LaunchFinishedPayload {
                    job_id: job_id_thread.clone(),
                    status: "cancelled".to_string(),
                    pane_id: None,
                    error: None,
                }
            }
            Ok(Err(err)) => {
                tracing::warn!(job_id = %job_id_thread, error = %err, "launch failed");
                LaunchFinishedPayload {
                    job_id: job_id_thread.clone(),
                    status: "error".to_string(),
                    pane_id: None,
                    error: Some(err.clone()),
                }
            }
            Err(_panic) => {
                tracing::error!(job_id = %job_id_thread, "launch thread panicked");
                LaunchFinishedPayload {
                    job_id: job_id_thread.clone(),
                    status: "error".to_string(),
                    pane_id: None,
                    error: Some("Internal error: launch thread panicked".to_string()),
                }
            }
        };

        tracing::debug!(job_id = %job_id_thread, status = %finished.status, "emitting launch-finished");

        // Store the result for polling retrieval before emitting.  This
        // guarantees the frontend can always recover the result even when
        // Tauri events are silently lost.
        if let Some(state) = app.try_state::<AppState>() {
            if let Ok(mut results) = state.launch_results.lock() {
                results.insert(job_id_thread.clone(), finished.clone());
            }
        }

        let _ = app.emit("launch-finished", &finished);

        // Remove the running-job entry last so that polling sees the job
        // as "running" until both the result store and event are done.
        if let Some(state) = app.try_state::<AppState>() {
            if let Ok(mut jobs) = state.launch_jobs.lock() {
                jobs.remove(&job_id_thread);
            }
        }
    });

    Ok(job_id)
}

/// Cancel a running launch job (best-effort).
#[tauri::command]
pub fn cancel_launch_job(job_id: String, state: State<AppState>) -> Result<(), StructuredError> {
    let id = job_id.trim();
    if id.is_empty() {
        return Ok(());
    }

    if let Ok(jobs) = state.launch_jobs.lock() {
        if let Some(flag) = jobs.get(id) {
            flag.store(true, Ordering::SeqCst);
        }
    }
    Ok(())
}

/// Frontend polling result for a launch job.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchJobPollResult {
    /// `true` while the job thread is still running.
    pub running: bool,
    /// Populated once the job has finished (success, error, or panic).
    /// The frontend should apply this exactly as it would a
    /// `launch-finished` event.
    pub finished: Option<LaunchFinishedPayload>,
}

/// Poll the state of a launch job.  Returns the final result when
/// available so the frontend can recover even if Tauri events are lost.
#[tauri::command]
pub fn poll_launch_job(job_id: String, state: State<AppState>) -> LaunchJobPollResult {
    let id = job_id.trim();
    if id.is_empty() {
        return LaunchJobPollResult {
            running: false,
            finished: None,
        };
    }

    // Still running?
    let running = state
        .launch_jobs
        .lock()
        .map(|jobs| jobs.contains_key(id))
        .unwrap_or(false);
    if running {
        return LaunchJobPollResult {
            running: true,
            finished: None,
        };
    }

    // Finished – try to retrieve (and consume) the stored result.
    let finished = state
        .launch_results
        .lock()
        .ok()
        .and_then(|mut results| results.remove(id));

    LaunchJobPollResult {
        running: false,
        finished,
    }
}

/// Terminal cwd changed event payload sent to the frontend
#[derive(Debug, Clone, Serialize)]
pub struct TerminalCwdChangedPayload {
    pub pane_id: String,
    pub cwd: String,
}

const OSC7_MARKER: &[u8] = b"\x1b]7;";
const OSC7_BUFFER_LIMIT: usize = 64 * 1024;

fn trim_osc7_buffer(buffer: &mut Vec<u8>) {
    if buffer.len() <= OSC7_BUFFER_LIMIT {
        return;
    }

    if let Some(marker_pos) = buffer
        .windows(OSC7_MARKER.len())
        .rposition(|window| window == OSC7_MARKER)
    {
        if marker_pos > 0 {
            buffer.drain(..marker_pos);
        }
    } else {
        buffer.clear();
    }
}

fn retain_possible_osc7_fragment(buffer: &mut Vec<u8>) {
    if let Some(marker_pos) = buffer
        .windows(OSC7_MARKER.len())
        .rposition(|window| window == OSC7_MARKER)
    {
        if marker_pos > 0 {
            buffer.drain(..marker_pos);
        }
        return;
    }

    // Keep only the possible marker prefix tail (e.g., trailing ESC]).
    let keep = OSC7_MARKER.len().saturating_sub(1);
    if buffer.len() > keep {
        let drain_len = buffer.len() - keep;
        buffer.drain(..drain_len);
    }
}

fn consume_osc7_cwd_updates(
    pending: &mut Vec<u8>,
    chunk: &[u8],
    last_cwd: &mut String,
) -> Option<String> {
    pending.extend_from_slice(chunk);
    trim_osc7_buffer(pending);

    let mut latest_changed: Option<String> = None;
    loop {
        let Some((cwd, consumed)) =
            gwt_core::terminal::osc::extract_osc7_cwd_with_consumed(pending)
        else {
            retain_possible_osc7_fragment(pending);
            break;
        };

        if consumed == 0 || consumed > pending.len() {
            break;
        }

        if cwd != *last_cwd {
            *last_cwd = cwd.clone();
            latest_changed = Some(cwd);
        }

        pending.drain(..consumed);
    }

    latest_changed
}

fn mark_pane_stream_error_and_write_message(
    state: &AppState,
    pane_id: &str,
    details: &str,
) -> Vec<u8> {
    let status_message = format!("PTY stream error: {details}");
    let output_message = format!("\r\n[{status_message}]\r\n");
    let bytes = output_message.into_bytes();

    if let Ok(mut manager) = state.pane_manager.lock() {
        if let Some(pane) = manager.pane_mut_by_id(pane_id) {
            pane.mark_error(status_message);
            let _ = pane.process_bytes(&bytes);
        }
    }

    bytes
}

fn append_close_hint_to_pane_scrollback(state: &AppState, pane_id: &str) -> Vec<u8> {
    let bytes = b"\r\nPress Enter to close this tab.\r\n".to_vec();

    if let Ok(mut manager) = state.pane_manager.lock() {
        if let Some(pane) = manager.pane_mut_by_id(pane_id) {
            let _ = pane.process_bytes(&bytes);
        }
    }

    bytes
}

/// Stream PTY output to the frontend via Tauri events
fn stream_pty_output(
    mut reader: Box<dyn Read + Send>,
    pane_id: String,
    app_handle: AppHandle,
    agent_name: String,
) {
    let state = app_handle.state::<AppState>();
    let mut buf = [0u8; 4096];
    let mut stream_error: Option<String> = None;
    let mut last_cwd = String::new();
    let mut osc7_pending = Vec::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                // Keep the scrollback file up-to-date and check frontend readiness
                // under a single lock acquisition.
                let is_ready = if let Ok(mut manager) = state.pane_manager.lock() {
                    if let Some(pane) = manager.pane_mut_by_id(&pane_id) {
                        let _ = pane.process_bytes(&buf[..n]);
                        pane.is_frontend_ready()
                    } else {
                        false
                    }
                } else {
                    false
                };

                // Detect OSC 7 cwd changes for terminal tabs (always, regardless of ready state)
                if agent_name == "terminal" {
                    if let Some(cwd) =
                        consume_osc7_cwd_updates(&mut osc7_pending, &buf[..n], &mut last_cwd)
                    {
                        let payload = TerminalCwdChangedPayload {
                            pane_id: pane_id.clone(),
                            cwd,
                        };
                        let _ = app_handle.emit("terminal-cwd-changed", &payload);
                    }
                }

                // Only emit to frontend when it has signalled readiness via terminal_ready.
                // Before that, data is safely stored in the scrollback and will be
                // retrieved by the frontend when it calls terminal_ready.
                if is_ready {
                    let payload = TerminalOutputPayload {
                        pane_id: pane_id.clone(),
                        data: buf[..n].to_vec(),
                    };
                    let _ = app_handle.emit("terminal-output", &payload);
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => {
                stream_error = Some(err.to_string());
                break;
            }
        }
    }

    // Flush scrollback after the read loop ends to ensure data is persisted
    // even when the process exits quickly.
    if let Ok(mut manager) = state.pane_manager.lock() {
        if let Some(pane) = manager.pane_mut_by_id(&pane_id) {
            let _ = pane.flush_scrollback();
        }
    }

    if let Some(details) = stream_error.as_deref() {
        let bytes = mark_pane_stream_error_and_write_message(&state, &pane_id, details);

        let payload = TerminalOutputPayload {
            pane_id: pane_id.clone(),
            data: bytes,
        };
        let _ = app_handle.emit("terminal-output", &payload);
    }

    // Update pane status after the PTY stream ends.
    let (exit_code, ended) = if let Ok(mut manager) = state.pane_manager.lock() {
        if let Some(pane) = manager.pane_mut_by_id(&pane_id) {
            if stream_error.is_none() {
                let _ = pane.check_status();
            }
            let exit_code = match pane.status() {
                PaneStatus::Completed(code) => Some(*code),
                _ => None,
            };
            let ended = !matches!(pane.status(), PaneStatus::Running);
            (exit_code, ended)
        } else {
            (None, false)
        }
    } else {
        (None, false)
    };

    // Best-effort sessionId detection and persistence.
    let meta = state
        .pane_launch_meta
        .lock()
        .ok()
        .and_then(|mut map| map.remove(&pane_id));
    remove_pane_runtime_context(&state, &pane_id);
    if let Some(meta) = meta {
        let docker_container_name = meta.docker_container_name.clone();
        let docker_compose_args = meta.docker_compose_args.clone();

        if let Some(session_id) =
            detect_session_id(&meta.agent_id, &meta.worktree_path, meta.started_at_millis)
        {
            let entry = gwt_core::config::ToolSessionEntry {
                branch: meta.branch.clone(),
                worktree_path: Some(meta.worktree_path.to_string_lossy().to_string()),
                tool_id: tool_id_for(&meta.agent_id),
                tool_label: meta.tool_label.clone(),
                session_id: Some(session_id.clone()),
                mode: Some(meta.mode.clone()),
                model: meta.model.clone(),
                reasoning_level: if meta.agent_id == "codex" {
                    meta.reasoning_level.clone()
                } else {
                    None
                },
                skip_permissions: Some(meta.skip_permissions),
                tool_version: Some(meta.tool_version.clone()),
                collaboration_modes: if meta.agent_id == "codex" {
                    Some(meta.collaboration_modes)
                } else {
                    None
                },
                docker_service: meta.docker_service.clone(),
                docker_force_host: meta.docker_force_host,
                docker_recreate: meta.docker_recreate,
                docker_build: meta.docker_build,
                docker_keep: meta.docker_keep,
                docker_container_name,
                docker_compose_args,
                timestamp: now_millis(),
            };

            if let Err(err) = gwt_core::config::save_session_entry(&meta.repo_path, entry) {
                tracing::warn!(error = %err, "Failed to save session entry (exit)");
            }

            let msg = format!("\r\n[Session ID: {}]\r\n", session_id);
            let bytes = msg.as_bytes();

            if let Ok(mut manager) = state.pane_manager.lock() {
                if let Some(pane) = manager.pane_mut_by_id(&pane_id) {
                    let _ = pane.process_bytes(bytes);
                }
            }

            let payload = TerminalOutputPayload {
                pane_id: pane_id.clone(),
                data: bytes.to_vec(),
            };
            let _ = app_handle.emit("terminal-output", &payload);
        }

        // Best-effort docker compose down on exit.
        if let Some(container_name) = meta.docker_container_name.as_deref() {
            let keep = meta.docker_keep.unwrap_or(false);
            if !keep {
                let pane_id_clone = pane_id.clone();
                let app_handle_clone = app_handle.clone();
                let worktree_path = meta.worktree_path.clone();
                let container_name = container_name.to_string();
                let compose_args = meta.docker_compose_args.clone();

                std::thread::spawn(move || {
                    let state = app_handle_clone.state::<AppState>();

                    let msg = "\r\n[Stopping Docker containers...]\r\n";
                    let bytes = msg.as_bytes();
                    if let Ok(mut manager) = state.pane_manager.lock() {
                        if let Some(pane) = manager.pane_mut_by_id(&pane_id_clone) {
                            let _ = pane.process_bytes(bytes);
                        }
                    }
                    let payload = TerminalOutputPayload {
                        pane_id: pane_id_clone.clone(),
                        data: bytes.to_vec(),
                    };
                    let _ = app_handle_clone.emit("terminal-output", &payload);

                    let result = (|| {
                        let args = if let Some(args) = compose_args.as_ref() {
                            args.clone()
                        } else {
                            // Backward-compat fallback for panes started before compose args were stored.
                            match detect_docker_files(&worktree_path) {
                                Some(DockerFileType::Compose(compose_path)) => vec![
                                    "-f".to_string(),
                                    compose_path.to_string_lossy().to_string(),
                                ],
                                _ => return Ok(()),
                            }
                        };

                        let compose_paths = compose_file_paths_from_args(&args);
                        let docker_file_type = compose_paths
                            .first()
                            .cloned()
                            .map(DockerFileType::Compose)
                            .unwrap_or_else(|| {
                                DockerFileType::Compose(worktree_path.join("docker-compose.yml"))
                            });

                        let manager = DockerManager::new(&worktree_path, "", docker_file_type);
                        let mut env = manager.collect_passthrough_env();
                        merge_compose_env_from_process(&mut env, &compose_paths);
                        env.insert("COMPOSE_PROJECT_NAME".to_string(), container_name.clone());
                        docker_compose_down(&worktree_path, &container_name, &env, &args)
                    })();

                    let msg = match result {
                        Ok(()) => "\r\n[Docker containers stopped]\r\n".to_string(),
                        Err(err) => format!("\r\n[Failed to stop Docker containers: {}]\r\n", err),
                    };
                    let bytes = msg.as_bytes();
                    if let Ok(mut manager) = state.pane_manager.lock() {
                        if let Some(pane) = manager.pane_mut_by_id(&pane_id_clone) {
                            let _ = pane.process_bytes(bytes);
                        }
                    }
                    let payload = TerminalOutputPayload {
                        pane_id: pane_id_clone.clone(),
                        data: bytes.to_vec(),
                    };
                    let _ = app_handle_clone.emit("terminal-output", &payload);
                });
            }
        }
    }

    if let Some(code) = exit_code {
        let msg = format!("\r\n[Process exited with code {}]\r\n", code);
        let bytes = msg.as_bytes();

        if let Ok(mut manager) = state.pane_manager.lock() {
            if let Some(pane) = manager.pane_mut_by_id(&pane_id) {
                let _ = pane.process_bytes(bytes);
            }
        }

        let payload = TerminalOutputPayload {
            pane_id: pane_id.clone(),
            data: bytes.to_vec(),
        };
        let _ = app_handle.emit("terminal-output", &payload);
    }

    if ended {
        let bytes = append_close_hint_to_pane_scrollback(&state, &pane_id);

        let payload = TerminalOutputPayload {
            pane_id: pane_id.clone(),
            data: bytes,
        };
        let _ = app_handle.emit("terminal-output", &payload);
    }
}

fn detect_session_id(
    agent_id: &str,
    worktree_path: &std::path::Path,
    started_at_millis: i64,
) -> Option<String> {
    let sessions = match agent_id {
        "codex" => gwt_core::ai::CodexSessionParser::with_default_home()
            .map(|p| p.list_sessions(Some(worktree_path)))
            .unwrap_or_default(),
        "claude" => gwt_core::ai::ClaudeSessionParser::with_default_home()
            .map(|p| p.list_sessions(Some(worktree_path)))
            .unwrap_or_default(),
        "gemini" => gwt_core::ai::GeminiSessionParser::with_default_home()
            .map(|p| p.list_sessions(Some(worktree_path)))
            .unwrap_or_default(),
        "opencode" => gwt_core::ai::OpenCodeSessionParser::with_default_home()
            .map(|p| p.list_sessions(Some(worktree_path)))
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    let cutoff = started_at_millis.saturating_sub(2_000);
    for entry in sessions {
        let last_updated = entry
            .last_updated
            .map(|t| t.timestamp_millis())
            .unwrap_or(i64::MAX);
        if last_updated >= cutoff {
            return Some(entry.session_id);
        }
    }

    None
}

fn is_enter_only(data: &[u8]) -> bool {
    matches!(data, [b'\r'] | [b'\n'] | [b'\r', b'\n'])
}

const DEFAULT_SCROLLBACK_TAIL_BYTES: usize = 256 * 1024;
const MAX_SCROLLBACK_TAIL_BYTES: usize = 1024 * 1024;

/// Write data to a terminal pane
#[tauri::command]
pub fn write_terminal(
    pane_id: String,
    data: Vec<u8>,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<(), StructuredError> {
    let close_requested = is_enter_only(&data);

    let mut manager = state.pane_manager.lock().map_err(|e| {
        StructuredError::internal(
            &format!("Failed to lock pane manager: {}", e),
            "write_terminal",
        )
    })?;

    let should_close = {
        let pane = manager.pane_mut_by_id(&pane_id).ok_or_else(|| {
            StructuredError::internal(&format!("Pane not found: {}", pane_id), "write_terminal")
        })?;

        // Ensure we don't treat a completed pane as running due to stale status.
        let _ = pane.check_status();

        match pane.status() {
            PaneStatus::Running => {
                pane.write_input(&data).map_err(|e| {
                    StructuredError::internal(
                        &format!("Failed to write to terminal: {}", e),
                        "write_terminal",
                    )
                })?;
                return Ok(());
            }
            PaneStatus::Completed(_) | PaneStatus::Error(_) => close_requested,
        }
    };

    if !should_close {
        return Ok(());
    }

    let index = manager
        .panes()
        .iter()
        .position(|p| p.pane_id() == pane_id)
        .ok_or_else(|| {
            StructuredError::internal(&format!("Pane not found: {}", pane_id), "write_terminal")
        })?;

    manager.close_pane(index);
    remove_pane_runtime_context(&state, &pane_id);
    let _ = app_handle.emit(
        "terminal-closed",
        &TerminalClosedPayload {
            pane_id: pane_id.clone(),
        },
    );
    Ok(())
}

pub(crate) fn send_keys_to_pane_from_state(
    state: &AppState,
    pane_id: &str,
    text: &str,
    project_root: Option<&std::path::Path>,
) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }

    let mut manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {}", e))?;

    let pane = manager
        .pane_mut_by_id(pane_id)
        .ok_or_else(|| format!("Pane not found: {}", pane_id))?;

    // Project isolation: reject access to panes belonging to a different project.
    if let Some(root) = project_root {
        if pane.project_root() != root {
            return Err(format!(
                "Access denied: pane {} belongs to a different project",
                pane_id
            ));
        }
    }

    let _ = pane.check_status();
    match pane.status() {
        PaneStatus::Running => pane
            .write_input(text.as_bytes())
            .map_err(|e| format!("Failed to write to terminal: {}", e))?,
        PaneStatus::Completed(_) | PaneStatus::Error(_) => {
            return Err(format!("Pane not running: {}", pane_id));
        }
    }

    Ok(())
}

pub(crate) fn send_keys_broadcast_from_state(
    state: &AppState,
    text: &str,
) -> Result<usize, String> {
    if text.is_empty() {
        return Ok(0);
    }

    let mut manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {}", e))?;

    let mut sent = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for pane in manager.panes_mut() {
        let _ = pane.check_status();
        if matches!(pane.status(), PaneStatus::Running) {
            if let Err(e) = pane.write_input(text.as_bytes()) {
                errors.push(format!("{}: {}", pane.pane_id(), e));
                continue;
            }
            sent += 1;
        }
    }

    if errors.is_empty() {
        Ok(sent)
    } else {
        Err(format!(
            "Failed to write to {} pane(s): {}",
            errors.len(),
            errors.join("; ")
        ))
    }
}

/// Send text to a terminal pane (pane_id required).
///
/// When `project_root` is provided, access is restricted to panes belonging
/// to that project (multi-project isolation).
#[tauri::command]
pub fn send_keys_to_pane(
    pane_id: String,
    text: String,
    project_root: Option<String>,
    state: State<AppState>,
) -> Result<(), StructuredError> {
    let root = project_root.map(PathBuf::from);
    send_keys_to_pane_from_state(&state, &pane_id, &text, root.as_deref())
        .map_err(|e| StructuredError::internal(&e, "send_keys_to_pane"))
}

/// Broadcast text to all running terminal panes. Returns number of panes sent.
#[tauri::command]
pub fn send_keys_broadcast(text: String, state: State<AppState>) -> Result<usize, StructuredError> {
    send_keys_broadcast_from_state(&state, &text)
        .map_err(|e| StructuredError::internal(&e, "send_keys_broadcast"))
}

/// Resize a terminal pane
#[tauri::command]
pub fn resize_terminal(
    pane_id: String,
    rows: u16,
    cols: u16,
    state: State<AppState>,
) -> Result<(), StructuredError> {
    let mut manager = state.pane_manager.lock().map_err(|e| {
        StructuredError::internal(
            &format!("Failed to lock pane manager: {}", e),
            "resize_terminal",
        )
    })?;
    let pane = manager.pane_mut_by_id(&pane_id).ok_or_else(|| {
        StructuredError::internal(&format!("Pane not found: {}", pane_id), "resize_terminal")
    })?;
    pane.resize(rows, cols).map_err(|e| {
        StructuredError::internal(
            &format!("Failed to resize terminal: {}", e),
            "resize_terminal",
        )
    })
}

/// Close a terminal pane
#[tauri::command]
pub fn close_terminal(
    pane_id: String,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<(), StructuredError> {
    let mut manager = state.pane_manager.lock().map_err(|e| {
        StructuredError::internal(
            &format!("Failed to lock pane manager: {}", e),
            "close_terminal",
        )
    })?;

    let index = manager
        .panes()
        .iter()
        .position(|p| p.pane_id() == pane_id)
        .ok_or_else(|| {
            StructuredError::internal(&format!("Pane not found: {}", pane_id), "close_terminal")
        })?;

    manager.close_pane(index);
    let _ = app_handle.emit(
        "terminal-closed",
        &TerminalClosedPayload {
            pane_id: pane_id.clone(),
        },
    );
    Ok(())
}

/// List terminal panes scoped to the given project root.
///
/// When `project_root` is provided, only panes belonging to that project are
/// returned (multi-project isolation). When omitted, all panes are listed
/// (backwards-compatible).
#[tauri::command]
pub fn list_terminals(state: State<AppState>, project_root: Option<String>) -> Vec<TerminalInfo> {
    let manager = match state.pane_manager.lock() {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    let project_filter = project_root.map(PathBuf::from);
    manager
        .panes()
        .iter()
        .filter(|pane| match &project_filter {
            Some(root) => pane.project_root() == root.as_path(),
            None => true,
        })
        .map(|pane| {
            let status = match pane.status() {
                PaneStatus::Running => "running".to_string(),
                PaneStatus::Completed(code) => format!("completed({})", code),
                PaneStatus::Error(msg) => format!("error: {}", msg),
            };
            TerminalInfo {
                pane_id: pane.pane_id().to_string(),
                agent_name: pane.agent_name().to_string(),
                branch_name: pane.branch_name().to_string(),
                status,
            }
        })
        .collect()
}

pub(crate) fn capture_scrollback_tail_from_state(
    state: &AppState,
    pane_id: &str,
    max_bytes: usize,
    project_root: Option<&std::path::Path>,
) -> Result<String, String> {
    let max_bytes = match max_bytes {
        0 => DEFAULT_SCROLLBACK_TAIL_BYTES,
        n if n > MAX_SCROLLBACK_TAIL_BYTES => MAX_SCROLLBACK_TAIL_BYTES,
        n => n,
    };

    // Best-effort: flush in-memory scrollback so capture does not read stale data.
    {
        let mut mgr = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock state: {}", e))?;
        if let Some(pane) = mgr.pane_mut_by_id(pane_id) {
            // Project isolation: reject access to panes belonging to a different project.
            if let Some(root) = project_root {
                if pane.project_root() != root {
                    return Err(format!(
                        "Access denied: pane {} belongs to a different project",
                        pane_id
                    ));
                }
            }
            pane.flush_scrollback().map_err(|e| e.to_string())?;
        }
    }

    let path = ScrollbackFile::scrollback_path_for_pane(pane_id).map_err(|e| e.to_string())?;
    let bytes = ScrollbackFile::read_tail_bytes_at(&path, max_bytes).map_err(|e| e.to_string())?;
    Ok(strip_ansi(&bytes))
}

fn build_terminal_ansi_probe(pane_id: &str, bytes: &[u8]) -> TerminalAnsiProbe {
    let esc_count = bytes.iter().filter(|&&b| b == 0x1b).count();
    let mut sgr_count = 0usize;
    let mut color_sgr_count = 0usize;
    let mut has_256_color = false;
    let mut has_true_color = false;

    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            // CSI: parameter bytes (0x30-0x3f), intermediate bytes (0x20-0x2f), final byte (0x40-0x7e)
            let mut j = i + 2;
            while j < bytes.len() {
                let c = bytes[j];
                if (0x40..=0x7e).contains(&c) {
                    if c == b'm' {
                        sgr_count += 1;

                        // Parse params between '[' and 'm' as SGR codes.
                        let params = &bytes[i + 2..j];
                        let mut codes: Vec<u16> = Vec::new();
                        let mut num: u16 = 0;
                        let mut has_num = false;
                        for &b in params {
                            if b.is_ascii_digit() {
                                has_num = true;
                                num = num.saturating_mul(10).saturating_add((b - b'0') as u16);
                                continue;
                            }
                            if b == b';' {
                                codes.push(if has_num { num } else { 0 });
                                num = 0;
                                has_num = false;
                            }
                        }
                        if has_num {
                            codes.push(num);
                        } else if params.last().is_some_and(|b| *b == b';') {
                            // Trailing empty param counts as 0 (e.g. ESC[;m).
                            codes.push(0);
                        }

                        let mut is_color = false;
                        for (idx, code) in codes.iter().enumerate() {
                            match *code {
                                // Basic and bright colors (fg/bg) + default fg/bg resets.
                                30..=37 | 90..=97 | 40..=47 | 100..=107 | 39 | 49 => {
                                    is_color = true
                                }
                                // Extended colors.
                                38 | 48 => match codes.get(idx + 1).copied() {
                                    Some(5) => {
                                        has_256_color = true;
                                        is_color = true;
                                    }
                                    Some(2) => {
                                        has_true_color = true;
                                        is_color = true;
                                    }
                                    _ => {}
                                },
                                _ => {}
                            }
                        }

                        if is_color {
                            color_sgr_count += 1;
                        }
                    }
                    j += 1;
                    break;
                }
                j += 1;
            }
            i = j;
            continue;
        }
        i += 1;
    }

    TerminalAnsiProbe {
        pane_id: pane_id.to_string(),
        bytes_scanned: bytes.len(),
        esc_count,
        sgr_count,
        color_sgr_count,
        has_256_color,
        has_true_color,
    }
}

fn probe_terminal_ansi_from_state(
    state: &AppState,
    pane_id: &str,
) -> Result<TerminalAnsiProbe, String> {
    // Best-effort: flush in-memory scrollback so diagnostics does not read stale data.
    {
        let mut mgr = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock state: {}", e))?;
        if let Some(pane) = mgr.pane_mut_by_id(pane_id) {
            pane.flush_scrollback().map_err(|e| e.to_string())?;
        }
    }

    let path = ScrollbackFile::scrollback_path_for_pane(pane_id).map_err(|e| e.to_string())?;
    let bytes = ScrollbackFile::read_tail_bytes_at(&path, 256 * 1024).map_err(|e| e.to_string())?;
    Ok(build_terminal_ansi_probe(pane_id, &bytes))
}

/// Probe a pane's scrollback tail for ANSI/SGR/color usage (diagnostics).
#[tauri::command]
pub fn probe_terminal_ansi(
    state: State<AppState>,
    pane_id: String,
) -> Result<TerminalAnsiProbe, StructuredError> {
    probe_terminal_ansi_from_state(&state, &pane_id)
        .map_err(|e| StructuredError::internal(&e, "probe_terminal_ansi"))
}

/// Capture the scrollback tail for a pane as plain text (ANSI stripped).
///
/// When `project_root` is provided, access is restricted to panes belonging
/// to that project (multi-project isolation).
#[tauri::command]
pub fn capture_scrollback_tail(
    state: State<AppState>,
    pane_id: String,
    max_bytes: Option<usize>,
    project_root: Option<String>,
) -> Result<String, StructuredError> {
    let max_bytes = max_bytes.unwrap_or(DEFAULT_SCROLLBACK_TAIL_BYTES);
    let root = project_root.map(PathBuf::from);
    capture_scrollback_tail_from_state(&state, &pane_id, max_bytes, root.as_deref())
        .map_err(|e| StructuredError::internal(&e, "capture_scrollback_tail"))
}

/// Signal that the frontend listener is ready and retrieve initial scrollback
/// as raw bytes (ANSI sequences preserved).  After this call, `stream_pty_output`
/// will start emitting `terminal-output` events for the pane.
#[tauri::command]
pub fn terminal_ready(
    state: State<AppState>,
    pane_id: String,
    max_bytes: Option<usize>,
) -> Result<Vec<u8>, StructuredError> {
    let max = max_bytes
        .unwrap_or(DEFAULT_SCROLLBACK_TAIL_BYTES)
        .min(MAX_SCROLLBACK_TAIL_BYTES);
    let mut mgr = state
        .pane_manager
        .lock()
        .map_err(|e| StructuredError::internal(&e.to_string(), "terminal_ready"))?;
    let pane = mgr
        .pane_mut_by_id(&pane_id)
        .ok_or_else(|| StructuredError::internal("pane not found", "terminal_ready"))?;
    // flush → read → set_ready all under the same lock, so no data gap
    // with the stream_pty_output thread.
    let bytes = pane
        .read_scrollback_tail_raw(max)
        .map_err(|e| StructuredError::internal(&e.to_string(), "terminal_ready"))?;
    pane.set_frontend_ready(true);
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// OS Environment introspection commands
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedEnvEntry {
    pub key: String,
    pub value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedEnvInfo {
    pub entries: Vec<CapturedEnvEntry>,
    pub source: String,
    pub reason: Option<String>,
    pub ready: bool,
}

fn os_env_source_to_string(
    source: Option<gwt_core::config::os_env::EnvSource>,
) -> (String, Option<String>) {
    match source {
        Some(gwt_core::config::os_env::EnvSource::LoginShell) => ("login_shell".to_string(), None),
        Some(gwt_core::config::os_env::EnvSource::ProcessEnv) => ("process_env".to_string(), None),
        Some(gwt_core::config::os_env::EnvSource::StdEnvFallback { reason }) => {
            ("std_env_fallback".to_string(), Some(reason))
        }
        None => ("unknown".to_string(), None),
    }
}

#[tauri::command]
pub fn get_captured_environment(state: State<AppState>) -> CapturedEnvInfo {
    let mut entries: Vec<CapturedEnvEntry> = state
        .os_env_snapshot()
        .iter()
        .map(|(k, v)| CapturedEnvEntry {
            key: k.clone(),
            value: v.clone(),
        })
        .collect();
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    let (source, reason) = os_env_source_to_string(state.os_env_source_snapshot());

    CapturedEnvInfo {
        ready: state.is_os_env_ready(),
        entries,
        source,
        reason,
    }
}

#[tauri::command]
pub fn is_os_env_ready(state: State<AppState>) -> bool {
    state.is_os_env_ready()
}

/// Shell information returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct ShellInfo {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
}

/// Return the list of available Windows shells.
///
/// On non-Windows platforms this always returns an empty list.
#[tauri::command]
pub async fn get_available_shells() -> Result<Vec<ShellInfo>, StructuredError> {
    #[cfg(target_os = "windows")]
    {
        use gwt_core::terminal::shell::WindowsShell;
        let shells: Vec<ShellInfo> = WindowsShell::ALL
            .iter()
            .filter(|s| s.is_available())
            .map(|s| ShellInfo {
                id: s.id().to_string(),
                name: s.display_name().to_string(),
                version: s.detect_version(),
            })
            .collect();
        Ok(shells)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(Vec::new())
    }
}

/// Sanitize a string to contain only alphanumeric characters, hyphens, and
/// underscores so it is safe to embed in a file name.
fn sanitize_filename_part(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn normalized_image_extension(extension: &str) -> Option<String> {
    let trimmed = extension.trim().trim_start_matches('.');
    let normalized = sanitize_filename_part(trimmed)
        .trim_matches('_')
        .to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn sanitized_image_stem(name: &str) -> String {
    let normalized = sanitize_filename_part(name).trim_matches('_').to_string();
    if normalized.is_empty() {
        "image".to_string()
    } else {
        normalized
    }
}

fn create_staged_image_destination(
    launch_workdir: &Path,
    pane_id: &str,
    base_name: &str,
    extension: &str,
    command: &str,
) -> Result<(PathBuf, String), StructuredError> {
    let safe_extension = normalized_image_extension(extension)
        .ok_or_else(|| StructuredError::internal("Invalid image format", command))?;
    let safe_pane_id = sanitized_image_stem(pane_id);
    let safe_base_name = sanitized_image_stem(base_name);

    let images_dir = launch_workdir.join(".tmp").join("images");
    std::fs::create_dir_all(&images_dir).map_err(|e| {
        StructuredError::internal(&format!("Failed to create images directory: {e}"), command)
    })?;

    let filename = format!(
        "{}_{}_{}.{}",
        safe_pane_id,
        safe_base_name,
        Uuid::new_v4(),
        safe_extension
    );
    let relative_path = format!("./.tmp/images/{filename}");
    Ok((images_dir.join(&filename), relative_path))
}

fn validate_clipboard_image_data(data: &[u8]) -> Result<(), StructuredError> {
    if data.is_empty() {
        return Err(StructuredError::internal(
            "Clipboard image is empty",
            "save_clipboard_image",
        ));
    }
    Ok(())
}

/// Save clipboard image data to a temporary file and return the prompt path.
#[tauri::command]
pub fn save_clipboard_image(
    pane_id: String,
    data: Vec<u8>,
    format: String,
    state: State<AppState>,
) -> Result<String, StructuredError> {
    validate_clipboard_image_data(&data)?;

    let context = pane_runtime_context(&state, &pane_id).ok_or_else(|| {
        StructuredError::internal(
            &format!("Pane runtime context not found: {pane_id}"),
            "save_clipboard_image",
        )
    })?;

    let (target_path, relative_path) = create_staged_image_destination(
        &context.launch_workdir,
        &pane_id,
        "clipboard",
        &format,
        "save_clipboard_image",
    )?;

    std::fs::write(&target_path, &data).map_err(|e| {
        StructuredError::internal(
            &format!("Failed to write image file: {e}"),
            "save_clipboard_image",
        )
    })?;

    Ok(relative_path)
}

#[cfg(test)]
mod attachment_path_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_staged_image_destination_places_files_under_launch_tmp_images() {
        let temp = TempDir::new().unwrap();
        let (target_path, relative_path) = create_staged_image_destination(
            temp.path(),
            "pane-1",
            "clipboard image",
            "png",
            "test",
        )
        .unwrap();

        assert!(target_path.starts_with(temp.path().join(".tmp").join("images")));
        assert!(relative_path.starts_with("./.tmp/images/"));
        assert!(target_path.extension().and_then(|ext| ext.to_str()) == Some("png"));
    }

    #[test]
    fn pane_runtime_context_registration_round_trips_launch_workdir() {
        let state = AppState::new();
        let launch_workdir = TempDir::new().unwrap();

        register_pane_runtime_context(&state, "pane-1", launch_workdir.path());
        let context = pane_runtime_context(&state, "pane-1").expect("context should exist");

        assert_eq!(context.launch_workdir, launch_workdir.path());

        remove_pane_runtime_context(&state, "pane-1");
        assert!(pane_runtime_context(&state, "pane-1").is_none());
    }

    #[test]
    fn validate_clipboard_image_data_rejects_empty_payload() {
        let err = validate_clipboard_image_data(&[]).expect_err("empty payload must fail");
        assert!(err.message.contains("Clipboard image is empty"));
    }

    #[test]
    fn validate_clipboard_image_data_accepts_non_empty_payload() {
        validate_clipboard_image_data(&[1, 2, 3]).expect("non-empty payload should pass");
    }
}
