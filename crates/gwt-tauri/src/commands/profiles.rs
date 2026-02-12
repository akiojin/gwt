//! Profiles (env + AI settings) management commands

use gwt_core::ai::{format_error_for_display, AIClient, ModelInfo};
use gwt_core::config::ProfilesConfig;
use std::panic::{catch_unwind, AssertUnwindSafe};
use tauri::AppHandle;
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
pub fn save_profiles(config: ProfilesConfig, app_handle: AppHandle) -> Result<(), String> {
    with_panic_guard("saving profiles", || {
        config.save().map_err(|e| e.to_string())?;
        let _ = crate::menu::rebuild_menu(&app_handle);
        Ok(())
    })
}

/// List AI models from a specific OpenAI-compatible endpoint (`GET /models`).
#[tauri::command]
pub fn list_ai_models(endpoint: String, api_key: String) -> Result<Vec<ModelInfo>, String> {
    with_panic_guard("listing ai models", || {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            return Err("Endpoint is required".to_string());
        }

        let client = AIClient::new_for_list_models(endpoint, api_key.trim())
            .map_err(|e| format_error_for_display(&e))?;
        let mut models = client
            .list_models()
            .map_err(|e| format_error_for_display(&e))?;
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models.dedup_by(|a, b| a.id == b.id);
        Ok(models)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_ai_models_rejects_empty_endpoint() {
        let err = list_ai_models("   ".to_string(), String::new()).unwrap_err();
        assert!(err.contains("Endpoint is required"));
    }

    #[test]
    fn list_ai_models_rejects_invalid_endpoint() {
        let err = list_ai_models("not-a-url".to_string(), String::new()).unwrap_err();
        assert!(
            err.contains("Invalid endpoint"),
            "unexpected error message: {}",
            err
        );
    }
}
