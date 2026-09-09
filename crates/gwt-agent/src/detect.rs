//! Agent detection: discover installed coding agents via PATH lookup.

use std::{
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use tracing::debug;

/// Upper bound for one `<command> --version` probe (SPEC-3864 T-006). A CLI
/// that hangs (or waits on a network registry) must not stall wizard
/// detection; the executable already resolved on PATH, so on timeout the
/// agent is still reported installed, just without a version string.
pub const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

use crate::types::{builtin_agent_descriptor_for_command, builtin_agent_descriptors, AgentId};

/// Result of detecting a single agent on the system.
#[derive(Debug, Clone)]
pub struct DetectedAgent {
    pub agent_id: AgentId,
    pub version: Option<String>,
    pub path: PathBuf,
}

/// Definition used internally to probe for a known agent.
struct AgentProbe {
    id: AgentId,
    command: &'static str,
    version_flag: &'static str,
    /// Extra subcommand args needed before the version flag (e.g. `gh copilot`).
    prefix_args: &'static [&'static str],
}

/// All builtin agents we attempt to detect.
fn builtin_probes() -> Vec<AgentProbe> {
    builtin_agent_descriptors()
        .iter()
        .map(|descriptor| AgentProbe {
            id: descriptor.id.clone(),
            command: descriptor.command,
            version_flag: descriptor.version_flag,
            prefix_args: descriptor.version_prefix_args,
        })
        .collect()
}

/// Detects installed coding agents.
pub struct AgentDetector;

