//! T-101 (SPEC #1942) — exit code contract tests.
//!
//! Pins the mapping from hook dispatch paths to process exit codes so
//! that a future refactor of `run_hook` cannot silently flip, for
//! example, an `InvalidEvent` failure from 1 to 2 and accidentally
//! surface it as a "block" to Claude Code.
//!
//! The exit code contract (from `specs/data-model.md`):
//!
//! - Unknown hook name                 → 2
//! - `runtime-state` missing <event>   → 2
//! - `runtime-state` invalid <event>   → 1
//! - `runtime-state` env unset (no-op) → 0
//! - Block hook returning `None`       → 0 (allow)
//! - Block hook returning `Some(..)`   → 2 (block + stdout JSON)

use chrono::Utc;
use gwt::cli::hook::{event_dispatcher, runtime_state::RuntimeState, HookOutput};
use gwt::cli::{dispatch, TestEnv};
use gwt_agent::{AgentId, Session, GWT_SESSION_ID_ENV, GWT_SESSION_RUNTIME_PATH_ENV};
use gwt_core::skill_state::{self, SkillState};

fn argv(strs: &[&str]) -> Vec<String> {
    strs.iter().map(std::string::ToString::to_string).collect()
}

fn env_test_lock() -> &'static gwt_core::test_support::EnvLock {
    gwt_core::test_support::env_lock()
}

use gwt_core::test_support::ScopedEnvVar;

#[test]
fn unknown_hook_name_exits_two() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let code = dispatch(&mut env, &argv(&["gwt", "hook", "no-such-thing"]));
    assert_eq!(code, 2);
    let err = String::from_utf8(env.stderr).unwrap();
    assert!(
        err.contains("unknown hook 'no-such-thing'"),
        "stderr: {err}"
    );
}

#[test]
fn runtime_state_missing_event_argument_exits_two() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let code = dispatch(&mut env, &argv(&["gwt", "hook", "runtime-state"]));
    assert_eq!(code, 2);
    let err = String::from_utf8(env.stderr).unwrap();
    assert!(err.contains("missing <event>"), "stderr: {err}");
}

#[test]
fn runtime_state_invalid_event_exits_one() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Set GWT_SESSION_RUNTIME_PATH so `handle` gets past the silent
    // no-op branch and actually calls `write_for_event`, which in turn
    // surfaces `InvalidEvent`.
    let tmp = tempfile::tempdir().unwrap();
    let runtime_path = tmp.path().join("runtime-state.json");
    let _runtime_path = ScopedEnvVar::set("GWT_SESSION_RUNTIME_PATH", &runtime_path);

    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let code = dispatch(
        &mut env,
        &argv(&["gwt", "hook", "runtime-state", "BogusEventName"]),
    );

    assert_eq!(code, 1, "InvalidEvent must map to exit code 1");
    let err = String::from_utf8(env.stderr).unwrap();
    assert!(
        err.contains("invalid hook event") || err.contains("BogusEventName"),
        "stderr should explain the invalid event, got: {err}"
    );
    assert!(
        !runtime_path.exists(),
        "InvalidEvent must not leave a partial runtime-state file"
    );
}

// The "env-unset → silent no-op → exit 0" path is covered by
// `cli_test::dispatch_hook_runtime_state_without_env_is_silent_ok`.
// We intentionally do not duplicate it here because tests in the same
// binary share the process environment and would race against
// `runtime_state_invalid_event_exits_one` (which sets
// `GWT_SESSION_RUNTIME_PATH`).

#[test]
fn block_hooks_with_empty_stdin_exit_zero() {
    // With no stdin payload, the consolidated block hook treats the call as
    // "no event to evaluate" → allow → exit 0. This pins that none
    // of them accidentally panic or return 2 on a missing payload.
    let tmp = tempfile::tempdir().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let code = dispatch(&mut env, &argv(&["gwt", "hook", "block-bash-policy"]));
    assert_eq!(
        code, 0,
        "block hook with empty stdin must exit 0, got {code}"
    );
}

#[test]
fn forward_hook_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let code = dispatch(&mut env, &argv(&["gwt", "hook", "forward"]));
    assert_eq!(code, 0, "forward stub must always allow (exit 0)");
    assert!(
        env.internal_command_call_log.is_empty(),
        "public hook dispatch must stay in-process on the hot path"
    );
}

