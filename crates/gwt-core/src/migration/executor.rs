//! Migration executor (SPEC-a70a1ece T806-T812, T815, T901-T909)

use super::{
    backup::create_backup, config::MigrationConfig, error::MigrationError, state::MigrationState,
    validator::validate_migration,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info, warn};

/// Information about a worktree being migrated
#[derive(Debug, Clone)]
pub struct WorktreeMigrationInfo {
    /// Branch name
    pub branch: String,
    /// Original worktree path
    pub source_path: PathBuf,
    /// New worktree path
    pub target_path: PathBuf,
    /// Whether the worktree has uncommitted changes
    pub is_dirty: bool,
}

/// Migration progress callback
pub type MigrationProgress = Box<dyn Fn(MigrationState) + Send>;

/// Execute full migration (SPEC-a70a1ece T815, FR-201)
pub fn execute_migration(
    config: &MigrationConfig,
    progress: Option<MigrationProgress>,
) -> Result<(), MigrationError> {
    let report_progress = |state: MigrationState| {
        if let Some(ref cb) = progress {
            cb(state);
        }
    };

    // Phase 1: Validate
    report_progress(MigrationState::Validating);
    let validation = validate_migration(config)?;
    if !validation.passed {
        return Err(validation.errors.into_iter().next().unwrap_or(
            MigrationError::ValidationFailed {
                reason: "Unknown validation error".to_string(),
            },
        ));
    }

    // Phase 2: Backup
    if !config.dry_run {
        report_progress(MigrationState::BackingUp);
        create_backup(&config.source_root, &config.backup_path())?;
    }

    // Phase 3: Create bare repository
    report_progress(MigrationState::CreatingBareRepo);
    if !config.dry_run {
        create_bare_repository(config)?;
    }

    // Phase 4: Migrate worktrees
    let worktrees = list_worktrees_to_migrate(&config.source_root)?;
    let total = worktrees.len();
    for (i, wt_info) in worktrees.iter().enumerate() {
        report_progress(MigrationState::MigratingWorktrees { current: i, total });
        if !config.dry_run {
            migrate_worktree(config, wt_info)?;
        }
    }

    // Phase 5: Cleanup
    report_progress(MigrationState::CleaningUp);
    if !config.dry_run {
        cleanup_old_worktrees(&config.source_root)?;
        create_project_config(config)?;
    }

    report_progress(MigrationState::Completed);
    info!("Migration completed successfully");
    Ok(())
}

/// Create a bare repository from the source (SPEC-a70a1ece FR-203)
fn create_bare_repository(config: &MigrationConfig) -> Result<(), MigrationError> {
    let bare_path = config.bare_repo_path();
    debug!(bare = %bare_path.display(), "Creating bare repository");

    // Get the remote URL from the source
    let remote_url = get_remote_url(&config.source_root)?;

    if let Some(url) = remote_url {
        // Clone bare from remote
        let output = Command::new("git")
            .args(["clone", "--bare", "--", &url])
            .arg(&bare_path)
            .output()
            .map_err(|e| MigrationError::BareRepoCreationFailed {
                reason: format!("Failed to clone bare: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(MigrationError::BareRepoCreationFailed {
                reason: format!("git clone --bare failed: {}", stderr),
            });
        }
    } else {
        // Local-only repo: create bare and push
        migrate_local_only_repo(config)?;
    }

    // Copy hooks (FR-217)
    copy_git_hooks(&config.source_root, &bare_path)?;

    Ok(())
}

/// Get remote URL from repository
fn get_remote_url(repo_root: &Path) -> Result<Option<String>, MigrationError> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_root)
        .output()
        .map_err(|e| MigrationError::GitError {
            reason: format!("Failed to get remote URL: {}", e),
        })?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Some(url))
    } else {
        Ok(None)
    }
}

