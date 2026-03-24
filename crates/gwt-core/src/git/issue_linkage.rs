//! Worktree-Issue local linkage store.
//!
//! Maps branch names to GitHub issue numbers with source tracking.
//! See gwt-spec #1714.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

/// How the linkage was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkSource {
    /// `gh issue develop` GitHub-side linkage.
    GitHubLinkage,
    /// Parsed from branch name pattern `issue-<number>`.
    BranchParse,
    /// User-initiated manual linkage.
    Manual,
}

impl LinkSource {
    /// Higher values take priority when overwriting.
    fn priority(self) -> u8 {
        match self {
            Self::BranchParse => 0,
            Self::Manual => 1,
            Self::GitHubLinkage => 2,
        }
    }
}

/// A single worktree-issue link entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeIssueLinkEntry {
    pub branch_name: String,
    pub issue_number: u64,
    pub source: LinkSource,
    /// Unix millis when first linked.
    pub linked_at: i64,
    /// Unix millis when last updated.
    pub updated_at: i64,
}

/// Per-repository worktree-issue linkage store.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeIssueLinkStore {
    /// branch_name -> link entry
    pub links: HashMap<String, WorktreeIssueLinkEntry>,
}

/// Branches that should never be auto-linked.
const EXCLUDED_BRANCHES: &[&str] = &["main", "master", "develop"];

fn is_excluded_branch(branch: &str) -> bool {
    let leaf = branch.rsplit('/').next().unwrap_or(branch);
    EXCLUDED_BRANCHES
        .iter()
        .any(|excl| leaf.eq_ignore_ascii_case(excl))
}

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

