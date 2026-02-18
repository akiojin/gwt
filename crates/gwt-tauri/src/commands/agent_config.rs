//! Agent-specific configuration (global, not per-profile)

use gwt_core::config::AgentConfig;
use std::panic::{catch_unwind, AssertUnwindSafe};
use tracing::error;

fn with_panic_guard<T>(context: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            error!(
                category = "tauri",
                operation = context,
                "Unexpected panic while handling agent config command"
            );
            Err(format!("Unexpected error while {}", context))
        }
    }
}

/// Get current agent config (global: ~/.gwt/agents.toml)
#[tauri::command]
pub fn get_agent_config() -> Result<AgentConfig, String> {
    with_panic_guard("loading agent config", || {
        AgentConfig::load().map_err(|e| e.to_string())
    })
}

/// Save agent config (always writes TOML: ~/.gwt/agents.toml)
#[tauri::command]
pub fn save_agent_config(config: AgentConfig) -> Result<(), String> {
    with_panic_guard("saving agent config", || {
        config.save().map_err(|e| e.to_string())
    })
}
