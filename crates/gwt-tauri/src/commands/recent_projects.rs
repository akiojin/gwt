//! Recent projects Tauri commands

use gwt_core::config;
use serde::Serialize;
use tracing::instrument;

/// Recent project entry returned to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProjectEntry {
    pub path: String,
    pub last_opened: String,
}

/// Get recent projects (most recent 10 entries).
#[instrument(skip_all, fields(command = "get_recent_projects"))]
#[tauri::command]
pub fn get_recent_projects() -> Vec<RecentProjectEntry> {
    config::load_recent_projects()
        .into_iter()
        .take(10)
        .map(|p| RecentProjectEntry {
            path: p.path,
            last_opened: p.last_opened.to_rfc3339(),
        })
        .collect()
}
