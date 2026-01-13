//! Branch operations

use crate::error::{GwtError, Result};
use std::path::Path;
use std::process::Command;

/// Represents a Git branch
#[derive(Debug, Clone)]
pub struct Branch {
    /// Branch name (e.g., "main", "feature/foo")
    pub name: String,
    /// Whether this is the current branch
    pub is_current: bool,
    /// Whether this branch has a remote tracking branch
    pub has_remote: bool,
    /// Remote tracking branch name (e.g., "origin/main")
    pub upstream: Option<String>,
    /// Commit SHA
    pub commit: String,
    /// Commits ahead of upstream
    pub ahead: usize,
    /// Commits behind upstream
    pub behind: usize,
    /// Last commit timestamp (Unix timestamp in seconds) - FR-041
    pub commit_timestamp: Option<i64>,
}

impl Branch {
    /// Create a new branch instance
    pub fn new(name: impl Into<String>, commit: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_current: false,
            has_remote: false,
            upstream: None,
            commit: commit.into(),
            ahead: 0,
            behind: 0,
            commit_timestamp: None,
        }
    }

    /// List all local branches in a repository
    pub fn list(repo_path: &Path) -> Result<Vec<Branch>> {
        let output = Command::new("git")
            .args([
                "for-each-ref",
                "--format=%(refname:short)%09%(objectname:short)%09%(upstream:short)%09%(HEAD)%09%(committerdate:unix)",
                "refs/heads/",
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "for-each-ref".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(GwtError::GitOperationFailed {
                operation: "for-each-ref".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut branches = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 5 {
                let name = parts[0].to_string();
                let commit = parts[1].to_string();
                let upstream = if parts[2].is_empty() {
                    None
                } else {
                    Some(parts[2].to_string())
                };
                let is_current = parts[3] == "*";
                let commit_timestamp = parts[4].parse::<i64>().ok();

                let mut branch = Branch {
                    name,
                    is_current,
                    has_remote: upstream.is_some(),
                    upstream: upstream.clone(),
                    commit,
                    ahead: 0,
                    behind: 0,
                    commit_timestamp,
                };

                // Get ahead/behind counts if upstream exists
                if let Some(ref up) = upstream {
                    if let Ok((ahead, behind)) = Self::get_divergence(repo_path, &branch.name, up) {
                        branch.ahead = ahead;
                        branch.behind = behind;
                    }
                }

                branches.push(branch);
            }
        }

        Ok(branches)
    }

    /// List all remote branches
    pub fn list_remote(repo_path: &Path) -> Result<Vec<Branch>> {
        let output = Command::new("git")
            .args([
                "for-each-ref",
                "--format=%(refname:short)%09%(objectname:short)%09%(committerdate:unix)",
                "refs/remotes/",
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "for-each-ref".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(GwtError::GitOperationFailed {
                operation: "for-each-ref".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut branches = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                // Skip HEAD refs
                if parts[0].ends_with("/HEAD") {
                    continue;
                }
                let commit_timestamp = parts[2].parse::<i64>().ok();
                branches.push(Branch {
                    name: parts[0].to_string(),
                    is_current: false,
                    has_remote: true,
                    upstream: None,
                    commit: parts[1].to_string(),
                    ahead: 0,
                    behind: 0,
                    commit_timestamp,
                });
            }
        }

        Ok(branches)
    }

    /// Get the current branch
    pub fn current(repo_path: &Path) -> Result<Option<Branch>> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Ok(None);
        }

        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name == "HEAD" {
            return Ok(None); // Detached HEAD
        }

        // Get commit
        let commit_output = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse".to_string(),
                details: e.to_string(),
            })?;

        let commit = String::from_utf8_lossy(&commit_output.stdout)
            .trim()
            .to_string();

        // Get commit timestamp (FR-041)
        let timestamp_output = Command::new("git")
            .args(["log", "-1", "--format=%ct", "HEAD"])
            .current_dir(repo_path)
            .output();

        let commit_timestamp = timestamp_output.ok().and_then(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<i64>()
                    .ok()
            } else {
                None
            }
        });

        // Get upstream
        let upstream_output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "@{u}"])
            .current_dir(repo_path)
            .output();

        let upstream = upstream_output.ok().and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

        let mut branch = Branch {
            name: name.clone(),
            is_current: true,
            has_remote: upstream.is_some(),
            upstream: upstream.clone(),
            commit,
            ahead: 0,
            behind: 0,
            commit_timestamp,
        };

        // Get ahead/behind
        if let Some(ref up) = upstream {
            if let Ok((ahead, behind)) = Self::get_divergence(repo_path, &name, up) {
                branch.ahead = ahead;
                branch.behind = behind;
            }
        }

        Ok(Some(branch))
    }

    /// Create a new branch from a base
    pub fn create(repo_path: &Path, name: &str, base: &str) -> Result<Branch> {
        let output = Command::new("git")
            .args(["branch", name, base])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "branch create".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(GwtError::BranchCreateFailed {
                name: name.to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        // Get commit of new branch
        let commit_output = Command::new("git")
            .args(["rev-parse", "--short", name])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse".to_string(),
                details: e.to_string(),
            })?;

        let commit = String::from_utf8_lossy(&commit_output.stdout)
            .trim()
            .to_string();

        Ok(Branch::new(name, commit))
    }

    /// Delete a branch
    pub fn delete(repo_path: &Path, name: &str, force: bool) -> Result<()> {
        let flag = if force { "-D" } else { "-d" };
        let output = Command::new("git")
            .args(["branch", flag, name])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "branch delete".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(GwtError::BranchDeleteFailed {
                name: name.to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    /// Get divergence (ahead, behind) between branch and upstream
    fn get_divergence(repo_path: &Path, branch: &str, upstream: &str) -> Result<(usize, usize)> {
        let output = Command::new("git")
            .args([
                "rev-list",
                "--left-right",
                "--count",
                &format!("{branch}...{upstream}"),
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-list".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Ok((0, 0));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split('\t').collect();
        if parts.len() == 2 {
            let ahead = parts[0].parse().unwrap_or(0);
            let behind = parts[1].parse().unwrap_or(0);
            Ok((ahead, behind))
        } else {
            Ok((0, 0))
        }
    }

    /// Get the divergence status from remote
    pub fn divergence_status(&self) -> DivergenceStatus {
        if !self.has_remote {
            return DivergenceStatus::NoRemote;
        }

        match (self.ahead, self.behind) {
            (0, 0) => DivergenceStatus::UpToDate,
            (a, 0) => DivergenceStatus::Ahead(a),
            (0, b) => DivergenceStatus::Behind(b),
            (a, b) => DivergenceStatus::Diverged {
                ahead: a,
                behind: b,
            },
        }
    }

    /// Check if a branch exists locally
    pub fn exists(repo_path: &Path, name: &str) -> Result<bool> {
        let output = Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{name}"),
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "show-ref".to_string(),
                details: e.to_string(),
            })?;

        Ok(output.status.success())
    }

    /// Check if a branch exists remotely
    pub fn remote_exists(repo_path: &Path, remote: &str, branch: &str) -> Result<bool> {
        let output = Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/{remote}/{branch}"),
            ])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "show-ref".to_string(),
                details: e.to_string(),
            })?;

        Ok(output.status.success())
    }

    /// Checkout this branch
    pub fn checkout(repo_path: &Path, name: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["checkout", name])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "checkout".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(GwtError::GitOperationFailed {
                operation: format!("checkout {name}"),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }
}

