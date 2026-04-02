//! Container information and lifecycle management.
//!
//! Provides data structures for representing Docker containers and functions
//! to list, start, stop, and restart them via the Docker CLI.

use gwt_core::{GwtError, Result};
use tracing::debug;

/// Status of a Docker container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerStatus {
    Created,
    Running,
    Paused,
    Stopped,
    Exited,
}

impl ContainerStatus {
    /// Parse a status string from `docker ps --format`.
    pub fn from_docker_state(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "created" => Self::Created,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "exited" => Self::Exited,
            _ => Self::Stopped,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

/// Information about a Docker container.
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    /// Short container ID.
    pub id: String,
    /// Container name.
    pub name: String,
    /// Current status.
    pub status: ContainerStatus,
    /// Image name.
    pub image: String,
    /// Published ports (e.g. "0.0.0.0:8080->80/tcp").
    pub ports: String,
}

/// List all containers (including stopped ones).
pub fn list_containers() -> Result<Vec<ContainerInfo>> {
    let output = std::process::Command::new("docker")
        .args([
            "ps",
            "-a",
            "--format",
            "{{.ID}}\t{{.Names}}\t{{.State}}\t{{.Image}}\t{{.Ports}}",
        ])
        .output()
        .map_err(|e| GwtError::Docker(format!("failed to run docker ps: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GwtError::Docker(format!("docker ps failed: {stderr}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let containers: Vec<ContainerInfo> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(5, '\t').collect();
            if parts.len() < 4 {
                return None;
            }
            Some(ContainerInfo {
                id: parts[0].to_string(),
                name: parts[1].to_string(),
                status: ContainerStatus::from_docker_state(parts[2]),
                image: parts[3].to_string(),
                ports: parts.get(4).unwrap_or(&"").to_string(),
            })
        })
        .collect();

    debug!(
        category = "docker",
        count = containers.len(),
        "listed containers"
    );
    Ok(containers)
}

/// Run a docker lifecycle command (`start`, `stop`, `restart`) on a container.
fn lifecycle(action: &str, id: &str) -> Result<()> {
    let output = std::process::Command::new("docker")
        .args([action, id])
        .output()
        .map_err(|e| GwtError::Docker(format!("failed to {action} container: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GwtError::Docker(format!(
            "docker {action} failed: {stderr}"
        )));
    }
    debug!(
        category = "docker",
        id = id,
        action = action,
        "container lifecycle"
    );
    Ok(())
}

/// Start a container by ID or name.
pub fn start(id: &str) -> Result<()> {
    lifecycle("start", id)
}

/// Stop a container by ID or name.
pub fn stop(id: &str) -> Result<()> {
    lifecycle("stop", id)
}

/// Restart a container by ID or name.
pub fn restart(id: &str) -> Result<()> {
    lifecycle("restart", id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_status_from_docker_state() {
        assert_eq!(
            ContainerStatus::from_docker_state("created"),
            ContainerStatus::Created
        );
        assert_eq!(
            ContainerStatus::from_docker_state("running"),
            ContainerStatus::Running
        );
        assert_eq!(
            ContainerStatus::from_docker_state("Running"),
            ContainerStatus::Running
        );
        assert_eq!(
            ContainerStatus::from_docker_state("paused"),
            ContainerStatus::Paused
        );
        assert_eq!(
            ContainerStatus::from_docker_state("exited"),
            ContainerStatus::Exited
        );
        assert_eq!(
            ContainerStatus::from_docker_state("unknown"),
            ContainerStatus::Stopped
        );
    }

    #[test]
    fn container_status_is_running() {
        assert!(ContainerStatus::Running.is_running());
        assert!(!ContainerStatus::Created.is_running());
        assert!(!ContainerStatus::Paused.is_running());
        assert!(!ContainerStatus::Stopped.is_running());
        assert!(!ContainerStatus::Exited.is_running());
    }

    #[test]
    fn container_info_fields() {
        let info = ContainerInfo {
            id: "abc123".to_string(),
            name: "my-app".to_string(),
            status: ContainerStatus::Running,
            image: "node:18".to_string(),
            ports: "0.0.0.0:3000->3000/tcp".to_string(),
        };
        assert_eq!(info.id, "abc123");
        assert_eq!(info.name, "my-app");
        assert!(info.status.is_running());
        assert_eq!(info.image, "node:18");
        assert_eq!(info.ports, "0.0.0.0:3000->3000/tcp");
    }

    #[test]
    fn parse_docker_ps_line() {
        let line = "abc123\tmy-app\trunning\tnode:18\t0.0.0.0:3000->3000/tcp";
        let parts: Vec<&str> = line.splitn(5, '\t').collect();
        assert_eq!(parts.len(), 5);
        let info = ContainerInfo {
            id: parts[0].to_string(),
            name: parts[1].to_string(),
            status: ContainerStatus::from_docker_state(parts[2]),
            image: parts[3].to_string(),
            ports: parts[4].to_string(),
        };
        assert_eq!(info.id, "abc123");
        assert!(info.status.is_running());
    }

    #[test]
    fn parse_docker_ps_line_no_ports() {
        let line = "def456\tstopped-app\texited\talpine:3.18\t";
        let parts: Vec<&str> = line.splitn(5, '\t').collect();
        let info = ContainerInfo {
            id: parts[0].to_string(),
            name: parts[1].to_string(),
            status: ContainerStatus::from_docker_state(parts[2]),
            image: parts[3].to_string(),
            ports: parts.get(4).unwrap_or(&"").to_string(),
        };
        assert_eq!(info.status, ContainerStatus::Exited);
        assert!(info.ports.is_empty());
    }
}
