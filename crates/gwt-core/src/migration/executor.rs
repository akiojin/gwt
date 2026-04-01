//! Migration executor (gwt-spec issue T806-T812, T815, T901-T909)

use std::{
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::{
    backup::create_backup, config::MigrationConfig, error::MigrationError, state::MigrationState,
    validator::validate_migration,
};
use crate::logging::{log_flow_start, log_flow_success};

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
    /// Whether this is the main repository (not a worktree)
    /// This must be determined before .git is deleted
    pub is_main_repo: bool,
}

/// Migration progress callback
pub type MigrationProgress = Box<dyn Fn(MigrationState) + Send>;

const EVACUATION_MANIFEST_FILENAME: &str = "evacuation-manifest.json";
const EVACUATION_MANIFEST_ENCODING: &str = "base64-os";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvacuationManifest {
    entries: Vec<String>,
    #[serde(default)]
    encoding: Option<String>,
}

/// Execute full migration (gwt-spec issue T815, FR-201)
/// gwt-spec issue FR-150: Migration creates structure INSIDE the original repo directory
pub fn execute_migration(
    config: &MigrationConfig,
    progress: Option<MigrationProgress>,
) -> Result<(), MigrationError> {
    log_flow_start("migration", "execute_migration");
    let report_progress = |state: MigrationState| {
        if let Some(ref cb) = progress {
            cb(state);
        }
    };

    // Phase 1: Validate
    info!("Phase 1: Validating migration config");
    report_progress(MigrationState::Validating);
    let validation = validate_migration(config)?;
    info!("Phase 1 complete: Validation passed");
    if !validation.passed {
        let err =
            validation
                .errors
                .into_iter()
                .next()
                .unwrap_or(MigrationError::ValidationFailed {
                    reason: "Unknown validation error".to_string(),
                });
        crate::logging::log_incident(
            "migration",
            "execute_migration",
            Some("MIGRATION_VALIDATION_FAILED"),
            &err.to_string(),
        );
        return Err(err);
    }

    // Phase 2: Backup
    info!("Phase 2: Creating backup");
    if !config.dry_run {
        report_progress(MigrationState::BackingUp);
        create_backup(&config.source_root, &config.backup_path())?;
    }
    info!("Phase 2 complete: Backup created");

    // Phase 3: Collect worktree info and prepare for migration
    info!("Phase 3: Collecting worktree info");
    let worktrees = list_worktrees_to_migrate(config)?;
    let total = worktrees.len();
    info!("Phase 3 complete: Found {} worktrees to migrate", total);
    for wt in &worktrees {
        info!(
            "  - branch={}, dirty={}, source={}",
            wt.branch,
            wt.is_dirty,
            wt.source_path.display()
        );
    }

    // Phase 4: Evacuate main repo files to temp directory (for dirty main worktree)
    info!("Phase 4: Checking if main repo files need evacuation");
    let temp_evacuation_dir = config.evacuation_temp_path();
    if !config.dry_run {
        // Find main worktree (the original repo itself) using pre-computed is_main_repo
        if let Some(main_wt) = worktrees.iter().find(|wt| wt.is_main_repo) {
            if main_wt.is_dirty {
                info!("Main repo is dirty, evacuating files to temp directory");
                evacuate_main_repo_files(&config.source_root, &temp_evacuation_dir)?;
                info!("Phase 4 complete: Files evacuated");
            } else {
                info!("Phase 4 complete: Main repo is clean, no evacuation needed");
            }
        } else {
            info!("Phase 4 complete: No main repo found");
        }
    }

    // Phase 5: Create bare repository
    info!(
        "Phase 5: Creating bare repository at {}",
        config.bare_repo_path().display()
    );
    report_progress(MigrationState::CreatingBareRepo);
    if !config.dry_run {
        create_bare_repository(config)?;
    }
    info!("Phase 5 complete: Bare repository created");

    // Phase 6: Cleanup original .git directory BEFORE creating worktrees
    // This is necessary because worktrees will be created in the same directory
    info!("Phase 6: Cleaning up original .git directory");
    if !config.dry_run {
        cleanup_original_git_dir(config)?;
    }
    info!("Phase 6 complete: Original .git directory removed");

    // Phase 7: Migrate worktrees
    info!("Phase 7: Migrating {} worktrees", total);
    for (i, wt_info) in worktrees.iter().enumerate() {
        info!(
            "Migrating worktree {}/{}: branch={}, source={}, target={}",
            i + 1,
            total,
            wt_info.branch,
            wt_info.source_path.display(),
            wt_info.target_path.display()
        );
        report_progress(MigrationState::MigratingWorktrees { current: i, total });
        if !config.dry_run {
            migrate_worktree(config, wt_info)?;
        }
        info!("Completed worktree {}/{}: {}", i + 1, total, wt_info.branch);
    }

    // Phase 8: Restore evacuated files to main worktree (if dirty)
    info!("Phase 8: Restoring evacuated files (if any)");
    if !config.dry_run && temp_evacuation_dir.exists() {
        // Find main worktree using pre-computed is_main_repo
        if let Some(main_wt) = worktrees.iter().find(|wt| wt.is_main_repo) {
            info!(
                "Restoring evacuated files to main worktree: {}",
                main_wt.target_path.display()
            );
            restore_evacuated_files(&temp_evacuation_dir, &main_wt.target_path)?;

            // Reset index to HEAD so files appear as unstaged changes, not deleted
            // This is needed because --no-checkout leaves the index empty
            info!("Resetting index to HEAD in main worktree");
            let _ = crate::process::command("git")
                .args(["reset"])
                .current_dir(&main_wt.target_path)
                .output();

            // Clean up root directory files (they've been moved to main worktree)
            info!("Cleaning up root directory files");
            cleanup_root_files(config, &worktrees)?;
        }
    }
    info!("Phase 8 complete");

    // Phase 9: Cleanup old .worktrees/ directory
    info!("Phase 9: Cleaning up old .worktrees/ directory");
    report_progress(MigrationState::CleaningUp);
    if !config.dry_run {
        cleanup_old_worktrees_dir(config)?;
        create_project_config(config)?;

        // Keep evacuation temp data until all migration phases succeed.
        if temp_evacuation_dir.exists() {
            info!("Phase 9: Removing temp evacuation directory");
            if let Err(e) = std::fs::remove_dir_all(&temp_evacuation_dir) {
                warn!(
                    temp = %temp_evacuation_dir.display(),
                    error = %e,
                    "Failed to remove temp evacuation directory after successful migration"
                );
            }
        }
    }
    info!("Phase 9 complete: Migration cleanup done");

    report_progress(MigrationState::Completed);
    info!("Migration completed successfully");
    log_flow_success("migration", "execute_migration");
    Ok(())
}