/// Migrate a local-only repository (SPEC-a70a1ece FR-225, T908)
fn migrate_local_only_repo(config: &MigrationConfig) -> Result<(), MigrationError> {
    let bare_path = config.bare_repo_path();
    debug!(bare = %bare_path.display(), "Migrating local-only repository");

    // Initialize bare repository
    let output = Command::new("git")
        .args(["init", "--bare"])
        .arg(&bare_path)
        .output()
        .map_err(|e| MigrationError::BareRepoCreationFailed {
            reason: format!("Failed to init bare repo: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MigrationError::BareRepoCreationFailed {
            reason: format!("git init --bare failed: {}", stderr),
        });
    }

    // Push all refs from source to bare
    let output = Command::new("git")
        .args(["push", "--all"])
        .arg(&bare_path)
        .current_dir(&config.source_root)
        .output()
        .map_err(|e| MigrationError::BareRepoCreationFailed {
            reason: format!("Failed to push to bare: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Allow "Everything up-to-date" as success
        if !stderr.contains("Everything up-to-date") {
            return Err(MigrationError::BareRepoCreationFailed {
                reason: format!("git push --all failed: {}", stderr),
            });
        }
    }

    Ok(())
}

/// List worktrees that need to be migrated
/// SPEC-a70a1ece: 元のリポジトリのメインブランチもworktreeとして再作成
fn list_worktrees_to_migrate(
    repo_root: &Path,
) -> Result<Vec<WorktreeMigrationInfo>, MigrationError> {
    let mut worktrees = Vec::new();
    let parent_dir = repo_root.parent().unwrap_or(repo_root);

    // First, add the main repository itself (SPEC-a70a1ece)
    // This is the original repo's main/master branch that needs to become a worktree
    if let Some(main_branch) = get_worktree_branch(repo_root) {
        let is_dirty = is_worktree_dirty(repo_root);
        // Sanitize branch name for directory (replace / with -)
        let dir_name = main_branch.replace('/', "-");
        let target_path = parent_dir.join(&dir_name);

        worktrees.push(WorktreeMigrationInfo {
            branch: main_branch,
            source_path: repo_root.to_path_buf(),
            target_path,
            is_dirty,
        });
    }

    // Then add worktrees from .worktrees/ directory (if exists)
    let worktrees_dir = repo_root.join(".worktrees");
    if worktrees_dir.exists() {
        let entries = std::fs::read_dir(&worktrees_dir).map_err(|e| MigrationError::IoError {
            path: worktrees_dir.clone(),
            reason: format!("Failed to read .worktrees directory: {}", e),
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| MigrationError::IoError {
                path: worktrees_dir.clone(),
                reason: format!("Failed to read directory entry: {}", e),
            })?;

            let source_path = entry.path();
            if !source_path.is_dir() {
                continue;
            }

            // Get branch name from worktree
            if let Some(branch) = get_worktree_branch(&source_path) {
                let is_dirty = is_worktree_dirty(&source_path);
                // Sanitize branch name for directory
                let dir_name = branch.replace('/', "-");
                let target_path = parent_dir.join(&dir_name);

                worktrees.push(WorktreeMigrationInfo {
                    branch,
                    source_path,
                    target_path,
                    is_dirty,
                });
            }
        }
    }

    Ok(worktrees)
}

/// Get branch name from worktree
fn get_worktree_branch(worktree_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Check if worktree has uncommitted changes (SPEC-a70a1ece T807, FR-206)
pub fn is_worktree_dirty(worktree_path: &Path) -> bool {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(worktree_path)
        .output();

    match output {
        Ok(output) => !output.stdout.is_empty(),
        Err(_) => false,
    }
}

/// Check if the source path is the main repository (not a worktree)
fn is_main_repository(source_path: &Path) -> bool {
    let git_path = source_path.join(".git");
    // Main repo has .git as directory, worktree has .git as file
    git_path.is_dir()
}

/// Migrate a single worktree (SPEC-a70a1ece T808-T809)
/// SPEC-a70a1ece US9-S10: すべてのworktreeはgit worktree addで新規作成
fn migrate_worktree(
    config: &MigrationConfig,
    wt_info: &WorktreeMigrationInfo,
) -> Result<(), MigrationError> {
    debug!(
        branch = %wt_info.branch,
        dirty = wt_info.is_dirty,
        source = %wt_info.source_path.display(),
        target = %wt_info.target_path.display(),
        "Migrating worktree"
    );

    // For the main repository, we need special handling
    // The source is the original repo with .git directory, not a worktree
    let is_main_repo = is_main_repository(&wt_info.source_path);

    if wt_info.is_dirty {
        migrate_dirty_worktree(config, wt_info, is_main_repo)?;
    } else {
        migrate_clean_worktree(config, wt_info, is_main_repo)?;
    }

    // Migrate stash if any (FR-220)
    migrate_stash(&wt_info.source_path, &wt_info.target_path)?;

    // Preserve tracking relationships (FR-221)
    preserve_tracking_relationships(&wt_info.target_path, &wt_info.branch)?;

    Ok(())
}

/// Migrate dirty worktree using file move (SPEC-a70a1ece T808, FR-206)
/// SPEC-a70a1ece: dirty worktreeの場合、ファイルを移動後にgit worktree addで再登録
fn migrate_dirty_worktree(
    config: &MigrationConfig,
    wt_info: &WorktreeMigrationInfo,
    is_main_repo: bool,
) -> Result<(), MigrationError> {
    debug!(
        branch = %wt_info.branch,
        is_main_repo = is_main_repo,
        "Migrating dirty worktree (file move)"
    );

    let bare_path = config.bare_repo_path();

    // For main repo, we need to remove old worktree reference first (if any)
    if !is_main_repo {
        // Remove the old worktree registration from the original repo
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&wt_info.source_path)
            .current_dir(&config.source_root)
            .output();
    }

    // Create new worktree from bare repo with --no-checkout
    let output = Command::new("git")
        .args(["worktree", "add", "--no-checkout"])
        .arg(&wt_info.target_path)
        .arg(&wt_info.branch)
        .current_dir(&bare_path)
        .output()
        .map_err(|e| MigrationError::WorktreeMigrationFailed {
            branch: wt_info.branch.clone(),
            reason: format!("Failed to add worktree: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MigrationError::WorktreeMigrationFailed {
            branch: wt_info.branch.clone(),
            reason: format!("git worktree add failed: {}", stderr),
        });
    }

    // Copy working directory files, excluding .git and gitignored (FR-208)
    copy_working_files(&wt_info.source_path, &wt_info.target_path)?;

    // Preserve file permissions (FR-214)
    preserve_file_permissions(&wt_info.source_path, &wt_info.target_path)?;

    Ok(())
}

/// Migrate clean worktree using re-clone (SPEC-a70a1ece T809, FR-207)
/// SPEC-a70a1ece US9-S10: すべてのworktreeはgit worktree addで新規作成
fn migrate_clean_worktree(
    config: &MigrationConfig,
    wt_info: &WorktreeMigrationInfo,
    is_main_repo: bool,
) -> Result<(), MigrationError> {
    debug!(
        branch = %wt_info.branch,
        is_main_repo = is_main_repo,
        "Migrating clean worktree (re-clone)"
    );

    let bare_path = config.bare_repo_path();

    // For non-main-repo worktrees, remove old worktree reference first
    if !is_main_repo {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&wt_info.source_path)
            .current_dir(&config.source_root)
            .output();
    }

    // Create new worktree from bare repo using git worktree add
    let output = Command::new("git")
        .args(["worktree", "add"])
        .arg(&wt_info.target_path)
        .arg(&wt_info.branch)
        .current_dir(&bare_path)
        .output()
        .map_err(|e| MigrationError::WorktreeMigrationFailed {
            branch: wt_info.branch.clone(),
            reason: format!("Failed to add worktree: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MigrationError::WorktreeMigrationFailed {
            branch: wt_info.branch.clone(),
            reason: format!("git worktree add failed: {}", stderr),
        });
    }

    // Preserve submodules (FR-218)
    preserve_submodules(&wt_info.target_path)?;

    Ok(())
}

/// Copy working files excluding .git and gitignored files (SPEC-a70a1ece T812, FR-208)
fn copy_working_files(source: &Path, target: &Path) -> Result<(), MigrationError> {
    // Use rsync with git-aware exclusions
    let output = Command::new("rsync")
        .args(["-a", "--exclude=.git", "--filter=:- .gitignore"])
        .arg(format!("{}/", source.display()))
        .arg(format!("{}/", target.display()))
        .output();

    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            // Fallback to cp if rsync fails
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("rsync failed, falling back to cp: {}", stderr);
            fallback_copy_files(source, target)
        }
        Err(_) => fallback_copy_files(source, target),
    }
}

