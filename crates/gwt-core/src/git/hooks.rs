//! Git hook management (SPEC-1787 FR-018 through FR-020)
//!
//! Provides functionality to install and manage Git hooks for repository protection.

use std::path::Path;

use tracing::{debug, info};

/// Marker comments used to identify gwt-managed hook sections
const GUARD_START: &str = "# gwt-develop-guard-start";
const GUARD_END: &str = "# gwt-develop-guard-end";

/// The develop branch protection script block
const DEVELOP_GUARD_SCRIPT: &str = r#"# gwt-develop-guard-start
branch=$(git symbolic-ref HEAD 2>/dev/null)
if [ "$branch" = "refs/heads/develop" ]; then
  echo "ERROR: Direct commits to develop are not allowed."
  echo "Create a feature branch first: git checkout -b feature/feature-{N}"
  exit 1
fi
# gwt-develop-guard-end"#;

/// Install a pre-commit hook that blocks direct commits to the develop branch.
///
/// If a pre-commit hook already exists, the gwt guard section is appended
/// (or replaced if already present). If no hook exists, a new one is created.
///
/// The hook uses `# gwt-develop-guard-start` / `# gwt-develop-guard-end`
/// markers to identify the managed section, enabling safe updates without
/// overwriting user-defined hook logic.
pub fn install_pre_commit_hook(repo_root: &Path) -> std::io::Result<()> {
    let hooks_dir = repo_root.join(".git").join("hooks");
    if !hooks_dir.exists() {
        debug!(
            path = %hooks_dir.display(),
            "Hooks directory does not exist, skipping hook installation"
        );
        return Ok(());
    }

    let hook_path = hooks_dir.join("pre-commit");

    if hook_path.exists() {
        let existing = std::fs::read_to_string(&hook_path)?;

        // Already installed — check if guard section exists
        if existing.contains(GUARD_START) {
            debug!(
                path = %hook_path.display(),
                "Pre-commit hook already contains gwt develop guard"
            );
            return Ok(());
        }

        // Append guard section to existing hook
        let updated = format!("{}\n\n{}\n", existing.trim_end(), DEVELOP_GUARD_SCRIPT);
        std::fs::write(&hook_path, updated)?;
        info!(
            path = %hook_path.display(),
            "Appended gwt develop guard to existing pre-commit hook"
        );
    } else {
        // Create new hook
        let content = format!("#!/bin/sh\n{}\n", DEVELOP_GUARD_SCRIPT);
        std::fs::write(&hook_path, content)?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&hook_path, perms)?;
        }

        info!(
            path = %hook_path.display(),
            "Created pre-commit hook with gwt develop guard"
        );
    }

    Ok(())
}

