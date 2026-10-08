//! Worktree **form** predicates (SPEC #3245 FR-007).
//!
//! An ephemeral, disposable worktree is used for one-shot launches. The
//! legacy `.intake` filesystem prefix remains stable for compatibility.

use std::path::Path;

use gwt_core::coordination::BoardOriginWorktreeForm;

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

/// The worktree form to record on a Board post made from `worktree_root`
/// (SPEC-1974 FR-063), given the branch the posting session reported.
///
/// The ephemeral form is read from the path, which stays true after the
/// session ends and the worktree is pruned. Anything else is on a branch —
/// unless the session named none, which is recorded as `Unknown` rather than
/// guessed at.
#[must_use]
pub fn board_origin_worktree_form(
    worktree_root: &Path,
    branch: Option<&str>,
) -> BoardOriginWorktreeForm {
    if is_ephemeral_worktree_path(worktree_root) {
        return BoardOriginWorktreeForm::Ephemeral;
    }
    match branch.map(str::trim).filter(|branch| !branch.is_empty()) {
        Some(_) => BoardOriginWorktreeForm::BranchBacked,
        None => BoardOriginWorktreeForm::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_board_post_records_the_form_of_the_worktree_it_came_from() {
        // SPEC-1974 FR-063 / Issue #3384 vocabulary. A branchless ephemeral
        // worktree stays identifiable on the Board after it is pruned, and an
        // unresolved form is recorded as unknown instead of assumed.
        assert_eq!(
            board_origin_worktree_form(Path::new("/repos/.intake-2"), None),
            BoardOriginWorktreeForm::Ephemeral
        );
        assert_eq!(
            board_origin_worktree_form(
                Path::new("/repos/work/issue-1974"),
                Some("work/issue-1974")
            ),
            BoardOriginWorktreeForm::BranchBacked
        );
        assert_eq!(
            board_origin_worktree_form(Path::new("/repos/work/issue-1974"), Some("   ")),
            BoardOriginWorktreeForm::Unknown
        );
    }
}