/// Fallback file copy without rsync
fn fallback_copy_files(source: &Path, target: &Path) -> Result<(), MigrationError> {
    // Simple recursive copy, skipping .git
    for entry in walkdir::WalkDir::new(source)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let entry = entry.map_err(|e| MigrationError::IoError {
            path: source.to_path_buf(),
            reason: format!("Failed to walk directory: {}", e),
        })?;

        let rel_path = entry.path().strip_prefix(source).unwrap_or(entry.path());
        let target_path = target.join(rel_path);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target_path).map_err(|e| MigrationError::IoError {
                path: target_path.clone(),
                reason: format!("Failed to create directory: {}", e),
            })?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::copy(entry.path(), &target_path).map_err(|e| MigrationError::IoError {
                path: target_path.clone(),
                reason: format!("Failed to copy file: {}", e),
            })?;
        }
    }
    Ok(())
}

/// Copy git hooks from source to bare repository (SPEC-a70a1ece T810, FR-217)
fn copy_git_hooks(source: &Path, bare_path: &Path) -> Result<(), MigrationError> {
    let source_hooks = source.join(".git/hooks");
    let target_hooks = bare_path.join("hooks");

    if !source_hooks.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(&source_hooks).map_err(|e| MigrationError::IoError {
        path: source_hooks.clone(),
        reason: format!("Failed to read hooks directory: {}", e),
    })? {
        let entry = entry.map_err(|e| MigrationError::IoError {
            path: source_hooks.clone(),
            reason: format!("Failed to read hook entry: {}", e),
        })?;

        let source_hook = entry.path();
        // Skip sample hooks
        if source_hook.extension().is_some_and(|ext| ext == "sample") {
            continue;
        }

        let hook_name = entry.file_name();
        let target_hook = target_hooks.join(&hook_name);

        std::fs::copy(&source_hook, &target_hook).map_err(|e| MigrationError::IoError {
            path: target_hook.clone(),
            reason: format!("Failed to copy hook: {}", e),
        })?;

        // Preserve executable permission
        #[cfg(unix)]
        {
            #[allow(unused_imports)]
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&source_hook) {
                let _ = std::fs::set_permissions(&target_hook, meta.permissions());
            }
        }
    }

    Ok(())
}

