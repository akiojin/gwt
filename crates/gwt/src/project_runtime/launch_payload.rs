//! Launch results shared by GUI and project runtime owners.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Immutable launch inputs from the Rust GUI. No Prepared binding or claim
/// can be carried here; the fixed-project owner creates those locally.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerLaunchMetadata {
    pub inherited_agent_name: Option<String>,
    pub workspace_resume_context: Option<WorkspaceResumeContext>,
    pub launch_feedback_context: Option<LaunchFeedbackContext>,
    pub manual_successor: Option<ManualSuccessorChoice>,
    #[serde(default)]
    pub monitor_requested_tier: Option<u8>,
    #[serde(default)]
    pub deliver_pending_autonomous_answer: bool,
}

/// Expected identity narrows the user's manual choice. It grants no authority
/// over a predecessor; the owner revalidates its durable and physical proof.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualSuccessorChoice {
    pub operation_id: String,
    pub expected_binding: gwt_agent::ExecutionBindingIdentity,
    pub expected_predecessor: Option<Box<gwt_agent::SessionExecutionIdentity>>,
    pub expected_runtime: Option<gwt_agent::ManualLaunchRuntimeEvidence>,
    pub predecessor_kind: gwt_agent::ManualLaunchSuccessorPredecessor,
}

impl ManualSuccessorChoice {
    pub fn from_intent(intent: &gwt_agent::ExecutionLaunchIntent) -> Result<Option<Self>, String> {
        use gwt_agent::ExecutionLaunchIntent;
        match intent {
            ExecutionLaunchIntent::Automatic => Ok(None),
            ExecutionLaunchIntent::ManualSuccessor {
                operation_id,
                expected_binding,
                expected_predecessor,
                expected_runtime,
                predecessor_kind,
            } => Ok(Some(Self {
                operation_id: operation_id.clone(),
                expected_binding: expected_binding.clone(),
                expected_predecessor: expected_predecessor.clone(),
                expected_runtime: *expected_runtime,
                predecessor_kind: *predecessor_kind,
            })),
            ExecutionLaunchIntent::PreparedContinuation(_)
            | ExecutionLaunchIntent::PreparedManualSuccessor(_) => {
                Err("Prepared launch authority must remain in the project owner".into())
            }
        }
    }

    pub(crate) fn into_intent(self) -> gwt_agent::ExecutionLaunchIntent {
        gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
            operation_id: self.operation_id,
            expected_binding: self.expected_binding,
            expected_predecessor: self.expected_predecessor,
            expected_runtime: self.expected_runtime,
            predecessor_kind: self.predecessor_kind,
        }
    }
}

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

#[derive(Debug, Clone)]
pub struct ActiveAgentSession {
    pub window_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub branch_name: String,
    pub display_name: String,
    pub worktree_path: PathBuf,
    pub agent_project_root: String,
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceResumeContext {
    pub title: Option<String>,
    pub owner: Option<String>,
    pub summary: Option<String>,
    pub next_action: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PendingContinueWork {
    pub client_id: String,
    pub operation_id: String,
    pub work_id: String,
    pub project_root: PathBuf,
    pub worktree_path: PathBuf,
    pub owner: crate::cli::execution_state::ExecutionOwnerKey,
    pub work_branch: String,
    pub work_agent_id: gwt_agent::AgentId,
    pub work_agent_session_id: Option<String>,
    pub execution: PendingContinueWorkExecution,
    pub binding: gwt_agent::SessionExecutionBinding,
    pub readiness_nonce: String,
    pub outcome: crate::ContinueWorkOutcomeKind,
    pub resume_context: WorkspaceResumeContext,
    pub predecessor_session_id: String,
    pub predecessor_binding: gwt_agent::ExecutionBindingIdentity,
}

#[derive(Debug, Clone)]
pub enum PendingContinueWorkExecution {
    Successor(crate::cli::execution_state::SuccessorRequest),
    #[allow(dead_code)] // Retained for reconciliation of legacy in-memory fixtures.
    Takeover(crate::cli::execution_state::GenerationTakeoverRequest),
}

/// Explicit linked-owner launch that is waiting for an authenticated
/// SessionStart before replacing an integrity-valid Blocked generation.
#[derive(Debug, Clone)]
pub struct PendingFreshExecutionLaunch {
    pub operation_id: String,
    pub project_root: PathBuf,
    pub worktree_path: PathBuf,
    pub owner: crate::cli::execution_state::ExecutionOwnerKey,
    pub request: crate::cli::execution_state::SuccessorRequest,
    pub binding: gwt_agent::SessionExecutionBinding,
    pub session_identity: gwt_agent::SessionExecutionIdentity,
    pub readiness_nonce: String,
    pub predecessor_binding: gwt_agent::ExecutionBindingIdentity,
    pub base_branch: Option<String>,
    pub linked_issue_number: Option<u64>,
    pub resume_context: Option<WorkspaceResumeContext>,
    pub launch_feedback_context: Option<LaunchFeedbackContext>,
}

impl WorkspaceResumeContext {
    pub fn purpose_title(&self) -> Option<String> {
        self.title
            .as_deref()
            .or(self.summary.as_deref())
            .or(self.owner.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LaunchFeedbackContext {
    pub client_id: String,
    pub title: String,
    pub issue_monitor_issue_number: Option<u64>,
    pub issue_monitor_delivery_id: Option<String>,
    pub issue_monitor_project_root: Option<PathBuf>,
    pub issue_monitor_session_mode: Option<gwt_agent::SessionMode>,
    /// Write-ahead answered handoff carried by a native Resume. Only the
    /// provider's UserPromptSubmit receipt may mark it delivered.
    #[serde(skip)]
    pub issue_monitor_autonomous_handoff: Option<crate::AutonomousHandoffDeliveryAttempt>,
    /// Set immediately before the provider process spawn call. Failures while
    /// false are definitely pre-submit and keep the bounded retry ladder;
    /// failures while true have an ambiguous provider outcome.
    #[serde(skip)]
    pub issue_monitor_autonomous_submit_started: bool,
    /// SPEC #3200 Option A / Issue #4041: the independent review agent is
    /// subordinate to the implementing launch. Its window must never be
    /// ACKed as the Issue's launch: that rebinds `launched_issues` to the
    /// review window, drops the implementation claim, and takes an active
    /// slot while the implementation window is still running.
    pub issue_monitor_review_dispatch: bool,
}

impl LaunchFeedbackContext {
    /// The Issue whose Monitor launch this window materializes. `None` for a
    /// review dispatch, whose window observes the Issue without owning its
    /// launch binding (Issue #4041).
    pub fn issue_monitor_launch_binding_issue_number(&self) -> Option<u64> {
        if self.issue_monitor_review_dispatch {
            None
        } else {
            self.issue_monitor_issue_number
        }
    }
}
