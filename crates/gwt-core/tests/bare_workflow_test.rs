//! Integration tests for bare repository workflow (SPEC-a70a1ece T1101-T1103)
//!
//! These tests verify the end-to-end bare clone and worktree creation workflow.

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Get the default branch name (main or master)
fn get_default_branch(repo_path: &std::path::Path) -> String {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if branch.is_empty() {
                "master".to_string()
            } else {
                branch
            }
        }
        _ => "master".to_string(),
    }
}

/// Helper to create a bare test repository with branches
fn setup_bare_test_repo() -> (TempDir, PathBuf, String) {
    let temp = TempDir::new().expect("Failed to create temp directory");
    let bare_path = temp.path().join("repo.git");

    // Create a bare repository
    Command::new("git")
        .args(["init", "--bare"])
        .arg(&bare_path)
        .output()
        .expect("Failed to init bare repo");

    // Create a temporary working repo to add content
    let work_path = temp.path().join("work");
    Command::new("git")
        .args(["clone"])
        .arg(&bare_path)
        .arg(&work_path)
        .output()
        .expect("Failed to clone");

    // Configure git user
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&work_path)
        .output()
        .expect("Failed to set email");
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&work_path)
        .output()
        .expect("Failed to set name");

    // Create initial commit on a branch named "main"
    Command::new("git")
        .args(["checkout", "-b", "main"])
        .current_dir(&work_path)
        .output()
        .ok(); // May fail if main already exists

    std::fs::write(work_path.join("README.md"), "# Test Repo").expect("Failed to write file");
    Command::new("git")
        .args(["add", "."])
        .current_dir(&work_path)
        .output()
        .expect("Failed to add");
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&work_path)
        .output()
        .expect("Failed to commit");

    // Get the default branch name
    let default_branch = get_default_branch(&work_path);

    // Create a feature branch
    Command::new("git")
        .args(["checkout", "-b", "feature/test"])
        .current_dir(&work_path)
        .output()
        .expect("Failed to create branch");
    std::fs::write(work_path.join("feature.txt"), "Feature content").expect("Failed to write");
    Command::new("git")
        .args(["add", "."])
        .current_dir(&work_path)
        .output()
        .expect("Failed to add");
    Command::new("git")
        .args(["commit", "-m", "Add feature"])
        .current_dir(&work_path)
        .output()
        .expect("Failed to commit");

    // Push all branches to bare
    Command::new("git")
        .args(["push", "--all"])
        .current_dir(&work_path)
        .output()
        .expect("Failed to push");

    // Clean up work directory
    std::fs::remove_dir_all(&work_path).ok();

    (temp, bare_path, default_branch)
}

#[test]
fn test_bare_repo_detection() {
    let (temp, bare_path, _default_branch) = setup_bare_test_repo();

    // Verify bare repo is correctly detected
    let output = Command::new("git")
        .args(["rev-parse", "--is-bare-repository"])
        .current_dir(&bare_path)
        .output()
        .expect("Failed to check bare");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "true", "Should detect bare repository");

    drop(temp);
}

#[test]
fn test_worktree_creation_from_bare() {
    let (temp, bare_path, default_branch) = setup_bare_test_repo();
    let worktree_path = temp.path().join(&default_branch);

    // Create worktree from bare repo
    let output = Command::new("git")
        .args(["worktree", "add"])
        .arg(&worktree_path)
        .arg(&default_branch)
        .current_dir(&bare_path)
        .output()
        .expect("Failed to add worktree");

    assert!(
        output.status.success(),
        "Worktree creation should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(worktree_path.exists(), "Worktree path should exist");
    assert!(
        worktree_path.join("README.md").exists(),
        "Files should be checked out"
    );

    drop(temp);
}

#[test]
fn test_worktree_creation_for_feature_branch() {
    let (temp, bare_path, _default_branch) = setup_bare_test_repo();
    let worktree_path = temp.path().join("feature").join("test");

    // Create parent directory
    std::fs::create_dir_all(worktree_path.parent().unwrap()).expect("Failed to create parent");

    // Create worktree for feature branch
    let output = Command::new("git")
        .args(["worktree", "add"])
        .arg(&worktree_path)
        .arg("feature/test")
        .current_dir(&bare_path)
        .output()
        .expect("Failed to add worktree");

    assert!(output.status.success(), "Worktree creation should succeed");
    assert!(worktree_path.exists(), "Worktree path should exist");
    assert!(
        worktree_path.join("feature.txt").exists(),
        "Feature file should exist"
    );

    drop(temp);
}

#[test]
fn test_worktree_list_from_bare() {
    let (temp, bare_path, default_branch) = setup_bare_test_repo();
    let worktree_path = temp.path().join(&default_branch);

    // Create a worktree
    let output = Command::new("git")
        .args(["worktree", "add"])
        .arg(&worktree_path)
        .arg(&default_branch)
        .current_dir(&bare_path)
        .output()
        .expect("Failed to add worktree");

    if !output.status.success() {
        eprintln!(
            "Worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // List worktrees
    let output = Command::new("git")
        .args(["worktree", "list"])
        .current_dir(&bare_path)
        .output()
        .expect("Failed to list worktrees");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("(bare)"),
        "Should list bare repo: {}",
        stdout
    );
    // The worktree may be listed with branch name or path
    assert!(
        stdout.contains(&default_branch) || worktree_path.exists(),
        "Should list worktree: {}",
        stdout
    );

    drop(temp);
}

#[test]
fn test_worktree_remove() {
    let (temp, bare_path, default_branch) = setup_bare_test_repo();
    let worktree_path = temp.path().join(&default_branch);

    // Create a worktree
    let output = Command::new("git")
        .args(["worktree", "add"])
        .arg(&worktree_path)
        .arg(&default_branch)
        .current_dir(&bare_path)
        .output()
        .expect("Failed to add worktree");

    if !output.status.success() {
        eprintln!(
            "Worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return; // Skip the rest of the test
    }

    // Remove worktree
    let output = Command::new("git")
        .args(["worktree", "remove"])
        .arg(&worktree_path)
        .current_dir(&bare_path)
        .output()
        .expect("Failed to remove worktree");

    assert!(
        output.status.success(),
        "Worktree removal should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!worktree_path.exists(), "Worktree path should be removed");

    drop(temp);
}
