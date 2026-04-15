use std::path::Path;

use gwt_core::repo_hash::{compute_repo_hash, RepoHash};

/// Determine `RepoHash` for the given repository root by shelling out to
/// `git remote get-url origin`. Returns `None` if no origin is configured.
pub fn detect_repo_hash(repo_root: &Path) -> Option<RepoHash> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        return None;
    }
    Some(compute_repo_hash(&url))
}
