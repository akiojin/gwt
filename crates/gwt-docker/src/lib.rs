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
    compose_restart, compose_restart_with_files, compose_service_exec_attached_args,
    compose_service_exec_capture, compose_service_exec_capture_with_files,
    compose_service_has_command, compose_service_has_command_with_files,
    compose_service_is_running, compose_service_is_running_with_files, compose_service_logs,
    compose_service_status, compose_service_status_with_files, compose_service_user_is_root,
    compose_service_user_is_root_with_files, compose_stop, compose_up, compose_up_force_recreate,
    compose_up_force_recreate_with_files, compose_up_force_recreate_with_files_output,
    compose_up_force_recreate_with_output, compose_up_with_files, compose_up_with_files_output,
    compose_up_with_output, list_containers, restart,
    spawn_compose_service_exec_attached_with_files, start, stop, CommandOutputStream,
    ComposeServiceStatus, ContainerInfo, ContainerStatus,
};
pub use detect::{
    compose_available, container_runtime_kind, daemon_running, detect_docker_files,
    docker_available, launch_preflight, ContainerRuntimeKind, DockerFiles, DOCKER_HOST_BRIDGE_NAME,
    DOCKER_HOST_GATEWAY_EXTRA_HOST, PODMAN_HOST_BRIDGE_NAME,
};
pub use devcontainer::DevContainerConfig;
pub use port::{check_port_available, PortAllocator, PortMapping};

/// Crate-wide lock for tests that mutate the process-global
/// `GWT_DOCKER_BIN` / docker timeout env vars. `detect` and `container`
/// tests previously used module-local locks, which let tests in
/// different modules race on the same env var under the parallel test
/// runner (the suspected source of the #2349 / #3021 fake-docker
/// flakes).
#[cfg(test)]
pub(crate) fn docker_env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}
