//! Event-level hook dispatcher.
//!
//! Managed Claude/Codex hook configs call this once per hook event. The
//! dispatcher preserves the previous per-handler ordering while keeping a
//! single stdout envelope for runtimes that require hook output to be one
//! valid JSON document.

use std::{path::Path, time::Instant};

use super::{
    board_reminder, diagnostics, skill_build_spec_stop_check, skill_discussion_stop_check,
    skill_plan_spec_stop_check, skill_register_spec_stop_check, workflow_policy,
    workspace_identity, HookError, HookOutput, IntentBoundaryEvent,
};
use crate::discussion_resume::{load_pending_goal, PendingDiscussionGoal};

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
    run_step(event, "runtime-state", || {
        crate::daemon_runtime::handle_runtime_state(event, input)
    })?;
    run_step(event, "forward", || {
        crate::daemon_runtime::handle_forward(input)
    })?;
    // SPEC-2359: register the running session into `projection.agents[]`
    // before any further coordination CLI runs so `gwtd workspace update
    // --title-summary` is not silently dropped. Fail-open: registration
    // errors must not abort the agent boot.
    run_value(event, "workspace-registration", || {
        if let Err(error) = workspace_identity::handle_session_start() {
            tracing::warn!(?error, "workspace-registration hook step failed");
        }
    });
    run_step(event, "coordination-event", || {
        crate::daemon_runtime::handle_coordination_event(event, input)
    })?;
    let output = run_step(event, "board-reminder", || {
        board_reminder::handle_with_input(event, input)
    })?;
    let worktree_root = crate::cli::hook::worktree::detect_worktree_root();
    let pending_goal = run_value(event, "discussion-goal-start", || {
        load_pending_goal(&worktree_root).ok().flatten()
    });
    Ok(append_pending_discussion_goal_context(
        output,
        IntentBoundaryEvent::UserPromptSubmit,
        pending_goal,
    ))
}

fn handle_user_prompt_submit(event: &str, input: &str) -> Result<HookOutput, HookError> {
    run_step(event, "runtime-state", || {
        crate::daemon_runtime::handle_runtime_state(event, input)
    })?;
    run_step(event, "forward", || {
        crate::daemon_runtime::handle_forward(input)
    })?;
    // SPEC-2359 Phase W-11 (US-58): the workspace-identity step no longer
    // derives a title from the prompt; it only performs the Phase W-10
    // canonical Project State split repair. Fail-open so a repair error does
    // not abort prompt handling.
    run_value(event, "workspace-identity", || {
        if let Err(error) = workspace_identity::handle_user_prompt_submit(input) {
            tracing::warn!(?error, "workspace-identity hook step failed");
        }
    });
    run_step(event, "board-reminder", || {
        board_reminder::handle_with_input(event, input)
    })
}

fn handle_pre_tool_use(event: &str, input: &str) -> Result<HookOutput, HookError> {
    run_step(event, "runtime-state", || {
        crate::daemon_runtime::handle_runtime_state(event, input)
    })?;
    run_step(event, "forward", || {
        crate::daemon_runtime::handle_forward(input)
    })?;
    run_step(event, "workflow-policy", || {
        workflow_policy::handle_with_input(input)
    })
}

fn handle_post_tool_use(event: &str, input: &str) -> Result<HookOutput, HookError> {
    run_step(event, "runtime-state", || {
        crate::daemon_runtime::handle_runtime_state(event, input)
    })?;
    run_step(event, "forward", || {
        crate::daemon_runtime::handle_forward(input)
    })?;
    Ok(HookOutput::Silent)
}

