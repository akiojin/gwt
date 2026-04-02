//! Git hooks management
//!
//! Provides utilities for installing and managing git hooks,
//! particularly the develop branch commit protection hook.

use std::io;
use std::path::Path;

/// Marker comments for the gwt develop guard section
const GUARD_START: &str = "# gwt-develop-guard-start";
const GUARD_END: &str = "# gwt-develop-guard-end";

/// The develop branch protection hook script section
const DEVELOP_GUARD_SCRIPT: &str = r#"# gwt-develop-guard-start
branch=$(git symbolic-ref HEAD 2>/dev/null)
if [ "$branch" = "refs/heads/develop" ]; then
  echo "ERROR: Direct commits to develop are not allowed."
  echo "Create a feature branch first: git checkout -b feature/your-feature"
  exit 1
fi
# gwt-develop-guard-end"#;

/// Install or update the pre-commit hook with the develop branch guard.
///
/// If a pre-commit hook already exists, the gwt guard section is appended
/// (or replaced if already present). If no hook exists, a new one is created.
///
/// # Arguments
///
/// * `repo_root` - Path to the repository root
///
/// # Returns
///
/// `Ok(())` if the hook was installed successfully, or an error.
pub fn install_pre_commit_hook(repo_root: &Path) -> io::Result<()> {
    let hooks_dir = repo_root.join(".git").join("hooks");

    // Ensure hooks directory exists
    if !hooks_dir.exists() {
        std::fs::create_dir_all(&hooks_dir)?;
    }

    let hook_path = hooks_dir.join("pre-commit");

    if hook_path.exists() {
        // Read existing hook content
        let content = std::fs::read_to_string(&hook_path)?;

        // Check if guard is already installed
        if content.contains(GUARD_START) {
            // Replace existing guard section
            let new_content = replace_guard_section(&content);
            std::fs::write(&hook_path, new_content)?;
        } else {
            // Append guard section
            let new_content = format!("{}\n\n{}\n", content.trim_end(), DEVELOP_GUARD_SCRIPT);
            std::fs::write(&hook_path, new_content)?;
        }
    } else {
        // Create new hook file
        let content = format!("#!/bin/sh\n\n{}\n", DEVELOP_GUARD_SCRIPT);
        std::fs::write(&hook_path, content)?;
    }

    // Make hook executable (Unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&hook_path, perms)?;
    }

    Ok(())
}

/// Check if the develop guard hook is already installed.
pub fn is_develop_guard_installed(repo_root: &Path) -> bool {
    let hook_path = repo_root.join(".git").join("hooks").join("pre-commit");
    if let Ok(content) = std::fs::read_to_string(hook_path) {
        content.contains(GUARD_START)
    } else {
        false
    }
}

/// Replace the existing guard section with the current version.
fn replace_guard_section(content: &str) -> String {
    let mut result = String::new();
    let mut in_guard = false;
    let mut guard_replaced = false;

    for line in content.lines() {
        if line.trim() == GUARD_START {
            in_guard = true;
            if !guard_replaced {
                result.push_str(DEVELOP_GUARD_SCRIPT);
                result.push('\n');
                guard_replaced = true;
            }
            continue;
        }
        if line.trim() == GUARD_END {
            in_guard = false;
            continue;
        }
        if !in_guard {
            result.push_str(line);
            result.push('\n');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        std::fs::create_dir_all(git_dir.join("hooks")).unwrap();
        temp
    }

    #[test]
    fn test_install_new_hook() {
        let temp = create_test_repo();
        install_pre_commit_hook(temp.path()).unwrap();

        let hook_path = temp.path().join(".git/hooks/pre-commit");
        assert!(hook_path.exists());

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("#!/bin/sh"));
        assert!(content.contains(GUARD_START));
        assert!(content.contains(GUARD_END));
        assert!(content.contains("refs/heads/develop"));
    }

    #[test]
    fn test_install_appends_to_existing_hook() {
        let temp = create_test_repo();
        let hook_path = temp.path().join(".git/hooks/pre-commit");
        std::fs::write(&hook_path, "#!/bin/sh\necho 'existing hook'\n").unwrap();

        install_pre_commit_hook(temp.path()).unwrap();

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("existing hook"));
        assert!(content.contains(GUARD_START));
    }

    #[test]
    fn test_install_replaces_existing_guard() {
        let temp = create_test_repo();
        let hook_path = temp.path().join(".git/hooks/pre-commit");
        let old_content =
            "#!/bin/sh\n# gwt-develop-guard-start\nold guard\n# gwt-develop-guard-end\n"
                .to_string();
        std::fs::write(&hook_path, &old_content).unwrap();

        install_pre_commit_hook(temp.path()).unwrap();

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(!content.contains("old guard"));
        assert!(content.contains("refs/heads/develop"));
    }

    #[test]
    fn test_is_develop_guard_installed() {
        let temp = create_test_repo();
        assert!(!is_develop_guard_installed(temp.path()));

        install_pre_commit_hook(temp.path()).unwrap();
        assert!(is_develop_guard_installed(temp.path()));
    }

    #[test]
    fn test_idempotent_install() {
        let temp = create_test_repo();
        install_pre_commit_hook(temp.path()).unwrap();
        install_pre_commit_hook(temp.path()).unwrap();

        let content = std::fs::read_to_string(temp.path().join(".git/hooks/pre-commit")).unwrap();
        // Should only have one guard section
        let count = content.matches(GUARD_START).count();
        assert_eq!(count, 1);
    }
}
