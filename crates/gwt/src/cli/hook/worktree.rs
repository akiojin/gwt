//! Worktree-root detection shared by `block-cd-command` and
//! `block-file-ops`. Shells out to `git rev-parse --show-toplevel`
//! exactly like the Node helpers did.

use std::path::PathBuf;

/// Return the worktree root as reported by `git rev-parse
/// --show-toplevel`. Falls back to the current process cwd if git is
/// unavailable or the command fails.
pub fn detect_worktree_root() -> PathBuf {
    let cwd = fallback_cwd();
    if let Some(worktree) = crate::pm_registry::pm_worktree_for_runtime_dir(&cwd) {
        return worktree;
    }
    let output = gwt_core::process::hidden_command("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout);
            let trimmed = s.trim();
            if trimmed.is_empty() {
                fallback_cwd()
            } else {
                PathBuf::from(trimmed)
            }
        }
        _ => fallback_cwd(),
    }
}

/// Navigation boundary for `block-cd-command`.
///
/// Matches [`detect_worktree_root`] except in a resident PM session, where
/// `cd` may reach the whole `pm/` directory so the PM can read its runtime
/// skill references (Issue #6989). Mutating policies keep using the worktree.
pub fn detect_cd_boundary() -> PathBuf {
    crate::pm_registry::pm_session_dir_for_cwd(&fallback_cwd()).unwrap_or_else(detect_worktree_root)
}

fn fallback_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
