//! Agent detection commands

use gwt_core::agent::{claude, codex, gemini, AgentInfo};
use serde::Serialize;

/// Serializable agent info for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedAgentInfo {
    /// Stable id used for launching (maps to executable name)
    pub id: String,
    pub name: String,
    pub version: String,
    pub path: Option<String>,
    pub authenticated: bool,
    pub available: bool,
}

/// Detect available coding agents
#[tauri::command]
pub fn detect_agents() -> Vec<DetectedAgentInfo> {
    fn map(id: &str, fallback_name: &str, info: Option<AgentInfo>) -> DetectedAgentInfo {
        match info {
            Some(info) => DetectedAgentInfo {
                id: id.to_string(),
                name: info.name,
                version: info.version,
                path: info.path.map(|p| p.to_string_lossy().to_string()),
                authenticated: info.authenticated,
                available: true,
            },
            None => DetectedAgentInfo {
                id: id.to_string(),
                name: fallback_name.to_string(),
                version: "not installed".to_string(),
                path: None,
                authenticated: false,
                available: false,
            },
        }
    }

    vec![
        map("claude", "Claude Code", claude::ClaudeAgent::detect()),
        map("codex", "Codex", codex::CodexAgent::detect()),
        map("gemini", "Gemini", gemini::GeminiAgent::detect()),
    ]
}