/// Evacuate main repo files (excluding .git, .worktrees, and the bare repo) to temp directory
fn evacuate_main_repo_files(source: &Path, temp_dir: &Path) -> Result<(), MigrationError> {
    if temp_dir.exists() {
        recover_stale_evacuation_data(source, temp_dir)?;
    }

    std::fs::create_dir_all(temp_dir).map_err(|e| MigrationError::IoError {
        path: temp_dir.to_path_buf(),
        reason: format!("Failed to create temp directory: {}", e),
    })?;

    let mut evacuated_entries = Vec::new();

    for entry in std::fs::read_dir(source).map_err(|e| MigrationError::IoError {
        path: source.to_path_buf(),
        reason: format!("Failed to read source directory: {}", e),
    })? {
        let entry = entry.map_err(|e| MigrationError::IoError {
            path: source.to_path_buf(),
            reason: format!("Failed to read directory entry: {}", e),
        })?;

        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip .git, .worktrees, and .gwt-* directories
        if name_str == ".git"
            || name_str == ".worktrees"
            || name_str == ".gwt"
            || name_str.starts_with(".gwt-")
            || name_str.ends_with(".git")
        {
            continue;
        }

        let src_path = entry.path();
        let dst_path = temp_dir.join(&name);
        move_path(&src_path, &dst_path, "evacuate path")?;
        evacuated_entries.push(encode_entry_name(&name));
    }

    write_evacuation_manifest(temp_dir, &evacuated_entries)?;

    Ok(())
}

/// Restore evacuated files to the target worktree
fn restore_evacuated_files(temp_dir: &Path, target: &Path) -> Result<(), MigrationError> {
    for entry_name in collect_evacuation_entries(temp_dir)? {
        ensure_safe_top_level_entry(entry_name.as_os_str(), "restore evacuated path entry")?;
        let src_path = temp_dir.join(&entry_name);
        if !src_path.exists() {
            continue;
        }

        let dst_path = target.join(&entry_name);
        copy_path(&src_path, &dst_path, "restore evacuated path")?;
    }

    Ok(())
}

fn evacuation_manifest_path(temp_dir: &Path) -> PathBuf {
    temp_dir.join(EVACUATION_MANIFEST_FILENAME)
}

fn write_evacuation_manifest(temp_dir: &Path, entries: &[String]) -> Result<(), MigrationError> {
    let manifest_path = evacuation_manifest_path(temp_dir);
    let manifest = EvacuationManifest {
        entries: entries.to_vec(),
        encoding: Some(EVACUATION_MANIFEST_ENCODING.to_string()),
    };
    let serialized =
        serde_json::to_string_pretty(&manifest).map_err(|e| MigrationError::IoError {
            path: manifest_path.clone(),
            reason: format!("Failed to serialize evacuation manifest: {}", e),
        })?;

    std::fs::write(&manifest_path, serialized).map_err(|e| MigrationError::IoError {
        path: manifest_path,
        reason: format!("Failed to write evacuation manifest: {}", e),
    })?;

    Ok(())
}

