//! On-disk path layout for the gwt vector index DB.
//!
//! Canonical layout:
//!
//! ```text
//! ~/.gwt/index/<repo-hash>/
//!   meta.json
//!   issues/
//!     chroma.sqlite3
//!     .lock
//!     meta.json
//!   worktrees/
//!     <wt-hash>/
//!       meta.json
//!       manifest-files.json
//!       manifest-specs.json
//!       .lock
//!       specs/chroma.sqlite3
//!       files/chroma.sqlite3
//!       files-docs/chroma.sqlite3
//! ```

use std::path::PathBuf;

use crate::error::{GwtError, Result};
use crate::paths::gwt_home;
use crate::repo_hash::RepoHash;
use crate::worktree_hash::WorktreeHash;

/// Index scope discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Worktree-independent: GitHub Issues.
    Issues,
    /// Worktree-scoped: local SPEC files.
    Specs,
    /// Worktree-scoped: project source code files.
    FilesCode,
    /// Worktree-scoped: project documentation files.
    FilesDocs,
}

impl Scope {
    pub fn requires_worktree(self) -> bool {
        !matches!(self, Scope::Issues)
    }

    /// Subdirectory leaf name relative to the worktree-or-repo prefix.
    pub fn subdir(self) -> &'static str {
        match self {
            Scope::Issues => "issues",
            Scope::Specs => "specs",
            Scope::FilesCode => "files",
            Scope::FilesDocs => "files-docs",
        }
    }
}

/// Return the absolute root directory holding all gwt vector index DBs.
pub fn gwt_index_root() -> PathBuf {
    gwt_home().join("index")
}

/// Return the per-repo root: `~/.gwt/index/<repo-hash>/`.
pub fn gwt_index_repo_dir(repo: &RepoHash) -> PathBuf {
    gwt_index_root().join(repo.as_str())
}

/// Return the per-worktree root under a given repo:
/// `~/.gwt/index/<repo-hash>/worktrees/<wt-hash>/`.
pub fn gwt_index_worktree_dir(repo: &RepoHash, worktree: &WorktreeHash) -> PathBuf {
    gwt_index_repo_dir(repo)
        .join("worktrees")
        .join(worktree.as_str())
}

/// Return the on-disk DB directory for the given (repo, worktree, scope) tuple.
///
/// Returns `Err` for worktree-scoped scopes when `worktree` is `None`. The
/// `Issues` scope ignores any provided `worktree` argument.
pub fn gwt_index_db_path(
    repo: &RepoHash,
    worktree: Option<&WorktreeHash>,
    scope: Scope,
) -> Result<PathBuf> {
    if scope == Scope::Issues {
        return Ok(gwt_index_repo_dir(repo).join(scope.subdir()));
    }
    let wt = worktree.ok_or_else(|| {
        GwtError::Other(format!(
            "scope {:?} requires a worktree hash",
            scope.subdir()
        ))
    })?;
    Ok(gwt_index_worktree_dir(repo, wt).join(scope.subdir()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_hash::compute_repo_hash;
    use crate::worktree_hash::compute_worktree_hash;

    #[test]
    fn issue_scope_resolution() {
        let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
        let path = gwt_index_db_path(&repo, None, Scope::Issues).unwrap();
        assert!(path.ends_with(format!("{}/issues", repo.as_str())));
    }

    #[test]
    fn worktree_scope_requires_hash() {
        let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
        assert!(gwt_index_db_path(&repo, None, Scope::Specs).is_err());
    }

    #[test]
    fn files_scope_layout() {
        let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
        let tmp = tempfile::tempdir().unwrap();
        let wt = compute_worktree_hash(tmp.path()).unwrap();
        let p = gwt_index_db_path(&repo, Some(&wt), Scope::FilesCode).unwrap();
        let s = p.to_string_lossy();
        assert!(s.contains("worktrees"));
        assert!(s.ends_with(&format!("{}/files", wt.as_str())));
    }
}
