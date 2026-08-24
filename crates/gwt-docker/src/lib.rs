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
    compose_restart, compose_restart_with_files, compose_service_exec_capture,
    compose_service_exec_capture_with_files, compose_service_has_command,
    compose_service_has_command_with_files, compose_service_is_running,
    compose_service_is_running_with_files, compose_service_logs, compose_service_status,
    compose_service_status_with_files, compose_service_user_is_root,
    compose_service_user_is_root_with_files, compose_stop, compose_up, compose_up_force_recreate,
    compose_up_force_recreate_with_files, compose_up_force_recreate_with_files_output,
    compose_up_force_recreate_with_output, compose_up_with_files, compose_up_with_files_output,
    compose_up_with_output, list_containers, restart, start, stop, CommandOutputStream,
    ComposeServiceStatus, ContainerInfo, ContainerStatus,
};
pub use detect::{
    compose_available, container_runtime_kind, daemon_running, detect_docker_files,
    docker_available, launch_preflight, launch_preflight_for_resolved_runtime,
    ContainerRuntimeKind, DockerFiles, DOCKER_HOST_BRIDGE_NAME, DOCKER_HOST_GATEWAY_EXTRA_HOST,
    PODMAN_HOST_BRIDGE_NAME,
};
pub use devcontainer::DevContainerConfig;
pub use port::{check_port_available, PortAllocator, PortMapping};

const EXECUTABLE_FILE_BUSY_RETRIES: usize = 5;
const EXECUTABLE_FILE_BUSY_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(10);

/// Retry the short fork-to-exec window where a generated CLI wrapper's
/// writable fd is still inherited by another child process.
pub(crate) fn retry_executable_file_busy<T>(
    operation: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    retry_executable_file_busy_with_wait(operation, std::thread::sleep)
}

fn retry_executable_file_busy_with_wait<T>(
    mut operation: impl FnMut() -> std::io::Result<T>,
    mut wait: impl FnMut(std::time::Duration),
) -> std::io::Result<T> {
    for attempt in 0..=EXECUTABLE_FILE_BUSY_RETRIES {
        match operation() {
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && attempt < EXECUTABLE_FILE_BUSY_RETRIES =>
            {
                wait(EXECUTABLE_FILE_BUSY_RETRY_DELAY);
            }
            result => return result,
        }
    }
    unreachable!("bounded executable busy retry loop must return")
}

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

#[cfg(test)]
mod tests {
    use std::{io, time::Duration};

    use super::*;

    #[test]
    fn executable_file_busy_is_retried_before_succeeding() {
        let mut attempts = 0;
        let mut waits = Vec::new();

        let result = retry_executable_file_busy_with_wait(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(io::Error::from(io::ErrorKind::ExecutableFileBusy))
                } else {
                    Ok("started")
                }
            },
            |delay| waits.push(delay),
        )
        .expect("transient executable busy error should be retried");

        assert_eq!(result, "started");
        assert_eq!(attempts, 3);
        assert_eq!(waits, vec![Duration::from_millis(10); 2]);
    }

    #[test]
    fn executable_file_busy_retry_is_bounded() {
        let mut attempts = 0;
        let mut waits = 0;

        let error = retry_executable_file_busy_with_wait(
            || {
                attempts += 1;
                Err::<(), _>(io::Error::from(io::ErrorKind::ExecutableFileBusy))
            },
            |_| waits += 1,
        )
        .expect_err("a persistently busy executable must fail after bounded retries");

        assert_eq!(error.kind(), io::ErrorKind::ExecutableFileBusy);
        assert_eq!(attempts, 6);
        assert_eq!(waits, 5);
    }

    #[test]
    fn non_executable_busy_error_is_not_retried() {
        let mut attempts = 0;
        let mut waits = 0;

        let error = retry_executable_file_busy_with_wait(
            || {
                attempts += 1;
                Err::<(), _>(io::Error::from(io::ErrorKind::PermissionDenied))
            },
            |_| waits += 1,
        )
        .expect_err("unrelated spawn error must be returned immediately");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(attempts, 1);
        assert_eq!(waits, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn retry_releases_a_busy_executable_before_the_next_spawn() {
        use std::{fs::OpenOptions, os::unix::fs::PermissionsExt};

        let dir = tempfile::tempdir().expect("create temp dir");
        let script = dir.path().join("busy-script");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").expect("write script");
        let mut permissions = std::fs::metadata(&script)
            .expect("stat script")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).expect("chmod script");

        let writer = OpenOptions::new()
            .write(true)
            .open(&script)
            .expect("hold executable open for writing");
        let mut writer = Some(writer);
        let mut command = gwt_core::process::hidden_command(&script);
        let mut waits = 0;

        let mut child = retry_executable_file_busy_with_wait(
            || command.spawn(),
            |_| {
                waits += 1;
                drop(writer.take());
            },
        )
        .expect("spawn should succeed after the writer closes");
        let status = child.wait().expect("wait for script");

        assert!(status.success());
        assert_eq!(waits, 1, "the first spawn must observe ETXTBSY");
    }
}
