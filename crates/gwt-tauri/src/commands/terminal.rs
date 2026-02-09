//! Terminal/PTY management commands for xterm.js integration

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::state::AppState;
use gwt_core::config::ProfilesConfig;
use gwt_core::git::Remote;
use gwt_core::terminal::pane::PaneStatus;
use gwt_core::terminal::{AgentColor, BuiltinLaunchConfig};
use gwt_core::worktree::WorktreeManager;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, State};
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchAgentRequest {
    /// Agent id (e.g., "claude", "codex", "gemini")
    pub agent_id: String,
    /// Branch name or remote ref (e.g., "main", "feature/foo", "origin/main")
    pub branch: String,
    /// Optional profile name override (uses active profile when omitted)
    pub profile: Option<String>,
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

struct BuiltinAgentDef {
    label: &'static str,
    local_command: &'static str,
    bunx_package: &'static str,
}

fn builtin_agent_def(agent_id: &str) -> Result<BuiltinAgentDef, String> {
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

fn resolve_agent_launch_command(
    agent_id: &str,
) -> Result<(String, Vec<String>, &'static str), String> {
    let def = builtin_agent_def(agent_id)?;

    // Prefer local installed command.
    if which(def.local_command).is_ok() {
        return Ok((def.local_command.to_string(), Vec::new(), def.label));
    }

    // Fallback to bunx (or npx for local node_modules bunx) per SPEC-3b0ed29b.
    let bunx_path = which("bunx").ok().map(|p| p.to_string_lossy().to_string());
    let npx_available = which("npx").is_ok();

    let runner = choose_fallback_runner(bunx_path.as_deref(), npx_available)
        .ok_or_else(|| "Agent is not installed and bunx/npx is not available".to_string())?;

    let package = format!("{}@latest", def.bunx_package);
    let (cmd, args) = build_fallback_launch(runner, &package);
    Ok((cmd, args, def.label))
}

fn launch_with_config(
    config: BuiltinLaunchConfig,
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

    launch_with_config(config, &state, app_handle)
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
    let (command, args, label) = resolve_agent_launch_command(agent_id)?;

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

    let config = BuiltinLaunchConfig {
        command,
        args,
        working_dir,
        branch_name,
        agent_name: label.to_string(),
        agent_color: AgentColor::Green,
        env_vars,
    };

    launch_with_config(config, &state, app_handle)
}

/// Stream PTY output to the frontend via Tauri events
fn stream_pty_output(mut reader: Box<dyn Read + Send>, pane_id: String, app_handle: AppHandle) {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                let payload = TerminalOutputPayload {
                    pane_id: pane_id.clone(),
                    data: buf[..n].to_vec(),
                };
                if app_handle.emit("terminal-output", &payload).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
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
