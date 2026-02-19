use crate::agent_master::{
    force_stop_project_team, get_agent_mode_state, list_project_team_sessions,
    restore_project_team_session, send_agent_message, send_project_team_message, AgentModeState,
    ProjectTeamSessionSummary,
};
use crate::state::AppState;
use tauri::{Manager, State, Window};

#[tauri::command]
pub fn get_agent_mode_state_cmd(window: Window, state: State<AppState>) -> AgentModeState {
    get_agent_mode_state(&state, window.label())
}

#[tauri::command]
pub async fn send_agent_mode_message(window: Window, input: String) -> AgentModeState {
    let window_label = window.label().to_string();
    let app_handle = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        send_agent_message(&state, &window_label, &input)
    })
    .await
    .unwrap_or_else(|_| AgentModeState::new())
}

#[tauri::command]
pub async fn send_project_team_message_cmd(window: Window, input: String) -> AgentModeState {
    let window_label = window.label().to_string();
    let app_handle = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        send_project_team_message(&state, &window_label, &input)
    })
    .await
    .unwrap_or_else(|_| AgentModeState::new())
}

#[tauri::command]
pub async fn restore_project_team_session_cmd(
    window: Window,
    session_id: String,
) -> Result<AgentModeState, String> {
    let window_label = window.label().to_string();
    let app_handle = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        let (_session, mode) = restore_project_team_session(&state, &session_id)?;

        // Store the restored mode in the window's agent mode state
        if let Ok(mut guard) = state.window_agent_modes.lock() {
            guard.insert(window_label, mode.clone());
        }

        Ok(mode)
    })
    .await
    .unwrap_or_else(|_| Err("Task join error".to_string()))
}

#[tauri::command]
pub async fn list_project_team_sessions_cmd(
    state: State<'_, AppState>,
) -> Result<Vec<ProjectTeamSessionSummary>, String> {
    list_project_team_sessions(&state)
}

#[tauri::command]
pub async fn stop_project_team_session_cmd(
    window: Window,
    session_id: String,
) -> Result<String, String> {
    let window_label = window.label().to_string();
    let app_handle = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        let msg = force_stop_project_team(&state, &session_id)?;

        // Update the in-memory agent mode state to reflect the pause
        if let Ok(mut guard) = state.window_agent_modes.lock() {
            if let Some(mode) = guard.get_mut(&window_label) {
                mode.is_waiting = false;
                mode.lead_status = Some("idle".to_string());
            }
        }

        Ok(msg)
    })
    .await
    .unwrap_or_else(|_| Err("Task join error".to_string()))
}
