//! Worktree manager

use super::{CleanupCandidate, Worktree, WorktreeLocation, WorktreePath, WorktreeStatus};
use crate::error::{GwtError, Result};
use crate::git::{get_main_repo_root, is_bare_repository, Branch, Remote, Repository};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

/// Protected branch names that cannot be deleted
const PROTECTED_BRANCHES: &[&str] = &["main", "master", "develop", "release"];

/// Worktree manager for creating, listing, and removing worktrees
pub struct WorktreeManager {
    /// Repository root path
    repo_root: PathBuf,
    /// Git repository handle
    repo: Repository,
    /// Worktree location strategy (SPEC-a70a1ece T404-T405)
    location: WorktreeLocation,
}

impl WorktreeManager {
    /// Create a new worktree manager
    ///
    /// If the given path is inside a worktree, this automatically resolves
    /// to the main repository root to ensure worktrees are created at the
    /// correct location (e.g., /repo/.worktrees/ instead of /repo/.worktrees/branch/.worktrees/)
    ///
    /// SPEC-a70a1ece T404-T405: Auto-detects bare repositories and uses Sibling location
    pub fn new(repo_root: impl AsRef<Path>) -> Result<Self> {
        let repo_root = repo_root.as_ref().to_path_buf();
        // Resolve to main repo root in case we're inside a worktree
        let main_repo_root = get_main_repo_root(&repo_root);
        let repo = Repository::discover(&main_repo_root)?;

        // SPEC-a70a1ece: Detect bare repository and use appropriate location strategy
        let location = if is_bare_repository(&main_repo_root) {
            debug!(
                category = "worktree",
                repo = %main_repo_root.display(),
                "Bare repository detected, using Sibling location"
            );
            WorktreeLocation::Sibling
        } else {
            WorktreeLocation::Subdir
        };

        Ok(Self {
            repo_root: main_repo_root,
            repo,
            location,
        })
    }

    /// Get the repository root path
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// List all worktrees
    pub fn list(&self) -> Result<Vec<Worktree>> {
        let git_worktrees = self.repo.list_worktrees()?;
        let mut worktrees = Vec::with_capacity(git_worktrees.len());

        for info in &git_worktrees {
            let mut wt = Worktree::from_git_info(info);

            // Check for changes if worktree is active
            if wt.status == WorktreeStatus::Active {
                if let Ok(wt_repo) = Repository::open(&wt.path) {
                    wt.has_changes = wt_repo.has_uncommitted_changes().unwrap_or(false);
                    wt.has_unpushed = wt_repo.has_unpushed_commits().unwrap_or(false);
                }
            }

            tracing::debug!(
                "Worktree: branch={:?}, path={:?}, status={:?}, has_changes={}, has_unpushed={}",
                wt.branch,
                wt.path,
                wt.status,
                wt.has_changes,
                wt.has_unpushed
            );

            worktrees.push(wt);
        }

        Ok(worktrees)
    }

    /// List all worktrees without checking git status (fast path)
    pub fn list_basic(&self) -> Result<Vec<Worktree>> {
        let git_worktrees = self.repo.list_worktrees()?;
        let mut worktrees = Vec::with_capacity(git_worktrees.len());

        for info in &git_worktrees {
            let wt = Worktree::from_git_info(info);
            worktrees.push(wt);
        }

        Ok(worktrees)
    }

    /// Get a specific worktree by branch name
    pub fn get_by_branch(&self, branch_name: &str) -> Result<Option<Worktree>> {
        let worktrees = self.list()?;
        Ok(worktrees
            .into_iter()
            .find(|wt| wt.branch.as_deref() == Some(branch_name)))
    }

    /// Get a specific worktree by branch name without status checks (fast path)
    pub fn get_by_branch_basic(&self, branch_name: &str) -> Result<Option<Worktree>> {
        let worktrees = self.list_basic()?;
        Ok(worktrees
            .into_iter()
            .find(|wt| wt.branch.as_deref() == Some(branch_name)))
    }

    /// Get a specific worktree by path
    pub fn get_by_path(&self, path: &Path) -> Result<Option<Worktree>> {
        let worktrees = self.list()?;
        if worktrees.is_empty() {
            return Ok(None);
        }

        let target = path;
        let target_canon = std::fs::canonicalize(target).ok();

        Ok(worktrees.into_iter().find(|wt| {
            if wt.path == target {
                return true;
            }

            // On macOS (and some temp-dir setups), git may report a canonicalized path
            // (e.g., /private/var/...) while our callers hold a non-canonical alias
            // (e.g., /var/...). Fall back to canonical comparison when possible.
            match (&target_canon, std::fs::canonicalize(&wt.path).ok()) {
                (Some(a), Some(b)) => a == &b,
                _ => false,
            }
        }))
    }

    /// Handle existing path for worktree creation (FR-038-040)
    ///
    /// FR-038: Do not auto-recover worktrees when an existing path is found
    /// FR-039: Never delete existing directories automatically
    /// FR-040: Abort and prompt user for manual resolution when a path exists
    fn handle_existing_path(&self, path: &Path) -> Result<()> {
        // Check if this path is in the git worktree list
        let git_worktrees = self.repo.list_worktrees()?;
        let target = path;
        let target_canon = std::fs::canonicalize(target).ok();
        let is_in_worktree_list = git_worktrees.iter().any(|info| {
            if info.path == target {
                return true;
            }
            match (&target_canon, std::fs::canonicalize(&info.path).ok()) {
                (Some(a), Some(b)) => a == &b,
                _ => false,
            }
        });

        if is_in_worktree_list {
            // Path exists AND is in worktree list → real worktree conflict
            return Err(GwtError::WorktreeAlreadyExists {
                path: path.to_path_buf(),
            });
        }

        // FR-038: Path exists but NOT in worktree list → do not auto-recover
        // FR-039: Auto-recovery disabled → do not delete anything
        // FR-040: Always require manual resolution
        Err(GwtError::WorktreePathConflict {
            path: path.to_path_buf(),
        })
    }

    fn get_registered_worktree_by_path_basic(&self, path: &Path) -> Result<Option<Worktree>> {
        let git_worktrees = self.repo.list_worktrees()?;
        let target = path;
        let target_canon = std::fs::canonicalize(target).ok();

        for info in git_worktrees {
            if info.path == target {
                return Ok(Some(Worktree::from_git_info(&info)));
            }
            match (&target_canon, std::fs::canonicalize(&info.path).ok()) {
                (Some(a), Some(b)) if a == &b => return Ok(Some(Worktree::from_git_info(&info))),
                _ => {}
            }
        }

        Ok(None)
    }

