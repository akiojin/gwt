use crate::agent_master::{get_agent_mode_state, send_agent_message, AgentModeState};
use crate::state::AppState;
use tauri::{Manager, State, Window};

#[tauri::command]
pub fn get_agent_mode_state_cmd(window: Window, state: State<AppState>) -> AgentModeState {
    get_agent_mode_state(&state, window.label())
}

#[tauri::command]
pub async fn send_agent_mode_message(window: Window, input: String) -> AgentModeState {
    let window_label = window.label().to_string();
    let app_handle = window.app_handle();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        send_agent_message(&state, &window_label, &input)
    })
    .await
    .unwrap_or_else(|_| AgentModeState::new())
}
