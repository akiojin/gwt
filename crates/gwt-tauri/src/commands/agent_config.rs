//! Agent-specific configuration (global, not per-profile)

use std::panic::{catch_unwind, AssertUnwindSafe};

use gwt_core::{config::AgentConfig, StructuredError};
use tracing::{error, instrument};

fn with_panic_guard<T>(
    context: &str,
    command: &str,
    f: impl FnOnce() -> Result<T, StructuredError>,
) -> Result<T, StructuredError> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            error!(
                category = "tauri",
                operation = context,
                "Unexpected panic while handling agent config command"
            );
            Err(StructuredError::internal(
                &format!("Unexpected error while {}", context),
                command,
            ))
        }
    }
}

/// Get current agent config from ~/.gwt/config.toml
#[instrument(skip_all, fields(command = "get_agent_config"))]
#[tauri::command]
pub fn get_agent_config() -> Result<AgentConfig, StructuredError> {
    with_panic_guard("loading agent config", "get_agent_config", || {
        AgentConfig::load().map_err(|e| StructuredError::from_gwt_error(&e, "get_agent_config"))
    })
}

/// Save agent config into ~/.gwt/config.toml
#[instrument(skip_all, fields(command = "save_agent_config"))]
#[tauri::command]
pub fn save_agent_config(config: AgentConfig) -> Result<(), StructuredError> {
    with_panic_guard("saving agent config", "save_agent_config", || {
        config
            .save()
            .map_err(|e| StructuredError::from_gwt_error(&e, "save_agent_config"))
    })
}
