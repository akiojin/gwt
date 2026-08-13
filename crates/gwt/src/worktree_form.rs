//! Worktree **form** predicates (SPEC #3245 FR-007).
//!
//! An ephemeral, disposable worktree is used for one-shot launches. The
//! legacy `.intake` filesystem prefix remains stable for compatibility.

use std::path::Path;

/// SPEC-3214: filename stem for ephemeral worktrees. Placed as a sibling of
/// the main worktree (`<layout_root>/.intake`, suffixed on collision) so it
/// is easy to recognize and prune.
pub const EPHEMERAL_WORKTREE_PREFIX: &str = ".intake";

/// Whether `path` is an ephemeral worktree created by the ephemeral launch
/// resolver — i.e. its file name is `.intake` or `.intake-<n>`. Session-end
/// cleanup, orphan pruning, and managed-asset policy key off this so they
/// never touch a real branch worktree.
#[must_use]
pub fn is_ephemeral_worktree_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == EPHEMERAL_WORKTREE_PREFIX
                || name.starts_with(&format!("{EPHEMERAL_WORKTREE_PREFIX}-"))
        })
}