#[test]
fn public_block_hook_preserves_block_json_contract_without_respawning() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.stdin = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "gh issue edit 123 --title updated"
        }
    })
    .to_string();

    let code = dispatch(&mut env, &argv(&["gwt", "hook", "block-bash-policy"]));

    assert_eq!(code, 2, "blocked hook must still exit 2");
    assert!(
        env.internal_command_call_log.is_empty(),
        "public hook dispatch must not spawn hidden daemon-hook for every hook"
    );

    let stdout = String::from_utf8(env.stdout).unwrap();
    assert!(
        stdout.contains("\"hookSpecificOutput\""),
        "stdout must emit hookSpecificOutput wrapper, got: {stdout}"
    );
    assert!(
        stdout.contains("\"hookEventName\":\"PreToolUse\""),
        "stdout must declare PreToolUse event, got: {stdout}"
    );
    assert!(
        stdout.contains("\"permissionDecision\":\"deny\""),
        "stdout must deny the tool, got: {stdout}"
    );
    assert!(
        stdout.contains("Direct GitHub workflow mutations are not allowed"),
        "short summary must remain in the visible reason: {stdout}"
    );
    assert!(
        stdout.contains("issue.edit"),
        "canonical gwt JSON operation alternative must be present in the visible reason: {stdout}"
    );
    assert!(
        !stdout.contains("\"decision\":\"block\""),
        "legacy decision:block output must not be emitted, got: {stdout}"
    );
    assert!(
        !stdout.contains("\"stopReason\""),
        "legacy stopReason output must not be emitted, got: {stdout}"
    );
}

#[test]
fn event_dispatcher_preserves_pre_tool_use_block_json_contract() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _runtime_path = ScopedEnvVar::unset("GWT_SESSION_RUNTIME_PATH");
    let _session_id = ScopedEnvVar::unset(GWT_SESSION_ID_ENV);
    let tmp = tempfile::tempdir().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.stdin = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "gh issue edit 123 --title updated"
        }
    })
    .to_string();

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "__internal", "daemon-hook", "event", "PreToolUse"]),
    );

    assert_eq!(code, 2, "blocked PreToolUse event must exit 2");
    let stdout = String::from_utf8(env.stdout).unwrap();
    assert!(
        stdout.contains("\"hookSpecificOutput\""),
        "stdout must emit hookSpecificOutput wrapper, got: {stdout}"
    );
    assert!(
        stdout.contains("\"hookEventName\":\"PreToolUse\""),
        "stdout must declare PreToolUse event, got: {stdout}"
    );
    assert!(
        stdout.contains("\"permissionDecision\":\"deny\""),
        "stdout must deny the tool, got: {stdout}"
    );
    assert!(
        stdout.lines().count() == 1,
        "event dispatcher must emit exactly one JSON line, got: {stdout}"
    );
    let stderr = String::from_utf8(env.stderr).unwrap();
    assert!(!stderr.contains("parked in NeedsHuman"), "{stderr}");
    assert!(
        !stderr.contains("Stop working on this Issue now"),
        "{stderr}"
    );
    // AC-3: a denial the agent can clear itself must say so, otherwise the
    // agent reads the bare gate reason as a park and stops (Issue #4488).
    let visible = stderr
        .lines()
        .next()
        .expect("gate reason")
        .chars()
        .take(256)
        .collect::<String>();
    assert!(visible.contains("does not park the Issue"), "{visible}");
}

