//! Hooks management — merge managed and user hooks while preserving ownership.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// Create a backup of the hooks file (hooks.json -> hooks.json.bak).
///
/// Returns the path to the backup file.
pub fn backup_hooks(path: &Path) -> Result<PathBuf, HooksError> {
    let bak = backup_path_for(path);
    std::fs::copy(path, &bak)?;
    Ok(bak)
}

/// Restore hooks file from its backup (.bak).
pub fn restore_from_backup(path: &Path) -> Result<(), HooksError> {
    let bak = backup_path_for(path);
    if !bak.exists() {
        return Err(HooksError::BackupNotFound(bak));
    }
    std::fs::copy(&bak, path)?;
    Ok(())
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
    let existing = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        if let Some(config) = try_parse_config(&content) {
            backup_hooks(path)?;
            config
        } else {
            // File is corrupt -- read backup directly if available
            let bak = backup_path_for(path);
            if bak.exists() {
                let bak_content = std::fs::read_to_string(&bak)?;
                try_parse_config(&bak_content).unwrap_or_default()
            } else {
                HooksConfig::default()
            }
        }
    } else {
        HooksConfig::default()
    };

    let new_config = HooksConfig {
        managed_hooks: managed.to_vec(),
        user_hooks: existing.user_hooks,
    };

    // Write atomically (temp file + rename)
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp_path = dir.join(".hooks.json.tmp");
    let json = serde_json::to_string_pretty(&new_config)?;
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, path)?;

    Ok(())
}
