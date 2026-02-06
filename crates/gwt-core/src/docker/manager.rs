//! Docker container manager (SPEC-f5f5657e)
//!
//! Manages Docker containers for worktrees, including startup, shutdown,
//! and executing commands inside containers.

use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use tracing::{debug, info, warn};

use super::container::{ContainerInfo, ContainerStatus};
use super::detector::DockerFileType;
use super::port::PortAllocator;
use crate::{GwtError, Result};
use serde_yaml::Value;

fn extract_port_envs_from_compose(content: &str) -> Vec<(String, u16)> {
    let Ok(value) = serde_yaml::from_str::<Value>(content) else {
        return Vec::new();
    };

    let Some(services) = value.get("services").and_then(|v| v.as_mapping()) else {
        return Vec::new();
    };

    let mut results = Vec::new();

    for service in services.values() {
        let Some(service_map) = service.as_mapping() else {
            continue;
        };
        let Some(ports) = service_map.get(Value::String("ports".to_string())) else {
            continue;
        };

        match ports {
            Value::Sequence(items) => {
                for item in items {
                    match item {
                        Value::String(s) => {
                            if let Some((name, port)) = parse_port_env_default(s) {
                                results.push((name, port));
                            }
                        }
                        Value::Mapping(map) => {
                            if let Some(Value::String(published)) =
                                map.get(Value::String("published".to_string()))
                            {
                                if let Some((name, port)) = parse_port_env_default(published) {
                                    results.push((name, port));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => continue,
        }
    }

    results
}

fn parse_port_env_default(value: &str) -> Option<(String, u16)> {
    let start = value.find("${")?;
    let rest = &value[start + 2..];
    let end = rest.find('}')?;
    let inner = &rest[..end];
    let (name, default) = inner.split_once(":-")?;
    let port = default.parse::<u16>().ok()?;
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), port))
}

fn detect_git_common_dir(worktree_path: &Path) -> Option<PathBuf> {
    if let Ok(output) = std::process::Command::new("git")
        .args([
            "-C",
            &worktree_path.to_string_lossy(),
            "rev-parse",
            "--git-common-dir",
        ])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let trimmed = stdout.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }
    }
    let git_path = worktree_path.join(".git");
    if git_path.is_dir() {
        return Some(git_path);
    }
    let content = fs::read_to_string(&git_path).ok()?;
    let gitdir = content.strip_prefix("gitdir: ")?.trim();
    let gitdir_path = PathBuf::from(gitdir);
    let base_dir = git_path.parent().unwrap_or(worktree_path);
    let gitdir_path = if gitdir_path.is_absolute() {
        gitdir_path
    } else {
        base_dir.join(gitdir_path)
    };
    let gitdir_path = gitdir_path.canonicalize().unwrap_or(gitdir_path);
    if let Some(common_dir) = gitdir_path
        .components()
        .position(|c| c.as_os_str() == "worktrees")
        .and_then(|idx| {
            let mut parts = Vec::new();
            for (i, comp) in gitdir_path.components().enumerate() {
                if i == idx {
                    break;
                }
                parts.push(comp);
            }
            if parts.is_empty() {
                None
            } else {
                let mut path = PathBuf::new();
                for comp in parts {
                    path.push(comp);
                }
                Some(path)
            }
        })
    {
        return Some(common_dir);
    }
    gitdir_path.parent().map(|p| p.to_path_buf())
}

fn detect_git_dir(worktree_path: &Path) -> Option<PathBuf> {
    if let Ok(output) = std::process::Command::new("git")
        .args([
            "-C",
            &worktree_path.to_string_lossy(),
            "rev-parse",
            "--git-dir",
        ])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let trimmed = stdout.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }
    }
    let git_path = worktree_path.join(".git");
    if git_path.is_dir() {
        return Some(git_path);
    }
    let content = fs::read_to_string(&git_path).ok()?;
    let gitdir = content.strip_prefix("gitdir: ")?.trim();
    let gitdir_path = PathBuf::from(gitdir);
    let base_dir = git_path.parent().unwrap_or(worktree_path);
    let gitdir_path = if gitdir_path.is_absolute() {
        gitdir_path
    } else {
        base_dir.join(gitdir_path)
    };
    Some(gitdir_path.canonicalize().unwrap_or(gitdir_path))
}