fn collect_evacuation_entries(temp_dir: &Path) -> Result<Vec<OsString>, MigrationError> {
    let manifest_path = evacuation_manifest_path(temp_dir);
    if manifest_path.exists() {
        match std::fs::read_to_string(&manifest_path) {
            Ok(content) => match serde_json::from_str::<EvacuationManifest>(&content) {
                Ok(manifest) => {
                    if manifest.encoding.as_deref() == Some(EVACUATION_MANIFEST_ENCODING) {
                        let mut decoded = Vec::with_capacity(manifest.entries.len());
                        for encoded_name in manifest.entries {
                            match decode_entry_name(&encoded_name) {
                                Ok(name) => {
                                    if !is_safe_top_level_entry(name.as_os_str()) {
                                        warn!(
                                            path = %manifest_path.display(),
                                            entry = %encoded_name,
                                            "Unsafe evacuation manifest entry detected, falling back to directory scan"
                                        );
                                        return collect_evacuation_entries_from_scan(temp_dir);
                                    }
                                    decoded.push(name);
                                }
                                Err(err) => {
                                    warn!(
                                        path = %manifest_path.display(),
                                        entry = %encoded_name,
                                        error = %err,
                                        "Failed to decode evacuation manifest entry, falling back to directory scan"
                                    );
                                    return collect_evacuation_entries_from_scan(temp_dir);
                                }
                            }
                        }
                        return Ok(decoded);
                    }

                    warn!(
                        path = %manifest_path.display(),
                        encoding = ?manifest.encoding,
                        "Unsupported or missing evacuation manifest encoding, falling back to directory scan"
                    );
                    return collect_evacuation_entries_from_scan(temp_dir);
                }
                Err(err) => warn!(
                    path = %manifest_path.display(),
                    error = %err,
                    "Failed to parse evacuation manifest, falling back to directory scan"
                ),
            },
            Err(err) => warn!(
                path = %manifest_path.display(),
                error = %err,
                "Failed to read evacuation manifest, falling back to directory scan"
            ),
        }
    }

    collect_evacuation_entries_from_scan(temp_dir)
}

fn collect_evacuation_entries_from_scan(temp_dir: &Path) -> Result<Vec<OsString>, MigrationError> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(temp_dir).map_err(|e| MigrationError::IoError {
        path: temp_dir.to_path_buf(),
        reason: format!("Failed to read temp directory: {}", e),
    })? {
        let entry = entry.map_err(|e| MigrationError::IoError {
            path: temp_dir.to_path_buf(),
            reason: format!("Failed to read directory entry: {}", e),
        })?;
        let name = entry.file_name();
        if name == OsStr::new(EVACUATION_MANIFEST_FILENAME) {
            continue;
        }
        entries.push(name);
    }

    Ok(entries)
}

fn recover_stale_evacuation_data(source: &Path, temp_dir: &Path) -> Result<(), MigrationError> {
    info!(
        temp = %temp_dir.display(),
        source = %source.display(),
        "Recovering stale evacuation data before starting new migration"
    );

    for entry_name in collect_evacuation_entries(temp_dir)? {
        ensure_safe_top_level_entry(entry_name.as_os_str(), "stale evacuation entry")?;
        let src_path = temp_dir.join(&entry_name);
        if !src_path.exists() {
            continue;
        }

        let dst_path = source.join(&entry_name);
        if dst_path.exists() {
            return Err(MigrationError::IoError {
                path: dst_path,
                reason: "Stale evacuation entry conflicts with existing path".to_string(),
            });
        }

        move_path(&src_path, &dst_path, "recover stale evacuated path")?;
    }

    std::fs::remove_dir_all(temp_dir).map_err(|e| MigrationError::IoError {
        path: temp_dir.to_path_buf(),
        reason: format!("Failed to remove stale evacuation temp directory: {}", e),
    })?;

    Ok(())
}

fn encode_entry_name(name: &OsStr) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        BASE64_STANDARD.encode(name.as_bytes())
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut bytes = Vec::new();
        for unit in name.encode_wide() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        BASE64_STANDARD.encode(bytes)
    }

    #[cfg(not(any(unix, windows)))]
    {
        BASE64_STANDARD.encode(name.to_string_lossy().as_bytes())
    }
}

