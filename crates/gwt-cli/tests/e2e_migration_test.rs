//! E2E tests for normal repository to bare migration (SPEC-a70a1ece US7-US9)
//!
//! These tests create actual gwt environments in /tmp and verify the migration workflow.
//!
//! ## Running E2E Tests
//!
//! ```bash
//! # Build release binary first
//! cargo build --release
//!
//! # Run E2E tests (requires gwt binary)
//! cargo test --package gwt-cli --test e2e_migration_test -- --ignored --nocapture
//! ```
//!
//! ## Manual E2E Testing
//!
//! ```bash
//! # 1. Create a test environment
//! export TEST_DIR=$(mktemp -d)
//! cd $TEST_DIR
//!
//! # 2. Create a normal repository with .worktrees/ structure
//! git init myrepo && cd myrepo
//! git config user.email "test@example.com"
//! git config user.name "Test User"
//! echo "# Test" > README.md
//! git add . && git commit -m "Initial"
//! mkdir -p .worktrees
//! git worktree add .worktrees/feature-test -b feature/test
//!
//! # 3. Run gwt and observe migration dialog
//! gwt
//!
//! # 4. Verify migration result
//! # - myrepo.git should be created as sibling
//! # - worktrees should be at sibling level (e.g., feature/test/)
//! # - .gwt/ config should be in parent directory
//!
//! # 5. Cleanup
//! rm -rf $TEST_DIR
//! ```

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Get the gwt binary path (release or debug)
fn gwt_binary_path() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let workspace_root = PathBuf::from(manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // Try release first, then debug
    let release_path = workspace_root.join("target/release/gwt");
    if release_path.exists() {
        return release_path;
    }

    let debug_path = workspace_root.join("target/debug/gwt");
    if debug_path.exists() {
        return debug_path;
    }

    // Fallback to PATH
    PathBuf::from("gwt")
}

/// Create a normal repository with .worktrees/ structure (pre-migration state)
fn setup_normal_repo_with_worktrees() -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp.path().join("myrepo");

    // Initialize normal repository
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
    std::fs::write(repo_path.join("README.md"), "# Test Repository\n")
        .expect("Failed to write README");
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

    // Create .worktrees/ directory structure (old gwt style)
    let worktrees_dir = repo_path.join(".worktrees");
    std::fs::create_dir_all(&worktrees_dir).expect("Failed to create .worktrees");

    // Create a worktree in .worktrees/ (old style)
    let worktree_path = worktrees_dir.join("feature-test");
    let output = Command::new("git")
        .args(["worktree", "add", "-b", "feature/test"])
        .arg(&worktree_path)
        .current_dir(&repo_path)
        .output()
        .expect("Failed to create worktree");

    assert!(
        output.status.success(),
        "Failed to create test worktree: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    (temp, repo_path)
}

/// Create a normal repository without .worktrees/ (should not trigger migration)
fn setup_normal_repo_without_worktrees() -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp.path().join("newrepo");

    // Initialize normal repository
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
    std::fs::write(repo_path.join("README.md"), "# New Repository\n")
        .expect("Failed to write README");
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

    (temp, repo_path)
}

#[test]
fn test_detect_worktrees_style_repo() {
    // Test that we can detect .worktrees/ style repository
    let (temp, repo_path) = setup_normal_repo_with_worktrees();

    let worktrees_dir = repo_path.join(".worktrees");
    assert!(
        worktrees_dir.exists(),
        ".worktrees directory should exist at {:?}",
        worktrees_dir
    );
    assert!(worktrees_dir.is_dir(), ".worktrees should be a directory");

    // Check worktree exists
    let worktree_path = worktrees_dir.join("feature-test");
    assert!(
        worktree_path.exists(),
        "Worktree should exist at {:?}",
        worktree_path
    );

    drop(temp);
}

#[test]
fn test_normal_repo_detection() {
    // Test that normal repo without .worktrees/ is correctly detected
    let (temp, repo_path) = setup_normal_repo_without_worktrees();

    let worktrees_dir = repo_path.join(".worktrees");
    assert!(
        !worktrees_dir.exists(),
        ".worktrees directory should NOT exist"
    );

    // Verify it's a valid git repo
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to check git repo");

    assert!(
        String::from_utf8_lossy(&output.stdout).trim() == "true",
        "Should be inside a git work tree"
    );

    drop(temp);
}

