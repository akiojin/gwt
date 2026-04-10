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
pub mod forward;
pub mod runtime_state;
pub mod segments;
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
    BlockBashPolicy,
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
            "block-bash-policy" => Some(Self::BlockBashPolicy),
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
    /// Read a single JSON payload from stdin.
    ///
    /// - Empty stdin → `Ok(None)` (treated as a no-op by the caller).
    /// - Malformed JSON → `Err(HookError::Json)`.
    pub fn read_from_stdin() -> Result<Option<Self>, HookError> {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        if buf.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&buf)?))
    }

    /// Convenience accessor for `tool_input.command` (Bash tool payloads).
    pub fn command(&self) -> Option<&str> {
        self.tool_input.as_ref()?.get("command")?.as_str()
    }
}

/// JSON shape a block hook writes to stdout when it vetoes a tool call.
#[derive(Debug, Clone, Serialize)]
pub struct BlockDecision {
    pub decision: &'static str,
    pub reason: String,
    #[serde(rename = "stopReason")]
    pub stop_reason: String,
}

impl BlockDecision {
    pub fn new(reason: impl Into<String>, stop_reason: impl Into<String>) -> Self {
        Self {
            decision: "block",
            reason: reason.into(),
            stop_reason: stop_reason.into(),
        }
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
