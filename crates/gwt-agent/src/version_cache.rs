//! Agent version cache: caches detected versions with a 24-hour TTL.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

use crate::types::AgentId;

/// Time-to-live for cached version entries (24 hours).
const TTL_SECS: i64 = 86400;

/// A single version option presented to the user in the wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionOption {
    /// Display label (e.g., "Installed (v1.2.3)", "1.2.3", "latest").
    pub label: String,
    /// Value used for launch resolution ("installed", "latest", "1.2.3").
    pub value: String,
}

/// Build the list of version options for the wizard VersionSelect step.
///
/// - If `is_installed` is true and `installed_version` is provided, prepends "Installed (vX.Y.Z)".
/// - If `is_installed` is true but version is unknown, prepends "Installed".
/// - Appends "latest" if the agent has an npm package.
/// - Appends each cached version from `cached_versions`.
pub fn build_version_options(
    is_installed: bool,
    installed_version: Option<&str>,
    has_npm_package: bool,
    cached_versions: &[String],
) -> Vec<VersionOption> {
    let mut options = Vec::new();
    let installed_version = installed_version.map(str::trim);

    if is_installed {
        let label = match installed_version {
            Some(v) => format!("Installed (v{})", v),
            None => "Installed".to_string(),
        };
        options.push(VersionOption {
            label,
            value: "installed".to_string(),
        });
    }

    if has_npm_package {
        options.push(VersionOption {
            label: "latest".to_string(),
            value: "latest".to_string(),
        });

        for version in cached_versions {
            if installed_version == Some(version.as_str()) {
                continue;
            }
            options.push(VersionOption {
                label: version.clone(),
                value: version.clone(),
            });
        }
    }

    options
}

/// Maximum number of version strings retained per agent.
const MAX_VERSIONS_PER_AGENT: usize = 10;

/// A single cache entry for one agent's version history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    pub versions: Vec<String>,
    #[serde(rename = "fetched_at", alias = "updated_at")]
    pub updated_at: DateTime<Utc>,
}

/// Errors produced while fetching or parsing version metadata.
#[derive(Debug, thiserror::Error)]
pub enum VersionCacheError {
    #[error("network error: {0}")]
    Network(String),
    #[error("parse error: {0}")]
    Parse(String),
}

/// Cache mapping agent IDs to their recent version strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCache {
    #[serde(rename = "agents", alias = "entries")]
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
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|err| {
                debug!(path = %path.display(), error = %err, "Failed to parse version cache");
                Self::new()
            }),
            Err(err) => {
                debug!(path = %path.display(), error = %err, "Failed to read version cache");
                Self::new()
            }
        }
    }

    /// Save the cache to a JSON file using an atomic temp-file replace.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let tmp_path = temp_path_for(path);
        std::fs::write(&tmp_path, content)?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        std::fs::rename(tmp_path, path)
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

    /// Return `true` if the cache entry is missing or stale.
    pub fn needs_refresh(&self, agent_id: &AgentId) -> bool {
        self.get(agent_id).is_none()
    }

    /// Record a new version for an agent, maintaining the max-versions limit.
    pub fn record_version(&mut self, agent_id: &AgentId, version: String) {
        self.record_versions(agent_id, vec![version]);
    }

    /// Record a full version list for an agent.
    pub fn record_versions(&mut self, agent_id: &AgentId, versions: Vec<String>) {
        let key = Self::agent_key(agent_id);
        let entry = self.entries.entry(key).or_insert_with(|| VersionEntry {
            versions: Vec::new(),
            updated_at: Utc::now(),
        });

        for version in versions {
            if entry.versions.last().map(|v| v.as_str()) == Some(version.as_str()) {
                continue;
            }
            entry.versions.push(version);
        }

        while entry.versions.len() > MAX_VERSIONS_PER_AGENT {
            entry.versions.remove(0);
        }
        entry.updated_at = Utc::now();
    }

    /// Refresh version history for a given agent by running the npm registry query.
    pub async fn refresh(
        &mut self,
        agent_id: &AgentId,
    ) -> Result<Option<Vec<String>>, VersionCacheError> {
        self.refresh_from_source(agent_id, fetch_npm_versions).await
    }

    /// Refresh version history using a custom fetcher.
    pub async fn refresh_with_fetcher<F, Fut>(
        &mut self,
        agent_id: &AgentId,
        fetcher: F,
    ) -> Result<Option<Vec<String>>, VersionCacheError>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<String, VersionCacheError>>,
    {
        self.refresh_from_source(agent_id, fetcher).await
    }

    async fn refresh_from_source<F, Fut>(
        &mut self,
        agent_id: &AgentId,
        fetcher: F,
    ) -> Result<Option<Vec<String>>, VersionCacheError>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<String, VersionCacheError>>,
    {
        let Some(package) = agent_id.package_name() else {
            return Ok(None);
        };
        let url = npm_registry_url(package);
        debug!(package = package, %url, "Refreshing version history from npm registry");

        let payload = fetcher(url).await?;
        let versions = parse_npm_versions(&payload)?;
        self.record_versions(agent_id, versions.clone());
        Ok(Some(versions))
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

fn temp_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "agent-versions.json".to_string());
    path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()))
}

