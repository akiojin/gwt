use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use gwt_core::worktree_hash::{compute_worktree_hash, WorktreeHash};
use gwt_git::worktree::{main_worktree_root, WorktreeInfo, WorktreeManager};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeEntryKind {
    BareMain,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeEntry {
    /// Stable id derived from the canonical absolute path.
    pub id: String,
    pub kind: WorktreeEntryKind,
    pub path: PathBuf,
    /// Human-friendly label (branch name when available, else last path segment).
    pub label: String,
    pub branch: Option<String>,
    /// True for the worktree that currently anchors the active project tab.
    pub is_active: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error("failed to list worktrees: {0}")]
    List(String),
    #[error("worktree hash failed for {path}: {message}")]
    Hash { path: PathBuf, message: String },
}

/// Enumerate worktrees visible to a gwt-managed repository, sorted with the
/// bare/main entry first and workspaces after by their label. `active_root`
/// (when provided) marks the entry the current tab points to so the GUI can
/// pre-highlight it in the picker.
pub fn enumerate_worktrees(
    repo_root: &Path,
    active_root: Option<&Path>,
) -> Result<Vec<WorktreeEntry>, InventoryError> {
    let manager = WorktreeManager::new(repo_root);
    let infos = manager
        .list()
        .map_err(|err| InventoryError::List(err.to_string()))?;

    let canonical_main = main_worktree_root(repo_root)
        .ok()
        .map(|path| canonicalize_or(path.as_path()));
    let canonical_active = active_root.map(canonicalize_or);

    let mut entries = Vec::new();
    for info in infos {
        if info.prunable {
            continue;
        }
        entries.push(make_entry(
            &info,
            canonical_main.as_deref(),
            canonical_active.as_deref(),
        )?);
    }

    entries.sort_by(entry_ordering);
    Ok(entries)
}

fn make_entry(
    info: &WorktreeInfo,
    canonical_main: Option<&Path>,
    canonical_active: Option<&Path>,
) -> Result<WorktreeEntry, InventoryError> {
    let canonical = canonicalize_or(&info.path);
    let id = id_for(&canonical)?;
    let kind = if canonical_main
        .map(|main| main == canonical.as_path())
        .unwrap_or(false)
    {
        WorktreeEntryKind::BareMain
    } else {
        WorktreeEntryKind::Workspace
    };
    let label = label_for(info, kind);
    let is_active = canonical_active
        .map(|active| active == canonical.as_path())
        .unwrap_or(false);

    Ok(WorktreeEntry {
        id,
        kind,
        path: canonical,
        label,
        branch: info.branch.clone(),
        is_active,
    })
}

fn id_for(path: &Path) -> Result<String, InventoryError> {
    compute_worktree_hash(path)
        .map(|hash: WorktreeHash| hash.to_string())
        .map_err(|err| InventoryError::Hash {
            path: path.to_path_buf(),
            message: err.to_string(),
        })
}

fn canonicalize_or(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn label_for(info: &WorktreeInfo, kind: WorktreeEntryKind) -> String {
    if matches!(kind, WorktreeEntryKind::BareMain) {
        return "main repository".to_string();
    }
    if let Some(branch) = &info.branch {
        return branch.clone();
    }
    info.path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| info.path.display().to_string())
}

fn entry_ordering(left: &WorktreeEntry, right: &WorktreeEntry) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left.kind, right.kind) {
        (WorktreeEntryKind::BareMain, WorktreeEntryKind::Workspace) => Ordering::Less,
        (WorktreeEntryKind::Workspace, WorktreeEntryKind::BareMain) => Ordering::Greater,
        _ => left
            .label
            .to_ascii_lowercase()
            .cmp(&right.label.to_ascii_lowercase())
            .then_with(|| left.label.cmp(&right.label)),
    }
}
