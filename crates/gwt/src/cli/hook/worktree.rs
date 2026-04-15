//! Worktree-root detection shared by `block-cd-command` and
//! `block-file-ops`. Shells out to `git rev-parse --show-toplevel`
//! exactly like the Node helpers did.

use std::path::PathBuf;
use std::process::Command;

/// Return the worktree root as reported by `git rev-parse
/// --show-toplevel`. Falls back to the current process cwd if git is
/// unavailable or the command fails.
pub fn detect_worktree_root() -> PathBuf {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
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

fn fallback_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
