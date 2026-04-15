//! Container information and lifecycle management.
//!
//! Provides data structures for representing Docker containers and functions
//! to list, start, stop, and restart them via the Docker CLI.

use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read},
    process::{Command, Output, Stdio},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

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

/// Output stream emitted by a Docker command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutputStream {
    Stdout,
    Stderr,
}

/// Status of a Docker Compose service.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ComposeServiceStatus {
    Running,
    Stopped,
    Exited,
    #[default]
    NotFound,
}

fn docker_binary() -> OsString {
    std::env::var_os("GWT_DOCKER_BIN").unwrap_or_else(|| OsString::from("docker"))
}

fn docker_timeout() -> Duration {
    const DEFAULT_TIMEOUT_MS: u64 = 5_000;
    std::env::var("GWT_DOCKER_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_TIMEOUT_MS))
}

fn docker_compose_up_timeout() -> Duration {
    const DEFAULT_TIMEOUT_MS: u64 = 300_000;
    std::env::var("GWT_DOCKER_COMPOSE_UP_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_TIMEOUT_MS))
}

fn run_docker_with_timeout(args: &[&str], action: &str) -> Result<Output> {
    run_docker_with_timeout_in_dir(args, action, None)
}

fn run_docker_with_timeout_in_dir(
    args: &[&str],
    action: &str,
    current_dir: Option<&std::path::Path>,
) -> Result<Output> {
    run_docker_with_timeout_in_dir_and_timeout(args, action, current_dir, docker_timeout())
}

fn run_docker_with_timeout_in_dir_and_timeout(
    args: &[&str],
    action: &str,
    current_dir: Option<&std::path::Path>,
    timeout: Duration,
) -> Result<Output> {
    run_docker_with_output_streaming_in_dir_and_timeout(
        args,
        action,
        current_dir,
        timeout,
        |_, _| {},
    )
}

#[derive(Debug)]
struct CommandOutputLine {
    stream: CommandOutputStream,
    line: String,
}

fn spawn_output_reader(
    reader: impl Read + Send + 'static,
    stream: CommandOutputStream,
    tx: mpsc::Sender<CommandOutputLine>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines().map_while(|line| line.ok()) {
            if tx.send(CommandOutputLine { stream, line }).is_err() {
                break;
            }
        }
    })
}

fn push_output_line(buf: &mut Vec<u8>, line: &str) {
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
}

fn handle_command_output_line<F>(
    line: CommandOutputLine,
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    on_line: &mut F,
) where
    F: FnMut(CommandOutputStream, &str),
{
    match line.stream {
        CommandOutputStream::Stdout => push_output_line(stdout, &line.line),
        CommandOutputStream::Stderr => push_output_line(stderr, &line.line),
    }
    on_line(line.stream, &line.line);
}

fn drain_output_lines<F>(
    rx: &mpsc::Receiver<CommandOutputLine>,
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    on_line: &mut F,
) where
    F: FnMut(CommandOutputStream, &str),
{
    while let Ok(line) = rx.try_recv() {
        handle_command_output_line(line, stdout, stderr, on_line);
    }
}

fn run_docker_with_output_streaming_in_dir_and_timeout<F>(
    args: &[&str],
    action: &str,
    current_dir: Option<&std::path::Path>,
    timeout: Duration,
    mut on_line: F,
) -> Result<Output>
where
    F: FnMut(CommandOutputStream, &str),
{
    let mut command = Command::new(docker_binary());
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }
    let mut child = command
        .spawn()
        .map_err(|e| GwtError::Docker(format!("failed to run {action}: {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| GwtError::Docker(format!("failed to capture stdout for {action}")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| GwtError::Docker(format!("failed to capture stderr for {action}")))?;
    let (tx, rx) = mpsc::channel::<CommandOutputLine>();
    let stdout_handle = spawn_output_reader(stdout, CommandOutputStream::Stdout, tx.clone());
    let stderr_handle = spawn_output_reader(stderr, CommandOutputStream::Stderr, tx);

    let deadline = Instant::now() + timeout;
    let mut collected_stdout = Vec::new();
    let mut collected_stderr = Vec::new();

    let status = loop {
        drain_output_lines(
            &rx,
            &mut collected_stdout,
            &mut collected_stderr,
            &mut on_line,
        );
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_handle.join();
                    let _ = stderr_handle.join();
                    drain_output_lines(
                        &rx,
                        &mut collected_stdout,
                        &mut collected_stderr,
                        &mut on_line,
                    );
                    return Err(GwtError::Docker(format!(
                        "{action} timed out after {}ms",
                        timeout.as_millis()
                    )));
                }
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                drain_output_lines(
                    &rx,
                    &mut collected_stdout,
                    &mut collected_stderr,
                    &mut on_line,
                );
                return Err(GwtError::Docker(format!(
                    "failed while waiting for {action}: {e}"
                )));
            }
        }

        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(line) => handle_command_output_line(
                line,
                &mut collected_stdout,
                &mut collected_stderr,
                &mut on_line,
            ),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {}
        }
    };

    let _ = stdout_handle.join();
    let _ = stderr_handle.join();
    drain_output_lines(
        &rx,
        &mut collected_stdout,
        &mut collected_stderr,
        &mut on_line,
    );

    Ok(Output {
        status,
        stdout: collected_stdout,
        stderr: collected_stderr,
    })
}

