//! Claude Code Hooks management commands

use gwt_core::config;
use serde::Serialize;

/// Status returned by check_and_update_hooks
#[derive(Debug, Serialize)]
pub struct HooksStatus {
    pub registered: bool,
    pub updated: bool,
    pub temporary_execution: bool,
}

/// Check hooks status and silently update if already registered.
#[tauri::command]
pub fn check_and_update_hooks() -> Result<HooksStatus, String> {
    let settings_path = config::get_claude_settings_path()
        .ok_or_else(|| "Could not determine Claude settings path".to_string())?;

    let temporary_execution = config::is_temporary_execution().is_some();

    if !config::is_gwt_hooks_registered(&settings_path) {
        return Ok(HooksStatus {
            registered: false,
            updated: false,
            temporary_execution,
        });
    }

    // Already registered: silently update executable path
    let updated = config::reregister_gwt_hooks(&settings_path).map_err(|e| e.to_string())?;

    Ok(HooksStatus {
        registered: true,
        updated,
        temporary_execution,
    })
}

/// Register hooks for the first time (called after user confirmation).
#[tauri::command]
pub fn register_hooks() -> Result<(), String> {
    let settings_path = config::get_claude_settings_path()
        .ok_or_else(|| "Could not determine Claude settings path".to_string())?;

    config::register_gwt_hooks(&settings_path).map_err(|e| e.to_string())
}