/// Branch divergence status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceStatus {
    /// Branch is up to date with remote
    UpToDate,
    /// Branch is ahead of remote
    Ahead(usize),
    /// Branch is behind remote
    Behind(usize),
    /// Branch has diverged from remote
    Diverged { ahead: usize, behind: usize },
    /// No remote tracking branch
    NoRemote,
}

impl std::fmt::Display for DivergenceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UpToDate => write!(f, "up to date"),
            Self::Ahead(n) => write!(f, "{n} ahead"),
            Self::Behind(n) => write!(f, "{n} behind"),
            Self::Diverged { ahead, behind } => write!(f, "{ahead} ahead, {behind} behind"),
            Self::NoRemote => write!(f, "no remote"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        // Create initial commit
        std::fs::write(temp.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        temp
    }

    #[test]
    fn test_list_branches() {
        let temp = create_test_repo();
        let branches = Branch::list(temp.path()).unwrap();
        assert_eq!(branches.len(), 1);
        assert!(branches[0].is_current);
    }

    #[test]
    fn test_current_branch() {
        let temp = create_test_repo();
        let current = Branch::current(temp.path()).unwrap();
        assert!(current.is_some());
        let branch = current.unwrap();
        assert!(branch.is_current);
        // Could be main or master depending on git version
        assert!(branch.name == "main" || branch.name == "master");
    }

    #[test]
    fn test_create_branch() {
        let temp = create_test_repo();
        let current = Branch::current(temp.path()).unwrap().unwrap();
        let branch = Branch::create(temp.path(), "feature/test", &current.name).unwrap();
        assert_eq!(branch.name, "feature/test");
        assert!(Branch::exists(temp.path(), "feature/test").unwrap());
    }

    #[test]
    fn test_delete_branch() {
        let temp = create_test_repo();
        let current = Branch::current(temp.path()).unwrap().unwrap();
        Branch::create(temp.path(), "feature/test", &current.name).unwrap();
        assert!(Branch::exists(temp.path(), "feature/test").unwrap());
        Branch::delete(temp.path(), "feature/test", false).unwrap();
        assert!(!Branch::exists(temp.path(), "feature/test").unwrap());
    }

    #[test]
    fn test_divergence_status() {
        let branch = Branch {
            name: "main".to_string(),
            is_current: true,
            has_remote: true,
            upstream: Some("origin/main".to_string()),
            commit: "abc123".to_string(),
            ahead: 2,
            behind: 1,
            commit_timestamp: None,
        };
        assert_eq!(
            branch.divergence_status(),
            DivergenceStatus::Diverged {
                ahead: 2,
                behind: 1
            }
        );
    }
}
