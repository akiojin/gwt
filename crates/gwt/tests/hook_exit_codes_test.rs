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

use gwt::cli::hook::{event_dispatcher, HookOutput};
use gwt::cli::{dispatch, TestEnv};

fn argv(strs: &[&str]) -> Vec<String> {
    strs.iter().map(std::string::ToString::to_string).collect()
}

fn env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

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
        stdout.contains("gwtd pr view"),
        "canonical gwt alternative must be present in the visible reason: {stdout}"
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
