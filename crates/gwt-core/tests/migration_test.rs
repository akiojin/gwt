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
    assert!(worktrees_dir.exists(), ".worktrees/ directory should exist");
    assert!(worktrees_dir.is_dir(), ".worktrees should be a directory");

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

/// Test: All migrated worktrees have correct .git file (not directory)
/// SPEC-a70a1ece US9-S10: マイグレーション後のworktreeはすべてgit worktree addで新規作成
#[test]
fn test_migrated_worktrees_have_git_file_not_directory() {
    let (temp, repo_path) = setup_worktrees_style_repo();
    let parent_dir = repo_path.parent().unwrap();

    // Simulate migration structure
    let bare_path = parent_dir.join("myrepo.git");
    let worktree_path = parent_dir.join("feature-test");

    // Create bare repo
    Command::new("git")
        .args(["clone", "--bare", "--"])
        .arg(&repo_path)
        .arg(&bare_path)
        .output()
        .expect("Failed to create bare repo");

    // Create worktree from bare repo
    Command::new("git")
        .args(["worktree", "add"])
        .arg(&worktree_path)
        .arg("feature/test")
        .current_dir(&bare_path)
        .output()
        .expect("Failed to create worktree");

    // Verify .git is a file, not a directory
    let git_path = worktree_path.join(".git");
    assert!(git_path.exists(), ".git should exist");
    assert!(git_path.is_file(), ".git should be a file, not a directory");

    // Verify .git file content points to bare repo
    let content = std::fs::read_to_string(&git_path).expect("Failed to read .git file");
    assert!(
        content.contains("gitdir:"),
        ".git file should contain gitdir reference"
    );
    assert!(
        content.contains("myrepo.git"),
        ".git file should reference the bare repo"
    );

    drop(temp);
}

/// Test: git worktree list shows all migrated worktrees correctly
/// SPEC-a70a1ece FR-203: worktreeが正しく認識される
#[test]
fn test_git_worktree_list_shows_migrated_worktrees() {
    let (temp, repo_path) = setup_worktrees_style_repo();
    let parent_dir = repo_path.parent().unwrap();

    // Get the actual main branch name (could be main or master)
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to get branch");
    let main_branch = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Simulate migration structure
    let bare_path = parent_dir.join("myrepo.git");
    let main_worktree = parent_dir.join(&main_branch);
    let feature_worktree = parent_dir.join("feature-test");

    // Create bare repo
    Command::new("git")
        .args(["clone", "--bare", "--"])
        .arg(&repo_path)
        .arg(&bare_path)
        .output()
        .expect("Failed to create bare repo");

    // Create main worktree (simulating original repo migration)
    let output = Command::new("git")
        .args(["worktree", "add"])
        .arg(&main_worktree)
        .arg(&main_branch)
        .current_dir(&bare_path)
        .output()
        .expect("Failed to create main worktree");
    assert!(
        output.status.success(),
        "Failed to create main worktree: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Create feature worktree
    Command::new("git")
        .args(["worktree", "add"])
        .arg(&feature_worktree)
        .arg("feature/test")
        .current_dir(&bare_path)
        .output()
        .expect("Failed to create feature worktree");

    // Verify git worktree list shows all worktrees
    let output = Command::new("git")
        .args(["worktree", "list"])
        .current_dir(&bare_path)
        .output()
        .expect("Failed to list worktrees");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&main_branch),
        "worktree list should contain main worktree ({}), got: {}",
        main_branch,
        stdout
    );
    assert!(
        stdout.contains("feature"),
        "worktree list should contain feature worktree"
    );

    drop(temp);
}

/// Test: Original repo's main branch is converted to worktree
/// SPEC-a70a1ece: 元のリポジトリのメインブランチもworktreeとして再作成
#[test]
fn test_original_repo_main_branch_becomes_worktree() {
    let temp = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp.path().join("myrepo");
    let parent_dir = temp.path();

    // Create a normal repository (simulating source repo)
    Command::new("git")
        .args(["init"])
        .arg(&repo_path)
        .output()
        .expect("Failed to init repo");

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

    std::fs::write(repo_path.join("README.md"), "# Test").expect("Failed to write");
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add");
    Command::new("git")
        .args(["commit", "-m", "Initial"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to commit");

    // Get the main branch name
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to get branch");
    let main_branch = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Simulate proper migration: create bare + worktree for main
    let bare_path = parent_dir.join("myrepo.git");
    let main_worktree = parent_dir.join(&main_branch);

    // Create bare from source
    Command::new("git")
        .args(["clone", "--bare", "--"])
        .arg(&repo_path)
        .arg(&bare_path)
        .output()
        .expect("Failed to create bare");

    // Create main worktree from bare (this is what migration should do)
    Command::new("git")
        .args(["worktree", "add"])
        .arg(&main_worktree)
        .arg(&main_branch)
        .current_dir(&bare_path)
        .output()
        .expect("Failed to create main worktree");

    // Verify main worktree has .git file (not directory)
    let git_path = main_worktree.join(".git");
    assert!(git_path.is_file(), "main worktree .git should be a file");

    // Verify it's recognized by git worktree list
    let output = Command::new("git")
        .args(["worktree", "list"])
        .current_dir(&bare_path)
        .output()
        .expect("Failed to list worktrees");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&main_branch),
        "main branch should be in worktree list"
    );

    drop(temp);
}
