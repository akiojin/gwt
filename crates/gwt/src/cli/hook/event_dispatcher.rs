//! Event-level hook dispatcher.
//!
//! Managed Claude/Codex hook configs call this once per hook event. The
//! dispatcher preserves the previous per-handler ordering while keeping a
//! single stdout envelope for runtimes that require hook output to be one
//! valid JSON document.

use std::path::Path;

use super::{
    board_reminder, skill_build_spec_stop_check, skill_discussion_stop_check,
    skill_plan_spec_stop_check, workflow_policy, HookError, HookOutput,
};

pub fn handle_with_input(
    event: &str,
    input: &str,
    worktree_root: &Path,
    current_session: Option<&str>,
) -> Result<HookOutput, HookError> {
    match event {
        "SessionStart" => handle_session_start(event, input),
        "UserPromptSubmit" => handle_user_prompt_submit(event, input),
        "PreToolUse" => handle_pre_tool_use(event, input),
        "PostToolUse" => handle_post_tool_use(event, input),
        "Stop" => handle_stop(event, input, worktree_root, current_session),
        other => Err(HookError::InvalidEvent(other.to_string())),
    }
}

fn handle_session_start(event: &str, input: &str) -> Result<HookOutput, HookError> {
    crate::daemon_runtime::handle_runtime_state(event, input)?;
    crate::daemon_runtime::handle_forward(input)?;
    crate::daemon_runtime::handle_coordination_event(event, input)?;
    board_reminder::handle_with_input(event, input)
}

fn handle_user_prompt_submit(event: &str, input: &str) -> Result<HookOutput, HookError> {
    crate::daemon_runtime::handle_runtime_state(event, input)?;
    crate::daemon_runtime::handle_forward(input)?;
    board_reminder::handle_with_input(event, input)
}

fn handle_pre_tool_use(event: &str, input: &str) -> Result<HookOutput, HookError> {
    crate::daemon_runtime::handle_runtime_state(event, input)?;
    crate::daemon_runtime::handle_forward(input)?;
    workflow_policy::handle_with_input(input)
}

fn handle_post_tool_use(event: &str, input: &str) -> Result<HookOutput, HookError> {
    crate::daemon_runtime::handle_runtime_state(event, input)?;
    crate::daemon_runtime::handle_forward(input)?;
    Ok(HookOutput::Silent)
}

fn handle_stop(
    event: &str,
    input: &str,
    worktree_root: &Path,
    current_session: Option<&str>,
) -> Result<HookOutput, HookError> {
    crate::daemon_runtime::handle_runtime_state(event, input)?;
    crate::daemon_runtime::handle_forward(input)?;
    crate::daemon_runtime::handle_coordination_event(event, input)?;

    let reminder = board_reminder::handle_with_input(event, input)?;
    for output in [
        skill_discussion_stop_check::handle_with_input(worktree_root, input),
        skill_plan_spec_stop_check::handle_with_input(worktree_root, input, current_session),
        skill_build_spec_stop_check::handle_with_input(worktree_root, input, current_session),
    ] {
        if matches!(output, HookOutput::StopBlock { .. }) {
            return Ok(output);
        }
    }

    Ok(reminder)
}
