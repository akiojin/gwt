use std::{
    ffi::OsString,
    fs,
    sync::{mpsc, Mutex, OnceLock},
    time::Duration,
};

use axum::{
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use chrono::TimeZone;
use gwt::daemon_runtime::{handle_coordination_event, handle_forward, handle_runtime_state};
use gwt_agent::{runtime_state_path, AgentId, Session};
use gwt_core::paths::gwt_cache_dir;
use gwt_github::{CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt};
use serde_json::Value;
use tokio::{net::TcpListener, runtime::Runtime, sync::oneshot};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
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
            .route("/hook-live", post(capture_hook_live))
            .with_state(CaptureState { tx });

        runtime.spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            server.await.expect("capture server");
        });

        Self {
            runtime,
            shutdown_tx: Some(shutdown_tx),
            rx,
            url: format!("http://127.0.0.1:{}/hook-live", addr.port()),
        }
    }

    fn recv(&self) -> (String, Value) {
        self.rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected hook live event")
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
}

#[test]
fn coordination_event_owner_forwards_live_event_to_loopback_target() {
    let _lock = env_lock();
    let mut env = EnvGuard::new();
    let home = tempfile::tempdir().unwrap();
    env.set("HOME", home.path().as_os_str().to_os_string());
    env.set("USERPROFILE", home.path().as_os_str().to_os_string());

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
fn forward_owner_is_fail_open_without_live_target_env() {
    let _lock = env_lock();
    let mut env = EnvGuard::new();
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
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve loopback port");
    let port = listener.local_addr().expect("listener addr").port();
    drop(listener);
    env.set(
        "GWT_HOOK_FORWARD_URL",
        format!("http://127.0.0.1:{port}/hook-live"),
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
