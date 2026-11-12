use std::{ffi::OsString, fs, sync::mpsc, time::Duration};

use axum::{
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use chrono::TimeZone;
use gwt::daemon_runtime::{
    handle_coordination_event, handle_forward, handle_runtime_state, HOOK_LIVE_ATTEMPT_BUDGET,
    HOOK_LIVE_READINESS_BUDGET,
};
use gwt_agent::{runtime_state_path, AgentId, Session};
use gwt_core::{deadline_budget::ScopedDeadlineBudget, paths::gwt_cache_dir};
use gwt_github::{CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt};
use serde_json::Value;
use tokio::{net::TcpListener, runtime::Runtime, sync::oneshot};

fn env_lock() -> gwt_core::test_support::EnvLockGuard {
    gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    fn new() -> Self {
        Self { saved: Vec::new() }
    }

    fn set(&mut self, key: &'static str, value: impl Into<OsString>) {
        if !self.saved.iter().any(|(saved, _)| *saved == key) {
            self.saved.push((key, std::env::var_os(key)));
        }
        std::env::set_var(key, value.into());
    }

    fn unset(&mut self, key: &'static str) {
        if !self.saved.iter().any(|(saved, _)| *saved == key) {
            self.saved.push((key, std::env::var_os(key)));
        }
        std::env::remove_var(key);
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        while let Some((key, value)) = self.saved.pop() {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

#[derive(Clone)]
struct CaptureState {
    tx: mpsc::Sender<(String, Value)>,
}

struct CaptureServer {
    // SPEC #4740: every test that starts a capture server asserts delivery.
    // Hook-live is fail-open, so an attempt that outlives its production
    // bounded attempt or SessionStart readiness budget drops the event silently.
    // Pin both to make the verdict independent of how slow the host is.
    _attempt_budget: ScopedDeadlineBudget,
    _readiness_budget: ScopedDeadlineBudget,
    runtime: Runtime,
    shutdown_tx: Option<oneshot::Sender<()>>,
    rx: mpsc::Receiver<(String, Value)>,
    url: String,
}

impl CaptureServer {
    fn start() -> Self {
        let runtime = Runtime::new().expect("tokio runtime");
        let listener = runtime
            .block_on(TcpListener::bind(("127.0.0.1", 0)))
            .expect("bind loopback listener");
        let addr = listener.local_addr().expect("listener addr");
        let (tx, rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let app = Router::new()
            .route("/internal/hook-live", post(capture_hook_live))
            .with_state(CaptureState { tx });

        runtime.spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            server.await.expect("capture server");
        });

        Self {
            _attempt_budget: ScopedDeadlineBudget::hang_guard(&HOOK_LIVE_ATTEMPT_BUDGET),
            _readiness_budget: ScopedDeadlineBudget::hang_guard(&HOOK_LIVE_READINESS_BUDGET),
            runtime,
            shutdown_tx: Some(shutdown_tx),
            rx,
            url: format!("http://127.0.0.1:{}/internal/hook-live", addr.port()),
        }
    }

    /// The delivered event. Emission is synchronous and the capture handler
    /// queues the event before it answers 2xx, so by the time the hook
    /// handler returns a delivered event is already queued: no wall-clock
    /// wait is involved. An empty queue means the fail-open emission failed.
    fn recv(&self) -> (String, Value) {
        self.rx
            .try_recv()
            .expect("expected hook live event to be delivered before the handler returned")
    }
}

impl Drop for CaptureServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.runtime
            .block_on(async { tokio::time::sleep(Duration::from_millis(25)).await });
    }
}

async fn capture_hook_live(
    headers: HeaderMap,
    State(state): State<CaptureState>,
    Json(body): Json<Value>,
) -> StatusCode {
    let auth = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    state.tx.send((auth, body)).expect("capture send");
    StatusCode::NO_CONTENT
}

fn sample_issue_snapshot(number: u64, title: &str, labels: Vec<&str>) -> IssueSnapshot {
    IssueSnapshot {
        author_login: None,
        number: IssueNumber(number),
        title: title.to_string(),
        body: String::new(),
        labels: labels.into_iter().map(str::to_string).collect(),
        state: IssueState::Open,
        updated_at: UpdatedAt::new(
            chrono::Utc
                .with_ymd_and_hms(2026, 4, 20, 0, 0, 0)
                .single()
                .unwrap()
                .to_rfc3339(),
        ),
        comments: Vec::<CommentSnapshot>::new(),
    }
}

#[test]
fn runtime_state_owner_forwards_live_event_to_loopback_target() {
    let _lock = env_lock();
    let mut env = EnvGuard::new();
    let home = tempfile::tempdir().unwrap();
    env.set("HOME", home.path().as_os_str().to_os_string());
    env.set("USERPROFILE", home.path().as_os_str().to_os_string());
    env.unset("CODEX_THREAD_ID");

    let sessions_dir = home.path().join(".gwt").join("sessions");
    let worktree = home.path().join("repo");
    fs::create_dir_all(&worktree).unwrap();

    let session = Session::new(&worktree, "feature/runtime-daemon", AgentId::Codex);
    session.save(&sessions_dir).unwrap();
    let runtime_path = runtime_state_path(&sessions_dir, &session.id);

    let server = CaptureServer::start();
    env.set("GWT_SESSION_ID", session.id.clone());
    env.set(
        "GWT_SESSION_RUNTIME_PATH",
        runtime_path.as_os_str().to_os_string(),
    );
    env.set("GWT_HOOK_FORWARD_URL", server.url.clone());
    env.set("GWT_HOOK_FORWARD_TOKEN", "secret-token");

    let input = serde_json::json!({
        "session_id": "agent-session-1",
        "cwd": worktree.display().to_string(),
        "tool_name": "Bash",
        "tool_input": {
            "command": "git status"
        }
    })
    .to_string();
    handle_runtime_state("PreToolUse", &input).unwrap();

    let (auth, payload) = server.recv();
    assert_eq!(auth, "Bearer secret-token");
    assert_eq!(payload["kind"], "runtime_state");
    assert_eq!(payload["source_event"], "PreToolUse");
    assert_eq!(payload["gwt_session_id"], session.id);
    assert_eq!(payload["agent_session_id"], "agent-session-1");
    assert_eq!(payload["branch"], "feature/runtime-daemon");
    assert_eq!(payload["status"], "Running");

    // Issue #4905: losing runtime-state must not lose the transition when
    // the next successful handler is Forward (which has no status itself).
    env.set("GWT_HOOK_FORWARD_URL", "unsupported://local");
    handle_runtime_state("Stop", &input).unwrap();
    let health = gwt::daemon_runtime::hook_live_delivery_health(&runtime_path).unwrap();
    assert_eq!(health.consecutive_failures, 1);
    assert!(health.pending_runtime_state);

    env.set("GWT_HOOK_FORWARD_URL", server.url.clone());
    handle_forward(&input).unwrap();
    let (_, recovered) = server.recv();
    assert_eq!(recovered["kind"], "runtime_state");
    assert_eq!(recovered["source_event"], "Stop");
    let recovered: gwt::RuntimeHookEvent = serde_json::from_value(recovered).unwrap();
    assert_eq!(
        gwt::window_state::runtime_hook_window_state(&recovered),
        Some(gwt::WindowState::Idle)
    );
    assert_eq!(server.recv().1["kind"], "forward");
    let health = gwt::daemon_runtime::hook_live_delivery_health(&runtime_path).unwrap();
    assert_eq!(health.consecutive_failures, 0);
    assert!(!health.pending_runtime_state);
}

#[test]
fn user_prompt_submit_journal_wait_preserves_transport_budget_and_ack() {
    use gwt::daemon_runtime::USER_PROMPT_SUBMIT_HOOK_LIVE_BUDGET;
    use gwt_core::{
        deadline_budget::HANG_GUARD, operation_deadline::ScopedOperationDeadline,
        test_support::ScopedGwtHome,
    };
    use std::time::Instant;

    let _lock = env_lock();
    let mut env = EnvGuard::new();
    let home = tempfile::tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(home.path().join(".gwt"));
    env.set("HOME", home.path().as_os_str().to_os_string());
    env.set("USERPROFILE", home.path().as_os_str().to_os_string());
    env.unset("CODEX_THREAD_ID");
    let sessions_dir = home.path().join(".gwt/sessions");
    let worktree = home.path().join("repo");
    fs::create_dir_all(&worktree).unwrap();
    let session = Session::new(&worktree, "work/issue-5542", AgentId::Codex);
    session.save(&sessions_dir).unwrap();
    let runtime_path = runtime_state_path(&sessions_dir, &session.id);
    let journal_path = sessions_dir
        .join("hook-live")
        .join(runtime_path.parent().unwrap().file_name().unwrap())
        .join(runtime_path.file_name().unwrap());
    fs::create_dir_all(journal_path.parent().unwrap()).unwrap();
    let journal_lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(journal_path.with_extension("lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&journal_lock).unwrap();

    let server = CaptureServer::start();
    let profile_path = home.path().join("hook-profile.jsonl");
    env.set("GWT_SESSION_ID", session.id.clone());
    env.set("GWT_SESSION_RUNTIME_PATH", runtime_path.as_os_str());
    env.set("GWT_HOOK_FORWARD_URL", server.url.clone());
    env.set("GWT_HOOK_FORWARD_TOKEN", "secret-token");
    env.set("GWT_HOOK_PROFILE_PATH", profile_path.as_os_str());
    let input = serde_json::json!({
        "session_id": "provider-session-5542",
        "cwd": worktree,
        "prompt": "continue the current work",
    })
    .to_string();
    const LIVE_BUDGET: Duration = Duration::from_secs(1);
    let worker = std::thread::spawn(move || {
        let _attempt = ScopedDeadlineBudget::hang_guard(&HOOK_LIVE_ATTEMPT_BUDGET);
        let _readiness = ScopedDeadlineBudget::hang_guard(&HOOK_LIVE_READINESS_BUDGET);
        let _live = ScopedDeadlineBudget::pin(&USER_PROMPT_SUBMIT_HOOK_LIVE_BUDGET, LIVE_BUDGET);
        let _aggregate = ScopedOperationDeadline::enter(
            // test-hygiene: allow-wall-clock-deadline Exercise the real journal/transport boundary while the outer deadline is only a hang guard.
            Instant::now() + HANG_GUARD,
        );
        handle_runtime_state("UserPromptSubmit", &input).map_err(|error| error.to_string())
    });

    // Observe journal preparation before holding it past the local transport
    // budget. Loaded runners may take longer to reach this phase; elapsed time
    // before the phase is not evidence that the journal caused the delay.
    let waiting_started = Instant::now();
    let journal_prepared = loop {
        let prepared = fs::read_to_string(&profile_path).is_ok_and(|profile| {
            profile.lines().any(|line| {
                serde_json::from_str::<Value>(line)
                    .is_ok_and(|record| record["handler"] == "hook-live/journal-prepare")
            })
        });
        // test-hygiene: allow-wall-clock-deadline Hang guard only; the observed journal phase triggers release timing.
        if prepared || worker.is_finished() || waiting_started.elapsed() >= Duration::from_secs(30)
        {
            break prepared;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if journal_prepared {
        let held_since_prepare = Instant::now();
        // test-hygiene: allow-wall-clock-deadline Prove the real journal wait outlives the transport cap before releasing it.
        while held_since_prepare.elapsed() <= LIVE_BUDGET {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    // Always release the owned lock and join before assertions or environment
    // restoration, including when preparation failed to reach the journal.
    drop(journal_lock);
    let outcome = worker.join().expect("hook worker");
    assert!(journal_prepared, "hook did not reach journal preparation");
    outcome.expect("runtime-state hook remains fail-open");

    let (auth, payload) = server.recv();
    assert_eq!(auth, "Bearer secret-token");
    assert_eq!(payload["source_event"], "UserPromptSubmit");
    assert_eq!(payload["gwt_session_id"], session.id);
    assert_eq!(payload["agent_session_id"], "provider-session-5542");
    let health = gwt::daemon_runtime::hook_live_delivery_health(&runtime_path).unwrap();
    assert_eq!(health.consecutive_failures, 0);
    assert!(
        health.last_success_at.is_some(),
        "POST must receive its ACK"
    );
    assert!(!health.pending_runtime_state);
    let journal: Value = serde_json::from_slice(&fs::read(journal_path).unwrap()).unwrap();
    assert!(journal["pending"].is_null());
}

#[test]
fn coordination_event_owner_forwards_live_event_to_loopback_target() {
    let _lock = env_lock();
    let mut env = EnvGuard::new();
    let home = tempfile::tempdir().unwrap();
    env.set("HOME", home.path().as_os_str().to_os_string());
    env.set("USERPROFILE", home.path().as_os_str().to_os_string());
    env.unset("CODEX_THREAD_ID");

    let sessions_dir = home.path().join(".gwt").join("sessions");
    let worktree = home.path().join("repo");
    fs::create_dir_all(&worktree).unwrap();

    let mut session = Session::new(&worktree, "feature/runtime-daemon", AgentId::Codex);
    session.linked_issue_number = Some(1989);
    session.save(&sessions_dir).unwrap();

    let cache_root = gwt_cache_dir().join("issues").join("__detached__");
    let cache = gwt_github::Cache::new(cache_root);
    cache
        .write_snapshot(&sample_issue_snapshot(
            1989,
            "Runtime coordination fanout",
            vec!["gwt-spec"],
        ))
        .unwrap();

    let server = CaptureServer::start();
    env.set("GWT_SESSION_ID", session.id.clone());
    env.set(
        "GWT_SESSION_RUNTIME_PATH",
        runtime_state_path(&sessions_dir, &session.id)
            .as_os_str()
            .to_os_string(),
    );
    env.set("GWT_HOOK_FORWARD_URL", server.url.clone());
    env.set("GWT_HOOK_FORWARD_TOKEN", "secret-token");

    handle_coordination_event("SessionStart", "{}").unwrap();

    let (auth, payload) = server.recv();
    assert_eq!(auth, "Bearer secret-token");
    assert_eq!(payload["kind"], "coordination_event");
    assert_eq!(payload["source_event"], "SessionStart");
    assert_eq!(payload["gwt_session_id"], session.id);
    assert_eq!(payload["branch"], "feature/runtime-daemon");
}

#[test]
fn forward_owner_forwards_live_event_to_loopback_target() {
    let _lock = env_lock();
    let mut env = EnvGuard::new();
    let home = tempfile::tempdir().unwrap();
    env.set("HOME", home.path().as_os_str().to_os_string());
    env.set("USERPROFILE", home.path().as_os_str().to_os_string());
    env.unset("CODEX_THREAD_ID");

    let sessions_dir = home.path().join(".gwt").join("sessions");
    let worktree = home.path().join("repo");
    fs::create_dir_all(&worktree).unwrap();

    let session = Session::new(&worktree, "feature/runtime-daemon", AgentId::Codex);
    session.save(&sessions_dir).unwrap();

    let server = CaptureServer::start();
    env.set("GWT_SESSION_ID", session.id.clone());
    env.set(
        "GWT_SESSION_RUNTIME_PATH",
        runtime_state_path(&sessions_dir, &session.id)
            .as_os_str()
            .to_os_string(),
    );
    env.set("GWT_HOOK_FORWARD_URL", server.url.clone());
    env.set("GWT_HOOK_FORWARD_TOKEN", "secret-token");

    let input = serde_json::json!({
        "session_id": "agent-session-2",
        "cwd": worktree.display().to_string(),
        "tool_name": "Bash",
        "tool_input": {
            "command": "echo daemon"
        }
    })
    .to_string();
    handle_forward(&input).unwrap();

    let (auth, payload) = server.recv();
    assert_eq!(auth, "Bearer secret-token");
    assert_eq!(payload["kind"], "forward");
    assert_eq!(payload["gwt_session_id"], session.id);
    assert_eq!(payload["agent_session_id"], "agent-session-2");
    assert_eq!(payload["tool_name"], "Bash");
}

#[test]
fn forward_owner_uses_persisted_agent_session_id_when_hook_session_id_is_missing() {
    let _lock = env_lock();
    let mut env = EnvGuard::new();
    let home = tempfile::tempdir().unwrap();
    env.set("HOME", home.path().as_os_str().to_os_string());
    env.set("USERPROFILE", home.path().as_os_str().to_os_string());
    env.unset("CODEX_THREAD_ID");

    let sessions_dir = home.path().join(".gwt").join("sessions");
    let worktree = home.path().join("repo");
    fs::create_dir_all(&worktree).unwrap();

    let mut session = Session::new(&worktree, "feature/runtime-daemon", AgentId::Codex);
    session.agent_session_id = Some("persisted-agent-session".to_string());
    session.save(&sessions_dir).unwrap();

    let server = CaptureServer::start();
    env.set("GWT_SESSION_ID", session.id.clone());
    env.set(
        "GWT_SESSION_RUNTIME_PATH",
        runtime_state_path(&sessions_dir, &session.id)
            .as_os_str()
            .to_os_string(),
    );
    env.set("GWT_HOOK_FORWARD_URL", server.url.clone());
    env.set("GWT_HOOK_FORWARD_TOKEN", "secret-token");

    let input = serde_json::json!({
        "cwd": worktree.display().to_string(),
        "tool_name": "Bash",
        "tool_input": {
            "command": "echo daemon"
        }
    })
    .to_string();
    handle_forward(&input).unwrap();

    let (_auth, payload) = server.recv();
    assert_eq!(payload["kind"], "forward");
    assert_eq!(payload["gwt_session_id"], session.id);
    assert_eq!(payload["agent_session_id"], "persisted-agent-session");
}

#[test]
fn forward_owner_is_fail_open_without_live_target_env() {
    let _lock = env_lock();
    let mut env = EnvGuard::new();
    env.unset("GWT_SESSION_ID");
    env.unset("GWT_SESSION_RUNTIME_PATH");
    env.unset("CODEX_THREAD_ID");
    env.unset("GWT_HOOK_FORWARD_URL");
    env.unset("GWT_HOOK_FORWARD_TOKEN");

    let input = serde_json::json!({
        "session_id": "agent-session-missing-target",
        "cwd": "E:/gwt/feature/gwt-cli",
        "tool_name": "Bash",
        "tool_input": {
            "command": "echo ok"
        }
    })
    .to_string();

    handle_forward(&input).expect("missing live target env must fail open");
}

#[test]
fn forward_owner_is_fail_open_when_live_target_is_unreachable() {
    let _lock = env_lock();
    let mut env = EnvGuard::new();
    let home = tempfile::tempdir().expect("isolated hook home");
    let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
    env.unset("GWT_SESSION_ID");
    env.unset("GWT_SESSION_RUNTIME_PATH");
    env.unset("CODEX_THREAD_ID");
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve loopback port");
    let port = listener.local_addr().expect("listener addr").port();
    drop(listener);
    env.set(
        "GWT_HOOK_FORWARD_URL",
        format!("http://127.0.0.1:{port}/internal/hook-live"),
    );
    env.set("GWT_HOOK_FORWARD_TOKEN", "secret-token");

    let input = serde_json::json!({
        "session_id": "agent-session-unreachable-target",
        "cwd": "E:/gwt/feature/gwt-cli",
        "tool_name": "Bash",
        "tool_input": {
            "command": "echo ok"
        }
    })
    .to_string();

    handle_forward(&input).expect("unreachable live target must fail open");
}

/// Issue #3541 AC-6: live forwarding stays fail-open, but the transport
/// failure must still leave a sanitized ledger row that names the event.
#[test]
fn live_forward_failure_is_fail_open_and_recorded_in_the_error_ledger() {
    use gwt::daemon_runtime::handle_forward_for_event;
    use gwt_core::error_ledger::ErrorKind;

    let _lock = env_lock();
    let mut env = EnvGuard::new();
    let home = tempfile::tempdir().expect("isolated home");
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path().join(".gwt"));
    let worktree = home.path().join("repo");
    fs::create_dir_all(&worktree).expect("worktree");
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve loopback port");
    let port = listener.local_addr().expect("listener addr").port();
    drop(listener);

    const TOKEN_SENTINEL: &str = "LIVE_FORWARD_TOKEN_MUST_NOT_PERSIST_3541";
    const PAYLOAD_SENTINEL: &str = "LIVE_FORWARD_PAYLOAD_MUST_NOT_PERSIST_3541";
    const SESSION_ID: &str = "session-3541-live-forward";
    env.set("GWT_SESSION_ID", SESSION_ID);
    env.set(
        "GWT_HOOK_FORWARD_URL",
        format!("http://127.0.0.1:{port}/internal/hook-live"),
    );
    env.set("GWT_HOOK_FORWARD_TOKEN", TOKEN_SENTINEL);
    env.unset("GWT_SESSION_RUNTIME_PATH");
    env.unset("GWT_HOOK_PROFILE_PATH");
    env.unset("CODEX_THREAD_ID");

    let input = serde_json::json!({
        "session_id": "provider-session-sensitive",
        "cwd": worktree.display().to_string(),
        "prompt": PAYLOAD_SENTINEL,
        "tool_name": "Bash",
        "tool_input": { "command": format!("echo {PAYLOAD_SENTINEL}") }
    })
    .to_string();

    handle_forward_for_event("PreToolUse", &input)
        .expect("live forwarding transport failure must remain fail-open");

    let rows = gwt_core::error_ledger::list_since(None).expect("error ledger");
    let row = rows
        .iter()
        .find(|row| {
            row.kind == ErrorKind::HookFailure
                && row.context.get("fail_open").map(String::as_str) == Some("true")
        })
        .unwrap_or_else(|| panic!("fail-open transport failure must be recorded: {rows:?}"));
    assert_eq!(
        row.context.get("event").map(String::as_str),
        Some("PreToolUse")
    );
    assert!(
        row.context
            .get("handler")
            .is_some_and(|handler| handler.contains("forward")),
        "failure must identify the forwarding handler: {row:?}"
    );
    assert_eq!(
        row.context.get("exit_status").map(String::as_str),
        Some("0")
    );
    assert_eq!(row.target.session_id.as_deref(), Some(SESSION_ID));

    let ledger_text = serde_json::to_string(&rows).expect("ledger json");
    for sentinel in [
        TOKEN_SENTINEL,
        PAYLOAD_SENTINEL,
        "provider-session-sensitive",
    ] {
        assert!(
            !ledger_text.contains(sentinel),
            "sensitive input leaked into the ledger: {ledger_text}"
        );
    }
}
