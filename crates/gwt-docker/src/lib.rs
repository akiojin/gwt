//! gwt-docker: Docker detection, container management, and DevContainer support.
//!
//! Provides utilities for detecting Docker environments, managing containers,
//! parsing DevContainer and Docker Compose configurations, and allocating ports.

pub mod compose;
pub mod container;
pub mod detect;
pub mod devcontainer;
pub mod port;

pub use compose::{parse_compose_file, ComposeService};
pub use container::{
    compose_restart, compose_service_exec_capture, compose_service_has_command,
    compose_service_is_running, compose_service_logs, compose_service_status,
    compose_service_user_is_root, compose_stop, compose_up, compose_up_force_recreate,
    compose_up_force_recreate_with_output, compose_up_with_output, list_containers, restart, start,
    stop, CommandOutputStream, ComposeServiceStatus, ContainerInfo, ContainerStatus,
};
pub use detect::{
    compose_available, daemon_running, detect_docker_files, docker_available, DockerFiles,
};
pub use devcontainer::DevContainerConfig;
pub use port::{check_port_available, PortAllocator, PortMapping};
