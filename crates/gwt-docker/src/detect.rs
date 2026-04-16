//! Docker environment detection.
//!
//! Checks for Docker CLI availability, daemon status, and discovers
//! Docker-related files (Dockerfile, docker-compose.yml, .devcontainer/).

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use tracing::debug;

/// Detected Docker files in a directory.
#[derive(Debug, Clone, Default)]
pub struct DockerFiles {
    /// Path to Dockerfile, if found.
    pub dockerfile: Option<PathBuf>,
    /// Path to docker-compose.yml (or compose.yml variant), if found.
    pub compose_file: Option<PathBuf>,
    /// Path to .devcontainer/ directory, if found.
    pub devcontainer_dir: Option<PathBuf>,
}

impl DockerFiles {
    /// Returns true if any Docker files were detected.
    pub fn any_found(&self) -> bool {
        self.dockerfile.is_some() || self.compose_file.is_some() || self.devcontainer_dir.is_some()
    }
}

/// Run a docker sub-command and return whether it succeeded.
fn docker_probe(args: &[&str], label: &str) -> bool {
    let result = std::process::Command::new(docker_binary())
        .args(args)
        .output();
    match result {
        Ok(output) => {
            let ok = output.status.success();
            debug!(category = "docker", ok = ok, label = label, "probe");
            ok
        }
        Err(e) => {
            debug!(category = "docker", error = %e, label = label, "probe failed");
            false
        }
    }
}

fn docker_binary() -> OsString {
    std::env::var_os("GWT_DOCKER_BIN").unwrap_or_else(|| OsString::from("docker"))
}

/// Check if the `docker` command is available in PATH.
pub fn docker_available() -> bool {
    docker_probe(&["--version"], "docker CLI")
}

/// Check if `docker compose` (v2) is available.
pub fn compose_available() -> bool {
    docker_probe(&["compose", "version"], "docker compose")
}

/// Check if the Docker daemon is running.
pub fn daemon_running() -> bool {
    docker_probe(&["info"], "daemon")
}

/// Detect Docker-related files in a directory.
///
/// Scans for Dockerfile, docker-compose.yml / compose.yml variants,
/// and .devcontainer/ directory.
pub fn detect_docker_files(dir: &Path) -> DockerFiles {
    let mut files = DockerFiles::default();

    // Compose files (check common variants)
    let compose_names = [
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
    ];
    for name in compose_names {
        let p = dir.join(name);
        if p.is_file() {
            debug!(category = "docker", file = %name, "found compose file");
            files.compose_file = Some(p);
            break;
        }
    }

    // Dockerfile
    let dockerfile = dir.join("Dockerfile");
    if dockerfile.is_file() {
        debug!(category = "docker", "found Dockerfile");
        files.dockerfile = Some(dockerfile);
    }

    // .devcontainer/
    let devcontainer = dir.join(".devcontainer");
    if devcontainer.is_dir() {
        debug!(category = "docker", "found .devcontainer/");
        files.devcontainer_dir = Some(devcontainer);
    }

    files
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn detect_empty_dir_finds_nothing() {
        let tmp = TempDir::new().unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(!files.any_found());
        assert!(files.dockerfile.is_none());
        assert!(files.compose_file.is_none());
        assert!(files.devcontainer_dir.is_none());
    }

    #[test]
    fn detect_dockerfile() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Dockerfile"), "FROM ubuntu:22.04").unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.any_found());
        assert!(files.dockerfile.is_some());
        assert!(files.compose_file.is_none());
    }

    #[test]
    fn detect_docker_compose_yml() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("docker-compose.yml"), "version: '3'").unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.any_found());
        assert!(files.compose_file.is_some());
        assert!(files
            .compose_file
            .as_ref()
            .unwrap()
            .ends_with("docker-compose.yml"));
    }

    #[test]
    fn detect_compose_yml() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("compose.yml"), "version: '3'").unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.compose_file.is_some());
    }

    #[test]
    fn detect_devcontainer_dir() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".devcontainer")).unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.any_found());
        assert!(files.devcontainer_dir.is_some());
    }

    #[test]
    fn detect_all_docker_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Dockerfile"), "FROM node:18").unwrap();
        std::fs::write(tmp.path().join("docker-compose.yml"), "version: '3'").unwrap();
        std::fs::create_dir(tmp.path().join(".devcontainer")).unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.dockerfile.is_some());
        assert!(files.compose_file.is_some());
        assert!(files.devcontainer_dir.is_some());
    }

    #[test]
    fn docker_compose_yml_takes_priority_over_compose_yml() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("docker-compose.yml"), "version: '3'").unwrap();
        std::fs::write(tmp.path().join("compose.yml"), "version: '3'").unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files
            .compose_file
            .as_ref()
            .unwrap()
            .ends_with("docker-compose.yml"));
    }

    #[test]
    fn directory_named_dockerfile_is_ignored() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("Dockerfile")).unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.dockerfile.is_none());
    }

    // Smoke tests — just verify the functions return without panic.
    #[test]
    fn docker_available_returns_bool() {
        let _ = docker_available();
    }

    #[test]
    fn compose_available_returns_bool() {
        let _ = compose_available();
    }

    #[test]
    fn daemon_running_returns_bool() {
        let _ = daemon_running();
    }
}
