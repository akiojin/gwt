//! Docker environment detection.
//!
//! Checks for Docker CLI availability, daemon status, and discovers
//! Docker-related files (Dockerfile, docker-compose.yml, .devcontainer/).

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use tracing::{debug, info};

/// Reserved hostname exposed by Docker for reaching the host from a container.
pub const DOCKER_HOST_BRIDGE_NAME: &str = "host.docker.internal";
/// Reserved hostname exposed by Podman for reaching the host from a container.
pub const PODMAN_HOST_BRIDGE_NAME: &str = "host.containers.internal";
/// Compose mapping required to make Docker's reserved host alias available on
/// Linux as well as Docker Desktop.
pub const DOCKER_HOST_GATEWAY_EXTRA_HOST: &str = "host.docker.internal:host-gateway";
const CONTAINER_RUNTIME_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Container CLI selected for a launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerRuntimeKind {
    Docker,
    Podman,
}

impl ContainerRuntimeKind {
    /// Reserved hostname that reaches the host from this runtime.
    pub fn host_bridge_name(self) -> &'static str {
        match self {
            Self::Docker => DOCKER_HOST_BRIDGE_NAME,
            Self::Podman => PODMAN_HOST_BRIDGE_NAME,
        }
    }

    /// Compose `extra_hosts` entry required by this runtime, if any.
    pub fn compose_extra_host(self) -> Option<&'static str> {
        match self {
            Self::Docker => Some(DOCKER_HOST_GATEWAY_EXTRA_HOST),
            Self::Podman => None,
        }
    }
}

/// Container runtime identity resolved once for a launch.
///
/// Keeping the configured binary and detected kind together prevents a
/// stateful wrapper from being re-probed by each launch-contract consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedContainerRuntime {
    binary: String,
    kind: ContainerRuntimeKind,
}

impl ResolvedContainerRuntime {
    /// Resolve the configured CLI once and pin its runtime kind.
    pub fn resolve(container_runtime_binary: &str) -> Result<Self, String> {
        Self::resolve_with_timeout(container_runtime_binary, CONTAINER_RUNTIME_PROBE_TIMEOUT)
    }

    fn resolve_with_timeout(
        container_runtime_binary: &str,
        timeout: Duration,
    ) -> Result<Self, String> {
        let binary = container_runtime_binary.trim();
        if binary.is_empty() {
            return Err(
                "container launch requires the Docker or Podman CLI, but GWT_DOCKER_BIN is empty"
                    .to_string(),
            );
        }
        let kind = probe_container_runtime_kind_with_timeout(binary, timeout)?;
        Ok(Self {
            binary: binary.to_string(),
            kind,
        })
    }

    /// Configured CLI binary associated with this resolved runtime.
    pub fn binary(&self) -> &str {
        &self.binary
    }

    /// Runtime kind pinned when this value was resolved.
    pub fn kind(&self) -> ContainerRuntimeKind {
        self.kind
    }
}

/// Derive the runtime contract from the configured container CLI binary.
pub fn container_runtime_kind(
    container_runtime_binary: &str,
) -> Result<ContainerRuntimeKind, String> {
    let binary_name = container_runtime_binary
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    match binary_name.trim_end_matches(".exe") {
        "docker" => Ok(ContainerRuntimeKind::Docker),
        "podman" | "podman-remote" => Ok(ContainerRuntimeKind::Podman),
        _ => probe_container_runtime_kind(container_runtime_binary),
    }
}

fn probe_container_runtime_kind(
    container_runtime_binary: &str,
) -> Result<ContainerRuntimeKind, String> {
    probe_container_runtime_kind_with_timeout(
        container_runtime_binary,
        CONTAINER_RUNTIME_PROBE_TIMEOUT,
    )
}

