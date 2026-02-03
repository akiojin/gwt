//! Docker container manager (SPEC-f5f5657e)
//!
//! Manages Docker containers for worktrees, including startup, shutdown,
//! and executing commands inside containers.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use tracing::{debug, info, warn};

use super::container::{ContainerInfo, ContainerStatus};
use super::detector::DockerFileType;
use crate::{GwtError, Result};

/// Environment variable prefixes to pass through to containers
const ENV_PASSTHROUGH_PREFIXES: &[&str] = &[
    "ANTHROPIC_",
    "OPENAI_",
    "GEMINI_",
    "GOOGLE_",
    "GITHUB_",
    "GIT_",
    "SSH_AUTH_SOCK",
    "HOME",
    "USER",
    "SHELL",
];

/// Maximum number of retry attempts for Docker operations
const MAX_RETRY_ATTEMPTS: u32 = 3;

/// Retry delay in seconds (increases: 2s, 5s)
const RETRY_DELAYS_SECS: &[u64] = &[2, 5];

/// Check if an error is retryable
fn is_retryable_error(error: &GwtError) -> bool {
    match error {
        GwtError::DockerDaemonNotRunning => true,
        GwtError::DockerTimeout => true,
        GwtError::Docker(msg) => {
            // Network-related errors are typically retryable
            msg.contains("connection refused")
                || msg.contains("timeout")
                || msg.contains("network")
                || msg.contains("temporary")
        }
        GwtError::DockerStartFailed { reason } => {
            reason.contains("network") || reason.contains("timeout")
        }
        _ => false,
    }
}

