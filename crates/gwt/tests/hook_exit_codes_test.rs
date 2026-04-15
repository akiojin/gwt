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

use gwt::cli::{dispatch, TestEnv};

fn argv(strs: &[&str]) -> Vec<String> {
    strs.iter().map(|s| s.to_string()).collect()
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
    // Set GWT_SESSION_RUNTIME_PATH so `handle` gets past the silent
    // no-op branch and actually calls `write_for_event`, which in turn
    // surfaces `InvalidEvent`.
    let tmp = tempfile::tempdir().unwrap();
    let runtime_path = tmp.path().join("runtime-state.json");
    let prev = std::env::var_os("GWT_SESSION_RUNTIME_PATH");
    std::env::set_var("GWT_SESSION_RUNTIME_PATH", &runtime_path);

    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let code = dispatch(
        &mut env,
        &argv(&["gwt", "hook", "runtime-state", "BogusEventName"]),
    );

    if let Some(v) = prev {
        std::env::set_var("GWT_SESSION_RUNTIME_PATH", v);
    } else {
        std::env::remove_var("GWT_SESSION_RUNTIME_PATH");
    }

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
}
