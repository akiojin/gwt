//! Commit and branch summary data structures
//!
//! Provides data structures for displaying branch summary information
//! in the TUI panel.

use std::path::PathBuf;

use super::Branch;

/// Individual commit entry from git log --oneline
#[derive(Debug, Clone)]
pub struct CommitEntry {
    /// Commit hash (7 characters)
    pub hash: String,
    /// Commit message (first line only)
    pub message: String,
}

impl CommitEntry {
    /// Parse a git log --oneline output line
    ///
    /// # Example
    /// ```
    /// use gwt_core::git::CommitEntry;
    /// let entry = CommitEntry::from_oneline("a1b2c3d fix: update README").unwrap();
    /// assert_eq!(entry.hash, "a1b2c3d");
    /// assert_eq!(entry.message, "fix: update README");
    /// ```
    pub fn from_oneline(line: &str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 {
            Some(Self {
                hash: parts[0].to_string(),
                message: parts[1].to_string(),
            })
        } else if parts.len() == 1 && !parts[0].is_empty() {
            // Hash only, no message
            Some(Self {
                hash: parts[0].to_string(),
                message: String::new(),
            })
        } else {
            None
        }
    }

    /// Create a new CommitEntry
    pub fn new(hash: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            hash: hash.into(),
            message: message.into(),
        }
    }
}

/// Change statistics from git diff --shortstat
#[derive(Debug, Clone, Default)]
pub struct ChangeStats {
    /// Number of files changed
    pub files_changed: usize,
    /// Number of insertions
    pub insertions: usize,
    /// Number of deletions
    pub deletions: usize,
    /// Has uncommitted changes (from existing BranchItem)
    pub has_uncommitted: bool,
    /// Has unpushed commits (from existing BranchItem)
    pub has_unpushed: bool,
}

impl ChangeStats {
    /// Parse git diff --shortstat output
    ///
    /// # Example
    /// ```
    /// use gwt_core::git::ChangeStats;
    /// let stats = ChangeStats::from_shortstat(" 5 files changed, 120 insertions(+), 45 deletions(-)");
    /// assert_eq!(stats.files_changed, 5);
    /// assert_eq!(stats.insertions, 120);
    /// assert_eq!(stats.deletions, 45);
    /// ```
    pub fn from_shortstat(line: &str) -> Self {
        let mut stats = Self::default();
        let line = line.trim();

        // Parse "N file(s) changed"
        if let Some(files_match) = Self::extract_number(line, "file") {
            stats.files_changed = files_match;
        }

        // Parse "N insertion(s)(+)"
        if let Some(insertions_match) = Self::extract_number(line, "insertion") {
            stats.insertions = insertions_match;
        }

        // Parse "N deletion(s)(-)"
        if let Some(deletions_match) = Self::extract_number(line, "deletion") {
            stats.deletions = deletions_match;
        }

        stats
    }

    /// Extract a number before a keyword from the line
    fn extract_number(line: &str, keyword: &str) -> Option<usize> {
        let parts: Vec<&str> = line.split(',').collect();
        for part in parts {
            let part = part.trim();
            if part.contains(keyword) {
                // Find the number at the start of this part
                let words: Vec<&str> = part.split_whitespace().collect();
                if let Some(num_str) = words.first() {
                    if let Ok(num) = num_str.parse::<usize>() {
                        return Some(num);
                    }
                }
            }
        }
        None
    }

    /// Create a new ChangeStats with uncommitted/unpushed flags
    pub fn with_flags(mut self, has_uncommitted: bool, has_unpushed: bool) -> Self {
        self.has_uncommitted = has_uncommitted;
        self.has_unpushed = has_unpushed;
        self
    }

    /// Check if there are any changes
    pub fn has_changes(&self) -> bool {
        self.files_changed > 0 || self.has_uncommitted || self.has_unpushed
    }
}

