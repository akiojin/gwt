//! Docker integration tests (SPEC-f5f5657e)
//!
//! These tests verify the complete Docker workflow including:
//! - Docker file detection
//! - DockerManager creation and configuration
//! - Container naming
//! - Environment passthrough configuration
//!
//! Note: Tests that require actual Docker daemon are marked with #[ignore]
//! and should be run manually in an environment with Docker available.

use gwt_core::docker::{detect_docker_files, DevContainerConfig, DockerManager, PortAllocator};
use tempfile::TempDir;

// =============================================================================
// E2E Test: Docker file detection workflow
// =============================================================================

/// Test complete workflow: detect -> create manager -> generate command
#[test]
fn test_docker_workflow_with_compose() {
    let temp_dir = TempDir::new().unwrap();
    let worktree_path = temp_dir.path();

    // Create docker-compose.yml
    std::fs::write(
        worktree_path.join("docker-compose.yml"),
        r#"
version: '3'
services:
  app:
    image: node:18
    working_dir: /workspace
    volumes:
      - .:/workspace
"#,
    )
    .unwrap();

    // Step 1: Detect Docker files
    let docker_type = detect_docker_files(worktree_path);
    assert!(docker_type.is_some());
    let docker_type = docker_type.unwrap();
    assert!(docker_type.is_compose());

    // Step 2: Create DockerManager
    let manager = DockerManager::new(worktree_path, "test-worktree", docker_type);
    assert_eq!(manager.container_name(), "gwt-test-worktree");
    assert!(manager.docker_file_type().is_compose());
}

/// Test workflow with Dockerfile only
#[test]
fn test_docker_workflow_with_dockerfile() {
    let temp_dir = TempDir::new().unwrap();
    let worktree_path = temp_dir.path();

    // Create Dockerfile
    std::fs::write(
        worktree_path.join("Dockerfile"),
        r#"
FROM ubuntu:22.04
WORKDIR /workspace
"#,
    )
    .unwrap();

    // Detect and create manager
    let docker_type = detect_docker_files(worktree_path).unwrap();
    assert!(docker_type.is_dockerfile());

    let manager = DockerManager::new(worktree_path, "dockerfile-test", docker_type);
    assert_eq!(manager.container_name(), "gwt-dockerfile-test");
}

/// Test workflow with devcontainer
#[test]
fn test_docker_workflow_with_devcontainer() {
    let temp_dir = TempDir::new().unwrap();
    let worktree_path = temp_dir.path();

    // Create .devcontainer directory and devcontainer.json
    let devcontainer_dir = worktree_path.join(".devcontainer");
    std::fs::create_dir(&devcontainer_dir).unwrap();
    std::fs::write(
        devcontainer_dir.join("devcontainer.json"),
        r#"{
            "name": "Test Dev Container",
            "image": "mcr.microsoft.com/devcontainers/base:ubuntu"
        }"#,
    )
    .unwrap();

    // Detect devcontainer
    let docker_type = detect_docker_files(worktree_path).unwrap();
    assert!(docker_type.is_devcontainer());

    // Parse devcontainer.json
    let devcontainer_path = devcontainer_dir.join("devcontainer.json");
    let config = DevContainerConfig::load(&devcontainer_path).unwrap();
    assert_eq!(config.name, Some("Test Dev Container".to_string()));
}

// =============================================================================
// E2E Test: Priority detection
// =============================================================================

/// Test priority: compose > Dockerfile > devcontainer
#[test]
fn test_detection_priority_all_present() {
    let temp_dir = TempDir::new().unwrap();
    let worktree_path = temp_dir.path();

    // Create all Docker file types
    std::fs::write(worktree_path.join("docker-compose.yml"), "version: '3'").unwrap();
    std::fs::write(worktree_path.join("Dockerfile"), "FROM ubuntu").unwrap();

    let devcontainer_dir = worktree_path.join(".devcontainer");
    std::fs::create_dir(&devcontainer_dir).unwrap();
    std::fs::write(
        devcontainer_dir.join("devcontainer.json"),
        r#"{"image": "ubuntu"}"#,
    )
    .unwrap();

    // Compose should have highest priority
    let docker_type = detect_docker_files(worktree_path).unwrap();
    assert!(docker_type.is_compose());
}

/// Test priority: devcontainer > Dockerfile (no compose)
#[test]
fn test_detection_priority_devcontainer_over_dockerfile() {
    let temp_dir = TempDir::new().unwrap();
    let worktree_path = temp_dir.path();

    // Create Dockerfile and devcontainer (no compose)
    std::fs::write(worktree_path.join("Dockerfile"), "FROM ubuntu").unwrap();

    let devcontainer_dir = worktree_path.join(".devcontainer");
    std::fs::create_dir(&devcontainer_dir).unwrap();
    std::fs::write(
        devcontainer_dir.join("devcontainer.json"),
        r#"{"image": "ubuntu"}"#,
    )
    .unwrap();

    // devcontainer should have priority over Dockerfile
    let docker_type = detect_docker_files(worktree_path).unwrap();
    assert!(docker_type.is_devcontainer());
}

// =============================================================================
// E2E Test: Container naming for various branch names
// =============================================================================

