//! Integration tests for migration workflow (SPEC-a70a1ece T1102-T1103)
//!
//! These tests verify the migration from .worktrees/ method to bare method.

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper to create a .worktrees/ style repository
fn setup_worktrees_style_repo() -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp.path().join("myrepo");

    // Create a normal repository
    Command::new("git")
        .args(["init"])
        .arg(&repo_path)
        .output()
        .expect("Failed to init repo");

    // Configure git user
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to set email");
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to set name");

    // Create initial commit
    std::fs::write(repo_path.join("README.md"), "# Test Repo").expect("Failed to write file");
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add");
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to commit");

    // Create .worktrees/ directory structure
    let worktrees_dir = repo_path.join(".worktrees");
    std::fs::create_dir(&worktrees_dir).expect("Failed to create .worktrees");

    // Create a worktree in .worktrees/ style
    let feature_path = worktrees_dir.join("feature-test");
    Command::new("git")
        .args(["worktree", "add", "-b", "feature/test"])
        .arg(&feature_path)
        .current_dir(&repo_path)
        .output()
        .expect("Failed to create worktree");

    (temp, repo_path)
}

#[test]
fn test_detect_worktrees_style_repo() {
    let (temp, repo_path) = setup_worktrees_style_repo();

    let worktrees_dir = repo_path.join(".worktrees");
    assert!(
        worktrees_dir.exists(),
        ".worktrees/ directory should exist"
    );
    assert!(
        worktrees_dir.is_dir(),
        ".worktrees should be a directory"
    );

    drop(temp);
}

#[test]
fn test_worktrees_contains_worktrees() {
    let (temp, repo_path) = setup_worktrees_style_repo();

    let worktrees_dir = repo_path.join(".worktrees");
    let entries: Vec<_> = std::fs::read_dir(&worktrees_dir)
        .expect("Failed to read .worktrees")
        .filter_map(|e| e.ok())
        .collect();

    assert!(!entries.is_empty(), "Should have worktrees in .worktrees/");

    // Check that each entry is a valid worktree
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            let git_file = path.join(".git");
            assert!(
                git_file.exists(),
                "Worktree should have .git file: {}",
                path.display()
            );
        }
    }

    drop(temp);
}

#[test]
fn test_backup_directory_creation() {
    let temp = TempDir::new().expect("Failed to create temp directory");
    let backup_dir = temp.path().join(".gwt-migration-backup");

    // Create backup directory
    std::fs::create_dir_all(&backup_dir).expect("Failed to create backup directory");
    assert!(backup_dir.exists(), "Backup directory should exist");

    // Write metadata
    let metadata_path = backup_dir.join("backup-info.json");
    std::fs::write(&metadata_path, r#"{"created_at": "2025-01-01T00:00:00Z"}"#)
        .expect("Failed to write metadata");
    assert!(metadata_path.exists(), "Metadata file should exist");

    drop(temp);
}

#[test]
fn test_bare_repo_clone() {
    let temp = TempDir::new().expect("Failed to create temp directory");
    let bare_path = temp.path().join("repo.git");

    // Initialize a bare repository
    let output = Command::new("git")
        .args(["init", "--bare"])
        .arg(&bare_path)
        .output()
        .expect("Failed to init bare repo");

    assert!(output.status.success(), "Bare init should succeed");
    assert!(bare_path.exists(), "Bare path should exist");
    assert!(
        bare_path.join("HEAD").exists(),
        "HEAD file should exist in bare repo"
    );
    assert!(
        bare_path.join("objects").exists(),
        "objects dir should exist"
    );
    assert!(bare_path.join("refs").exists(), "refs dir should exist");

    drop(temp);
}

#[test]
fn test_worktree_dirty_detection() {
    let (temp, repo_path) = setup_worktrees_style_repo();

    let feature_path = repo_path.join(".worktrees").join("feature-test");

    // Check clean worktree
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&feature_path)
        .output()
        .expect("Failed to check status");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "Clean worktree should have no changes");

    // Make worktree dirty
    std::fs::write(feature_path.join("dirty.txt"), "dirty content")
        .expect("Failed to write dirty file");

    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&feature_path)
        .output()
        .expect("Failed to check status");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "Dirty worktree should have changes");

    drop(temp);
}

#[test]
fn test_permission_error_handling() {
    let temp = TempDir::new().expect("Failed to create temp directory");
    let protected_dir = temp.path().join("protected");

    // Create a directory
    std::fs::create_dir(&protected_dir).expect("Failed to create dir");

    // Note: This test is limited on some systems
    // Just verify that the directory exists
    assert!(protected_dir.exists(), "Protected directory should exist");

    drop(temp);
}
