//! Repo-scoped trusted store (SPEC-3248 P9b, T-172/T-173-lite).
//!
//! The authoritative copies of the Execution Control Record, Verification
//! Run Record, and Verification Plan Record live under the machine-local
//! repo-scoped store — `~/.gwt/projects/<repo-hash>/trusted/<worktree-key>/`
//! — instead of the worktree. The worktree's `.gwt/skill-state/*.json`
//! files remain as human-inspectable **mirrors**: every canonical writer
//! writes both, and every gate reads the repo-scoped copy first, so editing
//! the mirror changes nothing the gates trust (T-174 core) and the records
//! survive ephemeral worktree deletion (T-175 core).
//!
//! A worktree without a trusted copy falls back to the mirror as a legacy
//! (pre-P9b) record — same one-release-cycle sunset policy as the P9a
//! integrity hashes. Worktrees where the repo hash cannot be resolved
//! (non-git test dirs) run in mirror-only degenerate mode.
//!
//! Follow-ups (dependent): store health gates (T-177), GC/retention
//! (T-181), legacy import (T-182), cross-worktree conflict surfacing.

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

/// Resolve the repo-scoped trusted directory for a worktree. `None` when the
/// repo hash cannot be determined (non-git dirs — degenerate mirror-only
/// mode).
#[must_use]
pub fn trusted_dir_for_worktree(worktree: &Path) -> Option<PathBuf> {
    let repo_hash = crate::index_worker::detect_repo_hash(worktree)?;
    Some(
        gwt_core::paths::gwt_projects_dir()
            .join(repo_hash.as_str())
            .join("trusted")
            .join(worktree_key(worktree)),
    )
}

/// Stable key for one worktree: sha256 of the canonicalized, normalized
/// absolute path (backslashes unified, lowercased — Windows paths reach the
/// writers and readers in different spellings, and the key must not fork).
/// `dunce` keeps the canonical form free of the Windows `\\?\` prefix so the
/// key stays identical when canonicalization later fails (e.g. reading the
/// record of an already-deleted ephemeral worktree, T-175).
fn worktree_key(worktree: &Path) -> String {
    let canonical = dunce::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    let normalized = canonical
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    // Case-fold only where the filesystem does: on case-sensitive systems
    // two paths differing in case are genuinely different worktrees and
    // must not share a trusted directory.
    #[cfg(windows)]
    let normalized = normalized.to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    format!("{digest:x}")[..16].to_string()
}

/// Read the trusted copy of `file_name` for the worktree. `Ok(None)` when the
/// store or the file is absent (legacy / degenerate mode — callers fall back
/// to the worktree mirror).
pub fn read(worktree: &Path, file_name: &str) -> io::Result<Option<String>> {
    let Some(dir) = trusted_dir_for_worktree(worktree) else {
        return Ok(None);
    };
    match fs::read_to_string(dir.join(file_name)) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Write the trusted copy atomically. A worktree without a resolvable repo
/// hash is a silent no-op (degenerate mode; the mirror still carries the
/// record).
pub fn write(worktree: &Path, file_name: &str, bytes: &[u8]) -> io::Result<()> {
    let Some(dir) = trusted_dir_for_worktree(worktree) else {
        return Ok(());
    };
    gwt_github::cache::write_atomic(&dir.join(file_name), bytes)
}

/// Write the authoritative trusted copy, then the worktree mirror. Once the
/// trusted copy is written the mirror is informational only — its failure is
/// logged, not surfaced, so an operation can never report failure while the
/// gates already honor the new record. In degenerate mode the mirror is the
/// only copy and its failure propagates.
pub fn write_with_mirror(
    worktree: &Path,
    file_name: &str,
    mirror_path: &Path,
    bytes: &[u8],
) -> io::Result<()> {
    let trusted_dir = trusted_dir_for_worktree(worktree);
    if let Some(dir) = &trusted_dir {
        gwt_github::cache::write_atomic(&dir.join(file_name), bytes)?;
    }
    match gwt_github::cache::write_atomic(mirror_path, bytes) {
        Err(err) if trusted_dir.is_some() => {
            tracing::warn!(
                ?err,
                path = %mirror_path.display(),
                "worktree mirror write failed after trusted store write"
            );
            Ok(())
        }
        result => result,
    }
}

/// True when the worktree is under trusted-store management: launch
/// materialization wrote the Execution Control Record's trusted copy, so
/// every later canonical `verify.plan` / `verify.run` write produced a
/// trusted copy too. Readers use this to refuse mirror-only verification
/// state in managed worktrees — there, a mirror without a trusted copy can
/// only be a forgery or a pre-P9b binary write, never canonical evidence.
#[must_use]
pub fn under_trusted_management(worktree: &Path) -> bool {
    trusted_dir_for_worktree(worktree)
        .is_some_and(|dir| dir.join("execution-control.json").exists())
}

/// Initialize a git repo with an `origin` remote so `detect_repo_hash`
/// resolves (it derives the repo hash from the origin URL). Shared by the
/// authority-precedence tests in `execution_state` / `verification_record`.
#[cfg(test)]
pub(crate) fn init_git_repo_with_origin(dir: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec![
            "remote",
            "add",
            "origin",
            "https://example.com/t/trusted-store.git",
        ],
        vec!["commit", "--allow-empty", "-qm", "init"],
    ] {
        let status = gwt_core::process::hidden_command("git")
            .args(&args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    #[test]
    fn non_git_dir_is_degenerate_mirror_only() {
        let dir = tempfile::tempdir().unwrap();
        assert!(trusted_dir_for_worktree(dir.path()).is_none());
        assert_eq!(read(dir.path(), "x.json").unwrap(), None);
        write(dir.path(), "x.json", b"{}").unwrap();
        assert_eq!(read(dir.path(), "x.json").unwrap(), None);
    }

    #[test]
    fn git_worktree_roundtrips_through_repo_scoped_store() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        init_git_repo_with_origin(dir.path());

        assert_eq!(read(dir.path(), "r.json").unwrap(), None);
        write(dir.path(), "r.json", b"{\"a\":1}").unwrap();
        assert_eq!(
            read(dir.path(), "r.json").unwrap().as_deref(),
            Some("{\"a\":1}")
        );
        // Store lives under the scoped HOME, outside the worktree.
        let trusted = trusted_dir_for_worktree(dir.path()).unwrap();
        assert!(trusted.starts_with(home.path()));
        assert!(!trusted.starts_with(dir.path()));

        // Key is stable across path spellings of the same worktree.
        let respelled = dir.path().to_string_lossy().to_uppercase();
        let respelled_key_dir = trusted_dir_for_worktree(Path::new(&respelled));
        if let Some(respelled_dir) = respelled_key_dir {
            assert_eq!(respelled_dir, trusted);
        }

        // T-175 core: the stored bytes live under HOME, so deleting the
        // (ephemeral) worktree does not take the record with it.
        let stored_file = trusted.join("r.json");
        drop(dir);
        assert!(stored_file.exists());
    }
}
