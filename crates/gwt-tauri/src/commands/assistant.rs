#![allow(dead_code)]
//! Assistant Mode Tauri commands

use serde::Serialize;
use std::path::PathBuf;

use crate::assistant_engine::AssistantEngine;
use crate::assistant_monitor::{
    build_snapshot_for_window, start_monitor, MonitorEvent, MonitorSnapshot,
};
use crate::state::AppState;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;

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
) -> Result<(), String> {
    let window_label = window.label().to_string();
    let project_path = state
        .project_for_window(&window_label)
        .ok_or_else(|| "No project opened. Open a project first.".to_string())?;

    state.clear_assistant_session_for_window(&window_label);

    let engine = AssistantEngine::new(PathBuf::from(&project_path), window_label.clone());
    let (event_tx, mut event_rx) = mpsc::channel::<MonitorEvent>(8);
    let app_handle = window.app_handle().clone();
    let receiver_window_label = window_label.clone();
    let monitor_handle = start_monitor(app_handle.clone(), window_label.clone(), event_tx);

    {
        let mut engine_guard = state
            .assistant_engine
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        engine_guard.insert(window_label.clone(), engine);
    }
    {
        let mut monitor_guard = state
            .assistant_monitor_handle
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        monitor_guard.insert(window_label.clone(), monitor_handle);
    }

    tokio::spawn(async move {
        while let Some(first_event) = event_rx.recv().await {
            let mut events = vec![first_event];
            while let Ok(event) = event_rx.try_recv() {
                events.push(event);
            }

            let state = app_handle.state::<AppState>();
            let mut engine = {
                let mut engine_guard = match state.assistant_engine.lock() {
                    Ok(guard) => guard,
                    Err(_) => continue,
                };
                let Some(engine) = engine_guard.remove(&receiver_window_label) else {
                    continue;
                };
                engine
            };

            let _ = engine.handle_monitor_batch(events, &state);
            let assistant_state =
                build_assistant_state_response(&engine, Some(receiver_window_label.clone()));
            let dashboard = build_snapshot_for_window(&state, &receiver_window_label)
                .ok()
                .map(build_dashboard_response);

            if let Ok(mut engine_guard) = state.assistant_engine.lock() {
                engine_guard.insert(receiver_window_label.clone(), engine);
            }

            if let Some(window) = app_handle.get_webview_window(&receiver_window_label) {
                let _ = window.emit("assistant-state-updated", &assistant_state);
                if let Some(dashboard) = dashboard.as_ref() {
                    let _ = window.emit("assistant-dashboard-updated", dashboard);
                }
            }
        }
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
            handle.stop_now();
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn assistant_get_dashboard(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<DashboardResponse, String> {
    let snapshot = build_snapshot_for_window(&state, window.label())?;
    Ok(build_dashboard_response(snapshot))
}

fn build_dashboard_response(snapshot: MonitorSnapshot) -> DashboardResponse {
    DashboardResponse {
        panes: snapshot
            .panes
            .into_iter()
            .map(|pane| PaneDashboard {
                pane_id: pane.pane_id,
                agent_name: pane.agent_name,
                status: pane.status,
            })
            .collect(),
        git: GitDashboard {
            branch: snapshot.git.branch,
            uncommitted_count: snapshot.git.uncommitted_count,
            unpushed_count: snapshot.git.unpushed_count,
        },
    }
}

fn build_assistant_state_response(
    engine: &AssistantEngine,
    session_id: Option<String>,
) -> AssistantStateResponse {
    AssistantStateResponse {
        messages: build_messages_from_conversation(engine),
        ai_ready: check_ai_configured(),
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

fn build_messages_from_conversation(engine: &AssistantEngine) -> Vec<AssistantMessage> {
    engine
        .conversation()
        .iter()
        .enumerate()
        .filter_map(|(index, msg)| {
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
                timestamp: engine
                    .message_timestamps()
                    .get(index)
                    .copied()
                    .unwrap_or_else(|| chrono::Utc::now().timestamp()),
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
