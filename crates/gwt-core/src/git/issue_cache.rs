//! Local exact issue cache for offline-first UI display.
//!
//! Provides `issue_number -> issue metadata` lookup without hitting GitHub on
//! every render. Supports diff sync (watermark-based) and full sync (with stale
//! cleanup). See gwt-spec #1714.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// Sync strategy type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncType {
    Diff,
    Full,
}

/// Result of a cache sync operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub sync_type: SyncType,
    pub updated_count: u32,
    pub deleted_count: u32,
    pub duration_ms: u64,
    pub completed_at: i64,
    pub error: Option<String>,
}

/// Sync state tracking for watermark-based diff sync.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCacheSyncState {
    pub last_diff_sync_at: Option<i64>,
    pub last_full_sync_at: Option<i64>,
    /// ISO 8601 watermark for `since=` parameter.
    pub last_issue_updated_at: Option<String>,
    pub last_result: Option<SyncResult>,
}

/// A single cached issue entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueExactCacheEntry {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub labels: Vec<String>,
    /// GitHub-side `updatedAt` (ISO 8601).
    pub updated_at: String,
    /// Local fetch timestamp (Unix millis).
    pub fetched_at: i64,
}

/// Per-repository exact issue cache.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueExactCache {
    pub entries: HashMap<u64, IssueExactCacheEntry>,
    pub sync_state: IssueCacheSyncState,
}

// ── helpers ──

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn repo_hash(repo_path: &Path) -> String {
    let canonical = dunce::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let hash = format!("{digest:x}");
    hash[..16].to_string()
}

fn cache_file_path(repo_path: &Path) -> PathBuf {
    let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(".gwt")
        .join("cache")
        .join("issue-exact")
        .join(format!("{}.json", repo_hash(repo_path)))
}

fn rename_with_replace(tmp: &Path, path: &Path, label: &str) -> Result<(), String> {
    match fs::rename(tmp, path) {
        Ok(()) => Ok(()),
        Err(rename_err) if path.exists() => {
            fs::remove_file(path)
                .map_err(|e| format!("Failed to replace existing {label} file: {e}"))?;
            fs::rename(tmp, path).map_err(|e| {
                format!(
                    "Failed to rename {label} file after replace fallback (initial rename error: {rename_err}): {e}"
                )
            })
        }
        Err(rename_err) => Err(format!("Failed to rename {label} file: {rename_err}")),
    }
}

// ── IssueExactCache public API ──

impl IssueExactCache {
    /// Load cache from disk. Returns empty cache if file does not exist or is
    /// malformed (NFR-006: never destroy existing data on read failure).
    pub fn load(repo_path: &Path) -> Self {
        let path = cache_file_path(repo_path);
        let data = match fs::read_to_string(&path) {
            Ok(d) => d,
            Err(_) => return Self::default(),
        };
        serde_json::from_str(&data).unwrap_or_default()
    }

