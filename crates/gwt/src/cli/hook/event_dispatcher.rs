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
    work_event_settlement_stop_check, workflow_policy, workspace_identity, HookError, HookOutput,
    IntentBoundaryEvent,
};
use crate::discussion_resume::{
    load_pending_goal, load_pending_goal_from_worktree_files, PendingDiscussionGoal,
};

pub(super) const USER_PROMPT_SUBMIT_HOOK_DEADLINE: std::time::Duration =
    std::time::Duration::from_millis(200);

pub fn handle_with_input(
    event: &str,
    input: &str,
    worktree_root: &Path,
    current_session: Option<&str>,
) -> Result<HookOutput, HookError> {
    let started = Instant::now();
    #[cfg(debug_assertions)]
    mark_playwright_hook_dispatcher_entry();
    diagnostics::begin_event();
    let _deadline = enter_event_deadline(event, started);
    let result = match event {
        "SessionStart" => handle_session_start(event, input, worktree_root),
        "UserPromptSubmit" => handle_user_prompt_submit(event, input, worktree_root),
        "PreToolUse" => handle_pre_tool_use(event, input),
        "PostToolUse" => handle_post_tool_use(event, input),
        "Stop" => handle_stop(event, input, worktree_root, current_session),
        other => Err(HookError::InvalidEvent(other.to_string())),
    };
    let additional_context_bytes = result
        .as_ref()
        .ok()
        .map(additional_context_bytes)
        .unwrap_or_default();
    diagnostics::record_event_total(
        event,
        started.elapsed(),
        if result.is_ok() { "ok" } else { "error" },
        diagnostics::event_metrics(additional_context_bytes),
    );
    result
}

#[cfg(debug_assertions)]
fn mark_playwright_hook_dispatcher_entry() {
    let Ok(path) = std::env::var("GWT_PLAYWRIGHT_HOOK_DISPATCHER_RENDEZVOUS") else {
        return;
    };
    if path.trim().is_empty() {
        return;
    }
    let _ = std::fs::write(path, b"dispatcher-entered\n");
}

