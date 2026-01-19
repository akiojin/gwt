//! TypeScript session file compatibility (FR-069, FR-070)
//!
//! Reads and writes session history from/to TypeScript-format session files
//! stored at ~/.config/gwt/sessions/{repoName}_{hash}.json

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Tool session entry from TypeScript format (FR-069, FR-070)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSessionEntry {
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    pub tool_id: String,
    pub tool_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_permissions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
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

    let normalized: String = lower.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    match normalized.as_str() {
        "claude" | "claudecode" => "claude-code".to_string(),
        "codex" | "codexcli" => "codex-cli".to_string(),
        "gemini" | "geminicli" => "gemini-cli".to_string(),
        "opencode" => "opencode".to_string(),
        _ => trimmed.to_string(),
    }
}

/// TypeScript session data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TsSessionData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_worktree_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub timestamp: i64,
    pub repository_root: String,
    #[serde(default)]
    pub history: Vec<ToolSessionEntry>,
}

/// Get the TypeScript session file path for a repository
pub fn get_ts_session_path(repo_root: &Path) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let session_dir = home.join(".config").join("gwt").join("sessions");

    let repo_name = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo");

    // Match TypeScript: Buffer.from(repositoryRoot).toString("base64").replace(/[/+=]/g, "_")
    let repo_path_str = repo_root.to_string_lossy();
    let hash = STANDARD.encode(repo_path_str.as_bytes());
    let hash_safe = hash.replace(['/', '+', '='], "_");

    session_dir.join(format!("{}_{}.json", repo_name, hash_safe))
}

/// Load TypeScript session data for a repository
pub fn load_ts_session(repo_root: &Path) -> Option<TsSessionData> {
    let session_path = get_ts_session_path(repo_root);
    if let Ok(content) = std::fs::read_to_string(&session_path) {
        if let Ok(session) = serde_json::from_str(&content) {
            return Some(session);
        }
    }

    // Fallback: if the exact hash path is missing or invalid, try to locate
    // the latest session file that matches the repo name.
    let repo_name = repo_root.file_name()?.to_string_lossy().to_string();
    let session_dir = session_path.parent()?;
    let prefix = format!("{}_", repo_name);
    let mut latest: Option<TsSessionData> = None;

    let entries = std::fs::read_dir(session_dir).ok()?;
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
            .map(|current| session.timestamp > current.timestamp)
            .unwrap_or(true);
        if should_update {
            latest = Some(session);
        }
    }

    latest
}

/// Save session entry to TypeScript-compatible session file (FR-069)
///
/// Adds a new entry to the session history and updates last-used fields.
/// Creates the session file if it doesn't exist.
pub fn save_session_entry(
    repo_root: &Path,
    mut entry: ToolSessionEntry,
) -> Result<(), std::io::Error> {
    // Resolve to main repo root for consistency
    let main_root = crate::git::get_main_repo_root(repo_root);
    let session_path = get_ts_session_path(&main_root);

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

    // Write to file
    let content = serde_json::to_string_pretty(&session)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(&session_path, content)?;

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

            let existing = tool_map.get(&entry.tool_id);
            if existing.is_none() || existing.unwrap().timestamp < entry.timestamp {
                tool_map.insert(entry.tool_id.clone(), entry);
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
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

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

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_get_branch_tool_history_fallback_to_last_branch() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

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

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_get_branch_tool_history_canonicalizes_tool_id() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

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

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
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
            timestamp: 1704067200000,
        };
        let result = entry.format_tool_usage();
        assert_eq!(result, "Codex@latest");
    }
}
