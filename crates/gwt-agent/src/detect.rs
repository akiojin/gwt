//! Agent detection: discover installed coding agents via PATH lookup.

use std::path::PathBuf;

use tracing::debug;

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
    pub fn detect_all() -> Vec<DetectedAgent> {
        let mut found = Vec::new();
        for probe in builtin_probes() {
            if let Some(detected) = Self::detect_one(&probe) {
                found.push(detected);
            }
        }
        found
    }

    /// Detect a single agent by its command name.
    pub fn detect_by_command(command: &str) -> Option<DetectedAgent> {
        let descriptor = builtin_agent_descriptor_for_command(command);
        let version = match descriptor {
            Some(descriptor) => Self::fetch_version(
                command,
                descriptor.version_flag,
                descriptor.version_prefix_args,
            ),
            None => Self::fetch_version(command, "--version", &[]),
        }
        .ok()?;
        let resolved_path = gwt_core::process::resolve_process_plan(
            gwt_core::process::ProcessPlanRequest::new(command),
        )
        .ok()?
        .program;
        let path = if cfg!(windows) {
            resolved_path
        } else {
            which::which(command).ok()?
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
        let resolved_path = gwt_core::process::resolve_process_plan(
            gwt_core::process::ProcessPlanRequest::new(probe.command),
        )
        .ok()?
        .program;
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
        let version =
            Self::fetch_version(probe.command, probe.version_flag, probe.prefix_args).ok()?;
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
    ) -> Result<Option<String>, String> {
        let request = gwt_core::process::ProcessPlanRequest::new(command)
            .args(prefix_args)
            .arg(version_flag);
        let mut cmd = gwt_core::process::resolved_command(request).map_err(|error| {
            debug!(command, error = %error, "Agent version probe resolution failed");
            error.to_string()
        })?;
        let output = cmd.output().map_err(|error| error.to_string())?;
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if raw.is_empty() {
                Ok(None)
            } else {
                Ok(Some(raw))
            }
        } else {
            Ok(None)
        }
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
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture =
            gwt_core::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
                .expect("create real Windows Bun fixture");
        let _path = gwt_core::test_support::ScopedEnvVar::set("PATH", &fixture.bun_bin);
        let _path_ext = gwt_core::test_support::ScopedEnvVar::set("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", &fixture.profile);

        let detected = AgentDetector::detect_by_command("claude")
            .expect("safe fixture must be detected as Claude Code");

        assert_eq!(detected.agent_id, AgentId::ClaudeCode);
        assert_eq!(detected.version.as_deref(), Some("2.1.210 (Claude Code)"));
        assert_eq!(detected.path, fixture.bun_exe);
    }

    #[cfg(windows)]
    #[test]
    fn detector_rejects_real_bun_global_placeholder_fixture_without_safe_target() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture =
            gwt_core::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
                .expect("create real Windows Bun fixture");
        fixture
            .remove_safe_targets()
            .expect("remove safe redirect targets");
        let _path = gwt_core::test_support::ScopedEnvVar::set("PATH", &fixture.bun_bin);
        let _path_ext = gwt_core::test_support::ScopedEnvVar::set("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", &fixture.profile);

        assert!(AgentDetector::detect_by_command("claude").is_none());
    }

    #[test]
    fn detect_by_command_maps_known() {
        // Use a command that definitely exists
        if let Some(detected) = AgentDetector::detect_by_command("git") {
            assert_eq!(detected.agent_id, AgentId::Custom("git".to_string()));
        }
    }

    #[test]
    fn builtin_probes_cover_all_variants() {
        let probes = builtin_probes();
        assert_eq!(probes.len(), 8);
        let ids: Vec<_> = probes.iter().map(|p| &p.id).collect();
        assert!(ids.contains(&&AgentId::ClaudeCode));
        assert!(ids.contains(&&AgentId::Codex));
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
