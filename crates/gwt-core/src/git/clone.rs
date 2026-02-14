//! Git clone operations (SPEC-a70a1ece)
//!
//! Provides bare repository cloning functionality.

use crate::error::{GwtError, Result};
use std::path::Path;
use tracing::{debug, info};

/// Clone configuration (SPEC-a70a1ece T301)
#[derive(Debug, Clone)]
pub struct CloneConfig {
    /// Repository URL to clone
    pub url: String,
    /// Target directory for the clone
    pub target_dir: std::path::PathBuf,
    /// Clone as bare repository
    pub bare: bool,
    /// Shallow clone with depth
    pub depth: Option<u32>,
}

impl CloneConfig {
    /// Create a new clone configuration for a bare repository
    pub fn bare(url: impl Into<String>, target_dir: impl AsRef<Path>) -> Self {
        Self {
            url: url.into(),
            target_dir: target_dir.as_ref().to_path_buf(),
            bare: true,
            depth: None,
        }
    }

    /// Create a new clone configuration for a bare repository with shallow clone
    pub fn bare_shallow(url: impl Into<String>, target_dir: impl AsRef<Path>, depth: u32) -> Self {
        Self {
            url: url.into(),
            target_dir: target_dir.as_ref().to_path_buf(),
            bare: true,
            depth: Some(depth),
        }
    }
}

/// Extract repository name from URL (SPEC-a70a1ece)
///
/// Examples:
/// - `https://github.com/user/repo.git` -> `repo.git`
/// - `git@github.com:user/repo.git` -> `repo.git`
/// - `https://github.com/user/repo` -> `repo.git`
pub fn extract_repo_name(url: &str) -> String {
    let url = url.trim_end_matches('/');

    // Extract the last path segment
    let name = url
        .rsplit('/')
        .next()
        .or_else(|| url.rsplit(':').next())
        .unwrap_or("repo");

    // Add .git suffix if not present
    if name.ends_with(".git") {
        name.to_string()
    } else {
        format!("{}.git", name)
    }
}

/// Clone a repository as bare (SPEC-a70a1ece T302)
///
/// Clones a repository in bare format, suitable for worktree-based workflow.
///
/// # Arguments
///
/// * `config` - Clone configuration
///
/// # Returns
///
/// Path to the cloned bare repository
pub fn clone_bare(config: &CloneConfig) -> Result<std::path::PathBuf> {
    let repo_name = extract_repo_name(&config.url);
    let bare_path = config.target_dir.join(&repo_name);

    if bare_path.exists() {
        return Err(GwtError::GitOperationFailed {
            operation: "clone".to_string(),
            details: format!("Target directory already exists: {}", bare_path.display()),
        });
    }

    info!(
        url = %config.url,
        target = %bare_path.display(),
        bare = config.bare,
        depth = ?config.depth,
        "Cloning repository"
    );

    let mut args = vec!["clone"];

    if config.bare {
        args.push("--bare");
    }

    if let Some(depth) = config.depth {
        args.push("--depth");
        args.push(Box::leak(depth.to_string().into_boxed_str()));
    }

    args.push(&config.url);
    args.push(Box::leak(
        bare_path.to_string_lossy().into_owned().into_boxed_str(),
    ));

    debug!(args = ?args, "Running git clone");

    let output = crate::process::command("git")
        .args(&args)
        .current_dir(&config.target_dir)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "clone".to_string(),
            details: format!("Failed to execute git clone: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GwtError::GitOperationFailed {
            operation: "clone".to_string(),
            details: format!("git clone failed: {}", stderr),
        });
    }

    info!(path = %bare_path.display(), "Repository cloned successfully");

    // SPEC-a70a1ece: For shallow clones, fetch all branch references
    // Shallow clone only downloads the default branch. We need to configure
    // the remote to track all branches and fetch their references.
    if config.depth.is_some() {
        debug!(
            path = %bare_path.display(),
            "Fetching all branch references for shallow clone"
        );

        // Configure remote to track all branches
        let config_output = crate::process::command("git")
            .args([
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/*",
            ])
            .current_dir(&bare_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "config".to_string(),
                details: format!("Failed to configure remote fetch: {}", e),
            })?;

        if !config_output.status.success() {
            debug!(
                stderr = %String::from_utf8_lossy(&config_output.stderr),
                "Failed to configure remote fetch, continuing anyway"
            );
        }

        // Fetch all branch references with shallow depth
        let fetch_output = crate::process::command("git")
            .args(["fetch", "--depth=1", "origin"])
            .current_dir(&bare_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "fetch".to_string(),
                details: format!("Failed to fetch branch references: {}", e),
            })?;

        if !fetch_output.status.success() {
            debug!(
                stderr = %String::from_utf8_lossy(&fetch_output.stderr),
                "Failed to fetch branch references, continuing anyway"
            );
        } else {
            info!(
                path = %bare_path.display(),
                "Fetched all branch references successfully"
            );
        }
    }

    Ok(bare_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_repo_name_https_with_git() {
        assert_eq!(
            extract_repo_name("https://github.com/user/repo.git"),
            "repo.git"
        );
    }

    #[test]
    fn test_extract_repo_name_https_without_git() {
        assert_eq!(
            extract_repo_name("https://github.com/user/repo"),
            "repo.git"
        );
    }

    #[test]
    fn test_extract_repo_name_ssh() {
        assert_eq!(
            extract_repo_name("git@github.com:user/repo.git"),
            "repo.git"
        );
    }

    #[test]
    fn test_extract_repo_name_trailing_slash() {
        assert_eq!(
            extract_repo_name("https://github.com/user/repo/"),
            "repo.git"
        );
    }

    #[test]
    fn test_clone_config_bare() {
        let config = CloneConfig::bare("https://github.com/user/repo.git", "/tmp/test");
        assert!(config.bare);
        assert!(config.depth.is_none());
    }

    #[test]
    fn test_clone_config_bare_shallow() {
        let config = CloneConfig::bare_shallow("https://github.com/user/repo.git", "/tmp/test", 1);
        assert!(config.bare);
        assert_eq!(config.depth, Some(1));
    }
}
