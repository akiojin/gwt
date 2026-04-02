//! Agent detection: discover installed coding agents via PATH lookup.

use std::path::PathBuf;

use tracing::debug;

use crate::types::AgentId;

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
    vec![
        AgentProbe {
            id: AgentId::ClaudeCode,
            command: "claude",
            version_flag: "--version",
            prefix_args: &[],
        },
        AgentProbe {
            id: AgentId::Codex,
            command: "codex",
            version_flag: "--version",
            prefix_args: &[],
        },
        AgentProbe {
            id: AgentId::Gemini,
            command: "gemini",
            version_flag: "--version",
            prefix_args: &[],
        },
        AgentProbe {
            id: AgentId::OpenCode,
            command: "opencode",
            version_flag: "--version",
            prefix_args: &[],
        },
        AgentProbe {
            id: AgentId::Copilot,
            command: "gh",
            version_flag: "--version",
            prefix_args: &["copilot"],
        },
    ]
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
        let path = which::which(command).ok()?;
        let version = Self::fetch_version(command, "--version", &[]);
        // Map known commands to AgentIds, fall back to Custom
        let agent_id = match command {
            "claude" => AgentId::ClaudeCode,
            "codex" => AgentId::Codex,
            "gemini" => AgentId::Gemini,
            "opencode" => AgentId::OpenCode,
            "gh" => AgentId::Copilot,
            other => AgentId::Custom(other.to_string()),
        };
        Some(DetectedAgent {
            agent_id,
            version,
            path,
        })
    }

    fn detect_one(probe: &AgentProbe) -> Option<DetectedAgent> {
        let path = which::which(probe.command).ok()?;
        debug!(
            agent = %probe.id,
            path = %path.display(),
            "Found agent binary"
        );
        let version = Self::fetch_version(probe.command, probe.version_flag, probe.prefix_args);
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
    ) -> Option<String> {
        let mut cmd = gwt_core::process::command(command);
        for arg in prefix_args {
            cmd.arg(arg);
        }
        cmd.arg(version_flag);
        let output = cmd.output().ok()?;
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if raw.is_empty() {
                None
            } else {
                Some(raw)
            }
        } else {
            None
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
        assert_eq!(probes.len(), 5);
        let ids: Vec<_> = probes.iter().map(|p| &p.id).collect();
        assert!(ids.contains(&&AgentId::ClaudeCode));
        assert!(ids.contains(&&AgentId::Codex));
        assert!(ids.contains(&&AgentId::Gemini));
        assert!(ids.contains(&&AgentId::OpenCode));
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
