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
pub mod diagnostics;
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

// ---------------------------------------------------------------------------
// SPEC-1942 Phase 4: in-process hook dispatch + daemon-front-door bootstrap
// (moved here from cli.rs as part of family helper migration). All entry
// points stay reachable via super::hook::* from cli.rs and env.rs.
// ---------------------------------------------------------------------------

use gwt_github::{client::ApiError, SpecOpsError};

use crate::cli::CliEnv;

fn io_as_api_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

pub fn run_hook<E: CliEnv>(env: &mut E, name: &str, rest: &[String]) -> Result<i32, SpecOpsError> {
    run_daemon_hook(env, name, rest)
}

#[cfg(test)]
pub(crate) fn daemon_hook_argv(name: &str, rest: &[String]) -> Vec<String> {
    let mut argv = vec![
        "gwtd".to_string(),
        "__internal".to_string(),
        "daemon-hook".to_string(),
        name.to_string(),
    ];
    argv.extend(rest.iter().cloned());
    argv
}

#[cfg(test)]
pub(crate) fn write_internal_command_output<E: CliEnv>(
    env: &mut E,
    output: crate::cli::env::InternalCommandOutput,
) -> Result<i32, SpecOpsError> {
    env.stdout()
        .write_all(&output.stdout)
        .map_err(io_as_api_error)?;
    env.stdout().flush().map_err(io_as_api_error)?;
    env.stderr()
        .write_all(&output.stderr)
        .map_err(io_as_api_error)?;
    env.stderr().flush().map_err(io_as_api_error)?;
    Ok(output.status)
}