/// Issue #3716: Grok treats exit 2 as a gate denial but reads the visible
/// reason from stderr rather than Claude's hookSpecificOutput envelope.
#[test]
fn grok_question_denial_exposes_the_handoff_reason_on_stderr() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("home");
    let worktree = home.path().join("repo");
    std::fs::create_dir_all(&worktree).expect("create worktree");
    let _home = ScopedEnvVar::set("HOME", home.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
    let _runtime_path = ScopedEnvVar::unset(GWT_SESSION_RUNTIME_PATH_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _autonomous = ScopedEnvVar::set(gwt::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV, "1");
    let _issue = ScopedEnvVar::set(gwt::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV, "3716");
    let mut session = Session::new(&worktree, "work/issue-3716", AgentId::GrokBuild);
    session.id = "grok-question-session".to_string();
    session.linked_issue_number = Some(3716);
    session
        .save(&gwt_core::paths::gwt_sessions_dir())
        .expect("save Grok Session");
    let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);

    let prefs_path = gwt::issue_monitor::issue_monitor_prefs_path_for_repo_path(&worktree);
    let mut prefs = gwt::IssueMonitorPrefs::default();
    let mut other = gwt::issue_monitor::AutonomousIssueRecord::new(4474);
    other.phase = gwt::issue_monitor::AutonomousPhase::NeedsHuman;
    prefs.autonomous_records.push(other);
    gwt::issue_monitor::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();

    let mut env = TestEnv::new(worktree);
    env.stdin = serde_json::json!({
        "hookEventName": "pre_tool_use",
        "sessionId": "native-grok-session",
        "toolName": "ask_user_question",
        "toolInput": {
            "questions": [{
                "question": "Proceed with the release?",
                "options": []
            }]
        }
    })
    .to_string();

    let question_input = env.stdin.clone();
    let code = dispatch(&mut env, &argv(&["gwt", "hook", "event", "PreToolUse"]));

    assert_eq!(code, 2, "Grok question must be denied before its UI opens");
    let stderr = String::from_utf8(env.stderr.clone()).expect("stderr UTF-8");
    let reason = stderr.lines().next().expect("Grok gate reason");
    let visible_reason = reason.chars().take(256).collect::<String>();
    assert!(
        visible_reason
            .starts_with(gwt::cli::hook::autonomous_question_guard::QUESTION_HANDOFF_SUMMARY),
        "Grok reads the first stderr line as its visible gate reason: {stderr:?}",
    );
    assert!(
        !visible_reason.contains("parked in NeedsHuman"),
        "another Issue's park and this pending handoff are not this owner's park: {visible_reason:?}",
    );
    let stdout = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(!stdout.contains("It is parked for a human"), "{stdout}");
    assert!(!stdout.contains("slot has been released"), "{stdout}");

    let mut prefs = gwt::issue_monitor::load_issue_monitor_prefs(&prefs_path).unwrap();
    let mut owner = gwt::issue_monitor::AutonomousIssueRecord::new(3716);
    owner.phase = gwt::issue_monitor::AutonomousPhase::NeedsHuman;
    owner.needs_human_kind = Some(gwt::issue_monitor::NeedsHumanKind::StrandedExecutionGeneration);
    prefs.autonomous_records.push(owner);
    gwt::issue_monitor::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
    env.stdout.clear();
    env.stderr.clear();
    env.stdin = question_input;
    assert_eq!(
        dispatch(&mut env, &argv(&["gwt", "hook", "event", "PreToolUse"])),
        2
    );
    let stderr = String::from_utf8(env.stderr).unwrap();
    let visible = stderr
        .lines()
        .next()
        .unwrap()
        .chars()
        .take(256)
        .collect::<String>();
    assert!(
        visible.contains("Issue #3716 is parked in NeedsHuman"),
        "{visible}"
    );
    // AC-3: the park branch names the human decision instead of leaving the
    // agent to infer it from the gate reason.
    assert!(visible.contains("a human must decide"), "{visible}");
    assert!(!visible.contains("does not park the Issue"), "{visible}");
    assert!(!visible.contains("human judgment is required"), "{visible}");
}

#[test]
fn event_dispatcher_non_blocking_events_are_silent_without_live_runtime() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _runtime_path = ScopedEnvVar::unset("GWT_SESSION_RUNTIME_PATH");
    let _session_id = ScopedEnvVar::unset("GWT_SESSION_ID");
    let tmp = tempfile::tempdir().unwrap();

    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "Stop",
    ] {
        let output =
            event_dispatcher::handle_with_input(event, "", tmp.path(), Some("sess-1")).unwrap();
        assert_eq!(output, HookOutput::Silent);
    }
}

