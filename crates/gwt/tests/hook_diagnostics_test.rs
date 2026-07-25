//! Regression coverage for opt-in hook timing diagnostics.

use std::collections::BTreeSet;

use gwt::cli::{dispatch, TestEnv};
use gwt_agent::{runtime_state_path, AgentId, Session};
use serde_json::Value;

fn argv(strs: &[&str]) -> Vec<String> {
    strs.iter().map(std::string::ToString::to_string).collect()
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
    let handler_keys = BTreeSet::from([
        "duration_ms",
        "event",
        "forward_url_set",
        "gwt_session_id",
        "handler",
        "occurred_at",
        "status",
    ]);
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

#[test]
fn user_prompt_submit_profiles_total_duration_and_additional_context_bytes_content_free() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let profile_path = tmp.path().join("hook-profile.jsonl");
    let discussion_path = tmp.path().join(".gwt/discussion.md");
    std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
    std::fs::write(
        &discussion_path,
        "## Discussion TODO\n\n\
         ### Proposal Sensitive - private-title [chosen]\n\
         - Goal Condition: private-board-body\n\
         - Goal State: pending\n",
    )
    .unwrap();
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
    let _forward_url = ScopedEnvVar::set("GWT_HOOK_FORWARD_URL", "private-hook-url");
    let _forward_token = ScopedEnvVar::set("GWT_HOOK_FORWARD_TOKEN", "private-hook-token");

    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.stdin = serde_json::json!({
        "prompt": "private-prompt-body",
        "session_id": "private-provider-session",
        "cwd": tmp.path()
    })
    .to_string();

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "hook", "event", "UserPromptSubmit"]),
    );

    assert_eq!(code, 0);
    let output: Value = serde_json::from_slice(&env.stdout).expect("hook output JSON");
    let additional_context = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("pending goal additionalContext");

    let raw = std::fs::read_to_string(&profile_path).expect("profile jsonl should be written");
    for sensitive in [
        "private-prompt-body",
        "private-provider-session",
        "private-title",
        "private-board-body",
        "private-hook-url",
        "private-hook-token",
        tmp.path().to_string_lossy().as_ref(),
    ] {
        assert!(
            !raw.contains(sensitive),
            "profile must not include sensitive hook content {sensitive:?}: {raw}"
        );
    }

    let records: Vec<Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).expect("valid profile json"))
        .collect();
    let total = records
        .iter()
        .find(|record| record["event"] == "UserPromptSubmit" && record["handler"] == "event-total")
        .expect("UserPromptSubmit event-total record");
    assert_eq!(total["status"], "ok");
    assert!(total["duration_ms"].as_f64().is_some());
    assert_eq!(total["additional_context_bytes"], additional_context.len());
    assert_eq!(total["provider_read_count"], 1);
    assert_eq!(total["history_materialization_count"], 1);
    assert!(
        total["projection_load_count"]
            .as_u64()
            .is_some_and(|count| count <= 2),
        "audience/canonical projections must each load at most once: {total:?}"
    );
    assert_eq!(
        record_keys(total),
        BTreeSet::from([
            "additional_context_bytes",
            "duration_ms",
            "event",
            "forward_url_set",
            "gwt_session_id",
            "handler",
            "history_materialization_count",
            "occurred_at",
            "projection_load_count",
            "provider_read_count",
            "status",
        ]),
        "event-total record must use the exact content-free allowlist"
    );
}
