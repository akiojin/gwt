//! Docker environment detection.
//!
//! Checks for Docker CLI availability, daemon status, and discovers
//! Docker-related files (Dockerfile, docker-compose.yml, .devcontainer/).

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use tracing::{debug, info};

/// Detected Docker files in a directory.
#[derive(Debug, Clone, Default)]
pub struct DockerFiles {
    /// Path to Dockerfile, if found.
    pub dockerfile: Option<PathBuf>,
    /// Path to docker-compose.yml (or compose.yml variant), if found.
    pub compose_file: Option<PathBuf>,
    /// Path to .devcontainer/ directory, if found.
    pub devcontainer_dir: Option<PathBuf>,
}

impl DockerFiles {
    /// Returns true if any Docker files were detected.
    pub fn any_found(&self) -> bool {
        self.dockerfile.is_some() || self.compose_file.is_some() || self.devcontainer_dir.is_some()
    }
}

/// Run a docker sub-command and return whether it succeeded.
fn docker_probe(args: &[&str], label: &str) -> bool {
    docker_probe_diagnostics(args, label).is_ok()
}

/// Run a docker sub-command, returning failure diagnostics (probe stderr
/// or the spawn error) so preflight errors can explain *why* a probe
/// failed instead of only that it failed (Issue #3029).
fn docker_probe_diagnostics(args: &[&str], label: &str) -> std::result::Result<(), String> {
    // SPEC-2809 / SPEC-1924 Phase D-docker — route docker probes through
    // `spawn_logged_blocking` so the docker tab of the Console window /
    // Logs Process facet sees them. The `binary` may be a `GWT_DOCKER_BIN`
    // override; pass it as the program directly.
    let binary = docker_binary();
    let attempted_binary = binary.to_string_lossy().into_owned();
    // Emit before spawning so the event is captured by tracing subscribers that
    // wrap this call (e.g. `with_default` in tests). On Linux the tokio
    // current-thread runtime's block_on can displace the thread-local dispatcher
    // between the spawn and the match arm, causing post-spawn events to be missed.
    info!(
        target: "gwt::launch::probe",
        category = "docker",
        label = label,
        attempted_binary = %attempted_binary,
        "docker probe"
    );
    let hub = gwt_core::process_console::global();
    let options =
        gwt_core::process_console::SpawnOptions::new(format!("docker {}", args.join(" ")));
    let result = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Docker,
        &binary,
        args,
        options,
    );
    match result {
        Ok(output) => {
            if output.success() {
                return Ok(());
            }
            let stderr = output.stderr.trim();
            if stderr.is_empty() {
                Err(format!(
                    "docker {} exited with status {:?}",
                    args.join(" "),
                    output.exit_code
                ))
            } else {
                debug!(
                    target: "gwt::launch::probe",
                    category = "docker",
                    label = label,
                    attempted_binary = %attempted_binary,
                    stderr = %stderr,
                    "docker probe stderr"
                );
                Err(stderr.to_string())
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

fn preflight_message(summary: &str, detail: &str) -> String {
    let detail = detail.trim();
    if detail.is_empty() {
        summary.to_string()
    } else {
        // Keep the hint single-line so the agent pane error stays compact.
        let detail = detail.lines().next().unwrap_or(detail);
        format!("{summary} ({detail})")
    }
}

/// Docker launch preflight: CLI present, compose v2 plugin available,
/// daemon running. On failure the message includes the probe's stderr
/// (e.g. `docker compose is not available (docker: unknown command:
/// docker compose)`) so environment issues like an invisible
/// `~/.docker/cli-plugins` are distinguishable from a broken Docker
/// install (Issue #3029).
pub fn launch_preflight() -> std::result::Result<(), String> {
    docker_probe_diagnostics(&["--version"], "docker CLI").map_err(|detail| {
        preflight_message("Docker is not installed or not available on PATH", &detail)
    })?;
    docker_probe_diagnostics(&["compose", "version"], "docker compose")
        .map_err(|detail| preflight_message("docker compose is not available", &detail))?;
    docker_probe_diagnostics(&["info"], "daemon")
        .map_err(|detail| preflight_message("Docker daemon is not running", &detail))?;
    Ok(())
}

fn docker_binary() -> OsString {
    std::env::var_os("GWT_DOCKER_BIN").unwrap_or_else(|| OsString::from("docker"))
}

/// Check if the `docker` command is available in PATH.
pub fn docker_available() -> bool {
    docker_probe(&["--version"], "docker CLI")
}

/// Check if `docker compose` (v2) is available.
pub fn compose_available() -> bool {
    docker_probe(&["compose", "version"], "docker compose")
}

/// Check if the Docker daemon is running.
pub fn daemon_running() -> bool {
    docker_probe(&["info"], "daemon")
}

/// Detect Docker-related files in a directory.
///
/// Scans for Dockerfile, docker-compose.yml / compose.yml variants,
/// and .devcontainer/ directory.
pub fn detect_docker_files(dir: &Path) -> DockerFiles {
    let mut files = DockerFiles::default();

    // Compose files (check common variants)
    let compose_names = [
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
    ];
    for name in compose_names {
        let p = dir.join(name);
        if p.is_file() {
            debug!(category = "docker", file = %name, "found compose file");
            files.compose_file = Some(p);
            break;
        }
    }

    // Dockerfile
    let dockerfile = dir.join("Dockerfile");
    if dockerfile.is_file() {
        debug!(category = "docker", "found Dockerfile");
        files.dockerfile = Some(dockerfile);
    }

    // .devcontainer/
    let devcontainer = dir.join(".devcontainer");
    if devcontainer.is_dir() {
        debug!(category = "docker", "found .devcontainer/");
        files.devcontainer_dir = Some(devcontainer);
    }

    files
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn detect_empty_dir_finds_nothing() {
        let tmp = TempDir::new().unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(!files.any_found());
        assert!(files.dockerfile.is_none());
        assert!(files.compose_file.is_none());
        assert!(files.devcontainer_dir.is_none());
    }

    #[test]
    fn detect_dockerfile() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Dockerfile"), "FROM ubuntu:22.04").unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.any_found());
        assert!(files.dockerfile.is_some());
        assert!(files.compose_file.is_none());
    }

    #[test]
    fn detect_docker_compose_yml() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("docker-compose.yml"), "version: '3'").unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.any_found());
        assert!(files.compose_file.is_some());
        assert!(files
            .compose_file
            .as_ref()
            .unwrap()
            .ends_with("docker-compose.yml"));
    }

    #[test]
    fn detect_compose_yml() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("compose.yml"), "version: '3'").unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.compose_file.is_some());
    }

    #[test]
    fn detect_devcontainer_dir() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".devcontainer")).unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.any_found());
        assert!(files.devcontainer_dir.is_some());
    }

    #[test]
    fn detect_all_docker_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Dockerfile"), "FROM node:18").unwrap();
        std::fs::write(tmp.path().join("docker-compose.yml"), "version: '3'").unwrap();
        std::fs::create_dir(tmp.path().join(".devcontainer")).unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.dockerfile.is_some());
        assert!(files.compose_file.is_some());
        assert!(files.devcontainer_dir.is_some());
    }

    #[test]
    fn docker_compose_yml_takes_priority_over_compose_yml() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("docker-compose.yml"), "version: '3'").unwrap();
        std::fs::write(tmp.path().join("compose.yml"), "version: '3'").unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files
            .compose_file
            .as_ref()
            .unwrap()
            .ends_with("docker-compose.yml"));
    }

    #[test]
    fn directory_named_dockerfile_is_ignored() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("Dockerfile")).unwrap();
        let files = detect_docker_files(tmp.path());
        assert!(files.dockerfile.is_none());
    }

    // Smoke tests — just verify the functions return without panic.
    #[test]
    fn docker_available_returns_bool() {
        let _ = docker_available();
    }

    #[test]
    fn compose_available_returns_bool() {
        let _ = compose_available();
    }

    #[test]
    fn daemon_running_returns_bool() {
        let _ = daemon_running();
    }

    #[test]
    fn docker_probe_emits_info_event_with_label_and_attempted_binary() {
        use std::sync::{Arc, Mutex};
        use tracing::{Event, Level, Subscriber};
        use tracing_subscriber::{
            layer::{Context, Layer, SubscriberExt},
            registry::LookupSpan,
        };

        #[derive(Clone, Debug)]
        struct CapturedEvent {
            level: Level,
            target: String,
            fields: std::collections::HashMap<String, String>,
        }

        struct CaptureLayer {
            events: Arc<Mutex<Vec<CapturedEvent>>>,
        }

        struct CaptureVisitor<'a>(&'a mut CapturedEvent);

        impl<'a> tracing::field::Visit for CaptureVisitor<'a> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0
                    .fields
                    .insert(field.name().to_string(), format!("{value:?}"));
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.0
                    .fields
                    .insert(field.name().to_string(), value.to_string());
            }
            fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
                self.0
                    .fields
                    .insert(field.name().to_string(), value.to_string());
            }
        }

        impl<S> Layer<S> for CaptureLayer
        where
            S: Subscriber + for<'a> LookupSpan<'a>,
        {
            fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
                let mut captured = CapturedEvent {
                    level: *event.metadata().level(),
                    target: event.metadata().target().to_string(),
                    fields: std::collections::HashMap::new(),
                };
                event.record(&mut CaptureVisitor(&mut captured));
                self.events.lock().unwrap().push(captured);
            }
        }

        let _lock = docker_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous_bin = std::env::var_os("GWT_DOCKER_BIN");
        std::env::set_var("GWT_DOCKER_BIN", "/this-binary-does-not-exist-gwt-test");

        let events = Arc::new(Mutex::new(Vec::<CapturedEvent>::new()));
        let layer = CaptureLayer {
            events: Arc::clone(&events),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            let _ = docker_available();
        });

        match previous_bin {
            Some(value) => std::env::set_var("GWT_DOCKER_BIN", value),
            None => std::env::remove_var("GWT_DOCKER_BIN"),
        }

        let captured = events.lock().unwrap().clone();
        let info_events: Vec<_> = captured
            .iter()
            .filter(|event| event.level == Level::INFO && event.target == "gwt::launch::probe")
            .collect();
        assert!(
            !info_events.is_empty(),
            "expected at least one INFO event with target gwt::launch::probe; captured = {:?}",
            captured
        );
        let event = info_events[0];
        assert_eq!(
            event.fields.get("label").map(String::as_str),
            Some("docker CLI")
        );
        assert!(event.fields.contains_key("attempted_binary"));
    }

    fn docker_test_lock() -> &'static std::sync::Mutex<()> {
        crate::docker_env_test_lock()
    }

    fn write_compose_failing_fake_docker(dir: &Path) -> std::path::PathBuf {
        #[cfg(windows)]
        {
            let script_path = dir.join("docker.cmd");
            std::fs::write(
                &script_path,
                "@echo off\r\nif \"%1\"==\"compose\" (\r\n  echo docker: unknown command: docker compose 1>&2\r\n  exit /b 1\r\n)\r\nexit /b 0\r\n",
            )
            .expect("write fake docker");
            script_path
        }

        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let script_path = dir.join("docker");
            std::fs::write(
                &script_path,
                "#!/bin/sh\nif [ \"$1\" = \"compose\" ]; then\n  echo 'docker: unknown command: docker compose' >&2\n  exit 1\nfi\nexit 0\n",
            )
            .expect("write fake docker");
            let mut perms = std::fs::metadata(&script_path)
                .expect("stat fake docker")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).expect("chmod fake docker");
            script_path
        }
    }

    #[test]
    fn launch_preflight_includes_probe_stderr_in_failure_message() {
        let _lock = docker_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let script_path = write_compose_failing_fake_docker(tmp.path());
        let previous_bin = std::env::var_os("GWT_DOCKER_BIN");
        std::env::set_var("GWT_DOCKER_BIN", &script_path);

        let result = launch_preflight();

        match previous_bin {
            Some(value) => std::env::set_var("GWT_DOCKER_BIN", value),
            None => std::env::remove_var("GWT_DOCKER_BIN"),
        }

        let message = result.expect_err("compose probe should fail");
        assert!(
            message.contains("docker compose is not available"),
            "summary missing: {message}"
        );
        assert!(
            message.contains("unknown command"),
            "stderr hint missing: {message}"
        );
    }

    #[test]
    fn preflight_message_appends_single_line_detail() {
        assert_eq!(
            preflight_message("docker compose is not available", ""),
            "docker compose is not available"
        );
        assert_eq!(
            preflight_message(
                "docker compose is not available",
                "docker: unknown command\nRun 'docker --help'"
            ),
            "docker compose is not available (docker: unknown command)"
        );
    }
}
