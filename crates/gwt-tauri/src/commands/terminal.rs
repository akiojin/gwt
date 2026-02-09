//! Terminal/PTY management commands for xterm.js integration

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::state::{AppState, PaneLaunchMeta};
use gwt_core::ai::SessionParser;
use gwt_core::config::{ProfilesConfig, Settings};
use gwt_core::docker::{
    compose_available, daemon_running, detect_docker_files, docker_available, try_start_daemon,
    DockerFileType, DockerManager,
};
use gwt_core::git::Remote;
use gwt_core::terminal::pane::PaneStatus;
use gwt_core::terminal::{AgentColor, BuiltinLaunchConfig};
use gwt_core::worktree::WorktreeManager;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};
use which::which;

/// Terminal output event payload sent to the frontend
#[derive(Debug, Clone, Serialize)]
pub struct TerminalOutputPayload {
    pub pane_id: String,
    pub data: Vec<u8>,
}

/// Serializable terminal info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct TerminalInfo {
    pub pane_id: String,
    pub agent_name: String,
    pub branch_name: String,
    pub status: String,
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
    /// - `None`: auto (prefer installed command when present, else bunx/npx)
    /// - `"installed"`: force installed command (falls back to bunx/npx when missing)
    /// - other: force bunx/npx package `@...@{version}` (e.g. "latest", "1.2.3")
    pub agent_version: Option<String>,
    /// Optional session mode override (default: normal).
    pub mode: Option<SessionMode>,
    /// Skip permissions / approvals (agent-specific; default: false).
    pub skip_permissions: Option<bool>,
    /// Codex reasoning override (e.g. "low", "medium", "high", "xhigh").
    pub reasoning_level: Option<String>,
    /// Enable collaboration_modes for Codex (default: false).
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

fn resolve_worktree_path(repo_path: &std::path::Path, branch_ref: &str) -> Result<PathBuf, String> {
    let manager = WorktreeManager::new(repo_path).map_err(|e| e.to_string())?;

    let remotes = Remote::list(repo_path).unwrap_or_default();
    let normalized = strip_known_remote_prefix(branch_ref, &remotes);

    if let Ok(Some(wt)) = manager.get_by_branch_basic(normalized) {
        return Ok(wt.path);
    }
    // Rare: worktree registered with the raw remote-like name.
    if normalized != branch_ref {
        if let Ok(Some(wt)) = manager.get_by_branch_basic(branch_ref) {
            return Ok(wt.path);
        }
    }

    let wt = manager
        .create_for_branch(branch_ref)
        .map_err(|e| e.to_string())?;
    Ok(wt.path)
}

fn create_new_worktree_path(
    repo_path: &std::path::Path,
    branch_name: &str,
    base_branch: Option<&str>,
) -> Result<PathBuf, String> {
    let manager = WorktreeManager::new(repo_path).map_err(|e| e.to_string())?;
    let wt = manager
        .create_new_branch(branch_name, base_branch)
        .map_err(|e| e.to_string())?;
    Ok(wt.path)
}

