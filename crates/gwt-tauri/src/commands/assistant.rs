#![allow(dead_code)]
//! Assistant Mode Tauri commands

use serde::Serialize;

use crate::assistant_engine::AssistantEngine;
use crate::state::AppState;

// ── Response types ──────────────────────────────────────────────────

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

// ── Commands ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn assistant_get_state(
    state: tauri::State<'_, AppState>,
) -> Result<AssistantStateResponse, String> {
    let engine_guard = state
        .assistant_engine
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    match engine_guard.as_ref() {
        Some(engine) => {
            let messages = build_messages_from_conversation(engine);
            Ok(AssistantStateResponse {
                messages,
                ai_ready: true,
                is_thinking: false,
                session_id: Some("active".to_string()),
                llm_call_count: engine.llm_call_count,
                estimated_tokens: engine.estimated_tokens,
            })
        }
        None => Ok(AssistantStateResponse {
            messages: Vec::new(),
            ai_ready: check_ai_configured(),
            is_thinking: false,
            session_id: None,
            llm_call_count: 0,
            estimated_tokens: 0,
        }),
    }
}

#[tauri::command]
pub async fn assistant_send_message(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    input: String,
) -> Result<AssistantStateResponse, String> {
    let input = input.trim().to_string();
    if input.is_empty() {
        return Err("Message cannot be empty".to_string());
    }

    // Extract what we need from state before the blocking operation
    let (mut engine, _project_path) = {
        let mut engine_guard = state
            .assistant_engine
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let engine = engine_guard
            .take()
            .ok_or_else(|| "Assistant not started. Call assistant_start first.".to_string())?;

        let window_label = get_window_label_from_app(&app);
        let project_path = state
            .project_for_window(&window_label)
            .unwrap_or_default();

        (engine, project_path)
    };

    // Clone the state reference for the blocking call
    // We need to pass AppState to the engine, so we create it fresh
    let state_ref: &AppState = &state;

    // Run the LLM loop (this may be slow)
    let result = engine.handle_user_message(&input, state_ref);

    // Put the engine back
    {
        let mut engine_guard = state
            .assistant_engine
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        *engine_guard = Some(engine);
    }

    // Build response
    result.map(|_response| {
        let engine_guard = state.assistant_engine.lock().unwrap();
        let engine = engine_guard.as_ref().unwrap();
        let messages = build_messages_from_conversation(engine);
        AssistantStateResponse {
            messages,
            ai_ready: true,
            is_thinking: false,
            session_id: Some("active".to_string()),
            llm_call_count: engine.llm_call_count,
            estimated_tokens: engine.estimated_tokens,
        }
    })
}

#[tauri::command]
pub async fn assistant_start(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let window_label = get_window_label_from_app(&app);
    let project_path = state
        .project_for_window(&window_label)
        .ok_or_else(|| "No project opened. Open a project first.".to_string())?;

    let engine = AssistantEngine::new(
        std::path::PathBuf::from(&project_path),
        window_label,
    );

    let mut engine_guard = state
        .assistant_engine
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    *engine_guard = Some(engine);

    Ok(())
}

#[tauri::command]
pub async fn assistant_stop(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // Stop the engine
    {
        let mut engine_guard = state
            .assistant_engine
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        *engine_guard = None;
    }

    // Stop the monitor
    {
        let mut monitor_guard = state
            .assistant_monitor_handle
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(handle) = monitor_guard.take() {
            // Fire-and-forget stop since we can't .await in sync context easily
            tokio::spawn(async move {
                handle.stop().await;
            });
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn assistant_get_dashboard(
    state: tauri::State<'_, AppState>,
) -> Result<DashboardResponse, String> {
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

    // Git status would require project path; return defaults for now
    Ok(DashboardResponse {
        panes,
        git: GitDashboard {
            branch: String::new(),
            uncommitted_count: 0,
            unpushed_count: 0,
        },
    })
}

// ── Helpers ─────────────────────────────────────────────────────────

fn build_messages_from_conversation(engine: &AssistantEngine) -> Vec<AssistantMessage> {
    let now = chrono::Utc::now().timestamp();
    engine
        .conversation()
        .iter()
        .filter_map(|msg| {
            let content = msg.content.as_deref().unwrap_or("");
            // Skip system messages and tool messages from the UI view
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

fn get_window_label_from_app(app: &tauri::AppHandle) -> String {
    use tauri::Manager;
    app.webview_windows()
        .keys()
        .next()
        .cloned()
        .unwrap_or_else(|| "main".to_string())
}
