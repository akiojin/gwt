//! Worktree path generation

use std::path::{Path, PathBuf};

/// Worktree path utilities
pub struct WorktreePath;

impl WorktreePath {
    /// Generate a worktree path from branch name
    ///
    /// Format: {repo_root}/.worktrees/{sanitized_branch_name}
    pub fn generate(repo_root: &Path, branch_name: &str) -> PathBuf {
        let sanitized = Self::sanitize_branch_name(branch_name);
        repo_root.join(".worktrees").join(sanitized)
    }

    /// Sanitize branch name for use as directory name
    fn sanitize_branch_name(name: &str) -> String {
        name.replace(['/', '\\'], "-")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_path() {
        let root = PathBuf::from("/repo");
        let path = WorktreePath::generate(&root, "feature/foo");
        assert_eq!(path, PathBuf::from("/repo/.worktrees/feature-foo"));
    }

    #[test]
    fn test_sanitize_branch_name() {
        assert_eq!(
            WorktreePath::sanitize_branch_name("feature/foo/bar"),
            "feature-foo-bar"
        );
    }
}
