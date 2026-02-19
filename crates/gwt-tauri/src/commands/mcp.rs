use crate::state::AppState;
use gwt_core::config::{
    get_mcp_registration_status, repair_mcp_registration, McpRegistrationStatus,
};
use tauri::{Manager, State};

#[tauri::command]
pub fn get_mcp_registration_status_cmd(
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<McpRegistrationStatus, String> {
    let resource_dir = app_handle.path().resource_dir().ok();
    let status = get_mcp_registration_status(resource_dir.as_deref());
    state.set_mcp_registration_status(status);
    Ok(state.get_mcp_registration_status())
}

#[tauri::command]
pub fn repair_mcp_registration_cmd(
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<McpRegistrationStatus, String> {
    crate::mcp_ws_server::cleanup_stale_state_file();
    let resource_dir = app_handle.path().resource_dir().ok();
    let status = repair_mcp_registration(resource_dir.as_deref());
    state.set_mcp_registration_status(status);
    Ok(state.get_mcp_registration_status())
}
