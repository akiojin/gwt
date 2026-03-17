#![allow(dead_code)]
//! Assistant Mode Tauri commands

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::assistant_engine::{AssistantEngine, AssistantStartupStatus};
use crate::state::AppState;
use gwt_core::git::{self, Branch};
use gwt_core::worktree::WorktreeManager;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub role: String,
    pub kind: String,
    pub content: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantStateResponse {
    pub messages: Vec<AssistantMessage>,
    pub ai_ready: bool,
    pub is_thinking: bool,
    pub session_id: Option<String>,
    pub llm_call_count: u64,
    pub estimated_tokens: u64,
    pub startup_status: String,
    pub startup_summary_ready: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneDashboard {
    pub pane_id: String,
    pub agent_name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDashboard {
    pub branch: String,
    pub uncommitted_count: u32,
    pub unpushed_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardResponse {
    pub panes: Vec<PaneDashboard>,
    pub git: GitDashboard,
}

#[tauri::command]
pub async fn assistant_get_state(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<AssistantStateResponse, String> {
    let window_label = window.label().to_string();
    let engine_guard = state
        .assistant_engine
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    match engine_guard.get(&window_label) {
        Some(engine) => Ok(build_assistant_state_response(engine, Some(window_label))),
        None => Ok(build_empty_assistant_state_response()),
    }
}

#[tauri::command]
pub async fn assistant_send_message(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    input: String,
) -> Result<AssistantStateResponse, String> {
    let window_label = window.label().to_string();
    let input = input.trim().to_string();
    if input.is_empty() {
        return Err("Message cannot be empty".to_string());
    }

    let mut engine = {
        let mut engine_guard = state
            .assistant_engine
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        engine_guard
            .remove(&window_label)
            .ok_or_else(|| "Assistant not started. Call assistant_start first.".to_string())?
    };

    let state_ref: &AppState = &state;
    let result = engine.handle_user_message(&input, state_ref);

    {
        let mut engine_guard = state
            .assistant_engine
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        engine_guard.insert(window_label.clone(), engine);
    }

    result?;

    let engine_guard = state
        .assistant_engine
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let engine = engine_guard
        .get(&window_label)
        .ok_or_else(|| "Assistant session disappeared after send.".to_string())?;

    Ok(build_assistant_state_response(engine, Some(window_label)))
}

#[tauri::command]
pub async fn assistant_start(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<AssistantStateResponse, String> {
    let window_label = window.label().to_string();
    let project_path = state
        .project_for_window(&window_label)
        .ok_or_else(|| "No project opened. Open a project first.".to_string())?;

    state.clear_assistant_session_for_window(&window_label);

    let mut engine = AssistantEngine::new(PathBuf::from(&project_path), window_label.clone());
    engine.handle_startup(&state)?;

    finalize_started_engine(&state, &window_label, &project_path, engine)
}

#[tauri::command]
pub async fn assistant_stop(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let window_label = window.label().to_string();

    {
        let mut engine_guard = state
            .assistant_engine
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        engine_guard.remove(&window_label);
    }

    {
        let mut monitor_guard = state
            .assistant_monitor_handle
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(handle) = monitor_guard.remove(&window_label) {
            tokio::spawn(async move {
                handle.stop().await;
            });
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn assistant_get_dashboard(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<DashboardResponse, String> {
    let window_label = window.label().to_string();
    let Some(project_path) = state.project_for_window(&window_label) else {
        return Ok(DashboardResponse {
            panes: Vec::new(),
            git: build_git_dashboard(&state, &window_label)?,
        });
    };

    let panes = {
        let repo_path =
            crate::commands::project::resolve_repo_path_for_project_root(Path::new(&project_path))
                .map_err(|e| format!("Failed to resolve repository path: {}", e))?;
        let mgr = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        mgr.panes()
            .iter()
            .filter(|pane| pane.project_root() == repo_path.as_path())
            .map(|pane| PaneDashboard {
                pane_id: pane.pane_id().to_string(),
                agent_name: pane.agent_name().to_string(),
                status: format!("{:?}", pane.status()),
            })
            .collect::<Vec<_>>()
    };

    Ok(DashboardResponse {
        panes,
        git: build_git_dashboard(&state, &window_label)?,
    })
}

fn build_assistant_state_response(
    engine: &AssistantEngine,
    session_id: Option<String>,
) -> AssistantStateResponse {
    AssistantStateResponse {
        messages: build_messages_from_conversation(engine),
        ai_ready: true,
        is_thinking: engine.startup_status() == AssistantStartupStatus::Analyzing,
        session_id,
        llm_call_count: engine.llm_call_count,
        estimated_tokens: engine.estimated_tokens,
        startup_status: engine.startup_status().as_str().to_string(),
        startup_summary_ready: engine.startup_summary_ready(),
    }
}

fn build_empty_assistant_state_response() -> AssistantStateResponse {
    AssistantStateResponse {
        messages: Vec::new(),
        ai_ready: check_ai_configured(),
        is_thinking: false,
        session_id: None,
        llm_call_count: 0,
        estimated_tokens: 0,
        startup_status: AssistantStartupStatus::Idle.as_str().to_string(),
        startup_summary_ready: false,
    }
}

fn finalize_started_engine(
    state: &AppState,
    window_label: &str,
    project_path: &str,
    engine: AssistantEngine,
) -> Result<AssistantStateResponse, String> {
    let current_project_path = state.project_for_window(window_label);
    let mut engine_guard = state
        .assistant_engine
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    if current_project_path.as_deref() != Some(project_path) {
        return Ok(engine_guard
            .get(window_label)
            .map(|current| build_assistant_state_response(current, Some(window_label.to_string())))
            .unwrap_or_else(build_empty_assistant_state_response));
    }

    if let Some(existing) = engine_guard.get(window_label) {
        return Ok(build_assistant_state_response(
            existing,
            Some(window_label.to_string()),
        ));
    }

    engine_guard.insert(window_label.to_string(), engine);
    let inserted = engine_guard
        .get(window_label)
        .ok_or_else(|| "Assistant session disappeared after startup.".to_string())?;

    Ok(build_assistant_state_response(
        inserted,
        Some(window_label.to_string()),
    ))
}

fn build_git_dashboard(state: &AppState, window_label: &str) -> Result<GitDashboard, String> {
    let Some(project_path) = state.project_for_window(window_label) else {
        return Ok(GitDashboard {
            branch: String::new(),
            uncommitted_count: 0,
            unpushed_count: 0,
        });
    };

    let repo_path =
        crate::commands::project::resolve_repo_path_for_project_root(Path::new(&project_path))
            .map_err(|e| format!("Failed to resolve repository path: {}", e))?;

    let current_branch = Branch::current(&repo_path)
        .map_err(|e| format!("Failed to resolve current branch: {}", e))?;

    let (branch, unpushed_count) = current_branch
        .as_ref()
        .map(|branch| {
            (
                branch.name.clone(),
                branch.ahead.min(u32::MAX as usize) as u32,
            )
        })
        .unwrap_or_else(|| (String::new(), 0));

    let uncommitted_count = resolve_dashboard_worktree_path(&repo_path, current_branch.as_ref())
        .and_then(|path| git::get_working_tree_status(&path).ok())
        .map(|entries| entries.len().min(u32::MAX as usize) as u32)
        .unwrap_or(0);

    Ok(GitDashboard {
        branch,
        uncommitted_count,
        unpushed_count,
    })
}

fn resolve_dashboard_worktree_path(
    repo_path: &Path,
    current_branch: Option<&Branch>,
) -> Option<PathBuf> {
    if !git::is_bare_repository(repo_path) {
        return Some(repo_path.to_path_buf());
    }

    let manager = WorktreeManager::new(repo_path).ok()?;
    let worktrees = manager.list_basic().ok()?;

    if let Some(branch_name) = current_branch.map(|branch| branch.name.as_str()) {
        if let Some(worktree) = worktrees.iter().find(|worktree| {
            worktree.is_active() && worktree.branch.as_deref() == Some(branch_name)
        }) {
            return Some(worktree.path.clone());
        }
    }

    worktrees
        .iter()
        .find(|worktree| worktree.is_active() && !worktree.is_main)
        .map(|worktree| worktree.path.clone())
}

fn build_messages_from_conversation(engine: &AssistantEngine) -> Vec<AssistantMessage> {
    let now = chrono::Utc::now().timestamp();
    engine
        .conversation()
        .iter()
        .filter_map(|msg| {
            let content = msg.content.as_deref().unwrap_or("");
            if msg.role == "system" || msg.role == "tool" {
                return None;
            }
            let kind = if msg.tool_calls.is_some() {
                "tool_use".to_string()
            } else {
                "text".to_string()
            };
            Some(AssistantMessage {
                role: msg.role.clone(),
                kind,
                content: content.to_string(),
                timestamp: now,
            })
        })
        .collect()
}

fn check_ai_configured() -> bool {
    gwt_core::config::ProfilesConfig::load()
        .ok()
        .map(|profiles| profiles.resolve_active_ai_settings().resolved.is_some())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_started_engine_skips_stale_project_startup() {
        let state = AppState::new();
        state
            .claim_project_for_window_with_identity(
                "main",
                "/tmp/current".to_string(),
                "/tmp/current-id".to_string(),
            )
            .unwrap();

        let stale_engine = AssistantEngine::new(PathBuf::from("/tmp/stale"), "main".to_string());
        let response = finalize_started_engine(&state, "main", "/tmp/stale", stale_engine).unwrap();

        assert!(state.assistant_engine.lock().unwrap().is_empty());
        assert_eq!(response.session_id, None);
    }

    #[test]
    fn finalize_started_engine_keeps_existing_session() {
        let state = AppState::new();
        state
            .claim_project_for_window_with_identity(
                "main",
                "/tmp/current".to_string(),
                "/tmp/current-id".to_string(),
            )
            .unwrap();

        state.assistant_engine.lock().unwrap().insert(
            "main".to_string(),
            AssistantEngine::new(PathBuf::from("/tmp/current"), "main".to_string()),
        );

        let stale_engine = AssistantEngine::new(PathBuf::from("/tmp/stale"), "main".to_string());
        let response = finalize_started_engine(&state, "main", "/tmp/stale", stale_engine).unwrap();

        let stored = state.assistant_engine.lock().unwrap();
        let current = stored.get("main").unwrap();
        assert_eq!(current.project_path(), Path::new("/tmp/current"));
        assert_eq!(response.session_id.as_deref(), Some("main"));
    }
}