fn decode_entry_name(encoded: &str) -> Result<OsString, String> {
    let bytes = BASE64_STANDARD
        .decode(encoded)
        .map_err(|e| format!("Invalid base64 entry name: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(OsString::from_vec(bytes))
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        if bytes.len() % 2 != 0 {
            return Err("Invalid UTF-16 byte length".to_string());
        }
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        Ok(OsString::from_wide(&units))
    }

    #[cfg(not(any(unix, windows)))]
    {
        Ok(OsString::from(String::from_utf8_lossy(&bytes).to_string()))
    }
}

fn is_safe_top_level_entry(name: &OsStr) -> bool {
    let path = Path::new(name);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }

    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn ensure_safe_top_level_entry(name: &OsStr, context: &str) -> Result<(), MigrationError> {
    if is_safe_top_level_entry(name) {
        return Ok(());
    }

    Err(MigrationError::IoError {
        path: PathBuf::from(name),
        reason: format!("Invalid evacuation entry ({context})"),
    })
}

fn move_path(src: &Path, dst: &Path, action: &str) -> Result<(), MigrationError> {
    std::fs::rename(src, dst).map_err(|e| MigrationError::IoError {
        path: src.to_path_buf(),
        reason: format!(
            "Failed to {} from {} to {}: {}",
            action,
            src.display(),
            dst.display(),
            e
        ),
    })
}

fn copy_path(src: &Path, dst: &Path, action: &str) -> Result<(), MigrationError> {
    if dst.exists() {
        return Err(MigrationError::IoError {
            path: dst.to_path_buf(),
            reason: format!("Failed to {}: destination already exists", action),
        });
    }

    if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| MigrationError::IoError {
            path: dst.to_path_buf(),
            reason: format!("Failed to create directory while {}: {}", action, e),
        })?;

        for entry in std::fs::read_dir(src).map_err(|e| MigrationError::IoError {
            path: src.to_path_buf(),
            reason: format!("Failed to read directory while {}: {}", action, e),
        })? {
            let entry = entry.map_err(|e| MigrationError::IoError {
                path: src.to_path_buf(),
                reason: format!("Failed to read directory entry while {}: {}", action, e),
            })?;
            let child_src = entry.path();
            let child_dst = dst.join(entry.file_name());
            copy_path(&child_src, &child_dst, action)?;
        }

        return Ok(());
    }

    if src.is_file() {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MigrationError::IoError {
                path: parent.to_path_buf(),
                reason: format!("Failed to create parent directory while {}: {}", action, e),
            })?;
        }

        std::fs::copy(src, dst).map_err(|e| MigrationError::IoError {
            path: src.to_path_buf(),
            reason: format!(
                "Failed to {} from {} to {}: {}",
                action,
                src.display(),
                dst.display(),
                e
            ),
        })?;
        return Ok(());
    }

    Err(MigrationError::IoError {
        path: src.to_path_buf(),
        reason: format!("Failed to {}: unsupported file type", action),
    })
}

/// Cleanup original .git directory before creating worktrees
fn cleanup_original_git_dir(config: &MigrationConfig) -> Result<(), MigrationError> {
    let git_dir = config.source_root.join(".git");
    if git_dir.exists() && git_dir.is_dir() {
        debug!(path = %git_dir.display(), "Removing original .git directory");
        std::fs::remove_dir_all(&git_dir).map_err(|e| MigrationError::IoError {
            path: git_dir,
            reason: format!("Failed to remove .git directory: {}", e),
        })?;
    }
    Ok(())
}

/// Cleanup root directory files after migration
/// Files have been moved to main worktree, so we remove them from root
fn cleanup_root_files(
    config: &MigrationConfig,
    worktrees: &[WorktreeMigrationInfo],
) -> Result<(), MigrationError> {
    let root = &config.source_root;

    // Collect worktree directory names to skip
    let worktree_dirs: std::collections::HashSet<_> = worktrees
        .iter()
        .filter_map(|wt| {
            wt.target_path
                .strip_prefix(root)
                .ok()
                .and_then(|p| p.components().next())
                .map(|c| c.as_os_str().to_string_lossy().to_string())
        })
        .collect();

    for entry in std::fs::read_dir(root).map_err(|e| MigrationError::IoError {
        path: root.to_path_buf(),
        reason: format!("Failed to read root directory: {}", e),
    })? {
        let entry = entry.map_err(|e| MigrationError::IoError {
            path: root.to_path_buf(),
            reason: format!("Failed to read directory entry: {}", e),
        })?;

        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip directories that should remain:
        // - .git (already removed)
        // - .worktrees (will be cleaned up later)
        // - .gwt-* (migration temp/backup)
        // - *.git (bare repo)
        // - .gwt (config directory)
        // - worktree directories (main, develop, feature, etc.)
        if name_str == ".git"
            || name_str == ".worktrees"
            || name_str.starts_with(".gwt")
            || name_str.ends_with(".git")
            || worktree_dirs.contains(name_str.as_ref())
        {
            continue;
        }

        let path = entry.path();
        debug!(path = %path.display(), "Removing root file/directory");

        if path.is_dir() {
            let _ = remove_dir_all_for_cleanup(&path).map_err(|e| MigrationError::IoError {
                path: path.clone(),
                reason: format!("Failed to remove directory: {}", e),
            })?;
        } else {
            std::fs::remove_file(&path).map_err(|e| MigrationError::IoError {
                path: path.clone(),
                reason: format!("Failed to remove file: {}", e),
            })?;
        }
    }

    Ok(())
}

