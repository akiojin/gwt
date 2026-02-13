//! Agent launch statistics and worktree creation counts.
//!
//! Persisted to `~/.gwt/stats.toml`.

use crate::config::migration::{backup_broken_file, ensure_config_dir, write_atomic};
use crate::error::{GwtError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Per-scope statistics entry (used for both global and per-repo).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatsEntry {
    /// Agent launch counts keyed by "{agent_id}.{model}".
    pub agents: HashMap<String, u64>,
    /// Number of worktrees created.
    pub worktrees_created: u64,
}

/// Root statistics structure persisted to `~/.gwt/stats.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    /// Global (all repos) statistics.
    pub global: StatsEntry,
    /// Per-repository statistics keyed by absolute repo path.
    pub repos: HashMap<String, StatsEntry>,
}

impl Stats {
    /// TOML config file path (~/.gwt/stats.toml).
    pub fn toml_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".gwt").join("stats.toml")
    }

    /// Load stats from disk. Returns default on missing or corrupted file.
    pub fn load() -> Result<Self> {
        let path = Self::toml_path();

        if !path.exists() {
            return Ok(Self::default());
        }

        debug!(
            category = "stats",
            path = %path.display(),
            "Loading stats (TOML)"
        );

        match Self::load_toml(&path) {
            Ok(stats) => Ok(stats),
            Err(e) => {
                warn!(
                    category = "stats",
                    path = %path.display(),
                    error = %e,
                    "Failed to load stats; falling back to defaults"
                );
                let _ = backup_broken_file(&path);
                Ok(Self::default())
            }
        }
    }

    fn load_toml(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let stats: Stats = toml::from_str(&content).map_err(|e| GwtError::ConfigParseError {
            reason: format!("Failed to parse stats TOML: {}", e),
        })?;
        Ok(stats)
    }

    /// Save stats to disk atomically.
    pub fn save(&self) -> Result<()> {
        let path = Self::toml_path();

        if let Some(parent) = path.parent() {
            ensure_config_dir(parent)?;
        }

        let content = toml::to_string_pretty(self).map_err(|e| GwtError::ConfigWriteError {
            reason: format!("Failed to serialize stats to TOML: {}", e),
        })?;

        write_atomic(&path, &content)?;

        info!(
            category = "stats",
            path = %path.display(),
            "Saved stats (TOML)"
        );
        Ok(())
    }

    /// Increment agent launch count for both global and repo-specific stats.
    ///
    /// Key format: `"{agent_id}.{model}"`. If model is empty, `"default"` is used.
    pub fn increment_agent_launch(&mut self, agent_id: &str, model: &str, repo_path: &str) {
        let model = if model.is_empty() { "default" } else { model };
        let key = format!("{}.{}", agent_id, model);

        *self.global.agents.entry(key.clone()).or_insert(0) += 1;
        *self
            .repos
            .entry(repo_path.to_string())
            .or_default()
            .agents
            .entry(key)
            .or_insert(0) += 1;
    }

    /// Increment worktree creation count for both global and repo-specific stats.
    pub fn increment_worktree_created(&mut self, repo_path: &str) {
        self.global.worktrees_created += 1;
        self.repos
            .entry(repo_path.to_string())
            .or_default()
            .worktrees_created += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- T011: default + roundtrip ---

    #[test]
    fn stats_default_is_empty() {
        let stats = Stats::default();
        assert!(stats.global.agents.is_empty());
        assert_eq!(stats.global.worktrees_created, 0);
        assert!(stats.repos.is_empty());
    }

    #[test]
    fn stats_serialize_deserialize_roundtrip() {
        let stats = Stats::default();
        let toml_str = toml::to_string_pretty(&stats).unwrap();
        let deserialized: Stats = toml::from_str(&toml_str).unwrap();
        assert!(deserialized.global.agents.is_empty());
        assert_eq!(deserialized.global.worktrees_created, 0);
        assert!(deserialized.repos.is_empty());
    }

    // --- T012: increment_agent_launch ---

    #[test]
    fn increment_agent_launch_creates_key() {
        let mut stats = Stats::default();
        stats.increment_agent_launch("claude-code", "claude-sonnet", "/path/repo");

        assert_eq!(
            stats.global.agents.get("claude-code.claude-sonnet"),
            Some(&1)
        );
        assert_eq!(
            stats
                .repos
                .get("/path/repo")
                .unwrap()
                .agents
                .get("claude-code.claude-sonnet"),
            Some(&1)
        );
    }

    #[test]
    fn increment_agent_launch_twice_increments() {
        let mut stats = Stats::default();
        stats.increment_agent_launch("claude-code", "claude-sonnet", "/path/repo");
        stats.increment_agent_launch("claude-code", "claude-sonnet", "/path/repo");

        assert_eq!(
            stats.global.agents.get("claude-code.claude-sonnet"),
            Some(&2)
        );
        assert_eq!(
            stats
                .repos
                .get("/path/repo")
                .unwrap()
                .agents
                .get("claude-code.claude-sonnet"),
            Some(&2)
        );
    }

    #[test]
    fn increment_agent_launch_different_repos() {
        let mut stats = Stats::default();
        stats.increment_agent_launch("claude-code", "claude-sonnet", "/repo/a");
        stats.increment_agent_launch("claude-code", "claude-sonnet", "/repo/b");

        assert_eq!(
            stats.global.agents.get("claude-code.claude-sonnet"),
            Some(&2)
        );
        assert_eq!(
            stats
                .repos
                .get("/repo/a")
                .unwrap()
                .agents
                .get("claude-code.claude-sonnet"),
            Some(&1)
        );
        assert_eq!(
            stats
                .repos
                .get("/repo/b")
                .unwrap()
                .agents
                .get("claude-code.claude-sonnet"),
            Some(&1)
        );
    }

    #[test]
    fn increment_agent_launch_empty_model_uses_default() {
        let mut stats = Stats::default();
        stats.increment_agent_launch("custom-agent", "", "/path/repo");

        assert_eq!(stats.global.agents.get("custom-agent.default"), Some(&1));
    }

    // --- T013: increment_worktree_created ---

    #[test]
    fn increment_worktree_created_basic() {
        let mut stats = Stats::default();
        stats.increment_worktree_created("/path/repo");

        assert_eq!(stats.global.worktrees_created, 1);
        assert_eq!(stats.repos.get("/path/repo").unwrap().worktrees_created, 1);
    }

    #[test]
    fn increment_worktree_created_different_repos() {
        let mut stats = Stats::default();
        stats.increment_worktree_created("/repo/a");
        stats.increment_worktree_created("/repo/b");

        assert_eq!(stats.global.worktrees_created, 2);
        assert_eq!(stats.repos.get("/repo/a").unwrap().worktrees_created, 1);
        assert_eq!(stats.repos.get("/repo/b").unwrap().worktrees_created, 1);
    }

    // --- T017: load / save ---

    #[test]
    fn load_nonexistent_returns_default() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let stats = Stats::load().unwrap();
        assert!(stats.global.agents.is_empty());
        assert_eq!(stats.global.worktrees_created, 0);
        assert!(stats.repos.is_empty());
    }

    #[test]
    fn save_load_roundtrip() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let mut stats = Stats::default();
        stats.increment_agent_launch("claude-code", "claude-sonnet", "/repo/test");
        stats.increment_agent_launch("claude-code", "claude-sonnet", "/repo/test");
        stats.increment_agent_launch("codex", "o3", "/repo/test");
        stats.increment_worktree_created("/repo/test");
        stats.save().unwrap();

        let loaded = Stats::load().unwrap();
        assert_eq!(
            loaded.global.agents.get("claude-code.claude-sonnet"),
            Some(&2)
        );
        assert_eq!(loaded.global.agents.get("codex.o3"), Some(&1));
        assert_eq!(loaded.global.worktrees_created, 1);
        assert_eq!(
            loaded
                .repos
                .get("/repo/test")
                .unwrap()
                .agents
                .get("claude-code.claude-sonnet"),
            Some(&2)
        );
    }

    #[test]
    fn load_corrupted_returns_default_and_backs_up() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let path = Stats::toml_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, "this is not valid toml ==").unwrap();

        let stats = Stats::load().unwrap();
        assert!(stats.global.agents.is_empty());
        assert_eq!(stats.global.worktrees_created, 0);

        let broken = path.with_extension("broken");
        assert!(broken.exists());
        assert!(!path.exists());
    }
}
