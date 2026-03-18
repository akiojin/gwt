#![allow(dead_code)]
//! Assistant Mode Tauri commands

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager};

use crate::assistant_engine::AssistantEngine;
use crate::state::AppState;
use gwt_core::git::{self, Branch};
use gwt_core::process::command as process_command;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StartupAnalysisFingerprint {
    branch: String,
    head_revision: String,
    worktree_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StartupAnalysisCacheEntry {
    fingerprint: StartupAnalysisFingerprint,
    summary: String,
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

    if let Some(engine) = engine_guard.get(&window_label) {
        return Ok(build_assistant_state_response(engine, Some(window_label)));
    }

    drop(engine_guard);

    let startup_guard = state
        .assistant_startup_inflight
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    if let Some(status) = startup_guard.get(&window_label) {
        return Ok(build_startup_inflight_state_response(
            window_label,
            status.clone(),
        ));
    }

    Ok(build_empty_assistant_state_response())
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
) -> Result<(), String> {
    let window_label = window.label().to_string();
    let project_path = state
        .project_for_window(&window_label)
        .ok_or_else(|| "No project opened. Open a project first.".to_string())?;

    state.clear_assistant_session_for_window(&window_label);
    set_startup_status(
        &state,
        &window_label,
        "Inspecting repository state...".to_string(),
    )
    .map_err(|e| format!("Lock error: {}", e))?;

    let app_handle = window.app_handle().clone();
    let window_label_for_task = window_label.clone();
    let project_path_for_task = project_path.clone();
    tokio::spawn(async move {
        let state = app_handle.state::<AppState>();
        let _ = emit_startup_status(
            &app_handle,
            &window_label_for_task,
            "Inspecting repository state...".to_string(),
        );
        let mut engine = AssistantEngine::new(
            PathBuf::from(&project_path_for_task),
            window_label_for_task.clone(),
        );

        let fingerprint = match resolve_startup_analysis_fingerprint(&state, &window_label_for_task)
        {
            Ok(fingerprint) => fingerprint,
            Err(err) => {
                engine.push_visible_assistant_message(format!(
                    "Assistant started, but repository inspection failed: {err}"
                ));
                finish_startup_session(&app_handle, &window_label_for_task, engine);
                return;
            }
        };

        let cache_path = startup_analysis_cache_path(&project_path_for_task);
        let _ = emit_startup_status(
            &app_handle,
            &window_label_for_task,
            "Checking startup analysis cache...".to_string(),
        );
        if let Some(cache) = load_startup_analysis_cache(&cache_path) {
            if cache.fingerprint == fingerprint {
                let _ = emit_startup_status(
                    &app_handle,
                    &window_label_for_task,
                    "Using cached startup analysis...".to_string(),
                );
                engine.push_visible_assistant_message(cache.summary);
                finish_startup_session(&app_handle, &window_label_for_task, engine);
                return;
            }
        }

        let _ = emit_startup_status(
            &app_handle,
            &window_label_for_task,
            "Running startup analysis...".to_string(),
        );
        match engine.run_initial_analysis(&state) {
            Ok(response) => {
                if !response.text.is_empty() {
                    let cache = StartupAnalysisCacheEntry {
                        fingerprint,
                        summary: response.text,
                    };
                    let _ = save_startup_analysis_cache(&cache_path, &cache);
                }
            }
            Err(err) => {
                engine.push_visible_assistant_message(format!(
                    "Assistant started, but the initial analysis failed: {err}"
                ));
            }
        }

        finish_startup_session(&app_handle, &window_label_for_task, engine);
    });

    Ok(())
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
    let panes = {
        let mgr = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        mgr.panes()
            .iter()
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
        is_thinking: false,
        session_id,
        llm_call_count: engine.llm_call_count,
        estimated_tokens: engine.estimated_tokens,
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
    }
}

fn build_startup_inflight_state_response(
    session_id: String,
    status_message: String,
) -> AssistantStateResponse {
    AssistantStateResponse {
        messages: vec![AssistantMessage {
            role: "assistant".to_string(),
            kind: "text".to_string(),
            content: status_message,
            timestamp: chrono::Utc::now().timestamp(),
        }],
        ai_ready: check_ai_configured(),
        is_thinking: true,
        session_id: Some(session_id),
        llm_call_count: 0,
        estimated_tokens: 0,
    }
}

fn set_startup_status(
    state: &AppState,
    window_label: &str,
    status_message: String,
) -> Result<(), String> {
    let mut guard = state
        .assistant_startup_inflight
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    guard.insert(window_label.to_string(), status_message);
    Ok(())
}

fn emit_startup_status(
    app_handle: &tauri::AppHandle,
    window_label: &str,
    status_message: String,
) -> Result<(), String> {
    let state = app_handle.state::<AppState>();
    set_startup_status(&state, window_label, status_message.clone())?;

    if let Some(window) = app_handle.get_webview_window(window_label) {
        let response =
            build_startup_inflight_state_response(window_label.to_string(), status_message);
        let _ = window.emit("assistant-state-updated", &response);
    }

    Ok(())
}

fn finish_startup_session(
    app_handle: &tauri::AppHandle,
    window_label: &str,
    engine: AssistantEngine,
) {
    let state = app_handle.state::<AppState>();
    let response = build_assistant_state_response(&engine, Some(window_label.to_string()));

    if let Ok(mut startup_guard) = state.assistant_startup_inflight.lock() {
        startup_guard.remove(window_label);
    }
    if let Ok(mut engine_guard) = state.assistant_engine.lock() {
        engine_guard.insert(window_label.to_string(), engine);
    }
    if let Some(window) = app_handle.get_webview_window(window_label) {
        let _ = window.emit("assistant-state-updated", &response);
    }
}

fn startup_analysis_cache_path(project_path: &str) -> PathBuf {
    Path::new(project_path)
        .join(".gwt")
        .join("assistant")
        .join("startup-analysis.json")
}

fn load_startup_analysis_cache(path: &Path) -> Option<StartupAnalysisCacheEntry> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_startup_analysis_cache(
    path: &Path,
    entry: &StartupAnalysisCacheEntry,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid startup analysis cache path".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create cache dir: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(entry)
        .map_err(|e| format!("Failed to serialize startup cache: {e}"))?;
    std::fs::write(&tmp, content).map_err(|e| format!("Failed to write cache tmp file: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Failed to finalize startup cache: {e}"))?;
    Ok(())
}

fn resolve_startup_analysis_fingerprint(
    state: &AppState,
    window_label: &str,
) -> Result<StartupAnalysisFingerprint, String> {
    let project_path = state
        .project_for_window(window_label)
        .ok_or_else(|| "No project opened for startup analysis.".to_string())?;

    let repo_path =
        crate::commands::project::resolve_repo_path_for_project_root(Path::new(&project_path))
            .map_err(|e| format!("Failed to resolve repository path: {e}"))?;

    let current_branch = Branch::current(&repo_path)
        .map_err(|e| format!("Failed to resolve current branch: {e}"))?;
    let branch = current_branch
        .as_ref()
        .map(|branch| branch.name.clone())
        .unwrap_or_default();

    let worktree_path = resolve_dashboard_worktree_path(&repo_path, current_branch.as_ref())
        .unwrap_or_else(|| repo_path.clone());
    let head_revision = run_git_text(&worktree_path, &["rev-parse", "HEAD"])?;
    let worktree_status = run_git_text(&worktree_path, &["status", "--short"])?;

    Ok(StartupAnalysisFingerprint {
        branch,
        head_revision,
        worktree_status,
    })
}

fn run_git_text(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = process_command("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("Failed to run git {}: {}", args.join(" "), e))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
    fn assistant_build_messages_from_conversation_hides_system_messages() {
        let mut engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        engine.push_hidden_system_message_for_test("hidden startup prompt");
        engine.push_visible_assistant_message("visible guidance");

        let messages = build_messages_from_conversation(&engine);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[0].content, "visible guidance");
    }

    #[test]
    fn assistant_startup_inflight_state_exposes_session_id_and_thinking() {
        let response = build_startup_inflight_state_response(
            "main".to_string(),
            "Checking startup analysis cache...".to_string(),
        );

        assert_eq!(response.session_id.as_deref(), Some("main"));
        assert!(response.is_thinking);
        assert_eq!(response.messages.len(), 1);
        assert_eq!(
            response.messages[0].content,
            "Checking startup analysis cache..."
        );
    }

    #[test]
    fn assistant_startup_analysis_cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("startup-analysis.json");
        let entry = StartupAnalysisCacheEntry {
            fingerprint: StartupAnalysisFingerprint {
                branch: "main".to_string(),
                head_revision: "abc123".to_string(),
                worktree_status: " M src/lib.rs".to_string(),
            },
            summary: "Cached startup summary".to_string(),
        };

        save_startup_analysis_cache(&path, &entry).unwrap();
        let loaded = load_startup_analysis_cache(&path).unwrap();

        assert_eq!(loaded, entry);
    }
}