#[test]
fn event_dispatcher_keeps_blocked_stop_runtime_state_running() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let sessions_dir = tmp.path().join(".gwt").join("sessions");
    let mut session = Session::new(tmp.path(), "feature/demo", AgentId::Codex);
    session.agent_session_id = Some("agent-123".to_string());
    let session_id = session.id.clone();
    session.save(&sessions_dir).unwrap();
    let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
    let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
    let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
    let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");

    skill_state::save(
        tmp.path(),
        "build-spec",
        &SkillState {
            start_evidence: None,
            active: true,
            owner_spec: Some(2077),
            started_at: Utc::now(),
            phase: Some("red".to_string()),
            session_id: session_id.clone(),
        },
    )
    .unwrap();

    let output = event_dispatcher::handle_with_input(
        "Stop",
        r#"{"session_id":"agent-123"}"#,
        tmp.path(),
        Some(&session_id),
    )
    .expect("blocked Stop should still dispatch");

    assert!(matches!(output, HookOutput::StopBlock { .. }));
    let runtime_raw = std::fs::read_to_string(&runtime_path).unwrap();
    let runtime_state: RuntimeState = serde_json::from_str(&runtime_raw).unwrap();
    assert_eq!(runtime_state.status, "Running");
    assert_eq!(runtime_state.source_event, "Stop");

    let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
    assert_eq!(
        serde_json::to_string(&loaded.status).unwrap(),
        "\"Running\""
    );
}

/// Regression for the user-visible symptom: Codex's managed PreToolUse /
/// PostToolUse hooks ran `gwtd hook event <event>` and exited with code 1 on
/// every tool call because the runtime-state step failed closed when the
/// payload carried no session_id (the shape Codex sends on tool-use events)
/// and `CODEX_THREAD_ID` was unset. The dispatcher must now fail open and
/// return `HookOutput::Silent` (exit code 0) while preserving the persisted
/// agent_session_id.
#[test]
fn event_dispatcher_codex_tool_use_fails_open_without_hook_session_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let sessions_dir = tmp.path().join(".gwt").join("sessions");
    let mut session = Session::new(tmp.path(), "feature/demo", AgentId::Codex);
    session.agent_session_id = Some("agent-existing".to_string());
    let session_id = session.id.clone();
    session.save(&sessions_dir).unwrap();
    let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
    let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
    let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
    let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");

    for event in ["PreToolUse", "PostToolUse"] {
        let output = event_dispatcher::handle_with_input(
            event,
            r#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#,
            tmp.path(),
            Some(&session_id),
        )
        .unwrap_or_else(|err| panic!("{event} must fail open, got {err:?}"));
        assert_eq!(output, HookOutput::Silent, "{event}");
    }

    // The id captured at SessionStart is preserved (placeholder never written).
    let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
    assert_eq!(loaded.agent_session_id.as_deref(), Some("agent-existing"));

    // Runtime state is still written so the Branches tab keeps tracking status.
    let runtime_raw = std::fs::read_to_string(&runtime_path).unwrap();
    let runtime_state: RuntimeState = serde_json::from_str(&runtime_raw).unwrap();
    assert_eq!(runtime_state.status, "Running");
}

#[test]
fn event_dispatcher_session_start_fails_closed_when_session_toml_is_corrupt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", tmp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
    let sessions_dir = tmp.path().join(".gwt").join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::write(
        sessions_dir.join("session-corrupt.toml"),
        "started_at = \"2026-06-16T04:30:00.\n320310Z\"",
    )
    .unwrap();
    let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "session-corrupt");
    let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-corrupt");
    let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);

    let error = event_dispatcher::handle_with_input(
        "SessionStart",
        r#"{"session_id":"agent-123"}"#,
        tmp.path(),
        Some("session-corrupt"),
    )
    .expect_err("SessionStart must reject provider identity that cannot be persisted");
    assert!(error.to_string().contains("runtime-state"), "{error}");
    assert!(
        !runtime_path.exists(),
        "failed identity must not publish Idle"
    );
}

#[test]
fn event_dispatcher_user_prompt_fails_open_when_session_toml_is_corrupt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _autonomous_execution =
        ScopedEnvVar::unset(gwt::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV);
    let _autonomous_issue = ScopedEnvVar::unset(gwt::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV);
    let tmp = tempfile::tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", tmp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
    let sessions_dir = tmp.path().join(".gwt").join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let session_path = sessions_dir.join("session-corrupt.toml");
    std::fs::write(&session_path, "odex\"").unwrap();
    let metadata = std::fs::read(&session_path).unwrap();
    let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "session-corrupt");
    let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-corrupt");
    let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);

    let output = event_dispatcher::handle_with_input(
        "UserPromptSubmit",
        r#"{"session_id":"agent-123"}"#,
        tmp.path(),
        Some("session-corrupt"),
    )
    .expect("corrupt session TOML must not make UserPromptSubmit exit 1");

    assert_eq!(output, HookOutput::Silent);
    assert_eq!(std::fs::read(&session_path).unwrap(), metadata);
    assert!(
        !runtime_path.exists(),
        "unknown source must not create a sidecar"
    );
}