fn compose_parent_dir(compose_file: &std::path::Path) -> &std::path::Path {
    compose_file
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
}

/// Start a compose service in detached mode.
pub fn compose_up(compose_file: &std::path::Path, service: &str) -> Result<()> {
    compose_up_with_output(compose_file, service, |_, _| {})
}

/// Start a compose service in detached mode and force container recreation.
pub fn compose_up_force_recreate(compose_file: &std::path::Path, service: &str) -> Result<()> {
    compose_up_force_recreate_with_output(compose_file, service, |_, _| {})
}

/// Start a compose service in detached mode while streaming stdout/stderr lines.
pub fn compose_up_with_output<F>(
    compose_file: &std::path::Path,
    service: &str,
    on_line: F,
) -> Result<()>
where
    F: FnMut(CommandOutputStream, &str),
{
    let compose_file = compose_file.display().to_string();
    let output = run_docker_with_output_streaming_in_dir_and_timeout(
        &["compose", "-f", &compose_file, "up", "-d", service],
        "docker compose up",
        Some(compose_parent_dir(std::path::Path::new(&compose_file))),
        docker_compose_up_timeout(),
        on_line,
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Docker(if stderr.is_empty() {
            "docker compose up failed".to_string()
        } else {
            stderr
        }));
    }
    debug!(
        category = "docker",
        service = service,
        compose_file = compose_file,
        "compose service started"
    );
    Ok(())
}

/// Start a compose service in detached mode while forcing recreation and
/// streaming stdout/stderr lines.
pub fn compose_up_force_recreate_with_output<F>(
    compose_file: &std::path::Path,
    service: &str,
    on_line: F,
) -> Result<()>
where
    F: FnMut(CommandOutputStream, &str),
{
    let compose_file = compose_file.display().to_string();
    let output = run_docker_with_output_streaming_in_dir_and_timeout(
        &[
            "compose",
            "-f",
            &compose_file,
            "up",
            "-d",
            "--force-recreate",
            service,
        ],
        "docker compose up --force-recreate",
        Some(compose_parent_dir(std::path::Path::new(&compose_file))),
        docker_compose_up_timeout(),
        on_line,
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Docker(if stderr.is_empty() {
            "docker compose up --force-recreate failed".to_string()
        } else {
            stderr
        }));
    }
    debug!(
        category = "docker",
        service = service,
        compose_file = compose_file,
        "compose service recreated"
    );
    Ok(())
}

fn compose_service_statuses(
    compose_file: &std::path::Path,
) -> Result<Vec<(String, ComposeServiceStatus)>> {
    let compose_file = compose_file.display().to_string();
    let output = run_docker_with_timeout_in_dir(
        &[
            "compose",
            "-f",
            &compose_file,
            "ps",
            "--all",
            "--format",
            "{{.Service}}\t{{.State}}",
        ],
        "docker compose ps",
        Some(compose_parent_dir(std::path::Path::new(&compose_file))),
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Docker(if stderr.is_empty() {
            "docker compose ps failed".to_string()
        } else {
            stderr
        }));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\t');
            let service = parts.next()?.trim();
            if service.is_empty() {
                return None;
            }
            let status = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
            let status = match status.as_str() {
                "running" => ComposeServiceStatus::Running,
                "exited" => ComposeServiceStatus::Exited,
                // Older tests and fallback scripts emit only service names.
                // Treat a missing state column as "listed by ps", which implies running.
                "" => ComposeServiceStatus::Running,
                _ => ComposeServiceStatus::Stopped,
            };
            Some((service.to_string(), status))
        })
        .collect())
}