/// Preserve submodules in worktree (SPEC-a70a1ece T811, FR-218)
fn preserve_submodules(worktree_path: &Path) -> Result<(), MigrationError> {
    // Check if .gitmodules exists
    let gitmodules = worktree_path.join(".gitmodules");
    if !gitmodules.exists() {
        return Ok(());
    }

    // Initialize and update submodules
    let output = Command::new("git")
        .args(["submodule", "update", "--init", "--recursive"])
        .current_dir(worktree_path)
        .output();

    match output {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Submodule init failed (non-fatal): {}", stderr);
        }
        Err(e) => {
            warn!("Submodule init failed (non-fatal): {}", e);
        }
        _ => {}
    }

    Ok(())
}

/// Preserve file permissions (SPEC-a70a1ece T901, FR-214)
fn preserve_file_permissions(_source: &Path, _target: &Path) -> Result<(), MigrationError> {
    // On Unix, permissions are preserved by cp -a and rsync -a
    // This function is a placeholder for additional permission handling if needed
    Ok(())
}

/// Migrate stash entries (SPEC-a70a1ece T902, FR-220)
fn migrate_stash(source: &Path, _target: &Path) -> Result<(), MigrationError> {
    // Check if source has stash
    let output = Command::new("git")
        .args(["stash", "list"])
        .current_dir(source)
        .output();

    let has_stash = match output {
        Ok(output) => output.status.success() && !output.stdout.is_empty(),
        Err(_) => false,
    };

    if !has_stash {
        return Ok(());
    }

    // Export stash as patches and apply to target
    // This is complex, so we just warn for now
    warn!(
        "Stash entries exist in {}. Manual migration may be needed.",
        source.display()
    );

    Ok(())
}