#[test]
fn event_dispatcher_stop_fails_open_when_completed_stop_metadata_is_corrupt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", tmp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
    let sessions_dir = tmp.path().join(".gwt").join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let session_path = sessions_dir.join("session-corrupt.toml");
    std::fs::write(&session_path, "411253Z\"").unwrap();
    let metadata = std::fs::read(&session_path).unwrap();
    let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "session-corrupt");
    let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-corrupt");
    let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);

    let output =
        event_dispatcher::handle_with_input("Stop", r#"{}"#, tmp.path(), Some("session-corrupt"))
            .expect("corrupt session TOML must not make Stop completed-stop exit 1");

    assert_eq!(output, HookOutput::Silent);
    assert_eq!(std::fs::read(&session_path).unwrap(), metadata);
    assert!(
        !runtime_path.exists(),
        "unknown source must not create a sidecar"
    );
}

// SPEC #3245 AC-1/AC-4 (T-107): the lane mechanism is gone — a leftover
// `.gwt/session-kind.json` from a pre-removal worktree is inert. PreToolUse
// lets a production edit through and Stop does not block, with or without
// the stale file.
#[test]
fn stale_lane_file_does_not_alter_hook_dispatch_behavior() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _runtime_path = ScopedEnvVar::unset("GWT_SESSION_RUNTIME_PATH");
    let _session_id = ScopedEnvVar::unset("GWT_SESSION_ID");

    fn run(stale_lane_file: bool, event: &str, stdin: String) -> (i32, String) {
        let tmp = tempfile::tempdir().unwrap();
        if stale_lane_file {
            // SPEC #3245 FR-007: a leftover pre-removal lane file is inert.
            let lane = tmp.path().join(".gwt/session-kind.json");
            std::fs::create_dir_all(lane.parent().unwrap()).unwrap();
            std::fs::write(&lane, r#"{"version":1,"id":"intake"}"#).unwrap();
        }
        let mut env = TestEnv::new(tmp.path().join("cache"));
        env.repo_path = tmp.path().to_path_buf();
        env.stdin = stdin;
        let code = dispatch(
            &mut env,
            &argv(&["gwt", "__internal", "daemon-hook", "event", event]),
        );
        (code, String::from_utf8(env.stdout).unwrap())
    }

    let edit_stdin = serde_json::json!({
        "tool_name": "Edit",
        "tool_input": {
            "file_path": "crates/gwt/src/lib.rs",
            "old_string": "a",
            "new_string": "b"
        }
    })
    .to_string();
    let stop_stdin = serde_json::json!({
        "session_id": "agent-x",
        "stop_hook_active": false
    })
    .to_string();

    for stale_lane_file in [true, false] {
        let (code, stdout) = run(stale_lane_file, "PreToolUse", edit_stdin.clone());
        assert_eq!(
            code, 0,
            "a production edit must pass (stale_lane_file={stale_lane_file}): {stdout}"
        );
        assert!(
            !stdout.contains("\"permissionDecision\":\"deny\""),
            "no code-edit guard may fire (stale_lane_file={stale_lane_file}): {stdout}"
        );

        let (code, stdout) = run(stale_lane_file, "Stop", stop_stdin.clone());
        assert_ne!(
            code, 2,
            "Stop must not block (stale_lane_file={stale_lane_file}): {stdout}"
        );
        assert!(
            !stdout.contains("\"decision\":\"block\""),
            "no Stop gate may fire (stale_lane_file={stale_lane_file}): {stdout}"
        );
    }
}

