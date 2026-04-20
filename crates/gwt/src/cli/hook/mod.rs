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

/// JSON shape a block hook writes to stdout when it vetoes a tool call.
///
/// Uses the Claude Code PreToolUse `hookSpecificOutput` contract so that
/// `permissionDecisionReason` is the single visible field. The legacy
/// `{"decision":"block","reason":"...","stopReason":"..."}` shape is
/// deliberately not emitted: `stopReason` is a Stop/SubagentStop-only
/// field and was silently dropped on PreToolUse, so only the short
/// summary ever reached the user.
///
/// `reason` and `stop_reason` are kept as `#[serde(skip)]` internal fields
/// so tests and call sites can still inspect the short summary and the
/// detailed guidance independently, while the wire format emits only the
/// merged `permissionDecisionReason`.
#[derive(Debug, Clone, Serialize)]
pub struct BlockDecision {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
    #[serde(skip)]
    pub reason: String,
    #[serde(skip)]
    pub stop_reason: String,
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

impl BlockDecision {
    pub fn new(reason: impl Into<String>, stop_reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let stop_reason = stop_reason.into();
        let permission_decision_reason = if stop_reason.is_empty() {
            reason.clone()
        } else if reason.is_empty() {
            stop_reason.clone()
        } else {
            format!("{reason}\n\n{stop_reason}")
        };
        Self {
            hook_specific_output: HookSpecificOutput {
                hook_event_name: "PreToolUse",
                permission_decision: "deny",
                permission_decision_reason,
            },
            reason,
            stop_reason,
        }
    }

    /// The merged text that Claude Code / Codex actually show to the
    /// LLM and user when the tool call is denied.
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
    #[error("unknown hook: {0}")]
    UnknownHook(String),
    #[error("missing environment variable: {0}")]
    MissingEnv(&'static str),
    #[error("invalid hook event: {0}")]
    InvalidEvent(String),
}
