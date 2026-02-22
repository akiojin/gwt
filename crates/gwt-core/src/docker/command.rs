//! Docker command wrapper (SPEC-f5f5657e)
//!
//! Provides functions to check Docker availability and daemon status.

use tracing::debug;

/// Check if the `docker` command is available in PATH
pub fn docker_available() -> bool {
    let result = crate::process::command("docker").arg("--version").output();

    match result {
        Ok(output) => {
            let available = output.status.success();
            debug!(
                category = "docker",
                available = available,
                "Checked docker availability"
            );
            available
        }
        Err(e) => {
            debug!(
                category = "docker",
                error = %e,
                "docker command not found"
            );
            false
        }
    }
}

/// Check if `docker compose` is available
pub fn compose_available() -> bool {
    // Try docker compose (v2)
    let result = crate::process::command("docker")
        .args(["compose", "version"])
        .output();

    match result {
        Ok(output) => {
            let available = output.status.success();
            debug!(
                category = "docker",
                available = available,
                "Checked docker compose availability"
            );
            available
        }
        Err(e) => {
            debug!(
                category = "docker",
                error = %e,
                "docker compose not available"
            );
            false
        }
    }
}

/// Check if the Docker daemon is running
pub fn daemon_running() -> bool {
    let result = crate::process::command("docker").arg("info").output();

    match result {
        Ok(output) => {
            let running = output.status.success();
            debug!(
                category = "docker",
                running = running,
                "Checked daemon status"
            );
            running
        }
        Err(e) => {
            debug!(
                category = "docker",
                error = %e,
                "Failed to check daemon status"
            );
            false
        }
    }
}

/// Attempt to start the Docker daemon
///
/// Note: This may require elevated privileges on some systems.
/// Returns Ok(()) if daemon is already running or was started successfully.
pub fn try_start_daemon() -> crate::Result<()> {
    // First check if daemon is already running
    if daemon_running() {
        debug!(category = "docker", "Daemon already running");
        return Ok(());
    }

    // Try to start Docker daemon based on platform
    #[cfg(target_os = "macos")]
    {
        debug!(
            category = "docker",
            "Attempting to start Docker Desktop on macOS"
        );
        let result = crate::process::command("open")
            .args(["-a", "Docker"])
            .output();

        match result {
            Ok(output) if output.status.success() => {
                debug!(category = "docker", "Docker Desktop start command executed");
                // Wait a bit for daemon to start
                std::thread::sleep(std::time::Duration::from_secs(5));
                if daemon_running() {
                    return Ok(());
                }
            }
            _ => {}
        }
    }

    #[cfg(target_os = "linux")]
    {
        debug!(
            category = "docker",
            "Attempting to start Docker daemon on Linux"
        );
        // Try systemctl first
        let result = crate::process::command("systemctl")
            .args(["start", "docker"])
            .output();

        if let Ok(output) = result {
            if output.status.success() {
                debug!(category = "docker", "Docker daemon started via systemctl");
                return Ok(());
            }
        }

        // Try service command as fallback
        let result = crate::process::command("service")
            .args(["docker", "start"])
            .output();

        if let Ok(output) = result {
            if output.status.success() {
                debug!(category = "docker", "Docker daemon started via service");
                return Ok(());
            }
        }
    }

    Err(crate::GwtError::Docker(
        "Failed to start Docker daemon".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-201: docker command availability test
    #[test]
    fn test_docker_available_returns_bool() {
        // This test just verifies the function runs without panic
        // The actual result depends on the environment
        let _result = docker_available();
    }

    // Test compose availability check
    #[test]
    fn test_compose_available_returns_bool() {
        let _result = compose_available();
    }

    // Test daemon status check
    #[test]
    fn test_daemon_running_returns_bool() {
        let _result = daemon_running();
    }

    // Test that docker_available returns false when docker is not installed
    // This is a conditional test that only runs in environments without docker
    #[test]
    #[ignore = "requires environment without docker"]
    fn test_docker_not_available() {
        // This test would be run in a clean environment without Docker
        assert!(!docker_available());
    }
}
