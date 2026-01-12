//! TypeScript session file compatibility (FR-070)
//!
//! Reads session history from TypeScript-format session files
//! stored at ~/.config/gwt/sessions/{repoName}_{hash}.json

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Local, TimeZone, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Tool session entry from TypeScript format (FR-070)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSessionEntry {
    pub branch: String,
    #[serde(default)]
    pub worktree_path: Option<String>,
    pub tool_id: String,
    pub tool_label: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_level: Option<String>,
    #[serde(default)]
    pub skip_permissions: Option<bool>,
    #[serde(default)]
    pub tool_version: Option<String>,
    /// Unix timestamp in milliseconds
    pub timestamp: i64,
}

impl ToolSessionEntry {
    /// Format tool usage for display (FR-070)
    /// Returns: "ToolLabel@version | YYYY-MM-DD HH:mm" (local time)
    pub fn format_tool_usage(&self) -> String {
        let version = self.tool_version.as_deref().unwrap_or("latest");
        let local_time = self.datetime().with_timezone(&Local);
        let formatted_time = local_time.format("%Y-%m-%d %H:%M").to_string();
        format!("{}@{} | {}", self.tool_label, version, formatted_time)
    }

    /// Get timestamp as DateTime
    pub fn datetime(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(self.timestamp)
            .single()
            .unwrap_or_else(Utc::now)
    }
}

/// TypeScript session data structure
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TsSessionData {
    #[serde(default)]
    pub last_worktree_path: Option<String>,
    #[serde(default)]
    pub last_branch: Option<String>,
    #[serde(default)]
    pub last_used_tool: Option<String>,
    #[serde(default)]
    pub last_session_id: Option<String>,
    #[serde(default)]
    pub tool_label: Option<String>,
    #[serde(default)]
    pub tool_version: Option<String>,
    #[serde(default)]
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
    if !session_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&session_path).ok()?;
    serde_json::from_str(&content).ok()
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
            let entry = ToolSessionEntry {
                branch: branch.clone(),
                worktree_path: Some(worktree_path),
                tool_id: session.last_used_tool.clone().unwrap_or_default(),
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

    for entry in session.history {
        if entry.branch == branch {
            let existing = tool_map.get(&entry.tool_id);
            if existing.is_none() || existing.unwrap().timestamp < entry.timestamp {
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
        // FR-070: Format includes date/time (timezone-dependent)
        let result = entry.format_tool_usage();
        assert!(result.starts_with("Claude Code@1.0.3 | "));
        assert!(result.contains("-")); // Date separator
        assert!(result.contains(":")); // Time separator
    }

    #[test]
    fn test_format_tool_usage_no_version() {
        let entry = ToolSessionEntry {
            branch: "main".to_string(),
            worktree_path: None,
            tool_id: "codex-cli".to_string(),
            tool_label: "Codex CLI".to_string(),
            session_id: None,
            mode: None,
            model: None,
            reasoning_level: None,
            skip_permissions: None,
            tool_version: None,
            timestamp: 1704067200000,
        };
        // FR-070: Format includes date/time (timezone-dependent)
        let result = entry.format_tool_usage();
        assert!(result.starts_with("Codex CLI@latest | "));
    }
}