/// Remove the gwt develop guard section from the pre-commit hook.
///
/// If the hook contains only the gwt guard, the file is removed entirely.
/// If it contains other content, only the gwt section is stripped.
pub fn uninstall_pre_commit_hook(repo_root: &Path) -> std::io::Result<()> {
    let hook_path = repo_root.join(".git").join("hooks").join("pre-commit");
    if !hook_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&hook_path)?;
    if !content.contains(GUARD_START) {
        return Ok(());
    }

    // Remove the guard section
    let mut result = String::new();
    let mut in_guard = false;
    for line in content.lines() {
        if line.contains(GUARD_START) {
            in_guard = true;
            continue;
        }
        if line.contains(GUARD_END) {
            in_guard = false;
            continue;
        }
        if !in_guard {
            result.push_str(line);
            result.push('\n');
        }
    }

    let trimmed = result.trim();
    if trimmed.is_empty() || trimmed == "#!/bin/sh" {
        // Only the gwt guard was present; remove the file
        std::fs::remove_file(&hook_path)?;
        info!(
            path = %hook_path.display(),
            "Removed pre-commit hook (only contained gwt guard)"
        );
    } else {
        std::fs::write(&hook_path, result)?;
        info!(
            path = %hook_path.display(),
            "Removed gwt develop guard from pre-commit hook"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        crate::process::command("git")
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        temp
    }

    #[test]
    fn test_install_creates_new_hook() {
        let temp = create_test_repo();
        install_pre_commit_hook(temp.path()).unwrap();

        let hook_path = temp.path().join(".git/hooks/pre-commit");
        assert!(hook_path.exists());

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("#!/bin/sh"));
        assert!(content.contains(GUARD_START));
        assert!(content.contains(GUARD_END));
        assert!(content.contains("refs/heads/develop"));

        // Check executable permission on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&hook_path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o111, 0o111, "Hook should be executable");
        }
    }

    #[test]
    fn test_install_appends_to_existing_hook() {
        let temp = create_test_repo();
        let hook_path = temp.path().join(".git/hooks/pre-commit");

        // Create an existing hook
        std::fs::write(&hook_path, "#!/bin/sh\necho 'existing hook'\n").unwrap();

        install_pre_commit_hook(temp.path()).unwrap();

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("existing hook"));
        assert!(content.contains(GUARD_START));
    }

    #[test]
    fn test_install_idempotent() {
        let temp = create_test_repo();
        install_pre_commit_hook(temp.path()).unwrap();
        install_pre_commit_hook(temp.path()).unwrap();

        let content = std::fs::read_to_string(temp.path().join(".git/hooks/pre-commit")).unwrap();
        // Should only have one guard section
        assert_eq!(content.matches(GUARD_START).count(), 1);
    }

    #[test]
    fn test_uninstall_removes_guard_only_hook() {
        let temp = create_test_repo();
        install_pre_commit_hook(temp.path()).unwrap();
        uninstall_pre_commit_hook(temp.path()).unwrap();

        let hook_path = temp.path().join(".git/hooks/pre-commit");
        assert!(!hook_path.exists());
    }

    #[test]
    fn test_uninstall_preserves_other_content() {
        let temp = create_test_repo();
        let hook_path = temp.path().join(".git/hooks/pre-commit");

        // Create hook with existing content + guard
        std::fs::write(&hook_path, "#!/bin/sh\necho 'keep me'\n").unwrap();
        install_pre_commit_hook(temp.path()).unwrap();
        uninstall_pre_commit_hook(temp.path()).unwrap();

        assert!(hook_path.exists());
        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("keep me"));
        assert!(!content.contains(GUARD_START));
    }

    #[test]
    fn test_install_skips_non_git_dir() {
        let temp = TempDir::new().unwrap();
        // No .git directory — should succeed silently
        install_pre_commit_hook(temp.path()).unwrap();
    }

    #[test]
    fn test_hook_blocks_develop_commit() {
        let temp = create_test_repo();
        crate::process::command("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        crate::process::command("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        // Create initial commit on default branch
        std::fs::write(temp.path().join("README.md"), "# Test\n").unwrap();
        crate::process::command("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        crate::process::command("git")
            .args(["commit", "-m", "initial"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        // Create and switch to develop branch
        crate::process::command("git")
            .args(["checkout", "-b", "develop"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        install_pre_commit_hook(temp.path()).unwrap();

        // Try to commit on develop — should be blocked
        std::fs::write(temp.path().join("test.txt"), "test\n").unwrap();
        crate::process::command("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        let output = crate::process::command("git")
            .args(["commit", "-m", "should fail"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        assert!(
            !output.status.success(),
            "Commit on develop should be blocked by pre-commit hook"
        );
    }

    #[test]
    fn test_hook_allows_feature_branch_commit() {
        let temp = create_test_repo();
        crate::process::command("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        crate::process::command("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        // Create initial commit
        std::fs::write(temp.path().join("README.md"), "# Test\n").unwrap();
        crate::process::command("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        crate::process::command("git")
            .args(["commit", "-m", "initial"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        // Switch to feature branch
        crate::process::command("git")
            .args(["checkout", "-b", "feature/test-1234"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        install_pre_commit_hook(temp.path()).unwrap();

        // Commit on feature branch — should succeed
        std::fs::write(temp.path().join("test.txt"), "test\n").unwrap();
        crate::process::command("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        let output = crate::process::command("git")
            .args(["commit", "-m", "feature commit"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "Commit on feature branch should be allowed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
