//! Profiles (env + AI settings) management commands

use gwt_core::ai::{format_error_for_display, AIClient, ModelInfo};
use gwt_core::config::ProfilesConfig;
use gwt_core::StructuredError;
use std::panic::{catch_unwind, AssertUnwindSafe};
use tauri::AppHandle;
use tracing::error;

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
                "Unexpected panic while handling profiles command"
            );
            Err(StructuredError::internal(
                &format!("Unexpected error while {}", context),
                command,
            ))
        }
    }
}

/// Get current profiles config (global: ~/.gwt/config.toml [profiles]).
#[tauri::command]
pub fn get_profiles() -> Result<ProfilesConfig, StructuredError> {
    with_panic_guard("loading profiles", "get_profiles", || {
        ProfilesConfig::load().map_err(|e| StructuredError::from_gwt_error(&e, "get_profiles"))
    })
}

/// Save profiles config (writes into ~/.gwt/config.toml [profiles]).
#[tauri::command]
pub fn save_profiles(config: ProfilesConfig, app_handle: AppHandle) -> Result<(), StructuredError> {
    with_panic_guard("saving profiles", "save_profiles", || {
        config
            .save()
            .map_err(|e| StructuredError::from_gwt_error(&e, "save_profiles"))?;
        let _ = crate::menu::rebuild_menu(&app_handle);
        Ok(())
    })
}

/// List AI models from a specific OpenAI-compatible endpoint (`GET /models`).
#[tauri::command]
pub fn list_ai_models(
    endpoint: String,
    api_key: String,
) -> Result<Vec<ModelInfo>, StructuredError> {
    with_panic_guard("listing ai models", "list_ai_models", || {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            return Err(StructuredError::internal(
                "Endpoint is required",
                "list_ai_models",
            ));
        }

        let client = AIClient::new_for_list_models(endpoint, api_key.trim()).map_err(|e| {
            StructuredError::internal(&format_error_for_display(&e), "list_ai_models")
        })?;
        let mut models = client.list_models().map_err(|e| {
            StructuredError::internal(&format_error_for_display(&e), "list_ai_models")
        })?;
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
        assert!(err.message.contains("Endpoint is required"));
    }

    #[test]
    fn list_ai_models_rejects_invalid_endpoint() {
        let err = list_ai_models("not-a-url".to_string(), String::new()).unwrap_err();
        assert!(
            err.message.contains("Invalid endpoint"),
            "unexpected error message: {}",
            err.message
        );
    }
}
