//! Migration validation (SPEC-a70a1ece T801-T803)

use super::{MigrationConfig, MigrationError};
use std::path::Path;
use std::process::Command;
use tracing::debug;

/// Validation result
#[derive(Debug)]
pub struct ValidationResult {
    /// Whether validation passed
    pub passed: bool,
    /// List of validation errors
    pub errors: Vec<MigrationError>,
    /// List of warnings (non-blocking)
    pub warnings: Vec<String>,
    /// Estimated space needed in bytes
    pub space_needed: u64,
    /// Available space in bytes
    pub space_available: u64,
}

impl ValidationResult {
    /// Create a successful validation result
    pub fn success(space_needed: u64, space_available: u64) -> Self {
        Self {
            passed: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            space_needed,
            space_available,
        }
    }

    /// Add an error (marks validation as failed)
    pub fn add_error(&mut self, error: MigrationError) {
        self.passed = false;
        self.errors.push(error);
    }

    /// Add a warning (non-blocking)
    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

/// Check available disk space (SPEC-a70a1ece T801, FR-212)
pub fn check_disk_space(path: &Path) -> Result<(u64, u64), MigrationError> {
    // Get available space using df command
    let output = Command::new("df")
        .args(["-B1", "--output=avail"])
        .arg(path)
        .output()
        .map_err(|e| MigrationError::IoError {
            path: path.to_path_buf(),
            reason: format!("Failed to check disk space: {}", e),
        })?;

    if !output.status.success() {
        return Err(MigrationError::IoError {
            path: path.to_path_buf(),
            reason: "df command failed".to_string(),
        });
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let available: u64 = output_str
        .lines()
        .nth(1)
        .and_then(|line| line.trim().parse().ok())
        .unwrap_or(0);

    // Estimate needed space (rough estimate: 2x the source size for safety)
    let source_size = get_directory_size(path).unwrap_or(0);
    let needed = source_size * 2;

    Ok((needed, available))
}

/// Get directory size recursively
fn get_directory_size(path: &Path) -> Result<u64, MigrationError> {
    let output = Command::new("du")
        .args(["-sb"])
        .arg(path)
        .output()
        .map_err(|e| MigrationError::IoError {
            path: path.to_path_buf(),
            reason: format!("Failed to get directory size: {}", e),
        })?;

    if !output.status.success() {
        return Ok(0);
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let size: u64 = output_str
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    Ok(size)
}

/// Check for locked worktrees (SPEC-a70a1ece T802, FR-222)
pub fn check_locked_worktrees(repo_root: &Path) -> Result<Vec<String>, MigrationError> {
    let output = crate::process::git_command()
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_root)
        .output()
        .map_err(|e| MigrationError::GitError {
            reason: format!("Failed to list worktrees: {}", e),
        })?;

    if !output.status.success() {
        return Err(MigrationError::GitError {
            reason: "Failed to list worktrees".to_string(),
        });
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let mut locked = Vec::new();
    let mut current_worktree: Option<String> = None;

    for line in output_str.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current_worktree = Some(path.to_string());
        } else if line == "locked" {
            if let Some(ref wt) = current_worktree {
                locked.push(wt.clone());
            }
        }
    }

    Ok(locked)
}

/// Validate migration prerequisites (SPEC-a70a1ece T803)
pub fn validate_migration(config: &MigrationConfig) -> Result<ValidationResult, MigrationError> {
    debug!(
        source = %config.source_root.display(),
        target = %config.target_root.display(),
        "Validating migration"
    );

    // Check disk space (non-fatal: use defaults if check fails)
    let (space_needed, space_available) = match check_disk_space(&config.source_root) {
        Ok((needed, available)) => (needed, available),
        Err(e) => {
            debug!("Disk space check failed (non-fatal): {}", e);
            // Use reasonable defaults if check fails (e.g., Docker environment)
            (0, u64::MAX)
        }
    };
    let mut result = ValidationResult::success(space_needed, space_available);

    // Check if we have enough space (FR-212, FR-213)
    // Skip this check if we couldn't get disk space info
    if space_needed > 0 && space_available < space_needed {
        result.add_error(MigrationError::InsufficientDiskSpace {
            needed: space_needed,
            available: space_available,
        });
    }

    // Check for locked worktrees (FR-222)
    match check_locked_worktrees(&config.source_root) {
        Ok(locked) => {
            for locked_path in locked {
                result.add_error(MigrationError::LockedWorktree {
                    path: locked_path.into(),
                });
            }
        }
        Err(e) => {
            // Non-fatal: might not have worktrees yet
            debug!("Locked worktree check failed (non-fatal): {}", e);
        }
    }

    // SPEC-a70a1ece FR-200: All normal repositories are migration candidates
    // (no longer require .worktrees/ directory to exist)

    // Check if target already exists
    if config.bare_repo_path().exists() {
        result.add_error(MigrationError::InvalidSource {
            reason: format!(
                "Target bare repository already exists: {}",
                config.bare_repo_path().display()
            ),
        });
    }

    // Check write permission to target
    if let Some(parent) = config.target_root.parent() {
        if parent.exists() && !is_writable(parent) {
            result.add_error(MigrationError::PermissionDenied {
                path: parent.to_path_buf(),
            });
        }
    }

    Ok(result)
}

/// Check if a path is writable
fn is_writable(path: &Path) -> bool {
    use std::fs::OpenOptions;
    let test_file = path.join(".gwt-write-test");
    let result = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&test_file);
    if result.is_ok() {
        let _ = std::fs::remove_file(&test_file);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_validation_result_success() {
        let result = ValidationResult::success(1000, 2000);
        assert!(result.passed);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validation_result_add_error() {
        let mut result = ValidationResult::success(1000, 2000);
        result.add_error(MigrationError::Cancelled);
        assert!(!result.passed);
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn test_check_locked_worktrees_empty() {
        let temp = TempDir::new().unwrap();
        // Initialize git repo
        crate::process::git_command()
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        crate::process::git_command()
            .args(["config", "user.email", "test@test.com"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        crate::process::git_command()
            .args(["config", "user.name", "Test"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        let locked = check_locked_worktrees(temp.path()).unwrap();
        assert!(locked.is_empty());
    }

    /// Test that validate_migration succeeds even when disk space check fails
    /// SPEC-a70a1ece: Edge case - df command failure should not block migration
    #[test]
    fn test_validate_migration_continues_on_disk_check_failure() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&target).unwrap();

        // Initialize git repo
        crate::process::git_command()
            .args(["init"])
            .current_dir(&source)
            .output()
            .unwrap();
        crate::process::git_command()
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&source)
            .output()
            .unwrap();
        crate::process::git_command()
            .args(["config", "user.name", "Test"])
            .current_dir(&source)
            .output()
            .unwrap();
        std::fs::write(source.join("README.md"), "# Test").unwrap();
        crate::process::git_command()
            .args(["add", "."])
            .current_dir(&source)
            .output()
            .unwrap();
        crate::process::git_command()
            .args(["commit", "-m", "Initial"])
            .current_dir(&source)
            .output()
            .unwrap();

        let config = crate::migration::MigrationConfig::new(
            source.clone(),
            target.clone(),
            "test.git".to_string(),
        );

        // validate_migration should succeed (or return validation errors that are NOT disk-related)
        // The key is that it doesn't panic or return IoError for df failure
        let result = validate_migration(&config);
        assert!(
            result.is_ok(),
            "validate_migration should not fail due to disk check"
        );
    }

    /// Test that validation does not require .worktrees/ directory
    /// SPEC-a70a1ece FR-200: All normal repos should trigger migration
    #[test]
    fn test_validate_migration_no_worktrees_dir_required() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&target).unwrap();

        // Initialize git repo WITHOUT .worktrees/
        crate::process::git_command()
            .args(["init"])
            .current_dir(&source)
            .output()
            .unwrap();
        crate::process::git_command()
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&source)
            .output()
            .unwrap();
        crate::process::git_command()
            .args(["config", "user.name", "Test"])
            .current_dir(&source)
            .output()
            .unwrap();
        std::fs::write(source.join("README.md"), "# Test").unwrap();
        crate::process::git_command()
            .args(["add", "."])
            .current_dir(&source)
            .output()
            .unwrap();
        crate::process::git_command()
            .args(["commit", "-m", "Initial"])
            .current_dir(&source)
            .output()
            .unwrap();

        // No .worktrees/ directory exists
        assert!(!source.join(".worktrees").exists());

        let config = crate::migration::MigrationConfig::new(
            source.clone(),
            target.clone(),
            "test.git".to_string(),
        );

        let result = validate_migration(&config);
        assert!(result.is_ok(), "validate_migration should succeed");

        // The validation result should pass (no error for missing .worktrees/)
        let validation = result.unwrap();
        let has_worktrees_error = validation.errors.iter().any(|e| {
            matches!(e, MigrationError::InvalidSource { reason } if reason.contains(".worktrees"))
        });
        assert!(
            !has_worktrees_error,
            "Should NOT have error for missing .worktrees/ (FR-200)"
        );
    }
}
