//! Session management

use crate::error::{GwtError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json;
use std::path::{Path, PathBuf};

/// Session information (FR-069: Store version info in session history)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Session ID
    pub id: String,
    /// Worktree path
    pub worktree_path: PathBuf,
    /// Branch name
    pub branch: String,
    /// Agent ID (e.g., "claude-code", "codex-cli")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Agent display label (e.g., "Claude Code", "Codex CLI")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_label: Option<String>,
    /// Agent session ID (for resume)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    /// Tool version (e.g., "1.0.3", "latest", "installed")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    /// Model used (e.g., "opus", "sonnet")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
}

impl Session {
    /// Legacy JSON session file name
    const LEGACY_JSON_NAME: &'static str = ".gwt-session.json";

    /// Create a new session
    pub fn new(worktree_path: impl Into<PathBuf>, branch: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: worktree_path.into(),
            branch: branch.into(),
            agent: None,
            agent_label: None,
            agent_session_id: None,
            tool_version: None,
            model: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Format tool usage string for display (FR-070)
    /// Returns format: "ToolName@X.Y.Z"
    pub fn format_tool_usage(&self) -> Option<String> {
        let label = self.agent_label.as_ref().or(self.agent.as_ref())?;
        let short_label = short_tool_label(self.agent.as_deref(), label);
        let version = self.tool_version.as_deref().unwrap_or("latest");
        Some(format!("{}@{}", short_label, version))
    }

    /// Save session to file
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| GwtError::ConfigWriteError {
            reason: e.to_string(),
        })?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(path, content)?;
        Ok(())
    }

    /// Load session from file
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content).map_err(|e| GwtError::ConfigParseError {
            reason: e.to_string(),
        })
    }

    /// Get the session file path for a worktree
    pub fn session_path(worktree_path: &Path) -> PathBuf {
        worktree_path.join(".gwt-session.toml")
    }

    /// Get legacy JSON session file path
    fn legacy_session_path(worktree_path: &Path) -> PathBuf {
        worktree_path.join(Self::LEGACY_JSON_NAME)
    }

    /// Migrate legacy JSON session file to TOML if needed
    fn migrate_legacy_session(worktree_path: &Path) -> Result<()> {
        let json_path = Self::legacy_session_path(worktree_path);
        let toml_path = Self::session_path(worktree_path);

        if !json_path.exists() || toml_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&json_path)?;
        let session: Session =
            serde_json::from_str(&content).map_err(|e| GwtError::MigrationFailed {
                reason: format!("Failed to parse session JSON: {}", e),
            })?;
        session.save(&toml_path)?;
        Ok(())
    }

    /// Load session for a worktree if exists
    pub fn load_for_worktree(worktree_path: &Path) -> Option<Self> {
        let _ = Self::migrate_legacy_session(worktree_path);
        let session_path = Self::session_path(worktree_path);
        if session_path.exists() {
            Self::load(&session_path).ok()
        } else {
            None
        }
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

/// Load all sessions from worktrees
pub fn load_sessions_from_worktrees(worktrees: &[crate::worktree::Worktree]) -> Vec<Session> {
    worktrees
        .iter()
        .filter_map(|wt| Session::load_for_worktree(&wt.path))
        .collect()
}

/// Get session for a specific branch
pub fn get_session_for_branch<'a>(sessions: &'a [Session], branch: &str) -> Option<&'a Session> {
    sessions.iter().find(|s| s.branch == branch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_session_save_load() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("session.toml");

        let session = Session::new("/repo/.worktrees/feature", "feature/test");

        session.save(&session_path).unwrap();

        let loaded = Session::load(&session_path).unwrap();
        assert_eq!(loaded.branch, "feature/test");
    }

    #[test]
    fn test_legacy_session_migration() {
        let temp = TempDir::new().unwrap();
        let legacy_path = temp.path().join(Session::LEGACY_JSON_NAME);

        let session = Session::new(temp.path(), "feature/legacy");
        std::fs::write(
            &legacy_path,
            serde_json::to_string_pretty(&session).unwrap(),
        )
        .unwrap();

        let loaded = Session::load_for_worktree(temp.path()).unwrap();
        assert_eq!(loaded.branch, "feature/legacy");
        assert!(Session::session_path(temp.path()).exists());
    }

    #[test]
    fn test_format_tool_usage_short_label() {
        let mut session = Session::new("/repo/.worktrees/feature", "feature/test");
        session.agent_label = Some("Claude Code".to_string());
        session.tool_version = Some("1.0.3".to_string());
        assert_eq!(
            session.format_tool_usage(),
            Some("Claude@1.0.3".to_string())
        );

        let mut session = Session::new("/repo/.worktrees/feature", "feature/test");
        session.agent = Some("codex-cli".to_string());
        session.tool_version = None;
        assert_eq!(
            session.format_tool_usage(),
            Some("Codex@latest".to_string())
        );
    }
}
