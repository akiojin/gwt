//! Remote operations

use crate::error::{GwtError, Result};
use std::path::Path;
use std::process::Command;
use tracing::{debug, error, info};

/// Represents a Git remote
#[derive(Debug, Clone)]
pub struct Remote {
    /// Remote name (e.g., "origin")
    pub name: String,
    /// Fetch URL
    pub fetch_url: String,
    /// Push URL (may differ from fetch URL)
    pub push_url: String,
}

impl Remote {
    /// Create a new remote instance
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        let url = url.into();
        Self {
            name: name.into(),
            fetch_url: url.clone(),
            push_url: url,
        }
    }

    /// List all remotes in a repository
    pub fn list(repo_path: &Path) -> Result<Vec<Remote>> {
        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            "Listing remotes"
        );

        let output = Command::new("git")
            .args(["remote", "-v"])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "remote -v".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(GwtError::GitOperationFailed {
                operation: "remote -v".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut remotes: Vec<Remote> = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let name = parts[0];
                let url = parts[1];
                let kind = parts[2]; // (fetch) or (push)

                // Find or create remote
                let remote = remotes.iter_mut().find(|r| r.name == name);
                match remote {
                    Some(r) => {
                        if kind == "(fetch)" {
                            r.fetch_url = url.to_string();
                        } else if kind == "(push)" {
                            r.push_url = url.to_string();
                        }
                    }
                    None => {
                        let mut new_remote = Remote::new(name, url);
                        if kind == "(push)" {
                            new_remote.push_url = url.to_string();
                        }
                        remotes.push(new_remote);
                    }
                }
            }
        }

        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            remote_count = remotes.len(),
            "Remotes listed"
        );

        Ok(remotes)
    }

    /// Get a specific remote by name
    pub fn get(repo_path: &Path, name: &str) -> Result<Option<Remote>> {
        let remotes = Self::list(repo_path)?;
        Ok(remotes.into_iter().find(|r| r.name == name))
    }

    /// Fetch this remote
    pub fn fetch(repo_path: &Path, name: &str, prune: bool) -> Result<()> {
        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            remote = name,
            prune,
            "Fetching remote"
        );

        let mut args = vec!["fetch", name];
        if prune {
            args.push("--prune");
        }

        let output = Command::new("git")
            .args(&args)
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: format!("fetch {name}"),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "fetch",
                remote = name,
                prune,
                "Remote fetched successfully"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                remote = name,
                error = err_msg.as_str(),
                "Failed to fetch remote"
            );
            Err(GwtError::GitOperationFailed {
                operation: format!("fetch {name}"),
                details: err_msg,
            })
        }
    }

    /// Fetch all remotes
    pub fn fetch_all(repo_path: &Path, prune: bool) -> Result<()> {
        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            prune,
            "Fetching all remotes"
        );

        let mut args = vec!["fetch", "--all"];
        if prune {
            args.push("--prune");
        }

        let output = Command::new("git")
            .args(&args)
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "fetch --all".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "fetch_all",
                prune,
                "All remotes fetched successfully"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                error = err_msg.as_str(),
                "Failed to fetch all remotes"
            );
            Err(GwtError::GitOperationFailed {
                operation: "fetch --all".to_string(),
                details: err_msg,
            })
        }
    }

    /// Push a branch to a remote
    pub fn push(repo_path: &Path, name: &str, branch: &str, set_upstream: bool) -> Result<()> {
        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            remote = name,
            branch,
            set_upstream,
            "Pushing to remote"
        );

        let mut args = vec!["push"];
        if set_upstream {
            args.push("-u");
        }
        args.push(name);
        args.push(branch);

        let output = Command::new("git")
            .args(&args)
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: format!("push {name}"),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "push",
                remote = name,
                branch,
                set_upstream,
                "Pushed to remote successfully"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                remote = name,
                branch,
                error = err_msg.as_str(),
                "Failed to push to remote"
            );
            Err(GwtError::GitOperationFailed {
                operation: format!("push {name}"),
                details: err_msg,
            })
        }
    }

    /// Check if a remote exists
    pub fn exists(repo_path: &Path, name: &str) -> Result<bool> {
        let remotes = Self::list(repo_path)?;
        Ok(remotes.iter().any(|r| r.name == name))
    }

    /// Add a new remote
    pub fn add(repo_path: &Path, name: &str, url: &str) -> Result<Remote> {
        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            remote = name,
            url,
            "Adding remote"
        );

        let output = Command::new("git")
            .args(["remote", "add", name, url])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "remote add".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "remote_add",
                remote = name,
                url,
                "Remote added successfully"
            );
            Ok(Remote::new(name, url))
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                remote = name,
                url,
                error = err_msg.as_str(),
                "Failed to add remote"
            );
            Err(GwtError::GitOperationFailed {
                operation: format!("remote add {name}"),
                details: err_msg,
            })
        }
    }

    /// Remove a remote
    pub fn remove(repo_path: &Path, name: &str) -> Result<()> {
        debug!(
            category = "git",
            repo_path = %repo_path.display(),
            remote = name,
            "Removing remote"
        );

        let output = Command::new("git")
            .args(["remote", "remove", name])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "remote remove".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "remote_remove",
                remote = name,
                "Remote removed successfully"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                remote = name,
                error = err_msg.as_str(),
                "Failed to remove remote"
            );
            Err(GwtError::GitOperationFailed {
                operation: format!("remote remove {name}"),
                details: err_msg,
            })
        }
    }

    /// Update remote tracking references
    pub fn update_refs(repo_path: &Path) -> Result<()> {
        let output = Command::new("git")
            .args(["remote", "update", "--prune"])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "remote update".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(GwtError::GitOperationFailed {
                operation: "remote update".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    /// Check network connectivity to remote
    pub fn is_reachable(repo_path: &Path, name: &str) -> bool {
        let output = Command::new("git")
            .args(["ls-remote", "--exit-code", name])
            .current_dir(repo_path)
            .output();

        match output {
            Ok(o) => o.status.success(),
            Err(_) => false,
        }
    }

    /// Get default remote (usually "origin")
    pub fn default(repo_path: &Path) -> Result<Option<Remote>> {
        // Try origin first
        if let Some(remote) = Self::get(repo_path, "origin")? {
            return Ok(Some(remote));
        }

        // Fall back to first remote
        let remotes = Self::list(repo_path)?;
        Ok(remotes.into_iter().next())
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
        temp
    }

    #[test]
    fn test_list_empty() {
        let temp = create_test_repo();
        let remotes = Remote::list(temp.path()).unwrap();
        assert!(remotes.is_empty());
    }

    #[test]
    fn test_add_remote() {
        let temp = create_test_repo();
        let remote =
            Remote::add(temp.path(), "origin", "https://github.com/test/test.git").unwrap();
        assert_eq!(remote.name, "origin");
        assert!(Remote::exists(temp.path(), "origin").unwrap());
    }

    #[test]
    fn test_remove_remote() {
        let temp = create_test_repo();
        Remote::add(temp.path(), "origin", "https://github.com/test/test.git").unwrap();
        assert!(Remote::exists(temp.path(), "origin").unwrap());
        Remote::remove(temp.path(), "origin").unwrap();
        assert!(!Remote::exists(temp.path(), "origin").unwrap());
    }

    #[test]
    fn test_default_remote() {
        let temp = create_test_repo();
        Remote::add(temp.path(), "origin", "https://github.com/test/test.git").unwrap();
        let default = Remote::default(temp.path()).unwrap();
        assert!(default.is_some());
        assert_eq!(default.unwrap().name, "origin");
    }

    #[test]
    fn test_push_to_local_remote() {
        let temp = create_test_repo();

        std::fs::write(temp.path().join("README.md"), "# Test").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        let remote_dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init", "--bare"])
            .current_dir(remote_dir.path())
            .output()
            .unwrap();

        Remote::add(
            temp.path(),
            "origin",
            remote_dir.path().to_string_lossy().as_ref(),
        )
        .unwrap();

        let branch = crate::git::Branch::current(temp.path())
            .unwrap()
            .map(|b| b.name)
            .unwrap_or_else(|| "main".to_string());

        Remote::push(temp.path(), "origin", &branch, true).unwrap();

        let output = Command::new("git")
            .args([
                "--git-dir",
                remote_dir.path().to_str().unwrap(),
                "rev-parse",
                &branch,
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
    }
}
