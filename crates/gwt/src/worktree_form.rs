//! Worktree **form** predicates (SPEC #3245 FR-007).
//!
//! The behavioral Intake/Execution lanes are gone; what remains is the
//! worktree form: an ephemeral, disposable worktree used for one-shot
//! launches. Naming still says "intake" until the vocabulary rename (#3384).

use std::path::Path;

/// SPEC-3214: filename stem for ephemeral worktrees. Placed as a sibling of
/// the main worktree (`<layout_root>/.intake`, suffixed on collision) so it
/// is easy to recognize and prune.
pub const INTAKE_WORKTREE_PREFIX: &str = ".intake";

/// Whether `path` is an ephemeral worktree created by the ephemeral launch
/// resolver — i.e. its file name is `.intake` or `.intake-<n>`. Session-end
/// cleanup, orphan pruning, and managed-asset policy key off this so they
/// never touch a real branch worktree.
#[must_use]
pub fn is_ephemeral_intake_worktree(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == INTAKE_WORKTREE_PREFIX
                || name.starts_with(&format!("{INTAKE_WORKTREE_PREFIX}-"))
        })
}
