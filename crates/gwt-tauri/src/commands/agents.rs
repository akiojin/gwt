//! Agent detection commands

use crate::state::AppState;
use gwt_core::agent::AgentManager;
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

/// Serializable agent info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct DetectedAgentInfo {
    pub name: String,
    pub version: String,
    pub path: Option<String>,
    pub authenticated: bool,
}

/// Detect available coding agents
#[tauri::command]
pub fn detect_agents(state: State<AppState>) -> Vec<DetectedAgentInfo> {
    let working_dir = {
        let project_path = match state.project_path.lock() {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };
        match project_path.as_ref() {
            Some(p) => PathBuf::from(p),
            None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    };

    let manager = AgentManager::new(&working_dir);
    let agents = manager.detect_agents();

    agents
        .into_iter()
        .map(|info| DetectedAgentInfo {
            name: info.name,
            version: info.version,
            path: info.path.map(|p| p.to_string_lossy().to_string()),
            authenticated: info.authenticated,
        })
        .collect()
}
