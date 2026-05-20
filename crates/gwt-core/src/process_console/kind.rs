//! `ProcessKind` — taxonomy for external processes that gwt spawns.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Logical category of an external process for the Console facet.
///
/// The set is intentionally small and closed. Adding a new variant
/// requires a SPEC update (#1924 FR-038).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProcessKind {
    #[serde(rename = "gh")]
    Gh,
    #[serde(rename = "git")]
    Git,
    #[serde(rename = "docker")]
    Docker,
    #[serde(rename = "agent")]
    AgentBootstrap,
    #[serde(rename = "runner")]
    IndexRunner,
}

impl ProcessKind {
    pub const ALL: &'static [ProcessKind] = &[
        ProcessKind::Gh,
        ProcessKind::Git,
        ProcessKind::Docker,
        ProcessKind::AgentBootstrap,
        ProcessKind::IndexRunner,
    ];

    /// Lower-case stable identifier used on the wire (socket payloads,
    /// `tracing` field values, and Logs window chip labels).
    pub fn as_str(self) -> &'static str {
        match self {
            ProcessKind::Gh => "gh",
            ProcessKind::Git => "git",
            ProcessKind::Docker => "docker",
            ProcessKind::AgentBootstrap => "agent",
            ProcessKind::IndexRunner => "runner",
        }
    }
}

impl fmt::Display for ProcessKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProcessKind {
    type Err = ParseProcessKindError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "gh" => Ok(ProcessKind::Gh),
            "git" => Ok(ProcessKind::Git),
            "docker" => Ok(ProcessKind::Docker),
            "agent" | "agent_bootstrap" => Ok(ProcessKind::AgentBootstrap),
            "runner" | "index_runner" => Ok(ProcessKind::IndexRunner),
            other => Err(ParseProcessKindError {
                value: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseProcessKindError {
    pub value: String,
}

impl fmt::Display for ParseProcessKindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown ProcessKind: {}", self.value)
    }
}

impl std::error::Error for ParseProcessKindError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_kinds_roundtrip_through_str() {
        for kind in ProcessKind::ALL {
            let s = kind.as_str();
            let parsed: ProcessKind = s.parse().unwrap();
            assert_eq!(parsed, *kind);
        }
    }

    #[test]
    fn agent_bootstrap_accepts_both_aliases() {
        assert_eq!(
            "agent".parse::<ProcessKind>().unwrap(),
            ProcessKind::AgentBootstrap
        );
        assert_eq!(
            "agent_bootstrap".parse::<ProcessKind>().unwrap(),
            ProcessKind::AgentBootstrap
        );
    }

    #[test]
    fn index_runner_accepts_both_aliases() {
        assert_eq!(
            "runner".parse::<ProcessKind>().unwrap(),
            ProcessKind::IndexRunner
        );
        assert_eq!(
            "index_runner".parse::<ProcessKind>().unwrap(),
            ProcessKind::IndexRunner
        );
    }

    #[test]
    fn unknown_kind_returns_error() {
        let err = "unknown".parse::<ProcessKind>().unwrap_err();
        assert_eq!(err.value, "unknown");
    }

    #[test]
    fn serde_matches_wire_string() {
        assert_eq!(
            serde_json::to_string(&ProcessKind::AgentBootstrap).unwrap(),
            "\"agent\""
        );
        assert_eq!(
            serde_json::to_string(&ProcessKind::IndexRunner).unwrap(),
            "\"runner\""
        );
        let kind: ProcessKind = serde_json::from_str("\"gh\"").unwrap();
        assert_eq!(kind, ProcessKind::Gh);
    }
}