/// Branch metadata derived from existing Branch struct
#[derive(Debug, Clone)]
pub struct BranchMeta {
    /// Upstream name (e.g., "origin/main")
    pub upstream: Option<String>,
    /// Commits ahead of upstream
    pub ahead: usize,
    /// Commits behind upstream
    pub behind: usize,
    /// Last commit timestamp (Unix timestamp)
    pub last_commit_timestamp: Option<i64>,
    /// Base branch (e.g., "main")
    pub base_branch: Option<String>,
}

impl BranchMeta {
    /// Create from an existing Branch struct
    pub fn from_branch(branch: &Branch) -> Self {
        Self {
            upstream: branch.upstream.clone(),
            ahead: branch.ahead,
            behind: branch.behind,
            last_commit_timestamp: branch.commit_timestamp,
            base_branch: None, // To be determined separately
        }
    }

    /// Calculate relative time from timestamp
    ///
    /// Returns a human-readable string like "2 days ago", "5 hours ago"
    pub fn relative_time(&self) -> Option<String> {
        let timestamp = self.last_commit_timestamp?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;

        let diff_secs = now - timestamp;
        if diff_secs < 0 {
            return Some("in the future".to_string());
        }

        let diff_secs = diff_secs as u64;

        // Calculate time units
        let minutes = diff_secs / 60;
        let hours = minutes / 60;
        let days = hours / 24;
        let weeks = days / 7;
        let months = days / 30;
        let years = days / 365;

        Some(if years > 0 {
            format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
        } else if months > 0 {
            format!("{} month{} ago", months, if months == 1 { "" } else { "s" })
        } else if weeks > 0 {
            format!("{} week{} ago", weeks, if weeks == 1 { "" } else { "s" })
        } else if days > 0 {
            format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
        } else if hours > 0 {
            format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
        } else if minutes > 0 {
            format!(
                "{} minute{} ago",
                minutes,
                if minutes == 1 { "" } else { "s" }
            )
        } else {
            "just now".to_string()
        })
    }

    /// Set the base branch
    pub fn with_base_branch(mut self, base: Option<String>) -> Self {
        self.base_branch = base;
        self
    }
}

/// Loading state for each section of the summary panel
#[derive(Debug, Clone, Default)]
pub struct LoadingState {
    /// Commit log is loading
    pub commits: bool,
    /// Change stats are loading
    pub stats: bool,
    /// Metadata is loading
    pub meta: bool,
    /// Session summary is loading
    pub session_summary: bool,
}

impl LoadingState {
    /// Check if any section is loading
    pub fn is_any_loading(&self) -> bool {
        self.commits || self.stats || self.meta || self.session_summary
    }

    /// Set commits loading state
    pub fn with_commits(mut self, loading: bool) -> Self {
        self.commits = loading;
        self
    }

    /// Set stats loading state
    pub fn with_stats(mut self, loading: bool) -> Self {
        self.stats = loading;
        self
    }

    /// Set meta loading state
    pub fn with_meta(mut self, loading: bool) -> Self {
        self.meta = loading;
        self
    }

    /// Set AI summary loading state
    pub fn with_session_summary(mut self, loading: bool) -> Self {
        self.session_summary = loading;
        self
    }
}

/// Complete branch summary data for the panel
#[derive(Debug, Clone, Default)]
pub struct BranchSummary {
    /// Branch name
    pub branch_name: String,
    /// Worktree path (if any)
    pub worktree_path: Option<PathBuf>,
    /// Recent commits (max 5)
    pub commits: Vec<CommitEntry>,
    /// Change statistics
    pub stats: Option<ChangeStats>,
    /// Branch metadata
    pub meta: Option<BranchMeta>,
    /// Session summary
    pub session_summary: Option<crate::ai::SessionSummary>,
    /// Loading state for each section
    pub loading: LoadingState,
    /// Error messages for failed sections
    pub errors: SectionErrors,
}

/// Error messages for each section
#[derive(Debug, Clone, Default)]
pub struct SectionErrors {
    pub commits: Option<String>,
    pub stats: Option<String>,
    pub meta: Option<String>,
    pub session_summary: Option<String>,
}