fn linkage_file_path(repo_path: &Path) -> PathBuf {
    let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(".gwt")
        .join("cache")
        .join("issue-links")
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

/// Extract issue number from branch name using `issue-<number>` convention.
///
/// Re-exports the same logic used by `issue.rs` to keep behavior consistent.
pub fn extract_issue_number(branch: &str) -> Option<u64> {
    for segment in branch.trim().split('/') {
        let lower = segment.to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("issue-") else {
            continue;
        };
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        if let Ok(number) = digits.parse::<u64>() {
            return Some(number);
        }
    }
    None
}

impl WorktreeIssueLinkStore {
    /// Load store from disk. Returns empty store on missing or corrupted file.
    pub fn load(repo_path: &Path) -> Self {
        let path = linkage_file_path(repo_path);
        let data = match fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to read issue linkage store"
                );
                return Self::default();
            }
        };
        serde_json::from_str(&data).unwrap_or_else(|e| {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to parse issue linkage store"
            );
            Self::default()
        })
    }

    /// Persist store to disk atomically.
    pub fn save(&self, repo_path: &Path) -> Result<(), String> {
        let path = linkage_file_path(repo_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create linkage directory: {e}"))?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("Serialization error: {e}"))?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &json).map_err(|e| format!("Failed to write linkage file: {e}"))?;
        rename_with_replace(&tmp, &path, "linkage")?;
        Ok(())
    }

    /// Get linkage for a branch.
    pub fn get_link(&self, branch: &str) -> Option<&WorktreeIssueLinkEntry> {
        self.links.get(branch)
    }

    /// Set linkage. Respects source priority: higher-priority sources
    /// overwrite lower ones; lower-priority sources do not overwrite higher.
    pub fn set_link(&mut self, branch: &str, issue_number: u64, source: LinkSource) {
        let now = now_millis();
        if let Some(existing) = self.links.get(branch) {
            if source.priority() < existing.source.priority() {
                return; // Do not overwrite higher-priority source
            }
        }
        self.links.insert(
            branch.to_string(),
            WorktreeIssueLinkEntry {
                branch_name: branch.to_string(),
                issue_number,
                source,
                linked_at: self.links.get(branch).map(|e| e.linked_at).unwrap_or(now),
                updated_at: now,
            },
        );
    }

    /// Remove linkage for a branch.
    pub fn remove_link(&mut self, branch: &str) -> Option<WorktreeIssueLinkEntry> {
        self.links.remove(branch)
    }

    /// Bootstrap linkage from a list of branch names by parsing `issue-<number>`.
    ///
    /// Skips excluded branches (main, master, develop) and branches that
    /// already have a linkage entry.
    pub fn bootstrap_from_branches(&mut self, branches: &[String]) {
        for branch in branches {
            if is_excluded_branch(branch) {
                continue;
            }
            if self.links.contains_key(branch.as_str()) {
                continue;
            }
            if let Some(number) = extract_issue_number(branch) {
                self.set_link(branch, number, LinkSource::BranchParse);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_entry() {
        let entry = WorktreeIssueLinkEntry {
            branch_name: "feature/issue-42".to_string(),
            issue_number: 42,
            source: LinkSource::BranchParse,
            linked_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: WorktreeIssueLinkEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.branch_name, "feature/issue-42");
        assert_eq!(parsed.issue_number, 42);
        assert_eq!(parsed.source, LinkSource::BranchParse);
    }

    #[test]
    fn serde_roundtrip_store() {
        let mut store = WorktreeIssueLinkStore::default();
        store.set_link("feature/issue-1", 1, LinkSource::BranchParse);
        store.set_link("feature/issue-2", 2, LinkSource::GitHubLinkage);

        let json = serde_json::to_string_pretty(&store).unwrap();
        let parsed: WorktreeIssueLinkStore = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.links.len(), 2);
        assert_eq!(
            parsed.links["feature/issue-1"].source,
            LinkSource::BranchParse
        );
        assert_eq!(
            parsed.links["feature/issue-2"].source,
            LinkSource::GitHubLinkage
        );
    }

    #[test]
    fn extract_issue_number_basic() {
        assert_eq!(extract_issue_number("feature/issue-42"), Some(42));
        assert_eq!(extract_issue_number("issue-1"), Some(1));
        assert_eq!(extract_issue_number("bugfix/issue-100-fix"), Some(100));
        assert_eq!(extract_issue_number("origin/feature/ISSUE-9"), Some(9));
    }

    #[test]
    fn extract_issue_number_no_match() {
        assert_eq!(extract_issue_number("main"), None);
        assert_eq!(extract_issue_number("feature/no-number"), None);
        assert_eq!(extract_issue_number("issue-"), None);
    }

    #[test]
    fn excluded_branches() {
        assert!(is_excluded_branch("main"));
        assert!(is_excluded_branch("master"));
        assert!(is_excluded_branch("develop"));
        assert!(is_excluded_branch("origin/main"));
        assert!(is_excluded_branch("origin/develop"));
        assert!(!is_excluded_branch("feature/issue-42"));
        assert!(!is_excluded_branch("maintenance"));
    }

    #[test]
    fn set_link_and_get() {
        let mut store = WorktreeIssueLinkStore::default();
        assert!(store.get_link("feature/issue-42").is_none());

        store.set_link("feature/issue-42", 42, LinkSource::BranchParse);
        let link = store.get_link("feature/issue-42").unwrap();
        assert_eq!(link.issue_number, 42);
        assert_eq!(link.source, LinkSource::BranchParse);
    }

    #[test]
    fn source_priority_higher_overwrites() {
        let mut store = WorktreeIssueLinkStore::default();
        store.set_link("feature/issue-42", 42, LinkSource::BranchParse);
        assert_eq!(
            store.get_link("feature/issue-42").unwrap().source,
            LinkSource::BranchParse
        );

        // Higher priority overwrites
        store.set_link("feature/issue-42", 42, LinkSource::GitHubLinkage);
        assert_eq!(
            store.get_link("feature/issue-42").unwrap().source,
            LinkSource::GitHubLinkage
        );
    }

    #[test]
    fn source_priority_lower_does_not_overwrite() {
        let mut store = WorktreeIssueLinkStore::default();
        store.set_link("feature/issue-42", 42, LinkSource::GitHubLinkage);

        // Lower priority does not overwrite
        store.set_link("feature/issue-42", 99, LinkSource::BranchParse);
        assert_eq!(store.get_link("feature/issue-42").unwrap().issue_number, 42);
        assert_eq!(
            store.get_link("feature/issue-42").unwrap().source,
            LinkSource::GitHubLinkage
        );
    }

    #[test]
    fn remove_link() {
        let mut store = WorktreeIssueLinkStore::default();
        store.set_link("feature/issue-42", 42, LinkSource::BranchParse);
        assert!(store.get_link("feature/issue-42").is_some());

        let removed = store.remove_link("feature/issue-42");
        assert!(removed.is_some());
        assert!(store.get_link("feature/issue-42").is_none());
    }

    #[test]
    fn bootstrap_from_branches() {
        let mut store = WorktreeIssueLinkStore::default();
        let branches = vec![
            "main".to_string(),
            "develop".to_string(),
            "feature/issue-42".to_string(),
            "bugfix/issue-100-fix".to_string(),
            "feature/no-issue".to_string(),
        ];

        store.bootstrap_from_branches(&branches);
        assert_eq!(store.links.len(), 2);
        assert_eq!(store.get_link("feature/issue-42").unwrap().issue_number, 42);
        assert_eq!(
            store.get_link("bugfix/issue-100-fix").unwrap().issue_number,
            100
        );
        assert!(store.get_link("main").is_none());
        assert!(store.get_link("develop").is_none());
    }

    #[test]
    fn bootstrap_skips_existing() {
        let mut store = WorktreeIssueLinkStore::default();
        store.set_link("feature/issue-42", 42, LinkSource::GitHubLinkage);

        let branches = vec!["feature/issue-42".to_string()];
        store.bootstrap_from_branches(&branches);

        // Should not overwrite existing GitHubLinkage entry
        assert_eq!(
            store.get_link("feature/issue-42").unwrap().source,
            LinkSource::GitHubLinkage
        );
    }

    #[test]
    fn file_io_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path();

        let mut store = WorktreeIssueLinkStore::default();
        store.set_link("feature/issue-10", 10, LinkSource::BranchParse);
        store.set_link("feature/issue-20", 20, LinkSource::GitHubLinkage);

        store.save(repo_path).unwrap();

        let loaded = WorktreeIssueLinkStore::load(repo_path);
        assert_eq!(loaded.links.len(), 2);
        assert_eq!(loaded.links["feature/issue-10"].issue_number, 10);
        assert_eq!(
            loaded.links["feature/issue-20"].source,
            LinkSource::GitHubLinkage
        );
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let store = WorktreeIssueLinkStore::load(tmp.path());
        assert!(store.links.is_empty());
    }
}