fn enter_event_deadline(
    event: &str,
    started: Instant,
) -> Option<gwt_core::operation_deadline::ScopedOperationDeadline> {
    (event == "UserPromptSubmit").then(|| {
        gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            started + USER_PROMPT_SUBMIT_HOOK_DEADLINE,
        )
    })
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
    // The three bookkeeping writes below are independent of RuntimeState and
    // Board reminder planning. Start them together so a contended Work refresh
    // contributes its slowest substage to the prompt wall clock, not the sum of
    // every lock/write. Their timing records are emitted after join in the
    // historical handler order, keeping the exact profile contract stable.
    let event_deadline = gwt_core::operation_deadline::current();
    std::thread::scope(|scope| {
        let pm_delivery_ack = scope.spawn(|| {
            let _deadline =
                event_deadline.map(gwt_core::operation_deadline::ScopedOperationDeadline::enter);
            let started = Instant::now();
            pm_loop_stop_check::handle_delivery_acknowledgement(worktree_root, input);
            started.elapsed()
        });
        let action_obligation = scope.spawn(|| {
            let _deadline =
                event_deadline.map(gwt_core::operation_deadline::ScopedOperationDeadline::enter);
            let started = Instant::now();
            action_obligation_stop_check::handle_user_prompt_submit(worktree_root, input);
            started.elapsed()
        });
        let pm_loop_reset = scope.spawn(|| {
            let _deadline =
                event_deadline.map(gwt_core::operation_deadline::ScopedOperationDeadline::enter);
            let started = Instant::now();
            pm_loop_stop_check::handle_user_prompt_submit(worktree_root);
            started.elapsed()
        });

        let prepared_session = run_step(event, "runtime-state", || {
            crate::daemon_runtime::handle_runtime_state_prepared(event, input)
        });
        let pm_delivery_ack_duration = pm_delivery_ack
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        diagnostics::record_handler_duration(
            event,
            "pm-delivery-ack",
            pm_delivery_ack_duration,
            "ok",
        );

        match prepared_session {
            Ok(prepared_session) => {
                // SPEC-2359 Phase W-11 (US-58): the workspace-identity step no
                // longer derives a title from the prompt; it only performs the
                // Phase W-10 canonical Project State split repair. Keep it
                // before Board planning so the reminder consumes the repaired
                // authority.
                run_value(event, "workspace-identity", || {
                    let result = prepared_session
                        .as_ref()
                        .map(workspace_identity::handle_user_prompt_submit_for_session)
                        .unwrap_or_else(|| workspace_identity::handle_user_prompt_submit(input));
                    if let Err(error) = result {
                        tracing::warn!(?error, "workspace-identity hook step failed");
                    }
                });

                let board_started = Instant::now();
                let output = if let Some(session) = prepared_session.as_ref() {
                    board_reminder::handle_with_input_for_session(event, input, session)
                } else {
                    board_reminder::handle_with_input(event, input)
                };
                let board_duration = board_started.elapsed();

                let action_obligation_duration = action_obligation
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                let pm_loop_reset_duration = pm_loop_reset
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                diagnostics::record_handler_duration(
                    event,
                    "action-obligation-record",
                    action_obligation_duration,
                    "ok",
                );
                diagnostics::record_handler_duration(
                    event,
                    "pm-loop-reset",
                    pm_loop_reset_duration,
                    "ok",
                );
                diagnostics::record_handler_duration(
                    event,
                    "board-reminder",
                    board_duration,
                    if output.is_ok() { "ok" } else { "error" },
                );

                let output = append_additional_context(
                    output?,
                    IntentBoundaryEvent::UserPromptSubmit,
                    autonomous_decision_policy_context(),
                );
                let pending_goal = run_value(event, "discussion-goal-start", || {
                    load_pending_goal_for_hook_worktree_with_session(
                        worktree_root,
                        prepared_session.as_ref(),
                    )
                });
                Ok(append_pending_discussion_goal_context(
                    output,
                    IntentBoundaryEvent::UserPromptSubmit,
                    pending_goal,
                ))
            }
            Err(error) => {
                // RuntimeState failures still join every scoped writer before
                // returning and keep the allowlisted diagnostic order valid.
                let action_obligation_duration = action_obligation
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                let pm_loop_reset_duration = pm_loop_reset
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                diagnostics::record_handler_duration(
                    event,
                    "action-obligation-record",
                    action_obligation_duration,
                    "ok",
                );
                diagnostics::record_handler_duration(
                    event,
                    "pm-loop-reset",
                    pm_loop_reset_duration,
                    "ok",
                );
                Err(error)
            }
        }
    })
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

fn load_pending_goal_for_hook_worktree_with_session(
    worktree_root: &Path,
    prepared_session: Option<&gwt_agent::Session>,
) -> Option<PendingDiscussionGoal> {
    if let Some(session) = prepared_session {
        return load_pending_goal_from_worktree_files(&session.worktree_path)
            .ok()
            .flatten();
    }
    load_pending_goal_for_hook_worktree(worktree_root)
}