impl BranchSummary {
    /// Create a new empty summary for a branch
    pub fn new(branch_name: impl Into<String>) -> Self {
        Self {
            branch_name: branch_name.into(),
            ..Default::default()
        }
    }

    /// Set the worktree path
    pub fn with_worktree_path(mut self, path: Option<PathBuf>) -> Self {
        self.worktree_path = path;
        self
    }

    /// Set commits
    pub fn with_commits(mut self, commits: Vec<CommitEntry>) -> Self {
        self.commits = commits;
        self.loading.commits = false;
        self
    }

    /// Set change stats
    pub fn with_stats(mut self, stats: ChangeStats) -> Self {
        self.stats = Some(stats);
        self.loading.stats = false;
        self
    }

    /// Set metadata
    pub fn with_meta(mut self, meta: BranchMeta) -> Self {
        self.meta = Some(meta);
        self.loading.meta = false;
        self
    }

    /// Set AI summary
    pub fn with_session_summary(mut self, summary: crate::ai::SessionSummary) -> Self {
        self.session_summary = Some(summary);
        self.loading.session_summary = false;
        self
    }

    /// Check if branch has a worktree
    pub fn has_worktree(&self) -> bool {
        self.worktree_path.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commit_entry_from_oneline() {
        let entry = CommitEntry::from_oneline("a1b2c3d fix: update README").unwrap();
        assert_eq!(entry.hash, "a1b2c3d");
        assert_eq!(entry.message, "fix: update README");

        let entry = CommitEntry::from_oneline("abc1234 feat: add new feature with spaces").unwrap();
        assert_eq!(entry.hash, "abc1234");
        assert_eq!(entry.message, "feat: add new feature with spaces");

        assert!(CommitEntry::from_oneline("").is_none());
        assert!(CommitEntry::from_oneline("   ").is_none());
    }

    #[test]
    fn test_commit_entry_hash_only() {
        let entry = CommitEntry::from_oneline("a1b2c3d").unwrap();
        assert_eq!(entry.hash, "a1b2c3d");
        assert_eq!(entry.message, "");
    }

    #[test]
    fn test_change_stats_from_shortstat() {
        let stats =
            ChangeStats::from_shortstat(" 5 files changed, 120 insertions(+), 45 deletions(-)");
        assert_eq!(stats.files_changed, 5);
        assert_eq!(stats.insertions, 120);
        assert_eq!(stats.deletions, 45);

        let stats = ChangeStats::from_shortstat(" 1 file changed, 10 insertions(+)");
        assert_eq!(stats.files_changed, 1);
        assert_eq!(stats.insertions, 10);
        assert_eq!(stats.deletions, 0);

        let stats = ChangeStats::from_shortstat(" 3 files changed, 5 deletions(-)");
        assert_eq!(stats.files_changed, 3);
        assert_eq!(stats.insertions, 0);
        assert_eq!(stats.deletions, 5);
    }

    #[test]
    fn test_branch_meta_relative_time() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let meta = BranchMeta {
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_timestamp: Some(now - 60), // 1 minute ago
            base_branch: None,
        };
        assert_eq!(meta.relative_time(), Some("1 minute ago".to_string()));

        let meta = BranchMeta {
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_timestamp: Some(now - 3600), // 1 hour ago
            base_branch: None,
        };
        assert_eq!(meta.relative_time(), Some("1 hour ago".to_string()));

        let meta = BranchMeta {
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_timestamp: Some(now - 86400 * 2), // 2 days ago
            base_branch: None,
        };
        assert_eq!(meta.relative_time(), Some("2 days ago".to_string()));
    }

    #[test]
    fn test_loading_state() {
        let state = LoadingState::default();
        assert!(!state.is_any_loading());

        let state = LoadingState::default().with_commits(true);
        assert!(state.is_any_loading());
    }
}