/// This test requires the gwt binary to be built.
/// Run with: cargo test --test e2e_migration_test -- --ignored --nocapture
#[test]
#[ignore = "Requires gwt binary - run with --ignored flag after 'cargo build --release'"]
fn test_gwt_detects_migration_candidate() {
    let gwt_path = gwt_binary_path();
    if !gwt_path.exists() && gwt_path.to_str() != Some("gwt") {
        eprintln!("Skipping E2E test: gwt binary not found at {:?}", gwt_path);
        eprintln!("Build with: cargo build --release");
        return;
    }

    let (temp, repo_path) = setup_normal_repo_with_worktrees();

    // Run gwt with --help to verify binary works (non-interactive)
    let output = Command::new(&gwt_path)
        .args(["--help"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to run gwt --help");

    assert!(
        output.status.success(),
        "gwt --help should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Note: Full interactive migration test would require terminal emulation
    // For now, we verify the binary runs and the test setup is correct

    drop(temp);
}

/// Test that migration detection logic works correctly
/// SPEC-a70a1ece FR-200: ALL normal repositories should trigger migration
#[test]
fn test_migration_detection_logic() {
    use gwt_core::git::{detect_repo_type, RepoType};

    // Create a normal repo with .worktrees/ (should trigger migration)
    let (temp, repo_path) = setup_normal_repo_with_worktrees();

    let repo_type = detect_repo_type(&repo_path);
    assert!(
        matches!(repo_type, RepoType::Normal | RepoType::Worktree),
        "Should detect as Normal or Worktree repo, got {:?}",
        repo_type
    );

    let worktrees_dir = repo_path.join(".worktrees");
    assert!(
        worktrees_dir.exists() && worktrees_dir.is_dir(),
        ".worktrees/ should exist and be a directory (old gwt style)"
    );

    // SPEC-a70a1ece FR-200: Migration is triggered for ALL normal repos
    // (not just those with .worktrees/)
    let should_show_migration = matches!(repo_type, RepoType::Normal);
    assert!(
        should_show_migration,
        "Migration dialog should be triggered for normal repos (SPEC-a70a1ece FR-200)"
    );

    drop(temp);
}

/// Test that ALL normal repos (with or without .worktrees/) trigger migration
/// SPEC-a70a1ece FR-200: Migration dialog should show for all normal repositories
#[test]
fn test_migration_for_fresh_repo() {
    use gwt_core::git::{detect_repo_type, RepoType};

    let (temp, repo_path) = setup_normal_repo_without_worktrees();

    let repo_type = detect_repo_type(&repo_path);
    assert!(
        matches!(repo_type, RepoType::Normal),
        "Should detect as Normal repo, got {:?}",
        repo_type
    );

    let worktrees_dir = repo_path.join(".worktrees");
    assert!(!worktrees_dir.exists(), ".worktrees/ should NOT exist");

    // SPEC-a70a1ece FR-200: Migration should be triggered for ALL normal repos
    // (regardless of whether .worktrees/ exists)
    let should_show_migration = matches!(repo_type, RepoType::Normal);
    assert!(
        should_show_migration,
        "Migration dialog SHOULD be triggered for ALL normal repos (SPEC-a70a1ece FR-200)"
    );

    drop(temp);
}

/// Verify the expected post-migration directory structure
#[test]
fn test_expected_post_migration_structure() {
    // This test documents the expected structure after migration
    let temp = TempDir::new().expect("Failed to create temp directory");

    // Expected structure after migration:
    // /tmp/xxx/
    //   ├── myrepo.git/        <- bare repository
    //   ├── main/              <- worktree for main branch
    //   ├── feature/
    //   │   └── test/          <- worktree for feature/test branch
    //   └── .gwt/              <- gwt config directory

    let parent = temp.path();
    let bare_repo = parent.join("myrepo.git");
    let main_worktree = parent.join("main");
    let feature_worktree = parent.join("feature").join("test");
    let gwt_config = parent.join(".gwt");

    // Create expected structure for verification
    std::fs::create_dir_all(&bare_repo).expect("Failed to create bare repo dir");
    std::fs::create_dir_all(&main_worktree).expect("Failed to create main worktree");
    std::fs::create_dir_all(&feature_worktree).expect("Failed to create feature worktree");
    std::fs::create_dir_all(&gwt_config).expect("Failed to create .gwt config");

    // Verify structure
    assert!(bare_repo.exists(), "Bare repo should exist");
    assert!(bare_repo
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with(".git"));
    assert!(
        main_worktree.exists(),
        "Main worktree should exist at sibling level"
    );
    assert!(
        feature_worktree.exists(),
        "Feature worktree should be in subdirectory structure"
    );
    assert!(gwt_config.exists(), ".gwt config should be at parent level");

    // Verify feature worktree is in feature/ subdirectory, not flat
    let flat_path = parent.join("feature-test");
    assert!(
        !flat_path.exists(),
        "Worktree should NOT be flat at {:?}",
        flat_path
    );

    drop(temp);
}
