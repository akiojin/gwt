//! TypeScript session file compatibility (FR-069, FR-070, SPEC-a3f4c9df)
//!
//! Reads and writes session history from/to session files.
//! Supports both TOML (new) and JSON (legacy) formats.
//!
//! File locations:
//! - New: ~/.gwt/sessions/{repoName}_{hash}.toml (SPEC-a3f4c9df FR-014)
//! - Legacy: ~/.config/gwt/sessions/{repoName}_{hash}.json
//!
//! Migration strategy:
//! - TOML format is preferred for reading if it exists
//! - JSON format is read as fallback
//! - Writes always use TOML format to ~/.gwt/sessions/

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Tool session entry from TypeScript format (FR-069, FR-070, SPEC-a3f4c9df)
///
/// Supports both camelCase (JSON legacy) and snake_case (TOML new) via serde aliases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSessionEntry {
    pub branch: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "worktreePath"
    )]
    pub worktree_path: Option<String>,
    #[serde(alias = "toolId")]
    pub tool_id: String,
    #[serde(alias = "toolLabel")]
    pub tool_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "sessionId")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "reasoningLevel"
    )]
    pub reasoning_level: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "skipPermissions"
    )]
    pub skip_permissions: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "toolVersion"
    )]
    pub tool_version: Option<String>,
    /// collaboration_modes enabled (Codex v0.91.0+, SPEC-fdebd681)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "collaborationModes"
    )]
    pub collaboration_modes: Option<bool>,
    /// Docker service name (compose) for Quick Start
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "dockerService"
    )]
    pub docker_service: Option<String>,
    /// Force host launch (skip docker) for Quick Start
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "dockerForceHost"
    )]
    pub docker_force_host: Option<bool>,
    /// Force recreate containers for Quick Start
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "dockerRecreate"
    )]
    pub docker_recreate: Option<bool>,
    /// Build docker images before launch for Quick Start
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "dockerBuild"
    )]
    pub docker_build: Option<bool>,
    /// Keep containers running after agent exit for Quick Start
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "dockerKeep")]
    pub docker_keep: Option<bool>,
    /// Container name used for docker compose/dockerfile launch
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "dockerContainerName"
    )]
    pub docker_container_name: Option<String>,
    /// Docker compose args used when launch was started
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "dockerComposeArgs"
    )]
    pub docker_compose_args: Option<Vec<String>>,
    /// Unix timestamp in milliseconds
    pub timestamp: i64,
}

impl ToolSessionEntry {
    /// Format tool usage for display (FR-070)
    /// Returns: "ToolLabel@version"
    pub fn format_tool_usage(&self) -> String {
        let version = self.tool_version.as_deref().unwrap_or("latest");
        let label = short_tool_label(Some(&self.tool_id), &self.tool_label);
        format!("{}@{}", label, version)
    }

    /// Get timestamp as DateTime
    pub fn datetime(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(self.timestamp)
            .single()
            .unwrap_or_else(Utc::now)
    }
}

fn short_tool_label(tool_id: Option<&str>, tool_label: &str) -> String {
    let id = tool_id.unwrap_or("");
    let id_lower = id.to_lowercase();
    if id_lower.contains("claude") {
        return "Claude".to_string();
    }
    if id_lower.contains("codex") {
        return "Codex".to_string();
    }
    if id_lower.contains("gemini") {
        return "Gemini".to_string();
    }
    if id_lower.contains("opencode") || id_lower.contains("open-code") {
        return "OpenCode".to_string();
    }

    let label_lower = tool_label.to_lowercase();
    if label_lower.contains("claude") {
        return "Claude".to_string();
    }
    if label_lower.contains("codex") {
        return "Codex".to_string();
    }
    if label_lower.contains("gemini") {
        return "Gemini".to_string();
    }
    if label_lower.contains("opencode") || label_lower.contains("open-code") {
        return "OpenCode".to_string();
    }

    tool_label.to_string()
}

fn canonical_tool_id(tool_id: &str) -> String {
    let trimmed = tool_id.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lower = trimmed.to_lowercase();
    match lower.as_str() {
        "claude" | "claude-code" => return "claude-code".to_string(),
        "codex" | "codex-cli" => return "codex-cli".to_string(),
        "gemini" | "gemini-cli" => return "gemini-cli".to_string(),
        "opencode" | "open-code" => return "opencode".to_string(),
        _ => {}
    }

    let normalized: String = lower
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    match normalized.as_str() {
        "claude" | "claudecode" => "claude-code".to_string(),
        "codex" | "codexcli" => "codex-cli".to_string(),
        "gemini" | "geminicli" => "gemini-cli".to_string(),
        "opencode" => "opencode".to_string(),
        _ => trimmed.to_string(),
    }
}

/// TypeScript session data structure (SPEC-a3f4c9df)
///
/// Supports both camelCase (JSON legacy) and snake_case (TOML new) via serde aliases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TsSessionData {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "lastWorktreePath"
    )]
    pub last_worktree_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "lastBranch")]
    pub last_branch: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "lastUsedTool"
    )]
    pub last_used_tool: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "lastSessionId"
    )]
    pub last_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolLabel")]
    pub tool_label: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "toolVersion"
    )]
    pub tool_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub timestamp: i64,
    #[serde(alias = "repositoryRoot")]
    pub repository_root: String,
    #[serde(default)]
    pub history: Vec<ToolSessionEntry>,
}