fn resolve_compose_status(running_output: &str, all_output: &str) -> ContainerStatus {
    if !running_output.trim().is_empty() {
        return ContainerStatus::Running;
    }
    if !all_output.trim().is_empty() {
        return ContainerStatus::Stopped;
    }
    ContainerStatus::NotFound
}

/// Environment variable prefixes to pass through to containers
const ENV_PASSTHROUGH_PREFIXES: &[&str] = &[
    "ANTHROPIC_",
    "OPENAI_",
    "GEMINI_",
    "GOOGLE_",
    "GITHUB_",
    "GIT_",
    "SSH_AUTH_SOCK",
    "USER",
    "SHELL",
];
const ENV_PASSTHROUGH_DENYLIST: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];
const ENV_HOST_GIT_COMMON_DIR: &str = "HOST_GIT_COMMON_DIR";
const ENV_HOST_GIT_WORKTREE_DIR: &str = "HOST_GIT_WORKTREE_DIR";

fn filter_passthrough_env<I>(vars: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut env_vars = HashMap::new();

    for (key, value) in vars {
        for prefix in ENV_PASSTHROUGH_PREFIXES {
            if key.starts_with(prefix) || key == *prefix {
                env_vars.insert(key.clone(), value.clone());
                break;
            }
        }
    }

    for key in ENV_PASSTHROUGH_DENYLIST {
        env_vars.remove(*key);
    }

    env_vars
}

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
    pub fn new(
        worktree_path: &Path,
        worktree_name: &str,
        docker_file_type: DockerFileType,
    ) -> Self {
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
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
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
        let running_output = Command::new("docker")
            .args(["compose", "ps", "-q"])
            .current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name)
            .output();

        let all_output = Command::new("docker")
            .args(["compose", "ps", "-a", "-q"])
            .current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name)
            .output();

        let status = match (running_output, all_output) {
            (Ok(running), Ok(all)) if running.status.success() && all.status.success() => {
                let running_stdout = String::from_utf8_lossy(&running.stdout);
                let all_stdout = String::from_utf8_lossy(&all.stdout);
                resolve_compose_status(&running_stdout, &all_stdout)
            }
            (Ok(running), _) if running.status.success() => {
                let running_stdout = String::from_utf8_lossy(&running.stdout);
                resolve_compose_status(&running_stdout, "")
            }
            _ => ContainerStatus::NotFound,
        };

        info!(
            category = "docker",
            container = %self.container_name,
            status = ?status,
            "Resolved docker compose container status"
        );

        status
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
        let mut command = Command::new("docker");
        command
            .args(["compose", "up", "-d", "--build"])
            .current_dir(&self.worktree_path)
            .env("COMPOSE_PROJECT_NAME", &self.container_name);

        if let Ok(port_envs) = self.collect_compose_port_envs() {
            for (key, value) in port_envs {
                command.env(key, value);
            }
        }

        let output = command
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
        let services = self.list_services_internal().unwrap_or_default();

        let mut info = ContainerInfo::new(
            container_id,
            self.container_name.clone(),
            ContainerStatus::Running,
        );

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
    fn list_services_internal(&self) -> Option<Vec<String>> {
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

    /// List services defined in the compose file (public API)
    pub fn list_services(&self) -> Result<Vec<String>> {
        self.list_services_internal()
            .ok_or_else(|| GwtError::Docker("Failed to list docker compose services".to_string()))
    }

    /// Collect environment variables to pass through to the container
    ///
    /// Collects variables matching predefined prefixes (API keys, Git config, etc.)
    pub fn collect_passthrough_env(&self) -> HashMap<String, String> {
        let mut env_vars = filter_passthrough_env(std::env::vars());

        if let Some(common_dir) = detect_git_common_dir(&self.worktree_path) {
            env_vars.insert(
                ENV_HOST_GIT_COMMON_DIR.to_string(),
                common_dir.to_string_lossy().to_string(),
            );
        }
        if let Some(gitdir) = detect_git_dir(&self.worktree_path) {
            env_vars.insert(
                ENV_HOST_GIT_WORKTREE_DIR.to_string(),
                gitdir.to_string_lossy().to_string(),
            );
        }

        if let Ok(port_envs) = self.collect_compose_port_envs() {
            for (key, value) in port_envs {
                env_vars.entry(key).or_insert(value);
            }
        }

        debug!(
            category = "docker",
            count = env_vars.len(),
            "Collected environment variables for passthrough"
        );

        env_vars
    }

    fn collect_compose_port_envs(&self) -> Result<HashMap<String, String>> {
        let compose_path = match &self.docker_file_type {
            DockerFileType::Compose(path) => path,
            DockerFileType::DevContainer(_) | DockerFileType::Dockerfile(_) => {
                return Ok(HashMap::new())
            }
        };

        let content = fs::read_to_string(compose_path)?;
        let port_envs = extract_port_envs_from_compose(&content);
        if port_envs.is_empty() {
            return Ok(HashMap::new());
        }

        let allocator = PortAllocator::new();
        let mut allocated = HashMap::new();
        let mut used_names = HashSet::new();

        for (env_name, default_port) in port_envs {
            if !used_names.insert(env_name.clone()) {
                continue;
            }
            if let Ok(value) = std::env::var(&env_name) {
                if let Ok(port) = value.parse::<u16>() {
                    if PortAllocator::is_port_in_use(port) {
                        let next = allocator
                            .find_available_port(port)
                            .unwrap_or(port)
                            .to_string();
                        allocated.insert(env_name, next);
                    } else {
                        allocated.insert(env_name, value);
                    }
                } else {
                    allocated.insert(env_name, value);
                }
                continue;
            }

            let port = if PortAllocator::is_port_in_use(default_port) {
                allocator
                    .find_available_port(default_port)
                    .unwrap_or(default_port)
            } else {
                default_port
            };
            allocated.insert(env_name, port.to_string());
        }

        Ok(allocated)
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

        let last_build_time = self
            .last_build_time
            .or_else(|| self.container_created_time());
        let needs = Self::should_prompt_build(modified, last_build_time);
        debug!(
            category = "docker",
            needs_rebuild = needs,
            "Checked rebuild status"
        );
        needs
    }

    fn should_prompt_build(
        modified: Option<SystemTime>,
        last_build_time: Option<SystemTime>,
    ) -> bool {
        match (modified, last_build_time) {
            (Some(mod_time), Some(build_time)) => mod_time > build_time,
            (Some(_), None) => true,
            _ => false,
        }
    }

    fn container_created_time(&self) -> Option<SystemTime> {
        let container_id = self.get_container_id()?;
        let output = Command::new("docker")
            .args(["inspect", "-f", "{{.Created}}", &container_id])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let created_raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if created_raw.is_empty() {
            return None;
        }
        let parsed = DateTime::parse_from_rfc3339(&created_raw).ok()?;
        Some(parsed.with_timezone(&Utc).into())
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
        let services = self.list_services()?;

        let service = services
            .first()
            .ok_or_else(|| GwtError::Docker("No services found in compose file".to_string()))?;

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

        let status = cmd
            .status()
            .map_err(|e| GwtError::Docker(format!("Failed to run docker compose exec: {}", e)))?;

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
    use std::time::{Duration, SystemTime};

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
    fn test_resolve_compose_status_running() {
        let status = resolve_compose_status("abc123\n", "");
        assert_eq!(status, ContainerStatus::Running);
    }

    #[test]
    fn test_resolve_compose_status_stopped() {
        let status = resolve_compose_status("", "abc123\n");
        assert_eq!(status, ContainerStatus::Stopped);
    }

    #[test]
    fn test_resolve_compose_status_not_found() {
        let status = resolve_compose_status("", "");
        assert_eq!(status, ContainerStatus::NotFound);
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
    fn test_should_prompt_build_when_modified_after_build() {
        let now = SystemTime::now();
        let build_time = now - Duration::from_secs(30);
        let modified = now;
        assert!(DockerManager::should_prompt_build(
            Some(modified),
            Some(build_time)
        ));
    }

    #[test]
    fn test_should_prompt_build_when_modified_before_build() {
        let now = SystemTime::now();
        let build_time = now;
        let modified = now - Duration::from_secs(30);
        assert!(!DockerManager::should_prompt_build(
            Some(modified),
            Some(build_time)
        ));
    }

    #[test]
    fn test_should_prompt_build_without_build_time() {
        let now = SystemTime::now();
        assert!(DockerManager::should_prompt_build(Some(now), None));
    }

    #[test]
    fn test_should_prompt_build_without_modified_time() {
        let now = SystemTime::now();
        assert!(!DockerManager::should_prompt_build(None, Some(now)));
        assert!(!DockerManager::should_prompt_build(None, None));
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

    #[test]
    fn test_extract_port_envs_from_compose() {
        let content = r#"
services:
  app:
    ports:
      - "${PORT:-3000}:3000"
      - "127.0.0.1:${LOCAL_PORT:-8080}:8080"
  worker:
    ports:
      - target: 3000
        published: "${PUBLISHED_PORT:-3000}"
"#;

        let envs = extract_port_envs_from_compose(content);
        assert!(envs.contains(&("PORT".to_string(), 3000)));
        assert!(envs.contains(&("LOCAL_PORT".to_string(), 8080)));
        assert!(envs.contains(&("PUBLISHED_PORT".to_string(), 3000)));
    }

    #[test]
    fn test_filter_passthrough_env_excludes_git_internals() {
        let envs = filter_passthrough_env([
            ("GIT_DIR".to_string(), "/tmp/gitdir".to_string()),
            ("GIT_WORK_TREE".to_string(), "/tmp/worktree".to_string()),
            ("HOME".to_string(), "/Users/example".to_string()),
            ("OPENAI_API_KEY".to_string(), "sk-test".to_string()),
        ]);
        assert!(!envs.contains_key("GIT_DIR"));
        assert!(!envs.contains_key("GIT_WORK_TREE"));
        assert!(!envs.contains_key("HOME"));
        assert!(envs.contains_key("OPENAI_API_KEY"));
    }

    #[test]
    fn test_detect_git_common_dir_from_worktree_gitfile() {
        let temp = tempfile::TempDir::new().unwrap();
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let git_file = worktree.join(".git");
        let gitdir = temp
            .path()
            .join("repo.git")
            .join("worktrees")
            .join("worktree");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(&git_file, format!("gitdir: {}\n", gitdir.to_string_lossy())).unwrap();

        let common = detect_git_common_dir(&worktree)
            .unwrap()
            .canonicalize()
            .unwrap();
        let expected = temp.path().join("repo.git").canonicalize().unwrap();
        assert_eq!(common, expected);
    }

    #[test]
    fn test_detect_git_common_dir_from_relative_gitfile() {
        let temp = tempfile::TempDir::new().unwrap();
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let git_file = worktree.join(".git");
        let gitdir = temp
            .path()
            .join("repo.git")
            .join("worktrees")
            .join("worktree");
        std::fs::create_dir_all(&gitdir).unwrap();
        let gitdir_rel = PathBuf::from("../repo.git/worktrees/worktree");
        std::fs::write(&git_file, format!("gitdir: {}\n", gitdir_rel.display())).unwrap();

        let common = detect_git_common_dir(&worktree)
            .unwrap()
            .canonicalize()
            .unwrap();
        let expected = temp.path().join("repo.git").canonicalize().unwrap();
        let normalize_private = |path: &Path| {
            if let Ok(stripped) = path.strip_prefix("/private") {
                PathBuf::from("/").join(stripped)
            } else {
                path.to_path_buf()
            }
        };
        assert_eq!(normalize_private(&common), normalize_private(&expected));
    }

    #[test]
    fn test_detect_git_common_dir_from_git_dir() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let git_dir = repo.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();

        let common = detect_git_common_dir(&repo).unwrap();
        assert_eq!(common, git_dir);
    }
}
