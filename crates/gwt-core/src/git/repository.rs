//! Repository operations

use crate::error::{GwtError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, error, info, warn};

/// Repository type classification (SPEC-a70a1ece)
///
/// Represents the type of directory where gwt is launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoType {
    /// Normal git repository (.git/ is a directory)
    Normal,
    /// Bare repository (is-bare-repository = true)
    Bare,
    /// Inside a worktree (normal or bare-based)
    Worktree,
    /// Empty directory (no files including hidden)
    Empty,
    /// Not a git repository (has files but not git)
    NonRepo,
}

/// Header display context (SPEC-a70a1ece)
///
/// Contains information needed for header display.
#[derive(Debug, Clone)]
pub struct HeaderContext {
    /// Working directory path
    pub working_dir: PathBuf,
    /// Current branch name (None for bare repos)
    pub branch_name: Option<String>,
    /// Repository type
    pub repo_type: RepoType,
    /// Bare repository name (for worktrees in bare-based projects)
    pub bare_name: Option<String>,
}

impl HeaderContext {
    /// Format the header display string
    ///
    /// Returns formatted string based on repo type:
    /// - Normal/Worktree: `/path [branch]`
    /// - Bare: `/path [bare]`
    /// - Bare worktree: `/path [branch] (repo.git)`
    pub fn format_display(&self) -> String {
        let path = self.working_dir.display();
        match self.repo_type {
            RepoType::Bare => format!("{} [bare]", path),
            RepoType::Worktree if self.bare_name.is_some() => {
                format!(
                    "{} [{}] ({})",
                    path,
                    self.branch_name.as_deref().unwrap_or(""),
                    self.bare_name.as_deref().unwrap()
                )
            }
            _ => format!("{} [{}]", path, self.branch_name.as_deref().unwrap_or("")),
        }
    }
}

/// Check if a directory is empty (SPEC-a70a1ece)
pub fn is_empty_dir(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => false,
    }
}

/// Check if a path is inside a git repository (SPEC-a70a1ece)
pub fn is_git_repo(path: &Path) -> bool {
    let output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(path)
        .output();

    matches!(output, Ok(o) if o.status.success())
}

/// Check if a repository is bare (SPEC-a70a1ece)
pub fn is_bare_repository(path: &Path) -> bool {
    let output = Command::new("git")
        .args(["rev-parse", "--is-bare-repository"])
        .current_dir(path)
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim() == "true",
        _ => false,
    }
}

/// Check if inside a worktree (not the main repo) (SPEC-a70a1ece)
pub fn is_inside_worktree(path: &Path) -> bool {
    // A worktree has a .git file (not directory) pointing to the main repo
    let git_path = path.join(".git");
    git_path.is_file()
}

/// Find a bare repository (*.git directory) in the given directory (SPEC-a70a1ece)
///
/// Returns the path to the first bare repository found, if any.
/// This is used to detect bare repos when gwt is started from the parent directory.
pub fn find_bare_repo_in_dir(path: &Path) -> Option<std::path::PathBuf> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return None,
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            // Check if it's a *.git directory and is a bare repository
            if let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".git") && is_bare_repository(&entry_path) {
                    return Some(entry_path);
                }
            }
        }
    }

    None
}

/// Detect the repository type at a given path (SPEC-a70a1ece)
pub fn detect_repo_type(path: &Path) -> RepoType {
    // 1. Check if directory is empty
    if is_empty_dir(path) {
        return RepoType::Empty;
    }

    // 2. Check if it's a git repository
    if !is_git_repo(path) {
        return RepoType::NonRepo;
    }

    // 3. Check if it's a bare repository
    if is_bare_repository(path) {
        return RepoType::Bare;
    }

    // 4. Check if inside a worktree
    if is_inside_worktree(path) {
        return RepoType::Worktree;
    }

    RepoType::Normal
}

/// Get header context for display (SPEC-a70a1ece)
pub fn get_header_context(path: &Path) -> HeaderContext {
    let repo_type = detect_repo_type(path);
    let branch_name = if repo_type != RepoType::Bare {
        get_current_branch(path)
    } else {
        None
    };

    let bare_name = if repo_type == RepoType::Worktree {
        detect_bare_parent_name(path)
    } else {
        None
    };

    HeaderContext {
        working_dir: path.to_path_buf(),
        branch_name,
        repo_type,
        bare_name,
    }
}