/// Get the session file name components (repo_name and hash)
fn get_session_file_components(repo_root: &Path) -> (String, String) {
    let repo_name = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo")
        .to_string();

    // Match TypeScript: Buffer.from(repositoryRoot).toString("base64").replace(/[/+=]/g, "_")
    let repo_path_str = repo_root.to_string_lossy();
    let hash = STANDARD.encode(repo_path_str.as_bytes());
    let hash_safe = hash.replace(['/', '+', '='], "_");

    (repo_name, hash_safe)
}

/// Get the new TOML session file path for a repository (SPEC-a3f4c9df FR-014)
/// Path: ~/.gwt/sessions/{repoName}_{hash}.toml
pub fn get_ts_session_toml_path(repo_root: &Path) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let session_dir = home.join(".gwt").join("sessions");
    let (repo_name, hash_safe) = get_session_file_components(repo_root);
    session_dir.join(format!("{}_{}.toml", repo_name, hash_safe))
}

/// Get the legacy JSON session file path for a repository
/// Path: ~/.config/gwt/sessions/{repoName}_{hash}.json
pub fn get_ts_session_json_path(repo_root: &Path) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let session_dir = home.join(".config").join("gwt").join("sessions");
    let (repo_name, hash_safe) = get_session_file_components(repo_root);
    session_dir.join(format!("{}_{}.json", repo_name, hash_safe))
}

/// Get the TypeScript session file path for a repository (legacy JSON path)
/// Deprecated: Use get_ts_session_toml_path for new code
pub fn get_ts_session_path(repo_root: &Path) -> PathBuf {
    get_ts_session_json_path(repo_root)
}

/// Check if migration is needed for a repository's session file (SPEC-a3f4c9df)
pub fn needs_ts_session_migration(repo_root: &Path) -> bool {
    let toml_path = get_ts_session_toml_path(repo_root);
    let json_path = get_ts_session_json_path(repo_root);
    json_path.exists() && !toml_path.exists()
}

/// Migrate JSON session to TOML if needed (SPEC-a3f4c9df FR-014)
pub fn migrate_ts_session_if_needed(repo_root: &Path) -> Result<bool, std::io::Error> {
    if !needs_ts_session_migration(repo_root) {
        return Ok(false);
    }

    let json_path = get_ts_session_json_path(repo_root);
    let toml_path = get_ts_session_toml_path(repo_root);

    debug!(
        category = "config",
        json_path = %json_path.display(),
        toml_path = %toml_path.display(),
        "Starting TS session JSON to TOML migration"
    );

    // Read and parse JSON
    let json_content = std::fs::read_to_string(&json_path)?;
    let session: TsSessionData = serde_json::from_str(&json_content).map_err(|e| {
        warn!(
            category = "config",
            json_path = %json_path.display(),
            error = %e,
            "Failed to parse JSON session file"
        );
        std::io::Error::other(format!("Failed to parse JSON: {}", e))
    })?;

    // Convert to TOML
    let toml_content = toml::to_string_pretty(&session).map_err(|e| {
        warn!(
            category = "config",
            error = %e,
            "Failed to convert session to TOML"
        );
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    })?;

    // Ensure parent directory exists
    if let Some(parent) = toml_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write TOML file atomically
    crate::config::write_atomic(&toml_path, &toml_content)
        .map_err(|e| std::io::Error::other(format!("Failed to write TOML: {}", e)))?;

    info!(
        category = "config",
        operation = "migration",
        json_path = %json_path.display(),
        toml_path = %toml_path.display(),
        "TS session migration completed successfully"
    );

    Ok(true)
}

