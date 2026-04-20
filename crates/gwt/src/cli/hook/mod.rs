//! Shared types for the `gwt hook ...` CLI surface (SPEC #1942).
//!
//! This module defines the canonical vocabulary for hook dispatch:
//!
//! - [`HookKind`] — the enumerated hook name set.
//! - [`HookEvent`] — the stdin JSON payload Claude Code / Codex send.
//! - [`BlockDecision`] — the stdout JSON payload a block hook writes
//!   when it refuses to let the tool call proceed.
//! - [`HookError`] — the error enum every hook handler returns.
//!
//! Individual hook handlers (runtime-state, block-*, forward) will live in
//! sibling files and consume these types.

pub mod block_bash_policy;
pub mod block_cd_command;
pub mod block_file_ops;
pub mod block_git_branch_ops;
pub mod block_git_dir_override;
pub mod board_reminder;
pub mod coordination_event;
pub mod forward;
pub mod runtime_state;
pub mod segments;
pub mod workflow_policy;
pub mod worktree;

use std::io::{self, Read};

use serde::{Deserialize, Serialize};

/// Every hook name exposed via `gwt hook <name>`.
///
/// Adding a new variant requires updating [`HookKind::from_name`] and
/// the dispatch match in `cli::run_hook`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    RuntimeState,
    CoordinationEvent,
    BoardReminder,
    BlockBashPolicy,
    WorkflowPolicy,
    Forward,
}

impl HookKind {
    /// Parse a hook name exactly as it appears on the command line.
    ///
    /// Returns `None` for any unknown string; the caller is responsible
    /// for converting that into a [`HookError::UnknownHook`].
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "runtime-state" => Some(Self::RuntimeState),
            "coordination-event" => Some(Self::CoordinationEvent),
            "board-reminder" => Some(Self::BoardReminder),
            "block-bash-policy" => Some(Self::BlockBashPolicy),
            "workflow-policy" => Some(Self::WorkflowPolicy),
            "forward" => Some(Self::Forward),
            _ => None,
        }
    }
}

/// The Claude Code / Codex hook event payload. Every field is optional
/// so that schema extensions on the Claude Code side do not break our
/// parser.
#[derive(Debug, Clone, Deserialize)]
pub struct HookEvent {
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub cwd: Option<String>,
}

impl HookEvent {
    pub fn read_from_str(input: &str) -> Result<Option<Self>, HookError> {
        if input.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(input)?))
    }

    /// Read a single JSON payload from stdin.
    ///
    /// - Empty stdin → `Ok(None)` (treated as a no-op by the caller).
    /// - Malformed JSON → `Err(HookError::Json)`.
    pub fn read_from_stdin() -> Result<Option<Self>, HookError> {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        Self::read_from_str(&buf)
    }

    /// Convenience accessor for `tool_input.command` (Bash tool payloads).
    pub fn command(&self) -> Option<&str> {
        self.tool_input.as_ref()?.get("command")?.as_str()
    }
}

/// PreToolUse `hookSpecificOutput` denial payload.
///
/// The wire format exposes only `permissionDecisionReason` because the
/// legacy top-level `stopReason` is ignored on PreToolUse and only the
/// short `reason` was reaching the user before this was introduced.
#[derive(Debug, Clone, Serialize)]
pub struct BlockDecision {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
    #[serde(skip)]
    summary: String,
    #[serde(skip)]
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "permissionDecision")]
    permission_decision: &'static str,
    #[serde(rename = "permissionDecisionReason")]
    permission_decision_reason: String,
}

impl HookSpecificOutput {
    const EVENT_NAME: &'static str = "PreToolUse";
    const DECISION_DENY: &'static str = "deny";
}

impl BlockDecision {
    pub fn new(summary: impl Into<String>, detail: impl Into<String>) -> Self {
        let summary = summary.into();
        let detail = detail.into();
        let permission_decision_reason = match (summary.is_empty(), detail.is_empty()) {
            (true, _) => detail.clone(),
            (_, true) => summary.clone(),
            _ => format!("{summary}\n\n{detail}"),
        };
        Self {
            hook_specific_output: HookSpecificOutput {
                hook_event_name: HookSpecificOutput::EVENT_NAME,
                permission_decision: HookSpecificOutput::DECISION_DENY,
                permission_decision_reason,
            },
            summary,
            detail,
        }
    }

    /// Short headline. Kept separate from `detail` so tests can assert the
    /// rule name without scanning the merged reason.
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Full guidance (alternatives, blocked command, etc.).
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// The merged text Claude Code / Codex surface to the LLM and user.
    pub fn permission_decision_reason(&self) -> &str {
        &self.hook_specific_output.permission_decision_reason
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid hook event json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("coordination error: {0}")]
    Coordination(#[from] gwt_core::GwtError),
    #[error("unknown hook: {0}")]
    UnknownHook(String),
    #[error("missing environment variable: {0}")]
    MissingEnv(&'static str),
    #[error("invalid hook event: {0}")]
    InvalidEvent(String),
}