// Issue #3165: output from an older Stop must not complete a newer Stop.
fn with_stop_completion_fixture(
    check: impl FnOnce(&mut TestEnv, &std::path::Path, &std::path::Path),
) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
    let _home = ScopedEnvVar::set("HOME", home.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _codex_thread = ScopedEnvVar::unset("CODEX_THREAD_ID");
    let _autonomous = ScopedEnvVar::unset(gwt::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV);
    let _issue = ScopedEnvVar::unset(gwt::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV);
    let worktree = home.path().join("repo");
    std::fs::create_dir_all(&worktree).unwrap();
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    assert!(sessions_dir.starts_with(home.path()));
    let mut session = Session::new(&worktree, "work/issue-3165", AgentId::Codex);
    session.agent_session_id = Some("provider-stop-3165".to_string());
    session.project_state_root = Some(worktree.clone());
    session.repo_hash = Some("repo-stop-3165".to_string());
    session.linked_issue_number = Some(3165);
    session
        .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: "repo-stop-3165".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3165,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-stop-3165".to_string(),
                binding_id: "binding-stop-3165".to_string(),
                ledger_head_hash: "head-stop-3165".to_string(),
            },
            capability_generation: 3,
        }))
        .unwrap();
    session.save(&sessions_dir).unwrap();
    let session_path = sessions_dir.join(format!("{}.toml", session.id));
    let saved = Session::load(&session_path).unwrap();
    let identity = gwt_agent::SessionExecutionIdentity::from_session(&saved)
        .unwrap()
        .unwrap();
    let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session.id);
    let process_id = std::process::id();
    let process_started_at = gwt::process::host_process_start_time(process_id)
        .expect("current test process start identity");
    gwt_agent::SessionRuntimeState::for_execution_process(
        gwt_agent::AgentStatus::Running,
        &identity,
        41,
        process_started_at,
        process_id,
        process_started_at,
    )
    .save(&runtime_path)
    .unwrap();
    let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
    let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
    skill_state::save(
        &worktree,
        "build-spec",
        &SkillState {
            start_evidence: None,
            active: true,
            owner_spec: Some(3165),
            started_at: Utc::now(),
            phase: Some("red".to_string()),
            session_id: session.id,
        },
    )
    .unwrap();
    let mut env = TestEnv::new(worktree);
    env.stdin = r#"{"session_id":"provider-stop-3165"}"#.to_string();
    check(&mut env, &runtime_path, &session_path);
}

struct StopProtocolWriter {
    before_first_write: Option<Box<dyn FnOnce() + Send>>,
    bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    fail: bool,
}

impl std::io::Write for StopProtocolWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(before) = self.before_first_write.take() {
            before();
        }
        if self.fail {
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        }
        self.bytes.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn assert_stop_completion_is_not_assigned_to_a_successor(stop_is_allowed: bool) {
    with_stop_completion_fixture(|env, runtime_path, session_path| {
        if stop_is_allowed {
            env.stdin =
                r#"{"session_id":"provider-stop-3165","stop_hook_active":true}"#.to_string();
        }
        let runtime = runtime_path.to_path_buf();
        let session_path = session_path.to_path_buf();
        let worktree = env.repo_path.clone();
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let b_bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_b = b_bytes.clone();
        env.stdout_writer = Some(Box::new(StopProtocolWriter {
            before_first_write: Some(Box::new(move || {
                let a = gwt_agent::SessionRuntimeState::load(&runtime).unwrap();
                let expected_status = if stop_is_allowed {
                    gwt_agent::AgentStatus::Idle
                } else {
                    gwt_agent::AgentStatus::Running
                };
                assert_eq!(a.status, expected_status);
                assert_eq!(a.source_event.as_deref(), Some("Stop"));
                assert!(a.last_completed_hook_event_at.is_none());
                let session = Session::load(&session_path).unwrap();
                assert_eq!(
                    session.agent_session_id.as_deref(),
                    Some("provider-stop-3165")
                );
                assert_eq!(session.last_completed_stop_at.is_some(), stop_is_allowed);
                let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
                    .unwrap()
                    .unwrap();
                assert_eq!(a.execution_identity.as_ref(), Some(&identity));
                assert_eq!(a.runtime_incarnation, Some(41));
                let process_id = std::process::id();
                let process_started_at = gwt::process::host_process_start_time(process_id)
                    .expect("current test process still has its start identity");
                assert_eq!(a.host_started_at, Some(process_started_at));
                assert_eq!(a.child_pid, Some(process_id));
                assert_eq!(a.child_started_at, Some(process_started_at));
                let output = event_dispatcher::handle_with_input(
                    "Stop",
                    r#"{"session_id":"provider-stop-3165","stop_hook_active":true}"#,
                    &worktree,
                    Some(&session.id),
                )
                .unwrap();
                assert!(matches!(output, HookOutput::SystemMessage(_)));
                let b = gwt_agent::SessionRuntimeState::load(&runtime).unwrap();
                assert_eq!(b.status, gwt_agent::AgentStatus::Idle);
                assert_eq!(b.source_event.as_deref(), Some("Stop"));
                assert!(b.last_completed_hook_event_at.is_none());
                assert_eq!(b.execution_identity, a.execution_identity);
                assert_eq!(b.runtime_incarnation, a.runtime_incarnation);
                assert_eq!(b.host_started_at, a.host_started_at);
                assert_eq!(b.child_pid, a.child_pid);
                assert_eq!(b.child_started_at, a.child_started_at);
                let a_observation = a.current_hook_event.as_ref().unwrap();
                let b_observation = b.current_hook_event.as_ref().unwrap();
                assert_ne!(a_observation.correlation_id, b_observation.correlation_id);
                assert_eq!(b_observation.event, "Stop");
                assert!(b_observation.completed_outcome.is_none());
                let stopped = Session::load(&session_path).unwrap();
                assert_eq!(stopped.agent_session_id, session.agent_session_id);
                assert_eq!(
                    gwt_agent::SessionExecutionIdentity::from_session(&stopped).unwrap(),
                    Some(identity)
                );
                assert_eq!(stopped.last_hook_event.as_deref(), Some("Stop"));
                assert!(stopped.last_completed_stop_at.is_some());
                *captured_b.lock().unwrap() = std::fs::read(&runtime).unwrap();
                // B has made its decision; no B protocol output is emitted.
            })),
            bytes: bytes.clone(),
            fail: false,
        }));
        let code = gwt::cli::hook::run_daemon_hook(env, "event", &argv(&["Stop"]))
            .expect("Stop dispatcher must succeed");
        assert_eq!(code, 0, "successful Stop protocol serialization exits zero");
        let output: serde_json::Value = serde_json::from_slice(&bytes.lock().unwrap()).unwrap();
        if stop_is_allowed {
            assert!(output["systemMessage"].is_string());
            assert!(output.get("decision").is_none());
        } else {
            assert_eq!(output["decision"], "block");
        }
        let b = gwt_agent::SessionRuntimeState::load(runtime_path).unwrap();
        assert!(
            b.last_completed_hook_event_at.is_none(),
            "A's output (allowed={stop_is_allowed}) must not complete B's un-emitted protocol: {b:?}"
        );
        assert_eq!(
            std::fs::read(runtime_path).unwrap(),
            *b_bytes.lock().unwrap(),
            "A's completion must leave B's publication unchanged"
        );
    });
}