/// Load TypeScript session data for a repository (SPEC-a3f4c9df)
///
/// Priority: TOML (new) > JSON (legacy)
pub fn load_ts_session(repo_root: &Path) -> Option<TsSessionData> {
    // Try TOML first (new format)
    let toml_path = get_ts_session_toml_path(repo_root);
    if let Ok(content) = std::fs::read_to_string(&toml_path) {
        if let Ok(session) = toml::from_str::<TsSessionData>(&content) {
            debug!(
                category = "config",
                path = %toml_path.display(),
                "Loaded session from TOML"
            );
            return Some(normalize_and_persist_session(toml_path, session, true));
        }
    }

    // Fallback to JSON (legacy format) and auto-migrate
    let json_path = get_ts_session_json_path(repo_root);
    if let Ok(content) = std::fs::read_to_string(&json_path) {
        if let Ok(session) = serde_json::from_str::<TsSessionData>(&content) {
            debug!(
                category = "config",
                path = %json_path.display(),
                "Loaded session from JSON (legacy)"
            );
            // Auto-migrate: save as TOML for next time (SPEC-a3f4c9df)
            if let Ok(toml_content) = toml::to_string_pretty(&session) {
                if let Some(parent) = toml_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if crate::config::write_atomic(&toml_path, &toml_content).is_ok() {
                    info!(
                        category = "config",
                        operation = "auto_migrate",
                        "Auto-migrated session history JSON to TOML"
                    );
                    // Return from TOML path after migration
                    return Some(normalize_and_persist_session(toml_path, session, true));
                }
            }
            return Some(normalize_and_persist_session(json_path, session, false));
        }
    }

    // Fallback: if the exact hash path is missing or invalid, try to locate
    // the latest session file that matches the repo name.
    let repo_name = repo_root.file_name()?.to_string_lossy().to_string();
    let prefix = format!("{}_", repo_name);
    let mut latest: Option<(TsSessionData, PathBuf, bool)> = None;

    // Search in TOML directory first
    let toml_dir = toml_path.parent()?;
    if toml_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(toml_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let file_name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(name) => name,
                    None => continue,
                };
                if !file_name.starts_with(&prefix) || !file_name.ends_with(".toml") {
                    continue;
                }
                let content = match std::fs::read_to_string(&path) {
                    Ok(content) => content,
                    Err(_) => continue,
                };
                let session: TsSessionData = match toml::from_str(&content) {
                    Ok(session) => session,
                    Err(_) => continue,
                };
                let should_update = latest
                    .as_ref()
                    .map(|current| session.timestamp > current.0.timestamp)
                    .unwrap_or(true);
                if should_update {
                    latest = Some((session, path, true));
                }
            }
        }
    }

    // If no TOML found, search in JSON directory
    if latest.is_none() {
        let json_dir = json_path.parent()?;
        if json_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(json_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let file_name = match path.file_name().and_then(|n| n.to_str()) {
                        Some(name) => name,
                        None => continue,
                    };
                    if !file_name.starts_with(&prefix) || !file_name.ends_with(".json") {
                        continue;
                    }
                    let content = match std::fs::read_to_string(&path) {
                        Ok(content) => content,
                        Err(_) => continue,
                    };
                    let session: TsSessionData = match serde_json::from_str(&content) {
                        Ok(session) => session,
                        Err(_) => continue,
                    };
                    let should_update = latest
                        .as_ref()
                        .map(|current| session.timestamp > current.0.timestamp)
                        .unwrap_or(true);
                    if should_update {
                        latest = Some((session, path, false));
                    }
                }
            }
        }
    }

    latest.map(|(session, path, is_toml)| normalize_and_persist_session(path, session, is_toml))
}

fn normalize_and_persist_session(
    session_path: PathBuf,
    mut session: TsSessionData,
    is_toml: bool,
) -> TsSessionData {
    let mut changed = false;

    if let Some(last_used_tool) = session.last_used_tool.as_mut() {
        let canonical = canonical_tool_id(last_used_tool);
        if canonical != *last_used_tool {
            *last_used_tool = canonical;
            changed = true;
        }
    }

    for entry in session.history.iter_mut() {
        let canonical = canonical_tool_id(&entry.tool_id);
        if canonical != entry.tool_id {
            entry.tool_id = canonical;
            changed = true;
        }
    }

    if changed {
        if is_toml {
            if let Ok(content) = toml::to_string_pretty(&session) {
                let _ = std::fs::write(&session_path, content);
            }
        } else if let Ok(content) = serde_json::to_string_pretty(&session) {
            let _ = std::fs::write(&session_path, content);
        }
    }

    session
}

/// Save session entry to session file (FR-069, SPEC-a3f4c9df)
///
/// Adds a new entry to the session history and updates last-used fields.
/// Creates the session file if it doesn't exist.
/// Always saves in TOML format to ~/.gwt/sessions/ (SPEC-a3f4c9df FR-006, FR-014)
pub fn save_session_entry(
    repo_root: &Path,
    mut entry: ToolSessionEntry,
) -> Result<(), std::io::Error> {
    // Resolve to main repo root for consistency
    let main_root = crate::git::get_main_repo_root(repo_root);
    let session_path = get_ts_session_toml_path(&main_root);

    // Load existing session or create new one
    let mut session = load_ts_session(&main_root).unwrap_or_else(|| TsSessionData {
        last_worktree_path: None,
        last_branch: None,
        last_used_tool: None,
        last_session_id: None,
        tool_label: None,
        tool_version: None,
        model: None,
        timestamp: Utc::now().timestamp_millis(),
        repository_root: main_root.to_string_lossy().to_string(),
        history: Vec::new(),
    });

    let canonical_id = canonical_tool_id(&entry.tool_id);
    if canonical_id != entry.tool_id {
        entry.tool_id = canonical_id;
    }

    // Update last-used fields
    session.last_worktree_path = entry.worktree_path.clone();
    session.last_branch = Some(entry.branch.clone());
    session.last_used_tool = Some(entry.tool_id.clone());
    session.last_session_id = entry.session_id.clone();
    session.tool_label = Some(entry.tool_label.clone());
    session.tool_version = entry.tool_version.clone();
    session.model = entry.model.clone();
    session.timestamp = entry.timestamp;

    // Add entry to history
    session.history.push(entry);

    // Ensure parent directory exists
    if let Some(parent) = session_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write to file in TOML format (SPEC-a3f4c9df FR-006)
    let content = toml::to_string_pretty(&session)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    crate::config::write_atomic(&session_path, &content)
        .map_err(|e| std::io::Error::other(format!("Failed to write session: {}", e)))?;

    debug!(
        category = "config",
        path = %session_path.display(),
        "Saved session in TOML format"
    );

    Ok(())
}