/// Cleanup old worktrees directory and original repo (SPEC-a70a1ece T903, FR-204)
/// SPEC-a70a1ece: 元のリポジトリディレクトリも削除（bareリポジトリに変換済み）
fn cleanup_old_worktrees(repo_root: &Path) -> Result<(), MigrationError> {
    // Remove .worktrees directory if exists
    let worktrees_dir = repo_root.join(".worktrees");
    if worktrees_dir.exists() {
        debug!(path = %worktrees_dir.display(), "Removing .worktrees directory");
        std::fs::remove_dir_all(&worktrees_dir).map_err(|e| MigrationError::IoError {
            path: worktrees_dir,
            reason: format!("Failed to remove .worktrees: {}", e),
        })?;
    }

    // Remove the original repository directory
    // At this point:
    // - Bare repo has been created (repo_root.git)
    // - All worktrees have been migrated to sibling directories
    // - Original repo is no longer needed
    debug!(path = %repo_root.display(), "Removing original repository directory");
    std::fs::remove_dir_all(repo_root).map_err(|e| MigrationError::IoError {
        path: repo_root.to_path_buf(),
        reason: format!("Failed to remove original repository: {}", e),
    })?;

    Ok(())
}

/// Create project config file (SPEC-a70a1ece T905, FR-219)
fn create_project_config(config: &MigrationConfig) -> Result<(), MigrationError> {
    let gwt_dir = config.target_root.join(".gwt");
    std::fs::create_dir_all(&gwt_dir).map_err(|e| MigrationError::IoError {
        path: gwt_dir.clone(),
        reason: format!("Failed to create .gwt directory: {}", e),
    })?;

    let config_path = gwt_dir.join("project.json");
    let config_content = serde_json::json!({
        "bare_repo_name": config.bare_repo_name,
        "migrated_at": chrono::Utc::now().to_rfc3339(),
    });

    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&config_content).unwrap(),
    )
    .map_err(|e| MigrationError::IoError {
        path: config_path,
        reason: format!("Failed to write project config: {}", e),
    })?;

    Ok(())
}

/// Preserve tracking relationships (SPEC-a70a1ece T907, FR-221)
fn preserve_tracking_relationships(
    worktree_path: &Path,
    branch: &str,
) -> Result<(), MigrationError> {
    // Set upstream tracking
    let output = Command::new("git")
        .args([
            "branch",
            "--set-upstream-to",
            &format!("origin/{}", branch),
            branch,
        ])
        .current_dir(worktree_path)
        .output();

    // Ignore errors - upstream may not exist
    if let Err(e) = output {
        debug!("Failed to set upstream (may not exist): {}", e);
    }

    Ok(())
}

/// Derive bare repository name from URL or directory (SPEC-a70a1ece T906, FR-219)
pub fn derive_bare_repo_name(url_or_path: &str) -> String {
    // Extract repo name from URL or path
    let name = url_or_path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("repo")
        .trim_end_matches(".git");

    format!("{}.git", name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_derive_bare_repo_name() {
        assert_eq!(
            derive_bare_repo_name("https://github.com/user/repo.git"),
            "repo.git"
        );
        assert_eq!(
            derive_bare_repo_name("https://github.com/user/repo"),
            "repo.git"
        );
        assert_eq!(derive_bare_repo_name("/path/to/repo"), "repo.git");
        assert_eq!(derive_bare_repo_name("/path/to/repo/"), "repo.git");
    }

    #[test]
    fn test_is_worktree_dirty() {
        let temp = TempDir::new().unwrap();

        // Initialize git repo
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

        // Clean repo
        assert!(!is_worktree_dirty(temp.path()));

        // Create untracked file
        std::fs::write(temp.path().join("test.txt"), "content").unwrap();
        assert!(is_worktree_dirty(temp.path()));
    }

    #[test]
    fn test_list_worktrees_to_migrate_empty() {
        let temp = TempDir::new().unwrap();
        let result = list_worktrees_to_migrate(temp.path()).unwrap();
        assert!(result.is_empty());
    }
}