fn probe_container_runtime_kind_with_timeout(
    container_runtime_binary: &str,
    timeout: Duration,
) -> Result<ContainerRuntimeKind, String> {
    let binary = container_runtime_binary.trim();
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        "container runtime probe deadline exceeds the supported clock range".to_string()
    })?;
    let hub = gwt_core::process_console::global();
    let options = gwt_core::process_console::SpawnOptions::new("container runtime --version")
        .forward_output(false);
    let output = gwt_core::process_console::spawn_logged_blocking_with_deadline(
        &hub,
        gwt_core::process_console::ProcessKind::Docker,
        binary,
        &["--version"],
        options,
        deadline,
    )
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            format!(
                "container launch requires the Docker or Podman CLI, but GWT_DOCKER_BIN '{container_runtime_binary}' timed out during its --version probe after {}ms",
                timeout.as_millis()
            )
        } else {
            format!(
                "container launch requires the Docker or Podman CLI, but GWT_DOCKER_BIN '{container_runtime_binary}' could not be probed with --version: {error}"
            )
        }
    })?;
    if !output.success() {
        return Err(format!(
            "container launch requires the Docker or Podman CLI, but GWT_DOCKER_BIN '{container_runtime_binary}' failed its --version probe"
        ));
    }

    container_runtime_kind_from_version_output(output.stdout.as_bytes(), output.stderr.as_bytes())
        .ok_or_else(|| {
            format!(
                "container launch requires the Docker or Podman CLI, but GWT_DOCKER_BIN '{container_runtime_binary}' did not identify itself as either runtime"
            )
        })
}

fn container_runtime_kind_from_version_output(
    stdout: &[u8],
    stderr: &[u8],
) -> Option<ContainerRuntimeKind> {
    let output = String::from_utf8_lossy(stdout);
    let errors = String::from_utf8_lossy(stderr);
    let mut detected = None;
    for line in output.lines().chain(errors.lines()) {
        let normalized = line.trim().to_ascii_lowercase();
        let candidate = if normalized.starts_with("docker version ") {
            Some(ContainerRuntimeKind::Docker)
        } else if normalized.starts_with("podman version ") {
            Some(ContainerRuntimeKind::Podman)
        } else {
            None
        };
        let Some(candidate) = candidate else {
            continue;
        };
        match detected {
            None => detected = Some(candidate),
            Some(current) if current == candidate => {}
            Some(_) => return None,
        }
    }
    detected
}

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
    let binary = docker_binary();
    docker_probe_diagnostics_with_binary(&binary, args, label)
}

