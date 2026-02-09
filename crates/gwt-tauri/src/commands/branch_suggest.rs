//! AI branch name suggestion command (GUI Launch Agent)
//!
//! Mirrors the TUI branch naming assistant behavior (SPEC-1ad9c07d) and exposes it to the GUI.

use gwt_core::ai::{
    format_error_for_display, suggest_branch_names as core_suggest_branch_names, AIClient,
};
use gwt_core::config::{ProfilesConfig, ResolvedAISettings};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSuggestResult {
    /// "ok" | "ai-not-configured" | "error"
    pub status: String,
    /// Suggestions are full branch names with prefix (e.g., "feature/foo").
    pub suggestions: Vec<String>,
    pub error: Option<String>,
}

fn resolve_active_ai_settings(config: &ProfilesConfig) -> Option<ResolvedAISettings> {
    // Match the TUI behavior: active profile AI overrides default_ai.
    if let Some(profile) = config.active_profile() {
        if let Some(settings) = profile.ai.as_ref() {
            if settings.is_enabled() {
                return Some(settings.resolved());
            }
        }
    }
    if let Some(settings) = config.default_ai.as_ref() {
        if settings.is_enabled() {
            return Some(settings.resolved());
        }
    }
    None
}

/// Suggest 3 branch names for "New Branch Name".
///
/// This never writes config/history files.
#[tauri::command]
pub fn suggest_branch_names(description: String) -> Result<BranchSuggestResult, String> {
    let description = description.trim().to_string();
    if description.is_empty() {
        return Err("Description is required".to_string());
    }

    let profiles = ProfilesConfig::load().map_err(|e| e.to_string())?;
    let Some(settings) = resolve_active_ai_settings(&profiles) else {
        return Ok(BranchSuggestResult {
            status: "ai-not-configured".to_string(),
            suggestions: Vec::new(),
            error: None,
        });
    };

    let client = AIClient::new(settings).map_err(|e| e.to_string())?;
    match core_suggest_branch_names(&client, &description) {
        Ok(suggestions) => Ok(BranchSuggestResult {
            status: "ok".to_string(),
            suggestions,
            error: None,
        }),
        Err(err) => Ok(BranchSuggestResult {
            status: "error".to_string(),
            suggestions: Vec::new(),
            error: Some(format_error_for_display(&err)),
        }),
    }
}
