//! Agent detection commands

use crate::commands::terminal::builtin_agent_def;
use crate::commands::terminal::{choose_fallback_runner, FallbackRunner};
use crate::state::{AgentVersionsCache, AppState};
use gwt_core::agent::{claude, codex, gemini, AgentInfo};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;
use tauri::State;
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentVersionsInfo {
    pub agent_id: String,
    pub package: String,
    pub tags: Vec<String>,
    pub versions: Vec<String>,
    /// "cache" | "registry" | "fallback"
    pub source: String,
}

fn encode_npm_package_for_url(package: &str) -> String {
    // npm registry expects scoped packages to be URL-encoded.
    // Example: "@openai/codex" -> "%40openai%2Fcodex"
    package.replace('@', "%40").replace('/', "%2F")
}

fn parse_npm_versions(doc: &serde_json::Value, max_versions: usize) -> (Vec<String>, Vec<String>) {
    let mut tags: Vec<String> = doc
        .get("dist-tags")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    // Prefer "latest" first when present.
    tags.sort();
    if let Some(pos) = tags.iter().position(|t| t == "latest") {
        let latest = tags.remove(pos);
        tags.insert(0, latest);
    }

    let mut versions: Vec<String> = doc
        .get("versions")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    let times: HashMap<&str, &str> = doc
        .get("time")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.as_str(), s)))
                .collect()
        })
        .unwrap_or_default();

    // Sort by publish time (ISO string; lex order matches chronological order), then by version.
    versions.sort_by(|a, b| {
        let ta = times.get(a.as_str()).copied().unwrap_or("");
        let tb = times.get(b.as_str()).copied().unwrap_or("");
        tb.cmp(ta).then_with(|| b.cmp(a))
    });

    if versions.len() > max_versions {
        versions.truncate(max_versions);
    }

    (tags, versions)
}

fn fetch_npm_versions(package: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let encoded = encode_npm_package_for_url(package);
    let url = format!("https://registry.npmjs.org/{encoded}");

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to fetch npm metadata: {e}"))?
        .error_for_status()
        .map_err(|e| format!("npm registry returned error: {e}"))?;

    let doc = resp
        .json::<serde_json::Value>()
        .map_err(|e| format!("Failed to parse npm metadata: {e}"))?;

    Ok(parse_npm_versions(&doc, 200))
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

    fn detect_opencode() -> Option<AgentInfo> {
        let path = which("opencode").ok()?;
        let version = gwt_core::agent::get_command_version("opencode", "--version")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "unknown".to_string());

        Some(AgentInfo {
            name: "OpenCode".to_string(),
            version,
            path: Some(path),
            // OpenCode can use multiple providers; we avoid false negatives here.
            authenticated: true,
        })
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
        map_with_fallback(
            "opencode",
            "OpenCode",
            detect_opencode(),
            runner,
            bunx_path.as_deref(),
            npx_path.as_deref(),
        ),
    ]
}

/// List available npm versions for the agent's bunx package.
///
/// - Uses an in-memory cache in AppState to avoid repeated registry calls.
/// - Returns a fallback of `latest` only when the registry is unreachable.
#[tauri::command]
pub fn list_agent_versions(
    agent_id: String,
    state: State<AppState>,
) -> Result<AgentVersionsInfo, String> {
    let agent_id = agent_id.trim().to_string();
    if agent_id.is_empty() {
        return Err("Agent is required".to_string());
    }

    if let Ok(cache) = state.agent_versions_cache.lock() {
        if let Some(cached) = cache.get(&agent_id) {
            let def = builtin_agent_def(&agent_id)?;
            return Ok(AgentVersionsInfo {
                agent_id,
                package: def.bunx_package.to_string(),
                tags: cached.tags.clone(),
                versions: cached.versions.clone(),
                source: "cache".to_string(),
            });
        }
    }

    let def = builtin_agent_def(&agent_id)?;
    let package = def.bunx_package;

    let (tags, versions, source) = match fetch_npm_versions(package) {
        Ok((tags, versions)) => (tags, versions, "registry"),
        Err(_) => (vec!["latest".to_string()], Vec::new(), "fallback"),
    };

    if let Ok(mut cache) = state.agent_versions_cache.lock() {
        cache.insert(
            agent_id.clone(),
            AgentVersionsCache {
                tags: tags.clone(),
                versions: versions.clone(),
            },
        );
    }

    Ok(AgentVersionsInfo {
        agent_id,
        package: package.to_string(),
        tags,
        versions,
        source: source.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encode_npm_package_for_url_encodes_scoped_package() {
        assert_eq!(
            encode_npm_package_for_url("@openai/codex"),
            "%40openai%2Fcodex"
        );
        assert_eq!(encode_npm_package_for_url("opencode-ai"), "opencode-ai");
    }

    #[test]
    fn parse_npm_versions_prefers_latest_tag_and_sorts_by_time() {
        let doc = json!({
            "dist-tags": { "next": "2.0.0", "latest": "1.1.0" },
            "versions": { "1.0.0": {}, "1.1.0": {}, "2.0.0": {} },
            "time": {
                "created": "2020-01-01T00:00:00.000Z",
                "modified": "2020-01-03T00:00:00.000Z",
                "1.0.0": "2020-01-01T00:00:00.000Z",
                "1.1.0": "2020-01-02T00:00:00.000Z",
                "2.0.0": "2020-01-03T00:00:00.000Z"
            }
        });

        let (tags, versions) = parse_npm_versions(&doc, 200);
        assert_eq!(tags[0], "latest");
        assert_eq!(versions[0], "2.0.0");
        assert_eq!(versions[1], "1.1.0");
        assert_eq!(versions[2], "1.0.0");
    }
}
