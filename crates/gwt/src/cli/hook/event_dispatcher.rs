//! Event-level hook dispatcher.
//!
//! Managed Claude/Codex hook configs call this once per hook event. The
//! dispatcher preserves the previous per-handler ordering while keeping a
//! single stdout envelope for runtimes that require hook output to be one
//! valid JSON document.

use std::{path::Path, time::Instant};

use super::{
    action_obligation_stop_check, autonomous_question_guard, board_reminder, diagnostics,
    execution_control_stop_check, pm_loop_stop_check, skill_build_spec_stop_check,
    skill_discussion_stop_check, skill_plan_spec_stop_check, skill_register_spec_stop_check,
    work_event_settlement_stop_check, workflow_policy, workspace_identity, GwtSessionId,
    HookAgentSessionId, HookError, HookOutput, IntentBoundaryEvent, RawHookEvent,
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
    // Issue #3478 (AC-1/AC-2): deliver the autonomous decision policy at the
    // intent boundary so the agent resolves reversible choices itself instead
    // of reaching for a question tool.
    let output = append_additional_context(
        output,
        IntentBoundaryEvent::SessionStart,
        autonomous_decision_policy_context(),
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

/// The autonomous decision policy for this session, or `None` for every
/// human-driven launch (which must stay byte-identical to before).
fn autonomous_decision_policy_context() -> Option<String> {
    crate::autonomous_handoff::autonomous_execution_context_from_env(|name| {
        std::env::var(name).ok()
    })
    .as_ref()
    .map(crate::autonomous_handoff::autonomous_decision_policy)
}

fn handle_user_prompt_submit(
    event: &str,
    input: &str,
    worktree_root: &Path,
) -> Result<HookOutput, HookError> {
    let pm_refresh_context = pm_loop_stop_check::handle_user_prompt_submit(worktree_root)?;
    run_step(event, "runtime-state", || {
        crate::daemon_runtime::handle_runtime_state(event, input)
    })?;
    run_value(event, "autonomous-answer-receipt", || {
        if let Err(error) = handle_autonomous_answer_receipt(worktree_root, input) {
            tracing::warn!(%error, "autonomous answer receipt was rejected");
        }
    });
    run_step(event, "forward", || {
        crate::daemon_runtime::handle_forward(input)
    })?;
    run_value(event, "pm-delivery-ack", || {
        pm_loop_stop_check::handle_delivery_acknowledgement(worktree_root, input);
    });
    // SPEC-2359 Phase W-11 (US-58): the workspace-identity step no longer
    // derives a title from the prompt; it only performs the Phase W-10
    // canonical Project State split repair. Fail-open so a repair error does
    // not abort prompt handling.
    run_value(event, "workspace-identity", || {
        if let Err(error) = workspace_identity::handle_user_prompt_submit(input) {
            tracing::warn!(?error, "workspace-identity hook step failed");
        }
    });
    // SPEC-3248 P11 (T-240 core): producing prompts in execution lanes arm
    // typed action obligations. Fail-open state writer.
    run_value(event, "action-obligation-record", || {
        action_obligation_stop_check::handle_user_prompt_submit(worktree_root, input);
    });
    let output = run_step(event, "board-reminder", || {
        board_reminder::handle_with_input(event, input)
    })?;
    let output = append_additional_context(
        output,
        IntentBoundaryEvent::UserPromptSubmit,
        pm_refresh_context,
    );
    let output = append_additional_context(
        output,
        IntentBoundaryEvent::UserPromptSubmit,
        autonomous_decision_policy_context(),
    );
    let pending_goal = run_value(event, "discussion-goal-start", || {
        load_pending_goal_for_hook_worktree(worktree_root)
    });
    Ok(append_pending_discussion_goal_context(
        output,
        IntentBoundaryEvent::UserPromptSubmit,
        pending_goal,
    ))
}

/// Finalize one autonomous answer only when the exact managed Session and
/// native conversation that own the protected prompt observe UserPromptSubmit.
fn handle_autonomous_answer_receipt(worktree_root: &Path, input: &str) -> Result<bool, String> {
    let Some(raw) = RawHookEvent::read_from_str(input).map_err(|error| error.to_string())? else {
        return Ok(false);
    };
    let Some(prompt) = raw.prompt() else {
        return Ok(false);
    };
    let Some(marker) =
        crate::autonomous_handoff::parse_protected_autonomous_handoff_answer_prompt(prompt)
    else {
        return Ok(false);
    };
    let Some(current_gwt_session_id) = GwtSessionId::from_env() else {
        return Ok(false);
    };
    let sessions_dir = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .map(std::path::PathBuf::from)
        .and_then(|path| gwt_agent::sessions_dir_from_runtime_path(&path))
        .unwrap_or_else(gwt_core::paths::gwt_sessions_dir);
    let current = gwt_agent::Session::load(
        &sessions_dir.join(format!("{}.toml", current_gwt_session_id.as_str())),
    )
    .map_err(|error| format!("current Session receipt identity is unavailable: {error}"))?;
    if current.id != current_gwt_session_id.as_str() {
        return Ok(false);
    }
    let source =
        gwt_agent::Session::load(&sessions_dir.join(format!("{}.toml", marker.session_id)))
            .map_err(|error| format!("asking Session receipt identity is unavailable: {error}"))?;
    let provider_session_id = match super::resolve_hook_agent_session_id(Some(&current), Some(&raw))
    {
        HookAgentSessionId::Provided(session_id) => session_id.into_string(),
        HookAgentSessionId::MissingRequiredForCodex
            if current.agent_id == gwt_agent::AgentId::Codex =>
        {
            let Some(session_id) = current.exact_resume_session_id() else {
                return Ok(false);
            };
            session_id.to_string()
        }
        HookAgentSessionId::MissingRequiredForCodex | HookAgentSessionId::MissingOptional => {
            return Ok(false);
        }
    };
    let Some(current_native_id) = current.exact_resume_session_id() else {
        return Ok(false);
    };
    let Some(source_native_id) = source.exact_resume_session_id() else {
        return Ok(false);
    };
    let current_project_state_root =
        match crate::agent_project_state::validated_project_state_root_for_session_recovery(
            &current,
        ) {
            Ok(root) => root,
            Err(_) => return Ok(false),
        };
    let source_project_state_root =
        match crate::agent_project_state::validated_project_state_root_for_session_recovery(&source)
        {
            Ok(root) => root,
            Err(_) => return Ok(false),
        };
    let issue_number = std::env::var(crate::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let resolved_worktree = gwt_core::paths::resolve_current_worktree_root(worktree_root);
    if current.agent_id != source.agent_id
        || current_native_id != source_native_id
        || provider_session_id != current_native_id
        || current.linked_issue_number.is_none()
        || current.linked_issue_number != source.linked_issue_number
        || current.linked_issue_number != issue_number
        || current.repo_hash.is_none()
        || current.repo_hash != source.repo_hash
        || !same_receipt_path(&current.worktree_path, &resolved_worktree)
        || !same_receipt_path(&source.worktree_path, &resolved_worktree)
        || !same_receipt_path(&current_project_state_root, &source_project_state_root)
    {
        return Ok(false);
    }
    let prefs_path =
        crate::issue_monitor::issue_monitor_prefs_path_for_repo_path(&resolved_worktree);
    let receipt_identity = crate::autonomous_handoff::AutonomousHandoffReceiptIdentity {
        gwt_session_id: current.id.clone(),
        native_session_id: current_native_id.to_string(),
        provider: current.agent_id.to_string(),
        issue_number: current
            .linked_issue_number
            .expect("validated linked Issue identity"),
        repo_hash: current
            .repo_hash
            .clone()
            .expect("validated repository identity"),
        project_state_root: current_project_state_root.to_string_lossy().into_owned(),
    };
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    crate::issue_monitor::acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
        &prefs_path,
        &source.id,
        &receipt_identity,
        prompt,
        &now,
    )
    .map_err(|error| error.to_string())
}

fn same_receipt_path(left: &Path, right: &Path) -> bool {
    match (dunce::canonicalize(left), dunce::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn handle_pre_tool_use(event: &str, input: &str) -> Result<HookOutput, HookError> {
    run_step(event, "runtime-state", || {
        crate::daemon_runtime::handle_runtime_state(event, input)
    })?;
    run_step(event, "forward", || {
        crate::daemon_runtime::handle_forward(input)
    })?;
    // Issue #3478 (FR-025): the question guard runs before every other policy.
    // A question tool call must be converted while it is still refusable — any
    // later check that returns first would let the provider open the question
    // UI and hold the Issue Monitor slot until the stuck timeout.
    let question_guard = run_step(event, "autonomous-question-guard", || {
        autonomous_question_guard::handle_with_input(input)
    })?;
    if question_guard != HookOutput::Silent {
        return Ok(question_guard);
    }
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
    // wins and the remaining checks must NOT run.
    let stop_checks: [(&str, StopCheck<'_>); 8] = [
        // SPEC-3431 FR-012: the resident PM's loop driver runs first — for the
        // PM every other stop gate is either exempt (FR-029) or fail-open, and
        // the loop continuation must not be shadowed by one of them.
        (
            "pm-loop-stop-check",
            Box::new(|| {
                pm_loop_stop_check::handle_with_input(worktree_root, input, current_session)
            }),
        ),
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
        (
            "work-event-settlement-stop-check",
            Box::new(|| {
                work_event_settlement_stop_check::handle_with_input(
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
        // SPEC-3248 P11 (T-242 core): open producing obligations from this
        // session's prompts block Stop until settled by canonical
        // operations or deferred via execution.blocked.
        (
            "action-obligation-stop-check",
            Box::new(|| {
                action_obligation_stop_check::handle_with_input(
                    worktree_root,
                    input,
                    current_session,
                )
            }),
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

    /// Issue #3716: a physical launch/write is not an answer receipt. Only a
    /// protected prompt observed by the exact native conversation may commit
    /// delivered_at; a forged provider session id remains silent.
    #[test]
    fn grok_user_prompt_submit_receipts_the_exact_autonomous_answer() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let worktree = home.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("create repo");
        init_git_repo(&worktree);
        let remote = gwt_core::process::hidden_command("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/example/receipt-test.git",
            ])
            .current_dir(&worktree)
            .status()
            .expect("configure origin");
        assert!(remote.success(), "configure origin failed");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _runtime_path = ScopedEnvVar::unset(GWT_SESSION_RUNTIME_PATH_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        let _autonomous =
            ScopedEnvVar::set(crate::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV, "1");
        let _autonomous_issue =
            ScopedEnvVar::set(crate::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV, "3716");
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let native_id = "01a0195c-fbd7-7352-8d29-da6f6f755016";
        let mut source = Session::new(&worktree, "work/issue-3716", AgentId::GrokBuild);
        source.id = "asking-gwt-session".to_string();
        source.agent_session_id = Some(native_id.to_string());
        source.linked_issue_number = Some(3716);
        source.project_state_root = Some(worktree.clone());
        source.repo_hash = Some(
            gwt_core::repo_hash::detect_repo_hash(&worktree)
                .expect("origin repo hash")
                .as_str()
                .to_string(),
        );
        source.save(&sessions_dir).expect("save asking Session");
        let mut current = source.clone();
        current.id = "resumed-gwt-session".to_string();
        current.save(&sessions_dir).expect("save resumed Session");
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &current.id);

        let handoff = crate::autonomous_handoff::AutonomousQuestionHandoff::new(
            "handoff-3716-receipt".to_string(),
            &crate::autonomous_handoff::AutonomousExecutionContext {
                issue_number: 3716,
                session_id: source.id.clone(),
            },
            &current.agent_id.to_string(),
            "ask_user_question",
            crate::autonomous_handoff::ExtractedQuestion {
                question: "Proceed with the release?".to_string(),
                options: Vec::new(),
            },
            "2026-08-20T00:00:00Z",
        );
        let mut monitor = crate::issue_monitor::IssueMonitorState::with_prefs(
            crate::issue_monitor::IssueMonitorConfig::default(),
            crate::issue_monitor::IssueMonitorPrefs {
                autonomous_mode: true,
                ..crate::issue_monitor::IssueMonitorPrefs::default()
            },
        );
        monitor.absorb_autonomous_handoffs(vec![handoff]);
        monitor.apply_pending_autonomous_handoffs("2026-08-20T00:00:30Z");
        assert!(monitor.answer_autonomous_handoff(
            "handoff-3716-receipt",
            "Proceed",
            "2026-08-20T00:01:00Z",
        ));
        monitor.resume_answered_autonomous_handoffs("2026-08-20T00:01:30Z");
        monitor.complete_active_launch(3716, "tab-1::agent-receipt");
        let prefs_path = crate::issue_monitor::issue_monitor_prefs_path_for_repo_path(&worktree);
        crate::issue_monitor::save_issue_monitor_prefs(&prefs_path, &monitor.prefs())
            .expect("seed handoff");
        let prepared = crate::issue_monitor::prepare_autonomous_handoff_delivery_from_prefs(
            &prefs_path,
            3716,
            "2026-08-20T00:02:00Z",
        )
        .expect("prepare answer")
        .expect("pending handoff");
        let crate::issue_monitor::AutonomousHandoffDeliveryPreparation::Ready(prepared) = prepared
        else {
            panic!("expected ready answer delivery");
        };
        let project_state_root =
            crate::agent_project_state::validated_project_state_root_for_session_recovery(&current)
                .expect("validated receipt Project State");
        assert!(
            crate::issue_monitor::bind_autonomous_handoff_delivery_target_from_prefs(
                &prefs_path,
                &prepared.handoff_id,
                &prepared.session_id,
                prepared.attempt,
                &crate::autonomous_handoff::AutonomousHandoffDeliveryTarget {
                    gwt_session_id: current.id.clone(),
                    native_session_id: native_id.to_string(),
                    provider: current.agent_id.to_string(),
                    issue_number: 3716,
                    repo_hash: current.repo_hash.clone().expect("repo hash"),
                    project_state_root: project_state_root.to_string_lossy().into_owned(),
                    window_id: "tab-1::agent-receipt".to_string(),
                    materializer_id: "receipt-test-materializer".to_string(),
                    materializer_pid: std::process::id(),
                    materializer_started_at: crate::process::host_process_start_time(
                        std::process::id(),
                    )
                    .expect("receipt test materializer start time"),
                    delivery_id: None,
                },
            )
            .expect("bind receipt target")
        );

        let forged = serde_json::json!({
            "sessionId": "different-native-session",
            "prompt": prepared.prompt,
        })
        .to_string();
        assert!(!handle_autonomous_answer_receipt(&worktree, &forged).expect("forged receipt"));
        let pending = crate::issue_monitor::load_issue_monitor_prefs(&prefs_path)
            .expect("load pending receipt");
        assert!(pending.autonomous_handoffs[0].delivered_at.is_none());

        let receipt = serde_json::json!({
            "sessionId": native_id,
            "prompt": prepared.prompt,
        })
        .to_string();
        assert!(handle_autonomous_answer_receipt(&worktree, &receipt).expect("exact receipt"));
        let delivered = crate::issue_monitor::load_issue_monitor_prefs(&prefs_path)
            .expect("load delivered receipt");
        assert!(delivered.autonomous_handoffs[0].delivered_at.is_some());
    }

    /// Issue #3478 (AC-3): the question guard runs on PreToolUse, and it must
    /// win over the later policy checks so a question can never reach a
    /// waiting UI while some other guard debates the same call.
    #[test]
    fn pre_tool_use_converts_an_autonomous_question_before_any_other_policy() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        init_git_repo(worktree.path());
        let _home = ScopedEnvVar::set("HOME", worktree.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", worktree.path());
        let session_id = "session-question-guard";
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, session_id);
        let _runtime_path = ScopedEnvVar::unset(GWT_SESSION_RUNTIME_PATH_ENV);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        let _autonomous =
            ScopedEnvVar::set(crate::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV, "1");
        let _autonomous_issue =
            ScopedEnvVar::set(crate::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV, "3478");

        let input = serde_json::json!({
            "tool_name": "AskUserQuestion",
            "tool_input": {"questions": [{"question": "Delete the release tag?"}]}
        });
        let output = handle_with_input("PreToolUse", &input.to_string(), worktree.path(), None)
            .expect("PreToolUse output");

        let HookOutput::PreToolUsePermission { summary, .. } = output else {
            panic!("expected the autonomous question to be denied");
        };
        assert_eq!(
            summary,
            crate::cli::hook::autonomous_question_guard::QUESTION_HANDOFF_SUMMARY
        );
    }

    /// AC-6 non-regression: without the autonomous markers the same question
    /// tool passes straight through.
    #[test]
    fn pre_tool_use_leaves_a_human_driven_question_alone() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        init_git_repo(worktree.path());
        let _home = ScopedEnvVar::set("HOME", worktree.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", worktree.path());
        let _session = ScopedEnvVar::unset(GWT_SESSION_ID_ENV);
        let _runtime_path = ScopedEnvVar::unset(GWT_SESSION_RUNTIME_PATH_ENV);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        let _autonomous =
            ScopedEnvVar::unset(crate::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV);
        let _autonomous_issue =
            ScopedEnvVar::unset(crate::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV);

        let input = serde_json::json!({
            "tool_name": "AskUserQuestion",
            "tool_input": {"questions": [{"question": "Which option do you prefer?"}]}
        });
        let output = handle_with_input("PreToolUse", &input.to_string(), worktree.path(), None)
            .expect("PreToolUse output");

        assert_eq!(output, HookOutput::Silent);
    }

    /// AC-1/AC-2: the decision policy reaches the agent at every intent
    /// boundary, so it survives context compaction.
    #[test]
    fn session_start_injects_the_autonomous_decision_policy() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        init_git_repo(worktree.path());
        let _home = ScopedEnvVar::set("HOME", worktree.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", worktree.path());
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-policy");
        let _runtime_path = ScopedEnvVar::unset(GWT_SESSION_RUNTIME_PATH_ENV);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        let _autonomous =
            ScopedEnvVar::set(crate::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV, "1");
        let _autonomous_issue =
            ScopedEnvVar::set(crate::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV, "3478");

        let output =
            handle_with_input("SessionStart", "{}", worktree.path(), None).expect("hook output");

        let HookOutput::HookSpecificAdditionalContext { text, .. } = output else {
            panic!("expected additional context carrying the autonomous policy");
        };
        assert!(
            text.contains("Autonomous execution policy (Issue #3478)"),
            "{text}"
        );
        assert!(text.contains("Question tools are blocked"), "{text}");
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
    fn pre_tool_use_keeps_recovery_reachable_for_stale_binding_without_a_host_bridge() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        init_git_repo(worktree.path());
        let remote_status = gwt_core::process::hidden_command("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/stale-policy.git",
            ])
            .current_dir(worktree.path())
            .status()
            .expect("git remote add");
        assert!(remote_status.success(), "git remote add failed");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let sessions_dir = home.path().join(".gwt").join("sessions");
        let mut session = Session::new(worktree.path(), "work/issue-3394", AgentId::Codex);
        session.linked_issue_number = Some(3394);
        let session_id = session.id.clone();
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 3394,
        };
        session.save(&sessions_dir).unwrap();
        crate::cli::execution_state::materialize_at_launch(
            worktree.path(),
            owner.kind,
            owner.number,
            &session_id,
            "gwt-execute",
            false,
        )
        .unwrap();
        crate::cli::execution_state::ensure_generation_ledger(
            worktree.path(),
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .unwrap();
        let current =
            crate::cli::execution_state::current_execution_binding(worktree.path(), owner)
                .unwrap()
                .unwrap();
        session
            .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session_id.clone(),
                repo_hash: session.repo_hash.clone().unwrap(),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity: current,
                capability_generation: 1,
            }))
            .unwrap();
        session.save(&sessions_dir).unwrap();
        let takeover = crate::cli::execution_state::GenerationTakeoverRequest {
            operation_id: "stale-policy-fixture".to_string(),
            principal_id: "test-host".to_string(),
            work_id: Some(format!("work-session-{session_id}")),
            source: Some("continue-work:resume".to_string()),
            from_session_id: session_id.clone(),
            to_session_id: "replacement-session".to_string(),
            reason: "test stale predecessor".to_string(),
            requested_at: chrono::Utc::now(),
        };
        crate::cli::execution_state::prepare_generation_takeover(worktree.path(), owner, &takeover)
            .unwrap();
        crate::cli::execution_state::activate_generation_takeover(
            worktree.path(),
            owner,
            &takeover,
        )
        .unwrap();
        assert!(
            !crate::cli::execution_state::current_active_execution_binding_matches(
                worktree.path(),
                owner,
                &session_id,
                &session.execution_binding.as_ref().unwrap().identity,
            )
            .unwrap()
        );
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
        let _runtime_path =
            ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, runtime_path.as_os_str());
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");

        for input in [
            serde_json::json!({
                "tool_name": "Edit",
                "tool_input": {
                    "file_path": "crates/gwt/src/lib.rs",
                    "old_string": "old",
                    "new_string": "new"
                }
            }),
            serde_json::json!({
                "tool_name": "Write",
                "tool_input": {
                    "file_path": "crates/gwt/src/lib.rs",
                    "content": "replacement"
                }
            }),
            serde_json::json!({
                "tool_name": "Bash",
                "tool_input": { "command": "git add crates/gwt/src/lib.rs" }
            }),
            serde_json::json!({
                "tool_name": "Bash",
                "tool_input": { "command": "cargo test -p gwt --lib" }
            }),
            serde_json::json!({
                "tool_name": "Bash",
                "tool_input": {
                    "command": "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"execution.status\",\"params\":{}}\nJSON"
                }
            }),
            serde_json::json!({
                "tool_name": "Bash",
                "tool_input": {
                    "command": "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"execution.continue\",\"params\":{\"operation_id\":\"recover-stale-binding\"}}\nJSON"
                }
            }),
        ] {
            let output = handle_with_input("PreToolUse", &input.to_string(), worktree.path(), None)
                .expect("PreToolUse output");
            assert_eq!(
                output,
                HookOutput::Silent,
                "general issue-owned work must not depend on Host binding availability: {input}"
            );
        }
        assert!(
            runtime_path.exists(),
            "removing the binding step must preserve later runtime-state handling"
        );
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

    // SPEC #3245 FR-001 / AC-1: the intake completion hard gate is removed.
    // A session that registers nothing stops exactly like an execution
    // session — no artifact outcome requirement, no auto-capture. Uniform
    // gates (e.g. the P11 obligation gate) apply to everyone equally.
    #[test]
    fn intake_shaped_stop_never_hits_the_artifact_gate() {
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

        handle_with_input(
            "UserPromptSubmit",
            &prompt_input,
            worktree.path(),
            Some(&session_id),
        )
        .expect("prompt hook output");

        let output = handle_with_input("Stop", &stop_input, worktree.path(), Some(&session_id))
            .expect("stop hook output");
        // The uniform prompt-to-action obligation gate (SPEC-3248 P11) may
        // legitimately fire — exactly as it would for an execution session.
        // What must never fire again is the removed intake artifact gate.
        if let HookOutput::StopBlock { reason } = &output {
            assert!(
                !reason.contains("Intake artifact gate"),
                "the removed intake artifact gate must not contribute: {reason}"
            );
            assert!(
                reason.contains("Producing obligations"),
                "only the uniform obligation gate may block here: {reason}"
            );
        }
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

    #[test]
    fn stop_allows_legitimate_completion_report_without_scanning_prose() {
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

        assert!(
            !matches!(output, HookOutput::StopBlock { .. }),
            "legitimate completion prose must not be interpreted as gate state: {output:?}"
        );
    }
}
