//! Docker file detection (SPEC-f5f5657e)
//!
//! Detects Docker-related files in a worktree directory.
//! Detection priority: docker-compose.yml/compose.yml > .devcontainer > Dockerfile

use std::path::{Path, PathBuf};
use tracing::debug;

/// Type of Docker file detected in a worktree
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerFileType {
    /// docker-compose.yml or compose.yml
    Compose(PathBuf),
    /// Dockerfile (no compose file)
    Dockerfile(PathBuf),
    /// .devcontainer/devcontainer.json
    DevContainer(PathBuf),
}

impl DockerFileType {
    /// Get the path to the detected Docker file
    pub fn path(&self) -> &Path {
        match self {
            DockerFileType::Compose(p) => p,
            DockerFileType::Dockerfile(p) => p,
            DockerFileType::DevContainer(p) => p,
        }
    }

    /// Check if this is a Compose file
    pub fn is_compose(&self) -> bool {
        matches!(self, DockerFileType::Compose(_))
    }

    /// Check if this is a Dockerfile
    pub fn is_dockerfile(&self) -> bool {
        matches!(self, DockerFileType::Dockerfile(_))
    }

    /// Check if this is a DevContainer
    pub fn is_devcontainer(&self) -> bool {
        matches!(self, DockerFileType::DevContainer(_))
    }
}