fn remove_dir_all_for_cleanup(path: &Path) -> std::io::Result<bool> {
    remove_dir_all_for_cleanup_with(path, |target| std::fs::remove_dir_all(target))
}

fn remove_dir_all_for_cleanup_with<F>(path: &Path, remove_dir_all: F) -> std::io::Result<bool>
where
    F: Fn(&Path) -> std::io::Result<()>,
{
    match remove_dir_all(path) {
        Ok(()) => Ok(true),
        Err(err) if is_directory_not_empty_error(&err) => {
            warn!(
                path = %path.display(),
                error = %err,
                "Skipping directory cleanup because it is still non-empty"
            );
            Ok(false)
        }
        Err(err) => Err(err),
    }
}

fn is_directory_not_empty_error(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::DirectoryNotEmpty
        || matches!(err.raw_os_error(), Some(39 | 66))
}

/// Cleanup old .worktrees/ directory (gwt-spec issue T903, FR-204)
fn cleanup_old_worktrees_dir(config: &MigrationConfig) -> Result<(), MigrationError> {
    let worktrees_dir = config.source_root.join(".worktrees");
    if worktrees_dir.exists() {
        debug!(path = %worktrees_dir.display(), "Removing .worktrees directory");
        std::fs::remove_dir_all(&worktrees_dir).map_err(|e| MigrationError::IoError {
            path: worktrees_dir,
            reason: format!("Failed to remove .worktrees: {}", e),
        })?;
    }
    Ok(())
}

