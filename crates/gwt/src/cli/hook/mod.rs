//! Shared types for the `gwtd hook ...` CLI surface (SPEC #1942).
//!
//! This module defines the canonical vocabulary for hook dispatch:
//!
//! - [`HookKind`] — the enumerated hook name set.
//! - [`HookEvent`] — the stdin JSON payload Claude Code / Codex send.
//! - [`HookOutput`] — the stdout JSON envelope hooks write when they need
//!   to deny a tool call, inject context, or show a system message.
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
pub mod envelope;
pub mod event_dispatcher;
pub mod forward;
pub mod runtime_state;
pub mod segments;
pub mod skill_build_spec_stop_check;
pub mod skill_discussion_stop_check;
pub mod skill_plan_spec_stop_check;
pub mod workflow_policy;
pub mod worktree;

use std::io::{self, Read};

use serde::Deserialize;

pub use envelope::{HookOutput, IntentBoundaryEvent};

/// Every hook name exposed via `gwtd hook <name>`.
///
/// Adding a new variant requires updating [`HookKind::from_name`] and
/// the dispatch match in `cli::run_hook`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    Event,
    RuntimeState,
    CoordinationEvent,
    BoardReminder,
    BlockBashPolicy,
    WorkflowPolicy,
    Forward,
    SkillDiscussionStopCheck,
    SkillPlanSpecStopCheck,
    SkillBuildSpecStopCheck,
}

impl HookKind {
    /// Parse a hook name exactly as it appears on the command line.
    ///
    /// Returns `None` for any unknown string; the caller is responsible
    /// for converting that into a [`HookError::UnknownHook`].
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "event" => Some(Self::Event),
            "runtime-state" => Some(Self::RuntimeState),
            "coordination-event" => Some(Self::CoordinationEvent),
            "board-reminder" => Some(Self::BoardReminder),
            "block-bash-policy" => Some(Self::BlockBashPolicy),
            "workflow-policy" => Some(Self::WorkflowPolicy),
            "forward" => Some(Self::Forward),
            "skill-discussion-stop-check" => Some(Self::SkillDiscussionStopCheck),
            "skill-plan-spec-stop-check" => Some(Self::SkillPlanSpecStopCheck),
            "skill-build-spec-stop-check" => Some(Self::SkillBuildSpecStopCheck),
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