fn load_profile_env(profile_override: Option<&str>) -> HashMap<String, String> {
    let Ok(config) = ProfilesConfig::load() else {
        return HashMap::new();
    };

    let profile_name = profile_override
        .map(|s| s.to_string())
        .or_else(|| config.active.clone());

    let Some(name) = profile_name else {
        return HashMap::new();
    };

    config
        .profiles
        .get(&name)
        .map(|p| p.env.clone())
        .unwrap_or_default()
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
        _ => Err(format!("Unknown agent: {}", agent_id)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FallbackRunner {
    Bunx,
    Npx,
}

pub(crate) fn is_node_modules_bin(path: &str) -> bool {
    // Cross-platform substring match is fine here; this is only used to avoid
    // project-local bunx/npx wrappers per SPEC-3b0ed29b FR-002a.
    path.contains("node_modules/.bin") || path.contains("node_modules\\.bin")
}

pub(crate) fn choose_fallback_runner(
    _bunx_path: Option<&str>,
    _npx_available: bool,
) -> Option<FallbackRunner> {
    match _bunx_path {
        Some(path) if !is_node_modules_bin(path) => Some(FallbackRunner::Bunx),
        Some(_) if _npx_available => Some(FallbackRunner::Npx),
        _ => None,
    }
}

pub(crate) fn build_fallback_launch(
    runner: FallbackRunner,
    package: &str,
) -> (String, Vec<String>) {
    match runner {
        FallbackRunner::Bunx => ("bunx".to_string(), vec![package.to_string()]),
        FallbackRunner::Npx => (
            "npx".to_string(),
            vec!["--yes".to_string(), package.to_string()],
        ),
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
        // SPEC-3b0ed29b FR-005: Codex uses `--model=...`.
        "codex" => vec![format!("--model={model}")],
        // SPEC-3b0ed29b: Claude Code uses `--model <name>`.
        "claude" => vec!["--model".to_string(), model.to_string()],
        // SPEC-3b0ed29b: Gemini CLI uses `-m <name>`.
        "gemini" => vec!["-m".to_string(), model.to_string()],
        // SPEC-3b0ed29b: OpenCode uses `-m provider/model`.
        "opencode" => vec!["-m".to_string(), model.to_string()],
        _ => Vec::new(),
    }
}

fn get_command_version_with_timeout(command: &str) -> Option<String> {
    let command = command.to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = std::process::Command::new(command)
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
struct ResolvedAgentLaunchCommand {
    command: String,
    args: Vec<String>,
    label: &'static str,
    tool_version: String, // "installed" | "latest" | "1.2.3" | dist-tag
    version_for_gates: Option<String>, // best-effort raw version string (may be "latest")
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

    // Force npm runner when a specific version/dist-tag is provided.
    if let Some(v) = requested.as_deref() {
        if !requested_is_installed {
            let bunx_path = which("bunx").ok().map(|p| p.to_string_lossy().to_string());
            let npx_available = which("npx").is_ok();

            let runner = choose_fallback_runner(bunx_path.as_deref(), npx_available)
                .ok_or_else(|| "bunx/npx is not available".to_string())?;
            let package = build_bunx_package_spec(def.bunx_package, Some(v));
            let (cmd, args) = build_fallback_launch(runner, &package);
            return Ok(ResolvedAgentLaunchCommand {
                command: cmd.clone(),
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
            command: def.local_command.to_string(),
            args: Vec::new(),
            label: def.label,
            tool_version: "installed".to_string(),
            version_for_gates: version_raw,
        });
    }

    // Installed was requested but missing: fall back to npm runner with latest.
    // Auto mode also lands here when installed is missing.
    let bunx_path = which("bunx").ok().map(|p| p.to_string_lossy().to_string());
    let npx_available = which("npx").is_ok();

    let runner = choose_fallback_runner(bunx_path.as_deref(), npx_available)
        .ok_or_else(|| "Agent is not installed and bunx/npx is not available".to_string())?;

    let package = build_bunx_package_spec(def.bunx_package, None);
    let (cmd, args) = build_fallback_launch(runner, &package);
    let tool_version = if requested_is_installed {
        "installed".to_string()
    } else {
        "latest".to_string()
    };
    Ok(ResolvedAgentLaunchCommand {
        command: cmd,
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
            command: def.local_command.to_string(),
            args: Vec::new(),
            label: def.label,
            tool_version: "installed".to_string(),
            version_for_gates: None,
        });
    }

    // Container execution is resolved without host-side command detection.
    // Prefer `npx --yes` for portability (bun is not guaranteed in containers).
    let version = requested.unwrap_or_else(|| "latest".to_string());
    let package = build_bunx_package_spec(def.bunx_package, Some(version.as_str()));

    Ok(ResolvedAgentLaunchCommand {
        command: "npx".to_string(),
        args: vec!["--yes".to_string(), package],
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
        _ => AgentColor::White,
    }
}

fn tool_id_for(agent_id: &str) -> String {
    match agent_id {
        "claude" => "claude-code".to_string(),
        "codex" => "codex-cli".to_string(),
        "gemini" => "gemini-cli".to_string(),
        "opencode" => "opencode".to_string(),
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

const DOCKER_WORKDIR: &str = "/workspace";

fn build_docker_compose_up_args(build: bool, recreate: bool) -> Vec<String> {
    let mut args = vec![
        "compose".to_string(),
        "up".to_string(),
        "-d".to_string(),
        if build {
            "--build".to_string()
        } else {
            "--no-build".to_string()
        },
    ];
    if recreate {
        args.push("--force-recreate".to_string());
    }
    args
}

fn build_docker_compose_down_args() -> Vec<String> {
    vec!["compose".to_string(), "down".to_string()]
}

fn build_docker_compose_exec_args(
    service: &str,
    workdir: &str,
    env_vars: &HashMap<String, String>,
    inner_command: &str,
    inner_args: &[String],
) -> Vec<String> {
    let mut args = vec![
        "compose".to_string(),
        "exec".to_string(),
        "-w".to_string(),
        workdir.to_string(),
    ];

    let mut keys: Vec<&String> = env_vars.keys().collect();
    keys.sort();
    for key in keys {
        let k = key.trim();
        if k.is_empty() {
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

fn docker_compose_up(
    worktree_path: &std::path::Path,
    container_name: &str,
    env_vars: &HashMap<String, String>,
    build: bool,
    recreate: bool,
) -> Result<(), String> {
    ensure_docker_compose_ready()?;

    let output = Command::new("docker")
        .args(build_docker_compose_up_args(build, recreate))
        .current_dir(worktree_path)
        .env("COMPOSE_PROJECT_NAME", container_name)
        .envs(env_vars)
        .output()
        .map_err(|e| format!("Failed to run docker compose up: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        return Err("docker compose up failed".to_string());
    }
    Err(stderr)
}

fn docker_compose_down(
    worktree_path: &std::path::Path,
    container_name: &str,
    env_vars: &HashMap<String, String>,
) -> Result<(), String> {
    ensure_docker_compose_ready()?;

    let output = Command::new("docker")
        .args(build_docker_compose_down_args())
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

fn codex_supports_collaboration_modes(version_for_gates: Option<&str>) -> bool {
    version_for_gates.is_some_and(|v| v.eq_ignore_ascii_case("latest"))
        || gwt_core::agent::codex::supports_collaboration_modes(version_for_gates)
}

fn build_agent_args(
    agent_id: &str,
    request: &LaunchAgentRequest,
    version_for_gates: Option<&str>,
) -> Result<Vec<String>, String> {
    let mode = request.mode.unwrap_or(SessionMode::Normal);
    let skip_permissions = request.skip_permissions.unwrap_or(false);
    let collaboration_requested = request.collaboration_modes.unwrap_or(false);
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

            let collaboration =
                collaboration_requested && codex_supports_collaboration_modes(version_for_gates);
            args.extend(gwt_core::agent::codex::codex_default_args(
                request.model.as_deref(),
                request.reasoning_level.as_deref(),
                version_for_gates,
                skip_permissions,
                collaboration,
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
        _ => {}
    }

    args.extend(extra_args);
    Ok(args)
}

fn launch_with_config(
    config: BuiltinLaunchConfig,
    meta: Option<PaneLaunchMeta>,
    state: &AppState,
    app_handle: AppHandle,
) -> Result<String, String> {
    let pane_id = {
        let mut manager = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock pane manager: {}", e))?;
        manager
            .launch_agent(config, 24, 80)
            .map_err(|e| format!("Failed to launch terminal: {}", e))?
    };

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
        stream_pty_output(reader, pane_id_clone, app_handle);
    });

    Ok(pane_id)
}

/// Launch a new terminal pane with an agent
#[tauri::command]
pub fn launch_terminal(
    agent_name: String,
    branch: String,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let project_root = {
        let project_path = state
            .project_path
            .lock()
            .map_err(|e| format!("Failed to lock state: {}", e))?;
        match project_path.as_ref() {
            Some(p) => PathBuf::from(p),
            None => return Err("No project opened".to_string()),
        }
    };

    let repo_path = resolve_repo_path_for_project_root(&project_root)?;
    let working_dir = resolve_worktree_path(&repo_path, &branch)?;

    let config = BuiltinLaunchConfig {
        command: agent_name.clone(),
        args: vec![],
        working_dir,
        branch_name: branch,
        agent_name,
        agent_color: AgentColor::Green,
        env_vars: HashMap::new(),
    };

    launch_with_config(config, None, &state, app_handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_node_modules_bin_matches_common_paths() {
        assert!(is_node_modules_bin("/repo/node_modules/.bin/bunx"));
        assert!(is_node_modules_bin("C:\\repo\\node_modules\\.bin\\bunx"));
        assert!(!is_node_modules_bin("/usr/local/bin/bunx"));
    }

    #[test]
    fn choose_fallback_runner_prefers_bunx_when_not_local() {
        assert_eq!(
            choose_fallback_runner(Some("/usr/local/bin/bunx"), true),
            Some(FallbackRunner::Bunx)
        );
    }

    #[test]
    fn choose_fallback_runner_uses_npx_when_bunx_is_local_node_modules() {
        assert_eq!(
            choose_fallback_runner(Some("/repo/node_modules/.bin/bunx"), true),
            Some(FallbackRunner::Npx)
        );
    }

    #[test]
    fn choose_fallback_runner_none_when_only_local_bunx_and_no_npx() {
        assert_eq!(
            choose_fallback_runner(Some("/repo/node_modules/.bin/bunx"), false),
            None
        );
    }

    #[test]
    fn build_fallback_launch_bunx_uses_package_as_first_arg() {
        let (cmd, args) = build_fallback_launch(FallbackRunner::Bunx, "@openai/codex@latest");
        assert_eq!(cmd, "bunx");
        assert_eq!(args, vec!["@openai/codex@latest".to_string()]);
    }

    #[test]
    fn build_fallback_launch_npx_uses_yes_flag() {
        let (cmd, args) = build_fallback_launch(FallbackRunner::Npx, "@openai/codex@latest");
        assert_eq!(cmd, "npx");
        assert_eq!(
            args,
            vec!["--yes".to_string(), "@openai/codex@latest".to_string()]
        );
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
        }
    }

    #[test]
    fn build_agent_args_codex_continue_defaults_to_resume_last() {
        let mut req = make_request("codex");
        req.mode = Some(SessionMode::Continue);
        let args = build_agent_args("codex", &req, Some("0.92.0")).unwrap();
        assert_eq!(args[0], "resume");
        assert_eq!(args[1], "--last");
        assert!(args.iter().any(|a| a.starts_with("--model=")));
    }

    #[test]
    fn build_agent_args_codex_collaboration_modes_allows_latest() {
        let mut req = make_request("codex");
        req.collaboration_modes = Some(true);
        let args = build_agent_args("codex", &req, Some("latest")).unwrap();
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--enable" && w[1] == "collaboration_modes"));
    }

    #[test]
    fn build_agent_args_codex_skip_flag_is_version_gated() {
        let mut req = make_request("codex");
        req.skip_permissions = Some(true);

        let legacy = build_agent_args("codex", &req, Some("0.79.9")).unwrap();
        assert!(legacy.iter().any(|a| a == "--yolo"));

        let modern = build_agent_args("codex", &req, Some("0.80.0")).unwrap();
        assert!(modern
            .iter()
            .any(|a| a == "--dangerously-bypass-approvals-and-sandbox"));
    }

    #[test]
    fn build_agent_args_claude_continue_prefers_resume_id_when_provided() {
        let mut req = make_request("claude");
        req.mode = Some(SessionMode::Continue);
        req.resume_session_id = Some("sess-123".to_string());
        let args = build_agent_args("claude", &req, None).unwrap();
        assert_eq!(args[0], "--resume");
        assert_eq!(args[1], "sess-123");
    }

    #[test]
    fn build_agent_args_claude_resume_without_id_opens_picker() {
        let mut req = make_request("claude");
        req.mode = Some(SessionMode::Resume);
        let args = build_agent_args("claude", &req, None).unwrap();
        assert_eq!(args[0], "--resume");
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn build_agent_args_gemini_continue_prefers_resume_id_when_provided() {
        let mut req = make_request("gemini");
        req.mode = Some(SessionMode::Continue);
        req.resume_session_id = Some("sess-123".to_string());
        let args = build_agent_args("gemini", &req, None).unwrap();
        assert_eq!(args, vec!["-r".to_string(), "sess-123".to_string()]);
    }

    #[test]
    fn build_agent_args_opencode_continue_prefers_resume_id_when_provided() {
        let mut req = make_request("opencode");
        req.mode = Some(SessionMode::Continue);
        req.resume_session_id = Some("sess-123".to_string());
        let args = build_agent_args("opencode", &req, None).unwrap();
        assert_eq!(args, vec!["-s".to_string(), "sess-123".to_string()]);
    }

    #[test]
    fn build_agent_args_opencode_resume_requires_session_id() {
        let mut req = make_request("opencode");
        req.mode = Some(SessionMode::Resume);
        let err = build_agent_args("opencode", &req, None).unwrap_err();
        assert!(err.to_lowercase().contains("session id"));
    }

    #[test]
    fn build_docker_compose_up_args_build_and_recreate_flags() {
        assert_eq!(
            build_docker_compose_up_args(false, false),
            vec![
                "compose".to_string(),
                "up".to_string(),
                "-d".to_string(),
                "--no-build".to_string(),
            ]
        );

        let build = build_docker_compose_up_args(true, false);
        assert!(build.contains(&"--build".to_string()));
        assert!(!build.contains(&"--no-build".to_string()));

        let recreate = build_docker_compose_up_args(false, true);
        assert!(recreate.contains(&"--force-recreate".to_string()));
    }

    #[test]
    fn build_docker_compose_exec_args_sorts_env_and_appends_inner_command() {
        let mut env = HashMap::new();
        env.insert("B".to_string(), "2".to_string());
        env.insert("A".to_string(), "1".to_string());

        let inner_args = vec!["--yes".to_string(), "pkg@latest".to_string()];
        let args = build_docker_compose_exec_args("app", "/workspace", &env, "npx", &inner_args);

        let pos_a = args.iter().position(|s| s == "A=1").unwrap();
        let pos_b = args.iter().position(|s| s == "B=2").unwrap();
        assert!(pos_a < pos_b);

        let pos_service = args.iter().position(|s| s == "app").unwrap();
        let pos_cmd = args.iter().position(|s| s == "npx").unwrap();
        assert!(pos_service < pos_cmd);

        assert!(args.ends_with(&inner_args));
    }
}

/// Launch an agent with gwt semantics (worktree + profiles)
#[tauri::command]
pub fn launch_agent(
    request: LaunchAgentRequest,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let project_root = {
        let project_path = state
            .project_path
            .lock()
            .map_err(|e| format!("Failed to lock state: {}", e))?;
        match project_path.as_ref() {
            Some(p) => PathBuf::from(p),
            None => return Err("No project opened".to_string()),
        }
    };

    let agent_id = request.agent_id.trim();
    if agent_id.is_empty() {
        return Err("Agent is required".to_string());
    }
    let repo_path = resolve_repo_path_for_project_root(&project_root)?;

    let (working_dir, branch_name) = if let Some(create) = request.create_branch.as_ref() {
        let new_branch = create.name.trim();
        if new_branch.is_empty() {
            return Err("New branch name is required".to_string());
        }
        let base = create
            .base
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());

        (
            create_new_worktree_path(&repo_path, new_branch, base)?,
            new_branch.to_string(),
        )
    } else {
        let branch_ref = request.branch.trim();
        if branch_ref.is_empty() {
            return Err("Branch is required".to_string());
        }
        let remotes = Remote::list(&repo_path).unwrap_or_default();
        let name = strip_known_remote_prefix(branch_ref, &remotes).to_string();
        (resolve_worktree_path(&repo_path, branch_ref)?, name)
    };

    let mut env_vars = load_profile_env(request.profile.as_deref());
    // Useful for debugging and for agents that want to introspect gwt context.
    env_vars.insert(
        "GWT_PROJECT_ROOT".to_string(),
        project_root.to_string_lossy().to_string(),
    );

    // Request-specific overrides are highest precedence.
    if let Some(overrides) = request.env_overrides.as_ref() {
        for (k, v) in overrides {
            env_vars.insert(k.to_string(), v.to_string());
        }
    }

    let skip_permissions = request.skip_permissions.unwrap_or(false);
    if agent_id == "claude" && skip_permissions && std::env::consts::OS != "windows" {
        // SPEC-3b0ed29b: Skip-permissions on non-Windows sets IS_SANDBOX=1 to avoid
        // accidental confirmation prompts in sandboxed environments.
        env_vars.insert("IS_SANDBOX".to_string(), "1".to_string());
    }

    let settings = Settings::load(&project_root).unwrap_or_default();
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
    let mut docker_env: Option<HashMap<String, String>> = None;

    let use_docker = if docker_force_host {
        false
    } else {
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

                let container_name = DockerManager::generate_container_name(&branch_name);
                let manager = DockerManager::new(
                    &working_dir,
                    &branch_name,
                    DockerFileType::Compose(compose_path),
                );

                let mut env = manager.collect_passthrough_env();
                // Merge profile/env overrides so compose interpolation and container env inherit them.
                for (k, v) in &env_vars {
                    env.insert(k.to_string(), v.to_string());
                }
                env.insert("COMPOSE_PROJECT_NAME".to_string(), container_name.clone());

                docker_compose_up(
                    &working_dir,
                    &container_name,
                    &env,
                    docker_build,
                    docker_recreate,
                )?;

                docker_container_name = Some(container_name);
                docker_env = Some(env);
                true
            }
            _ => false,
        }
    };

    let resolved = if use_docker {
        resolve_agent_launch_command_for_container(agent_id, request.agent_version.as_deref())?
    } else {
        resolve_agent_launch_command(agent_id, request.agent_version.as_deref())?
    };

    let version_for_gates = resolved
        .version_for_gates
        .as_deref()
        .or(Some(resolved.tool_version.as_str()));

    let mut args = resolved.args.clone();
    args.extend(build_agent_args(agent_id, &request, version_for_gates)?);

    let mode = request.mode.unwrap_or(SessionMode::Normal);
    let mode_str = match mode {
        SessionMode::Normal => "normal",
        SessionMode::Continue => "continue",
        SessionMode::Resume => "resume",
    }
    .to_string();

    let collaboration_modes = if agent_id == "codex" {
        let requested = request.collaboration_modes.unwrap_or(false);
        requested && codex_supports_collaboration_modes(version_for_gates)
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
        let docker_service_entry = if use_docker {
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
            timestamp: started_at_millis,
        };

        if let Err(err) = gwt_core::config::save_session_entry(&repo_path, entry) {
            tracing::warn!(error = %err, "Failed to save session entry (launch): continuing");
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
        skip_permissions,
        collaboration_modes,
        docker_service: if use_docker {
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
        started_at_millis,
    };

    let config = if use_docker {
        let service = docker_service
            .as_deref()
            .ok_or_else(|| "Docker service is required".to_string())?;
        let docker_env = docker_env
            .as_ref()
            .ok_or_else(|| "Docker env is missing".to_string())?;
        let docker_args = build_docker_compose_exec_args(
            service,
            DOCKER_WORKDIR,
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
        }
    } else {
        BuiltinLaunchConfig {
            command: resolved.command,
            args,
            working_dir,
            branch_name,
            agent_name: resolved.label.to_string(),
            agent_color: agent_color_for(agent_id),
            env_vars,
        }
    };

    launch_with_config(config, Some(meta), &state, app_handle)
}

/// Stream PTY output to the frontend via Tauri events
fn stream_pty_output(mut reader: Box<dyn Read + Send>, pane_id: String, app_handle: AppHandle) {
    let state = app_handle.state::<AppState>();
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                // Keep the scrollback file up-to-date even if the UI is not listening.
                if let Ok(mut manager) = state.pane_manager.lock() {
                    if let Some(pane) = manager.pane_mut_by_id(&pane_id) {
                        let _ = pane.process_bytes(&buf[..n]);
                    }
                }

                let payload = TerminalOutputPayload {
                    pane_id: pane_id.clone(),
                    data: buf[..n].to_vec(),
                };
                // UI output is best-effort. Never stop consuming the PTY stream just because
                // the frontend isn't ready (tab switch, hot reload, etc.).
                let _ = app_handle.emit("terminal-output", &payload);
            }
            Err(_) => break,
        }
    }

    // Update pane status after the PTY stream ends.
    let exit_code = if let Ok(mut manager) = state.pane_manager.lock() {
        if let Some(pane) = manager.pane_mut_by_id(&pane_id) {
            let _ = pane.check_status();
            match pane.status() {
                PaneStatus::Completed(code) => Some(*code),
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    // Best-effort sessionId detection and persistence.
    let meta = state
        .pane_launch_meta
        .lock()
        .ok()
        .and_then(|mut map| map.remove(&pane_id));
    if let Some(meta) = meta {
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

                    let result = match detect_docker_files(&worktree_path) {
                        Some(DockerFileType::Compose(compose_path)) => {
                            let manager = DockerManager::new(
                                &worktree_path,
                                "",
                                DockerFileType::Compose(compose_path),
                            );
                            let mut env = manager.collect_passthrough_env();
                            env.insert("COMPOSE_PROJECT_NAME".to_string(), container_name.clone());
                            docker_compose_down(&worktree_path, &container_name, &env)
                        }
                        _ => Ok(()),
                    };

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

/// Write data to a terminal pane
#[tauri::command]
pub fn write_terminal(
    pane_id: String,
    data: Vec<u8>,
    state: State<AppState>,
) -> Result<(), String> {
    let mut manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {}", e))?;
    let pane = manager
        .pane_mut_by_id(&pane_id)
        .ok_or_else(|| format!("Pane not found: {}", pane_id))?;
    pane.write_input(&data)
        .map_err(|e| format!("Failed to write to terminal: {}", e))
}

/// Resize a terminal pane
#[tauri::command]
pub fn resize_terminal(
    pane_id: String,
    rows: u16,
    cols: u16,
    state: State<AppState>,
) -> Result<(), String> {
    let mut manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {}", e))?;
    let pane = manager
        .pane_mut_by_id(&pane_id)
        .ok_or_else(|| format!("Pane not found: {}", pane_id))?;
    pane.resize(rows, cols)
        .map_err(|e| format!("Failed to resize terminal: {}", e))
}

/// Close a terminal pane
#[tauri::command]
pub fn close_terminal(pane_id: String, state: State<AppState>) -> Result<(), String> {
    let mut manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {}", e))?;

    let index = manager
        .panes()
        .iter()
        .position(|p| p.pane_id() == pane_id)
        .ok_or_else(|| format!("Pane not found: {}", pane_id))?;

    manager.close_pane(index);
    Ok(())
}

/// List all active terminal panes
#[tauri::command]
pub fn list_terminals(state: State<AppState>) -> Vec<TerminalInfo> {
    let manager = match state.pane_manager.lock() {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    manager
        .panes()
        .iter()
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
