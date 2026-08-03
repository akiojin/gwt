//! Regression coverage for opt-in hook timing diagnostics.

use gwt::cli::{dispatch, TestEnv};
use serde_json::Value;

fn argv(strs: &[&str]) -> Vec<String> {
    strs.iter().map(std::string::ToString::to_string).collect()
}

fn env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

use gwt_core::test_support::ScopedEnvVar;

#[test]
fn hook_event_writes_opt_in_handler_timing_without_stdout_noise() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let profile_path = tmp.path().join("hook-profile.jsonl");
    let _profile = ScopedEnvVar::set("GWT_HOOK_PROFILE_PATH", &profile_path);
    let _gwt_session_id = ScopedEnvVar::unset("GWT_SESSION_ID");
    let _runtime_path = ScopedEnvVar::unset("GWT_SESSION_RUNTIME_PATH");
    let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");

    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.stdin = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "pwd"
        },
        "session_id": "agent-session",
        "cwd": tmp.path()
    })
    .to_string();

    let code = dispatch(&mut env, &argv(&["gwt", "hook", "event", "PreToolUse"]));

    assert_eq!(code, 0);
    assert!(
        env.stdout.is_empty(),
        "allowed PreToolUse hook must not emit stdout JSON, got: {}",
        String::from_utf8_lossy(&env.stdout)
    );

    let raw = std::fs::read_to_string(&profile_path).expect("profile jsonl should be written");
    let records: Vec<Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).expect("valid profile json"))
        .collect();
    assert!(
        records.iter().any(|record| record["event"] == "PreToolUse"
            && record["handler"] == "runtime-state"
            && record["status"] == "ok"),
        "expected runtime-state timing record, got: {records:?}"
    );
    assert!(
        records
            .iter()
            .any(|record| record["handler"] == "workflow-policy"),
        "expected workflow-policy timing record, got: {records:?}"
    );
    assert!(
        records
            .iter()
            .all(|record| record["duration_ms"].as_f64().is_some()),
        "every timing record must include duration_ms, got: {records:?}"
    );
}
