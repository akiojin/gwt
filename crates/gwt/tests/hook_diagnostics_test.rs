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

/// Issue #3541 AC-1 / AC-3 / AC-4: a failing handler inside `gwtd hook event`
/// must leave a durable, sanitized ledger row that names the event and the
/// handler, and the user-visible error must say where that evidence lives
/// and that nobody has been told yet.
#[test]
fn hook_handler_failure_is_recorded_in_the_error_ledger_with_handler_context() {
    use gwt_agent::{runtime_state_path, AgentId, Session};
    use gwt_core::error_ledger::ErrorKind;

    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("isolated home");
    let gwt_home = home.path().join(".gwt");
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(&gwt_home);
    let worktree = home.path().join("repo");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let sessions_dir = gwt_home.join("sessions");
    let mut session = Session::new(&worktree, "work/issue-3541", AgentId::Codex);
    session.linked_issue_number = Some(3541);
    session.save(&sessions_dir).expect("session metadata");
    let runtime_path = runtime_state_path(&sessions_dir, &session.id);
    // A directory where the runtime-state file belongs makes the
    // `runtime-state` handler fail with an I/O error.
    std::fs::create_dir_all(&runtime_path).expect("invalid runtime-state destination");

    let _home = ScopedEnvVar::set("HOME", home.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
    let _gwt_session_id = ScopedEnvVar::set("GWT_SESSION_ID", &session.id);
    let _runtime_path = ScopedEnvVar::set("GWT_SESSION_RUNTIME_PATH", &runtime_path);
    let _profile_path = ScopedEnvVar::unset("GWT_HOOK_PROFILE_PATH");
    let _forward_url = ScopedEnvVar::unset("GWT_HOOK_FORWARD_URL");
    let _forward_token = ScopedEnvVar::unset("GWT_HOOK_FORWARD_TOKEN");
    let _codex_thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");

    const PROMPT_SENTINEL: &str = "PROMPT_BODY_MUST_NOT_BE_RETAINED_3541";
    const TOKEN_SENTINEL: &str = "BEARER_TOKEN_MUST_NOT_BE_RETAINED_3541";
    let mut env = TestEnv::new(worktree.clone());
    env.stdin = serde_json::json!({
        "session_id": "provider-session-sensitive",
        "cwd": worktree,
        "prompt": PROMPT_SENTINEL,
        "authorization": format!("Bearer {TOKEN_SENTINEL}"),
        "tool_name": "Bash",
        "tool_input": { "command": format!("echo {PROMPT_SENTINEL}") }
    })
    .to_string();

    let code = dispatch(&mut env, &argv(&["gwtd", "hook", "event", "PreToolUse"]));

    assert_eq!(code, 1, "handler failure must keep hook exit status 1");
    assert!(
        env.stdout.is_empty(),
        "failed hook must keep protocol stdout silent: {}",
        String::from_utf8_lossy(&env.stdout)
    );

    let rows = gwt_core::error_ledger::list_since(None).expect("error ledger");
    let row = rows
        .iter()
        .find(|row| row.kind == ErrorKind::HookFailure)
        .unwrap_or_else(|| panic!("handler failure must land in the error ledger: {rows:?}"));
    assert_eq!(
        row.context.get("event").map(String::as_str),
        Some("PreToolUse")
    );
    assert_eq!(
        row.context.get("handler").map(String::as_str),
        Some("runtime-state")
    );
    assert_eq!(
        row.context.get("exit_status").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        row.context.get("fail_open").map(String::as_str),
        Some("false")
    );
    assert_eq!(row.target.session_id.as_deref(), Some(session.id.as_str()));
    assert_eq!(
        row.target.project_root.as_deref(),
        Some(worktree.display().to_string().as_str())
    );
    assert_eq!(row.target.issue, Some(3541));
    assert!(
        !row.message.trim().is_empty(),
        "ledger row must carry a sanitized error message"
    );

    let stderr = String::from_utf8(env.stderr).expect("UTF-8 stderr");
    for expected in [
        "PreToolUse",
        "runtime-state",
        "errors.list",
        "report_status=not_sent",
        "Board",
        "Issue #3541",
    ] {
        assert!(
            stderr.contains(expected),
            "stderr must identify {expected:?}: {stderr}"
        );
    }
    let ledger_text = serde_json::to_string(&rows).expect("ledger json");
    for output in [&ledger_text, &stderr] {
        assert!(!output.contains(PROMPT_SENTINEL), "prompt leaked: {output}");
        assert!(!output.contains(TOKEN_SENTINEL), "token leaked: {output}");
        assert!(
            !output.contains("provider-session-sensitive"),
            "raw provider payload leaked: {output}"
        );
    }
}