    fn handle_registered_worktree_path_conflict(
        &self,
        path: &Path,
        did_prune: &mut bool,
    ) -> Result<()> {
        let Some(wt) = self.get_registered_worktree_by_path_basic(path)? else {
            return Ok(());
        };

        match wt.status {
            WorktreeStatus::Active => Err(GwtError::WorktreeAlreadyExists { path: wt.path }),
            WorktreeStatus::Locked => Err(GwtError::WorktreeLocked { path: wt.path }),
            WorktreeStatus::Missing | WorktreeStatus::Prunable => {
                if *did_prune {
                    return Err(GwtError::OrphanedWorktree { path: wt.path });
                }

                if !self.prune_worktrees_if_safe()? {
                    return Err(GwtError::OrphanedWorktree { path: wt.path });
                }
                *did_prune = true;

                if self.get_registered_worktree_by_path_basic(path)?.is_some() {
                    return Err(GwtError::OrphanedWorktree { path: wt.path });
                }

                Ok(())
            }
        }
    }
    fn resolve_existing_worktree_for_create(
        &self,
        wt: Worktree,
        did_prune: &mut bool,
    ) -> Result<Option<Worktree>> {
        match wt.status {
            WorktreeStatus::Active => Ok(Some(wt)),
            WorktreeStatus::Locked => Err(GwtError::WorktreeLocked { path: wt.path }),
            WorktreeStatus::Missing | WorktreeStatus::Prunable => {
                // Stale worktree metadata can make git think the branch is still checked out.
                // Try a single safe prune to clear it, then re-check.
                if *did_prune {
                    return Err(GwtError::OrphanedWorktree { path: wt.path });
                }

                let branch = wt.branch.clone();
                if !self.prune_worktrees_if_safe()? {
                    return Err(GwtError::OrphanedWorktree { path: wt.path });
                }
                *did_prune = true;

                let Some(branch_name) = branch.as_deref() else {
                    return Ok(None);
                };

                match self.get_by_branch_basic(branch_name)? {
                    Some(wt2) => match wt2.status {
                        WorktreeStatus::Active => Ok(Some(wt2)),
                        WorktreeStatus::Locked => Err(GwtError::WorktreeLocked { path: wt2.path }),
                        WorktreeStatus::Missing | WorktreeStatus::Prunable => {
                            Err(GwtError::OrphanedWorktree { path: wt2.path })
                        }
                    },
                    None => Ok(None),
                }
            }
        }
    }

