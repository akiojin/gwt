//! Container information structures (SPEC-f5f5657e)
//!
//! Defines data structures for representing Docker container state.

use std::collections::HashMap;

/// Status of a Docker container
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerStatus {
    /// Container is running
    Running,
    /// Container exists but is stopped
    Stopped,
    /// Container does not exist
    NotFound,
}

impl ContainerStatus {
    /// Check if the container is running
    pub fn is_running(&self) -> bool {
        matches!(self, ContainerStatus::Running)
    }

    /// Check if the container exists (running or stopped)
    pub fn exists(&self) -> bool {
        !matches!(self, ContainerStatus::NotFound)
    }
}

/// Information about a Docker container
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    /// Container ID (short form)
    pub id: String,
    /// Container name
    pub name: String,
    /// Current status
    pub status: ContainerStatus,
    /// Port mappings (host_port -> container_port)
    pub ports: HashMap<u16, u16>,
    /// Service names (for compose)
    pub services: Vec<String>,
}

impl ContainerInfo {
    /// Create a new ContainerInfo
    pub fn new(id: String, name: String, status: ContainerStatus) -> Self {
        Self {
            id,
            name,
            status,
            ports: HashMap::new(),
            services: Vec::new(),
        }
    }

    /// Check if the container is running
    pub fn is_running(&self) -> bool {
        self.status.is_running()
    }

    /// Add a port mapping
    pub fn add_port(&mut self, host_port: u16, container_port: u16) {
        self.ports.insert(host_port, container_port);
    }

    /// Add a service name
    pub fn add_service(&mut self, service: String) {
        self.services.push(service);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_status_is_running() {
        assert!(ContainerStatus::Running.is_running());
        assert!(!ContainerStatus::Stopped.is_running());
        assert!(!ContainerStatus::NotFound.is_running());
    }

    #[test]
    fn test_container_status_exists() {
        assert!(ContainerStatus::Running.exists());
        assert!(ContainerStatus::Stopped.exists());
        assert!(!ContainerStatus::NotFound.exists());
    }

    #[test]
    fn test_container_info_new() {
        let info = ContainerInfo::new(
            "abc123".to_string(),
            "gwt-my-worktree".to_string(),
            ContainerStatus::Running,
        );

        assert_eq!(info.id, "abc123");
        assert_eq!(info.name, "gwt-my-worktree");
        assert!(info.is_running());
        assert!(info.ports.is_empty());
        assert!(info.services.is_empty());
    }

    #[test]
    fn test_container_info_add_port() {
        let mut info = ContainerInfo::new(
            "abc123".to_string(),
            "gwt-test".to_string(),
            ContainerStatus::Running,
        );

        info.add_port(8080, 80);
        info.add_port(3000, 3000);

        assert_eq!(info.ports.len(), 2);
        assert_eq!(info.ports.get(&8080), Some(&80));
        assert_eq!(info.ports.get(&3000), Some(&3000));
    }

    #[test]
    fn test_container_info_add_service() {
        let mut info = ContainerInfo::new(
            "abc123".to_string(),
            "gwt-test".to_string(),
            ContainerStatus::Running,
        );

        info.add_service("web".to_string());
        info.add_service("db".to_string());

        assert_eq!(info.services.len(), 2);
        assert_eq!(info.services[0], "web");
        assert_eq!(info.services[1], "db");
    }

    #[test]
    fn test_container_info_stopped() {
        let info = ContainerInfo::new(
            "def456".to_string(),
            "gwt-stopped".to_string(),
            ContainerStatus::Stopped,
        );

        assert!(!info.is_running());
        assert!(info.status.exists());
    }
}
