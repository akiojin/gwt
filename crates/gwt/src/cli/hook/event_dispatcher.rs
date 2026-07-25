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
    skill_plan_spec_stop_check, skill_register_spec_stop_check, work_event_settlement_stop_check,
    workflow_policy, workspace_identity, HookError, HookOutput, IntentBoundaryEvent,
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
    let pending_goal = run_value(event, "discussion-goal-start", || {
        load_pending_goal_for_hook_worktree(worktree_root, None)
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
    let prepared_session = run_step(event, "runtime-state", || {
        crate::daemon_runtime::handle_runtime_state_prepared(event, input)
    })?;
    // SPEC-2359 Phase W-11 (US-58): the workspace-identity step no longer
    // derives a title from the prompt; it only performs the Phase W-10
    // canonical Project State split repair. Fail-open so a repair error does
    // not abort prompt handling.
    run_value(event, "workspace-identity", || {
        let result = prepared_session
            .as_ref()
            .map(workspace_identity::handle_user_prompt_submit_for_session)
            .unwrap_or_else(|| workspace_identity::handle_user_prompt_submit(input));
        if let Err(error) = result {
            tracing::warn!(?error, "workspace-identity hook step failed");
        }
    });
    // SPEC-3248 P7A (FR-016): mark the intake artifact requirement dirty for
    // curation/producing prompts. Fail-open state writer.
    run_value(event, "intake-outcome-required-since", || {
        if let Some(session) = prepared_session.as_ref() {
            intake_completion_stop_check::handle_user_prompt_submit_for_resolved_worktree(
                &session.worktree_path,
                input,
            );
        } else {
            intake_completion_stop_check::handle_user_prompt_submit(worktree_root, input);
        }
    });
    let output = run_step(event, "board-reminder", || {
        if let Some(session) = prepared_session.as_ref() {
            board_reminder::handle_with_input_for_session(event, input, session)
        } else {
            board_reminder::handle_with_input(event, input)
        }
    })?;
    let pending_goal = run_value(event, "discussion-goal-start", || {
        load_pending_goal_for_hook_worktree(worktree_root, prepared_session.as_ref())
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
    let stop_checks: [(&str, StopCheck<'_>); 8] = [
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

fn additional_context_bytes(output: &HookOutput) -> usize {
    match output {
        HookOutput::HookSpecificAdditionalContext { text, .. } => text.len(),
        HookOutput::PreToolUsePermission { .. }
        | HookOutput::SystemMessage(_)
        | HookOutput::Silent
        | HookOutput::StopBlock { .. } => 0,
    }
}

fn load_pending_goal_for_hook_worktree(
    worktree_root: &Path,
    prepared_session: Option<&gwt_agent::Session>,
) -> Option<PendingDiscussionGoal> {
    if let Some(session) = prepared_session {
        return load_pending_goal_from_worktree_files(&session.worktree_path)
            .ok()
            .flatten();
    }
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
    use crate::board_remote::{http::ReqwestHttpClient, slack::SlackProvider};
    use crate::discussion_resume::PendingDiscussionGoal;
    use axum::{extract::State, routing::get, Json, Router};
    use gwt_agent::{AgentId, Session, GWT_SESSION_ID_ENV, GWT_SESSION_RUNTIME_PATH_ENV};
    use gwt_core::coordination::BoardProvider;
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

    #[test]
    fn only_user_prompt_submit_enters_the_aggregate_deadline() {
        assert!(
            gwt_core::operation_deadline::current().is_none(),
            "test must start without a scoped deadline"
        );
        let started = Instant::now();
        let guard = enter_event_deadline("UserPromptSubmit", started)
            .expect("UserPromptSubmit deadline guard");
        let deadline =
            gwt_core::operation_deadline::current().expect("aggregate deadline must be visible");
        assert_eq!(
            USER_PROMPT_SUBMIT_HOOK_DEADLINE,
            Duration::from_millis(200),
            "the deterministic hook budget must retain 50ms headroom below the 250ms profile gate"
        );
        assert!(deadline > started);
        assert!(deadline <= started + USER_PROMPT_SUBMIT_HOOK_DEADLINE);
        drop(guard);
        assert!(gwt_core::operation_deadline::current().is_none());

        assert!(enter_event_deadline("SessionStart", started).is_none());
        assert!(enter_event_deadline("PreToolUse", started).is_none());
        assert!(enter_event_deadline("PostToolUse", started).is_none());
        assert!(enter_event_deadline("Stop", started).is_none());
    }

    #[test]
    fn degraded_remote_board_and_hook_live_fail_open_within_prompt_budget() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated HOME");
        let worktree = home.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("create worktree");
        let sessions_dir = home.path().join(".gwt/sessions");
        let mut session = Session::new(&worktree, "work/degraded-prompt", AgentId::Codex);
        session.agent_session_id = Some("agent-degraded-prompt".to_string());
        session.save(&sessions_dir).expect("save Session");
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session.id);
        let profile_path = home.path().join("hook-profile.jsonl");
        let server = DegradedEndpointServer::start();
        let unavailable_hook =
            std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve unavailable hook port");
        let unavailable_hook_port = unavailable_hook
            .local_addr()
            .expect("unavailable hook address")
            .port();
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
        let _profile = ScopedEnvVar::set("GWT_HOOK_PROFILE_PATH", &profile_path);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");

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
        let input = serde_json::json!({
            "prompt": "continue",
            "session_id": "agent-degraded-prompt",
            "cwd": worktree,
        })
        .to_string();

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
            "degraded endpoints must fail open, got {result:?} after {elapsed:?}"
        );
        // This test runs inside the parallel full Rust suite, where the OS may
        // deschedule this thread while measuring wall time. Keep the exact
        // 200ms aggregate deadline deterministic above and use this live HTTP
        // fixture only as a scheduler-tolerant deadlock watchdog. The strict
        // 250ms user-facing gate is measured by the controlled 30-sample
        // checkout-local hook profile.
        assert!(
            elapsed < Duration::from_millis(500),
            "degraded prompt exceeded the deadlock watchdog, got {elapsed:?}: {timing_summary:?}"
        );

        let calls = server.collected_calls();
        let board_history_calls = calls
            .iter()
            .filter(|call| matches!(call, DegradedEndpointCall::BoardHistory))
            .count();
        assert!(
            board_history_calls <= 1,
            "remote history must materialize at most once: {calls:?}"
        );

        let total = records
            .iter()
            .find(|record| {
                record["event"] == "UserPromptSubmit" && record["handler"] == "event-total"
            })
            .expect("UserPromptSubmit event-total");
        assert_eq!(
            records
                .iter()
                .filter(|record| {
                    record["event"] == "UserPromptSubmit" && record["handler"] == "runtime-state"
                })
                .count(),
            1,
            "UserPromptSubmit must retain exactly one RuntimeState handler"
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| {
                    record["event"] == "UserPromptSubmit" && record["handler"] == "forward"
                })
                .count(),
            0,
            "UserPromptSubmit must not run the Forward handler"
        );
        assert_eq!(total["provider_read_count"], 1);
        assert_eq!(total["history_materialization_count"], 1);
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
    fn managed_user_prompt_uses_session_worktree_without_git_discovery() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let worktree = tempfile::tempdir().unwrap();
        let subdir = worktree.path().join("nested/agent");
        let empty_bin = worktree.path().join("empty-bin");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::create_dir_all(&empty_bin).unwrap();
        let _home = ScopedEnvVar::set("HOME", worktree.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", worktree.path());
        let sessions_dir = worktree.path().join(".gwt/sessions");
        let mut session = Session::new(
            worktree.path(),
            "work/prompt-fast-path",
            AgentId::ClaudeCode,
        );
        session.agent_session_id = Some("agent-fast-path".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
        let _runtime_env = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
        let _path = ScopedEnvVar::set("PATH", &empty_bin);
        write_pending_goal(worktree.path());

        let input = serde_json::json!({
            "prompt": "continue",
            "session_id": "agent-fast-path",
        })
        .to_string();
        let output = handle_with_input("UserPromptSubmit", &input, &subdir, Some(&session_id))
            .expect("managed prompt hook output");

        let HookOutput::HookSpecificAdditionalContext { event, text } = output else {
            panic!("expected pending goal context");
        };
        assert_eq!(event, IntentBoundaryEvent::UserPromptSubmit);
        assert!(
            text.contains("pending gwt-discussion Goal Start"),
            "managed UserPromptSubmit must use the prepared Session worktree instead of spawning git: {text}"
        );
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
            Some(0),
            "legacy intake capture must not fabricate a typed occurrence"
        );
        assert_eq!(
            candidates[0]
                .get("legacy_occurrence_count")
                .and_then(|v| v.as_u64()),
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
            Some(0),
            "legacy intake recapture must not fabricate typed occurrences"
        );
        assert_eq!(
            candidates[0]
                .get("legacy_occurrence_count")
                .and_then(|v| v.as_u64()),
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
}