impl AgentDetector {
    /// Scan the system for all known builtin agents.
    ///
    /// Each probe spawns `<command> --version`, so the probes run on scoped
    /// threads and the wall-clock cost is bounded by the slowest CLI rather
    /// than the sum of all nine (SPEC-3864 T-006). Results keep descriptor
    /// order so callers see a deterministic list.
    pub fn detect_all() -> Vec<DetectedAgent> {
        let probes = builtin_probes();
        std::thread::scope(|scope| {
            let handles: Vec<_> = probes
                .iter()
                .map(|probe| scope.spawn(move || Self::detect_one(probe)))
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| handle.join().ok().flatten())
                .collect()
        })
    }

    /// Detect a single agent by its command name.
    pub fn detect_by_command(command: &str) -> Option<DetectedAgent> {
        Self::detect_by_command_in_env(command, &[])
    }

    /// Detect a single agent by its command name with `env` layered over the
    /// host environment for executable lookup and the version probe child.
    ///
    /// The process environment is never touched: `PATH` is process-global and
    /// mutating it from one test thread makes every parallel `sh` / `git`
    /// spawn fail (Issue #3895), so fixtures pass their `PATH` here instead.
    pub(crate) fn detect_by_command_in_env(
        command: &str,
        env: &[(&str, &OsStr)],
    ) -> Option<DetectedAgent> {
        let descriptor = builtin_agent_descriptor_for_command(command);
        let (version, resolved_path) = match descriptor {
            Some(descriptor) => Self::fetch_version(
                command,
                descriptor.version_flag,
                descriptor.version_prefix_args,
                env,
            ),
            None => Self::fetch_version(command, "--version", &[], env),
        }
        .ok()?;
        let path = if cfg!(windows) {
            resolved_path
        } else {
            which_in_env(command, env).ok()?
        };
        // Map known commands to AgentIds, fall back to Custom
        let agent_id = descriptor
            .map(|descriptor| descriptor.id.clone())
            .unwrap_or_else(|| AgentId::Custom(command.to_string()));
        Some(DetectedAgent {
            agent_id,
            version,
            path,
        })
    }

    fn detect_one(probe: &AgentProbe) -> Option<DetectedAgent> {
        let (version, resolved_path) =
            Self::fetch_version(probe.command, probe.version_flag, probe.prefix_args, &[]).ok()?;
        let path = if cfg!(windows) {
            resolved_path
        } else {
            which::which(probe.command).ok()?
        };
        debug!(
            agent = %probe.id,
            path = %path.display(),
            "Found agent binary"
        );
        Some(DetectedAgent {
            agent_id: probe.id.clone(),
            version,
            path,
        })
    }

    fn fetch_version(
        command: &str,
        version_flag: &str,
        prefix_args: &[&str],
        env: &[(&str, &OsStr)],
    ) -> Result<(Option<String>, PathBuf), String> {
        let request = env.iter().fold(
            gwt_core::process::ProcessPlanRequest::new(command)
                .args(prefix_args)
                .arg(version_flag),
            |request, (key, value)| request.env(key, value),
        );
        let mut cmd = gwt_core::process::resolved_command(request).map_err(|error| {
            debug!(command, error = %error, "Agent version probe resolution failed");
            error.to_string()
        })?;
        let resolved_path = PathBuf::from(cmd.get_program());
        let version = Self::probe_version_bounded(&mut cmd, VERSION_PROBE_TIMEOUT)?;
        Ok((version, resolved_path))
    }

    /// Run the version probe with a hard deadline. Returns `Ok(None)` when
    /// the probe fails, prints nothing, or exceeds the deadline (the child is
    /// killed); `Err` only when the process could not be spawned at all.
    fn probe_version_bounded(
        cmd: &mut std::process::Command,
        timeout: Duration,
    ) -> Result<Option<String>, String> {
        let mut child = cmd
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        // The reader is detached rather than joined: a descendant that
        // inherits the pipe (e.g. a shell wrapper's child) would otherwise
        // hold the read open past the child's exit or kill.
        let (stdout_tx, stdout_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        if let Some(mut stdout) = child.stdout.take() {
            std::thread::spawn(move || {
                let mut raw = Vec::new();
                let _ = stdout.read_to_end(&mut raw);
                let _ = stdout_tx.send(raw);
            });
        }
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Ok(None) => {
                    debug!(?timeout, "Agent version probe timed out; killing probe");
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Err(error) => {
                    debug!(error = %error, "Agent version probe wait failed");
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let Some(status) = status else {
            return Ok(None);
        };
        if !status.success() {
            return Ok(None);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Ok(stdout) = stdout_rx.recv_timeout(remaining.max(Duration::from_millis(250))) else {
            return Ok(None);
        };
        let raw = String::from_utf8_lossy(&stdout).trim().to_string();
        Ok((!raw.is_empty()).then_some(raw))
    }
}

/// `which` lookup honoring a `PATH` override from `env` (last one wins) and
/// falling back to the process `PATH` when `env` carries none.
fn which_in_env(command: &str, env: &[(&str, &OsStr)]) -> which::Result<PathBuf> {
    match env
        .iter()
        .rev()
        .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
    {
        Some((_, path)) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
            which::which_in(command, Some(path), cwd)
        }
        None => which::which(command),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_all_returns_vec() {
        // Should not panic; returns whatever is installed
        let agents = AgentDetector::detect_all();
        // We cannot assert specific agents are installed, but the function should be safe
        for agent in &agents {
            assert!(!agent.path.as_os_str().is_empty());
        }
    }

    #[test]
    fn detect_by_command_nonexistent() {
        assert!(AgentDetector::detect_by_command("gwt_nonexistent_agent_xyz").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn detector_resolves_real_bun_global_placeholder_fixture() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture =
            gwt_core::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
                .expect("create real Windows Bun fixture");

        let detected =
            AgentDetector::detect_by_command_in_env("claude", &windows_fixture_env(&fixture))
                .expect("safe fixture must be detected as Claude Code");

        assert_eq!(detected.agent_id, AgentId::ClaudeCode);
        assert_eq!(detected.version.as_deref(), Some("2.1.210 (Claude Code)"));
        assert_eq!(detected.path, fixture.bun_exe);
    }

    #[cfg(windows)]
    #[test]
    fn detector_rejects_real_bun_global_placeholder_fixture_without_safe_target() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture =
            gwt_core::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
                .expect("create real Windows Bun fixture");
        fixture
            .remove_safe_targets()
            .expect("remove safe redirect targets");

        assert!(
            AgentDetector::detect_by_command_in_env("claude", &windows_fixture_env(&fixture))
                .is_none()
        );
    }

    /// Probe environment for the real Windows Bun fixture, layered over the
    /// host environment for the probe only (Issue #3895).
    #[cfg(windows)]
    fn windows_fixture_env(
        fixture: &gwt_core::test_support::WindowsBunClaudeFixture,
    ) -> [(&'static str, &OsStr); 3] {
        [
            ("PATH", fixture.bun_bin.as_os_str()),
            ("PATHEXT", OsStr::new(".COM;.EXE;.BAT;.CMD")),
            ("USERPROFILE", fixture.profile.as_os_str()),
        ]
    }

    #[test]
    fn detect_by_command_maps_known() {
        // Use a command that definitely exists
        if let Some(detected) = AgentDetector::detect_by_command("git") {
            assert_eq!(detected.agent_id, AgentId::Custom("git".to_string()));
        }
    }

    #[cfg(unix)]
    #[test]
    fn detect_by_command_maps_grok_build_and_version() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let executable = temp.path().join("grok");
        std::fs::write(&executable, "#!/bin/sh\nprintf '1.0.3\\n'\n")
            .expect("write Grok Build fixture");
        let mut permissions = std::fs::metadata(&executable)
            .expect("read Grok Build fixture metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions)
            .expect("make Grok Build fixture executable");

        // The fixture PATH is injected into the probe only; the process PATH
        // stays untouched so parallel `sh` / `git` spawns keep resolving
        // (Issue #3895).
        let detected =
            AgentDetector::detect_by_command_in_env("grok", &[("PATH", temp.path().as_os_str())])
                .expect("Grok Build fixture must be detected");

        assert_eq!(detected.agent_id, AgentId::GrokBuild);
        assert_eq!(detected.version.as_deref(), Some("1.0.3"));
        assert_eq!(detected.path, executable);
    }

    #[cfg(not(windows))]
    #[test]
    fn detect_by_command_preserves_the_absolute_which_path_off_windows() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let expected = which::which("git").expect("git must be available to the test runner");

        // `detect_by_command` spawns `git --version` to read a version string,
        // and a heavily loaded machine can make that child spawn transiently
        // fail (e.g. EAGAIN on fork), yielding a spurious None while the
        // spawn-free `which` lookup still succeeds. Retry a few times so only a
        // genuine detection failure (every attempt None) fails the test
        // (issue #3339).
        let detected = (0..8)
            .find_map(|_| {
                AgentDetector::detect_by_command("git").or_else(|| {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    None
                })
            })
            .expect("git must be detected");

        assert_eq!(detected.path, expected);
    }

    /// SPEC-3864 T-006: a version probe that never returns must not hang
    /// detection. The executable resolved on PATH, so the agent is still
    /// reported installed — just without a version string.
    #[cfg(unix)]
    #[test]
    fn detect_by_command_bounds_a_hanging_version_probe() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let executable = temp.path().join("agy");
        std::fs::write(&executable, "#!/bin/sh\nsleep 30\n").expect("write hanging fixture");
        let mut permissions = std::fs::metadata(&executable)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).expect("chmod fixture");

        // The fixture PATH is injected into the probe only; the process PATH
        // stays untouched so parallel `sh` / `git` spawns keep resolving
        // (Issue #3895).
        let started = std::time::Instant::now();
        let detected =
            AgentDetector::detect_by_command_in_env("agy", &[("PATH", temp.path().as_os_str())])
                .expect("resolvable executable stays detected when its probe hangs");
        let elapsed = started.elapsed();

        assert_eq!(detected.agent_id, AgentId::Antigravity);
        assert_eq!(detected.path, executable);
        assert_eq!(detected.version, None);
        assert!(
            elapsed < VERSION_PROBE_TIMEOUT + std::time::Duration::from_secs(3),
            "probe must be bounded by {VERSION_PROBE_TIMEOUT:?}, took {elapsed:?}"
        );
    }

    #[test]
    fn builtin_probes_cover_all_variants() {
        let probes = builtin_probes();
        assert_eq!(probes.len(), 9);
        let ids: Vec<_> = probes.iter().map(|p| &p.id).collect();
        assert!(ids.contains(&&AgentId::ClaudeCode));
        assert!(ids.contains(&&AgentId::Codex));
        assert!(ids.contains(&&AgentId::GrokBuild));
        assert!(ids.contains(&&AgentId::Antigravity));
        assert!(ids.contains(&&AgentId::Gemini));
        assert!(ids.contains(&&AgentId::OpenCode));
        assert!(ids.contains(&&AgentId::OpenClaw));
        assert!(ids.contains(&&AgentId::Hermes));
        assert!(ids.contains(&&AgentId::Copilot));
    }

    #[test]
    fn detected_agent_debug() {
        let agent = DetectedAgent {
            agent_id: AgentId::ClaudeCode,
            version: Some("1.0.0".into()),
            path: PathBuf::from("/usr/bin/claude"),
        };
        let debug = format!("{:?}", agent);
        assert!(debug.contains("ClaudeCode"));
    }
}