#[test]
fn test_container_naming_feature_branches() {
    let test_cases = vec![
        ("main", "gwt-main"),
        ("develop", "gwt-develop"),
        ("feature/add-login", "gwt-feature-add-login"),
        (
            "feature/JIRA-123/implement",
            "gwt-feature-jira-123-implement",
        ),
        ("bugfix/fix-crash", "gwt-bugfix-fix-crash"),
        ("release/v1.0.0", "gwt-release-v1-0-0"),
        ("user@branch", "gwt-user-branch"),
        ("branch with spaces", "gwt-branch-with-spaces"),
        ("UPPERCASE", "gwt-uppercase"),
        ("---leading-trailing---", "gwt-leading-trailing"),
    ];

    for (input, expected) in test_cases {
        let name = DockerManager::generate_container_name(input);
        assert_eq!(name, expected, "Failed for input: {}", input);
    }
}

// =============================================================================
// E2E Test: Port allocation
// =============================================================================

#[test]
fn test_port_allocation_workflow() {
    let allocator = PortAllocator::new();

    // Allocate ports for typical web development
    let ports =
        allocator.allocate_ports(&[("WEB_PORT", 3000), ("API_PORT", 8080), ("DB_PORT", 5432)]);

    assert_eq!(ports.len(), 3);
    assert!(ports.contains_key("WEB_PORT"));
    assert!(ports.contains_key("API_PORT"));
    assert!(ports.contains_key("DB_PORT"));

    // All ports should be different
    let port_values: Vec<_> = ports.values().collect();
    let mut unique_ports = port_values.clone();
    unique_ports.sort();
    unique_ports.dedup();
    assert_eq!(port_values.len(), unique_ports.len());
}

// =============================================================================
// E2E Test: devcontainer with compose
// =============================================================================

#[test]
fn test_devcontainer_with_compose_file() {
    let temp_dir = TempDir::new().unwrap();
    let devcontainer_dir = temp_dir.path().join(".devcontainer");
    std::fs::create_dir(&devcontainer_dir).unwrap();

    // Create devcontainer.json that references docker-compose
    std::fs::write(
        devcontainer_dir.join("devcontainer.json"),
        r#"{
            "name": "Full Stack Dev",
            "dockerComposeFile": ["docker-compose.yml", "docker-compose.dev.yml"],
            "service": "app",
            "workspaceFolder": "/workspace",
            "forwardPorts": [3000, 5432]
        }"#,
    )
    .unwrap();

    let config = DevContainerConfig::load(&devcontainer_dir.join("devcontainer.json")).unwrap();

    assert!(config.uses_compose());
    assert!(!config.uses_dockerfile());
    assert_eq!(config.get_service(), Some("app"));
    assert_eq!(
        config.get_compose_files(),
        vec!["docker-compose.yml", "docker-compose.dev.yml"]
    );
    assert_eq!(config.get_forward_ports(), vec![3000, 5432]);
}

// =============================================================================
// E2E Test: No Docker environment (fallback scenario)
// =============================================================================

#[test]
fn test_no_docker_environment() {
    let temp_dir = TempDir::new().unwrap();
    let worktree_path = temp_dir.path();

    // Create only regular files (no Docker)
    std::fs::write(worktree_path.join("README.md"), "# My Project").unwrap();
    std::fs::write(worktree_path.join("main.rs"), "fn main() {}").unwrap();

    // Should not detect any Docker files
    let docker_type = detect_docker_files(worktree_path);
    assert!(docker_type.is_none());
}

// =============================================================================
// Integration tests requiring Docker daemon (marked as ignored)
// =============================================================================

/// Test actual Docker compose up/down (requires Docker daemon)
#[test]
#[ignore = "requires Docker daemon"]
fn test_docker_start_stop_integration() {
    let temp_dir = TempDir::new().unwrap();
    let worktree_path = temp_dir.path();

    // Create a simple docker-compose.yml
    std::fs::write(
        worktree_path.join("docker-compose.yml"),
        r#"
version: '3'
services:
  test:
    image: alpine:latest
    command: sleep 30
"#,
    )
    .unwrap();

    let docker_type = detect_docker_files(worktree_path).unwrap();
    let manager = DockerManager::new(worktree_path, "integration-test", docker_type);

    // Start container
    let info = manager.start().expect("Failed to start container");
    assert!(info.is_running());

    // Verify running
    assert!(manager.is_running());

    // Stop container
    manager.stop().expect("Failed to stop container");
    assert!(!manager.is_running());
}

/// Test Docker environment with retry (requires Docker daemon)
#[test]
#[ignore = "requires Docker daemon"]
fn test_docker_start_with_retry_integration() {
    let temp_dir = TempDir::new().unwrap();
    let worktree_path = temp_dir.path();

    std::fs::write(
        worktree_path.join("docker-compose.yml"),
        r#"
version: '3'
services:
  test:
    image: alpine:latest
    command: sleep 10
"#,
    )
    .unwrap();

    let docker_type = detect_docker_files(worktree_path).unwrap();
    let manager = DockerManager::new(worktree_path, "retry-test", docker_type);

    // Start with retry
    let info = manager
        .start_with_retry()
        .expect("Failed to start with retry");
    assert!(info.is_running());

    // Cleanup
    manager.stop().expect("Failed to stop container");
}
