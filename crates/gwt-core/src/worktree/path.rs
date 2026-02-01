//! Worktree path generation

use super::WorktreeLocation;
use std::path::{Path, PathBuf};

/// Worktree path utilities
pub struct WorktreePath;

impl WorktreePath {
    /// Generate a worktree path from branch name (default: Subdir method)
    ///
    /// Format: {repo_root}/.worktrees/{sanitized_branch_name}
    pub fn generate(repo_root: &Path, branch_name: &str) -> PathBuf {
        Self::generate_with_location(repo_root, branch_name, WorktreeLocation::Subdir)
    }

    /// Generate a worktree path with specified location strategy (SPEC-a70a1ece T401-T403)
    ///
    /// - Subdir: {repo_root}/.worktrees/{sanitized_branch_name}
    /// - Sibling: {repo_root_parent}/{branch_name_with_subdirs}
    pub fn generate_with_location(
        repo_root: &Path,
        branch_name: &str,
        location: WorktreeLocation,
    ) -> PathBuf {
        match location {
            WorktreeLocation::Subdir => {
                let sanitized = Self::sanitize_branch_name(branch_name);
                repo_root.join(".worktrees").join(sanitized)
            }
            WorktreeLocation::Sibling => {
                // For sibling method, create worktree next to the bare repo
                // Branch name is preserved as-is (slash becomes subdirectory)
                if let Some(parent) = repo_root.parent() {
                    parent.join(branch_name)
                } else {
                    // Fallback to subdir if no parent
                    let sanitized = Self::sanitize_branch_name(branch_name);
                    repo_root.join(".worktrees").join(sanitized)
                }
            }
        }
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
    fn test_generate_path_subdir() {
        let root = PathBuf::from("/project/repo.git");
        let path = WorktreePath::generate_with_location(&root, "main", WorktreeLocation::Subdir);
        assert_eq!(path, PathBuf::from("/project/repo.git/.worktrees/main"));
    }

    #[test]
    fn test_generate_path_sibling() {
        let root = PathBuf::from("/project/repo.git");
        let path = WorktreePath::generate_with_location(&root, "main", WorktreeLocation::Sibling);
        assert_eq!(path, PathBuf::from("/project/main"));
    }

    #[test]
    fn test_generate_path_sibling_with_slash() {
        // SPEC-a70a1ece T403: slash becomes subdirectory in sibling mode
        let root = PathBuf::from("/project/repo.git");
        let path = WorktreePath::generate_with_location(
            &root,
            "feature/branch-name",
            WorktreeLocation::Sibling,
        );
        assert_eq!(path, PathBuf::from("/project/feature/branch-name"));
    }

    #[test]
    fn test_sanitize_branch_name() {
        assert_eq!(
            WorktreePath::sanitize_branch_name("feature/foo/bar"),
            "feature-foo-bar"
        );
    }
}
