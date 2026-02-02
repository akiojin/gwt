//! Migration configuration (SPEC-a70a1ece T702)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Migration configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationConfig {
    /// Source repository root (the normal repo with .worktrees/)
    pub source_root: PathBuf,
    /// Target project root (where bare repo and worktrees will be placed)
    pub target_root: PathBuf,
    /// Name for the bare repository (e.g., "repo.git")
    pub bare_repo_name: String,
    /// Whether to perform a dry run (no actual changes)
    pub dry_run: bool,
    /// Maximum retry count for network operations
    pub max_retries: u32,
}

impl MigrationConfig {
    /// Create a new migration configuration
    pub fn new(source_root: PathBuf, target_root: PathBuf, bare_repo_name: String) -> Self {
        Self {
            source_root,
            target_root,
            bare_repo_name,
            dry_run: false,
            max_retries: 3,
        }
    }

    /// Get the path where the bare repository will be created
    pub fn bare_repo_path(&self) -> PathBuf {
        self.target_root.join(&self.bare_repo_name)
    }

    /// Get the path for a worktree branch
    pub fn worktree_path(&self, branch_name: &str) -> PathBuf {
        self.target_root.join(branch_name)
    }

    /// Get the backup directory path
    pub fn backup_path(&self) -> PathBuf {
        self.target_root.join(".gwt-migration-backup")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_config() {
        let config = MigrationConfig::new(
            PathBuf::from("/old/repo"),
            PathBuf::from("/project"),
            "repo.git".to_string(),
        );
        assert_eq!(config.source_root, PathBuf::from("/old/repo"));
        assert_eq!(config.target_root, PathBuf::from("/project"));
        assert_eq!(config.bare_repo_name, "repo.git");
        assert!(!config.dry_run);
        assert_eq!(config.max_retries, 3);
    }

    #[test]
    fn test_bare_repo_path() {
        let config = MigrationConfig::new(
            PathBuf::from("/old/repo"),
            PathBuf::from("/project"),
            "my-repo.git".to_string(),
        );
        assert_eq!(
            config.bare_repo_path(),
            PathBuf::from("/project/my-repo.git")
        );
    }

    #[test]
    fn test_worktree_path() {
        let config = MigrationConfig::new(
            PathBuf::from("/old/repo"),
            PathBuf::from("/project"),
            "repo.git".to_string(),
        );
        assert_eq!(
            config.worktree_path("feature/test"),
            PathBuf::from("/project/feature/test")
        );
    }
}