fn docker_probe_diagnostics_with_binary(
    binary: &OsStr,
    args: &[&str],
    label: &str,
) -> std::result::Result<(), String> {
    // SPEC-2809 / SPEC-1924 Phase D-docker — route docker probes through
    // `spawn_logged_blocking` so the docker tab of the Console window /
    // Logs Process facet sees them. The `binary` may be a `GWT_DOCKER_BIN`
    // override; pass it as the program directly.
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
        binary,
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

/// Preflight a runtime whose binary and kind were already resolved for this
/// launch. The kind probe is deliberately not repeated.
pub fn launch_preflight_for_resolved_runtime(
    runtime: &ResolvedContainerRuntime,
) -> std::result::Result<(), String> {
    let binary = OsStr::new(runtime.binary());
    docker_probe_diagnostics_with_binary(binary, &["compose", "version"], "docker compose")
        .map_err(|detail| preflight_message("docker compose is not available", &detail))?;
    docker_probe_diagnostics_with_binary(binary, &["info"], "daemon")
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

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &str) {
        use std::os::unix::fs::PermissionsExt;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create executable parent");
        }
        std::fs::write(path, contents).expect("write executable");
        let mut permissions = std::fs::metadata(path)
            .expect("executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod executable");
    }

    #[test]
    fn container_runtime_kind_is_derived_from_the_selected_cli_binary() {
        for binary in ["docker", r"C:\Program Files\Docker\DOCKER.EXE"] {
            assert_eq!(
                container_runtime_kind(binary).expect("Docker runtime"),
                ContainerRuntimeKind::Docker
            );
        }
        for binary in ["podman", "/opt/homebrew/bin/podman-remote"] {
            assert_eq!(
                container_runtime_kind(binary).expect("Podman runtime"),
                ContainerRuntimeKind::Podman
            );
        }

        let error = container_runtime_kind("custom-container-wrapper")
            .expect_err("unknown container runtime must fail closed");
        assert!(
            error.contains("Docker or Podman"),
            "unexpected error: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn container_runtime_kind_probes_a_configured_cli_wrapper() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("tempdir");
        let wrapper = temp.path().join("docker-wrapper");
        std::fs::write(
            &wrapper,
            "#!/bin/sh\nprintf 'Docker version 28.3.0, build test\\n'\n",
        )
        .expect("write Docker wrapper");
        let mut permissions = std::fs::metadata(&wrapper)
            .expect("wrapper metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions).expect("chmod Docker wrapper");

        assert_eq!(
            container_runtime_kind(wrapper.to_str().expect("UTF-8 wrapper path"))
                .expect("wrapper runtime"),
            ContainerRuntimeKind::Docker
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolved_runtime_probes_known_basename_once_and_normalizes_the_binary() {
        let temp = TempDir::new().expect("tempdir");
        let wrapper = temp.path().join("docker");
        write_executable(
            &wrapper,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"${0}.calls\"\nprintf 'podman version 5.4.2\\n'\n",
        );
        let configured = format!("  {}  ", wrapper.display());

        let runtime =
            ResolvedContainerRuntime::resolve(&configured).expect("resolve masquerading wrapper");

        assert_eq!(runtime.kind(), ContainerRuntimeKind::Podman);
        assert_eq!(
            runtime.binary(),
            wrapper.to_str().expect("UTF-8 wrapper path")
        );
        let calls =
            std::fs::read_to_string(wrapper.with_extension("calls")).expect("read probe calls");
        assert_eq!(calls.lines().collect::<Vec<_>>(), ["--version"]);
    }

    #[cfg(unix)]
    #[test]
    fn resolved_runtime_rejects_ambiguous_or_failed_known_basename_wrappers() {
        let temp = TempDir::new().expect("tempdir");
        let ambiguous = temp.path().join("ambiguous").join("docker");
        write_executable(
            &ambiguous,
            "#!/bin/sh\nprintf 'Docker version 28.3.0, build test\\n'\nprintf 'podman version 5.4.2\\n' >&2\n",
        );
        let failed = temp.path().join("failed").join("docker");
        write_executable(
            &failed,
            "#!/bin/sh\nprintf 'Docker version 28.3.0, build test\\n'\nexit 19\n",
        );

        let ambiguous_error = ResolvedContainerRuntime::resolve(
            ambiguous.to_str().expect("UTF-8 ambiguous wrapper path"),
        )
        .expect_err("ambiguous known basename must fail closed");
        let failed_error =
            ResolvedContainerRuntime::resolve(failed.to_str().expect("UTF-8 failed wrapper path"))
                .expect_err("failed known basename must fail closed");

        assert!(
            ambiguous_error.contains("did not identify"),
            "{ambiguous_error}"
        );
        assert!(
            failed_error.contains("failed its --version probe"),
            "{failed_error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolved_runtime_times_out_a_stuck_known_basename_wrapper() {
        let temp = TempDir::new().expect("tempdir");
        let wrapper = temp.path().join("docker");
        write_executable(&wrapper, "#!/bin/sh\nexec sleep 30\n");

        let started = std::time::Instant::now();
        let error = ResolvedContainerRuntime::resolve_with_timeout(
            wrapper.to_str().expect("UTF-8 wrapper path"),
            Duration::from_millis(50),
        )
        .expect_err("stuck known basename must fail closed");

        assert!(error.contains("timed out"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the bounded resolver must terminate promptly"
        );
    }

    #[cfg(unix)]
    #[test]
    fn container_runtime_kind_times_out_a_stuck_cli_wrapper() {
        use std::{os::unix::fs::PermissionsExt, time::Duration};

        let temp = TempDir::new().expect("tempdir");
        let wrapper = temp.path().join("stuck-container-wrapper");
        std::fs::write(&wrapper, "#!/bin/sh\nexec sleep 30\n").expect("write stuck wrapper");
        let mut permissions = std::fs::metadata(&wrapper)
            .expect("wrapper metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions).expect("chmod stuck wrapper");

        let started = std::time::Instant::now();
        let error = probe_container_runtime_kind_with_timeout(
            wrapper.to_str().expect("UTF-8 wrapper path"),
            Duration::from_millis(50),
        )
        .expect_err("a stuck wrapper must fail closed");

        assert!(error.contains("timed out"), "unexpected error: {error}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the bounded probe must not wait for the wrapper's sleep"
        );
    }

    #[cfg(unix)]
    #[test]
    fn container_runtime_kind_timeout_terminates_non_exec_descendants() {
        use std::{os::unix::fs::PermissionsExt, time::Duration};

        let temp = TempDir::new().expect("tempdir");
        let wrapper = temp.path().join("descendant-container-wrapper");
        let marker = wrapper.with_extension("marker");
        let ready = wrapper.with_extension("ready");
        std::fs::write(
            &wrapper,
            "#!/bin/sh\nprintf ready > \"${0}.ready\"\n(trap '' HUP TERM; sleep 3; printf leaked > \"${0}.marker\") </dev/null >/dev/null 2>&1 &\nsleep 0.1\nwhile :; do sleep 1; done\n",
        )
        .expect("write descendant wrapper");
        let mut permissions = std::fs::metadata(&wrapper)
            .expect("wrapper metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions).expect("chmod descendant wrapper");

        let error = probe_container_runtime_kind_with_timeout(
            wrapper.to_str().expect("UTF-8 wrapper path"),
            Duration::from_secs(2),
        )
        .expect_err("a stuck wrapper must fail closed");
        assert!(error.contains("timed out"), "unexpected error: {error}");
        assert!(
            ready.exists(),
            "the non-exec descendant must start before the wrapper timeout"
        );

        std::thread::sleep(Duration::from_millis(3_200));
        assert!(
            !marker.exists(),
            "timeout cleanup must terminate the wrapper process tree before a descendant can act"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolved_runtime_preflight_reuses_the_pinned_binary_without_a_second_kind_probe() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("tempdir");
        let wrapper = temp.path().join("stateful-container-wrapper");
        let calls = wrapper.with_extension("calls");
        std::fs::write(
            &wrapper,
            r#"#!/bin/sh
printf '%s\n' "$*" >> "${0}.calls"
case "$*" in
  "--version")
    printf 'Docker version 28.3.0, build test\n'
    ;;
  "compose version"|"info")
    exit 0
    ;;
  *)
    exit 9
    ;;
esac
"#,
        )
        .expect("write stateful wrapper");
        let mut permissions = std::fs::metadata(&wrapper)
            .expect("wrapper metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions).expect("chmod stateful wrapper");

        let runtime =
            ResolvedContainerRuntime::resolve(wrapper.to_str().expect("UTF-8 wrapper path"))
                .expect("resolve runtime");
        launch_preflight_for_resolved_runtime(&runtime).expect("resolved runtime preflight");

        let calls = std::fs::read_to_string(calls).expect("read wrapper calls");
        assert_eq!(
            calls.lines().filter(|call| *call == "--version").count(),
            1,
            "preflight must not repeat runtime-kind detection"
        );
        assert!(calls.lines().any(|call| call == "compose version"));
        assert!(calls.lines().any(|call| call == "info"));
    }

    #[test]
    fn container_runtime_kind_parser_accepts_podman_identity_from_stderr() {
        assert_eq!(
            container_runtime_kind_from_version_output(
                b"wrapper diagnostics\n",
                b"podman version 5.4.2\n",
            ),
            Some(ContainerRuntimeKind::Podman)
        );
    }

    #[test]
    fn container_runtime_kind_parser_rejects_ambiguous_runtime_identity() {
        assert_eq!(
            container_runtime_kind_from_version_output(
                b"Docker version 28.3.0, build test\n",
                b"podman version 5.4.2\n",
            ),
            None,
            "a wrapper that claims both runtime contracts must fail closed"
        );
    }

    #[test]
    fn container_runtime_kind_exposes_reserved_host_bridge_contract() {
        assert_eq!(
            ContainerRuntimeKind::Docker.host_bridge_name(),
            DOCKER_HOST_BRIDGE_NAME
        );
        assert_eq!(
            ContainerRuntimeKind::Docker.compose_extra_host(),
            Some(DOCKER_HOST_GATEWAY_EXTRA_HOST)
        );
        assert_eq!(
            ContainerRuntimeKind::Podman.host_bridge_name(),
            PODMAN_HOST_BRIDGE_NAME
        );
        assert_eq!(ContainerRuntimeKind::Podman.compose_extra_host(), None);
    }

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
