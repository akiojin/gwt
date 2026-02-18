//! Recent projects Tauri commands

use gwt_core::config;
use serde::Serialize;

/// Recent project entry returned to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProjectEntry {
    pub path: String,
    pub last_opened: String,
}

/// Get recent projects (most recent 10 entries).
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
