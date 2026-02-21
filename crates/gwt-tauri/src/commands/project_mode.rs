use crate::agent_master::{
    force_stop_project_mode, get_project_mode_state, list_project_mode_sessions,
    restore_project_mode_session, send_project_mode_message as send_project_mode_message_impl,
    ProjectModeSessionSummary, ProjectModeState,
};
use crate::state::AppState;
use gwt_core::StructuredError;
use tauri::{Manager, State, Window};

#[tauri::command]
pub fn get_project_mode_state_cmd(window: Window, state: State<AppState>) -> ProjectModeState {
    get_project_mode_state(&state, window.label())
}

#[tauri::command]
pub async fn send_project_mode_message(window: Window, input: String) -> ProjectModeState {
    send_project_mode_message_cmd(window, input).await
}

#[tauri::command]
pub async fn send_project_mode_message_cmd(window: Window, input: String) -> ProjectModeState {
    let window_label = window.label().to_string();
    let app_handle = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        send_project_mode_message_impl(&state, &window_label, &input)
    })
    .await
    .unwrap_or_else(|_| ProjectModeState::new())
}

#[tauri::command]
pub async fn restore_project_mode_session_cmd(
    window: Window,
    session_id: String,
) -> Result<ProjectModeState, StructuredError> {
    let window_label = window.label().to_string();
    let app_handle = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        let (_session, mode) = restore_project_mode_session(&state, &session_id)
            .map_err(|e| StructuredError::internal(&e, "restore_project_mode_session_cmd"))?;

        // Store the restored mode in the window's project mode state.
        if let Ok(mut guard) = state.window_project_modes.lock() {
            guard.insert(window_label, mode.clone());
        }

        Ok(mode)
    })
    .await
    .unwrap_or_else(|_| Err(StructuredError::internal("Task join error", "restore_project_mode_session_cmd")))
}

#[tauri::command]
pub async fn list_project_mode_sessions_cmd(
    state: State<'_, AppState>,
) -> Result<Vec<ProjectModeSessionSummary>, StructuredError> {
    list_project_mode_sessions(&state)
        .map_err(|e| StructuredError::internal(&e, "list_project_mode_sessions_cmd"))
}

#[tauri::command]
pub async fn stop_project_mode_session_cmd(
    window: Window,
    session_id: String,
) -> Result<String, StructuredError> {
    let window_label = window.label().to_string();
    let app_handle = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        let msg = force_stop_project_mode(&state, &session_id)
            .map_err(|e| StructuredError::internal(&e, "stop_project_mode_session_cmd"))?;

        // Update the in-memory project mode state to reflect the pause.
        if let Ok(mut guard) = state.window_project_modes.lock() {
            if let Some(mode) = guard.get_mut(&window_label) {
                mode.is_waiting = false;
                mode.lead_status = Some("idle".to_string());
            }
        }

        Ok(msg)
    })
    .await
    .unwrap_or_else(|_| Err(StructuredError::internal("Task join error", "stop_project_mode_session_cmd")))
}