    /// Persist cache to disk atomically (write-tmp then rename).
    pub fn save(&self, repo_path: &Path) -> Result<(), String> {
        let path = cache_file_path(repo_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create cache directory: {e}"))?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("Serialization error: {e}"))?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &json).map_err(|e| format!("Failed to write cache file: {e}"))?;
        rename_with_replace(&tmp, &path, "cache")?;
        Ok(())
    }

    /// Look up a single cached entry.
    pub fn get(&self, issue_number: u64) -> Option<&IssueExactCacheEntry> {
        self.entries.get(&issue_number)
    }

    /// Insert or update a cache entry from a fetched issue.
    pub fn upsert(&mut self, entry: IssueExactCacheEntry) {
        self.entries.insert(entry.number, entry);
    }

    /// Remove an entry (used during full sync stale cleanup).
    pub fn remove(&mut self, issue_number: u64) -> Option<IssueExactCacheEntry> {
        self.entries.remove(&issue_number)
    }

    /// All cached entries (read-only). Used by #1520 for semantic index input.
    pub fn all_entries(&self) -> &HashMap<u64, IssueExactCacheEntry> {
        &self.entries
    }

    /// Build a cache entry from a `GitHubIssue`.
    pub fn entry_from_github_issue(issue: &super::issue::GitHubIssue) -> IssueExactCacheEntry {
        IssueExactCacheEntry {
            number: issue.number,
            title: issue.title.clone(),
            url: issue.html_url.clone(),
            state: issue.state.clone(),
            labels: issue.labels.iter().map(|l| l.name.clone()).collect(),
            updated_at: issue.updated_at.clone(),
            fetched_at: now_millis(),
        }
    }

    // ── Phase 3: lookup chain ──

    /// Resolve an issue from cache, falling back to online fetch.
    ///
    /// 1. Cache hit → return immediately
    /// 2. Cache miss → `fetch_issue_detail()` (includes REST fallback)
    /// 3. Online success → update cache, return entry
    /// 4. Online fail + cache hit → return stale cache (NFR-006)
    /// 5. Online fail + cache miss → return None
    pub fn resolve(&mut self, repo_path: &Path, issue_number: u64) -> Option<IssueExactCacheEntry> {
        // Fast path: cache hit
        if let Some(entry) = self.entries.get(&issue_number) {
            return Some(entry.clone());
        }

        // Slow path: online fetch
        match super::issue::fetch_issue_detail(repo_path, issue_number) {
            Ok(issue) => {
                let entry = Self::entry_from_github_issue(&issue);
                self.upsert(entry.clone());
                Some(entry)
            }
            Err(_) => None,
        }
    }

    // ── Phase 5: sync strategies ──

    /// Diff sync: fetch issues updated since the watermark.
    ///
    /// Uses GitHub REST API `since` parameter. Adds new issues and updates
    /// existing ones. Does NOT delete stale entries (NFR-003).
    pub fn diff_sync(&mut self, repo_path: &Path) -> Result<SyncResult, String> {
        let start = std::time::Instant::now();
        let since = self.sync_state.last_issue_updated_at.as_deref();

        let issues = match super::issue::fetch_all_issues_via_rest(repo_path, since) {
            Ok(issues) => issues,
            Err(e) => {
                let now = now_millis();
                let result = SyncResult {
                    sync_type: SyncType::Diff,
                    updated_count: 0,
                    deleted_count: 0,
                    duration_ms: start.elapsed().as_millis() as u64,
                    completed_at: now,
                    error: Some(e.clone()),
                };
                self.sync_state.last_result = Some(result.clone());
                return Err(e);
            }
        };

        let mut updated_count = 0u32;
        let mut max_updated_at: Option<String> = self.sync_state.last_issue_updated_at.clone();

        for issue in &issues {
            let entry = Self::entry_from_github_issue(issue);
            // Track the latest updated_at as new watermark
            if max_updated_at
                .as_ref()
                .map(|current| entry.updated_at > *current)
                .unwrap_or(true)
            {
                max_updated_at = Some(entry.updated_at.clone());
            }
            self.upsert(entry);
            updated_count += 1;
        }

        let now = now_millis();
        if let Some(watermark) = max_updated_at {
            self.sync_state.last_issue_updated_at = Some(watermark);
        }
        self.sync_state.last_diff_sync_at = Some(now);

        let result = SyncResult {
            sync_type: SyncType::Diff,
            updated_count,
            deleted_count: 0,
            duration_ms: start.elapsed().as_millis() as u64,
            completed_at: now,
            error: None,
        };
        self.sync_state.last_result = Some(result.clone());
        Ok(result)
    }

    /// Full sync: fetch all issues and reconcile with cache.
    ///
    /// Stale entries (present in cache but not on GitHub) are deleted only
    /// when the full fetch completes successfully. On network interruption,
    /// partial results are merged but stale cleanup is skipped (NFR-004).
    pub fn full_sync(&mut self, repo_path: &Path) -> Result<SyncResult, String> {
        let start = std::time::Instant::now();

        let issues = match super::issue::fetch_all_issues_via_rest(repo_path, None) {
            Ok(issues) => issues,
            Err(e) => {
                // Network failure → return error without destroying cache
                let now = now_millis();
                let result = SyncResult {
                    sync_type: SyncType::Full,
                    updated_count: 0,
                    deleted_count: 0,
                    duration_ms: start.elapsed().as_millis() as u64,
                    completed_at: now,
                    error: Some(e.clone()),
                };
                self.sync_state.last_result = Some(result.clone());
                return Err(e);
            }
        };

        // Build set of live issue numbers
        let mut live_numbers = std::collections::HashSet::new();
        let mut updated_count = 0u32;
        let mut max_updated_at: Option<String> = None;

        for issue in &issues {
            live_numbers.insert(issue.number);
            let entry = Self::entry_from_github_issue(issue);
            if max_updated_at
                .as_ref()
                .map(|current| entry.updated_at > *current)
                .unwrap_or(true)
            {
                max_updated_at = Some(entry.updated_at.clone());
            }
            self.upsert(entry);
            updated_count += 1;
        }

        // Stale cleanup: remove entries not in the live set
        let stale_numbers: Vec<u64> = self
            .entries
            .keys()
            .filter(|k| !live_numbers.contains(k))
            .copied()
            .collect();
        let deleted_count = stale_numbers.len() as u32;
        for number in stale_numbers {
            self.entries.remove(&number);
        }

        let now = now_millis();
        if let Some(watermark) = max_updated_at {
            self.sync_state.last_issue_updated_at = Some(watermark);
        }
        self.sync_state.last_full_sync_at = Some(now);
        self.sync_state.last_diff_sync_at = Some(now);

        let result = SyncResult {
            sync_type: SyncType::Full,
            updated_count,
            deleted_count,
            duration_ms: start.elapsed().as_millis() as u64,
            completed_at: now,
            error: None,
        };
        self.sync_state.last_result = Some(result.clone());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(number: u64) -> IssueExactCacheEntry {
        IssueExactCacheEntry {
            number,
            title: format!("Issue #{number}"),
            url: format!("https://github.com/owner/repo/issues/{number}"),
            state: "OPEN".to_string(),
            labels: vec!["bug".to_string()],
            updated_at: "2026-03-19T00:00:00Z".to_string(),
            fetched_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn serde_roundtrip_entry() {
        let entry = sample_entry(42);
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: IssueExactCacheEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.number, 42);
        assert_eq!(parsed.title, "Issue #42");
        assert_eq!(parsed.labels, vec!["bug"]);
    }

    #[test]
    fn serde_roundtrip_cache() {
        let mut cache = IssueExactCache::default();
        cache.upsert(sample_entry(1));
        cache.upsert(sample_entry(2));
        cache.sync_state.last_diff_sync_at = Some(1_700_000_000_000);
        cache.sync_state.last_issue_updated_at = Some("2026-03-19T00:00:00Z".to_string());

        let json = serde_json::to_string_pretty(&cache).unwrap();
        let parsed: IssueExactCache = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert!(parsed.entries.contains_key(&1));
        assert!(parsed.entries.contains_key(&2));
        assert_eq!(parsed.sync_state.last_diff_sync_at, Some(1_700_000_000_000));
    }

    #[test]
    fn serde_roundtrip_sync_result() {
        let result = SyncResult {
            sync_type: SyncType::Full,
            updated_count: 10,
            deleted_count: 2,
            duration_ms: 500,
            completed_at: 1_700_000_000_000,
            error: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: SyncResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.sync_type, SyncType::Full);
        assert_eq!(parsed.updated_count, 10);
        assert_eq!(parsed.deleted_count, 2);
    }

    #[test]
    fn empty_cache_default() {
        let cache = IssueExactCache::default();
        assert!(cache.entries.is_empty());
        assert!(cache.sync_state.last_diff_sync_at.is_none());
        assert!(cache.sync_state.last_full_sync_at.is_none());
    }

    #[test]
    fn upsert_and_get() {
        let mut cache = IssueExactCache::default();
        assert!(cache.get(42).is_none());

        cache.upsert(sample_entry(42));
        let entry = cache.get(42).unwrap();
        assert_eq!(entry.title, "Issue #42");

        // Upsert updates existing
        let mut updated = sample_entry(42);
        updated.title = "Updated title".to_string();
        cache.upsert(updated);
        assert_eq!(cache.get(42).unwrap().title, "Updated title");
    }

    #[test]
    fn remove_entry() {
        let mut cache = IssueExactCache::default();
        cache.upsert(sample_entry(42));
        assert!(cache.get(42).is_some());

        let removed = cache.remove(42);
        assert!(removed.is_some());
        assert!(cache.get(42).is_none());
    }

    #[test]
    fn all_entries_returns_full_map() {
        let mut cache = IssueExactCache::default();
        cache.upsert(sample_entry(1));
        cache.upsert(sample_entry(2));
        cache.upsert(sample_entry(3));
        assert_eq!(cache.all_entries().len(), 3);
    }

    #[test]
    fn file_io_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path();

        let mut cache = IssueExactCache::default();
        cache.upsert(sample_entry(100));
        cache.upsert(sample_entry(200));
        cache.sync_state.last_full_sync_at = Some(1_700_000_000_000);

        // Override cache file path for testing
        let path = cache_file_path(repo_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        cache.save(repo_path).unwrap();

        let loaded = IssueExactCache::load(repo_path);
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[&100].title, "Issue #100");
        assert_eq!(loaded.entries[&200].title, "Issue #200");
        assert_eq!(loaded.sync_state.last_full_sync_at, Some(1_700_000_000_000));
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = IssueExactCache::load(tmp.path());
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn load_corrupted_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = cache_file_path(tmp.path());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, "not valid json").unwrap();

        let cache = IssueExactCache::load(tmp.path());
        assert!(cache.entries.is_empty());
    }
}