fn npm_registry_url(package: &str) -> String {
    let encoded = package.replace('@', "%40").replace('/', "%2F");
    format!("https://registry.npmjs.org/{encoded}")
}

fn parse_npm_versions(payload: &str) -> Result<Vec<String>, VersionCacheError> {
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|e| VersionCacheError::Parse(format!("invalid JSON: {e}")))?;
    let versions = value
        .get("versions")
        .and_then(|v| v.as_object())
        .ok_or_else(|| VersionCacheError::Parse("missing versions object".into()))?;

    let mut parsed: Vec<Version> = versions
        .keys()
        .filter_map(|version| Version::parse(version).ok())
        .collect();
    if parsed.is_empty() {
        return Err(VersionCacheError::Parse(
            "no semver versions found in npm payload".into(),
        ));
    }

    parsed.sort_unstable_by(|a, b| b.cmp(a));
    Ok(parsed
        .into_iter()
        .take(MAX_VERSIONS_PER_AGENT)
        .map(|version| version.to_string())
        .collect())
}

async fn fetch_npm_versions(url: String) -> Result<String, VersionCacheError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| VersionCacheError::Network(format!("client build failed: {e}")))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| VersionCacheError::Network(e.to_string()))?;

    if !response.status().is_success() {
        return Err(VersionCacheError::Network(format!(
            "registry returned {}",
            response.status()
        )));
    }

    response
        .text()
        .await
        .map_err(|e| VersionCacheError::Network(e.to_string()))
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
        assert_eq!(loaded.get(&AgentId::ClaudeCode).unwrap(), &["1.2.3"]);
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

    #[test]
    fn load_legacy_schema_uses_old_field_names() {
        let fetched_at = Utc::now().to_rfc3339();
        let json = r#"{
            "entries": {
                "codex": {
                    "versions": ["1.2.3"],
                    "updated_at": "__FETCHED_AT__"
                }
            }
        }"#
        .replace("__FETCHED_AT__", &fetched_at);

        let cache: VersionCache = serde_json::from_str(&json).unwrap();
        assert_eq!(cache.get(&AgentId::Codex).unwrap(), &["1.2.3"]);
    }

    #[test]
    fn build_version_options_installed_with_version_and_npm() {
        let opts = build_version_options(
            true,
            Some("1.2.3"),
            true,
            &["2.0.0".to_string(), "1.9.0".to_string()],
        );
        assert_eq!(opts.len(), 4); // Installed + latest + 2 cached
        assert_eq!(opts[0].label, "Installed (v1.2.3)");
        assert_eq!(opts[0].value, "installed");
        assert_eq!(opts[1].label, "latest");
        assert_eq!(opts[1].value, "latest");
        assert_eq!(opts[2].label, "2.0.0");
        assert_eq!(opts[2].value, "2.0.0");
        assert_eq!(opts[3].label, "1.9.0");
        assert_eq!(opts[3].value, "1.9.0");
    }

    #[test]
    fn build_version_options_not_installed_with_npm() {
        let opts = build_version_options(false, None, true, &["3.0.0".to_string()]);
        assert_eq!(opts.len(), 2); // latest + 1 cached
        assert_eq!(opts[0].label, "latest");
        assert_eq!(opts[0].value, "latest");
        assert_eq!(opts[1].label, "3.0.0");
        assert_eq!(opts[1].value, "3.0.0");
    }

    #[test]
    fn build_version_options_installed_without_npm() {
        // OpenCode/Copilot: installed but no npm package
        let opts = build_version_options(true, Some("0.1.0"), false, &[]);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].label, "Installed (v0.1.0)");
        assert_eq!(opts[0].value, "installed");
    }

    #[test]
    fn build_version_options_installed_no_version_string() {
        let opts = build_version_options(true, None, true, &[]);
        assert_eq!(opts.len(), 2); // "Installed" + "latest"
        assert_eq!(opts[0].label, "Installed");
        assert_eq!(opts[0].value, "installed");
    }

    #[test]
    fn build_version_options_nothing_available() {
        let opts = build_version_options(false, None, false, &[]);
        assert!(opts.is_empty());
    }

    #[test]
    fn build_version_options_skips_duplicate_installed_version() {
        let opts = build_version_options(
            true,
            Some("1.2.3"),
            true,
            &["1.2.3".to_string(), "1.2.2".to_string()],
        );
        assert_eq!(opts.len(), 3);
        assert_eq!(opts[0].label, "Installed (v1.2.3)");
        assert_eq!(opts[1].label, "latest");
        assert_eq!(opts[2].label, "1.2.2");
    }

    #[test]
    fn ttl_check_flags_stale_and_fresh_entries() {
        let mut cache = VersionCache::new();
        cache.record_version(&AgentId::ClaudeCode, "1.0.0".into());

        assert!(!cache.needs_refresh(&AgentId::ClaudeCode));
        assert!(cache.needs_refresh(&AgentId::Codex));

        if let Some(entry) = cache.entries.get_mut("claude-code") {
            entry.updated_at = Utc::now() - chrono::Duration::seconds(TTL_SECS + 1);
        }

        assert!(cache.needs_refresh(&AgentId::ClaudeCode));
    }

    #[tokio::test]
    async fn refresh_with_fetcher_records_last_ten_sorted_versions() {
        let mut cache = VersionCache::new();
        let payload = r#"{
            "versions": {
                "1.0.0": {},
                "2.0.0": {},
                "1.5.0": {},
                "3.0.0": {},
                "0.9.0": {},
                "2.1.0": {},
                "1.1.0": {},
                "4.0.0": {},
                "3.1.0": {},
                "5.0.0": {},
                "4.1.0": {},
                "6.0.0": {}
            }
        }"#;

        let versions = cache
            .refresh_with_fetcher(&AgentId::ClaudeCode, |_url| async move {
                Ok(payload.to_string())
            })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(versions.len(), MAX_VERSIONS_PER_AGENT);
        assert_eq!(versions[0], "6.0.0");
        assert_eq!(versions[9], "1.1.0");
        assert_eq!(cache.get(&AgentId::ClaudeCode).unwrap()[0], "6.0.0");
    }

    #[tokio::test]
    async fn refresh_with_fetcher_preserves_cache_on_failure() {
        let mut cache = VersionCache::new();
        cache.record_version(&AgentId::ClaudeCode, "1.0.0".into());

        let result = cache
            .refresh_with_fetcher(&AgentId::ClaudeCode, |_url| async move {
                Err(VersionCacheError::Network("boom".into()))
            })
            .await;

        assert!(result.is_err());
        assert_eq!(cache.get(&AgentId::ClaudeCode).unwrap(), &["1.0.0"]);
    }
}
