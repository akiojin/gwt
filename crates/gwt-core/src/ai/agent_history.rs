//! Agent history persistence module (SPEC-a3f4c9df)
//!
//! This module provides functionality to persist agent usage history per branch,
//! allowing the display of recently used agents even after worktrees are deleted.
//!
//! File locations:
//! - New format: ~/.gwt/agent-history.toml
//! - Legacy format: ~/.config/gwt/agent-history.json

use crate::config::migration::{ensure_config_dir, write_atomic};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, info, warn};

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

    /// Get the new TOML history file path (~/.gwt/agent-history.toml)
    pub fn get_toml_history_path() -> Result<PathBuf, AgentHistoryError> {
        let home_dir = dirs::home_dir().ok_or(AgentHistoryError::HomeDirNotFound)?;
        Ok(home_dir.join(".gwt").join("agent-history.toml"))
    }

    /// Get the legacy JSON history file path (~/.config/gwt/agent-history.json)
    pub fn get_json_history_path() -> Result<PathBuf, AgentHistoryError> {
        let config_dir = dirs::config_dir().ok_or(AgentHistoryError::HomeDirNotFound)?;
        Ok(config_dir.join("gwt").join("agent-history.json"))
    }

    /// Get the default history file path (deprecated - use get_toml_history_path)
    #[deprecated(note = "Use get_toml_history_path() for new code")]
    pub fn get_history_path() -> Result<PathBuf, AgentHistoryError> {
        Self::get_json_history_path()
    }

    /// Load history from the default paths with format auto-detection (SPEC-a3f4c9df FR-005)
    ///
    /// Priority: TOML > JSON
    pub fn load() -> Result<Self, AgentHistoryError> {
        // Try TOML first (new format)
        if let Ok(toml_path) = Self::get_toml_history_path() {
            if toml_path.exists() {
                debug!("Loading agent history from TOML: {}", toml_path.display());
                if let Ok(store) = Self::load_from_toml(&toml_path) {
                    return Ok(store);
                }
            }
        }

        // Fall back to JSON (legacy format)
        if let Ok(json_path) = Self::get_json_history_path() {
            if json_path.exists() {
                debug!(
                    "Loading agent history from JSON (legacy): {}",
                    json_path.display()
                );
                return Self::load_from_json(&json_path);
            }
        }

        debug!("No history file found, returning empty store");
        Ok(Self::new())
    }

    /// Load history from TOML file
    fn load_from_toml(path: &Path) -> Result<Self, AgentHistoryError> {
        let content = fs::read_to_string(path)?;
        let store: Self = toml::from_str(&content).map_err(|e| {
            warn!("Failed to parse TOML history file: {}", e);
            AgentHistoryError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("TOML parse error: {}", e),
            ))
        })?;
        Ok(store)
    }

    /// Load history from JSON file (legacy)
    fn load_from_json(path: &Path) -> Result<Self, AgentHistoryError> {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read JSON history file: {}", e);
                return Ok(Self::new());
            }
        };

        match serde_json::from_str(&content) {
            Ok(store) => Ok(store),
            Err(e) => {
                warn!("Failed to parse JSON history file (corrupted?): {}", e);
                Ok(Self::new())
            }
        }
    }

    /// Load history from a specific path (auto-detects format by extension)
    pub fn load_from(path: &Path) -> Result<Self, AgentHistoryError> {
        if !path.exists() {
            debug!("History file does not exist, returning empty store");
            return Ok(Self::new());
        }

        if path.extension().is_some_and(|ext| ext == "toml") {
            Self::load_from_toml(path)
        } else {
            Self::load_from_json(path)
        }
    }

    /// Save history to the default path in TOML format (SPEC-a3f4c9df FR-006)
    pub fn save(&self) -> Result<(), AgentHistoryError> {
        let path = Self::get_toml_history_path()?;
        self.save_to_toml(&path)
    }

    /// Save history to a specific path in TOML format
    fn save_to_toml(&self, path: &Path) -> Result<(), AgentHistoryError> {
        if let Some(parent) = path.parent() {
            ensure_config_dir(parent)
                .map_err(|e| AgentHistoryError::Io(io::Error::other(e.to_string())))?;
        }

        let content = toml::to_string_pretty(self).map_err(|e| {
            AgentHistoryError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("TOML serialize error: {}", e),
            ))
        })?;

        write_atomic(path, &content)
            .map_err(|e| AgentHistoryError::Io(io::Error::other(e.to_string())))?;

        info!("Saved agent history to {:?}", path);
        Ok(())
    }

    /// Save history to a specific path (auto-detects format by extension)
    pub fn save_to(&self, path: &Path) -> Result<(), AgentHistoryError> {
        if path.extension().is_some_and(|ext| ext == "toml") {
            self.save_to_toml(path)
        } else {
            self.save_to_json(path)
        }
    }

    /// Save history to JSON format (legacy - for backward compatibility)
    fn save_to_json(&self, path: &Path) -> Result<(), AgentHistoryError> {
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

    /// Check if migration from JSON to TOML is needed
    pub fn needs_migration() -> bool {
        let toml_path = Self::get_toml_history_path().ok();
        let json_path = Self::get_json_history_path().ok();

        match (toml_path, json_path) {
            (Some(toml), Some(json)) => json.exists() && !toml.exists(),
            _ => false,
        }
    }

    /// Migrate from JSON to TOML if needed
    pub fn migrate_if_needed() -> Result<bool, AgentHistoryError> {
        if !Self::needs_migration() {
            return Ok(false);
        }

        info!("Migrating agent history from JSON to TOML");

        let store = Self::load()?;
        store.save()?;

        info!("Agent history migration completed");
        Ok(true)
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
            .map(|repo| repo.branches.iter().map(|(k, v)| (k.clone(), v)).collect())
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

    // T105: Test history file path generation (TOML)
    #[test]
    fn test_get_toml_history_path() {
        let path = AgentHistoryStore::get_toml_history_path();
        assert!(path.is_ok());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains(".gwt"));
        assert!(path.to_string_lossy().contains("agent-history.toml"));
    }

    // T105: Test legacy JSON history file path generation
    #[test]
    fn test_get_json_history_path() {
        let path = AgentHistoryStore::get_json_history_path();
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
            .record(Path::new("/repo"), "main", "codex-cli", "Codex@latest")
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
            .record(
                Path::new("/repo-a"),
                "feature",
                "claude-code",
                "Claude@latest",
            )
            .unwrap();

        let entries_b = store.get_all_for_repo(Path::new("/repo-b"));
        assert!(entries_b.is_empty());
    }

    // Test full save/load cycle (JSON - legacy)
    #[test]
    fn test_save_load_cycle_json() {
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
        let entry = loaded
            .get(Path::new("/my/repo"), "feature/awesome")
            .unwrap();
        assert_eq!(entry.agent_id, "opencode");
        assert_eq!(entry.agent_label, "OpenCode@latest");
    }

    // Test full save/load cycle (TOML - new)
    #[test]
    fn test_save_load_cycle_toml() {
        let temp_dir = TempDir::new().unwrap();
        let history_path = temp_dir.path().join("history.toml");

        // Create and save
        let mut store = AgentHistoryStore::new();
        store
            .record(
                Path::new("/my/repo"),
                "feature/toml",
                "claude-code",
                "Claude@latest",
            )
            .unwrap();
        store.save_to(&history_path).unwrap();

        // Verify TOML content
        let content = fs::read_to_string(&history_path).unwrap();
        assert!(content.contains("[repos"));

        // Load and verify
        let loaded = AgentHistoryStore::load_from(&history_path).unwrap();
        let entry = loaded.get(Path::new("/my/repo"), "feature/toml").unwrap();
        assert_eq!(entry.agent_id, "claude-code");
        assert_eq!(entry.agent_label, "Claude@latest");
    }

    // Test TOML priority over JSON
    #[test]
    fn test_toml_priority_over_json() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        // Create legacy JSON in ~/.config/gwt/
        let config_gwt = temp_dir.path().join(".config").join("gwt");
        fs::create_dir_all(&config_gwt).unwrap();
        let json_path = config_gwt.join("agent-history.json");
        let mut json_store = AgentHistoryStore::new();
        json_store
            .record(Path::new("/repo"), "main", "json-agent", "JSON Agent")
            .unwrap();
        json_store.save_to(&json_path).unwrap();

        // Create new TOML in ~/.gwt/
        let gwt_dir = temp_dir.path().join(".gwt");
        fs::create_dir_all(&gwt_dir).unwrap();
        let toml_path = gwt_dir.join("agent-history.toml");
        let mut toml_store = AgentHistoryStore::new();
        toml_store
            .record(Path::new("/repo"), "main", "toml-agent", "TOML Agent")
            .unwrap();
        toml_store.save_to(&toml_path).unwrap();

        // TOML should be loaded (priority)
        let loaded = AgentHistoryStore::load().unwrap();
        let entry = loaded.get(Path::new("/repo"), "main").unwrap();
        assert_eq!(entry.agent_id, "toml-agent");

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    // Test needs_migration
    #[test]
    fn test_needs_migration() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        // No files - no migration needed
        assert!(!AgentHistoryStore::needs_migration());

        // Create JSON only in ~/.config/gwt/
        let config_gwt = temp_dir.path().join(".config").join("gwt");
        fs::create_dir_all(&config_gwt).unwrap();
        fs::write(config_gwt.join("agent-history.json"), "{}").unwrap();
        assert!(AgentHistoryStore::needs_migration());

        // Create TOML in ~/.gwt/ - no longer needs migration
        let gwt_dir = temp_dir.path().join(".gwt");
        fs::create_dir_all(&gwt_dir).unwrap();
        fs::write(gwt_dir.join("agent-history.toml"), "").unwrap();
        assert!(!AgentHistoryStore::needs_migration());

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    // Test migrate_if_needed
    #[test]
    fn test_migrate_if_needed() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        // Create JSON file
        let config_gwt = temp_dir.path().join(".config").join("gwt");
        fs::create_dir_all(&config_gwt).unwrap();
        let json_path = config_gwt.join("agent-history.json");
        let mut store = AgentHistoryStore::new();
        store
            .record(
                Path::new("/repo"),
                "migrate",
                "migrate-agent",
                "Migrate Agent",
            )
            .unwrap();
        store.save_to(&json_path).unwrap();

        // Migrate
        let migrated = AgentHistoryStore::migrate_if_needed().unwrap();
        assert!(migrated);

        // TOML should now exist
        let gwt_dir = temp_dir.path().join(".gwt");
        let toml_path = gwt_dir.join("agent-history.toml");
        assert!(toml_path.exists());

        // Load should work
        let loaded = AgentHistoryStore::load().unwrap();
        let entry = loaded.get(Path::new("/repo"), "migrate").unwrap();
        assert_eq!(entry.agent_id, "migrate-agent");

        // Second migration should be no-op
        let migrated_again = AgentHistoryStore::migrate_if_needed().unwrap();
        assert!(!migrated_again);

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }
}
