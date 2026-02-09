//! Profiles (env + AI settings) management commands

use gwt_core::config::ProfilesConfig;
use std::panic::{catch_unwind, AssertUnwindSafe};
use tracing::error;

fn with_panic_guard<T>(context: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            error!(
                category = "tauri",
                operation = context,
                "Unexpected panic while handling profiles command"
            );
            Err(format!("Unexpected error while {}", context))
        }
    }
}

/// Get current profiles config (global: ~/.gwt/profiles.{toml,yaml})
#[tauri::command]
pub fn get_profiles() -> Result<ProfilesConfig, String> {
    with_panic_guard("loading profiles", || {
        ProfilesConfig::load().map_err(|e| e.to_string())
    })
}

/// Save profiles config (always writes TOML: ~/.gwt/profiles.toml)
#[tauri::command]
pub fn save_profiles(config: ProfilesConfig) -> Result<(), String> {
    with_panic_guard("saving profiles", || config.save().map_err(|e| e.to_string()))
}