fn handle_stop(
    event: &str,
    input: &str,
    worktree_root: &Path,
    current_session: Option<&str>,
) -> Result<HookOutput, HookError> {
    run_step(event, "runtime-state", || {
        crate::daemon_runtime::handle_runtime_state(event, input)
    })?;
    run_step(event, "forward", || {
        crate::daemon_runtime::handle_forward(input)
    })?;
    run_step(event, "coordination-event", || {
        crate::daemon_runtime::handle_coordination_event(event, input)
    })?;

    let reminder = run_step(event, "board-reminder", || {
        board_reminder::handle_with_input(event, input)
    })?;
    for output in [
        run_value(event, "skill-discussion-stop-check", || {
            skill_discussion_stop_check::handle_with_input(worktree_root, input)
        }),
        run_value(event, "skill-plan-spec-stop-check", || {
            skill_plan_spec_stop_check::handle_with_input(worktree_root, input, current_session)
        }),
        run_value(event, "skill-build-spec-stop-check", || {
            skill_build_spec_stop_check::handle_with_input(worktree_root, input, current_session)
        }),
        run_value(event, "skill-register-spec-stop-check", || {
            skill_register_spec_stop_check::handle_with_input(worktree_root, input, current_session)
        }),
    ] {
        if matches!(output, HookOutput::StopBlock { .. }) {
            run_step(event, "blocked-stop-runtime-state", || {
                crate::daemon_runtime::handle_blocked_stop_runtime_state(input)
            })?;
            return Ok(output);
        }
    }

    run_step(event, "completed-stop", || {
        super::runtime_state::record_completed_stop_from_env()
    })?;

    Ok(reminder)
}

fn run_step<T>(
    event: &str,
    handler: &str,
    operation: impl FnOnce() -> Result<T, HookError>,
) -> Result<T, HookError> {
    let started = Instant::now();
    let result = operation();
    diagnostics::record_handler_duration(
        event,
        handler,
        started.elapsed(),
        if result.is_ok() { "ok" } else { "error" },
    );
    result
}

fn run_value<T>(event: &str, handler: &str, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let value = operation();
    diagnostics::record_handler_duration(event, handler, started.elapsed(), "ok");
    value
}

fn append_pending_discussion_goal_context(
    output: HookOutput,
    event: IntentBoundaryEvent,
    pending_goal: Option<PendingDiscussionGoal>,
) -> HookOutput {
    let Some(goal) = pending_goal else {
        return output;
    };
    let text = pending_discussion_goal_context(&goal);
    match output {
        HookOutput::HookSpecificAdditionalContext {
            event: existing_event,
            text: existing,
        } => HookOutput::hook_specific_additional_context(
            existing_event,
            format!("{existing}\n\n{text}"),
        ),
        HookOutput::Silent => HookOutput::hook_specific_additional_context(event, text),
        other => other,
    }
}

fn pending_discussion_goal_context(goal: &PendingDiscussionGoal) -> String {
    format!(
        "pending gwt-discussion Goal Start\n\
\n\
Proposal: {label} - {title}\n\
Goal condition: {condition}\n\
\n\
If the latest user prompt approves this Action Bundle and asks work to continue autonomously, start the runtime goal before doing implementation or verification work:\n\
- Codex: call `create_goal` with the Goal condition above as the objective.\n\
- Claude Code: resolve `GWT_BIN`, then run `gwtd pane send --text '/goal <condition>'` using the Goal condition above.\n\
\n\
After a successful start, run `gwtd discuss goal-started --proposal \"{label}\"`. If the user rejects or revises the Action Bundle, run `gwtd discuss goal-skipped --proposal \"{label}\" --reason '<reason>'`. If goal start fails, run `gwtd discuss goal-failed --proposal \"{label}\" --reason '<reason>'` and show a manual `/goal <condition>` line to the user.",
        label = goal.proposal_label,
        title = goal.proposal_title,
        condition = goal.condition,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discussion_resume::PendingDiscussionGoal;

    #[test]
    fn pending_discussion_goal_context_is_appended_to_user_prompt_submit_output() {
        let output = append_pending_discussion_goal_context(
            HookOutput::hook_specific_additional_context(
                IntentBoundaryEvent::UserPromptSubmit,
                "Board reminder",
            ),
            IntentBoundaryEvent::UserPromptSubmit,
            Some(PendingDiscussionGoal {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Goal handoff".to_string(),
                condition: "verification handoff ready with User Verification Result recorded"
                    .to_string(),
            }),
        );

        let HookOutput::HookSpecificAdditionalContext { text, .. } = output else {
            panic!("expected additional context");
        };
        assert!(text.contains("Board reminder"), "{text}");
        assert!(text.contains("pending gwt-discussion Goal Start"), "{text}");
        assert!(text.contains("Proposal A - Goal handoff"), "{text}");
        assert!(text.contains("create_goal"), "{text}");
        assert!(text.contains("gwtd pane send"), "{text}");
        assert!(text.contains("verification handoff ready"), "{text}");
    }
}