/// Return whether a compose service is currently running.
pub fn compose_service_is_running(compose_file: &std::path::Path, service: &str) -> Result<bool> {
    Ok(compose_service_status(compose_file, service)? == ComposeServiceStatus::Running)
}

/// Return the current status of a compose service.
pub fn compose_service_status(
    compose_file: &std::path::Path,
    service: &str,
) -> Result<ComposeServiceStatus> {
    Ok(compose_service_statuses(compose_file)?
        .into_iter()
        .find_map(|(candidate, status)| (candidate == service).then_some(status))
        .unwrap_or(ComposeServiceStatus::NotFound))
}

/// Return recent logs for a compose service.
pub fn compose_service_logs(compose_file: &std::path::Path, service: &str) -> Result<String> {
    let compose_file = compose_file.display().to_string();
    let output = run_docker_with_timeout_in_dir(
        &[
            "compose",
            "-f",
            &compose_file,
            "logs",
            "--no-color",
            "--tail",
            "50",
            service,
        ],
        "docker compose logs",
        Some(compose_parent_dir(std::path::Path::new(&compose_file))),
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Docker(if stderr.is_empty() {
            "docker compose logs failed".to_string()
        } else {
            stderr
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Return whether a compose service can resolve a command inside the container.
pub fn compose_service_has_command(
    compose_file: &std::path::Path,
    service: &str,
    command: &str,
) -> Result<bool> {
    let compose_file = compose_file.display().to_string();
    let output = run_docker_with_timeout_in_dir(
        &[
            "compose",
            "-f",
            &compose_file,
            "exec",
            "-T",
            service,
            "sh",
            "-lc",
            "command -v \"$1\" >/dev/null 2>&1",
            "sh",
            command,
        ],
        "docker compose exec command check",
        Some(compose_parent_dir(std::path::Path::new(&compose_file))),
    )?;
    if output.status.success() {
        return Ok(true);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        return Ok(false);
    }

    Err(GwtError::Docker(stderr))
}

/// Execute a command inside a compose service and capture stdout/stderr.
///
/// Transport failures such as spawn errors and timeouts return `Err`. A
/// non-zero exit status from the command itself is returned via `Output`.
pub fn compose_service_exec_capture(
    compose_file: &std::path::Path,
    service: &str,
    working_dir: Option<&str>,
    args: &[String],
) -> Result<Output> {
    let compose_file = compose_file.display().to_string();
    let mut docker_args = vec!["compose", "-f", &compose_file, "exec", "-T"];
    if let Some(working_dir) = working_dir {
        docker_args.push("-w");
        docker_args.push(working_dir);
    }
    docker_args.push(service);
    docker_args.extend(args.iter().map(String::as_str));

    run_docker_with_timeout_in_dir(
        &docker_args,
        "docker compose exec",
        Some(compose_parent_dir(std::path::Path::new(&compose_file))),
    )
}

/// Return whether a compose service executes as root inside the container.
pub fn compose_service_user_is_root(compose_file: &std::path::Path, service: &str) -> Result<bool> {
    let output = compose_service_exec_capture(
        compose_file,
        service,
        None,
        &["sh".to_string(), "-lc".to_string(), "id -u".to_string()],
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "failed to determine container user".to_string()
        };
        return Err(GwtError::Docker(detail));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim() == "0")
}

/// Stop a compose service.
pub fn compose_stop(compose_file: &std::path::Path, service: &str) -> Result<()> {
    let compose_file = compose_file.display().to_string();
    let output = run_docker_with_timeout_in_dir(
        &["compose", "-f", &compose_file, "stop", service],
        "docker compose stop",
        Some(compose_parent_dir(std::path::Path::new(&compose_file))),
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Docker(if stderr.is_empty() {
            "docker compose stop failed".to_string()
        } else {
            stderr
        }));
    }
    Ok(())
}

/// Restart a compose service.
pub fn compose_restart(compose_file: &std::path::Path, service: &str) -> Result<()> {
    let compose_file = compose_file.display().to_string();
    let output = run_docker_with_timeout_in_dir(
        &["compose", "-f", &compose_file, "restart", service],
        "docker compose restart",
        Some(compose_parent_dir(std::path::Path::new(&compose_file))),
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Docker(if stderr.is_empty() {
            "docker compose restart failed".to_string()
        } else {
            stderr
        }));
    }
    Ok(())
}

/// List all containers (including stopped ones).
pub fn list_containers() -> Result<Vec<ContainerInfo>> {
    let output = run_docker_with_timeout(
        &[
            "ps",
            "-a",
            "--format",
            "{{.ID}}\t{{.Names}}\t{{.State}}\t{{.Image}}\t{{.Ports}}",
        ],
        "docker ps",
    )?;

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
    let output = run_docker_with_timeout(&[action, id], &format!("docker {action}"))?;
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
    use std::{fs, io::Write, path::PathBuf, sync::Mutex};

    use tempfile::TempDir;

    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn write_fake_docker(script_body: &str) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let script_path = dir.path().join("docker");
        let mut file = fs::File::create(&script_path).expect("create fake docker");
        file.write_all(script_body.as_bytes())
            .expect("write fake docker");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata().expect("stat fake docker").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).expect("chmod fake docker");
        }

        (dir, script_path)
    }

    fn with_fake_docker<R>(script_body: &str, f: impl FnOnce(&PathBuf) -> R) -> R {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, script_path) = write_fake_docker(script_body);
        let prev = std::env::var_os("GWT_DOCKER_BIN");
        std::env::set_var("GWT_DOCKER_BIN", &script_path);
        let result = f(&script_path);
        match prev {
            Some(value) => std::env::set_var("GWT_DOCKER_BIN", value),
            None => std::env::remove_var("GWT_DOCKER_BIN"),
        }
        result
    }

    fn read_invocation(path: &PathBuf) -> String {
        fs::read_to_string(path).expect("read invocation log")
    }

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

    #[test]
    fn start_invokes_docker_with_expected_arguments() {
        let log_dir = tempfile::tempdir().expect("temp log dir");
        let log_path = log_dir.path().join("args.txt");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            log_path.display()
        );

        with_fake_docker(&script, |_| {
            start("abc123").expect("start container");
        });

        assert_eq!(read_invocation(&log_path), "start\nabc123\n");
    }

    #[test]
    fn stop_invokes_docker_with_expected_arguments() {
        let log_dir = tempfile::tempdir().expect("temp log dir");
        let log_path = log_dir.path().join("args.txt");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            log_path.display()
        );

        with_fake_docker(&script, |_| {
            stop("abc123").expect("stop container");
        });

        assert_eq!(read_invocation(&log_path), "stop\nabc123\n");
    }

    #[test]
    fn restart_invokes_docker_with_expected_arguments() {
        let log_dir = tempfile::tempdir().expect("temp log dir");
        let log_path = log_dir.path().join("args.txt");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            log_path.display()
        );

        with_fake_docker(&script, |_| {
            restart("abc123").expect("restart container");
        });

        assert_eq!(read_invocation(&log_path), "restart\nabc123\n");
    }

    #[test]
    fn lifecycle_returns_docker_stderr_on_failure() {
        let script = "#!/bin/sh\necho 'permission denied' >&2\nexit 17\n";

        let err = with_fake_docker(script, |_| start("abc123").expect_err("start should fail"));

        assert!(
            format!("{err:?}").contains("docker start failed: permission denied"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn list_containers_times_out_when_docker_is_unresponsive() {
        let script = "#!/bin/sh\nif [ \"$1\" = \"ps\" ]; then\n  sleep 1\n  exit 0\nfi\nexit 0\n";

        with_fake_docker(script, |_| {
            let previous_timeout = std::env::var_os("GWT_DOCKER_TIMEOUT_MS");
            std::env::set_var("GWT_DOCKER_TIMEOUT_MS", "50");

            let result = list_containers();

            match previous_timeout {
                Some(value) => std::env::set_var("GWT_DOCKER_TIMEOUT_MS", value),
                None => std::env::remove_var("GWT_DOCKER_TIMEOUT_MS"),
            }

            let err = result.expect_err("list_containers should time out");
            assert!(
                err.to_string().contains("docker ps timed out"),
                "unexpected timeout error: {err}"
            );
        });
    }

    #[test]
    fn compose_up_uses_longer_timeout_than_default_docker_commands() {
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"up\" ]; then\n  sleep 0.1\n  exit 0\nfi\nexit 0\n";

        with_fake_docker(script, |_| {
            let previous_timeout = std::env::var_os("GWT_DOCKER_TIMEOUT_MS");
            let previous_compose_up_timeout = std::env::var_os("GWT_DOCKER_COMPOSE_UP_TIMEOUT_MS");
            std::env::set_var("GWT_DOCKER_TIMEOUT_MS", "50");
            std::env::set_var("GWT_DOCKER_COMPOSE_UP_TIMEOUT_MS", "500");

            let result = compose_up(&compose_path, "app");

            match previous_timeout {
                Some(value) => std::env::set_var("GWT_DOCKER_TIMEOUT_MS", value),
                None => std::env::remove_var("GWT_DOCKER_TIMEOUT_MS"),
            }
            match previous_compose_up_timeout {
                Some(value) => std::env::set_var("GWT_DOCKER_COMPOSE_UP_TIMEOUT_MS", value),
                None => std::env::remove_var("GWT_DOCKER_COMPOSE_UP_TIMEOUT_MS"),
            }

            result.expect("compose up should use compose-up timeout");
        });
    }

    #[test]
    fn compose_up_invokes_docker_with_expected_arguments() {
        let log_dir = tempfile::tempdir().expect("temp log dir");
        let log_path = log_dir.path().join("args.txt");
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            log_path.display()
        );

        with_fake_docker(&script, |_| {
            compose_up(&compose_path, "app").expect("compose up");
        });

        assert_eq!(
            read_invocation(&log_path),
            format!("compose\n-f\n{}\nup\n-d\napp\n", compose_path.display())
        );
    }

    #[test]
    fn compose_up_force_recreate_invokes_docker_with_expected_arguments() {
        let log_dir = tempfile::tempdir().expect("temp log dir");
        let log_path = log_dir.path().join("args.txt");
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            log_path.display()
        );

        with_fake_docker(&script, |_| {
            compose_up_force_recreate(&compose_path, "app").expect("compose up force recreate");
        });

        assert_eq!(
            read_invocation(&log_path),
            format!(
                "compose\n-f\n{}\nup\n-d\n--force-recreate\napp\n",
                compose_path.display()
            )
        );
    }

    #[test]
    fn compose_up_with_output_streams_stdout_and_stderr_lines() {
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"up\" ]; then\n  printf 'stdout line 1\\n'\n  printf 'stderr line 1\\n' >&2\n  printf 'stdout line 2\\n'\n  exit 0\nfi\nexit 0\n";

        with_fake_docker(script, |_| {
            let mut seen = Vec::new();
            compose_up_with_output(&compose_path, "app", |stream, line| {
                seen.push((stream, line.to_string()));
            })
            .expect("compose up");

            assert!(seen.contains(&(CommandOutputStream::Stdout, "stdout line 1".to_string())));
            assert!(seen.contains(&(CommandOutputStream::Stderr, "stderr line 1".to_string())));
            assert!(seen.contains(&(CommandOutputStream::Stdout, "stdout line 2".to_string())));
        });
    }

    #[test]
    fn compose_service_is_running_reads_compose_ps_output() {
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'app\\nworker\\n'\n  exit 0\nfi\nexit 0\n";

        with_fake_docker(script, |_| {
            assert!(compose_service_is_running(&compose_path, "app").expect("ps status"));
            assert!(!compose_service_is_running(&compose_path, "db").expect("ps status"));
        });
    }

    #[test]
    fn compose_service_status_reads_ps_output() {
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"ps\" ]; then\n  printf 'app\\trunning\\nworker\\texited\\n'\n  exit 0\nfi\nexit 0\n";

        with_fake_docker(script, |_| {
            assert_eq!(
                compose_service_status(&compose_path, "app").expect("service status"),
                ComposeServiceStatus::Running
            );
            assert_eq!(
                compose_service_status(&compose_path, "worker").expect("service status"),
                ComposeServiceStatus::Exited
            );
            assert_eq!(
                compose_service_status(&compose_path, "db").expect("service status"),
                ComposeServiceStatus::NotFound
            );
        });
    }

    #[test]
    fn compose_restart_invokes_docker_with_expected_arguments() {
        let log_dir = tempfile::tempdir().expect("temp log dir");
        let log_path = log_dir.path().join("args.txt");
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            log_path.display()
        );

        with_fake_docker(&script, |_| {
            compose_restart(&compose_path, "app").expect("compose restart");
        });

        assert_eq!(
            read_invocation(&log_path),
            format!("compose\n-f\n{}\nrestart\napp\n", compose_path.display())
        );
    }

    #[test]
    fn compose_stop_invokes_docker_with_expected_arguments() {
        let log_dir = tempfile::tempdir().expect("temp log dir");
        let log_path = log_dir.path().join("args.txt");
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            log_path.display()
        );

        with_fake_docker(&script, |_| {
            compose_stop(&compose_path, "app").expect("compose stop");
        });

        assert_eq!(
            read_invocation(&log_path),
            format!("compose\n-f\n{}\nstop\napp\n", compose_path.display())
        );
    }

    #[test]
    fn compose_service_logs_returns_stdout() {
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"logs\" ]; then\n  printf 'boot failed\\nstack line\\n'\n  exit 0\nfi\nexit 0\n";

        with_fake_docker(script, |_| {
            let logs = compose_service_logs(&compose_path, "app").expect("logs");
            assert!(logs.contains("boot failed"));
            assert!(logs.contains("stack line"));
        });
    }

    #[test]
    fn compose_service_has_command_returns_true_when_command_exists() {
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"app\" ]; then\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n";

        with_fake_docker(script, |_| {
            assert!(
                compose_service_has_command(&compose_path, "app", "claude").expect("command check")
            );
        });
    }

    #[test]
    fn compose_service_has_command_returns_false_when_command_is_missing() {
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ]; then\n  exit 1\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n";

        with_fake_docker(script, |_| {
            assert!(!compose_service_has_command(&compose_path, "app", "claude")
                .expect("command check"));
        });
    }

    #[test]
    fn compose_service_has_command_returns_docker_error_output() {
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ]; then\n  printf 'service is not running\\n' >&2\n  exit 1\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n";

        let err = with_fake_docker(script, |_| {
            compose_service_has_command(&compose_path, "app", "claude").expect_err("docker error")
        });

        assert!(
            err.to_string().contains("service is not running"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compose_service_exec_capture_preserves_non_zero_output() {
        let log_dir = tempfile::tempdir().expect("temp log dir");
        let log_path = log_dir.path().join("args.txt");
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"-w\" ] && [ \"$7\" = \"/workspace\" ] && [ \"$8\" = \"app\" ]; then\n  printf 'could not determine executable to run\\n' >&2\n  exit 1\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n",
            log_path.display()
        );

        with_fake_docker(&script, |_| {
            let output = compose_service_exec_capture(
                &compose_path,
                "app",
                Some("/workspace"),
                &[
                    "bunx".to_string(),
                    "@anthropic-ai/claude-code@latest".to_string(),
                    "--version".to_string(),
                ],
            )
            .expect("exec capture");

            assert_eq!(output.status.code(), Some(1));
            assert!(String::from_utf8_lossy(&output.stderr).contains("could not determine"));
            assert_eq!(
                read_invocation(&log_path),
                format!(
                    "compose\n-f\n{}\nexec\n-T\n-w\n/workspace\napp\nbunx\n@anthropic-ai/claude-code@latest\n--version\n",
                    compose_path.display()
                )
            );
        });
    }

    #[test]
    fn compose_service_user_is_root_returns_true_for_uid_zero() {
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"app\" ] && [ \"$7\" = \"sh\" ] && [ \"$8\" = \"-lc\" ] && [ \"$9\" = \"id -u\" ]; then\n  printf '0\\n'\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n";

        with_fake_docker(script, |_| {
            assert!(compose_service_user_is_root(&compose_path, "app").expect("root probe"));
        });
    }

    #[test]
    fn compose_service_user_is_root_returns_false_for_non_root() {
        let compose_dir = tempfile::tempdir().expect("temp compose dir");
        let compose_path = compose_dir.path().join("docker-compose.yml");
        fs::write(
            &compose_path,
            "services:\n  app:\n    image: nginx:latest\n",
        )
        .expect("compose");
        let script = "#!/bin/sh\nif [ \"$1\" = \"compose\" ] && [ \"$4\" = \"exec\" ] && [ \"$5\" = \"-T\" ] && [ \"$6\" = \"app\" ] && [ \"$7\" = \"sh\" ] && [ \"$8\" = \"-lc\" ] && [ \"$9\" = \"id -u\" ]; then\n  printf '1000\\n'\n  exit 0\nfi\nprintf 'unexpected invocation: %s\\n' \"$*\" >&2\nexit 1\n";

        with_fake_docker(script, |_| {
            assert!(!compose_service_user_is_root(&compose_path, "app").expect("root probe"));
        });
    }
}