/// Detect Docker files in a worktree directory
///
/// Returns the first detected Docker file type based on priority:
/// 1. docker-compose.yml or compose.yml (Compose)
/// 2. .devcontainer/devcontainer.json (DevContainer)
/// 3. Dockerfile (Dockerfile)
///
/// Returns None if no Docker files are found.
pub fn detect_docker_files(worktree_path: &Path) -> Option<DockerFileType> {
    debug!(
        category = "docker",
        path = %worktree_path.display(),
        "Detecting Docker files"
    );

    // Priority 1: docker-compose.yml or compose.yml
    let compose_files = [
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
    ];
    for filename in compose_files {
        let compose_path = worktree_path.join(filename);
        if compose_path.exists() && compose_path.is_file() {
            debug!(
                category = "docker",
                file = %filename,
                "Found Compose file"
            );
            return Some(DockerFileType::Compose(compose_path));
        }
    }

    // Priority 2: .devcontainer/devcontainer.json
    let devcontainer_path = worktree_path
        .join(".devcontainer")
        .join("devcontainer.json");
    if devcontainer_path.exists() && devcontainer_path.is_file() {
        debug!(category = "docker", "Found devcontainer.json");
        return Some(DockerFileType::DevContainer(devcontainer_path));
    }

    // Priority 3: Dockerfile
    let dockerfile_path = worktree_path.join("Dockerfile");
    if dockerfile_path.exists() && dockerfile_path.is_file() {
        debug!(category = "docker", "Found Dockerfile");
        return Some(DockerFileType::Dockerfile(dockerfile_path));
    }

    debug!(category = "docker", "No Docker files found");
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // T-101: docker-compose.yml detection test
    #[test]
    fn test_detect_docker_compose_yml() {
        let temp_dir = TempDir::new().unwrap();
        let compose_path = temp_dir.path().join("docker-compose.yml");
        std::fs::write(&compose_path, "version: '3'").unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_some());
        let docker_type = result.unwrap();
        assert!(docker_type.is_compose());
        assert_eq!(docker_type.path(), compose_path);
    }

    // T-101: docker-compose.yaml detection test (alternative extension)
    #[test]
    fn test_detect_docker_compose_yaml() {
        let temp_dir = TempDir::new().unwrap();
        let compose_path = temp_dir.path().join("docker-compose.yaml");
        std::fs::write(&compose_path, "version: '3'").unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_some());
        let docker_type = result.unwrap();
        assert!(docker_type.is_compose());
    }

    // T-102: compose.yml detection test
    #[test]
    fn test_detect_compose_yml() {
        let temp_dir = TempDir::new().unwrap();
        let compose_path = temp_dir.path().join("compose.yml");
        std::fs::write(&compose_path, "version: '3'").unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_some());
        let docker_type = result.unwrap();
        assert!(docker_type.is_compose());
        assert_eq!(docker_type.path(), compose_path);
    }

    // T-102: compose.yaml detection test (alternative extension)
    #[test]
    fn test_detect_compose_yaml() {
        let temp_dir = TempDir::new().unwrap();
        let compose_path = temp_dir.path().join("compose.yaml");
        std::fs::write(&compose_path, "version: '3'").unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_some());
        let docker_type = result.unwrap();
        assert!(docker_type.is_compose());
    }

    // T-103: Dockerfile detection test
    #[test]
    fn test_detect_dockerfile() {
        let temp_dir = TempDir::new().unwrap();
        let dockerfile_path = temp_dir.path().join("Dockerfile");
        std::fs::write(&dockerfile_path, "FROM ubuntu:22.04").unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_some());
        let docker_type = result.unwrap();
        assert!(docker_type.is_dockerfile());
        assert_eq!(docker_type.path(), dockerfile_path);
    }

    // T-104: .devcontainer detection test
    #[test]
    fn test_detect_devcontainer() {
        let temp_dir = TempDir::new().unwrap();
        let devcontainer_dir = temp_dir.path().join(".devcontainer");
        std::fs::create_dir(&devcontainer_dir).unwrap();
        let devcontainer_path = devcontainer_dir.join("devcontainer.json");
        std::fs::write(&devcontainer_path, r#"{"name": "test"}"#).unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_some());
        let docker_type = result.unwrap();
        assert!(docker_type.is_devcontainer());
        assert_eq!(docker_type.path(), devcontainer_path);
    }

    // T-105: Priority test - compose.yml takes priority over Dockerfile
    #[test]
    fn test_priority_compose_over_dockerfile() {
        let temp_dir = TempDir::new().unwrap();

        // Create both files
        std::fs::write(temp_dir.path().join("docker-compose.yml"), "version: '3'").unwrap();
        std::fs::write(temp_dir.path().join("Dockerfile"), "FROM ubuntu:22.04").unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_some());
        let docker_type = result.unwrap();
        // Compose should take priority
        assert!(docker_type.is_compose());
    }

    // T-105: Priority test - devcontainer takes priority over Dockerfile
    #[test]
    fn test_priority_devcontainer_over_dockerfile() {
        let temp_dir = TempDir::new().unwrap();

        // Create Dockerfile
        std::fs::write(temp_dir.path().join("Dockerfile"), "FROM ubuntu:22.04").unwrap();

        // Create devcontainer
        let devcontainer_dir = temp_dir.path().join(".devcontainer");
        std::fs::create_dir(&devcontainer_dir).unwrap();
        std::fs::write(
            devcontainer_dir.join("devcontainer.json"),
            r#"{"name": "test"}"#,
        )
        .unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_some());
        let docker_type = result.unwrap();
        // devcontainer should take priority over Dockerfile
        assert!(docker_type.is_devcontainer());
    }

    // T-105: Priority test - compose takes priority over all
    #[test]
    fn test_priority_compose_over_all() {
        let temp_dir = TempDir::new().unwrap();

        // Create all files
        std::fs::write(temp_dir.path().join("compose.yml"), "version: '3'").unwrap();
        std::fs::write(temp_dir.path().join("Dockerfile"), "FROM ubuntu:22.04").unwrap();
        let devcontainer_dir = temp_dir.path().join(".devcontainer");
        std::fs::create_dir(&devcontainer_dir).unwrap();
        std::fs::write(
            devcontainer_dir.join("devcontainer.json"),
            r#"{"name": "test"}"#,
        )
        .unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_some());
        let docker_type = result.unwrap();
        // Compose should take priority over all
        assert!(docker_type.is_compose());
    }

    // Test: No Docker files
    #[test]
    fn test_no_docker_files() {
        let temp_dir = TempDir::new().unwrap();
        // Create some non-Docker files
        std::fs::write(temp_dir.path().join("README.md"), "# Test").unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_none());
    }

    // Test: Empty directory
    #[test]
    fn test_empty_directory() {
        let temp_dir = TempDir::new().unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_none());
    }

    // Test: Directory named Dockerfile (should not match)
    #[test]
    fn test_dockerfile_directory_ignored() {
        let temp_dir = TempDir::new().unwrap();
        let dockerfile_dir = temp_dir.path().join("Dockerfile");
        std::fs::create_dir(&dockerfile_dir).unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_none());
    }

    // Test: docker-compose.yml priority over compose.yml
    #[test]
    fn test_docker_compose_priority_over_compose() {
        let temp_dir = TempDir::new().unwrap();

        // Create both compose files
        std::fs::write(temp_dir.path().join("docker-compose.yml"), "version: '3'").unwrap();
        std::fs::write(temp_dir.path().join("compose.yml"), "version: '3'").unwrap();

        let result = detect_docker_files(temp_dir.path());
        assert!(result.is_some());
        let docker_type = result.unwrap();
        assert!(docker_type.is_compose());
        // docker-compose.yml should be detected first
        assert!(docker_type.path().ends_with("docker-compose.yml"));
    }

    // Test: DockerFileType methods
    #[test]
    fn test_docker_file_type_methods() {
        let compose = DockerFileType::Compose(PathBuf::from("docker-compose.yml"));
        assert!(compose.is_compose());
        assert!(!compose.is_dockerfile());
        assert!(!compose.is_devcontainer());
        assert_eq!(compose.path(), Path::new("docker-compose.yml"));

        let dockerfile = DockerFileType::Dockerfile(PathBuf::from("Dockerfile"));
        assert!(!dockerfile.is_compose());
        assert!(dockerfile.is_dockerfile());
        assert!(!dockerfile.is_devcontainer());

        let devcontainer =
            DockerFileType::DevContainer(PathBuf::from(".devcontainer/devcontainer.json"));
        assert!(!devcontainer.is_compose());
        assert!(!devcontainer.is_dockerfile());
        assert!(devcontainer.is_devcontainer());
    }
}
