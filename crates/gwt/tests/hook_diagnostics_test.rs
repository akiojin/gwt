//! Regression coverage for opt-in hook timing diagnostics.

use std::collections::BTreeSet;

use gwt::cli::{dispatch, TestEnv};
use gwt_agent::{runtime_state_path, AgentId, Session};
use serde_json::Value;

fn argv(strs: &[&str]) -> Vec<String> {
    strs.iter().map(std::string::ToString::to_string).collect()
}

#[test]
fn user_prompt_submit_profile_is_content_free_and_uses_exact_allowlist() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let profile_path = tmp.path().join("hook-profile.jsonl");
    let sessions_dir = tmp.path().join(".gwt/sessions");
    let session = Session::new(tmp.path(), "work/hook-metrics", AgentId::Codex);
    session.save(&sessions_dir).expect("save Session");
    let runtime_path = runtime_state_path(&sessions_dir, &session.id);

    let _home = ScopedEnvVar::set("HOME", tmp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
    let _profile = ScopedEnvVar::set("GWT_HOOK_PROFILE_PATH", &profile_path);
    let _gwt_session_id = ScopedEnvVar::set("GWT_SESSION_ID", &session.id);
    let _runtime_path = ScopedEnvVar::set("GWT_SESSION_RUNTIME_PATH", &runtime_path);
    let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
    let _forward_url = ScopedEnvVar::set(
        "GWT_HOOK_FORWARD_URL",
        "http://127.0.0.1:1/private-provider-url",
    );
    let _forward_token = ScopedEnvVar::set("GWT_HOOK_FORWARD_TOKEN", "private-hook-token");

    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.stdin = serde_json::json!({
        "prompt": "private-prompt-body",
        "session_id": "private-provider-session",
        "cwd": tmp.path()
    })
    .to_string();

    assert_eq!(
        dispatch(
            &mut env,
            &argv(&["gwt", "hook", "event", "UserPromptSubmit"]),
        ),
        0
    );
    let raw = std::fs::read_to_string(&profile_path).expect("profile jsonl");
    for sensitive in [
        "private-prompt-body",
        "private-provider-session",
        "private-provider-url",
        "private-hook-token",
        session.id.as_str(),
        tmp.path().to_string_lossy().as_ref(),
    ] {
        assert!(
            !raw.contains(sensitive),
            "profile leaked {sensitive:?}: {raw}"
        );
    }

    let records: Vec<Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).expect("valid profile json"))
        .collect();
    let handler_keys = BTreeSet::from(["duration_ms", "event", "handler", "occurred_at", "status"]);
    let total_keys = BTreeSet::from([
        "additional_context_bytes",
        "duration_ms",
        "event",
        "handler",
        "history_materialization_count",
        "occurred_at",
        "projection_load_count",
        "provider_read_count",
        "status",
    ]);
    for record in &records {
        if record["handler"] == "event-total" {
            assert_eq!(record_keys(record), total_keys);
            assert_eq!(record["provider_read_count"], 1);
            assert_eq!(record["history_materialization_count"], 1);
            assert!(record["projection_load_count"]
                .as_u64()
                .is_some_and(|v| v <= 2));
        } else {
            assert_eq!(record_keys(record), handler_keys);
        }
    }
}

fn env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

use gwt_core::test_support::ScopedEnvVar;

fn record_keys(record: &Value) -> BTreeSet<&str> {
    record
        .as_object()
        .expect("profile record must be an object")
        .keys()
        .map(String::as_str)
        .collect()
}

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
    let handler_keys = BTreeSet::from(["duration_ms", "event", "handler", "occurred_at", "status"]);
    for record in records
        .iter()
        .filter(|record| record["handler"] != "event-total")
    {
        assert_eq!(
            record_keys(record),
            handler_keys,
            "handler timing record must use the content-free allowlist: {record:?}"
        );
    }
}
