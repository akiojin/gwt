//! Docker environment integration module (gwt-spec issue)
//!
//! Provides automatic Docker container management for coding agents.
//! When Docker files (docker-compose.yml, Dockerfile, .devcontainer) are detected
//! in a worktree, agents are automatically launched inside containers.

pub mod command;
pub mod container;
pub mod detector;
pub mod devcontainer;
pub mod manager;
pub mod port;

pub use command::{compose_available, daemon_running, docker_available, try_start_daemon};
pub use container::{ContainerInfo, ContainerStatus};
pub use detector::{detect_docker_files, DockerFileType};
pub use devcontainer::{normalize_docker_compose_path, DevContainerConfig};
pub use manager::DockerManager;
pub use port::PortAllocator;
