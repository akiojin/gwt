//! Launch results shared by GUI and project runtime owners.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct ProcessLaunch {
    pub initial_prompt_file: Option<Arc<tempfile::TempPath>>,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub remove_env: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// SPEC #1921 Phase 86 (#3813): resource policy applied to the PTY tree
    /// through the start gate before the target runs. `None` keeps the direct
    /// spawn route (Shell panes, or isolation disabled).
    pub resource_policy: Option<gwt_terminal::pty::ProcessPolicy>,
}

fn private_launch_env_key(key: &str) -> bool {
    matches!(
        key,
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV
            | gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV
            | gwt_agent::GWT_SESSION_ID_ENV
            | gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV
    )
}

fn private_launch_env_assignment(argument: &str) -> Option<&str> {
    let (key, _) = argument.split_once('=')?;
    private_launch_env_key(key).then_some(key)
}

impl std::fmt::Debug for ProcessLaunch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted_env = self
            .env
            .iter()
            .map(|(key, value)| {
                (
                    key.as_str(),
                    if private_launch_env_key(key) {
                        "<redacted>"
                    } else {
                        value.as_str()
                    },
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let redacted_args = self
            .args
            .iter()
            .map(|argument| {
                private_launch_env_assignment(argument)
                    .map(|key| format!("{key}=<redacted>"))
                    .unwrap_or_else(|| argument.clone())
            })
            .collect::<Vec<_>>();
        formatter
            .debug_struct("ProcessLaunch")
            .field("command", &self.command)
            .field("args", &redacted_args)
            .field("env", &redacted_env)
            .field("remove_env", &self.remove_env)
            .field("cwd", &self.cwd)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct AgentLaunchRuntimeContext {
    pub agent_project_root: String,
    pub predecessor_session_id: Option<String>,
    pub expected_execution_identity: Option<gwt_agent::SessionExecutionIdentity>,
    pub active_launch_handshake: Option<gwt_agent::SessionActiveLaunchHandshake>,
}

impl From<String> for AgentLaunchRuntimeContext {
    fn from(agent_project_root: String) -> Self {
        Self {
            agent_project_root,
            predecessor_session_id: None,
            expected_execution_identity: None,
            active_launch_handshake: None,
        }
    }
}

/// (process launch, session id, branch name, display name, worktree path,
/// agent id, linked issue number, base branch, runtime target, session mode,
/// prepared-execution flag, runtime context)
pub type AgentLaunchCompletion = (
    ProcessLaunch,
    String,
    String,
    String,
    PathBuf,
    gwt_agent::AgentId,
    Option<u64>,
    Option<String>,
    gwt_agent::LaunchRuntimeTarget,
    gwt_agent::SessionMode,
    bool,
    AgentLaunchRuntimeContext,
);

/// Preserve the authority decision through launch workers and caller replies.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentLaunchError {
    pub detail: String,
    pub error_code: &'static str,
    pub retryable: bool,
}

impl AgentLaunchError {
    pub fn active_conflict(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            error_code: "active_launch_conflict",
            retryable: true,
        }
    }

    pub fn authority_rejected(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            error_code: "active_launch_authority_rejected",
            retryable: false,
        }
    }
}

impl From<String> for AgentLaunchError {
    fn from(detail: String) -> Self {
        Self {
            detail,
            error_code: "launch_failed",
            retryable: true,
        }
    }
}

impl From<&str> for AgentLaunchError {
    fn from(detail: &str) -> Self {
        Self::from(detail.to_string())
    }
}

impl std::fmt::Display for AgentLaunchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

pub type AgentLaunchResult = Result<AgentLaunchCompletion, AgentLaunchError>;
