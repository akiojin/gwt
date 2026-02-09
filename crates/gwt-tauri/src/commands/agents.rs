//! Agent detection commands

use crate::commands::terminal::{choose_fallback_runner, FallbackRunner};
use gwt_core::agent::{claude, codex, gemini, AgentInfo};
use serde::Serialize;
use which::which;

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
    let bunx_path = which("bunx").ok().map(|p| p.to_string_lossy().to_string());
    let npx_path = which("npx").ok().map(|p| p.to_string_lossy().to_string());
    let runner = choose_fallback_runner(bunx_path.as_deref(), npx_path.is_some());

    fn fallback_version(runner: FallbackRunner) -> &'static str {
        match runner {
            FallbackRunner::Bunx => "bunx",
            FallbackRunner::Npx => "npx",
        }
    }

    fn fallback_path(
        runner: FallbackRunner,
        bunx_path: Option<&str>,
        npx_path: Option<&str>,
    ) -> Option<String> {
        match runner {
            FallbackRunner::Bunx => bunx_path.map(|s| s.to_string()),
            FallbackRunner::Npx => npx_path.map(|s| s.to_string()),
        }
    }

    fn map_with_fallback(
        id: &str,
        fallback_name: &str,
        info: Option<AgentInfo>,
        runner: Option<FallbackRunner>,
        bunx_path: Option<&str>,
        npx_path: Option<&str>,
    ) -> DetectedAgentInfo {
        if let Some(info) = info {
            return DetectedAgentInfo {
                id: id.to_string(),
                name: info.name,
                version: info.version,
                path: info.path.map(|p| p.to_string_lossy().to_string()),
                authenticated: info.authenticated,
                available: true,
            };
        }

        if let Some(runner) = runner {
            let authenticated = match id {
                "claude" => std::env::var("ANTHROPIC_API_KEY").is_ok(),
                "codex" => std::env::var("OPENAI_API_KEY").is_ok(),
                "gemini" => {
                    std::env::var("GOOGLE_API_KEY").is_ok()
                        || std::env::var("GEMINI_API_KEY").is_ok()
                }
                _ => false,
            };

            return DetectedAgentInfo {
                id: id.to_string(),
                name: fallback_name.to_string(),
                version: fallback_version(runner).to_string(),
                path: fallback_path(runner, bunx_path, npx_path),
                authenticated,
                available: true,
            };
        }

        DetectedAgentInfo {
            id: id.to_string(),
            name: fallback_name.to_string(),
            version: "not installed".to_string(),
            path: None,
            authenticated: false,
            available: false,
        }
    }

    vec![
        map_with_fallback(
            "claude",
            "Claude Code",
            claude::ClaudeAgent::detect(),
            runner,
            bunx_path.as_deref(),
            npx_path.as_deref(),
        ),
        map_with_fallback(
            "codex",
            "Codex",
            codex::CodexAgent::detect(),
            runner,
            bunx_path.as_deref(),
            npx_path.as_deref(),
        ),
        map_with_fallback(
            "gemini",
            "Gemini",
            gemini::GeminiAgent::detect(),
            runner,
            bunx_path.as_deref(),
            npx_path.as_deref(),
        ),
    ]
}
