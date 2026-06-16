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

fn env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
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
            "command": "gh issue view 123"
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
        stdout.contains("Direct GitHub workflow CLI commands are not allowed"),
        "short summary must remain in the visible reason: {stdout}"
    );
    assert!(
        stdout.contains("pr.view"),
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
    let tmp = tempfile::tempdir().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.stdin = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "gh issue view 123"
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
fn event_dispatcher_session_start_fails_open_when_session_toml_is_corrupt() {
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

    let output = event_dispatcher::handle_with_input(
        "SessionStart",
        r#"{"session_id":"agent-123"}"#,
        tmp.path(),
        Some("session-corrupt"),
    )
    .expect("corrupt session TOML must not make SessionStart exit 1");

    assert_eq!(output, HookOutput::Silent);
    let runtime_raw = std::fs::read_to_string(&runtime_path).unwrap();
    let runtime_state: RuntimeState = serde_json::from_str(&runtime_raw).unwrap();
    assert_eq!(runtime_state.status, "Idle");
    assert_eq!(runtime_state.source_event, "SessionStart");
}

#[test]
fn event_dispatcher_user_prompt_fails_open_when_session_toml_is_corrupt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", tmp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
    let sessions_dir = tmp.path().join(".gwt").join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::write(sessions_dir.join("session-corrupt.toml"), "odex\"").unwrap();
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
    let runtime_raw = std::fs::read_to_string(&runtime_path).unwrap();
    let runtime_state: RuntimeState = serde_json::from_str(&runtime_raw).unwrap();
    assert_eq!(runtime_state.status, "Running");
    assert_eq!(runtime_state.source_event, "UserPromptSubmit");
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
    std::fs::write(sessions_dir.join("session-corrupt.toml"), "411253Z\"").unwrap();
    let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "session-corrupt");
    let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-corrupt");
    let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);

    let output =
        event_dispatcher::handle_with_input("Stop", r#"{}"#, tmp.path(), Some("session-corrupt"))
            .expect("corrupt session TOML must not make Stop completed-stop exit 1");

    assert_eq!(output, HookOutput::Silent);
    let runtime_raw = std::fs::read_to_string(&runtime_path).unwrap();
    let runtime_state: RuntimeState = serde_json::from_str(&runtime_raw).unwrap();
    assert_eq!(runtime_state.status, "Idle");
    assert_eq!(runtime_state.source_event, "Stop");
}