/// Get the last tool usage for each branch from TypeScript session history
/// Returns a map of branch name -> ToolSessionEntry
///
/// This function automatically resolves worktree paths to the main repository root
/// before looking up the session file, ensuring compatibility with TypeScript session files
/// which are keyed by main repository path.
pub fn get_last_tool_usage_map(repo_root: &Path) -> HashMap<String, ToolSessionEntry> {
    let mut map = HashMap::new();

    // Resolve worktree path to main repository root (FR-070)
    // TypeScript session files are keyed by main repo path, not worktree path
    let main_root = crate::git::get_main_repo_root(repo_root);

    let session = match load_ts_session(&main_root) {
        Some(s) => s,
        None => return map,
    };

    // Process history entries
    for entry in session.history {
        let mut entry = entry;
        let canonical_id = canonical_tool_id(&entry.tool_id);
        if canonical_id != entry.tool_id {
            entry.tool_id = canonical_id;
        }

        let existing = map.get(&entry.branch);
        if existing.is_none() || existing.unwrap().timestamp < entry.timestamp {
            map.insert(entry.branch.clone(), entry);
        }
    }

    // Backward compatibility: if no history but last_branch exists
    if map.is_empty() {
        if let (Some(branch), Some(worktree_path)) =
            (session.last_branch, session.last_worktree_path)
        {
            let fallback_tool_id =
                canonical_tool_id(&session.last_used_tool.clone().unwrap_or_default());
            let entry = ToolSessionEntry {
                branch: branch.clone(),
                worktree_path: Some(worktree_path),
                tool_id: fallback_tool_id.clone(),
                tool_label: session
                    .tool_label
                    .or(session.last_used_tool)
                    .unwrap_or_else(|| "Custom".to_string()),
                session_id: session.last_session_id,
                mode: None,
                model: session.model,
                reasoning_level: None,
                skip_permissions: None,
                tool_version: session.tool_version,
                collaboration_modes: None,
                docker_service: None,
                docker_force_host: None,
                docker_recreate: None,
                docker_build: None,
                docker_keep: None,
                docker_container_name: None,
                docker_compose_args: None,
                timestamp: session.timestamp,
            };
            map.insert(branch, entry);
        }
    }

    map
}

