//! Git submodule operations (SPEC-a70a1ece US6)

use std::path::Path;
use std::process::Command;
use tracing::{debug, warn};

/// Check if repository has submodules (SPEC-a70a1ece T1001)
pub fn has_submodules(worktree_path: &Path) -> bool {
    let gitmodules = worktree_path.join(".gitmodules");
    gitmodules.exists()
}

/// Initialize and update submodules in a worktree (SPEC-a70a1ece T1002)
///
/// This function runs `git submodule update --init --recursive` to initialize
/// and update all submodules. Returns Ok(()) on success or if there are no submodules.
/// Returns Err only for fatal errors.
pub fn init_submodules(worktree_path: &Path) -> Result<(), String> {
    // Skip if no submodules
    if !has_submodules(worktree_path) {
        debug!(
            path = %worktree_path.display(),
            "No submodules found, skipping initialization"
        );
        return Ok(());
    }

    debug!(
        path = %worktree_path.display(),
        "Initializing submodules"
    );

    let output = Command::new("git")
        .args(["submodule", "update", "--init", "--recursive"])
        .current_dir(worktree_path)
        .output()
        .map_err(|e| format!("Failed to run git submodule: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Log as warning but don't fail - submodule init failures are non-fatal (T1005)
        warn!(
            path = %worktree_path.display(),
            error = %stderr,
            "Submodule initialization failed (non-fatal)"
        );
        // Return Ok to allow worktree creation to succeed
        return Ok(());
    }

    debug!(
        path = %worktree_path.display(),
        "Submodules initialized successfully"
    );
    Ok(())
}

/// List submodule paths in a repository
pub fn list_submodules(repo_root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["config", "--file", ".gitmodules", "--get-regexp", "path"])
        .current_dir(repo_root)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .filter_map(|line| {
                    // Format: "submodule.<name>.path <path>"
                    line.split_whitespace().nth(1).map(|s| s.to_string())
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_has_submodules_false() {
        let temp = TempDir::new().unwrap();
        assert!(!has_submodules(temp.path()));
    }

    #[test]
    fn test_has_submodules_true() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join(".gitmodules"), "[submodule]").unwrap();
        assert!(has_submodules(temp.path()));
    }

    #[test]
    fn test_init_submodules_no_submodules() {
        let temp = TempDir::new().unwrap();
        // Initialize git repo
        Command::new("git")
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        // Should succeed even without submodules
        let result = init_submodules(temp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_list_submodules_empty() {
        let temp = TempDir::new().unwrap();
        let result = list_submodules(temp.path());
        assert!(result.is_empty());
    }
}
