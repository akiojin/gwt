//! AI branch name suggestion command (GUI Launch Agent)

use gwt_core::ai::{
    format_error_for_display, suggest_branch_name as core_suggest_branch_name, AIClient,
};
use gwt_core::config::ProfilesConfig;
use gwt_core::StructuredError;
use serde::Serialize;
use tracing::instrument;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSuggestResult {
    /// "ok" | "ai-not-configured" | "error"
    pub status: String,
    /// Full branch name with prefix (e.g., "feature/foo").
    pub suggestion: String,
    pub error: Option<String>,
}

/// Suggest 1 branch name for "New Branch Name".
///
/// This never writes config/history files.
#[instrument(skip_all, fields(command = "suggest_branch_name"))]
#[tauri::command]
pub fn suggest_branch_name(description: String) -> Result<BranchSuggestResult, StructuredError> {
    let description = description.trim().to_string();
    if description.is_empty() {
        return Err(StructuredError::internal(
            "Description is required",
            "suggest_branch_name",
        ));
    }

    let profiles = ProfilesConfig::load()
        .map_err(|e| StructuredError::from_gwt_error(&e, "suggest_branch_name"))?;
    let ai = profiles.resolve_active_ai_settings();
    let Some(settings) = ai.resolved else {
        return Ok(BranchSuggestResult {
            status: "ai-not-configured".to_string(),
            suggestion: String::new(),
            error: None,
        });
    };

    let client = AIClient::new(settings)
        .map_err(|e| StructuredError::internal(&e.to_string(), "suggest_branch_name"))?;
    match core_suggest_branch_name(&client, &description) {
        Ok(suggestion) => Ok(BranchSuggestResult {
            status: "ok".to_string(),
            suggestion,
            error: None,
        }),
        Err(err) => Ok(BranchSuggestResult {
            status: "error".to_string(),
            suggestion: String::new(),
            error: Some(format_error_for_display(&err)),
        }),
    }
}

/// Check whether AI configuration is available without invoking model inference.
#[instrument(skip_all, fields(command = "is_ai_configured"))]
#[tauri::command]
pub fn is_ai_configured() -> Result<bool, StructuredError> {
    let profiles = ProfilesConfig::load()
        .map_err(|e| StructuredError::from_gwt_error(&e, "is_ai_configured"))?;
    Ok(profiles.resolve_active_ai_settings().resolved.is_some())
}