/// Execute a Docker operation with retry logic
fn with_retry<T, F>(operation_name: &str, mut operation: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let mut last_error = None;

    for attempt in 0..MAX_RETRY_ATTEMPTS {
        match operation() {
            Ok(result) => return Ok(result),
            Err(e) => {
                if !is_retryable_error(&e) || attempt == MAX_RETRY_ATTEMPTS - 1 {
                    return Err(e);
                }

                let delay = RETRY_DELAYS_SECS
                    .get(attempt as usize)
                    .copied()
                    .unwrap_or(5);

                warn!(
                    category = "docker",
                    operation = operation_name,
                    attempt = attempt + 1,
                    max_attempts = MAX_RETRY_ATTEMPTS,
                    delay_secs = delay,
                    error = %e,
                    "Docker operation failed, retrying"
                );

                std::thread::sleep(std::time::Duration::from_secs(delay));
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| GwtError::Docker("Unknown error".to_string())))
}

/// Manager for Docker containers associated with a worktree
#[derive(Debug)]
pub struct DockerManager {
    /// Path to the worktree directory
    worktree_path: PathBuf,
    /// Generated container name (gwt-{sanitized_worktree_name})
    container_name: String,
    /// Type of Docker file detected
    docker_file_type: DockerFileType,
    /// Timestamp when container was last built (for rebuild detection)
    last_build_time: Option<SystemTime>,
}

impl DockerManager {
    /// Create a new DockerManager for a worktree
    ///
    /// # Arguments
    /// * `worktree_path` - Path to the worktree directory
    /// * `worktree_name` - Name of the worktree (used for container naming)
    /// * `docker_file_type` - Type of Docker file detected
    pub fn new(worktree_path: &Path, worktree_name: &str, docker_file_type: DockerFileType) -> Self {
        let container_name = Self::generate_container_name(worktree_name);
        debug!(
            category = "docker",
            worktree = %worktree_path.display(),
            container_name = %container_name,
            "Created DockerManager"
        );

        Self {
            worktree_path: worktree_path.to_path_buf(),
            container_name,
            docker_file_type,
            last_build_time: None,
        }
    }

    /// Generate a sanitized container name from worktree name
    ///
    /// Container names must only contain alphanumeric characters, hyphens, and underscores.
    /// Format: gwt-{sanitized_worktree_name}
    pub fn generate_container_name(worktree_name: &str) -> String {
        let sanitized: String = worktree_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();

        // Remove leading/trailing hyphens and collapse multiple hyphens
        let sanitized = sanitized.trim_matches('-');
        let mut result = String::with_capacity(sanitized.len());
        let mut prev_hyphen = false;

        for c in sanitized.chars() {
            if c == '-' {
                if !prev_hyphen {
                    result.push(c);
                    prev_hyphen = true;
                }
            } else {
                result.push(c);
                prev_hyphen = false;
            }
        }

        format!("gwt-{}", result)
    }

    /// Get the container name
    pub fn container_name(&self) -> &str {
        &self.container_name
    }

    /// Get the worktree path
    pub fn worktree_path(&self) -> &Path {
        &self.worktree_path
    }

    /// Get the Docker file type
    pub fn docker_file_type(&self) -> &DockerFileType {
        &self.docker_file_type
    }

    /// Check if the container is currently running
    pub fn is_running(&self) -> bool {
        let output = Command::new("docker")
            .args(["compose", "ps", "-q"])
            .current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name)
            .output();

        match output {
            Ok(out) => {
                let is_running = out.status.success() && !out.stdout.is_empty();
                debug!(
                    category = "docker",
                    container = %self.container_name,
                    running = is_running,
                    "Checked container status"
                );
                is_running
            }
            Err(e) => {
                debug!(
                    category = "docker",
                    error = %e,
                    "Failed to check container status"
                );
                false
            }
        }
    }

    /// Get the status of the container
    pub fn get_status(&self) -> ContainerStatus {
        // Check if container is running using docker compose ps
        let output = Command::new("docker")
            .args(["compose", "ps", "--format", "json"])
            .current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name)
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.trim().is_empty() {
                    ContainerStatus::NotFound
                } else if stdout.contains("\"running\"") || stdout.contains("\"Running\"") {
                    ContainerStatus::Running
                } else {
                    ContainerStatus::Stopped
                }
            }
            _ => ContainerStatus::NotFound,
        }
    }

    /// Start the Docker container
    ///
    /// Runs `docker compose up -d` in the worktree directory.
    pub fn start(&self) -> Result<ContainerInfo> {
        self.start_internal()
    }

    /// Start the Docker container with automatic retry on transient failures
    ///
    /// Retries up to 3 times with delays of 2s and 5s between attempts.
    pub fn start_with_retry(&self) -> Result<ContainerInfo> {
        with_retry("start", || self.start_internal())
    }

    /// Internal start implementation
    fn start_internal(&self) -> Result<ContainerInfo> {
        info!(
            category = "docker",
            container = %self.container_name,
            "Starting Docker container"
        );

        // Run docker compose up -d
        let output = Command::new("docker")
            .args(["compose", "up", "-d"])
            .current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name)
            .output()
            .map_err(|e| GwtError::Docker(format!("Failed to run docker compose: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(
                category = "docker",
                container = %self.container_name,
                error = %stderr,
                "Docker compose up failed"
            );
            return Err(GwtError::DockerStartFailed {
                reason: stderr.to_string(),
            });
        }

        debug!(
            category = "docker",
            container = %self.container_name,
            "Docker compose up succeeded"
        );

        // Get container info
        let container_id = self.get_container_id().unwrap_or_default();
        let services = self.list_services().unwrap_or_default();

        let mut info =
            ContainerInfo::new(container_id, self.container_name.clone(), ContainerStatus::Running);

        for service in services {
            info.add_service(service);
        }

        Ok(info)
    }

    /// Stop the Docker container
    ///
    /// Runs `docker compose down` in the worktree directory.
    pub fn stop(&self) -> Result<()> {
        info!(
            category = "docker",
            container = %self.container_name,
            "Stopping Docker container"
        );

        let output = Command::new("docker")
            .args(["compose", "down"])
            .current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name)
            .output()
            .map_err(|e| GwtError::Docker(format!("Failed to run docker compose down: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(
                category = "docker",
                container = %self.container_name,
                error = %stderr,
                "Docker compose down failed"
            );
            return Err(GwtError::Docker(format!(
                "Docker compose down failed: {}",
                stderr
            )));
        }

        debug!(
            category = "docker",
            container = %self.container_name,
            "Docker compose down succeeded"
        );

        Ok(())
    }

    /// Get the container ID (short form)
    fn get_container_id(&self) -> Option<String> {
        let output = Command::new("docker")
            .args(["compose", "ps", "-q"])
            .current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name)
            .output()
            .ok()?;

        if output.status.success() {
            let id = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .map(|s| s.trim().to_string())?;
            if !id.is_empty() {
                return Some(id);
            }
        }
        None
    }

    /// List services defined in the compose file
    fn list_services(&self) -> Option<Vec<String>> {
        let output = Command::new("docker")
            .args(["compose", "config", "--services"])
            .current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name)
            .output()
            .ok()?;

        if output.status.success() {
            let services: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !services.is_empty() {
                return Some(services);
            }
        }
        None
    }

    /// Collect environment variables to pass through to the container
    ///
    /// Collects variables matching predefined prefixes (API keys, Git config, etc.)
    pub fn collect_passthrough_env(&self) -> HashMap<String, String> {
        let mut env_vars = HashMap::new();

        for (key, value) in std::env::vars() {
            for prefix in ENV_PASSTHROUGH_PREFIXES {
                if key.starts_with(prefix) || key == *prefix {
                    env_vars.insert(key.clone(), value.clone());
                    break;
                }
            }
        }

        debug!(
            category = "docker",
            count = env_vars.len(),
            "Collected environment variables for passthrough"
        );

        env_vars
    }

    /// Check if the Docker image needs to be rebuilt
    ///
    /// Compares Dockerfile modification time with last build time.
    pub fn needs_rebuild(&self) -> bool {
        let dockerfile_path = match &self.docker_file_type {
            DockerFileType::Compose(p) => p.clone(),
            DockerFileType::Dockerfile(p) => p.clone(),
            DockerFileType::DevContainer(p) => p.clone(),
        };

        // Get Dockerfile modification time
        let modified = match fs::metadata(&dockerfile_path) {
            Ok(meta) => meta.modified().ok(),
            Err(_) => return false,
        };

        match (modified, self.last_build_time) {
            (Some(mod_time), Some(build_time)) => {
                let needs = mod_time > build_time;
                debug!(
                    category = "docker",
                    needs_rebuild = needs,
                    "Checked rebuild status"
                );
                needs
            }
            (Some(_), None) => {
                // No previous build time recorded, might need rebuild
                debug!(category = "docker", "No previous build time, rebuild may be needed");
                true
            }
            _ => false,
        }
    }

    /// Rebuild the Docker image
    ///
    /// Runs `docker compose build` in the worktree directory.
    pub fn rebuild(&mut self) -> Result<()> {
        info!(
            category = "docker",
            container = %self.container_name,
            "Rebuilding Docker image"
        );

        let output = Command::new("docker")
            .args(["compose", "build"])
            .current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name)
            .output()
            .map_err(|e| GwtError::Docker(format!("Failed to run docker compose build: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(
                category = "docker",
                container = %self.container_name,
                error = %stderr,
                "Docker compose build failed"
            );
            return Err(GwtError::DockerBuildFailed {
                reason: stderr.to_string(),
            });
        }

        self.last_build_time = Some(SystemTime::now());

        debug!(
            category = "docker",
            container = %self.container_name,
            "Docker compose build succeeded"
        );

        Ok(())
    }

    /// Run a command inside the container
    ///
    /// Uses `docker compose exec` to run the command in the first service.
    pub fn run_in_container(&self, command: &str, args: &[String]) -> Result<()> {
        let services = self.list_services().ok_or_else(|| {
            GwtError::Docker("No services found in compose file".to_string())
        })?;

        let service = services.first().ok_or_else(|| {
            GwtError::Docker("No services found in compose file".to_string())
        })?;

        self.run_in_service(service, command, args)
    }

    /// Run a command inside a specific service container
    ///
    /// Uses `docker compose exec` with TTY allocation.
    pub fn run_in_service(&self, service: &str, command: &str, args: &[String]) -> Result<()> {
        info!(
            category = "docker",
            service = %service,
            command = %command,
            "Running command in container"
        );

        // Collect environment variables to pass through
        let env_vars = self.collect_passthrough_env();

        // Build the exec command
        let mut cmd = Command::new("docker");
        cmd.args(["compose", "exec"]);

        // Add -T flag for non-interactive mode (when running programmatically)
        // This prevents TTY allocation issues when not running from a terminal
        #[cfg(unix)]
        {
            // Check if stdin is a TTY using libc
            let is_tty = unsafe { libc::isatty(libc::STDIN_FILENO) } != 0;
            if !is_tty {
                cmd.arg("-T");
            }
        }
        #[cfg(not(unix))]
        {
            // On non-Unix platforms, default to non-TTY mode
            cmd.arg("-T");
        }

        // Add working directory
        cmd.args(["-w", "/workspace"]);

        // Add environment variables
        for (key, value) in &env_vars {
            cmd.args(["-e", &format!("{}={}", key, value)]);
        }

        // Add service name and command
        cmd.arg(service);
        cmd.arg(command);
        cmd.args(args);

        cmd.current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name);

        let status = cmd.status().map_err(|e| {
            GwtError::Docker(format!("Failed to run docker compose exec: {}", e))
        })?;

        if !status.success() {
            warn!(
                category = "docker",
                service = %service,
                command = %command,
                exit_code = ?status.code(),
                "Command in container failed"
            );
            return Err(GwtError::Docker(format!(
                "Command '{}' failed with exit code {:?}",
                command,
                status.code()
            )));
        }

        debug!(
            category = "docker",
            service = %service,
            command = %command,
            "Command in container succeeded"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-202: Container name generation test
    #[test]
    fn test_generate_container_name_simple() {
        let name = DockerManager::generate_container_name("my-worktree");
        assert_eq!(name, "gwt-my-worktree");
    }

    #[test]
    fn test_generate_container_name_with_slashes() {
        let name = DockerManager::generate_container_name("feature/add-login");
        assert_eq!(name, "gwt-feature-add-login");
    }

    #[test]
    fn test_generate_container_name_with_spaces() {
        let name = DockerManager::generate_container_name("my worktree name");
        assert_eq!(name, "gwt-my-worktree-name");
    }

    #[test]
    fn test_generate_container_name_uppercase() {
        let name = DockerManager::generate_container_name("MyWorktree");
        assert_eq!(name, "gwt-myworktree");
    }

    #[test]
    fn test_generate_container_name_special_chars() {
        let name = DockerManager::generate_container_name("test@#$%worktree");
        assert_eq!(name, "gwt-test-worktree");
    }

    #[test]
    fn test_generate_container_name_multiple_hyphens() {
        let name = DockerManager::generate_container_name("test---worktree");
        assert_eq!(name, "gwt-test-worktree");
    }

    #[test]
    fn test_generate_container_name_leading_trailing_special() {
        let name = DockerManager::generate_container_name("---test---");
        assert_eq!(name, "gwt-test");
    }

    #[test]
    fn test_docker_manager_new() {
        use std::path::PathBuf;

        let path = PathBuf::from("/tmp/test-worktree");
        let docker_type = DockerFileType::Compose(PathBuf::from("docker-compose.yml"));
        let manager = DockerManager::new(&path, "my-worktree", docker_type);

        assert_eq!(manager.container_name(), "gwt-my-worktree");
        assert_eq!(manager.worktree_path(), path);
        assert!(manager.docker_file_type().is_compose());
    }

    #[test]
    fn test_docker_manager_feature_branch() {
        use std::path::PathBuf;

        let path = PathBuf::from("/tmp/feature-branch");
        let docker_type = DockerFileType::Dockerfile(PathBuf::from("Dockerfile"));
        let manager = DockerManager::new(&path, "feature/JIRA-123/add-feature", docker_type);

        // feature/JIRA-123/add-feature -> feature-jira-123-add-feature
        assert_eq!(manager.container_name(), "gwt-feature-jira-123-add-feature");
    }

    // T-601: Retry logic tests
    #[test]
    fn test_is_retryable_error_daemon_not_running() {
        let error = GwtError::DockerDaemonNotRunning;
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_is_retryable_error_timeout() {
        let error = GwtError::DockerTimeout;
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_is_retryable_error_connection_refused() {
        let error = GwtError::Docker("connection refused".to_string());
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_is_retryable_error_network() {
        let error = GwtError::DockerStartFailed {
            reason: "network error".to_string(),
        };
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_is_not_retryable_build_error() {
        let error = GwtError::DockerBuildFailed {
            reason: "syntax error in Dockerfile".to_string(),
        };
        assert!(!is_retryable_error(&error));
    }

    #[test]
    fn test_is_not_retryable_port_conflict() {
        let error = GwtError::DockerPortConflict { port: 8080 };
        assert!(!is_retryable_error(&error));
    }

    #[test]
    fn test_with_retry_success_first_attempt() {
        let mut attempts = 0;
        let result = with_retry("test", || {
            attempts += 1;
            Ok::<i32, GwtError>(42)
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts, 1);
    }

    #[test]
    fn test_with_retry_non_retryable_error() {
        let mut attempts = 0;
        let result: Result<i32> = with_retry("test", || {
            attempts += 1;
            Err(GwtError::DockerBuildFailed {
                reason: "syntax error".to_string(),
            })
        });
        assert!(result.is_err());
        assert_eq!(attempts, 1); // Should not retry for non-retryable error
    }
}
