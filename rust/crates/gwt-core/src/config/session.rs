//! Session management

use crate::error::{GwtError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Session ID
    pub id: String,
    /// Worktree path
    pub worktree_path: PathBuf,
    /// Branch name
    pub branch: String,
    /// Agent name (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Agent session ID (for resume)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
}

impl Session {
    /// Create a new session
    pub fn new(worktree_path: impl Into<PathBuf>, branch: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: worktree_path.into(),
            branch: branch.into(),
            agent: None,
            agent_session_id: None,
            created_at: now,
            updated_at: now,
        }
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
}
