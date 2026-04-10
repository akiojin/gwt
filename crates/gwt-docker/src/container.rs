//! Container information and lifecycle management.
//!
//! Provides data structures for representing Docker containers and functions
//! to list, start, stop, and restart them via the Docker CLI.

use gwt_core::{GwtError, Result};
use std::ffi::OsString;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
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

fn run_docker_with_timeout(args: &[&str], action: &str) -> Result<Output> {
    run_docker_with_timeout_in_dir(args, action, None)
}

fn run_docker_with_timeout_in_dir(
    args: &[&str],
    action: &str,
    current_dir: Option<&std::path::Path>,
) -> Result<Output> {
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

    let timeout = docker_timeout();
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(GwtError::Docker(format!(
                        "{action} timed out after {}ms",
                        timeout.as_millis()
                    )));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                return Err(GwtError::Docker(format!(
                    "failed while waiting for {action}: {e}"
                )));
            }
        }
    }

    child
        .wait_with_output()
        .map_err(|e| GwtError::Docker(format!("failed to collect {action} output: {e}")))
}

fn compose_parent_dir(compose_file: &std::path::Path) -> &std::path::Path {
    compose_file
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
}

/// Start a compose service in detached mode.
pub fn compose_up(compose_file: &std::path::Path, service: &str) -> Result<()> {
    let compose_file = compose_file.display().to_string();
    let output = run_docker_with_timeout_in_dir(
        &["compose", "-f", &compose_file, "up", "-d", service],
        "docker compose up",
        Some(compose_parent_dir(std::path::Path::new(&compose_file))),
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

/// Return whether a compose service is currently running.
pub fn compose_service_is_running(compose_file: &std::path::Path, service: &str) -> Result<bool> {
    let compose_file = compose_file.display().to_string();
    let output = run_docker_with_timeout_in_dir(
        &[
            "compose",
            "-f",
            &compose_file,
            "ps",
            "--status",
            "running",
            "--services",
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
        .map(str::trim)
        .any(|running_service| running_service == service))
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
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::TempDir;

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
        let _guard = TEST_LOCK.lock().expect("lock tests");
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
}
