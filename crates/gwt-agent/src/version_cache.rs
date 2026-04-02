//! Agent version cache: caches detected versions with a 24-hour TTL.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::types::AgentId;

/// Time-to-live for cached version entries (24 hours).
const TTL_SECS: i64 = 86400;

/// Maximum number of version strings retained per agent.
const MAX_VERSIONS_PER_AGENT: usize = 10;

/// A single cache entry for one agent's version history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    pub versions: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

/// Cache mapping agent IDs to their recent version strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCache {
    pub entries: HashMap<String, VersionEntry>,
}

impl Default for VersionCache {
    fn default() -> Self {
        Self::new()
    }
}

impl VersionCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Load the cache from a JSON file, returning an empty cache on any error.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::new(),
        }
    }

    /// Save the cache to a JSON file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(path, content)
    }

    /// Get cached versions for an agent, or None if missing or expired.
    pub fn get(&self, agent_id: &AgentId) -> Option<&[String]> {
        let key = Self::agent_key(agent_id);
        let entry = self.entries.get(&key)?;
        if Self::is_expired(entry) {
            None
        } else {
            Some(&entry.versions)
        }
    }

    /// Record a new version for an agent, maintaining the max-versions limit.
    pub fn record_version(&mut self, agent_id: &AgentId, version: String) {
        let key = Self::agent_key(agent_id);
        let entry = self.entries.entry(key).or_insert_with(|| VersionEntry {
            versions: Vec::new(),
            updated_at: Utc::now(),
        });

        // Avoid duplicate consecutive entries
        if entry.versions.last().map(|v| v.as_str()) == Some(&version) {
            entry.updated_at = Utc::now();
            return;
        }

        entry.versions.push(version);
        if entry.versions.len() > MAX_VERSIONS_PER_AGENT {
            entry.versions.remove(0);
        }
        entry.updated_at = Utc::now();
    }

    /// Refresh version for a given agent by running the npm registry query.
    pub async fn refresh(&mut self, agent_id: &AgentId) -> Option<String> {
        let package = agent_id.package_name()?;
        debug!(package = package, "Refreshing version from npm registry");

        let url = format!("https://registry.npmjs.org/{}/latest", package);
        let version = tokio::task::spawn_blocking(move || {
            reqwest_get_version(&url)
        })
        .await
        .ok()
        .flatten()?;

        self.record_version(agent_id, version.clone());
        Some(version)
    }

    fn agent_key(agent_id: &AgentId) -> String {
        match agent_id {
            AgentId::ClaudeCode => "claude-code".to_string(),
            AgentId::Codex => "codex".to_string(),
            AgentId::Gemini => "gemini".to_string(),
            AgentId::OpenCode => "opencode".to_string(),
            AgentId::Copilot => "copilot".to_string(),
            AgentId::Custom(name) => format!("custom-{}", name),
        }
    }

    fn is_expired(entry: &VersionEntry) -> bool {
        let elapsed = Utc::now()
            .signed_duration_since(entry.updated_at)
            .num_seconds();
        elapsed >= TTL_SECS
    }
}

/// Blocking HTTP fetch for npm registry version.
fn reqwest_get_version(_url: &str) -> Option<String> {
    // Placeholder: in production this would do an HTTP GET.
    // We avoid adding reqwest as a direct dependency; gwt-core has it.
    // For now, return None — real implementation hooks into gwt-core's reqwest.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cache_is_empty() {
        let cache = VersionCache::new();
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn record_and_get_version() {
        let mut cache = VersionCache::new();
        cache.record_version(&AgentId::ClaudeCode, "1.0.0".into());

        let versions = cache.get(&AgentId::ClaudeCode).unwrap();
        assert_eq!(versions, &["1.0.0"]);
    }

    #[test]
    fn record_deduplicates_consecutive() {
        let mut cache = VersionCache::new();
        cache.record_version(&AgentId::ClaudeCode, "1.0.0".into());
        cache.record_version(&AgentId::ClaudeCode, "1.0.0".into());

        let versions = cache.get(&AgentId::ClaudeCode).unwrap();
        assert_eq!(versions.len(), 1);
    }

    #[test]
    fn record_caps_at_max_versions() {
        let mut cache = VersionCache::new();
        for i in 0..15 {
            cache.record_version(&AgentId::ClaudeCode, format!("1.0.{}", i));
        }

        let versions = cache.get(&AgentId::ClaudeCode).unwrap();
        assert_eq!(versions.len(), MAX_VERSIONS_PER_AGENT);
        // Oldest should be dropped
        assert_eq!(versions[0], "1.0.5");
        assert_eq!(versions[9], "1.0.14");
    }

    #[test]
    fn get_returns_none_for_unknown_agent() {
        let cache = VersionCache::new();
        assert!(cache.get(&AgentId::Codex).is_none());
    }

    #[test]
    fn expired_entry_returns_none() {
        let mut cache = VersionCache::new();
        cache.record_version(&AgentId::Codex, "2.0.0".into());
        // Manually expire the entry
        if let Some(entry) = cache.entries.get_mut("codex") {
            entry.updated_at = Utc::now() - chrono::Duration::seconds(TTL_SECS + 1);
        }
        assert!(cache.get(&AgentId::Codex).is_none());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache").join("agent-versions.json");

        let mut cache = VersionCache::new();
        cache.record_version(&AgentId::ClaudeCode, "1.2.3".into());
        cache.record_version(&AgentId::Codex, "0.5.0".into());
        cache.save(&path).unwrap();

        let loaded = VersionCache::load(&path);
        assert_eq!(
            loaded.get(&AgentId::ClaudeCode).unwrap(),
            &["1.2.3"]
        );
        assert_eq!(loaded.get(&AgentId::Codex).unwrap(), &["0.5.0"]);
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let cache = VersionCache::load(Path::new("/nonexistent/cache.json"));
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn load_invalid_json_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json!!!").unwrap();
        let cache = VersionCache::load(&path);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn agent_key_mapping() {
        assert_eq!(VersionCache::agent_key(&AgentId::ClaudeCode), "claude-code");
        assert_eq!(VersionCache::agent_key(&AgentId::Codex), "codex");
        assert_eq!(
            VersionCache::agent_key(&AgentId::Custom("aider".into())),
            "custom-aider"
        );
    }

    #[test]
    fn default_is_new() {
        let cache = VersionCache::default();
        assert!(cache.entries.is_empty());
    }
}