fn additional_context_bytes(output: &HookOutput) -> usize {
    match output {
        HookOutput::HookSpecificAdditionalContext { text, .. } => text.len(),
        HookOutput::PreToolUsePermission { .. }
        | HookOutput::SystemMessage(_)
        | HookOutput::Silent
        | HookOutput::StopBlock { .. } => 0,
    }
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
    use crate::board_remote::{http::ReqwestHttpClient, slack::SlackProvider};
    use crate::discussion_resume::PendingDiscussionGoal;
    use axum::{extract::State, routing::get, Json, Router};
    use gwt_agent::{AgentId, Session, GWT_SESSION_ID_ENV, GWT_SESSION_RUNTIME_PATH_ENV};
    use gwt_core::coordination::{BoardEntry, BoardEntryKind, BoardProvider};
    use gwt_core::test_support::ScopedEnvVar;
    use serde_json::Value;
    use std::{collections::BTreeMap, rc::Rc, sync::mpsc, time::Duration};
    use tokio::{net::TcpListener, runtime::Runtime, sync::oneshot};

    #[derive(Debug)]
    enum DegradedEndpointCall {
        BoardHistory,
    }

    #[derive(Clone)]
    struct DegradedEndpointState {
        calls: mpsc::Sender<DegradedEndpointCall>,
    }

    struct DegradedEndpointServer {
        runtime: Runtime,
        shutdown: Option<oneshot::Sender<()>>,
        calls: mpsc::Receiver<DegradedEndpointCall>,
        base_url: String,
    }

    #[test]
    fn issue_3777_playwright_rendezvous_marks_actual_dispatcher_entry() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let marker = temp.path().join("dispatcher-entered");
        let _rendezvous = ScopedEnvVar::set("GWT_PLAYWRIGHT_HOOK_DISPATCHER_RENDEZVOUS", &marker);

        let result = handle_with_input("UnsupportedEvent", "{}", temp.path(), None);

        assert!(matches!(result, Err(HookError::InvalidEvent(_))));
        assert_eq!(
            std::fs::read_to_string(marker).expect("dispatcher rendezvous marker"),
            "dispatcher-entered\n"
        );
    }

    impl DegradedEndpointServer {
        fn start() -> Self {
            let runtime = Runtime::new().expect("degraded endpoint runtime");
            let listener = runtime
                .block_on(TcpListener::bind(("127.0.0.1", 0)))
                .expect("bind degraded endpoint");
            let address = listener.local_addr().expect("degraded endpoint address");
            let (calls_tx, calls) = mpsc::channel();
            let (shutdown, shutdown_rx) = oneshot::channel();
            let app = Router::new()
                .route("/api/conversations.history", get(delayed_board_history))
                .with_state(DegradedEndpointState { calls: calls_tx });
            runtime.spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .expect("serve degraded endpoints");
            });
            Self {
                runtime,
                shutdown: Some(shutdown),
                calls,
                base_url: format!("http://127.0.0.1:{}", address.port()),
            }
        }

        fn slack_api_base(&self) -> String {
            format!("{}/api", self.base_url)
        }

        fn collected_calls(&self) -> Vec<DegradedEndpointCall> {
            self.calls.try_iter().collect()
        }
    }

    impl Drop for DegradedEndpointServer {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            self.runtime
                .block_on(async { tokio::time::sleep(Duration::from_millis(10)).await });
        }
    }

    async fn delayed_board_history(State(state): State<DegradedEndpointState>) -> Json<Value> {
        state
            .calls
            .send(DegradedEndpointCall::BoardHistory)
            .expect("record Board history request");
        tokio::time::sleep(Duration::from_millis(400)).await;
        Json(serde_json::json!({
            "ok": true,
            "messages": [],
            "response_metadata": {"next_cursor": ""}
        }))
    }

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
    fn only_user_prompt_submit_enters_the_aggregate_deadline() {
        assert!(gwt_core::operation_deadline::current().is_none());
        let started = Instant::now();
        let guard = enter_event_deadline("UserPromptSubmit", started)
            .expect("UserPromptSubmit deadline guard");
        let deadline = gwt_core::operation_deadline::current().expect("aggregate deadline");
        assert!(deadline > started);
        assert!(deadline <= started + USER_PROMPT_SUBMIT_HOOK_DEADLINE);
        drop(guard);
        assert!(gwt_core::operation_deadline::current().is_none());
        for event in ["SessionStart", "PreToolUse", "PostToolUse", "Stop"] {
            assert!(enter_event_deadline(event, started).is_none(), "{event}");
        }
    }

    #[test]
    fn degraded_remote_board_and_hook_live_fail_open_within_prompt_budget() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated HOME");
        let worktree = home.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("create worktree");
        init_git_repo(&worktree);
        let sessions_dir = home.path().join(".gwt/sessions");
        let mut session = Session::new(&worktree, "work/degraded-prompt", AgentId::Codex);
        session.agent_session_id = Some("agent-degraded-prompt".to_string());
        session.save(&sessions_dir).expect("save Session");
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session.id);
        let profile_path = home.path().join("hook-profile.jsonl");
        let server = DegradedEndpointServer::start();
        let unavailable_hook =
            std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve unavailable hook port");
        let unavailable_hook_port = unavailable_hook.local_addr().unwrap().port();
        drop(unavailable_hook);

        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
        let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
        let _forward_url = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
            format!("http://127.0.0.1:{unavailable_hook_port}/internal/hook-live"),
        );
        let _forward_token = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "test-token");
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        let input = serde_json::json!({
            "prompt": "continue",
            "session_id": "agent-degraded-prompt",
            "cwd": worktree,
        })
        .to_string();

        // Isolate remote degradation from one-time Session/project state
        // materialization. The measured call still performs the real remote
        // Board request and unreachable hook-live notification.
        {
            let _profile_disabled = ScopedEnvVar::unset("GWT_HOOK_PROFILE_PATH");
            handle_with_input("UserPromptSubmit", &input, &worktree, Some(&session.id))
                .expect("warm local prompt state");
        }
        let _profile = ScopedEnvVar::set("GWT_HOOK_PROFILE_PATH", &profile_path);
        let provider: Rc<dyn BoardProvider> = Rc::new(SlackProvider::new_with_base(
            server.slack_api_base(),
            "board-token",
            "channel-1",
            BTreeMap::new(),
            Box::new(ReqwestHttpClient::new()),
            0,
        ));
        let _provider =
            crate::board_provider::test_provider_override::force_prompt_provider(provider);

        let started = Instant::now();
        let result = handle_with_input("UserPromptSubmit", &input, &worktree, Some(&session.id));
        let elapsed = started.elapsed();
        let records: Vec<Value> = std::fs::read_to_string(&profile_path)
            .expect("read hook profile")
            .lines()
            .map(|line| serde_json::from_str(line).expect("profile JSON"))
            .collect();
        let timing_summary = records
            .iter()
            .map(|record| {
                (
                    record["handler"].as_str().unwrap_or("<missing>"),
                    record["duration_ms"].as_f64().unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>();

        assert!(
            result.is_ok(),
            "degraded endpoints must fail open: {result:?}"
        );
        assert!(
            elapsed < Duration::from_millis(250),
            "degraded prompt must stay below 250ms, got {elapsed:?}: {timing_summary:?}"
        );
        assert!(
            server
                .collected_calls()
                .iter()
                .filter(|call| matches!(call, DegradedEndpointCall::BoardHistory))
                .count()
                <= 1
        );
        let total = records
            .iter()
            .find(|record| {
                record["event"] == "UserPromptSubmit" && record["handler"] == "event-total"
            })
            .expect("UserPromptSubmit event-total");
        assert_eq!(total["provider_read_count"], 1);
        assert_eq!(total["history_materialization_count"], 1);
        assert_eq!(
            records
                .iter()
                .filter(|record| record["handler"] == "runtime-state")
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record["handler"] == "forward")
                .count(),
            0
        );
    }

    #[test]
    fn warm_four_megabyte_history_user_prompt_submit_p95_stays_within_budget() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated HOME");
        let worktree = home.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("create worktree");
        init_git_repo(&worktree);
        let sessions_dir = home.path().join(".gwt/sessions");
        let mut session = Session::new(&worktree, "work/large-history", AgentId::Codex);
        session.agent_session_id = Some("agent-large-history".to_string());
        session.save(&sessions_dir).expect("save Session");
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session.id);

        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
        let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _profile = ScopedEnvVar::unset("GWT_HOOK_PROFILE_PATH");
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");

        let mut entry = BoardEntry::new(
            gwt_core::coordination::AuthorKind::Agent,
            "Fixture",
            BoardEntryKind::Status,
            "x".repeat(4 * 1024 * 1024),
            None,
            None,
            vec![],
            vec![],
        );
        entry.created_at = chrono::Utc::now() - chrono::Duration::hours(1);
        entry.updated_at = entry.created_at;
        gwt_core::coordination::post_entry(&worktree, entry).expect("seed 4 MiB Board history");

        let input = serde_json::json!({
            "prompt": "continue",
            "session_id": "agent-large-history",
            "cwd": worktree,
        })
        .to_string();
        handle_with_input("UserPromptSubmit", &input, &worktree, Some(&session.id))
            .expect("warm prompt read");

        let mut samples = (0..30)
            .map(|_| {
                let started = Instant::now();
                handle_with_input("UserPromptSubmit", &input, &worktree, Some(&session.id))
                    .expect("warm UserPromptSubmit");
                started.elapsed()
            })
            .collect::<Vec<_>>();
        samples.sort_unstable();
        let p95 = samples[28];
        assert!(
            p95 < Duration::from_millis(250),
            "warm 4 MiB UserPromptSubmit p95 must stay below 250ms, got {p95:?}: {samples:?}"
        );
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
        assert!(
            crate::cli::improvement::candidate_public_values(worktree.path()).is_empty(),
            "no auto-capture side effect may fire for the removed gate"
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
