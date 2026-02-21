//! Claude Code Hooks management commands

use gwt_core::config;
use gwt_core::StructuredError;
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
pub fn check_and_update_hooks() -> Result<HooksStatus, StructuredError> {
    let settings_path = config::get_claude_settings_path().ok_or_else(|| {
        StructuredError::internal(
            "Could not determine Claude settings path",
            "check_and_update_hooks",
        )
    })?;

    let temporary_execution = config::is_temporary_execution().is_some();

    if !config::is_gwt_hooks_registered(&settings_path) {
        return Ok(HooksStatus {
            registered: false,
            updated: false,
            temporary_execution,
        });
    }

    let updated = if temporary_execution {
        false
    } else {
        config::reregister_gwt_hooks(&settings_path)
            .map_err(|e| StructuredError::internal(&e.to_string(), "check_and_update_hooks"))?
    };

    Ok(HooksStatus {
        registered: true,
        updated,
        temporary_execution,
    })
}

/// Register hooks for the first time (called after user confirmation).
#[tauri::command]
pub fn register_hooks() -> Result<(), StructuredError> {
    let settings_path = config::get_claude_settings_path().ok_or_else(|| {
        StructuredError::internal("Could not determine Claude settings path", "register_hooks")
    })?;

    config::register_gwt_hooks(&settings_path)
        .map_err(|e| StructuredError::internal(&e.to_string(), "register_hooks"))
}