#[test]
fn issue_3165_stop_completion_waits_for_its_own_protocol_output() {
    assert_stop_completion_is_not_assigned_to_a_successor(false);
}

#[test]
fn issue_3165_stop_completion_distinguishes_two_allowed_stops() {
    assert_stop_completion_is_not_assigned_to_a_successor(true);
}

#[test]
fn issue_3165_stop_completion_records_allowed_protocol_success() {
    with_stop_completion_fixture(|env, runtime_path, session_path| {
        env.stdin = r#"{"session_id":"provider-stop-3165","stop_hook_active":true}"#.to_string();
        let code = gwt::cli::hook::run_daemon_hook(env, "event", &argv(&["Stop"]))
            .expect("Stop dispatcher must succeed");
        assert_eq!(code, 0);
        let output: serde_json::Value = serde_json::from_slice(&env.stdout).unwrap();
        assert!(output["systemMessage"].is_string());
        assert!(output.get("decision").is_none());
        let runtime = gwt_agent::SessionRuntimeState::load(runtime_path).unwrap();
        assert_eq!(runtime.status, gwt_agent::AgentStatus::Idle);
        assert!(runtime.last_completed_hook_event_at.is_some());
        assert_eq!(
            runtime
                .current_hook_event
                .as_ref()
                .unwrap()
                .completed_outcome,
            Some(gwt_agent::session::HookCompletionOutcome::Normal)
        );
        assert!(Session::load(session_path)
            .unwrap()
            .last_completed_stop_at
            .is_some());
    });
}

#[test]
fn issue_3165_stop_completion_does_not_record_failed_protocol_output() {
    with_stop_completion_fixture(|env, runtime_path, _| {
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        env.stdout_writer = Some(Box::new(StopProtocolWriter {
            before_first_write: None,
            bytes: bytes.clone(),
            fail: true,
        }));
        let code = gwt::cli::hook::run_daemon_hook(env, "event", &argv(&["Stop"]))
            .expect("Stop dispatcher must succeed");
        assert_eq!(code, 1, "a failed nonempty StopBlock write must fail");
        assert!(bytes.lock().unwrap().is_empty());
        let runtime = gwt_agent::SessionRuntimeState::load(runtime_path).unwrap();
        assert_eq!(runtime.status, gwt_agent::AgentStatus::Running);
        assert!(runtime.last_completed_hook_event_at.is_none());
    });
}

