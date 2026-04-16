//! Hooks management — merge managed and user hooks while preserving ownership.

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};

/// Marker prefix that identifies gwt-managed hooks.
const GWT_MANAGED_MARKER: &str = "# gwt-managed";

/// A single hook definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hook {
    /// Event that triggers this hook (e.g. "pre-commit", "post-merge").
    pub event: String,
    /// Shell command to execute.
    pub command: String,
    /// Optional comment marker used to identify the hook's owner.
    pub comment_marker: Option<String>,
}

/// Configuration holding both managed and user hooks.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HooksConfig {
    /// Hooks managed by gwt (auto-generated, may be overwritten).
    pub managed_hooks: Vec<Hook>,
    /// Hooks added by the user (preserved across updates).
    pub user_hooks: Vec<Hook>,
}

/// Errors from hooks operations.
#[derive(Debug, thiserror::Error)]
pub enum HooksError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("Backup not found: {0}")]
    BackupNotFound(PathBuf),

    #[error("Hooks lock unavailable: {0}")]
    LockUnavailable(PathBuf),
}

/// Check whether a hook is gwt-managed based on its comment marker.
pub fn is_gwt_managed(hook: &Hook) -> bool {
    hook.comment_marker
        .as_deref()
        .is_some_and(|m| m.starts_with(GWT_MANAGED_MARKER))
}

/// Merge managed and user hooks into a single list.
///
/// Managed hooks come first, followed by user hooks. User hooks for the
/// same event are never overwritten.
pub fn merge_hooks(managed: &[Hook], user: &[Hook]) -> Vec<Hook> {
    let mut merged: Vec<Hook> = managed.to_vec();
    for uh in user {
        // Only add user hooks that don't duplicate a managed hook for the same event+command.
        let dominated = merged
            .iter()
            .any(|mh| mh.event == uh.event && mh.command == uh.command);
        if !dominated {
            merged.push(uh.clone());
        }
    }
    merged
}

/// Derive the backup path (.json.bak) for a hooks file.
fn backup_path_for(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

/// Derive the timestamped backup path for a hooks file.
fn timestamped_backup_path_for(path: &Path) -> PathBuf {
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%3fZ").to_string();
    path.with_extension(format!("json.{stamp}.bak"))
}

/// Derive the lock file path for a hooks file.
fn lock_path_for(path: &Path) -> PathBuf {
    path.with_extension("json.lock")
}

/// Resolve a hooks path to its actual file target when the path is a symlink.
fn resolved_hooks_path(path: &Path) -> PathBuf {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return path.to_path_buf();
    };
    if !meta.file_type().is_symlink() {
        return path.to_path_buf();
    }

    let Ok(link) = std::fs::read_link(path) else {
        return path.to_path_buf();
    };

    if link.is_absolute() {
        link
    } else {
        path.parent().unwrap_or(Path::new(".")).join(link)
    }
}

/// Find the newest timestamped backup path for a hooks file.
fn newest_timestamped_backup(path: &Path) -> Option<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let file_name = path.file_name()?.to_string_lossy().to_string();
    let prefix = format!("{file_name}.");
    let suffix = ".bak";

    let mut matches: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(suffix))
        })
        .collect();

    matches.sort();
    matches.pop()
}

/// Return the ordered backup candidates, newest stable/latest first.
fn backup_candidates_for(path: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![backup_path_for(path)];
    if let Some(timestamped) = newest_timestamped_backup(path) {
        if candidates[0] != timestamped {
            candidates.push(timestamped);
        }
    }
    candidates
}

/// Try to parse the first valid backup candidate.
fn load_backup_config(path: &Path) -> Result<Option<HooksConfig>, HooksError> {
    for candidate in backup_candidates_for(path) {
        if candidate.exists() {
            let content = std::fs::read_to_string(&candidate)?;
            if let Some(config) = try_parse_config(&content) {
                return Ok(Some(config));
            }
        }
    }
    Ok(None)
}

/// Acquire an exclusive lock for the resolved hooks target.
fn acquire_lock(path: &Path) -> Result<std::fs::File, HooksError> {
    let lock_path = lock_path_for(path);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    match file.try_lock_exclusive() {
        Ok(_) => Ok(file),
        Err(_) => Err(HooksError::LockUnavailable(lock_path)),
    }
}

/// Create a backup of the hooks file (timestamped + stable latest copy).
///
/// Returns the path to the timestamped backup file.
pub fn backup_hooks(path: &Path) -> Result<PathBuf, HooksError> {
    let target = resolved_hooks_path(path);
    let timestamped = timestamped_backup_path_for(&target);
    let stable = backup_path_for(&target);

    std::fs::copy(&target, &timestamped)?;
    std::fs::copy(&target, &stable)?;

    Ok(timestamped)
}

/// Restore hooks file from its backup (.bak).
pub fn restore_from_backup(path: &Path) -> Result<(), HooksError> {
    let target = resolved_hooks_path(path);
    for candidate in backup_candidates_for(&target) {
        if candidate.exists() {
            std::fs::copy(&candidate, &target)?;
            return Ok(());
        }
    }
    Err(HooksError::BackupNotFound(backup_path_for(&target)))
}

/// Check if content is corrupted (invalid JSON for HooksConfig).
pub fn detect_corruption(content: &str) -> bool {
    serde_json::from_str::<HooksConfig>(content).is_err()
}

/// Try to parse content as `HooksConfig`, returning `None` on failure.
fn try_parse_config(content: &str) -> Option<HooksConfig> {
    serde_json::from_str(content).ok()
}

/// Safe merge: backup, read/parse (restore on corruption), merge, write atomically.
///
/// 1. Read and validate (backup only if valid; restore from .bak if corrupt)
/// 2. Merge managed hooks (replace gwt-managed, keep user hooks)
/// 3. Write result atomically via temp file + rename
pub fn merge_hooks_safe(path: &Path, managed: &[Hook]) -> Result<(), HooksError> {
    let target = resolved_hooks_path(path);
    let _lock = acquire_lock(&target)?;

    let existing = if path.exists() || target.exists() {
        let content =
            std::fs::read_to_string(path).or_else(|_| std::fs::read_to_string(&target))?;
        if content.trim().is_empty() {
            load_backup_config(&target)?.unwrap_or_default()
        } else if let Some(config) = try_parse_config(&content) {
            backup_hooks(&target)?;
            config
        } else {
            load_backup_config(&target)?.unwrap_or_default()
        }
    } else {
        HooksConfig::default()
    };

    let new_config = HooksConfig {
        managed_hooks: managed.to_vec(),
        user_hooks: existing.user_hooks,
    };

    // Write atomically (temp file + rename)
    let dir = target.parent().unwrap_or(Path::new("."));
    let tmp_path = dir.join(format!(
        ".{}.tmp-{}",
        target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("hooks.json"),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let json = serde_json::to_string_pretty(&new_config)?;
    {
        let mut tmp = std::fs::File::create(&tmp_path)?;
        tmp.write_all(json.as_bytes())?;
        tmp.sync_all()?;
    }
    std::fs::rename(&tmp_path, &target)?;

    Ok(())
}