/// Get the current branch name
fn get_current_branch(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;

    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name == "HEAD" {
            None // Detached HEAD
        } else {
            Some(name)
        }
    } else {
        None
    }
}

/// Detect if worktree's parent is a bare repository and get its name
fn detect_bare_parent_name(path: &Path) -> Option<String> {
    // Read .git file to get the gitdir path
    let git_file = path.join(".git");
    if !git_file.is_file() {
        return None;
    }

    let content = std::fs::read_to_string(&git_file).ok()?;
    // Format: "gitdir: /path/to/repo.git/worktrees/branch-name"
    let gitdir = content.strip_prefix("gitdir: ")?.trim();

    // Extract the bare repo path (remove /worktrees/xxx suffix)
    let bare_path = PathBuf::from(gitdir);
    let parent = bare_path.parent()?.parent()?; // Go up from worktrees/branch

    // Check if it's actually a bare repo
    if is_bare_repository(parent) {
        parent.file_name()?.to_str().map(String::from)
    } else {
        None
    }
}

/// Represents a Git repository
#[derive(Debug)]
pub struct Repository {
    /// Repository root path
    root: PathBuf,
    /// Internal gix repository handle (lazy loaded)
    gix_repo: Option<gix::Repository>,
}

impl Repository {
    /// Discover a repository from a path
    ///
    /// This method first tries gix::discover(), and falls back to external git commands
    /// if that fails. This provides better compatibility with environments where
    /// gix may have issues (e.g., WSL).
    ///
    /// Issue #774: Added fallback to external git commands for WSL compatibility.
    pub fn discover(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        // Try using gix first
        match gix::discover(path) {
            Ok(repo) => {
                let root = repo
                    .workdir()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| repo.git_dir().to_path_buf());
                Ok(Self {
                    root,
                    gix_repo: Some(repo),
                })
            }
            Err(gix_err) => {
                // Fallback: Use external git command for environments where gix fails
                // (Issue #774: WSL compatibility)
                tracing::debug!(
                    "Repository::discover: gix::discover failed for {:?}, trying external git fallback: {}",
                    path,
                    gix_err
                );
                Self::discover_with_git_command(path)
            }
        }
    }

    /// Discover a repository using external git command (fallback for gix failures)
    fn discover_with_git_command(path: &Path) -> Result<Self> {
        // Use git rev-parse to find the repository root
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(path)
            .output();

        match output {
            Ok(o) if o.status.success() => {
                let root = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let root = PathBuf::from(root);

                tracing::debug!(
                    "Repository::discover_with_git_command: input_path={:?}, resolved_root={:?}",
                    path,
                    root
                );

                Ok(Self {
                    root,
                    gix_repo: None,
                })
            }
            _ => {
                // Final fallback: Manual .git file/directory search
                // This handles cases where git command itself fails
                Self::discover_manual(path)
            }
        }
    }

    /// Manual discovery by searching for .git file or directory
    fn discover_manual(path: &Path) -> Result<Self> {
        let mut current = path.to_path_buf();
        loop {
            let git_path = current.join(".git");
            if git_path.exists() {
                // .git can be a directory (normal repo) or a file (worktree)
                // Both are valid Git repository markers
                return Ok(Self {
                    root: current,
                    gix_repo: None,
                });
            }
            if !current.pop() {
                break;
            }
        }

        Err(GwtError::RepositoryNotFound {
            path: path.to_path_buf(),
        })
    }

    /// Open a repository at the given path
    ///
    /// This method first tries gix::open(), and falls back to external git commands
    /// if that fails. This provides better compatibility with environments where
    /// gix may have issues (e.g., WSL).
    ///
    /// Issue #774: Added fallback to external git commands for WSL compatibility.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        match gix::open(path) {
            Ok(repo) => {
                let work_dir = repo.workdir().map(|p| p.to_path_buf());
                let git_dir = repo.git_dir().to_path_buf();
                let root = work_dir.clone().unwrap_or_else(|| git_dir.clone());

                tracing::debug!(
                    "Repository::open: input_path={:?}, work_dir={:?}, git_dir={:?}, resolved_root={:?}",
                    path, work_dir, git_dir, root
                );

                Ok(Self {
                    root,
                    gix_repo: Some(repo),
                })
            }
            Err(gix_err) => {
                // Fallback: Use external git command for environments where gix fails
                // (Issue #774: WSL compatibility)
                tracing::debug!(
                    "Repository::open: gix::open failed for {:?}, trying external git fallback: {}",
                    path,
                    gix_err
                );
                Self::open_with_git_command(path)
            }
        }
    }

    /// Open a repository using external git command (fallback for gix failures)
    fn open_with_git_command(path: &Path) -> Result<Self> {
        // Verify this is a valid git repository by running git rev-parse
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse --show-toplevel".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let root = PathBuf::from(root);

            tracing::debug!(
                "Repository::open_with_git_command: input_path={:?}, resolved_root={:?}",
                path,
                root
            );

            Ok(Self {
                root,
                gix_repo: None,
            })
        } else {
            Err(GwtError::RepositoryNotFound {
                path: path.to_path_buf(),
            })
        }
    }

    /// Get the repository root path
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the main repository root (resolves through worktree to main repo)
    /// For worktrees, this returns the path to the main repository.
    /// For normal repos, this returns the same as root().
    pub fn main_repo_root(&self) -> PathBuf {
        // Use git rev-parse --git-common-dir to get the common git directory
        let output = Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(&self.root)
            .output();

        match output {
            Ok(o) if o.status.success() => {
                let common_dir = String::from_utf8_lossy(&o.stdout).trim().to_string();
                // common_dir is like "/gwt/.git" - parent is the repo root
                let common_path = PathBuf::from(&common_dir);
                if common_path.is_absolute() {
                    common_path
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| self.root.clone())
                } else {
                    // Relative path - resolve from current root
                    self.root
                        .join(&common_path)
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| self.root.clone())
                }
            }
            _ => self.root.clone(),
        }
    }

    /// Get internal gix repository reference
    fn gix_repo(&self) -> Option<&gix::Repository> {
        self.gix_repo.as_ref()
    }

    /// Check if there are uncommitted changes (staged or unstaged)
    pub fn has_uncommitted_changes(&self) -> Result<bool> {
        // Use external git for reliability with worktrees
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "status".to_string(),
                details: e.to_string(),
            })?;

        let has_changes = !output.stdout.is_empty();

        tracing::debug!(
            "has_uncommitted_changes: path={:?}, has_changes={}, output={:?}",
            self.root,
            has_changes,
            String::from_utf8_lossy(&output.stdout)
        );

        Ok(has_changes)
    }

    /// Check if there are unpushed commits
    pub fn has_unpushed_commits(&self) -> Result<bool> {
        let output = Command::new("git")
            .args(["log", "@{u}..", "--oneline"])
            .current_dir(&self.root)
            .output();

        match output {
            Ok(o) => Ok(!o.stdout.is_empty()),
            Err(_) => Ok(false), // No upstream configured
        }
    }

    /// Get the current HEAD reference name
    pub fn head_name(&self) -> Result<Option<String>> {
        if let Some(repo) = self.gix_repo() {
            match repo.head_name() {
                Ok(Some(name)) => Ok(Some(name.shorten().to_string())),
                Ok(None) => Ok(None), // Detached HEAD
                Err(_) => self.head_name_external(),
            }
        } else {
            self.head_name_external()
        }
    }

    /// Get HEAD name using external git command
    fn head_name_external(&self) -> Result<Option<String>> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if name == "HEAD" {
                Ok(None) // Detached HEAD
            } else {
                Ok(Some(name))
            }
        } else {
            Ok(None)
        }
    }

    /// Get the current HEAD commit SHA
    pub fn head_commit(&self) -> Result<String> {
        if let Some(repo) = self.gix_repo() {
            match repo.head_id() {
                Ok(id) => Ok(id.to_hex().to_string()),
                Err(_) => self.head_commit_external(),
            }
        } else {
            self.head_commit_external()
        }
    }

    /// Get HEAD commit using external git command
    fn head_commit_external(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(GwtError::GitOperationFailed {
                operation: "rev-parse HEAD".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    /// Get recent commit log entries (SPEC-4b893dae FR-010~FR-013)
    ///
    /// Returns a list of recent commits in oneline format (hash + message).
    /// Limit specifies the maximum number of commits to return (default: 5).
    pub fn get_commit_log(&self, limit: usize) -> Result<Vec<super::CommitEntry>> {
        let limit_arg = format!("-{}", limit.clamp(1, 20));

        let output = Command::new("git")
            .args(["log", "--oneline", &limit_arg])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "log".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            // Repository might have no commits yet
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("does not have any commits yet") {
                return Ok(Vec::new());
            }
            return Err(GwtError::GitOperationFailed {
                operation: "log --oneline".to_string(),
                details: stderr.to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let commits: Vec<super::CommitEntry> = stdout
            .lines()
            .filter_map(super::CommitEntry::from_oneline)
            .collect();

        tracing::debug!(
            "get_commit_log: path={:?}, limit={}, found={} commits",
            self.root,
            limit,
            commits.len()
        );

        Ok(commits)
    }

    /// Get diff statistics (SPEC-4b893dae FR-020~FR-024)
    ///
    /// Returns change statistics for the working directory compared to HEAD.
    pub fn get_diff_stats(&self) -> Result<super::ChangeStats> {
        let output = Command::new("git")
            .args(["diff", "--shortstat"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "diff".to_string(),
                details: e.to_string(),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stats = super::ChangeStats::from_shortstat(&stdout);

        tracing::debug!(
            "get_diff_stats: path={:?}, files={}, +{}/-{}",
            self.root,
            stats.files_changed,
            stats.insertions,
            stats.deletions
        );

        Ok(stats)
    }

    /// Pull with fast-forward only
    pub fn pull_fast_forward(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["pull", "--ff-only"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "pull".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(GwtError::GitOperationFailed {
                operation: "pull --ff-only".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    /// Fetch all remotes
    pub fn fetch_all(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["fetch", "--all", "--prune"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "fetch".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(GwtError::GitOperationFailed {
                operation: "fetch --all".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    /// List all worktrees using git worktree list
    pub fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree list".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(GwtError::GitOperationFailed {
                operation: "worktree list".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut worktrees = Vec::new();
        let mut current: Option<WorktreeInfo> = None;

        for line in stdout.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                if let Some(wt) = current.take() {
                    worktrees.push(wt);
                }
                current = Some(WorktreeInfo {
                    path: PathBuf::from(path),
                    head: String::new(),
                    branch: None,
                    is_bare: false,
                    is_detached: false,
                    is_locked: false,
                    is_prunable: false,
                });
            } else if let Some(ref mut wt) = current {
                if let Some(head) = line.strip_prefix("HEAD ") {
                    wt.head = head.to_string();
                } else if let Some(branch) = line.strip_prefix("branch ") {
                    // Convert refs/heads/xxx to xxx
                    wt.branch = Some(
                        branch
                            .strip_prefix("refs/heads/")
                            .unwrap_or(branch)
                            .to_string(),
                    );
                } else if line == "bare" {
                    wt.is_bare = true;
                } else if line == "detached" {
                    wt.is_detached = true;
                } else if line == "locked" {
                    wt.is_locked = true;
                } else if line == "prunable" {
                    wt.is_prunable = true;
                }
            }
        }

        if let Some(wt) = current {
            worktrees.push(wt);
        }

        Ok(worktrees)
    }

    /// Create a new worktree
    pub fn create_worktree(&self, path: &Path, branch: &str, new_branch: bool) -> Result<()> {
        debug!(
            category = "git",
            path = %path.display(),
            branch,
            new_branch,
            "Creating git worktree"
        );

        let mut args = vec!["worktree", "add"];

        if new_branch {
            args.push("-b");
            args.push(branch);
        }

        let path_str = path.to_string_lossy();
        args.push(&path_str);

        if !new_branch {
            args.push(branch);
        }

        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree add".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "worktree_add",
                path = %path.display(),
                branch,
                new_branch,
                "Git worktree created"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                operation = "worktree_add",
                path = %path.display(),
                branch,
                error = err_msg.as_str(),
                "Failed to create git worktree"
            );
            Err(GwtError::GitOperationFailed {
                operation: "worktree add".to_string(),
                details: err_msg,
            })
        }
    }

    /// Remove a worktree
    pub fn remove_worktree(&self, path: &Path, force: bool) -> Result<()> {
        debug!(
            category = "git",
            path = %path.display(),
            force,
            "Removing git worktree"
        );

        let path_str = path.to_string_lossy();
        let args = if force {
            vec!["worktree", "remove", "--force", &path_str]
        } else {
            vec!["worktree", "remove", &path_str]
        };

        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree remove".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "worktree_remove",
                path = %path.display(),
                force,
                "Git worktree removed"
            );
            return Ok(());
        }

        let err_msg = String::from_utf8_lossy(&output.stderr).to_string();

        // Handle submodule-related errors by manually removing the directory
        // Git cannot remove worktrees containing submodules even with --force
        if err_msg.contains("submodules cannot be moved or removed") {
            warn!(
                category = "git",
                path = %path.display(),
                "Worktree contains submodules, removing directory manually"
            );

            if path.exists() {
                std::fs::remove_dir_all(path).map_err(|e| GwtError::GitOperationFailed {
                    operation: "worktree remove (manual)".to_string(),
                    details: format!("Failed to remove worktree directory: {}", e),
                })?;
            }

            // Prune stale worktree metadata
            self.prune_worktrees()?;

            info!(
                category = "git",
                operation = "worktree_remove",
                path = %path.display(),
                force,
                "Git worktree removed (with submodules)"
            );
            return Ok(());
        }

        error!(
            category = "git",
            operation = "worktree_remove",
            path = %path.display(),
            error = err_msg.as_str(),
            "Failed to remove git worktree"
        );
        Err(GwtError::GitOperationFailed {
            operation: "worktree remove".to_string(),
            details: err_msg,
        })
    }

    /// Prune stale worktree metadata
    pub fn prune_worktrees(&self) -> Result<()> {
        debug!(category = "git", "Pruning stale worktree metadata");

        let output = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree prune".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "worktree_prune",
                "Worktree metadata pruned"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                operation = "worktree_prune",
                error = err_msg.as_str(),
                "Failed to prune worktree metadata"
            );
            Err(GwtError::GitOperationFailed {
                operation: "worktree prune".to_string(),
                details: err_msg,
            })
        }
    }
}

/// Information about a worktree from git worktree list
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Worktree path
    pub path: PathBuf,
    /// HEAD commit SHA
    pub head: String,
    /// Branch name (None if detached or bare)
    pub branch: Option<String>,
    /// Is this a bare repository
    pub is_bare: bool,
    /// Is HEAD detached
    pub is_detached: bool,
    /// Is worktree locked
    pub is_locked: bool,
    /// Is worktree prunable
    pub is_prunable: bool,
}

/// Get the main repository root from any path (resolves through worktree to main repo)
/// This is a standalone function that doesn't require a Repository instance.
/// For worktrees, this returns the path to the main repository.
/// For bare repos, this returns the bare repo path itself (SPEC-a70a1ece).
/// For normal repos or non-repo paths, this returns the original path.
pub fn get_main_repo_root(path: &Path) -> PathBuf {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let common_dir = String::from_utf8_lossy(&o.stdout).trim().to_string();

            // SPEC-a70a1ece: For bare repos, git-common-dir returns "." - return path as-is
            if common_dir == "." {
                return path.to_path_buf();
            }

            let common_path = PathBuf::from(&common_dir);
            if common_path.is_absolute() {
                common_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| path.to_path_buf())
            } else {
                // Relative path - resolve from current path
                let resolved = path.join(&common_path);
                resolved
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| path.to_path_buf())
            }
        }
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_repo() -> (TempDir, Repository) {
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

        let repo = Repository::discover(temp.path()).unwrap();
        (temp, repo)
    }

    fn create_test_repo_with_commit() -> (TempDir, Repository) {
        let (temp, repo) = create_test_repo();
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
        (temp, repo)
    }

    #[test]
    fn test_discover_not_found() {
        let temp = TempDir::new().unwrap();
        let result = Repository::discover(temp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_discover_found() {
        let (temp, repo) = create_test_repo();
        assert_eq!(repo.root(), temp.path());
    }

    #[test]
    fn test_has_uncommitted_changes_clean() {
        let (_temp, repo) = create_test_repo();
        // Empty repo, no changes
        let result = repo.has_uncommitted_changes().unwrap();
        assert!(!result);
    }

    #[test]
    fn test_has_uncommitted_changes_dirty() {
        let (temp, repo) = create_test_repo();
        // Create an untracked file
        std::fs::write(temp.path().join("test.txt"), "hello").unwrap();
        let result = repo.has_uncommitted_changes().unwrap();
        assert!(result);
    }

    #[test]
    fn test_head_name_initial() {
        let (_temp, repo) = create_test_repo();
        // Git 2.28+ defaults to main, older versions use master
        let name = repo.head_name().unwrap();
        // Initial repo might not have a valid HEAD yet
        assert!(
            name.is_none() || name.as_deref() == Some("main") || name.as_deref() == Some("master")
        );
    }

    #[test]
    fn test_list_worktrees() {
        let (temp, repo) = create_test_repo();
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

        let worktrees = repo.list_worktrees().unwrap();
        assert_eq!(worktrees.len(), 1);
        // macOS temp paths may appear as /var/... while git reports /private/var/... (canonical).
        let expected = temp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| temp.path().to_path_buf());
        let actual = worktrees[0]
            .path
            .canonicalize()
            .unwrap_or_else(|_| worktrees[0].path.clone());
        assert_eq!(actual, expected);
    }

    /// Issue #774: Repository::open should work with worktrees
    /// Worktrees have a .git file (not directory) pointing to the main repo
    #[test]
    fn test_open_worktree() {
        let (temp, repo) = create_test_repo_with_commit();

        // Create a worktree
        let worktree_path = temp.path().join(".worktrees").join("test-branch");
        repo.create_worktree(&worktree_path, "test-branch", true)
            .unwrap();

        // Verify the worktree has a .git file (not directory)
        let git_path = worktree_path.join(".git");
        assert!(git_path.exists(), ".git should exist in worktree");
        assert!(git_path.is_file(), ".git should be a file in worktree");

        // Repository::open should succeed on the worktree
        let wt_repo = Repository::open(&worktree_path);
        assert!(
            wt_repo.is_ok(),
            "Repository::open should succeed on worktree: {:?}",
            wt_repo.err()
        );

        let wt_repo = wt_repo.unwrap();
        assert_eq!(wt_repo.root(), worktree_path);
    }

    /// Issue #774: Repository::discover should work with worktrees
    #[test]
    fn test_discover_worktree() {
        let (temp, repo) = create_test_repo_with_commit();

        // Create a worktree
        let worktree_path = temp.path().join(".worktrees").join("test-branch");
        repo.create_worktree(&worktree_path, "test-branch", true)
            .unwrap();

        // Repository::discover should succeed from within the worktree
        let wt_repo = Repository::discover(&worktree_path);
        assert!(
            wt_repo.is_ok(),
            "Repository::discover should succeed on worktree: {:?}",
            wt_repo.err()
        );

        let wt_repo = wt_repo.unwrap();
        assert_eq!(wt_repo.root(), worktree_path);
    }

    /// Issue #774: Repository::discover should work from subdirectory of worktree
    #[test]
    fn test_discover_worktree_subdirectory() {
        let (temp, repo) = create_test_repo_with_commit();

        // Create a worktree
        let worktree_path = temp.path().join(".worktrees").join("test-branch");
        repo.create_worktree(&worktree_path, "test-branch", true)
            .unwrap();

        // Create a subdirectory in the worktree
        let subdir = worktree_path.join("subdir");
        std::fs::create_dir(&subdir).unwrap();

        // Repository::discover should succeed from subdirectory
        let wt_repo = Repository::discover(&subdir);
        assert!(
            wt_repo.is_ok(),
            "Repository::discover should succeed from worktree subdirectory: {:?}",
            wt_repo.err()
        );

        let wt_repo = wt_repo.unwrap();
        assert_eq!(wt_repo.root(), worktree_path);
    }

    // SPEC-a70a1ece T207: Unit tests for detect_repo_type()

    #[test]
    fn test_detect_repo_type_empty_dir() {
        let temp = TempDir::new().unwrap();
        let result = detect_repo_type(temp.path());
        assert_eq!(result, RepoType::Empty);
    }

    #[test]
    fn test_detect_repo_type_non_repo() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("some_file.txt"), "content").unwrap();
        let result = detect_repo_type(temp.path());
        assert_eq!(result, RepoType::NonRepo);
    }

    #[test]
    fn test_detect_repo_type_normal() {
        let (temp, _repo) = create_test_repo();
        let result = detect_repo_type(temp.path());
        assert_eq!(result, RepoType::Normal);
    }

    #[test]
    fn test_detect_repo_type_bare() {
        let temp = TempDir::new().unwrap();
        Command::new("git")
            .args(["init", "--bare"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        let result = detect_repo_type(temp.path());
        assert_eq!(result, RepoType::Bare);
    }

    #[test]
    fn test_detect_repo_type_worktree() {
        let (temp, repo) = create_test_repo_with_commit();

        // Create a worktree
        let worktree_path = temp.path().join(".worktrees").join("wt-branch");
        repo.create_worktree(&worktree_path, "wt-branch", true)
            .unwrap();

        // Check worktree path is detected as Worktree type
        let result = detect_repo_type(&worktree_path);
        assert_eq!(result, RepoType::Worktree);
    }

    #[test]
    fn test_is_empty_dir_true() {
        let temp = TempDir::new().unwrap();
        assert!(is_empty_dir(temp.path()));
    }

    #[test]
    fn test_is_empty_dir_false_with_file() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("file.txt"), "").unwrap();
        assert!(!is_empty_dir(temp.path()));
    }

    #[test]
    fn test_is_empty_dir_false_with_hidden() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join(".hidden"), "").unwrap();
        assert!(!is_empty_dir(temp.path()));
    }

    #[test]
    fn test_is_bare_repository_true() {
        let temp = TempDir::new().unwrap();
        Command::new("git")
            .args(["init", "--bare"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(is_bare_repository(temp.path()));
    }

    #[test]
    fn test_is_bare_repository_false() {
        let (temp, _repo) = create_test_repo();
        assert!(!is_bare_repository(temp.path()));
    }

    #[test]
    fn test_is_git_repo_true() {
        let (temp, _repo) = create_test_repo();
        assert!(is_git_repo(temp.path()));
    }

    #[test]
    fn test_is_git_repo_false() {
        let temp = TempDir::new().unwrap();
        assert!(!is_git_repo(temp.path()));
    }

    #[test]
    fn test_is_inside_worktree_true() {
        // is_inside_worktree returns true only when .git is a FILE (not directory)
        // which happens for git worktrees (not the main repo)
        let (temp, repo) = create_test_repo_with_commit();

        // Create a worktree - worktrees have .git as a file
        let worktree_path = temp.path().join(".worktrees").join("test-wt");
        repo.create_worktree(&worktree_path, "test-wt", true)
            .unwrap();

        // The worktree should have .git as a file
        assert!(is_inside_worktree(&worktree_path));

        // The main repo should NOT be detected as worktree (has .git directory)
        assert!(!is_inside_worktree(temp.path()));
    }

    #[test]
    fn test_is_inside_worktree_false_for_bare() {
        let temp = TempDir::new().unwrap();
        Command::new("git")
            .args(["init", "--bare"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(!is_inside_worktree(temp.path()));
    }

    #[test]
    fn test_find_bare_repo_in_dir_found() {
        let temp = TempDir::new().unwrap();
        // Create a bare repository with .git suffix
        let bare_path = temp.path().join("my-repo.git");
        std::fs::create_dir(&bare_path).unwrap();
        Command::new("git")
            .args(["init", "--bare"])
            .current_dir(&bare_path)
            .output()
            .unwrap();

        // Should find the bare repo from the parent directory
        let result = find_bare_repo_in_dir(temp.path());
        assert!(result.is_some());
        assert_eq!(result.unwrap(), bare_path);
    }

    #[test]
    fn test_find_bare_repo_in_dir_not_found() {
        let temp = TempDir::new().unwrap();
        // Create a regular file
        std::fs::write(temp.path().join("file.txt"), "content").unwrap();

        // Should not find any bare repo
        let result = find_bare_repo_in_dir(temp.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_find_bare_repo_in_dir_ignores_non_bare() {
        let temp = TempDir::new().unwrap();
        // Create a directory with .git suffix but NOT a bare repo
        let fake_git = temp.path().join("fake.git");
        std::fs::create_dir(&fake_git).unwrap();
        std::fs::write(fake_git.join("file.txt"), "content").unwrap();

        // Should not find it (not a bare repo)
        let result = find_bare_repo_in_dir(temp.path());
        assert!(result.is_none());
    }
}