#[test]
fn issue_3165_stop_completion_does_not_complete_before_cli_publication() {
    with_stop_completion_fixture(|env, runtime_path, _| {
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        env.stdout_writer = Some(Box::new(StopProtocolWriter {
            before_first_write: None,
            bytes: bytes.clone(),
            fail: true,
        }));
        let code = dispatch(
            env,
            &argv(&["gwt", "__internal", "daemon-hook", "event", "Stop"]),
        );
        assert_eq!(code, 1, "a failed nonempty StopBlock write must fail");
        assert!(bytes.lock().unwrap().is_empty());
        let runtime = gwt_agent::SessionRuntimeState::load(runtime_path).unwrap();
        assert_eq!(runtime.status, gwt_agent::AgentStatus::Running);
        assert!(runtime.last_completed_hook_event_at.is_none());
    });
}

#[test]
fn issue_3165_early_refusal_clears_normal_without_starting_tool_lifetime() {
    with_stop_completion_fixture(|env, runtime_path, session_path| {
        env.stdin = r#"{"session_id":"provider-stop-3165","stop_hook_active":true}"#.into();
        assert_eq!(
            gwt::cli::hook::run_daemon_hook(env, "event", &argv(&["Stop"])).unwrap(),
            0
        );
        let mut expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(runtime_path).unwrap()).unwrap();
        assert_eq!(
            expected["current_hook_event"]["completed_outcome"],
            "normal"
        );
        let session = std::fs::read(session_path).unwrap();
        env.stdout.clear();
        env.stdin = r#"{"session_id":"provider-stop-3165","tool_name":"Bash","tool_input":{"command":"gh issue edit 123 --title updated"}}"#.into();
        assert_eq!(
            gwt::cli::hook::run_daemon_hook(env, "event", &argv(&["PreToolUse"])).unwrap(),
            2
        );
        let output: serde_json::Value = serde_json::from_slice(&env.stdout).unwrap();
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
        let actual: serde_json::Value =
            serde_json::from_slice(&std::fs::read(runtime_path).unwrap()).unwrap();
        expected
            .as_object_mut()
            .unwrap()
            .remove("current_hook_event");
        expected["last_completed_hook_event_at"] = actual["last_completed_hook_event_at"].clone();
        expected["last_completed_hook_outcome"] = serde_json::json!("blocked");
        assert_eq!(
            actual, expected,
            "a refused tool changes only completion evidence"
        );
        assert_eq!(std::fs::read(session_path).unwrap(), session);
    });
}

#[test]
fn issue_3165_early_refusal_completion_cannot_change_a_newer_stop() {
    with_stop_completion_fixture(|env, runtime_path, session_path| {
        let runtime = runtime_path.to_path_buf();
        let metadata = session_path.to_path_buf();
        let worktree = env.repo_path.clone();
        let successor = std::sync::Arc::new(std::sync::Mutex::new((Vec::new(), Vec::new())));
        let captured = successor.clone();
        env.stdout_writer = Some(Box::new(StopProtocolWriter {
            before_first_write: Some(Box::new(move || {
                let session = Session::load(&metadata).unwrap();
                event_dispatcher::handle_with_input(
                    "Stop",
                    r#"{"session_id":"provider-stop-3165","stop_hook_active":true}"#,
                    &worktree,
                    Some(&session.id),
                )
                .unwrap();
                *captured.lock().unwrap() = (
                    std::fs::read(&runtime).unwrap(),
                    std::fs::read(&metadata).unwrap(),
                );
            })),
            bytes: Default::default(),
            fail: false,
        }));
        env.stdin = r#"{"session_id":"provider-stop-3165","tool_name":"Bash","tool_input":{"command":"gh issue edit 123 --title updated"}}"#.into();
        assert_eq!(
            gwt::cli::hook::run_daemon_hook(env, "event", &argv(&["PreToolUse"])).unwrap(),
            2
        );
        let successor = successor.lock().unwrap();
        assert!(!successor.0.is_empty());
        assert_eq!(std::fs::read(runtime_path).unwrap(), successor.0);
        assert_eq!(std::fs::read(session_path).unwrap(), successor.1);
    });
}
