//! Event-level hook dispatcher.
//!
//! Managed Claude/Codex hook configs call this once per hook event. The
//! dispatcher preserves the previous per-handler ordering while keeping a
//! single stdout envelope for runtimes that require hook output to be one
//! valid JSON document.

use std::{path::Path, time::Instant};

use super::{
    board_reminder, diagnostics, execution_completion_stop_check, execution_control_stop_check,
    intake_completion_stop_check, skill_build_spec_stop_check, skill_discussion_stop_check,
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
        "SessionStart" => handle_session_start(event, input, worktree_root),
        "UserPromptSubmit" => handle_user_prompt_submit(event, input, worktree_root),
        "PreToolUse" => handle_pre_tool_use(event, input),
        "PostToolUse" => handle_post_tool_use(event, input),
        "Stop" => handle_stop(event, input, worktree_root, current_session),
        other => Err(HookError::InvalidEvent(other.to_string())),
    }
}

fn handle_session_start(
    event: &str,
    input: &str,
    worktree_root: &Path,
) -> Result<HookOutput, HookError> {
    run_step(event, "runtime-state", || {
        crate::daemon_runtime::handle_runtime_state(event, input)
    })?;
    let session_start_diagnostic = run_value(event, "session-start-session-id-diagnostic", || {
        super::runtime_state::session_start_agent_session_diagnostic(input)
    });
    run_step(event, "forward", || {
        crate::daemon_runtime::handle_forward(input)
    })?;
    // SPEC-2359: register the running session into `projection.agents[]`
    // before any further coordination CLI runs so JSON `workspace.update`
    // is not silently dropped. Fail-open: registration errors must not
    // abort the agent boot.
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
    let output = append_additional_context(
        output,
        IntentBoundaryEvent::SessionStart,
        session_start_diagnostic,
    );
    let pending_goal = run_value(event, "discussion-goal-start", || {
        load_pending_goal_for_hook_worktree(worktree_root)
    });
    Ok(append_pending_discussion_goal_context(
        output,
        IntentBoundaryEvent::SessionStart,
        pending_goal,
    ))
}

