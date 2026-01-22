//! Agent history persistence module
//!
//! This module provides functionality to persist agent usage history per branch,
//! allowing the display of recently used agents even after worktrees are deleted.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, warn};

/// Error type for agent history operations
#[derive(Error, Debug)]
pub enum AgentHistoryError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Home directory not found")]
    HomeDirNotFound,
}

/// A single agent history entry for a branch
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentHistoryEntry {
    /// Agent identifier (e.g., "claude-code", "codex-cli", "gemini-cli", "opencode")
    pub agent_id: String,
    /// Display label (e.g., "Claude@latest")
    pub agent_label: String,
    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
}

/// Branch history map: branch name -> entry
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BranchHistory {
    pub branches: HashMap<String, AgentHistoryEntry>,
}

/// Repository history: contains branch histories
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RepoHistory {
    branches: HashMap<String, AgentHistoryEntry>,
}

/// The main agent history store
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentHistoryStore {
    repos: HashMap<String, RepoHistory>,
}

impl AgentHistoryStore {
    /// Create a new empty store
    pub fn new() -> Self {
        Self {
            repos: HashMap::new(),
        }
    }

    /// Get the default history file path (~/.config/gwt/agent-history.json)
    pub fn get_history_path() -> Result<PathBuf, AgentHistoryError> {
        let config_dir = dirs::config_dir().ok_or(AgentHistoryError::HomeDirNotFound)?;
        Ok(config_dir.join("gwt").join("agent-history.json"))
    }

    /// Load history from the default path
    pub fn load() -> Result<Self, AgentHistoryError> {
        let path = Self::get_history_path()?;
        Self::load_from(&path)
    }

    /// Load history from a specific path
    pub fn load_from(path: &Path) -> Result<Self, AgentHistoryError> {
        if !path.exists() {
            debug!("History file does not exist, returning empty store");
            return Ok(Self::new());
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read history file: {}", e);
                return Ok(Self::new());
            }
        };

