use crate::agent_master::{get_agent_mode_state, send_agent_message, AgentModeState};
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn get_agent_mode_state_cmd(state: State<AppState>) -> AgentModeState {
    get_agent_mode_state(&state)
}

#[tauri::command]
pub fn send_agent_mode_message(state: State<AppState>, input: String) -> AgentModeState {
    send_agent_message(&state, &input)
}
