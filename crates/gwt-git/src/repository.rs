//! Git repository discovery and inspection

use std::path::{Path, PathBuf};

use gwt_core::{GwtError, Result};

/// A thin wrapper around a Git repository path for discovery and inspection.
pub struct Repository {
    path: PathBuf,
}

impl Repository {
    /// Open a repository at the given path.
    ///
    /// The path must be a valid Git repository (contains `.git` directory
    /// or is a bare repository).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let git_dir = path.join(".git");
        let is_bare = path.join("HEAD").exists() && path.join("refs").exists();

        if !git_dir.exists() && !is_bare {
            return Err(GwtError::Git(format!("Not a git repository: {}", path.display())));
        }

        Ok(Self { path })
    }

    /// Discover a repository by walking up from the given path.
    pub fn discover(start: impl AsRef<Path>) -> Result<Self> {
        let start = start.as_ref();
        let mut current = start.to_path_buf();

        loop {
            if current.join(".git").exists() {
                return Ok(Self {
                    path: current,
                });
            }
            // Check bare repository
            if current.join("HEAD").exists() && current.join("refs").exists() {
                return Ok(Self {
                    path: current,
                });
            }
            if !current.pop() {
                break;
            }
        }

        Err(GwtError::Git(format!(
            "Not a git repository (or any parent): {}",
            start.display()
        )))
    }

    /// Return the repository root path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the current branch name (HEAD symbolic ref).
    ///
    /// Returns `None` for detached HEAD.
    pub fn current_branch(&self) -> Result<Option<String>> {
        let output = std::process::Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(&self.path)
            .output()
            .map_err(|e| GwtError::Git(format!("symbolic-ref: {e}")))?;

        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(Some(name))
        } else {
            // Detached HEAD
            Ok(None)
        }
    }

    /// List local and remote branch names.
    pub fn branches(&self) -> Result<Vec<String>> {
        let output = std::process::Command::new("git")
            .args(["branch", "-a", "--format=%(refname:short)"])
            .current_dir(&self.path)
            .output()
            .map_err(|e| GwtError::Git(format!("branch: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(format!("branch: {stderr}")));
        }

        let branches = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        Ok(branches)
    }

    /// Check if this repository is bare.
    pub fn is_bare(&self) -> bool {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--is-bare-repository"])
            .current_dir(&self.path)
            .output();

        match output {
            Ok(o) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout).trim() == "true"
            }
            _ => false,
        }
    }

    /// Check if the current directory is inside a worktree.
    pub fn is_worktree(&self) -> bool {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&self.path)
            .output();

        match output {
            Ok(o) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout).trim() == "true"
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_non_git_dir_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let result = Repository::open(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn open_valid_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let repo = Repository::open(tmp.path()).unwrap();
        assert_eq!(repo.path(), tmp.path());
    }

    #[test]
    fn discover_walks_up_to_repo() {
        let tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let subdir = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&subdir).unwrap();

        let repo = Repository::discover(&subdir).unwrap();
        assert_eq!(repo.path(), tmp.path());
    }

    #[test]
    fn discover_fails_for_non_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let result = Repository::discover(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn current_branch_returns_name() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        // Create an initial commit so HEAD exists
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(path)
            .output()
            .unwrap();

        let repo = Repository::open(path).unwrap();
        let branch = repo.current_branch().unwrap();
        assert!(branch.is_some());
    }

    #[test]
    fn branches_lists_at_least_one() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(path)
            .output()
            .unwrap();

        let repo = Repository::open(path).unwrap();
        let branches = repo.branches().unwrap();
        assert!(!branches.is_empty());
    }

    #[test]
    fn is_bare_false_for_normal_repo() {
        let tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let repo = Repository::open(tmp.path()).unwrap();
        assert!(!repo.is_bare());
    }

    #[test]
    fn is_worktree_true_for_normal_repo() {
        let tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let repo = Repository::open(tmp.path()).unwrap();
        assert!(repo.is_worktree());
    }
}