        match serde_json::from_str(&content) {
            Ok(store) => Ok(store),
            Err(e) => {
                warn!("Failed to parse history file (corrupted?): {}", e);
                Ok(Self::new())
            }
        }
    }

    /// Save history to the default path
    pub fn save(&self) -> Result<(), AgentHistoryError> {
        let path = Self::get_history_path()?;
        self.save_to(&path)
    }

    /// Save history to a specific path
    pub fn save_to(&self, path: &Path) -> Result<(), AgentHistoryError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, &content)?;

        // Set file permissions to 600 (owner read/write only) on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(path, perms)?;
        }

        debug!("Saved history to {:?}", path);
        Ok(())
    }

    /// Record agent usage for a branch
    pub fn record(
        &mut self,
        repo_path: &Path,
        branch: &str,
        agent_id: &str,
        agent_label: &str,
    ) -> Result<(), AgentHistoryError> {
        let repo_key = repo_path.to_string_lossy().to_string();

        let repo_history = self.repos.entry(repo_key).or_default();

        repo_history.branches.insert(
            branch.to_string(),
            AgentHistoryEntry {
                agent_id: agent_id.to_string(),
                agent_label: agent_label.to_string(),
                updated_at: Utc::now(),
            },
        );

        Ok(())
    }

    /// Get agent history for a specific branch
    pub fn get(&self, repo_path: &Path, branch: &str) -> Option<&AgentHistoryEntry> {
        let repo_key = repo_path.to_string_lossy().to_string();
        self.repos
            .get(&repo_key)
            .and_then(|repo| repo.branches.get(branch))
    }

    /// Get all agent history entries for a repository
    pub fn get_all_for_repo(&self, repo_path: &Path) -> HashMap<String, &AgentHistoryEntry> {
        let repo_key = repo_path.to_string_lossy().to_string();
        self.repos
            .get(&repo_key)
            .map(|repo| {
                repo.branches
                    .iter()
                    .map(|(k, v)| (k.clone(), v))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // T101: Test AgentHistoryEntry serialization/deserialization
    #[test]
    fn test_agent_history_entry_serialization() {
        let entry = AgentHistoryEntry {
            agent_id: "claude-code".to_string(),
            agent_label: "Claude@latest".to_string(),
            updated_at: DateTime::parse_from_rfc3339("2026-01-22T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: AgentHistoryEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(entry, deserialized);
    }

    // T102: Test AgentHistoryStore JSON read/write
    #[test]
    fn test_agent_history_store_json_roundtrip() {
        let mut store = AgentHistoryStore::new();
        store
            .record(
                Path::new("/path/to/repo"),
                "feature/test",
                "claude-code",
                "Claude@latest",
            )
            .unwrap();

        let json = serde_json::to_string_pretty(&store).unwrap();
        let deserialized: AgentHistoryStore = serde_json::from_str(&json).unwrap();

        let entry = deserialized.get(Path::new("/path/to/repo"), "feature/test");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().agent_id, "claude-code");
        assert_eq!(entry.unwrap().agent_label, "Claude@latest");
    }

    // T105: Test history file path generation
    #[test]
    fn test_get_history_path() {
        let path = AgentHistoryStore::get_history_path();
        assert!(path.is_ok());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains("gwt"));
        assert!(path.to_string_lossy().contains("agent-history.json"));
    }

    // T106: Test directory auto-creation
    #[test]
    fn test_directory_auto_creation() {
        let temp_dir = TempDir::new().unwrap();
        let history_path = temp_dir
            .path()
            .join("nested")
            .join("dir")
            .join("history.json");

        let store = AgentHistoryStore::new();
        store.save_to(&history_path).unwrap();

        assert!(history_path.exists());
    }

    // T107: Test file permissions (Unix only)
    #[cfg(unix)]
    #[test]
    fn test_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let history_path = temp_dir.path().join("history.json");

        let store = AgentHistoryStore::new();
        store.save_to(&history_path).unwrap();

        let metadata = fs::metadata(&history_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    // T109: Test record() method - new entry creation
    #[test]
    fn test_record_new_entry() {
        let mut store = AgentHistoryStore::new();
        store
            .record(
                Path::new("/repo"),
                "main",
                "codex-cli",
                "Codex@latest",
            )
            .unwrap();

        let entry = store.get(Path::new("/repo"), "main").unwrap();
        assert_eq!(entry.agent_id, "codex-cli");
        assert_eq!(entry.agent_label, "Codex@latest");
    }

    // T110: Test record() method - overwrite existing entry
    #[test]
    fn test_record_overwrite_existing() {
        let mut store = AgentHistoryStore::new();

        // First record
        store
            .record(Path::new("/repo"), "main", "claude-code", "Claude@latest")
            .unwrap();

        // Overwrite with different agent
        store
            .record(Path::new("/repo"), "main", "codex-cli", "Codex@latest")
            .unwrap();

        let entry = store.get(Path::new("/repo"), "main").unwrap();
        assert_eq!(entry.agent_id, "codex-cli");
        assert_eq!(entry.agent_label, "Codex@latest");
    }

    // T114: Test corrupted file fallback to empty history
    #[test]
    fn test_corrupted_file_fallback() {
        let temp_dir = TempDir::new().unwrap();
        let history_path = temp_dir.path().join("history.json");

        // Write corrupted JSON
        fs::write(&history_path, "{ invalid json }").unwrap();

        let store = AgentHistoryStore::load_from(&history_path).unwrap();
        assert!(store.repos.is_empty());
    }

    // T201: Test get() method - existing entry
    #[test]
    fn test_get_existing_entry() {
        let mut store = AgentHistoryStore::new();
        store
            .record(Path::new("/repo"), "develop", "gemini-cli", "Gemini@latest")
            .unwrap();

        let entry = store.get(Path::new("/repo"), "develop");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().agent_id, "gemini-cli");
    }

    // T202: Test get() method - non-existing entry
    #[test]
    fn test_get_non_existing_entry() {
        let store = AgentHistoryStore::new();
        let entry = store.get(Path::new("/repo"), "nonexistent");
        assert!(entry.is_none());
    }

    // T203: Test get_all_for_repo() method
    #[test]
    fn test_get_all_for_repo() {
        let mut store = AgentHistoryStore::new();
        store
            .record(Path::new("/repo"), "main", "claude-code", "Claude@latest")
            .unwrap();
        store
            .record(Path::new("/repo"), "develop", "codex-cli", "Codex@latest")
            .unwrap();
        store
            .record(
                Path::new("/other-repo"),
                "main",
                "gemini-cli",
                "Gemini@latest",
            )
            .unwrap();

        let entries = store.get_all_for_repo(Path::new("/repo"));
        assert_eq!(entries.len(), 2);
        assert!(entries.contains_key("main"));
        assert!(entries.contains_key("develop"));
    }

    // T301: Test repository isolation
    #[test]
    fn test_repository_isolation() {
        let mut store = AgentHistoryStore::new();
        store
            .record(Path::new("/repo-a"), "main", "claude-code", "Claude@latest")
            .unwrap();
        store
            .record(Path::new("/repo-b"), "main", "codex-cli", "Codex@latest")
            .unwrap();

        let entry_a = store.get(Path::new("/repo-a"), "main").unwrap();
        let entry_b = store.get(Path::new("/repo-b"), "main").unwrap();

        assert_eq!(entry_a.agent_id, "claude-code");
        assert_eq!(entry_b.agent_id, "codex-cli");
    }

    // T302: Test repo A history does not affect repo B
    #[test]
    fn test_repo_history_independence() {
        let mut store = AgentHistoryStore::new();
        store
            .record(Path::new("/repo-a"), "feature", "claude-code", "Claude@latest")
            .unwrap();

        let entries_b = store.get_all_for_repo(Path::new("/repo-b"));
        assert!(entries_b.is_empty());
    }

    // Test full save/load cycle
    #[test]
    fn test_save_load_cycle() {
        let temp_dir = TempDir::new().unwrap();
        let history_path = temp_dir.path().join("history.json");

        // Create and save
        let mut store = AgentHistoryStore::new();
        store
            .record(
                Path::new("/my/repo"),
                "feature/awesome",
                "opencode",
                "OpenCode@latest",
            )
            .unwrap();
        store.save_to(&history_path).unwrap();

        // Load and verify
        let loaded = AgentHistoryStore::load_from(&history_path).unwrap();
        let entry = loaded.get(Path::new("/my/repo"), "feature/awesome").unwrap();
        assert_eq!(entry.agent_id, "opencode");
        assert_eq!(entry.agent_label, "OpenCode@latest");
    }
}
