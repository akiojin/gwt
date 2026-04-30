//! Pure Git operations used by the SPEC-1934 US-6 migration: bare-ifying a
//! Normal Git repository, adding worktrees (clean / dirty), evacuating
//! uncommitted files for the dirty path, and restoring upstream + submodule
//! state after the move.
//!
//! Each function is intentionally narrow so tests in
//! `crates/gwt-git/tests/migration_test.rs` can target them in isolation.

use std::path::{Path, PathBuf};

use gwt_core::{GwtError, Result};

/// Clone a Normal repository's `origin` URL into `<target>` as a bare repo.
pub fn clone_bare_from_normal(_origin_url: &str, _target: &Path) -> Result<PathBuf> {
    // Filled in by T-040/T-041.
    Err(GwtError::Git(
        "migration::clone_bare_from_normal — not implemented (SPEC-1934 T-040)".to_string(),
    ))
}

/// Bare-ify a project's local `.git/` directory in place when no usable
/// `origin` URL is available (Edge Case in spec, T-042/T-043).
pub fn bareify_local(_project_root: &Path, _target: &Path) -> Result<PathBuf> {
    Err(GwtError::Git(
        "migration::bareify_local — not implemented (SPEC-1934 T-042)".to_string(),
    ))
}

/// Add a clean worktree at `<target>` for `<branch>` from the bare repo.
pub fn add_worktree_clean(_bare: &Path, _target: &Path, _branch: &str) -> Result<()> {
    Err(GwtError::Git(
        "migration::add_worktree_clean — not implemented (SPEC-1934 T-050)".to_string(),
    ))
}

/// Add a worktree without checkout, so callers can restore evacuated files
/// before running `git reset` (FR-023, T-052).
pub fn add_worktree_no_checkout(_bare: &Path, _target: &Path, _branch: &str) -> Result<()> {
    Err(GwtError::Git(
        "migration::add_worktree_no_checkout — not implemented (SPEC-1934 T-053)".to_string(),
    ))
}

/// Move all files except `.git/` and the migration backup to a temporary
/// evacuation directory; returns the evacuation root for later restore.
pub fn evacuate_dirty_files(_worktree: &Path, _evacuation_root: &Path) -> Result<PathBuf> {
    Err(GwtError::Git(
        "migration::evacuate_dirty_files — not implemented (SPEC-1934 T-053)".to_string(),
    ))
}

/// Restore previously-evacuated files into the new worktree.
pub fn restore_evacuated_files(_evacuation_root: &Path, _new_worktree: &Path) -> Result<()> {
    Err(GwtError::Git(
        "migration::restore_evacuated_files — not implemented (SPEC-1934 T-053)".to_string(),
    ))
}

/// Run `git submodule update --init --recursive` in the new worktree
/// (best effort; failure logs a warning).
pub fn init_submodules(_worktree: &Path) -> Result<()> {
    Err(GwtError::Git(
        "migration::init_submodules — not implemented (SPEC-1934 T-061)".to_string(),
    ))
}

/// Set upstream tracking for `<branch>` to `origin/<branch>`, if it exists.
pub fn set_upstream(_worktree: &Path, _branch: &str) -> Result<()> {
    Err(GwtError::Git(
        "migration::set_upstream — not implemented (SPEC-1934 T-063)".to_string(),
    ))
}