fn handle_user_prompt_submit(
    event: &str,
    input: &str,
    worktree_root: &Path,
) -> Result<HookOutput, HookError> {
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
    // SPEC-3248 P7A (FR-016): mark the intake artifact requirement dirty for
    // curation/producing prompts. Fail-open state writer.
    run_value(event, "intake-outcome-required-since", || {
        intake_completion_stop_check::handle_user_prompt_submit(worktree_root, input);
    });
    let output = run_step(event, "board-reminder", || {
        board_reminder::handle_with_input(event, input)
    })?;
    let pending_goal = run_value(event, "discussion-goal-start", || {
        load_pending_goal_for_hook_worktree(worktree_root)
    });
    Ok(append_pending_discussion_goal_context(
        output,
        IntentBoundaryEvent::UserPromptSubmit,
        pending_goal,
    ))
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

/// One lazily-evaluated Stop-check in [`handle_stop`]'s chain.
type StopCheck<'a> = Box<dyn FnOnce() -> HookOutput + 'a>;

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
    // Evaluate the stop-checks lazily, one at a time: the first StopBlock
    // wins and the remaining checks must NOT run. This matters since the
    // intake completion gate (SPEC-3248 P7A) has a persistent side effect
    // (self-improvement auto-capture) that must only fire for the block the
    // agent actually sees.
    let stop_checks: [(&str, StopCheck<'_>); 7] = [
        (
            "skill-discussion-stop-check",
            Box::new(|| skill_discussion_stop_check::handle_with_input(worktree_root, input)),
        ),
        (
            "skill-plan-spec-stop-check",
            Box::new(|| {
                skill_plan_spec_stop_check::handle_with_input(worktree_root, input, current_session)
            }),
        ),
        (
            "skill-build-spec-stop-check",
            Box::new(|| {
                skill_build_spec_stop_check::handle_with_input(
                    worktree_root,
                    input,
                    current_session,
                )
            }),
        ),
        (
            "skill-register-spec-stop-check",
            Box::new(|| {
                skill_register_spec_stop_check::handle_with_input(
                    worktree_root,
                    input,
                    current_session,
                )
            }),
        ),
        // SPEC-3248 P7A (FR-014): intake completion hard gate. Runs before
        // completed-stop recording like every entry in this chain.
        (
            "intake-completion-stop-check",
            Box::new(|| {
                intake_completion_stop_check::handle_with_input(
                    worktree_root,
                    input,
                    current_session,
                )
            }),
        ),
        // SPEC-3248 P8a (T-108): launch-written Execution Control Record
        // keeps the execution session working until it settles via
        // execution.complete / execution.blocked / build.complete.
        (
            "execution-control-stop-check",
            Box::new(|| {
                execution_control_stop_check::handle_with_input(
                    worktree_root,
                    input,
                    current_session,
                )
            }),
        ),
        (
            "execution-completion-stop-check",
            Box::new(|| execution_completion_stop_check::handle_with_input(worktree_root, input)),
        ),
    ];
    for (handler, check) in stop_checks {
        let output = run_value(event, handler, check);
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

fn load_pending_goal_for_hook_worktree(worktree_root: &Path) -> Option<PendingDiscussionGoal> {
    let resolved_worktree_root = gwt_core::paths::resolve_current_worktree_root(worktree_root);
    load_pending_goal(&resolved_worktree_root).ok().flatten()
}

fn append_pending_discussion_goal_context(
    output: HookOutput,
    event: IntentBoundaryEvent,
    pending_goal: Option<PendingDiscussionGoal>,
) -> HookOutput {
    let Some(goal) = pending_goal else {
        return output;
    };
    append_additional_context(output, event, Some(pending_discussion_goal_context(&goal)))
}

fn append_additional_context(
    output: HookOutput,
    event: IntentBoundaryEvent,
    text: Option<String>,
) -> HookOutput {
    let Some(text) = text else {
        return output;
    };
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
- Claude Code: resolve `GWT_BIN`, then run JSON operation `pane.send` with `params.text:\"/goal <condition>\"` using the Goal condition above.\n\
\n\
After a successful start, run JSON operation `discuss.goal_started` with `params.proposal:\"{label}\"`. If the user rejects or revises the Action Bundle, run JSON operation `discuss.goal_skipped` with `params.proposal:\"{label}\"` and `params.reason`. If goal start fails, run JSON operation `discuss.goal_failed` with `params.proposal:\"{label}\"` and `params.reason`, then show a manual `/goal <condition>` line to the user.",
        label = goal.proposal_label,
        title = goal.proposal_title,
        condition = goal.condition,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discussion_resume::PendingDiscussionGoal;
    use gwt_agent::{AgentId, Session, GWT_SESSION_ID_ENV, GWT_SESSION_RUNTIME_PATH_ENV};
    use gwt_core::test_support::ScopedEnvVar;

    fn write_pending_goal(worktree: &Path) {
        let discussion_path = worktree.join(".gwt/discussion.md");
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(
            discussion_path,
            "## Discussion TODO\n\n\
             ### Proposal A - Goal handoff [chosen]\n\
             - Summary: Action Bundle is approved.\n\
             - Goal Condition: verification handoff ready with User Verification Result recorded\n\
             - Goal State: pending\n",
        )
        .unwrap();
    }

    fn init_git_repo(worktree: &Path) {
        let status = gwt_core::process::hidden_command("git")
            .arg("init")
            .arg("-q")
            .current_dir(worktree)
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
    }

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
        assert!(text.contains("pane.send"), "{text}");
        assert!(text.contains("verification handoff ready"), "{text}");
    }

    #[test]
    fn user_prompt_submit_appends_pending_goal_from_dispatch_worktree() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", worktree.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", worktree.path());
        let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let _runtime_path = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        write_pending_goal(worktree.path());

        let output = handle_with_input("UserPromptSubmit", "{}", worktree.path(), None)
            .expect("hook output");

        let HookOutput::HookSpecificAdditionalContext { event, text } = output else {
            panic!("expected pending goal context");
        };
        assert_eq!(event, IntentBoundaryEvent::UserPromptSubmit);
        assert!(text.contains("pending gwt-discussion Goal Start"), "{text}");
        assert!(text.contains("Proposal A - Goal handoff"), "{text}");
        assert!(
            text.contains("verification handoff ready with User Verification Result recorded"),
            "{text}"
        );
        assert!(text.contains("create_goal"), "{text}");
        assert!(text.contains("discuss.goal_started"), "{text}");
    }

    #[test]
    fn user_prompt_submit_appends_legacy_pending_goal_when_started_from_subdirectory() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        init_git_repo(worktree.path());
        let subdir = worktree.path().join("nested/agent");
        std::fs::create_dir_all(&subdir).unwrap();
        let _home = ScopedEnvVar::set("HOME", worktree.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", worktree.path());
        let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let _runtime_path = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        write_pending_goal(worktree.path());

        let output =
            handle_with_input("UserPromptSubmit", "{}", &subdir, None).expect("hook output");

        let HookOutput::HookSpecificAdditionalContext { event, text } = output else {
            panic!("expected pending goal context");
        };
        assert_eq!(event, IntentBoundaryEvent::UserPromptSubmit);
        assert!(text.contains("pending gwt-discussion Goal Start"), "{text}");
        assert!(text.contains("Proposal A - Goal handoff"), "{text}");
    }

    #[test]
    fn session_start_pending_goal_context_uses_session_start_event_when_silent() {
        let output = append_pending_discussion_goal_context(
            HookOutput::Silent,
            IntentBoundaryEvent::SessionStart,
            Some(PendingDiscussionGoal {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Goal handoff".to_string(),
                condition: "tests green".to_string(),
            }),
        );

        let HookOutput::HookSpecificAdditionalContext { event, text } = output else {
            panic!("expected pending goal context");
        };
        assert_eq!(event, IntentBoundaryEvent::SessionStart);
        assert!(text.contains("pending gwt-discussion Goal Start"), "{text}");
    }

    // SPEC-3248 P7A (T-095): managed-hook lifecycle for the intake artifact
    // gate — a curation prompt marks the requirement dirty, Stop blocks while
    // no fresh outcome exists (auto-capturing one self-improvement
    // candidate), a valid outcome clears the block, and the next prompt makes
    // that outcome stale so Stop blocks again (updating the same candidate).
    #[test]
    fn intake_artifact_gate_lifecycle_blocks_until_fresh_outcome() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", worktree.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", worktree.path());
        let sessions_dir = worktree.path().join(".gwt").join("sessions");
        let mut session = Session::new(worktree.path(), "intake/curate", AgentId::ClaudeCode);
        session.agent_session_id = Some("agent-intake".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
        let _runtime_env = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        gwt_skills::write_lane_file(worktree.path(), &gwt_skills::INTAKE_PROFILE).unwrap();

        let prompt_input = serde_json::json!({
            "prompt": "このバグ報告を Issue に登録して",
            "session_id": "agent-intake",
        })
        .to_string();
        let stop_input = serde_json::json!({
            "session_id": "agent-intake",
            "stop_hook_active": false,
        })
        .to_string();

        // 1. Curation prompt marks the artifact requirement dirty.
        handle_with_input(
            "UserPromptSubmit",
            &prompt_input,
            worktree.path(),
            Some(&session_id),
        )
        .expect("prompt hook output");

        // 2. Stop without an outcome blocks and captures one candidate.
        let output = handle_with_input("Stop", &stop_input, worktree.path(), Some(&session_id))
            .expect("stop hook output");
        let HookOutput::StopBlock { reason } = output else {
            panic!("expected intake artifact gate StopBlock, got {output:?}");
        };
        assert!(reason.contains("Intake artifact gate"), "{reason}");
        let candidates = crate::cli::improvement::candidate_public_values(worktree.path());
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].get("occurrences").and_then(|v| v.as_u64()),
            Some(1)
        );

        // 3. A valid Issue/SPEC outcome clears the block.
        crate::cli::intake_outcome::record_outcome(
            worktree.path(),
            &session_id,
            crate::cli::intake_outcome::IntakeOutcome {
                kind: crate::cli::intake_outcome::IntakeOutcomeKind::IssueCreated,
                number: Some(4242),
                reason: None,
                source_operation: "issue.create".to_string(),
                recorded_at: chrono::Utc::now(),
            },
        )
        .unwrap();
        let output = handle_with_input("Stop", &stop_input, worktree.path(), Some(&session_id))
            .expect("stop hook output");
        assert!(
            !matches!(output, HookOutput::StopBlock { .. }),
            "fresh valid outcome must pass Stop, got {output:?}"
        );

        // 4. A later prompt makes the outcome stale; Stop blocks again and
        //    updates the same candidate (stable dedupe).
        std::thread::sleep(std::time::Duration::from_millis(20));
        handle_with_input(
            "UserPromptSubmit",
            &prompt_input,
            worktree.path(),
            Some(&session_id),
        )
        .expect("prompt hook output");
        let output = handle_with_input("Stop", &stop_input, worktree.path(), Some(&session_id))
            .expect("stop hook output");
        let HookOutput::StopBlock { reason } = output else {
            panic!("expected stale-outcome StopBlock, got {output:?}");
        };
        assert!(
            reason.contains("predates the latest user prompt"),
            "{reason}"
        );
        let candidates = crate::cli::improvement::candidate_public_values(worktree.path());
        assert_eq!(candidates.len(), 1, "dedupe must keep one candidate");
        assert_eq!(
            candidates[0].get("occurrences").and_then(|v| v.as_u64()),
            Some(2)
        );
    }

    // SPEC-3248 P8a (T-108/T-116 subset): a launch-written Execution Control
    // Record blocks Stop until the session settles it, then passes.
    #[test]
    fn execution_control_lifecycle_blocks_until_settled() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", worktree.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", worktree.path());
        let sessions_dir = worktree.path().join(".gwt").join("sessions");
        let mut session = Session::new(worktree.path(), "work/issue-42", AgentId::ClaudeCode);
        session.agent_session_id = Some("agent-exec".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
        let _runtime_env = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        gwt_skills::write_lane_file(worktree.path(), &gwt_skills::EXECUTION_PROFILE).unwrap();

        // Launch materialization wrote the record (plain Issue — no
        // build.start was ever called, T-109).
        crate::cli::execution_state::materialize_at_launch(
            worktree.path(),
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            42,
            &session_id,
            "gwt-execute",
            false,
        )
        .unwrap();

        let stop_input = serde_json::json!({
            "session_id": "agent-exec",
            "stop_hook_active": false,
        })
        .to_string();
        let output = handle_with_input("Stop", &stop_input, worktree.path(), Some(&session_id))
            .expect("stop hook output");
        let HookOutput::StopBlock { reason } = output else {
            panic!("expected execution control StopBlock, got {output:?}");
        };
        assert!(reason.contains("issue #42"), "{reason}");
        assert!(reason.contains("execution.complete"), "{reason}");

        // Settlement passes Stop.
        crate::cli::execution_state::settle(
            worktree.path(),
            &session_id,
            crate::cli::execution_state::ExecutionSettlement::Completed,
        )
        .unwrap();
        let output = handle_with_input("Stop", &stop_input, worktree.path(), Some(&session_id))
            .expect("stop hook output");
        assert!(
            !matches!(output, HookOutput::StopBlock { .. }),
            "settled execution must pass Stop, got {output:?}"
        );
    }

    // SPEC-3248 P7A review follow-up: stop-checks are evaluated lazily — when
    // an earlier gate (gwt-discussion) produces the StopBlock, the intake
    // completion gate must NOT run, so its auto-capture side effect fires
    // only for blocks the agent actually sees.
    #[test]
    fn earlier_stop_block_skips_intake_gate_side_effects() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", worktree.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", worktree.path());
        let sessions_dir = worktree.path().join(".gwt").join("sessions");
        let mut session = Session::new(worktree.path(), "intake/curate", AgentId::ClaudeCode);
        session.agent_session_id = Some("agent-intake".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
        let _runtime_env = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        gwt_skills::write_lane_file(worktree.path(), &gwt_skills::INTAKE_PROFILE).unwrap();

        // Arm the intake gate (dirty marker, no outcome) AND leave an active
        // discussion with a pending question so the discussion gate blocks
        // first.
        crate::cli::intake_outcome::mark_required_since(
            worktree.path(),
            &session_id,
            chrono::Utc::now(),
        )
        .unwrap();
        let discussions = worktree.path().join(".gwt/work/discussions.md");
        std::fs::create_dir_all(discussions.parent().unwrap()).unwrap();
        std::fs::write(
            &discussions,
            "## Discussion TODO\n\n\
             ### Proposal A - Hook-driven resume [active]\n\
             - Summary: Keep unfinished discussion state in the local artifact.\n\
             - Next Question: Should SessionStart surface the resume proposal?\n",
        )
        .unwrap();

        let stop_input = serde_json::json!({
            "session_id": "agent-intake",
            "stop_hook_active": false,
        })
        .to_string();
        let output = handle_with_input("Stop", &stop_input, worktree.path(), Some(&session_id))
            .expect("stop hook output");
        let HookOutput::StopBlock { reason } = output else {
            panic!("expected discussion StopBlock, got {output:?}");
        };
        assert!(
            reason.contains("Discussion is still"),
            "discussion gate must win: {reason}"
        );
        assert!(
            !reason.contains("Intake artifact gate"),
            "intake gate must not contribute: {reason}"
        );
        assert!(
            crate::cli::improvement::candidate_public_values(worktree.path()).is_empty(),
            "intake auto-capture must not fire when an earlier gate blocks"
        );
    }

    #[test]
    fn stop_blocks_push_only_completion_claim_without_pr_evidence() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        let sessions_dir = worktree.path().join(".gwt").join("sessions");
        let mut session = Session::new(worktree.path(), "feature/demo", AgentId::Codex);
        session.agent_session_id = Some("agent-123".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
        let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        let transcript = worktree.path().join("transcript.jsonl");
        std::fs::write(
            &transcript,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"完了しました。Issue #3233 は 3d827ffe2 として work/issue-3233 に push 済みです。Issue には closure comment も投稿しました。"}]}}"#,
        )
        .unwrap();
        let input = serde_json::json!({
            "transcript_path": transcript,
            "session_id": "agent-123",
            "stop_hook_active": false
        })
        .to_string();

        let output = handle_with_input("Stop", &input, worktree.path(), Some(&session_id))
            .expect("hook output");

        let HookOutput::StopBlock { reason } = output else {
            panic!("expected push-only completion StopBlock, got {output:?}");
        };
        assert!(reason.contains("PR"), "{reason}");
        assert!(reason.contains("push-only"), "{reason}");
        assert!(reason.contains("gwt-manage-pr"), "{reason}");
    }

    #[test]
    fn claude_child_pre_tool_use_denies_root_intake_checkpoint_cli() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let _runtime_path = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let input = r#"{
          "tool_name":"Bash",
          "tool_input":{"command":"$GWT_BIN < payload.json # intake.checkpoint.update"},
          "agent_id":"child-agent",
          "agent_type":"general-purpose"
        }"#;

        let output = handle_with_input("PreToolUse", input, worktree.path(), None).unwrap();

        assert!(matches!(
            output,
            HookOutput::PreToolUsePermission { ref summary, .. }
                if summary == "Intake checkpoints are root-session owned"
        ));
    }
}