pub fn prepare_daemon_front_door_for_path(project_root: &std::path::Path) -> Result<(), String> {
    if !project_root.exists() {
        return Ok(());
    }

    refresh_managed_assets_for_hook_front_door(project_root)?;

    crate::index_worker::bootstrap_project_index_for_path(project_root)?;

    let scope = gwt_core::daemon::RuntimeScope::from_project_root(
        project_root,
        gwt_core::daemon::RuntimeTarget::Host,
    )
    .map_err(|err| err.to_string())?;
    let gwt_home = gwt_core::paths::gwt_home();
    let action = gwt_core::daemon::resolve_bootstrap_action(
        &gwt_home,
        &scope,
        gwt_core::daemon::DAEMON_PROTOCOL_VERSION,
        |pid| pid == std::process::id(),
    )
    .map_err(|err| err.to_string())?;

    if let gwt_core::daemon::DaemonBootstrapAction::Spawn { endpoint_path } = action {
        let endpoint = gwt_core::daemon::DaemonEndpoint::new(
            scope,
            std::process::id(),
            "internal://gwt-front-door".to_string(),
            uuid::Uuid::new_v4().to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        gwt_core::daemon::persist_endpoint(&endpoint_path, &endpoint)
            .map_err(|err| err.to_string())?;
    }

    Ok(())
}

pub(crate) fn refresh_managed_assets_for_hook_front_door(
    project_root: &std::path::Path,
) -> Result<(), String> {
    if gwt_git::Repository::discover(project_root).is_err() {
        return Ok(());
    }
    crate::managed_assets::refresh_managed_gwt_assets_for_worktree(project_root)
        .map_err(|err| err.to_string())
}

pub fn run_daemon_hook<E: CliEnv>(
    env: &mut E,
    name: &str,
    rest: &[String],
) -> Result<i32, SpecOpsError> {
    use crate::cli::hook::{
        block_bash_policy, event_dispatcher, skill_build_spec_stop_check,
        skill_discussion_stop_check, skill_plan_spec_stop_check, workflow_policy, HookKind,
        HookOutput,
    };

    let Some(kind) = HookKind::from_name(name) else {
        let _ = writeln!(env.stderr(), "gwtd hook: unknown hook '{name}'");
        return Ok(2);
    };
    let stdin = env.read_stdin().map_err(io_as_api_error)?;

    fn emit_hook_output<E: CliEnv>(env: &mut E, output: &HookOutput) -> i32 {
        match output.serialize_to(env.stdout()) {
            Ok(()) => output.exit_code(),
            Err(err) => {
                let _ = writeln!(env.stderr(), "gwtd hook: failed to serialize output: {err}");
                1
            }
        }
    }
    fn emit_hook_error<E: CliEnv>(env: &mut E, name: &str, err: impl std::fmt::Display) -> i32 {
        let _ = writeln!(env.stderr(), "gwtd hook {name}: {err}");
        1
    }

    match kind {
        HookKind::Event => {
            let Some(event) = rest.first() else {
                let _ = writeln!(env.stderr(), "gwtd hook event: missing <event> argument");
                return Ok(2);
            };
            let cwd = env.repo_path().to_path_buf();
            let current_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok();
            match event_dispatcher::handle_with_input(
                event,
                &stdin,
                &cwd,
                current_session.as_deref(),
            ) {
                Ok(output) => Ok(emit_hook_output(env, &output)),
                Err(err) => Ok(emit_hook_error(env, name, err)),
            }
        }
        HookKind::RuntimeState => {
            let Some(event) = rest.first() else {
                let _ = writeln!(
                    env.stderr(),
                    "gwtd hook runtime-state: missing <event> argument"
                );
                return Ok(2);
            };
            match crate::daemon_runtime::handle_runtime_state(event, &stdin) {
                Ok(()) => Ok(0),
                Err(err) => Ok(emit_hook_error(env, name, err)),
            }
        }
        HookKind::CoordinationEvent => {
            let Some(event) = rest.first() else {
                let _ = writeln!(
                    env.stderr(),
                    "gwtd hook coordination-event: missing <event> argument"
                );
                return Ok(2);
            };
            match crate::daemon_runtime::handle_coordination_event(event, &stdin) {
                Ok(()) => Ok(0),
                Err(err) => Ok(emit_hook_error(env, name, err)),
            }
        }
        HookKind::BoardReminder => {
            let Some(event) = rest.first() else {
                let _ = writeln!(
                    env.stderr(),
                    "gwtd hook board-reminder: missing <event> argument"
                );
                return Ok(2);
            };
            match crate::cli::hook::board_reminder::handle_with_input(event, &stdin) {
                Ok(output) => Ok(emit_hook_output(env, &output)),
                Err(err) => Ok(emit_hook_error(env, name, err)),
            }
        }
        HookKind::BlockBashPolicy => match block_bash_policy::handle_with_input(&stdin) {
            Ok(output) => Ok(emit_hook_output(env, &output)),
            Err(err) => Ok(emit_hook_error(env, name, err)),
        },
        HookKind::WorkflowPolicy => match workflow_policy::handle_with_input(&stdin) {
            Ok(output) => Ok(emit_hook_output(env, &output)),
            Err(err) => Ok(emit_hook_error(env, name, err)),
        },
        HookKind::Forward => match crate::daemon_runtime::handle_forward(&stdin) {
            Ok(()) => Ok(0),
            Err(err) => Ok(emit_hook_error(env, name, err)),
        },
        HookKind::SkillDiscussionStopCheck => {
            let cwd = env.repo_path().to_path_buf();
            let output = skill_discussion_stop_check::handle_with_input(&cwd, &stdin);
            Ok(emit_hook_output(env, &output))
        }
        HookKind::SkillPlanSpecStopCheck => {
            let cwd = env.repo_path().to_path_buf();
            let current_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok();
            let output = skill_plan_spec_stop_check::handle_with_input(
                &cwd,
                &stdin,
                current_session.as_deref(),
            );
            Ok(emit_hook_output(env, &output))
        }
        HookKind::SkillBuildSpecStopCheck => {
            let cwd = env.repo_path().to_path_buf();
            let current_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok();
            let output = skill_build_spec_stop_check::handle_with_input(
                &cwd,
                &stdin,
                current_session.as_deref(),
            );
            Ok(emit_hook_output(env, &output))
        }
    }
}