/// Get session history entries for a specific branch, grouped by tool (FR-050)
/// Returns the latest entry for each tool that was used on this branch
/// This is used for the Quick Start feature
pub fn get_branch_tool_history(repo_root: &Path, branch: &str) -> Vec<ToolSessionEntry> {
    let main_root = crate::git::get_main_repo_root(repo_root);
    let session = match load_ts_session(&main_root) {
        Some(s) => s,
        None => return vec![],
    };

    // Collect entries for this branch, keeping only the latest per tool
    let mut tool_map: HashMap<String, ToolSessionEntry> = HashMap::new();
    let mut last_skip_permissions: HashMap<String, (i64, bool)> = HashMap::new();
    let TsSessionData {
        history,
        last_worktree_path,
        last_branch,
        last_used_tool,
        last_session_id,
        tool_label,
        tool_version,
        model,
        timestamp,
        ..
    } = session;

    for entry in history {
        if entry.branch == branch {
            let mut entry = entry;
            let canonical_id = canonical_tool_id(&entry.tool_id);
            if canonical_id != entry.tool_id {
                entry.tool_id = canonical_id;
            }

            if let Some(skip) = entry.skip_permissions {
                let should_update = last_skip_permissions
                    .get(&entry.tool_id)
                    .map(|(ts, _)| entry.timestamp > *ts)
                    .unwrap_or(true);
                if should_update {
                    last_skip_permissions.insert(entry.tool_id.clone(), (entry.timestamp, skip));
                }
            }

            let existing = tool_map.get(&entry.tool_id);
            if existing.is_none() || existing.unwrap().timestamp < entry.timestamp {
                tool_map.insert(entry.tool_id.clone(), entry);
            }
        }
    }

    for entry in tool_map.values_mut() {
        if entry.skip_permissions.is_none() {
            if let Some((_, skip)) = last_skip_permissions.get(&entry.tool_id) {
                entry.skip_permissions = Some(*skip);
            }
        }
    }

    if tool_map.is_empty() {
        if let Some(last_branch_name) = last_branch {
            if last_branch_name == branch {
                let fallback_tool_id = canonical_tool_id(&last_used_tool.unwrap_or_default());
                let label = tool_label
                    .or_else(|| {
                        if fallback_tool_id.is_empty() {
                            None
                        } else {
                            Some(fallback_tool_id.clone())
                        }
                    })
                    .unwrap_or_else(|| "Custom".to_string());
                let entry = ToolSessionEntry {
                    branch: last_branch_name,
                    worktree_path: last_worktree_path,
                    tool_id: fallback_tool_id,
                    tool_label: label,
                    session_id: last_session_id,
                    mode: None,
                    model,
                    reasoning_level: None,
                    skip_permissions: None,
                    tool_version,
                    collaboration_modes: None,
                    docker_service: None,
                    docker_force_host: None,
                    docker_recreate: None,
                    docker_build: None,
                    docker_keep: None,
                    docker_container_name: None,
                    docker_compose_args: None,
                    timestamp,
                };
                tool_map.insert(entry.tool_id.clone(), entry);
            }
        }
    }

    // Sort by timestamp (most recent first)
    let mut entries: Vec<ToolSessionEntry> = tool_map.into_values().collect();
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn test_session_path_generation() {
        let repo = PathBuf::from("/home/user/projects/myrepo");
        let path = get_ts_session_path(&repo);
        assert!(path.to_string_lossy().contains("sessions"));
        assert!(path.to_string_lossy().contains("myrepo_"));
        assert!(path.to_string_lossy().ends_with(".json"));
    }

    #[test]
    fn test_load_nonexistent_session() {
        let temp = TempDir::new().unwrap();
        let result = load_ts_session(temp.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_load_session_fallback_by_repo_name() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let session_dir = temp.path().join(".config").join("gwt").join("sessions");
        std::fs::create_dir_all(&session_dir).unwrap();

        let repo_root = PathBuf::from("/workspaces/sample-repo");
        let session = TsSessionData {
            last_worktree_path: None,
            last_branch: Some("feature/test".to_string()),
            last_used_tool: Some("codex-cli".to_string()),
            last_session_id: None,
            tool_label: Some("Codex".to_string()),
            tool_version: Some("latest".to_string()),
            model: Some("gpt-5.2-codex".to_string()),
            timestamp: 1_700_000_000_000,
            repository_root: "/workspaces/other-path".to_string(),
            history: Vec::new(),
        };

        // Create a file with a different hash but the same repo name prefix.
        let fallback_path = session_dir.join("sample-repo_other.json");
        let content = serde_json::to_string_pretty(&session).unwrap();
        std::fs::write(&fallback_path, content).unwrap();

        let loaded = load_ts_session(&repo_root).expect("fallback session should load");
        assert_eq!(loaded.timestamp, session.timestamp);
        assert_eq!(loaded.last_branch, session.last_branch);
    }

    #[test]
    fn test_get_branch_tool_history_fallback_to_last_branch() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let repo_root = temp.path().join("sample-repo");
        std::fs::create_dir_all(&repo_root).unwrap();

        let session = TsSessionData {
            last_worktree_path: Some("/path/to/wt".to_string()),
            last_branch: Some("feature/test".to_string()),
            last_used_tool: Some("codex-cli".to_string()),
            last_session_id: Some("session-123".to_string()),
            tool_label: Some("Codex".to_string()),
            tool_version: Some("latest".to_string()),
            model: Some("gpt-5.2-codex".to_string()),
            timestamp: 1_700_000_000_000,
            repository_root: repo_root.to_string_lossy().to_string(),
            history: Vec::new(),
        };

        let session_path = get_ts_session_path(&repo_root);
        if let Some(parent) = session_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let content = serde_json::to_string_pretty(&session).unwrap();
        std::fs::write(&session_path, content).unwrap();

        let entries = get_branch_tool_history(&repo_root, "feature/test");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.branch, "feature/test");
        assert_eq!(entry.tool_id, "codex-cli");
        assert_eq!(entry.tool_label, "Codex");
        assert_eq!(entry.model.as_deref(), Some("gpt-5.2-codex"));
        assert_eq!(entry.tool_version.as_deref(), Some("latest"));
        assert_eq!(entry.session_id.as_deref(), Some("session-123"));
    }

    #[test]
    fn test_get_branch_tool_history_canonicalizes_tool_id() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let repo_root = temp.path().join("sample-repo");
        std::fs::create_dir_all(&repo_root).unwrap();

        let session = TsSessionData {
            last_worktree_path: None,
            last_branch: None,
            last_used_tool: None,
            last_session_id: None,
            tool_label: None,
            tool_version: None,
            model: None,
            timestamp: 1_700_000_000_000,
            repository_root: repo_root.to_string_lossy().to_string(),
            history: vec![
                ToolSessionEntry {
                    branch: "feature/test".to_string(),
                    worktree_path: None,
                    tool_id: "claude".to_string(),
                    tool_label: "Claude".to_string(),
                    session_id: None,
                    mode: None,
                    model: Some("default".to_string()),
                    reasoning_level: None,
                    skip_permissions: None,
                    tool_version: Some("latest".to_string()),
                    collaboration_modes: None,
                    docker_service: None,
                    docker_force_host: None,
                    docker_recreate: None,
                    docker_build: None,
                    docker_keep: None,
                    docker_container_name: None,
                    docker_compose_args: None,
                    timestamp: 2_000,
                },
                ToolSessionEntry {
                    branch: "feature/test".to_string(),
                    worktree_path: None,
                    tool_id: "claude-code".to_string(),
                    tool_label: "Claude Code".to_string(),
                    session_id: None,
                    mode: None,
                    model: Some("default".to_string()),
                    reasoning_level: None,
                    skip_permissions: None,
                    tool_version: Some("latest".to_string()),
                    collaboration_modes: None,
                    docker_service: None,
                    docker_force_host: None,
                    docker_recreate: None,
                    docker_build: None,
                    docker_keep: None,
                    docker_container_name: None,
                    docker_compose_args: None,
                    timestamp: 1_000,
                },
            ],
        };

        let session_path = get_ts_session_path(&repo_root);
        if let Some(parent) = session_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let content = serde_json::to_string_pretty(&session).unwrap();
        std::fs::write(&session_path, content).unwrap();

        let entries = get_branch_tool_history(&repo_root, "feature/test");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool_id, "claude-code");
        assert_eq!(entries[0].timestamp, 2_000);
    }

    #[test]
    fn test_get_branch_tool_history_backfills_skip_permissions() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let repo_root = temp.path().join("sample-repo");
        std::fs::create_dir_all(&repo_root).unwrap();

        let session = TsSessionData {
            last_worktree_path: None,
            last_branch: None,
            last_used_tool: None,
            last_session_id: None,
            tool_label: None,
            tool_version: None,
            model: None,
            timestamp: 1_700_000_000_000,
            repository_root: repo_root.to_string_lossy().to_string(),
            history: vec![
                ToolSessionEntry {
                    branch: "feature/test".to_string(),
                    worktree_path: Some("/path/to/wt".to_string()),
                    tool_id: "claude-code".to_string(),
                    tool_label: "Claude Code".to_string(),
                    session_id: None,
                    mode: Some("Normal".to_string()),
                    model: None,
                    reasoning_level: None,
                    skip_permissions: Some(true),
                    tool_version: Some("latest".to_string()),
                    collaboration_modes: None,
                    docker_service: None,
                    docker_force_host: None,
                    docker_recreate: None,
                    docker_build: None,
                    docker_keep: None,
                    docker_container_name: None,
                    docker_compose_args: None,
                    timestamp: 1_000,
                },
                ToolSessionEntry {
                    branch: "feature/test".to_string(),
                    worktree_path: Some("/path/to/wt".to_string()),
                    tool_id: "claude-code".to_string(),
                    tool_label: "Claude Code".to_string(),
                    session_id: Some("session-123".to_string()),
                    mode: Some("Resume".to_string()),
                    model: None,
                    reasoning_level: None,
                    skip_permissions: None,
                    tool_version: Some("latest".to_string()),
                    collaboration_modes: None,
                    docker_service: None,
                    docker_force_host: None,
                    docker_recreate: None,
                    docker_build: None,
                    docker_keep: None,
                    docker_container_name: None,
                    docker_compose_args: None,
                    timestamp: 2_000,
                },
            ],
        };

        let session_path = get_ts_session_path(&repo_root);
        if let Some(parent) = session_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let content = serde_json::to_string_pretty(&session).unwrap();
        std::fs::write(&session_path, content).unwrap();

        let entries = get_branch_tool_history(&repo_root, "feature/test");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool_id, "claude-code");
        assert_eq!(entries[0].skip_permissions, Some(true));
    }

    #[test]
    fn test_canonical_tool_id_accepts_label_variants() {
        assert_eq!(canonical_tool_id("Codex CLI"), "codex-cli");
        assert_eq!(canonical_tool_id("codex cli"), "codex-cli");
        assert_eq!(canonical_tool_id("Claude Code"), "claude-code");
        assert_eq!(canonical_tool_id("claude code"), "claude-code");
        assert_eq!(canonical_tool_id("Gemini CLI"), "gemini-cli");
    }

    #[test]
    fn test_load_ts_session_persists_canonical_tool_ids() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let repo_root = temp.path().join("sample-repo");
        std::fs::create_dir_all(&repo_root).unwrap();

        let session = TsSessionData {
            last_worktree_path: Some("/path/to/wt".to_string()),
            last_branch: Some("feature/test".to_string()),
            last_used_tool: Some("Claude Code".to_string()),
            last_session_id: None,
            tool_label: Some("Claude Code".to_string()),
            tool_version: Some("latest".to_string()),
            model: Some("default".to_string()),
            timestamp: 1_700_000_000_000,
            repository_root: repo_root.to_string_lossy().to_string(),
            history: vec![ToolSessionEntry {
                branch: "feature/test".to_string(),
                worktree_path: Some("/path/to/wt".to_string()),
                tool_id: "Codex CLI".to_string(),
                tool_label: "Codex CLI".to_string(),
                session_id: None,
                mode: None,
                model: Some("gpt-5.2-codex".to_string()),
                reasoning_level: None,
                skip_permissions: None,
                tool_version: Some("latest".to_string()),
                collaboration_modes: None,
                docker_service: None,
                docker_force_host: None,
                docker_recreate: None,
                docker_build: None,
                docker_keep: None,
                docker_container_name: None,
                docker_compose_args: None,
                timestamp: 2_000,
            }],
        };

        let session_path = get_ts_session_path(&repo_root);
        if let Some(parent) = session_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let content = serde_json::to_string_pretty(&session).unwrap();
        std::fs::write(&session_path, content).unwrap();

        let loaded = load_ts_session(&repo_root).unwrap();
        assert_eq!(loaded.last_used_tool.as_deref(), Some("claude-code"));
        assert_eq!(loaded.history[0].tool_id, "codex-cli");

        // After auto-migration, TOML file should exist with canonical IDs
        let toml_path = get_ts_session_toml_path(&repo_root);
        assert!(
            toml_path.exists(),
            "TOML file should be created by auto-migration"
        );
        let updated = std::fs::read_to_string(&toml_path).unwrap();
        let updated_session: TsSessionData = toml::from_str(&updated).unwrap();
        assert_eq!(
            updated_session.last_used_tool.as_deref(),
            Some("claude-code")
        );
        assert_eq!(updated_session.history[0].tool_id, "codex-cli");
    }

    #[test]
    fn test_format_tool_usage() {
        let entry = ToolSessionEntry {
            branch: "feature/test".to_string(),
            worktree_path: Some("/path/to/wt".to_string()),
            tool_id: "claude-code".to_string(),
            tool_label: "Claude Code".to_string(),
            session_id: None,
            mode: None,
            model: Some("sonnet".to_string()),
            reasoning_level: None,
            skip_permissions: None,
            tool_version: Some("1.0.3".to_string()),
            collaboration_modes: None,
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            docker_container_name: None,
            docker_compose_args: None,
            timestamp: 1704067200000,
        };
        let result = entry.format_tool_usage();
        assert_eq!(result, "Claude@1.0.3");
    }

    #[test]
    fn test_format_tool_usage_no_version() {
        let entry = ToolSessionEntry {
            branch: "main".to_string(),
            worktree_path: None,
            tool_id: "codex-cli".to_string(),
            tool_label: "Codex".to_string(),
            session_id: None,
            mode: None,
            model: None,
            reasoning_level: None,
            skip_permissions: None,
            tool_version: None,
            collaboration_modes: None,
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            docker_container_name: None,
            docker_compose_args: None,
            timestamp: 1704067200000,
        };
        let result = entry.format_tool_usage();
        assert_eq!(result, "Codex@latest");
    }

    #[test]
    fn test_tool_session_entry_deserializes_docker_aliases() {
        let value = json!({
            "branch": "feature/test",
            "toolId": "claude-code",
            "toolLabel": "Claude",
            "timestamp": 1_700_000_000_000i64,
            "dockerService": "gwt",
            "dockerForceHost": false,
            "dockerRecreate": true,
            "dockerBuild": false,
            "dockerKeep": true
        });

        let entry: ToolSessionEntry = serde_json::from_value(value).unwrap();
        assert_eq!(entry.docker_service.as_deref(), Some("gwt"));
        assert_eq!(entry.docker_force_host, Some(false));
        assert_eq!(entry.docker_recreate, Some(true));
        assert_eq!(entry.docker_build, Some(false));
        assert_eq!(entry.docker_keep, Some(true));
    }

    #[test]
    fn test_ts_session_toml_path_generation() {
        let repo = PathBuf::from("/home/user/projects/myrepo");
        let path = get_ts_session_toml_path(&repo);
        assert!(path.to_string_lossy().contains(".gwt"));
        assert!(path.to_string_lossy().contains("sessions"));
        assert!(path.to_string_lossy().contains("myrepo_"));
        assert!(path.to_string_lossy().ends_with(".toml"));
    }

    #[test]
    fn test_needs_ts_session_migration() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let repo_root = temp.path().join("sample-repo");
        std::fs::create_dir_all(&repo_root).unwrap();

        // No files - no migration needed
        assert!(!needs_ts_session_migration(&repo_root));

        // Create JSON session file
        let json_path = get_ts_session_json_path(&repo_root);
        if let Some(parent) = json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let session = TsSessionData {
            last_worktree_path: None,
            last_branch: Some("main".to_string()),
            last_used_tool: Some("claude-code".to_string()),
            last_session_id: None,
            tool_label: Some("Claude".to_string()),
            tool_version: None,
            model: None,
            timestamp: 1_700_000_000_000,
            repository_root: repo_root.to_string_lossy().to_string(),
            history: Vec::new(),
        };
        let content = serde_json::to_string_pretty(&session).unwrap();
        std::fs::write(&json_path, content).unwrap();

        // JSON only - migration needed
        assert!(needs_ts_session_migration(&repo_root));

        // Create TOML session file
        let toml_path = get_ts_session_toml_path(&repo_root);
        if let Some(parent) = toml_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let toml_content = toml::to_string_pretty(&session).unwrap();
        std::fs::write(&toml_path, toml_content).unwrap();

        // Both files - no migration needed (TOML exists)
        assert!(!needs_ts_session_migration(&repo_root));
    }

    #[test]
    fn test_migrate_ts_session_if_needed() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let repo_root = temp.path().join("sample-repo");
        std::fs::create_dir_all(&repo_root).unwrap();

        // Create JSON session file with camelCase fields
        let json_path = get_ts_session_json_path(&repo_root);
        if let Some(parent) = json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let json_content = r#"{
            "lastWorktreePath": "/path/to/wt",
            "lastBranch": "feature/test",
            "lastUsedTool": "claude-code",
            "lastSessionId": "session-123",
            "toolLabel": "Claude",
            "toolVersion": "1.0.0",
            "model": "opus",
            "timestamp": 1700000000000,
            "repositoryRoot": "/tmp/repo",
            "history": [
                {
                    "branch": "feature/test",
                    "worktreePath": "/path/to/wt",
                    "toolId": "claude-code",
                    "toolLabel": "Claude",
                    "timestamp": 1700000000000
                }
            ]
        }"#;
        std::fs::write(&json_path, json_content).unwrap();

        // Perform migration
        let result = migrate_ts_session_if_needed(&repo_root);
        assert!(result.is_ok());
        assert!(result.unwrap()); // Should return true (migration performed)

        // Verify TOML file was created
        let toml_path = get_ts_session_toml_path(&repo_root);
        assert!(toml_path.exists());

        // Load from TOML and verify data
        let toml_content = std::fs::read_to_string(&toml_path).unwrap();
        let session: TsSessionData = toml::from_str(&toml_content).unwrap();
        assert_eq!(session.last_branch.as_deref(), Some("feature/test"));
        assert_eq!(session.last_used_tool.as_deref(), Some("claude-code"));
        assert_eq!(session.history.len(), 1);
        assert_eq!(session.history[0].branch, "feature/test");

        // Second migration should be skipped
        let result2 = migrate_ts_session_if_needed(&repo_root);
        assert!(result2.is_ok());
        assert!(!result2.unwrap()); // Should return false (no migration needed)
    }

    #[test]
    fn test_load_ts_session_prefers_toml() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let repo_root = temp.path().join("sample-repo");
        std::fs::create_dir_all(&repo_root).unwrap();

        // Create JSON session file (older timestamp)
        let json_path = get_ts_session_json_path(&repo_root);
        if let Some(parent) = json_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let json_session = TsSessionData {
            last_worktree_path: None,
            last_branch: Some("json-branch".to_string()),
            last_used_tool: Some("codex-cli".to_string()),
            last_session_id: None,
            tool_label: Some("Codex".to_string()),
            tool_version: None,
            model: None,
            timestamp: 1_600_000_000_000,
            repository_root: repo_root.to_string_lossy().to_string(),
            history: Vec::new(),
        };
        let json_content = serde_json::to_string_pretty(&json_session).unwrap();
        std::fs::write(&json_path, json_content).unwrap();

        // Create TOML session file (newer timestamp)
        let toml_path = get_ts_session_toml_path(&repo_root);
        if let Some(parent) = toml_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let toml_session = TsSessionData {
            last_worktree_path: None,
            last_branch: Some("toml-branch".to_string()),
            last_used_tool: Some("claude-code".to_string()),
            last_session_id: None,
            tool_label: Some("Claude".to_string()),
            tool_version: None,
            model: None,
            timestamp: 1_700_000_000_000,
            repository_root: repo_root.to_string_lossy().to_string(),
            history: Vec::new(),
        };
        let toml_content = toml::to_string_pretty(&toml_session).unwrap();
        std::fs::write(&toml_path, toml_content).unwrap();

        // Load should prefer TOML
        let loaded = load_ts_session(&repo_root).expect("should load session");
        assert_eq!(loaded.last_branch.as_deref(), Some("toml-branch"));
        assert_eq!(loaded.last_used_tool.as_deref(), Some("claude-code"));
    }

    #[test]
    fn test_save_session_entry_writes_toml() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        // Create a mock git repo
        let repo_root = temp.path().join("sample-repo");
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();

        let entry = ToolSessionEntry {
            branch: "feature/new".to_string(),
            worktree_path: Some(repo_root.to_string_lossy().to_string()),
            tool_id: "claude-code".to_string(),
            tool_label: "Claude".to_string(),
            session_id: Some("new-session".to_string()),
            mode: Some("Normal".to_string()),
            model: Some("opus".to_string()),
            reasoning_level: None,
            skip_permissions: None,
            tool_version: Some("2.0.0".to_string()),
            collaboration_modes: None,
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            docker_container_name: None,
            docker_compose_args: None,
            timestamp: 1_800_000_000_000,
        };

        let result = save_session_entry(&repo_root, entry);
        assert!(result.is_ok());

        // Verify TOML file was created (not JSON)
        let toml_path = get_ts_session_toml_path(&repo_root);
        assert!(toml_path.exists());

        let json_path = get_ts_session_json_path(&repo_root);
        assert!(!json_path.exists()); // JSON should not be created

        // Verify content
        let content = std::fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("feature/new"));
        assert!(content.contains("claude-code"));
    }
}