    fn prune_worktrees_if_safe(&self) -> Result<bool> {
        let current_name = current_worktree_metadata_name_for_repo(&self.repo_root);

        let output = match crate::process::command("git")
            .args(["worktree", "prune", "--dry-run", "--verbose"])
            .current_dir(&self.repo_root)
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                warn!(
                    category = "git",
                    operation = "worktree_prune_dry_run",
                    error = %e,
                    "Failed to run git worktree prune --dry-run"
                );
                return Ok(false);
            }
        };

        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            warn!(
                category = "git",
                operation = "worktree_prune_dry_run",
                error = %err_msg,
                "git worktree prune --dry-run failed"
            );
            return Ok(false);
        }

        let dry_run = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        if let Some(name) = current_name {
            let needle = format!("Removing worktrees/{}:", name);
            if dry_run.contains(&needle) {
                warn!(
                    category = "git",
                    operation = "worktree_prune",
                    worktree = name.as_str(),
                    "Refusing to auto-prune because current worktree metadata is in prune targets"
                );
                return Ok(false);
            }
        }

        self.prune()?;
        Ok(true)
    }

    /// Create a new worktree for an existing branch
    pub fn create_for_branch(&self, branch_name: &str) -> Result<Worktree> {
        debug!(
            category = "worktree",
            branch = branch_name,
            "Creating worktree for existing branch"
        );
        // Idempotency: if the branch is already checked out in some worktree, return it.
        // This prevents git worktree add failures like:
        // "fatal: '<branch>' is already checked out at '<path>'".
        let normalized_branch = normalize_remote_ref(branch_name);
        let mut did_prune = false;

        if let Some(wt) = self.get_by_branch_basic(branch_name)? {
            if let Some(wt) = self.resolve_existing_worktree_for_create(wt, &mut did_prune)? {
                return Ok(wt);
            }
        }

        if normalized_branch != branch_name {
            if let Some(wt) = self.get_by_branch_basic(normalized_branch)? {
                if let Some(wt) = self.resolve_existing_worktree_for_create(wt, &mut did_prune)? {
                    return Ok(wt);
                }
            }
        }

        let mut resolved_branch = branch_name.to_string();
        if !Branch::exists(&self.repo_root, branch_name)? {
            let remotes = Remote::list(&self.repo_root)?;

            // If caller passed a remote ref (e.g., origin/feature/foo), map it to the local
            // branch name and check again before fetching/creating anything.
            if let Some((remote_candidate, branch_candidate)) = split_remote_ref(normalized_branch)
            {
                if remotes.iter().any(|r| r.name == remote_candidate)
                    && Branch::remote_exists(&self.repo_root, remote_candidate, branch_candidate)?
                {
                    if let Some(wt) = self.get_by_branch_basic(branch_candidate)? {
                        if let Some(wt) =
                            self.resolve_existing_worktree_for_create(wt, &mut did_prune)?
                        {
                            return Ok(wt);
                        }
                    }
                }
            }

            let mut remote_branch =
                resolve_remote_branch(&self.repo_root, normalized_branch, &remotes)?;

            if remote_branch.is_none() && !remotes.is_empty() {
                // Refresh remote refs once if branch isn't found locally
                self.repo.fetch_all()?;
                remote_branch =
                    resolve_remote_branch(&self.repo_root, normalized_branch, &remotes)?;
            }

            if let Some((remote, branch)) = remote_branch {
                resolved_branch = branch.clone();
                if !Branch::exists(&self.repo_root, &resolved_branch)? {
                    // Check if refs/remotes/{remote}/{branch} exists locally
                    let has_local_remote_ref = crate::process::command("git")
                        .args([
                            "show-ref",
                            "--verify",
                            "--quiet",
                            &format!("refs/remotes/{}/{}", remote, branch),
                        ])
                        .current_dir(&self.repo_root)
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false);

                    if has_local_remote_ref {
                        // Normal repo with local remote ref: create branch from it
                        let remote_ref = format!("{}/{}", remote, resolved_branch);
                        Branch::create(&self.repo_root, &resolved_branch, &remote_ref)?;
                    } else {
                        // SPEC-a70a1ece FR-124: No local remote ref, fetch from remote
                        let fetch_output = crate::process::command("git")
                            .args(["fetch", &remote, &format!("{}:{}", branch, branch)])
                            .current_dir(&self.repo_root)
                            .output()
                            .map_err(|e| GwtError::GitOperationFailed {
                                operation: "fetch".to_string(),
                                details: e.to_string(),
                            })?;

                        if !fetch_output.status.success() {
                            let err = String::from_utf8_lossy(&fetch_output.stderr);
                            error!(
                                category = "worktree",
                                branch = branch.as_str(),
                                error = %err,
                                "Failed to fetch branch"
                            );
                            return Err(GwtError::GitOperationFailed {
                                operation: "fetch".to_string(),
                                details: err.to_string(),
                            });
                        }
                        debug!(
                            category = "worktree",
                            branch = branch.as_str(),
                            "Fetched branch from remote"
                        );
                    }
                }
            } else {
                error!(
                    category = "worktree",
                    branch = branch_name,
                    "Branch not found"
                );
                return Err(GwtError::BranchNotFound {
                    name: branch_name.to_string(),
                });
            }
        }

        if let Some(wt) = self.get_by_branch_basic(&resolved_branch)? {
            if let Some(wt) = self.resolve_existing_worktree_for_create(wt, &mut did_prune)? {
                return Ok(wt);
            }
        }

        // SPEC-a70a1ece T405: Use location-aware path generation
        let path =
            WorktreePath::generate_with_location(&self.repo_root, &resolved_branch, self.location);

        // Git can still have this path registered even when the directory is missing.
        // Reuse an existing active worktree only when it points to the same branch.
        // This avoids returning another branch's worktree when path sanitization
        // collides (for example, "feature/foo" vs "feature-foo" in Subdir mode).
        if let Some(wt) = self.get_registered_worktree_by_path_basic(&path)? {
            if wt.status == WorktreeStatus::Active
                && wt.branch.as_deref() == Some(resolved_branch.as_str())
            {
                return Ok(wt);
            }
        }

        // In case only metadata is stale (missing/prunable), remove stale metadata if safe.
        self.handle_registered_worktree_path_conflict(&path, &mut did_prune)?;

        // FR-038-040: Handle existing path (auto-recovery disabled)
        if path.exists() {
            self.handle_existing_path(&path)?;
        }

        // Create worktree (with one safe prune+retry on stale metadata)
        loop {
            match self.repo.create_worktree(&path, &resolved_branch, false) {
                Ok(()) => break,
                Err(err) => {
                    match &err {
                        GwtError::GitOperationFailed { operation, details }
                            if operation == "worktree add" =>
                        {
                            if let Some(conflict_path) =
                                parse_missing_registered_worktree_path(details)
                            {
                                if !did_prune && self.prune_worktrees_if_safe()? {
                                    did_prune = true;
                                    continue;
                                }
                                return Err(GwtError::OrphanedWorktree {
                                    path: conflict_path,
                                });
                            }
                            if let Some(checked_out_path) = parse_already_checked_out_path(details)
                            {
                                if checked_out_path.exists() {
                                    if let Some(wt) = self.get_by_path(&checked_out_path)? {
                                        return Ok(wt);
                                    }
                                    if let Some(wt) = self.get_by_branch_basic(&resolved_branch)? {
                                        return Ok(wt);
                                    }
                                } else if !did_prune {
                                    if self.prune_worktrees_if_safe()? {
                                        did_prune = true;
                                        continue;
                                    }
                                    return Err(GwtError::OrphanedWorktree {
                                        path: checked_out_path,
                                    });
                                } else {
                                    return Err(GwtError::OrphanedWorktree {
                                        path: checked_out_path,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                    return Err(err);
                }
            }
        }

        // SPEC-a70a1ece T1004-T1005: Initialize submodules (non-fatal on failure)
        if let Err(e) = crate::git::init_submodules(&path) {
            warn!(
                category = "worktree",
                path = %path.display(),
                error = %e,
                "Submodule initialization failed (non-fatal)"
            );
        }

        // Return the created worktree
        let worktree = self
            .get_by_path(&path)?
            .ok_or(GwtError::WorktreeNotFound { path: path.clone() })?;

        info!(
            category = "worktree",
            operation = "create",
            branch = resolved_branch.as_str(),
            path = %worktree.path.display(),
            "Worktree created for existing branch"
        );
        Ok(worktree)
    }

    /// Create a new worktree with a new branch
    pub fn create_new_branch(
        &self,
        branch_name: &str,
        base_branch: Option<&str>,
    ) -> Result<Worktree> {
        debug!(
            category = "worktree",
            branch = branch_name,
            base = base_branch.unwrap_or("HEAD"),
            "Creating worktree with new branch"
        );
        // SPEC-a70a1ece T405: Use location-aware path generation
        let path =
            WorktreePath::generate_with_location(&self.repo_root, branch_name, self.location);
        let mut did_prune = false;

        // FR-038-040: Handle existing path (auto-recovery disabled)
        if path.exists() {
            self.handle_existing_path(&path)?;
        }

        // The directory can be missing while git still has it registered.
        self.handle_registered_worktree_path_conflict(&path, &mut did_prune)?;

        // Check if branch already exists
        if Branch::exists(&self.repo_root, branch_name)? {
            error!(
                category = "worktree",
                branch = branch_name,
                "Branch already exists"
            );
            return Err(GwtError::BranchAlreadyExists {
                name: branch_name.to_string(),
            });
        }

        let normalized_base = base_branch.map(|base| normalize_remote_ref(base).to_string());
        let mut resolved_base = normalized_base.clone();
        // If base branch specified, validate it and ensure it's locally resolvable.
        if let Some(base) = normalized_base.as_deref() {
            // Verify base branch exists
            if !Branch::exists(&self.repo_root, base)? {
                if let Some((remote, branch)) = split_remote_ref(base) {
                    // Bare clones commonly keep fetched branches in refs/heads/* without
                    // refs/remotes/*, so allow remote-like bases to fall back to local refs.
                    if Branch::exists(&self.repo_root, branch)? {
                        resolved_base = Some(branch.to_string());
                    } else if !Branch::remote_exists(&self.repo_root, remote, branch)? {
                        error!(
                            category = "worktree",
                            branch = base,
                            "Base branch not found"
                        );
                        return Err(GwtError::BranchNotFound {
                            name: base.to_string(),
                        });
                    } else {
                        // remote_exists may succeed via ls-remote (bare repo), but the ref still
                        // needs to exist locally for `git reset --hard origin/<branch>` to work.
                        let mut local_remote_ref_present =
                            has_local_remote_ref(&self.repo_root, remote, branch);
                        if !local_remote_ref_present {
                            // Fetch once to materialize refs/remotes/* locally.
                            self.repo.fetch_all()?;
                            local_remote_ref_present =
                                has_local_remote_ref(&self.repo_root, remote, branch);
                        }

                        if local_remote_ref_present {
                            resolved_base = Some(base.to_string());
                        } else if Branch::exists(&self.repo_root, branch)? {
                            // Keep going with the local branch when only refs/heads/* exists.
                            resolved_base = Some(branch.to_string());
                        } else {
                            error!(
                                category = "worktree",
                                branch = base,
                                "Base branch not found after fetch"
                            );
                            return Err(GwtError::BranchNotFound {
                                name: base.to_string(),
                            });
                        }
                    }
                } else {
                    error!(
                        category = "worktree",
                        branch = base,
                        "Base branch not found"
                    );
                    return Err(GwtError::BranchNotFound {
                        name: base.to_string(),
                    });
                }
            }
        }

        // Create worktree with new branch
        loop {
            match self.repo.create_worktree(&path, branch_name, true) {
                Ok(()) => break,
                Err(err) => {
                    match &err {
                        GwtError::GitOperationFailed { operation, details }
                            if operation == "worktree add" =>
                        {
                            if let Some(conflict_path) =
                                parse_missing_registered_worktree_path(details)
                            {
                                if !did_prune && self.prune_worktrees_if_safe()? {
                                    did_prune = true;
                                    continue;
                                }
                                return Err(GwtError::OrphanedWorktree {
                                    path: conflict_path,
                                });
                            }
                        }
                        _ => {}
                    }
                    return Err(err);
                }
            }
        }

        // If base branch specified, reset to it
        if let Some(base) = resolved_base.as_deref() {
            let wt_repo = Repository::open(&path)?;
            let reset_output = crate::process::command("git")
                .args(["reset", "--hard", base])
                .current_dir(&path)
                .output()
                .map_err(|e| GwtError::WorktreeCreateFailed {
                    reason: e.to_string(),
                })?;
            if !reset_output.status.success() {
                return Err(GwtError::WorktreeCreateFailed {
                    reason: String::from_utf8_lossy(&reset_output.stderr).to_string(),
                });
            }
            drop(wt_repo);
        }

        // SPEC-a70a1ece T1004-T1005: Initialize submodules (non-fatal on failure)
        if let Err(e) = crate::git::init_submodules(&path) {
            warn!(
                category = "worktree",
                path = %path.display(),
                error = %e,
                "Submodule initialization failed (non-fatal)"
            );
        }

        // Return the created worktree
        let worktree = self
            .get_by_path(&path)?
            .ok_or(GwtError::WorktreeNotFound { path: path.clone() })?;

        info!(
            category = "worktree",
            operation = "create_new_branch",
            branch = branch_name,
            base = normalized_base.as_deref().unwrap_or("HEAD"),
            resolved_base = resolved_base.as_deref().unwrap_or("HEAD"),
            path = %worktree.path.display(),
            "Worktree created with new branch"
        );
        Ok(worktree)
    }

    /// Remove a worktree by path
    pub fn remove(&self, path: &Path, force: bool) -> Result<()> {
        debug!(
            category = "worktree",
            path = %path.display(),
            force,
            "Removing worktree"
        );

        // Check if worktree exists
        let wt = self
            .get_by_path(path)?
            .ok_or_else(|| GwtError::WorktreeNotFound {
                path: path.to_path_buf(),
            })?;

        let branch_name = wt.branch.clone();

        // Check for protected branch
        if let Some(ref branch) = wt.branch {
            if Self::is_protected(branch) && !force {
                warn!(
                    category = "worktree",
                    branch = branch.as_str(),
                    "Attempted to remove protected branch worktree"
                );
                return Err(GwtError::ProtectedBranch {
                    branch: branch.clone(),
                });
            }
        }

        // Check for uncommitted changes
        if wt.has_changes && !force {
            warn!(
                category = "worktree",
                path = %path.display(),
                "Attempted to remove worktree with uncommitted changes"
            );
            return Err(GwtError::UncommittedChanges);
        }

        // Remove worktree
        self.repo.remove_worktree(path, force)?;

        info!(
            category = "worktree",
            operation = "remove",
            path = %path.display(),
            branch = branch_name.as_deref().unwrap_or("unknown"),
            force,
            "Worktree removed"
        );

        Ok(())
    }

    /// Remove a worktree and delete the branch
    pub fn remove_with_branch(&self, path: &Path, force: bool) -> Result<()> {
        debug!(
            category = "worktree",
            path = %path.display(),
            force,
            "Removing worktree with branch"
        );

        let wt = self
            .get_by_path(path)?
            .ok_or_else(|| GwtError::WorktreeNotFound {
                path: path.to_path_buf(),
            })?;

        let branch_name = wt.branch.clone();

        // Remove worktree first
        self.remove(path, force)?;

        // Delete branch if it exists
        if let Some(ref name) = branch_name {
            if Branch::exists(&self.repo_root, name)? {
                Branch::delete(&self.repo_root, name, force)?;
                info!(
                    category = "worktree",
                    operation = "remove_with_branch",
                    path = %path.display(),
                    branch = name.as_str(),
                    "Branch deleted after worktree removal"
                );
            }
        }

        Ok(())
    }

    /// Remove a branch and its worktree if present (FR-011/FR-012)
    pub fn cleanup_branch(
        &self,
        branch_name: &str,
        force_worktree: bool,
        force_branch: bool,
    ) -> Result<()> {
        debug!(
            category = "worktree",
            branch = branch_name,
            force_worktree,
            force_branch,
            "Cleaning up branch"
        );

        let mut prune_error: Option<GwtError> = None;

        if let Some(wt) = self.get_by_branch(branch_name)? {
            if matches!(
                wt.status,
                WorktreeStatus::Missing | WorktreeStatus::Prunable
            ) {
                if let Err(err) = self.prune() {
                    prune_error = Some(err);
                }
            } else {
                match self.remove(&wt.path, force_worktree) {
                    Ok(_) => {}
                    Err(err) if Self::is_missing_worktree_error(&err) => {
                        if let Err(err) = self.prune() {
                            prune_error = Some(err);
                        }
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        if Branch::exists(&self.repo_root, branch_name)? {
            Branch::delete(&self.repo_root, branch_name, force_branch)?;
            info!(
                category = "worktree",
                operation = "cleanup_branch",
                branch = branch_name,
                "Branch deleted during cleanup"
            );
        }

        if let Some(err) = prune_error {
            return Err(err);
        }

        Ok(())
    }

    /// Check if a branch is protected
    pub fn is_protected(branch_name: &str) -> bool {
        PROTECTED_BRANCHES.contains(&branch_name)
    }

    fn is_missing_worktree_error(err: &GwtError) -> bool {
        match err {
            GwtError::WorktreeNotFound { .. } => true,
            GwtError::WorktreeRemoveFailed { .. } => true,
            GwtError::GitOperationFailed { operation, details } => {
                if operation != "worktree remove" {
                    return false;
                }
                let message = details.to_lowercase();
                message.contains("not a working tree")
                    || message.contains("not a worktree")
                    || message.contains("not a work tree")
                    || message.contains("no such file or directory")
            }
            _ => false,
        }
    }

    /// Detect orphaned worktrees
    pub fn detect_orphans(&self) -> Vec<CleanupCandidate> {
        CleanupCandidate::detect(&self.repo_root)
    }

    /// Auto-clean orphaned worktrees on startup
    pub fn auto_cleanup_orphans(&self) -> Result<usize> {
        debug!(
            category = "worktree",
            operation = "auto_cleanup",
            "Auto cleanup is disabled"
        );
        Ok(0)
    }

    /// Prune orphaned worktree metadata
    pub fn prune(&self) -> Result<()> {
        self.repo.prune_worktrees()
    }

    /// Lock a worktree
    pub fn lock(&self, path: &Path, reason: Option<&str>) -> Result<()> {
        debug!(
            category = "worktree",
            path = %path.display(),
            reason = reason.unwrap_or("none"),
            "Locking worktree"
        );

        let path_str = path.to_string_lossy();
        let mut args = vec!["worktree", "lock", &path_str];
        if let Some(r) = reason {
            args.push("--reason");
            args.push(r);
        }

        let output = crate::process::command("git")
            .args(&args)
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree lock".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "worktree",
                operation = "lock",
                path = %path.display(),
                reason = reason.unwrap_or("none"),
                "Worktree locked"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "worktree",
                path = %path.display(),
                error = err_msg.as_str(),
                "Failed to lock worktree"
            );
            Err(GwtError::GitOperationFailed {
                operation: "worktree lock".to_string(),
                details: err_msg,
            })
        }
    }

    /// Unlock a worktree
    pub fn unlock(&self, path: &Path) -> Result<()> {
        debug!(category = "worktree", path = %path.display(), "Unlocking worktree");

        let path_str = path.to_string_lossy();
        let output = crate::process::command("git")
            .args(["worktree", "unlock", &path_str])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree unlock".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "worktree",
                operation = "unlock",
                path = %path.display(),
                "Worktree unlocked"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "worktree",
                path = %path.display(),
                error = err_msg.as_str(),
                "Failed to unlock worktree"
            );
            Err(GwtError::GitOperationFailed {
                operation: "worktree unlock".to_string(),
                details: err_msg,
            })
        }
    }

    /// Get count of active worktrees (excluding main)
    pub fn active_count(&self) -> Result<usize> {
        let worktrees = self.list()?;
        Ok(worktrees
            .iter()
            .filter(|wt| !wt.is_main && wt.is_active())
            .count())
    }

    /// Get worktrees needing attention
    pub fn needing_attention(&self) -> Result<Vec<Worktree>> {
        let worktrees = self.list()?;
        Ok(worktrees
            .into_iter()
            .filter(|wt| wt.needs_attention())
            .collect())
    }
}

fn normalize_remote_ref(name: &str) -> &str {
    name.strip_prefix("remotes/").unwrap_or(name)
}

fn split_remote_ref(name: &str) -> Option<(&str, &str)> {
    name.split_once('/')
}

fn has_local_remote_ref(repo_root: &Path, remote: &str, branch: &str) -> bool {
    crate::process::git_command()
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{}/{}", remote, branch),
        ])
        .current_dir(repo_root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn parse_already_checked_out_path(details: &str) -> Option<PathBuf> {
    // Example:
    // fatal: 'feature/foo' is already checked out at '/path/to/worktree'
    if let Some(start) = details.find("is already checked out at '") {
        let rest = &details[start + "is already checked out at '".len()..];
        if let Some(end) = rest.find('\'') {
            return Some(PathBuf::from(rest[..end].trim()));
        }
    }
    if let Some(start) = details.find("is already checked out at \"") {
        let rest = &details[start + "is already checked out at \"".len()..];
        if let Some(end) = rest.find('"') {
            return Some(PathBuf::from(rest[..end].trim()));
        }
    }
    None
}

fn parse_missing_registered_worktree_path(details: &str) -> Option<PathBuf> {
    // Example:
    // fatal: '/path/to/worktree' is a missing but already registered worktree; use 'add -f' to override, or 'prune' or 'remove' to clear
    if let Some(start) = details.find("fatal: '") {
        let rest = &details[start + "fatal: '".len()..];
        if let Some(end) = rest.find("' is a missing but already registered worktree") {
            return Some(PathBuf::from(rest[..end].trim()));
        }
    }

    if let Some(start) = details.find("fatal: \"") {
        let rest = &details[start + "fatal: \"".len()..];
        if let Some(end) = rest.find("\" is a missing but already registered worktree") {
            return Some(PathBuf::from(rest[..end].trim()));
        }
    }

    None
}

fn current_worktree_metadata_name_for_repo(repo_root: &Path) -> Option<String> {
    fn rev_parse_path(dir: &Path, arg: &str) -> Option<PathBuf> {
        let output = crate::process::command("git")
            .args(["rev-parse", arg])
            .current_dir(dir)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() {
            return None;
        }

        let p = PathBuf::from(&raw);
        Some(if p.is_absolute() { p } else { dir.join(p) })
    }

    let cwd = std::env::current_dir().ok()?;
    let cwd_common = rev_parse_path(&cwd, "--git-common-dir")?;
    let repo_common = rev_parse_path(repo_root, "--git-common-dir")?;

    let same_common = match (
        std::fs::canonicalize(&cwd_common).ok(),
        std::fs::canonicalize(&repo_common).ok(),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => cwd_common == repo_common,
    };

    if !same_common {
        return None;
    }

    let git_dir = rev_parse_path(&cwd, "--git-dir")?;
    let abs_git_dir = std::fs::canonicalize(&git_dir).unwrap_or(git_dir);

    // Linked worktrees use <common-dir>/worktrees/<name> as git-dir.
    let parent = abs_git_dir.parent()?;
    if parent.file_name().and_then(|n| n.to_str()) != Some("worktrees") {
        return None;
    }

    abs_git_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

fn ordered_remote_names(remotes: &[Remote]) -> Vec<String> {
    let mut names: Vec<String> = remotes.iter().map(|r| r.name.clone()).collect();
    names.sort_by(|a, b| {
        if a == "origin" && b != "origin" {
            std::cmp::Ordering::Less
        } else if b == "origin" && a != "origin" {
            std::cmp::Ordering::Greater
        } else {
            a.cmp(b)
        }
    });
    names
}

fn resolve_remote_branch(
    repo_root: &Path,
    branch_name: &str,
    remotes: &[Remote],
) -> Result<Option<(String, String)>> {
    if remotes.is_empty() {
        return Ok(None);
    }

    let normalized_branch = normalize_remote_ref(branch_name);
    let remote_names = ordered_remote_names(remotes);

    if let Some((remote_candidate, branch_candidate)) = split_remote_ref(normalized_branch) {
        if remote_names.iter().any(|name| name == remote_candidate)
            && Branch::remote_exists(repo_root, remote_candidate, branch_candidate)?
        {
            return Ok(Some((
                remote_candidate.to_string(),
                branch_candidate.to_string(),
            )));
        }
    }

    for remote in remote_names {
        if Branch::remote_exists(repo_root, &remote, normalized_branch)? {
            return Ok(Some((remote, normalized_branch.to_string())));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn canonicalize_or_self(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    fn run_git_in(dir: &Path, args: &[&str]) {
        let output = crate::process::command("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(dir: &Path, args: &[&str]) -> String {
        let output = crate::process::command("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        crate::process::command("git")
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        crate::process::command("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        crate::process::command("git")
            .args(["config", "user.name", "Test"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        // Create initial commit
        std::fs::write(temp.path().join("test.txt"), "hello").unwrap();
        crate::process::command("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        crate::process::command("git")
            .args(["commit", "-m", "initial"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        temp
    }

    #[test]
    fn test_is_protected() {
        assert!(WorktreeManager::is_protected("main"));
        assert!(WorktreeManager::is_protected("master"));
        assert!(WorktreeManager::is_protected("develop"));
        assert!(!WorktreeManager::is_protected("feature/foo"));
    }

    #[test]
    fn test_list_worktrees() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let worktrees = manager.list().unwrap();
        assert_eq!(worktrees.len(), 1);
    }

    #[test]
    fn test_create_new_branch_worktree() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let wt = manager.create_new_branch("feature/test", None).unwrap();
        assert_eq!(wt.branch, Some("feature/test".to_string()));
        assert!(wt.path.exists());

        let worktrees = manager.list().unwrap();
        assert_eq!(worktrees.len(), 2);
    }

    #[test]
    fn test_create_for_existing_branch() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        // Create a branch first
        Branch::create(temp.path(), "feature/existing", "HEAD").unwrap();

        let wt = manager.create_for_branch("feature/existing").unwrap();
        assert_eq!(wt.branch, Some("feature/existing".to_string()));
        assert!(wt.path.exists());
    }

    #[test]
    fn test_create_for_branch_returns_existing_worktree_when_already_present() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        Branch::create(temp.path(), "feature/exists", "HEAD").unwrap();

        let wt1 = manager.create_for_branch("feature/exists").unwrap();
        let wt2 = manager.create_for_branch("feature/exists").unwrap();

        assert_eq!(
            canonicalize_or_self(&wt2.path),
            canonicalize_or_self(&wt1.path)
        );

        // Should not create a second worktree for the same branch.
        let worktrees = manager.list().unwrap();
        assert_eq!(worktrees.len(), 2);
    }

    #[test]
    fn test_create_for_branch_does_not_reuse_detached_registered_worktree_path() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let branch = "feature/reuse-path";
        Branch::create(temp.path(), branch, "HEAD").unwrap();

        // Register the expected worktree path as a detached active worktree.
        let wt_path = WorktreePath::generate(temp.path(), branch);
        run_git_in(
            temp.path(),
            &[
                "worktree",
                "add",
                "--detach",
                wt_path.to_str().unwrap(),
                "HEAD",
            ],
        );

        let result = manager.create_for_branch(branch);
        assert!(matches!(
            result,
            Err(GwtError::WorktreeAlreadyExists { .. })
        ));
    }

    #[test]
    fn test_create_for_branch_does_not_reuse_active_registered_path_for_other_branch() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        Branch::create(temp.path(), "feature-foo", "HEAD").unwrap();
        Branch::create(temp.path(), "feature/foo", "HEAD").unwrap();

        // In Subdir mode both names map to ".worktrees/feature-foo".
        let existing = manager.create_for_branch("feature-foo").unwrap();
        assert_eq!(existing.branch.as_deref(), Some("feature-foo"));

        let result = manager.create_for_branch("feature/foo");
        assert!(matches!(
            result,
            Err(GwtError::WorktreeAlreadyExists { .. })
        ));
    }

    #[test]
    fn test_create_for_branch_on_current_branch_returns_main_worktree() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let current_branch = git_stdout(temp.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
        let wt = manager.create_for_branch(&current_branch).unwrap();

        assert_eq!(
            canonicalize_or_self(&wt.path),
            canonicalize_or_self(temp.path())
        );

        // No new worktree should be created.
        let worktrees = manager.list().unwrap();
        assert_eq!(worktrees.len(), 1);
    }

    #[test]
    fn test_create_for_branch_recovers_missing_registered_worktree_path() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let branch = "feature/auto-merge";
        Branch::create(temp.path(), branch, "HEAD").unwrap();

        // Register the target path as a detached worktree, then delete the directory.
        // This simulates: "<path> is a missing but already registered worktree".
        let wt_path = WorktreePath::generate(temp.path(), branch);
        run_git_in(
            temp.path(),
            &[
                "worktree",
                "add",
                "--detach",
                wt_path.to_str().unwrap(),
                "HEAD",
            ],
        );
        std::fs::remove_dir_all(&wt_path).unwrap();

        let wt = manager.create_for_branch(branch).unwrap();
        assert_eq!(wt.branch.as_deref(), Some(branch));
        assert!(wt.path.exists());
    }

    #[test]
    fn test_get_by_branch_basic_skips_status_checks() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let wt = manager.create_new_branch("feature/dirty", None).unwrap();
        std::fs::write(wt.path.join("dirty.txt"), "dirty").unwrap();

        let basic = manager
            .get_by_branch_basic("feature/dirty")
            .unwrap()
            .expect("worktree should exist");
        assert_eq!(basic.branch.as_deref(), Some("feature/dirty"));
        assert!(!basic.has_changes);

        let detailed = manager
            .get_by_branch("feature/dirty")
            .unwrap()
            .expect("worktree should exist");
        assert!(detailed.has_changes);
    }

    #[test]
    fn test_create_for_remote_branch_with_slash_fetches() {
        let temp = create_test_repo();

        let remote = TempDir::new().unwrap();
        let remote_path = remote.path().to_string_lossy().to_string();
        run_git_in(remote.path(), &["init", "--bare"]);

        run_git_in(
            temp.path(),
            &["remote", "add", "origin", remote_path.as_str()],
        );

        let default_branch = git_stdout(temp.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
        run_git_in(
            temp.path(),
            &["push", "-u", "origin", default_branch.as_str()],
        );

        let creator = TempDir::new().unwrap();
        let creator_path = creator.path().to_string_lossy().to_string();
        let clone_output = crate::process::command("git")
            .args(["clone", remote_path.as_str(), creator_path.as_str()])
            .output()
            .unwrap();
        assert!(
            clone_output.status.success(),
            "git clone failed: {}",
            String::from_utf8_lossy(&clone_output.stderr)
        );

        run_git_in(creator.path(), &["checkout", "-b", "feature/issue-42"]);
        run_git_in(creator.path(), &["push", "origin", "feature/issue-42"]);

        assert!(!Branch::exists(temp.path(), "feature/issue-42").unwrap());
        // SPEC-a70a1ece FR-124: remote_exists now uses ls-remote fallback, so it finds the branch
        assert!(Branch::remote_exists(temp.path(), "origin", "feature/issue-42").unwrap());

        let manager = WorktreeManager::new(temp.path()).unwrap();
        let wt = manager.create_for_branch("feature/issue-42").unwrap();
        assert_eq!(wt.branch, Some("feature/issue-42".to_string()));
        assert!(wt.path.exists());
    }

    #[test]
    fn test_create_new_branch_from_remote_base_fetches_and_resets() {
        let temp = create_test_repo();

        let remote = TempDir::new().unwrap();
        let remote_path = remote.path().to_string_lossy().to_string();
        run_git_in(remote.path(), &["init", "--bare"]);

        run_git_in(
            temp.path(),
            &["remote", "add", "origin", remote_path.as_str()],
        );

        let default_branch = git_stdout(temp.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
        run_git_in(
            temp.path(),
            &["push", "-u", "origin", default_branch.as_str()],
        );

        // Create a remote-only branch with an extra commit so HEAD differs from the base ref.
        let creator = TempDir::new().unwrap();
        let creator_path = creator.path().to_string_lossy().to_string();
        let clone_output = crate::process::command("git")
            .args(["clone", remote_path.as_str(), creator_path.as_str()])
            .output()
            .unwrap();
        assert!(
            clone_output.status.success(),
            "git clone failed: {}",
            String::from_utf8_lossy(&clone_output.stderr)
        );

        // CI environments may not have git author identity configured globally.
        run_git_in(creator.path(), &["config", "user.email", "test@test.com"]);
        run_git_in(creator.path(), &["config", "user.name", "Test"]);

        run_git_in(creator.path(), &["checkout", "-b", "feature/remote-base"]);
        std::fs::write(creator.path().join("remote.txt"), "remote").unwrap();
        run_git_in(creator.path(), &["add", "."]);
        run_git_in(creator.path(), &["commit", "-m", "remote commit"]);
        run_git_in(creator.path(), &["push", "origin", "feature/remote-base"]);
        let remote_commit = git_stdout(creator.path(), &["rev-parse", "HEAD"]);

        let manager = WorktreeManager::new(temp.path()).unwrap();
        let wt = manager
            .create_new_branch("feature/from-remote", Some("origin/feature/remote-base"))
            .unwrap();

        // The new worktree should be based on the remote ref (not the local default branch HEAD).
        let wt_head = git_stdout(&wt.path, &["rev-parse", "HEAD"]);
        assert_eq!(wt_head, remote_commit);
    }

    #[test]
    fn test_create_for_branch_does_not_misinterpret_branch_as_remote_ref() {
        let temp = create_test_repo();

        let remote = TempDir::new().unwrap();
        let remote_path = remote.path().to_string_lossy().to_string();
        run_git_in(remote.path(), &["init", "--bare"]);

        run_git_in(
            temp.path(),
            &["remote", "add", "origin", remote_path.as_str()],
        );

        let default_branch = git_stdout(temp.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
        run_git_in(
            temp.path(),
            &["push", "-u", "origin", default_branch.as_str()],
        );

        let creator = TempDir::new().unwrap();
        let creator_path = creator.path().to_string_lossy().to_string();
        let clone_output = crate::process::command("git")
            .args(["clone", remote_path.as_str(), creator_path.as_str()])
            .output()
            .unwrap();
        assert!(
            clone_output.status.success(),
            "git clone failed: {}",
            String::from_utf8_lossy(&clone_output.stderr)
        );

        run_git_in(creator.path(), &["checkout", "-b", "feature/foo"]);
        run_git_in(creator.path(), &["push", "origin", "feature/foo"]);

        // Add a remote whose name collides with a common branch prefix.
        // This should not make "feature/foo" get misinterpreted as "<remote>/<branch>".
        run_git_in(
            temp.path(),
            &["remote", "add", "feature", remote_path.as_str()],
        );

        let manager = WorktreeManager::new(temp.path()).unwrap();

        Branch::create(temp.path(), "foo", "HEAD").unwrap();
        let foo_wt = manager.create_for_branch("foo").unwrap();
        assert_eq!(foo_wt.branch.as_deref(), Some("foo"));

        assert!(!Branch::exists(temp.path(), "feature/foo").unwrap());

        let wt = manager.create_for_branch("feature/foo").unwrap();
        assert_eq!(wt.branch.as_deref(), Some("feature/foo"));
        assert_ne!(
            canonicalize_or_self(&wt.path),
            canonicalize_or_self(&foo_wt.path)
        );
    }

    #[test]
    fn test_remove_worktree() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let wt = manager.create_new_branch("feature/remove", None).unwrap();
        let path = wt.path.clone();

        manager.remove(&path, false).unwrap();

        let worktrees = manager.list().unwrap();
        assert_eq!(worktrees.len(), 1);
    }

    #[test]
    fn test_cleanup_branch_without_worktree_deletes_branch() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        Branch::create(temp.path(), "feature/no-worktree", "HEAD").unwrap();
        assert!(Branch::exists(temp.path(), "feature/no-worktree").unwrap());

        manager
            .cleanup_branch("feature/no-worktree", false, true)
            .unwrap();

        assert!(!Branch::exists(temp.path(), "feature/no-worktree").unwrap());
        let worktrees = manager.list().unwrap();
        assert_eq!(worktrees.len(), 1);
    }

    #[test]
    fn test_cleanup_branch_with_missing_worktree_path() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let wt = manager
            .create_new_branch("feature/missing-worktree", None)
            .unwrap();
        let wt_path = wt.path.clone();

        std::fs::remove_dir_all(&wt_path).unwrap();
        assert!(Branch::exists(temp.path(), "feature/missing-worktree").unwrap());

        manager
            .cleanup_branch("feature/missing-worktree", false, true)
            .unwrap();

        assert!(!Branch::exists(temp.path(), "feature/missing-worktree").unwrap());
        let worktrees = manager.list().unwrap();
        assert_eq!(worktrees.len(), 1);
    }

    #[test]
    fn test_active_count() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        // Main worktree exists - it counts as 1 because is_main is false for regular repos
        // (is_main/is_bare is only true for bare repositories)
        let initial_count = manager.active_count().unwrap();

        manager.create_new_branch("feature/count", None).unwrap();
        let count = manager.active_count().unwrap();
        assert_eq!(count, initial_count + 1);
    }

    #[test]
    fn test_auto_cleanup_orphans_disabled() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let wt = manager.create_new_branch("feature/orphan", None).unwrap();
        let wt_path = wt.path.clone();
        assert!(wt_path.exists());

        std::fs::remove_dir_all(&wt_path).unwrap();

        let detected = manager.detect_orphans();
        assert!(!detected.is_empty());

        let cleaned = manager.auto_cleanup_orphans().unwrap();
        assert_eq!(cleaned, 0);

        let remaining = manager.detect_orphans();
        assert!(!remaining.is_empty());
    }

    #[test]
    fn test_stale_worktree_recovery_disabled_fr039() {
        // FR-039: Auto-recovery disabled -> existing paths must error
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let branch = "feature/stale";
        let wt_path = WorktreePath::generate(temp.path(), branch);
        std::fs::create_dir_all(&wt_path).unwrap();
        std::fs::write(wt_path.join(".git"), "stale worktree").unwrap();

        // Ensure it's NOT in worktree list
        let output = crate::process::command("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        let list_output = String::from_utf8_lossy(&output.stdout);
        assert!(!list_output.contains(branch));

        // Now try to create a new worktree at the same path - should fail
        let result = manager.create_new_branch(branch, None);
        assert!(matches!(result, Err(GwtError::WorktreePathConflict { .. })));
    }

    #[test]
    fn test_existing_path_conflict_fr040() {
        // FR-040: Existing path must error (auto-recovery disabled)
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        // Calculate where the worktree would be created
        let wt_path = WorktreePath::generate(temp.path(), "feature/conflict");

        // Create a directory without .git (simulating user data)
        std::fs::create_dir_all(&wt_path).unwrap();
        std::fs::write(wt_path.join("user_data.txt"), "important file").unwrap();

        // Try to create worktree - should fail with WorktreePathConflict
        let result = manager.create_new_branch("feature/conflict", None);
        assert!(result.is_err());
        if let Err(GwtError::WorktreePathConflict { path }) = result {
            assert_eq!(path, wt_path);
        } else {
            panic!("Expected WorktreePathConflict error");
        }
    }

    #[test]
    fn test_existing_worktree_conflict() {
        // If the branch already has a worktree, create_for_branch should be idempotent.
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        // Create a worktree
        let wt = manager.create_new_branch("feature/exists", None).unwrap();
        assert!(wt.path.exists());

        // Re-create for the same branch: should return the existing worktree.
        let wt2 = manager.create_for_branch("feature/exists").unwrap();
        assert_eq!(
            canonicalize_or_self(&wt2.path),
            canonicalize_or_self(&wt.path)
        );
    }

    /// Create a bare test repository (SPEC-a70a1ece T406)
    /// Returns (TempDir, PathBuf, String) where:
    /// - PathBuf is the bare repo path
    /// - String is the default branch name (main/master depending on git config)
    fn create_bare_test_repo() -> (TempDir, PathBuf, String) {
        let temp = TempDir::new().unwrap();
        // Create a source repo first
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();

        run_git_in(&source, &["init"]);
        run_git_in(&source, &["config", "user.email", "test@test.com"]);
        run_git_in(&source, &["config", "user.name", "Test"]);
        std::fs::write(source.join("test.txt"), "hello").unwrap();
        run_git_in(&source, &["add", "."]);
        run_git_in(&source, &["commit", "-m", "initial"]);
        let base_branch = git_stdout(&source, &["rev-parse", "--abbrev-ref", "HEAD"]);

        // Clone as bare
        let bare = temp.path().join("repo.git");
        let output = crate::process::command("git")
            .args([
                "clone",
                "--bare",
                source.to_str().unwrap(),
                bare.to_str().unwrap(),
            ])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Failed to create bare clone: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        (temp, bare, base_branch)
    }

    #[test]
    fn test_bare_repo_uses_sibling_location() {
        // SPEC-a70a1ece T406: Bare repository should use Sibling location
        let (_temp, bare_path, _base_branch) = create_bare_test_repo();

        let manager = WorktreeManager::new(&bare_path).unwrap();
        assert_eq!(manager.location, WorktreeLocation::Sibling);
    }

    #[test]
    fn test_bare_repo_worktree_sibling_path() {
        // SPEC-a70a1ece T406: Worktree should be created as sibling to bare repo
        let (temp, bare_path, base_branch) = create_bare_test_repo();

        let manager = WorktreeManager::new(&bare_path).unwrap();
        let wt = manager
            .create_new_branch("feature/test", Some(&base_branch))
            .unwrap();

        // Worktree should be at sibling path: /temp/feature/test
        let expected_path = temp.path().join("feature/test");
        assert_eq!(
            canonicalize_or_self(&wt.path),
            canonicalize_or_self(&expected_path)
        );
        assert!(wt.path.exists());
    }

    #[test]
    fn test_bare_repo_worktree_creates_subdirectory_structure() {
        // SPEC-a70a1ece FR-152: Slash-containing branches create subdirectory structure
        // e.g., "feature/branch-name" creates feature/branch-name/ directory
        let (temp, bare_path, base_branch) = create_bare_test_repo();

        let manager = WorktreeManager::new(&bare_path).unwrap();
        let wt = manager
            .create_new_branch("feature/my-feature", Some(&base_branch))
            .unwrap();

        // Verify worktree is at /temp/feature/my-feature
        let expected_path = temp.path().join("feature").join("my-feature");
        assert_eq!(
            canonicalize_or_self(&wt.path),
            canonicalize_or_self(&expected_path)
        );

        // Verify the feature/ subdirectory exists
        let feature_dir = temp.path().join("feature");
        assert!(
            feature_dir.exists(),
            "Parent directory 'feature/' should exist at {:?}",
            feature_dir
        );
        assert!(
            feature_dir.is_dir(),
            "'feature/' should be a directory, not a file"
        );

        // Verify the worktree directory exists inside feature/
        assert!(
            wt.path.exists(),
            "Worktree path should exist at {:?}",
            wt.path
        );
        assert!(wt.path.is_dir(), "Worktree should be a directory");

        // Verify worktree is NOT created flat at bare repo level
        // i.e., /temp/feature-my-feature should NOT exist
        let flat_path = temp.path().join("feature-my-feature");
        assert!(
            !flat_path.exists(),
            "Worktree should NOT be created flat at {:?}",
            flat_path
        );

        // Verify *.git directory still exists at expected location
        assert!(bare_path.exists(), "Bare repo should still exist");
    }

    #[test]
    fn test_bare_repo_create_for_branch_recovers_missing_registered_worktree_path() {
        let (_temp, bare_path, base_branch) = create_bare_test_repo();
        let manager = WorktreeManager::new(&bare_path).unwrap();

        let branch = "feature/auto-merge";
        Branch::create(&bare_path, branch, &base_branch).unwrap();

        let wt_path =
            WorktreePath::generate_with_location(&bare_path, branch, WorktreeLocation::Sibling);
        run_git_in(
            &bare_path,
            &[
                "worktree",
                "add",
                "--detach",
                wt_path.to_str().unwrap(),
                "HEAD",
            ],
        );
        std::fs::remove_dir_all(&wt_path).unwrap();

        let wt = manager.create_for_branch(branch).unwrap();
        assert_eq!(wt.branch.as_deref(), Some(branch));
        assert!(wt.path.exists());
    }

    #[test]
    fn test_bare_repo_worktree_bugfix_branch() {
        // SPEC-a70a1ece FR-152: Test bugfix/ prefix as well
        let (temp, bare_path, base_branch) = create_bare_test_repo();

        let manager = WorktreeManager::new(&bare_path).unwrap();
        let wt = manager
            .create_new_branch("bugfix/fix-123", Some(&base_branch))
            .unwrap();

        // Verify worktree is at /temp/bugfix/fix-123
        let expected_path = temp.path().join("bugfix").join("fix-123");
        assert_eq!(
            canonicalize_or_self(&wt.path),
            canonicalize_or_self(&expected_path)
        );

        // Verify the bugfix/ subdirectory exists
        let bugfix_dir = temp.path().join("bugfix");
        assert!(
            bugfix_dir.exists(),
            "Parent directory 'bugfix/' should exist"
        );
        assert!(bugfix_dir.is_dir(), "'bugfix/' should be a directory");
    }

    #[test]
    fn test_bare_repo_create_new_branch_from_remote_like_base_without_tracking_ref() {
        let (_temp, bare_path, base_branch) = create_bare_test_repo();
        let manager = WorktreeManager::new(&bare_path).unwrap();
        let remote_like_base = format!("origin/{}", base_branch);

        let has_remote_tracking_ref = crate::process::git_command()
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/origin/{}", base_branch),
            ])
            .current_dir(&bare_path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(
            !has_remote_tracking_ref,
            "bare clones should not require refs/remotes/origin/* to resolve base branches"
        );

        let expected = git_stdout(&bare_path, &["rev-parse", &base_branch]);
        let wt = manager
            .create_new_branch("feature/from-origin-base", Some(&remote_like_base))
            .unwrap();
        let actual = git_stdout(&wt.path, &["rev-parse", "HEAD"]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_normal_repo_uses_subdir_location() {
        // Non-bare repository should use Subdir location (default)
        let temp = create_test_repo();

        let manager = WorktreeManager::new(temp.path()).unwrap();
        assert_eq!(manager.location, WorktreeLocation::Subdir);
    }
}