/// Create a bare repository from the source (gwt-spec issue FR-203)
fn create_bare_repository(config: &MigrationConfig) -> Result<(), MigrationError> {
    let bare_path = config.bare_repo_path();
    debug!(bare = %bare_path.display(), "Creating bare repository");

    // Get the remote URL from the source
    let remote_url = get_remote_url(&config.source_root)?;

    if let Some(url) = remote_url {
        // Clone bare from remote
        let output = crate::process::command("git")
            .args(["clone", "--bare", "--", &url])
            .arg(&bare_path)
            .output()
            .map_err(|e| {
                crate::logging::log_incident(
                    "migration",
                    "create_bare_repository",
                    Some("MIGRATION_BARE_CLONE_SPAWN_FAILED"),
                    &e.to_string(),
                );
                MigrationError::BareRepoCreationFailed {
                    reason: format!("Failed to clone bare: {}", e),
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            crate::logging::log_incident(
                "migration",
                "create_bare_repository",
                Some("MIGRATION_BARE_CLONE_FAILED"),
                &stderr,
            );
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
    let output = crate::process::command("git")
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

/// Migrate a local-only repository (gwt-spec issue FR-225, T908)
fn migrate_local_only_repo(config: &MigrationConfig) -> Result<(), MigrationError> {
    let bare_path = config.bare_repo_path();
    debug!(bare = %bare_path.display(), "Migrating local-only repository");

    // Initialize bare repository
    let output = crate::process::command("git")
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
    let output = crate::process::command("git")
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
/// gwt-spec issue FR-150: 元のリポジトリディレクトリ内にworktreeを配置
fn list_worktrees_to_migrate(
    config: &MigrationConfig,
) -> Result<Vec<WorktreeMigrationInfo>, MigrationError> {
    let mut worktrees = Vec::new();
    let repo_root = &config.source_root;
    // gwt-spec issue FR-150: worktrees are placed inside target_root (same as source_root)
    let target_dir = &config.target_root;

    // First, add the main repository itself (gwt-spec issue)
    // This is the original repo's main/master branch that needs to become a worktree
    // IMPORTANT: Check is_main_repository NOW before .git is deleted
    if let Some(main_branch) = get_worktree_branch(repo_root) {
        let is_dirty = is_worktree_dirty(repo_root);
        let is_main = is_main_repository(repo_root);
        // Use branch name as directory (feature/test -> feature/test/)
        let target_path = target_dir.join(&main_branch);

        worktrees.push(WorktreeMigrationInfo {
            branch: main_branch,
            source_path: repo_root.to_path_buf(),
            target_path,
            is_dirty,
            is_main_repo: is_main,
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
            // IMPORTANT: Check is_main_repository NOW before .git is deleted
            if let Some(branch) = get_worktree_branch(&source_path) {
                let is_dirty = is_worktree_dirty(&source_path);
                let is_main = is_main_repository(&source_path);
                // Use branch name as directory (feature/test -> feature/test/)
                let target_path = target_dir.join(&branch);

                worktrees.push(WorktreeMigrationInfo {
                    branch,
                    source_path,
                    target_path,
                    is_dirty,
                    is_main_repo: is_main,
                });
            }
        }
    }

    Ok(worktrees)
}

/// Get branch name from worktree
fn get_worktree_branch(worktree_path: &Path) -> Option<String> {
    let output = crate::process::command("git")
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

/// Check if worktree has uncommitted changes (gwt-spec issue T807, FR-206)
pub fn is_worktree_dirty(worktree_path: &Path) -> bool {
    let output = crate::process::command("git")
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

/// Migrate a single worktree (gwt-spec issue T808-T809)
/// gwt-spec issue US9-S10: すべてのworktreeはgit worktree addで新規作成
fn migrate_worktree(
    config: &MigrationConfig,
    wt_info: &WorktreeMigrationInfo,
) -> Result<(), MigrationError> {
    info!(
        branch = %wt_info.branch,
        dirty = wt_info.is_dirty,
        is_main_repo = wt_info.is_main_repo,
        source = %wt_info.source_path.display(),
        target = %wt_info.target_path.display(),
        "migrate_worktree: Starting"
    );

    // Use pre-computed is_main_repo (determined before .git was deleted)
    let is_main_repo = wt_info.is_main_repo;

    if wt_info.is_dirty {
        info!("migrate_worktree: Calling migrate_dirty_worktree");
        migrate_dirty_worktree(config, wt_info, is_main_repo)?;
    } else {
        info!("migrate_worktree: Calling migrate_clean_worktree");
        migrate_clean_worktree(config, wt_info, is_main_repo)?;
    }
    info!("migrate_worktree: Worktree created");

    // Migrate stash if any (FR-220) - only for non-main repos since source may be gone
    if !is_main_repo {
        info!("migrate_worktree: Migrating stash");
        migrate_stash(&wt_info.source_path, &wt_info.target_path)?;
    }

    // Preserve tracking relationships (FR-221)
    info!("migrate_worktree: Preserving tracking relationships");
    preserve_tracking_relationships(&wt_info.target_path, &wt_info.branch)?;

    info!("migrate_worktree: Complete for branch={}", wt_info.branch);
    Ok(())
}

/// Migrate dirty worktree using file move (gwt-spec issue T808, FR-206)
/// gwt-spec issue: dirty worktreeの場合、ファイルを移動後にgit worktree addで再登録
fn migrate_dirty_worktree(
    config: &MigrationConfig,
    wt_info: &WorktreeMigrationInfo,
    is_main_repo: bool,
) -> Result<(), MigrationError> {
    info!(
        branch = %wt_info.branch,
        is_main_repo = is_main_repo,
        "migrate_dirty_worktree: Starting"
    );

    let bare_path = config.bare_repo_path();

    // For non-main-repo worktrees, remove old worktree reference first
    if !is_main_repo {
        // Remove the old worktree registration from the original repo
        // Note: This may fail if .git is already removed, but we ignore errors
        info!("migrate_dirty_worktree: Removing old worktree reference (may fail)");
        let _ = crate::process::command("git")
            .args(["worktree", "remove", "--force"])
            .arg(&wt_info.source_path)
            .current_dir(&config.source_root)
            .output();
    }

    // Create new worktree from bare repo with --no-checkout
    info!("migrate_dirty_worktree: Creating worktree with --no-checkout");
    let output = crate::process::command("git")
        .args(["worktree", "add", "--no-checkout"])
        .arg(&wt_info.target_path)
        .arg(&wt_info.branch)
        .current_dir(&bare_path)
        .output()
        .map_err(|e| {
            crate::logging::log_incident(
                "migration",
                "migrate_dirty_worktree",
                Some("MIGRATION_WORKTREE_ADD_SPAWN_FAILED"),
                &format!("branch={}: {}", wt_info.branch, e),
            );
            MigrationError::WorktreeMigrationFailed {
                branch: wt_info.branch.clone(),
                reason: format!("Failed to add worktree: {}", e),
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        crate::logging::log_incident(
            "migration",
            "migrate_dirty_worktree",
            Some("MIGRATION_WORKTREE_ADD_FAILED"),
            &format!("branch={}: {}", wt_info.branch, stderr),
        );
        return Err(MigrationError::WorktreeMigrationFailed {
            branch: wt_info.branch.clone(),
            reason: format!("git worktree add failed: {}", stderr),
        });
    }

    // For main repo: files are handled via evacuate (Phase 4) / restore (Phase 8)
    // For other worktrees: copy files from source to target
    if !is_main_repo {
        info!("migrate_dirty_worktree: Copying working files from source to target");
        // Copy working directory files, excluding .git and gitignored (FR-208)
        copy_working_files(&wt_info.source_path, &wt_info.target_path)?;

        // Preserve file permissions (FR-214)
        preserve_file_permissions(&wt_info.source_path, &wt_info.target_path)?;

        // Reset index to HEAD so files appear as unstaged changes, not deleted
        info!("migrate_dirty_worktree: Resetting index to HEAD");
        let _ = crate::process::command("git")
            .args(["reset"])
            .current_dir(&wt_info.target_path)
            .output();
    } else {
        info!("migrate_dirty_worktree: Main repo - files will be restored in Phase 8");
    }

    Ok(())
}

/// Migrate clean worktree using re-clone (gwt-spec issue T809, FR-207)
/// gwt-spec issue US9-S10: すべてのworktreeはgit worktree addで新規作成
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
        let _ = crate::process::command("git")
            .args(["worktree", "remove", "--force"])
            .arg(&wt_info.source_path)
            .current_dir(&config.source_root)
            .output();
    }

    // Create new worktree from bare repo using git worktree add
    let output = crate::process::command("git")
        .args(["worktree", "add"])
        .arg(&wt_info.target_path)
        .arg(&wt_info.branch)
        .current_dir(&bare_path)
        .output()
        .map_err(|e| {
            crate::logging::log_incident(
                "migration",
                "migrate_clean_worktree",
                Some("MIGRATION_WORKTREE_ADD_SPAWN_FAILED"),
                &format!("branch={}: {}", wt_info.branch, e),
            );
            MigrationError::WorktreeMigrationFailed {
                branch: wt_info.branch.clone(),
                reason: format!("Failed to add worktree: {}", e),
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        crate::logging::log_incident(
            "migration",
            "migrate_clean_worktree",
            Some("MIGRATION_WORKTREE_ADD_FAILED"),
            &format!("branch={}: {}", wt_info.branch, stderr),
        );
        return Err(MigrationError::WorktreeMigrationFailed {
            branch: wt_info.branch.clone(),
            reason: format!("git worktree add failed: {}", stderr),
        });
    }

    // Preserve submodules (FR-218)
    preserve_submodules(&wt_info.target_path)?;

    Ok(())
}

/// Copy working files excluding .git and gitignored files (gwt-spec issue T812, FR-208)
fn copy_working_files(source: &Path, target: &Path) -> Result<(), MigrationError> {
    // Use rsync with git-aware exclusions
    let output = crate::process::command("rsync")
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

/// Copy git hooks from source to bare repository (gwt-spec issue T810, FR-217)
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

/// Preserve submodules in worktree (gwt-spec issue T811, FR-218)
fn preserve_submodules(worktree_path: &Path) -> Result<(), MigrationError> {
    // Check if .gitmodules exists
    let gitmodules = worktree_path.join(".gitmodules");
    if !gitmodules.exists() {
        return Ok(());
    }

    // Initialize and update submodules
    let output = crate::process::command("git")
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

/// Preserve file permissions (gwt-spec issue T901, FR-214)
fn preserve_file_permissions(_source: &Path, _target: &Path) -> Result<(), MigrationError> {
    // On Unix, permissions are preserved by cp -a and rsync -a
    // This function is a placeholder for additional permission handling if needed
    Ok(())
}

/// Migrate stash entries (gwt-spec issue T902, FR-220)
fn migrate_stash(source: &Path, _target: &Path) -> Result<(), MigrationError> {
    // Check if source has stash
    let output = crate::process::command("git")
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

/// Create project config file (gwt-spec issue T905, FR-219)
/// NOTE: BareProjectConfig was removed in SPEC-1787 (Bare abolition).
/// This function is now a no-op but kept to avoid breaking the migration pipeline.
fn create_project_config(_config: &MigrationConfig) -> Result<(), MigrationError> {
    Ok(())
}

/// Preserve tracking relationships (gwt-spec issue T907, FR-221)
fn preserve_tracking_relationships(
    worktree_path: &Path,
    branch: &str,
) -> Result<(), MigrationError> {
    // Set upstream tracking
    let output = crate::process::command("git")
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

/// Derive bare repository name from URL or directory (gwt-spec issue T906, FR-219)
/// Priority: remote URL > directory name
pub fn derive_bare_repo_name(url_or_path: &str) -> String {
    // First, try to get the name from remote URL if it's a path
    let path = std::path::Path::new(url_or_path);
    if path.exists() {
        if let Ok(output) = crate::process::command("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(path)
            .output()
        {
            if output.status.success() {
                let remote_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !remote_url.is_empty() {
                    let name = remote_url
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or("repo")
                        .trim_end_matches(".git");
                    return format!("{}.git", name);
                }
            }
        }
    }

    // Fallback: extract repo name from path
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
    use tempfile::TempDir;

    use super::*;

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

        // Clean repo
        assert!(!is_worktree_dirty(temp.path()));

        // Create untracked file
        std::fs::write(temp.path().join("test.txt"), "content").unwrap();
        assert!(is_worktree_dirty(temp.path()));
    }

    #[test]
    fn test_list_worktrees_to_migrate_empty() {
        let temp = TempDir::new().unwrap();
        let config = super::super::config::MigrationConfig::new(
            temp.path().to_path_buf(),
            temp.path().to_path_buf(),
            "repo.git".to_string(),
        );
        let result = list_worktrees_to_migrate(&config).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn cleanup_dir_returns_false_for_directory_not_empty() {
        let path = std::path::Path::new("/tmp/dummy");
        let result =
            remove_dir_all_for_cleanup_with(path, |_| Err(std::io::Error::from_raw_os_error(66)))
                .unwrap();

        assert!(!result);
    }

    #[test]
    fn cleanup_dir_returns_error_for_permission_denied() {
        let path = std::path::Path::new("/tmp/dummy");
        let result = remove_dir_all_for_cleanup_with(path, |_| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        });

        assert!(result.is_err());
    }

    #[test]
    fn detects_directory_not_empty_error() {
        let err = std::io::Error::from_raw_os_error(66);
        assert!(is_directory_not_empty_error(&err));
    }

    #[test]
    fn evacuate_and_restore_main_repo_files_keeps_temp_entries_until_completion() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let temp_dir = source.join(".gwt-migration-temp");

        std::fs::create_dir_all(source.join(".git")).unwrap();
        std::fs::write(source.join(".git/config"), "git").unwrap();
        std::fs::create_dir_all(source.join(".gwt")).unwrap();
        std::fs::write(source.join(".gwt/project.toml"), "a = 1\n").unwrap();
        std::fs::create_dir_all(source.join(".worktrees")).unwrap();
        std::fs::create_dir_all(source.join(".svn/pristine/00")).unwrap();
        std::fs::write(source.join(".svn/pristine/00/a.svn-base"), "svn").unwrap();
        std::fs::write(source.join("notes.txt"), "note").unwrap();

        evacuate_main_repo_files(&source, &temp_dir).unwrap();

        assert!(source.join(".git/config").exists());
        assert!(source.join(".gwt/project.toml").exists());
        assert!(source.join(".worktrees").exists());
        assert!(!source.join(".svn").exists());
        assert!(!source.join("notes.txt").exists());
        assert!(temp_dir.join(".svn/pristine/00/a.svn-base").exists());
        assert!(temp_dir.join("notes.txt").exists());
        assert!(temp_dir.join(EVACUATION_MANIFEST_FILENAME).exists());

        std::fs::create_dir_all(&target).unwrap();
        restore_evacuated_files(&temp_dir, &target).unwrap();

        assert!(target.join(".svn/pristine/00/a.svn-base").exists());
        assert!(target.join("notes.txt").exists());
        assert!(!target.join(".gwt").exists());
        assert!(temp_dir.join(".svn/pristine/00/a.svn-base").exists());
        assert!(temp_dir.join("notes.txt").exists());
        assert!(temp_dir.join(EVACUATION_MANIFEST_FILENAME).exists());
    }

    #[test]
    fn recover_stale_evacuation_data_restores_entries() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let temp_dir = source.join(".gwt-migration-temp");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&temp_dir).unwrap();

        std::fs::write(temp_dir.join("stale.txt"), "stale").unwrap();
        write_evacuation_manifest(&temp_dir, &[encode_entry_name(OsStr::new("stale.txt"))])
            .unwrap();

        recover_stale_evacuation_data(&source, &temp_dir).unwrap();

        assert!(source.join("stale.txt").exists());
        assert!(!temp_dir.exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn manifest_roundtrip_preserves_non_utf8_names() {
        use std::os::unix::ffi::OsStringExt;

        let temp = TempDir::new().unwrap();
        let temp_dir = temp.path().join(".gwt-migration-temp");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let name = OsString::from_vec(vec![0x66, 0x6f, 0x80]);
        std::fs::write(temp_dir.join(&name), "x").unwrap();
        write_evacuation_manifest(&temp_dir, &[encode_entry_name(name.as_os_str())]).unwrap();

        let entries = collect_evacuation_entries(&temp_dir).unwrap();
        assert_eq!(entries, vec![name]);
    }

    #[test]
    fn collect_entries_falls_back_when_manifest_contains_traversal() {
        let temp = TempDir::new().unwrap();
        let temp_dir = temp.path().join(".gwt-migration-temp");
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(temp_dir.join("safe.txt"), "safe").unwrap();
        let traversal = BASE64_STANDARD.encode("../evil".as_bytes());
        let manifest = serde_json::json!({
            "entries": [traversal],
            "encoding": EVACUATION_MANIFEST_ENCODING,
        });
        std::fs::write(
            temp_dir.join(EVACUATION_MANIFEST_FILENAME),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let entries = collect_evacuation_entries(&temp_dir).unwrap();
        assert_eq!(entries, vec![OsString::from("safe.txt")]);
    }
}
