//! Integration coverage for the workspace gwtd JSON operations
//! (`workspace.candidates` / `workspace.create`). SPEC-2359 Workspace /
//! Start Work.
//!
//! Audit gap (#3143): only `workspace.update` had an end-to-end test
//! (`gwtd_cli_test.rs`); candidates / create had none. `workspace.create`
//! resolves the agent from the projection, so the fixture seeds the strict
//! Session-bound mutation prerequisites directly: Session ledger, canonical
//! assignment, active Work/container, and tracked Work event. All ops run the
//! real `gwtd` binary through the stdin JSON envelope with an isolated HOME.

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener as StdTcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread::JoinHandle,
    time::Duration,
};

use axum::{
    extract::State,
    http::{
        header::{AUTHORIZATION, LOCATION},
        HeaderMap, StatusCode,
    },
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use chrono::Utc;
use gwt_agent::{
    AgentId, Session, GWT_HOOK_FORWARD_TOKEN_ENV, GWT_HOOK_FORWARD_URL_ENV, GWT_SESSION_ID_ENV,
    GWT_SESSION_RUNTIME_PATH_ENV,
};
use gwt_core::process::hidden_command;
use gwt_core::{
    paths::project_scope_hash,
    workspace_projection::{
        append_workspace_work_event_to_path, load_recent_workspace_journal_entries_from_path,
        load_workspace_projection_from_path, load_workspace_work_items_from_path,
        save_workspace_projection_to_path, save_workspace_work_items_projection_to_path, WorkEvent,
        WorkEventApplyOutcome, WorkEventKind, WorkItemsProjection, WorkspaceAgentAffiliationStatus,
        WorkspaceAgentSummary, WorkspaceExecutionContainerRef, WorkspaceProjection,
        WorkspaceStatusCategory,
    },
};
use serde_json::Value;
use tempfile::TempDir;
use tokio::{net::TcpListener, runtime::Runtime, sync::oneshot};

const SESSION: &str = "ws-cli-session";
const BRANCH: &str = "work/ws-cli";
const WORK_ID: &str = "existing-similar-work";
const FOREIGN_CURRENT_WORK_ID: &str = "foreign-current-work";
const FORWARD_TOKEN: &str = "workspace-proxy-secret-sentinel";
const HOST_WORK_ID: &str = "host-work-id";
const HOST_JOURNAL_ENTRY_ID: &str = "host-journal-entry-id";

#[derive(Debug)]
struct CapturedWorkspaceUpdate {
    authorization: String,
    body: Value,
}

#[derive(Clone)]
struct CaptureState {
    tx: mpsc::Sender<CapturedWorkspaceUpdate>,
    response_status: StatusCode,
    response_body: String,
    real_host: Option<RealHostWorkspaceUpdate>,
    before_response_mutation: Option<BeforeResponseMutation>,
}

#[derive(Clone)]
struct RealHostWorkspaceUpdate {
    home: PathBuf,
    project_root: PathBuf,
    session_id: String,
    bearer_token: String,
}

#[derive(Clone)]
struct SessionCapabilityRotation {
    home: PathBuf,
    session_id: String,
}

#[derive(Clone)]
enum BeforeResponseMutation {
    RotateCapability(SessionCapabilityRotation),
    SwitchToDocker {
        home: PathBuf,
        project_root: PathBuf,
        session_id: String,
    },
    DuplicateProjectionAgent {
        home: PathBuf,
        project_root: PathBuf,
        session_id: String,
    },
    SwitchCurrentProjection {
        home: PathBuf,
        project_root: PathBuf,
        expected_work_id: String,
    },
}

struct CaptureServer {
    runtime: Runtime,
    shutdown_tx: Option<oneshot::Sender<()>>,
    rx: mpsc::Receiver<CapturedWorkspaceUpdate>,
    /// Existing launch plumbing exports the hook-live URL. The workspace
    /// client must retain the listener origin but address its dedicated route.
    forward_url: String,
}

impl CaptureServer {
    fn success() -> Self {
        Self::start(
            StatusCode::OK,
            serde_json::json!({
                "schema_version": 1,
                "work_id": HOST_WORK_ID,
                "journal_entry_id": HOST_JOURNAL_ENTRY_ID,
            })
            .to_string(),
        )
    }

    fn start(response_status: StatusCode, response_body: impl Into<String>) -> Self {
        Self::start_with_real_host(response_status, response_body, None, None)
    }

    fn start_with_capability_rotation(
        response_status: StatusCode,
        response_body: impl Into<String>,
        home: &Path,
        session_id: &str,
    ) -> Self {
        Self::start_with_real_host(
            response_status,
            response_body,
            None,
            Some(BeforeResponseMutation::RotateCapability(
                SessionCapabilityRotation {
                    home: home.to_path_buf(),
                    session_id: session_id.to_string(),
                },
            )),
        )
    }

    fn start_with_docker_switch(
        response_status: StatusCode,
        response_body: impl Into<String>,
        home: &Path,
        project_root: &Path,
        session_id: &str,
    ) -> Self {
        Self::start_with_real_host(
            response_status,
            response_body,
            None,
            Some(BeforeResponseMutation::SwitchToDocker {
                home: home.to_path_buf(),
                project_root: project_root.to_path_buf(),
                session_id: session_id.to_string(),
            }),
        )
    }

    fn start_with_projection_duplicate(
        response_status: StatusCode,
        response_body: impl Into<String>,
        home: &Path,
        project_root: &Path,
        session_id: &str,
    ) -> Self {
        Self::start_with_real_host(
            response_status,
            response_body,
            None,
            Some(BeforeResponseMutation::DuplicateProjectionAgent {
                home: home.to_path_buf(),
                project_root: project_root.to_path_buf(),
                session_id: session_id.to_string(),
            }),
        )
    }

    fn real_host(home: &Path, project_root: &Path, session_id: &str, bearer_token: &str) -> Self {
        Self::start_with_real_host(
            StatusCode::OK,
            String::new(),
            Some(RealHostWorkspaceUpdate {
                home: home.to_path_buf(),
                project_root: project_root.to_path_buf(),
                session_id: session_id.to_string(),
                bearer_token: bearer_token.to_string(),
            }),
            None,
        )
    }

    fn real_host_after_current_switch(
        home: &Path,
        project_root: &Path,
        session_id: &str,
        bearer_token: &str,
        expected_work_id: &str,
    ) -> Self {
        Self::start_with_real_host(
            StatusCode::OK,
            String::new(),
            Some(RealHostWorkspaceUpdate {
                home: home.to_path_buf(),
                project_root: project_root.to_path_buf(),
                session_id: session_id.to_string(),
                bearer_token: bearer_token.to_string(),
            }),
            Some(BeforeResponseMutation::SwitchCurrentProjection {
                home: home.to_path_buf(),
                project_root: project_root.to_path_buf(),
                expected_work_id: expected_work_id.to_string(),
            }),
        )
    }

    fn start_with_real_host(
        response_status: StatusCode,
        response_body: impl Into<String>,
        real_host: Option<RealHostWorkspaceUpdate>,
        before_response_mutation: Option<BeforeResponseMutation>,
    ) -> Self {
        let runtime = Runtime::new().expect("tokio runtime");
        let listener = runtime
            .block_on(TcpListener::bind(("127.0.0.1", 0)))
            .expect("bind loopback listener");
        let addr = listener.local_addr().expect("listener addr");
        let (tx, rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let app = Router::new()
            .route("/internal/workspace-update", post(capture_workspace_update))
            .with_state(CaptureState {
                tx,
                response_status,
                response_body: response_body.into(),
                real_host,
                before_response_mutation,
            });

        runtime.spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            server.await.expect("workspace update capture server");
        });

        Self {
            runtime,
            shutdown_tx: Some(shutdown_tx),
            rx,
            forward_url: format!("http://127.0.0.1:{}/internal/hook-live", addr.port()),
        }
    }

    fn recv(&self) -> CapturedWorkspaceUpdate {
        self.rx
            .recv_timeout(Duration::from_secs(2))
            .expect("expected workspace.update proxy request")
    }

    fn assert_no_request(&self) {
        assert!(
            self.rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "workspace.update must fail before contacting the proxy"
        );
    }

    fn assert_no_additional_request(&self) {
        assert!(
            self.rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "workspace.update must not replay the Host bridge request"
        );
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

struct DisconnectServer {
    forward_url: String,
    shutdown_tx: mpsc::Sender<()>,
    rx: mpsc::Receiver<usize>,
    handle: Option<JoinHandle<()>>,
}

impl DisconnectServer {
    fn start() -> Self {
        let listener = StdTcpListener::bind(("127.0.0.1", 0)).expect("bind disconnect server");
        let port = listener
            .local_addr()
            .expect("disconnect server addr")
            .port();
        listener
            .set_nonblocking(true)
            .expect("disconnect server nonblocking");
        let (tx, rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let first_request_deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut accepted = 0;
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .expect("disconnect stream timeout");
                        let mut request_prefix = [0_u8; 1024];
                        let _ = stream.read(&mut request_prefix);
                        accepted += 1;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        let now = std::time::Instant::now();
                        if shutdown_rx.try_recv().is_ok() {
                            tx.send(accepted)
                                .expect("record disconnected bridge request count");
                            return;
                        }
                        if accepted == 0 && now >= first_request_deadline {
                            tx.send(accepted)
                                .expect("record missing disconnected bridge request");
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("disconnect server accept: {error}"),
                }
            }
        });
        Self {
            forward_url: format!("http://127.0.0.1:{port}/internal/hook-live"),
            shutdown_tx,
            rx,
            handle: Some(handle),
        }
    }

    fn receive(&mut self) {
        self.shutdown_tx
            .send(())
            .expect("stop disconnect listener after the client exits");
        let accepted = self
            .rx
            .recv_timeout(Duration::from_secs(4))
            .expect("expected disconnected bridge request count");
        self.handle
            .take()
            .expect("disconnect server thread")
            .join()
            .expect("join disconnect server");
        assert_eq!(
            accepted, 1,
            "response loss must produce exactly one accepted bridge request"
        );
    }
}

#[derive(Debug)]
struct AppliedDisconnectObservation {
    accepted: usize,
    work_id: String,
    journal_entry_id: String,
}

struct ApplyThenDisconnectServer {
    forward_url: String,
    shutdown_tx: mpsc::Sender<()>,
    rx: mpsc::Receiver<AppliedDisconnectObservation>,
    handle: Option<JoinHandle<()>>,
}

impl ApplyThenDisconnectServer {
    fn start(home: &Path, project_root: &Path, session_id: &str, bearer_token: &str) -> Self {
        let listener =
            StdTcpListener::bind(("127.0.0.1", 0)).expect("bind apply-then-disconnect server");
        let port = listener
            .local_addr()
            .expect("apply-then-disconnect server addr")
            .port();
        listener
            .set_nonblocking(true)
            .expect("apply-then-disconnect server nonblocking");
        let home = home.to_path_buf();
        let project_root = project_root.to_path_buf();
        let session_id = session_id.to_string();
        let bearer_token = bearer_token.to_string();
        let (tx, rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let first_request_deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut accepted = 0;
            let mut applied_receipt = None;
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .expect("apply-then-disconnect stream timeout");
                        accepted += 1;
                        if applied_receipt.is_none() {
                            let (authorization, body) = read_http_json_request(&mut stream);
                            assert_eq!(authorization, format!("Bearer {bearer_token}"));
                            let request =
                                serde_json::from_slice::<gwt::AgentWorkspaceUpdateRequest>(&body)
                                    .expect("parse apply-then-disconnect Host request");
                            let _env_lock = gwt_core::test_support::env_lock()
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            let _home = gwt_core::test_support::ScopedGwtHome::set(&home);
                            let receipt = gwt::apply_authenticated_workspace_update(
                                &project_root,
                                &session_id,
                                request,
                            )
                            .expect("apply Host update before dropping response");
                            applied_receipt = Some((receipt.work_id, receipt.journal_entry_id));
                        } else {
                            let mut request_prefix = [0_u8; 1024];
                            let _ = stream.read(&mut request_prefix);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        let now = std::time::Instant::now();
                        if shutdown_rx.try_recv().is_ok() {
                            let (work_id, journal_entry_id) = applied_receipt
                                .expect("the accepted Host request must be applied exactly once");
                            tx.send(AppliedDisconnectObservation {
                                accepted,
                                work_id,
                                journal_entry_id,
                            })
                            .expect("record apply-then-disconnect observation");
                            return;
                        }
                        if accepted == 0 && now >= first_request_deadline {
                            panic!("timed out before the apply-then-disconnect Host request");
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("apply-then-disconnect server accept: {error}"),
                }
            }
        });
        Self {
            forward_url: format!("http://127.0.0.1:{port}/internal/hook-live"),
            shutdown_tx,
            rx,
            handle: Some(handle),
        }
    }

    fn receive(&mut self) -> AppliedDisconnectObservation {
        self.shutdown_tx
            .send(())
            .expect("stop apply-then-disconnect listener after the client exits");
        let observation = self
            .rx
            .recv_timeout(Duration::from_secs(4))
            .expect("expected apply-then-disconnect observation");
        self.handle
            .take()
            .expect("apply-then-disconnect server thread")
            .join()
            .expect("join apply-then-disconnect server");
        assert_eq!(
            observation.accepted, 1,
            "an applied Host update must not be retried after response loss"
        );
        observation
    }
}

fn read_http_json_request(stream: &mut std::net::TcpStream) -> (String, Vec<u8>) {
    let mut request = Vec::new();
    let (body_start, content_length) = loop {
        let mut buffer = [0_u8; 4096];
        let read = stream
            .read(&mut buffer)
            .expect("read apply-then-disconnect request");
        assert!(read > 0, "Host request closed before its headers arrived");
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let body_start = header_end + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .expect("Host request Content-Length");
        break (body_start, content_length);
    };
    while request.len() < body_start + content_length {
        let mut buffer = [0_u8; 4096];
        let read = stream
            .read(&mut buffer)
            .expect("read apply-then-disconnect request body");
        assert!(read > 0, "Host request closed before its body arrived");
        request.extend_from_slice(&buffer[..read]);
    }
    let headers = String::from_utf8_lossy(&request[..body_start - 4]);
    let authorization = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.trim().to_string())
        })
        .expect("Host request Authorization header");
    (
        authorization,
        request[body_start..body_start + content_length].to_vec(),
    )
}

struct RedirectServer {
    runtime: Runtime,
    shutdown_tx: Option<oneshot::Sender<()>>,
    source_rx: mpsc::Receiver<CapturedWorkspaceUpdate>,
    redirect_rx: mpsc::Receiver<CapturedWorkspaceUpdate>,
    forward_url: String,
}

impl RedirectServer {
    fn start() -> Self {
        let runtime = Runtime::new().expect("tokio runtime");
        let listener = runtime
            .block_on(TcpListener::bind(("127.0.0.1", 0)))
            .expect("bind redirect listener");
        let addr = listener.local_addr().expect("redirect listener addr");
        let (source_tx, source_rx) = mpsc::channel();
        let (redirect_tx, redirect_rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let app = Router::new()
            .route(
                "/internal/workspace-update",
                post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                    source_tx
                        .send(CapturedWorkspaceUpdate {
                            authorization: headers
                                .get(AUTHORIZATION)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_string(),
                            body,
                        })
                        .expect("capture redirect source request");
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        [(LOCATION, "/redirected-exact")],
                    )
                }),
            )
            .route(
                "/redirected-exact",
                post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                    redirect_tx
                        .send(CapturedWorkspaceUpdate {
                            authorization: headers
                                .get(AUTHORIZATION)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_string(),
                            body,
                        })
                        .expect("capture redirected request");
                    (
                        StatusCode::CONFLICT,
                        Json(serde_json::json!({
                            "code": "workspace_ensure_required",
                            "reason": "workspace_ensure_required",
                            "message": "redirect target returned an exact typed refusal",
                        })),
                    )
                }),
            );

        runtime.spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            server.await.expect("workspace update redirect server");
        });

        Self {
            runtime,
            shutdown_tx: Some(shutdown_tx),
            source_rx,
            redirect_rx,
            forward_url: format!("http://127.0.0.1:{}/internal/hook-live", addr.port()),
        }
    }

    fn recv_source(&self) -> CapturedWorkspaceUpdate {
        self.source_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("expected redirect source request")
    }

    fn assert_no_redirect_request(&self) {
        assert!(
            self.redirect_rx
                .recv_timeout(Duration::from_millis(300))
                .is_err(),
            "workspace.update must not follow redirects"
        );
    }
}

impl Drop for RedirectServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.runtime
            .block_on(async { tokio::time::sleep(Duration::from_millis(25)).await });
    }
}

async fn capture_workspace_update(
    headers: HeaderMap,
    State(state): State<CaptureState>,
    Json(body): Json<Value>,
) -> Response {
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    state
        .tx
        .send(CapturedWorkspaceUpdate {
            authorization: authorization.clone(),
            body: body.clone(),
        })
        .expect("capture workspace.update request");
    if let Some(mutation) = state.before_response_mutation {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match mutation {
            BeforeResponseMutation::RotateCapability(rotation) => {
                let _home = gwt_core::test_support::ScopedGwtHome::set(&rotation.home);
                gwt_agent::rotate_session_execution_capability(
                    &gwt_core::paths::gwt_sessions_dir(),
                    &rotation.session_id,
                )
                .expect("rotate Session capability before static Host response");
            }
            BeforeResponseMutation::SwitchToDocker {
                home,
                project_root,
                session_id,
            } => {
                let _home = gwt_core::test_support::ScopedGwtHome::set(&home);
                let session_path =
                    gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
                let mut session =
                    Session::load(&session_path).expect("load Host Session before Docker switch");
                session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
                session
                    .bind_docker_runtime(
                        project_root
                            .canonicalize()
                            .expect("canonical Docker runtime root"),
                        &project_root,
                    )
                    .expect("bind Docker runtime before static Host response");
                session
                    .save(&gwt_core::paths::gwt_sessions_dir())
                    .expect("save Docker Session before static Host response");
            }
            BeforeResponseMutation::DuplicateProjectionAgent {
                home,
                project_root,
                session_id,
            } => {
                let _home = gwt_core::test_support::ScopedGwtHome::set(&home);
                let projection_path =
                    gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root);
                let mut projection = load_workspace_projection_from_path(&projection_path)
                    .expect("load projection before duplicate")
                    .expect("projection exists before duplicate");
                let duplicate = projection
                    .latest_agent_for_session(&session_id)
                    .expect("Session projection row before duplicate")
                    .clone();
                projection.agents.push(duplicate);
                save_workspace_projection_to_path(&projection_path, &projection)
                    .expect("duplicate Session projection row before static Host response");
            }
            BeforeResponseMutation::SwitchCurrentProjection {
                home,
                project_root,
                expected_work_id,
            } => switch_current_projection_away_from_exact_work(
                &home,
                &project_root,
                &expected_work_id,
            ),
        }
    }
    if let Some(real_host) = state.real_host {
        if authorization != format!("Bearer {}", real_host.bearer_token) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "code": "invalid_request",
                    "message": "invalid test Host capability",
                })),
            )
                .into_response();
        }
        let request = match serde_json::from_value::<gwt::AgentWorkspaceUpdateRequest>(body) {
            Ok(request) => request,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "code": "invalid_request",
                        "message": format!("invalid test Host request: {error}"),
                    })),
                )
                    .into_response();
            }
        };
        let _home = gwt_core::test_support::ScopedGwtHome::set(&real_host.home);
        return match gwt::apply_authenticated_workspace_update(
            &real_host.project_root,
            &real_host.session_id,
            request,
        ) {
            Ok(receipt) => Json(receipt).into_response(),
            Err(error) => (StatusCode::CONFLICT, Json(error)).into_response(),
        };
    }
    (
        state.response_status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        state.response_body,
    )
        .into_response()
}

fn git_init_with_origin(path: &Path) {
    assert!(hidden_command("git")
        .args(["init", "-q", "-b", BRANCH])
        .arg(path)
        .status()
        .expect("git init")
        .success());
    assert!(hidden_command("git")
        .arg("-C")
        .arg(path)
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/gwt-workspace-cli.git",
        ])
        .status()
        .expect("git remote add")
        .success());
}

struct Fixture {
    home: TempDir,
    project: TempDir,
}

fn fixture() -> Fixture {
    let home = tempfile::tempdir().expect("home tempdir");
    let project = tempfile::tempdir().expect("project tempdir");
    git_init_with_origin(project.path());
    Fixture { home, project }
}

fn gwtd_command(fixture: &Fixture, session_id: &str) -> Command {
    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
    command
        .current_dir(fixture.project.path())
        .env("HOME", fixture.home.path())
        .env("USERPROFILE", fixture.home.path())
        .env(GWT_SESSION_ID_ENV, session_id)
        // Integration tests must not inherit this test runner's managed-agent
        // bridge. Each case below opts into an explicit capture target.
        .env_remove(GWT_HOOK_FORWARD_URL_ENV)
        .env_remove(GWT_HOOK_FORWARD_TOKEN_ENV)
        .env_remove(GWT_SESSION_RUNTIME_PATH_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run_ws(fixture: &Fixture, json: &str) -> Value {
    let mut child = gwtd_command(fixture, SESSION).spawn().expect("run gwtd");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(json.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait gwtd");
    assert!(
        output.status.success(),
        "gwtd should exit 0 for `{json}`, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "parse gwtd JSON response: {err}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn assert_ok(value: &Value, context: &str) {
    assert_eq!(
        value.get("ok").and_then(Value::as_bool),
        Some(true),
        "{context} should report ok=true, got: {value}"
    );
}

/// Run an op without asserting success — for exercising error/guard paths.
fn run_ws_raw(fixture: &Fixture, json: &str) -> std::process::Output {
    let mut child = gwtd_command(fixture, SESSION).spawn().expect("run gwtd");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(json.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait gwtd")
}

fn run_ws_raw_with_forward_env(
    fixture: &Fixture,
    json: &str,
    session_id: &str,
    forward_url: Option<&str>,
    forward_token: Option<&str>,
) -> std::process::Output {
    let mut command = gwtd_command(fixture, session_id);
    if let Some(url) = forward_url {
        command.env(GWT_HOOK_FORWARD_URL_ENV, url);
    }
    if let Some(token) = forward_token {
        command.env(GWT_HOOK_FORWARD_TOKEN_ENV, token);
    }
    let mut child = command.spawn().expect("run gwtd with proxy env");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(json.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait gwtd")
}

fn output_text(output: &std::process::Output) -> String {
    format!(
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_secret_redacted(output: &std::process::Output, secret: &str) {
    let rendered = output_text(output);
    assert!(
        !rendered.contains(secret),
        "workspace.update diagnostics must redact the forwarding bearer"
    );
}

#[derive(Debug, PartialEq, Eq)]
struct MutationStateSnapshot(Vec<(String, &'static str, Vec<u8>)>);

fn mutation_state_snapshot(fixture: &Fixture) -> MutationStateSnapshot {
    let mut entries = Vec::new();
    snapshot_tree(
        &fixture.home.path().join(".gwt"),
        Path::new("container-home/.gwt"),
        &mut entries,
    );
    snapshot_tree(
        &fixture.project.path().join(".gwt"),
        Path::new("container-project/.gwt"),
        &mut entries,
    );
    MutationStateSnapshot(entries)
}

fn container_home_state_snapshot(fixture: &Fixture) -> MutationStateSnapshot {
    let mut entries = Vec::new();
    snapshot_tree(
        fixture.home.path(),
        Path::new("container-home"),
        &mut entries,
    );
    MutationStateSnapshot(entries)
}

fn workspace_delivery_state_snapshot(fixture: &Fixture) -> MutationStateSnapshot {
    let mut entries = Vec::new();
    let project_state = fixture
        .home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(fixture.project.path()).as_str())
        .join("project-state");
    snapshot_tree(&project_state, Path::new("project-state"), &mut entries);
    snapshot_tree(
        &fixture.project.path().join(".gwt/work"),
        Path::new("tracked-work"),
        &mut entries,
    );
    match work_event_settlement_state_snapshot(fixture) {
        Some(bytes) => entries.push((
            "trusted-work/work-event-settlement.json".to_string(),
            "file",
            bytes,
        )),
        None => entries.push((
            "trusted-work/work-event-settlement.json".to_string(),
            "missing",
            Vec::new(),
        )),
    }
    MutationStateSnapshot(entries)
}

fn work_event_settlement_state_snapshot(fixture: &Fixture) -> Option<Vec<u8>> {
    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _home = gwt_core::test_support::ScopedGwtHome::set(fixture.home.path());
    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(fixture.project.path())
        .expect("fixture has a repo-scoped trusted store");
    fs::read(trusted_dir.join("work-event-settlement.json")).ok()
}

fn snapshot_tree(
    path: &Path,
    display_path: &Path,
    entries: &mut Vec<(String, &'static str, Vec<u8>)>,
) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        entries.push((display_path.display().to_string(), "missing", Vec::new()));
        return;
    };
    if metadata.is_dir() {
        entries.push((display_path.display().to_string(), "directory", Vec::new()));
        let mut children = fs::read_dir(path)
            .expect("read state snapshot directory")
            .map(|entry| entry.expect("read state snapshot entry"))
            .collect::<Vec<_>>();
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            snapshot_tree(
                &child.path(),
                &display_path.join(child.file_name()),
                entries,
            );
        }
    } else if metadata.is_file() {
        entries.push((
            display_path.display().to_string(),
            "file",
            fs::read(path).expect("read state snapshot file"),
        ));
    } else {
        entries.push((display_path.display().to_string(), "other", Vec::new()));
    }
}

fn poison_container_authority_state(fixture: &Fixture) {
    let sessions_dir = fixture.home.path().join(".gwt/sessions");
    fs::create_dir_all(&sessions_dir).expect("create poisoned Session directory");
    fs::write(
        sessions_dir.join(format!("{SESSION}.toml")),
        "this is intentionally invalid Session TOML",
    )
    .expect("write poisoned Session ledger");

    let state_dir = fixture
        .home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(fixture.project.path()).as_str())
        .join("project-state");
    fs::create_dir_all(&state_dir).expect("create poisoned Project State directory");
    fs::write(state_dir.join("current.json"), b"not-json")
        .expect("write poisoned current projection");
    fs::write(state_dir.join("works.json"), b"not-json")
        .expect("write poisoned WorkItems projection");
}

fn reserve_unreachable_forward_url() -> String {
    let listener = StdTcpListener::bind(("127.0.0.1", 0)).expect("reserve loopback port");
    let port = listener.local_addr().expect("listener addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}/internal/hook-live")
}

fn load_projection(fixture: &Fixture) -> WorkspaceProjection {
    let path = fixture
        .home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(fixture.project.path()).as_str())
        .join("project-state/current.json");
    load_workspace_projection_from_path(&path)
        .expect("load workspace projection")
        .expect("workspace projection should exist under isolated home")
}

fn switch_current_projection_away_from_exact_work(
    home: &Path,
    project_root: &Path,
    expected_work_id: &str,
) {
    let project_root = project_root
        .canonicalize()
        .expect("canonical project root for current switch");
    let state_dir = home
        .join(".gwt/projects")
        .join(project_scope_hash(&project_root).as_str())
        .join("project-state");
    let projection_path = state_dir.join("current.json");
    let works_path = state_dir.join("works.json");
    let mut projection = load_workspace_projection_from_path(&projection_path)
        .expect("load exact current projection before switch")
        .expect("exact current projection exists before switch");
    assert_eq!(
        projection.id, expected_work_id,
        "the race fixture must begin with the exact Work current"
    );
    assert_eq!(
        projection
            .latest_agent_for_session(SESSION)
            .and_then(|agent| agent.workspace_id.as_deref()),
        Some(expected_work_id),
        "switching current must preserve the exact durable Session assignment"
    );

    let now = Utc::now();
    let mut works = load_workspace_work_items_from_path(&works_path)
        .expect("load exact WorkItems before current switch")
        .expect("exact WorkItems exist before current switch");
    if !works
        .work_items
        .iter()
        .any(|work| work.id == FOREIGN_CURRENT_WORK_ID)
    {
        let mut event = WorkEvent::new(WorkEventKind::Start, FOREIGN_CURRENT_WORK_ID, now);
        event.title = Some("Concurrent foreign current Work".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        event.owner = Some("Issue #9999".to_string());
        assert_eq!(
            works.apply_event(event),
            WorkEventApplyOutcome::Applied,
            "materialize the foreign current Work"
        );
    }
    save_workspace_work_items_projection_to_path(&works_path, &works)
        .expect("save WorkItems after current switch");

    projection.id = FOREIGN_CURRENT_WORK_ID.to_string();
    projection.title = "Concurrent foreign current Work".to_string();
    projection.status_category = WorkspaceStatusCategory::Active;
    projection.status_text = "active".to_string();
    projection.owner = Some("Issue #9999".to_string());
    projection.updated_at = now;
    save_workspace_projection_to_path(&projection_path, &projection)
        .expect("save foreign current projection");
}

/// Seed the complete Session-bound mutation target without invoking the
/// `workspace.update` path under test or relying on default synthesis.
fn register_agent(fixture: &Fixture) {
    register_agent_at_home(fixture.home.path(), fixture.project.path());
}

fn register_bound_agent(fixture: &Fixture) -> gwt_agent::SessionExecutionIdentity {
    register_agent(fixture);
    bind_existing_session_to_execution(fixture)
}

fn register_projectionless_bound_session(fixture: &Fixture) -> gwt_agent::SessionExecutionIdentity {
    let project_root = fixture
        .project
        .path()
        .canonicalize()
        .expect("canonical project root");
    let mut session = Session::new(&project_root, BRANCH, AgentId::Codex);
    session.id = SESSION.to_string();
    session.project_state_root = Some(project_root);
    session.linked_issue_number = Some(3412);
    session
        .save(&fixture.home.path().join(".gwt/sessions"))
        .expect("save projectionless Session fixture");
    bind_existing_session_to_execution(fixture)
}

struct ExactEnsuredHost {
    identity: gwt_agent::SessionExecutionIdentity,
    work_id: String,
}

fn prepare_exact_ensured_host(fixture: &Fixture) -> ExactEnsuredHost {
    let identity = register_projectionless_bound_session(fixture);
    let ensure = run_ws(
        fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.ensure","params":{{"agent_session":"{SESSION}","purpose":"Exact ensured Host fixture","current_focus":"eligible compatibility authority","issue":3412}}}}"#
        ),
    );
    assert_ok(&ensure, "exact Host workspace.ensure");
    assert!(
        ensure["output"]
            .as_str()
            .is_some_and(|output| output.contains("created")),
        "eligible fixture must materialize one canonical Work: {ensure}"
    );
    let work_id = load_projection(fixture)
        .latest_agent_for_session(SESSION)
        .and_then(|agent| agent.workspace_id.clone())
        .expect("exact ensured Work id");
    ExactEnsuredHost { identity, work_id }
}

fn bind_existing_session_to_execution(fixture: &Fixture) -> gwt_agent::SessionExecutionIdentity {
    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _home = gwt_core::test_support::ScopedGwtHome::set(fixture.home.path());
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3412,
    };
    gwt::cli::execution_state::materialize_at_launch(
        fixture.project.path(),
        owner.kind,
        owner.number,
        SESSION,
        "gwt-execute",
        false,
    )
    .expect("materialize active execution fixture");
    gwt::cli::execution_state::ensure_generation_ledger(
        fixture.project.path(),
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize generation ledger fixture");
    let identity =
        gwt::cli::execution_state::current_execution_binding(fixture.project.path(), owner)
            .expect("read current execution binding")
            .expect("current execution binding exists");
    let session_path = fixture
        .home
        .path()
        .join(".gwt/sessions")
        .join(format!("{SESSION}.toml"));
    let mut session = Session::load(&session_path).expect("load Session fixture for binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: SESSION.to_string(),
        repo_hash: session
            .repo_hash
            .clone()
            .expect("fixture repository identity"),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    session
        .set_execution_binding(Some(binding))
        .expect("bind durable Session fixture");
    session
        .save(&fixture.home.path().join(".gwt/sessions"))
        .expect("save bound Session fixture");
    gwt_agent::SessionExecutionIdentity::from_session(&session)
        .expect("validate exact Session fixture")
        .expect("bound Session identity exists")
}

fn mark_bound_session_as_docker(fixture: &Fixture) {
    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _home = gwt_core::test_support::ScopedGwtHome::set(fixture.home.path());
    let session_path = fixture
        .home
        .path()
        .join(".gwt/sessions")
        .join(format!("{SESSION}.toml"));
    let mut session = Session::load(&session_path).expect("load bound Docker Session fixture");
    session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
    session
        .bind_docker_runtime(
            fixture
                .project
                .path()
                .canonicalize()
                .expect("canonical Docker runtime fixture"),
            fixture.project.path(),
        )
        .expect("bind Docker runtime fixture");
    session
        .save(&fixture.home.path().join(".gwt/sessions"))
        .expect("save bound Docker Session fixture");
}

fn register_agent_at_home(home: &Path, project: &Path) {
    let project_root = project.canonicalize().expect("canonical project root");
    let mut session = Session::new(&project_root, BRANCH, AgentId::Codex);
    session.id = SESSION.to_string();
    session.project_state_root = Some(project_root.clone());
    session.linked_issue_number = Some(3412);
    assert!(
        session.repo_hash.is_some(),
        "fixture origin must set repo hash"
    );
    session
        .save(&home.join(".gwt/sessions"))
        .expect("save Session ledger fixture");

    let state_dir = home
        .join(".gwt/projects")
        .join(project_scope_hash(&project_root).as_str())
        .join("project-state");
    let now = Utc::now();
    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.agents.push(WorkspaceAgentSummary {
        session_id: SESSION.to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: Some("registering".to_string()),
        title_summary: Some("workspace cli coverage".to_string()),
        worktree_path: Some(project_root.clone()),
        branch: Some(BRANCH.to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some(WORK_ID.to_string()),
        updated_at: now,
    });
    save_workspace_projection_to_path(&state_dir.join("current.json"), &projection)
        .expect("save canonical Session assignment");

    let mut event = WorkEvent::new(WorkEventKind::Start, WORK_ID, now);
    event.title = Some("workspace cli coverage".to_string());
    event.intent = Some("registering".to_string());
    event.status_category = Some(WorkspaceStatusCategory::Active);
    event.owner = Some("Issue #3412".to_string());
    event.agent_session_id = Some(SESSION.to_string());
    event.agent_id = Some("codex".to_string());
    event.display_name = Some("Codex".to_string());
    event.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some(BRANCH.to_string()),
        worktree_path: Some(project_root.clone()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    let mut work_items = WorkItemsProjection::empty(now);
    assert_eq!(
        work_items.apply_event(event.clone()),
        WorkEventApplyOutcome::Applied
    );
    save_workspace_work_items_projection_to_path(&state_dir.join("works.json"), &work_items)
        .expect("save active WorkItems fixture");
    append_workspace_work_event_to_path(&project_root.join(".gwt/work/events.jsonl"), &event)
        .expect("save tracked Work event fixture");
}

#[test]
fn workspace_candidates_reports_without_error() {
    let fixture = fixture();
    let candidates = run_ws(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.candidates","params":{{"agent_session":"{SESSION}"}}}}"#
        ),
    );
    assert_ok(&candidates, "workspace.candidates");
    let rendered = candidates
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        rendered.contains("candidate"),
        "workspace.candidates output should describe candidates (incl. the `none` case), got: {rendered}"
    );
}

#[test]
fn workspace_create_rejects_duplicate_similar_workspace() {
    // `register_agent` seeds an existing incomplete Work titled "workspace
    // cli coverage". `workspace.create` then guards against duplicating it and surfaces an
    // actionable error (SPEC-2359: prefer joining the existing Work over
    // minting a near-duplicate).
    let fixture = fixture();
    register_agent(&fixture);

    let output = run_ws_raw(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.create","params":{{"agent_session":"{SESSION}","purpose":"workspace cli coverage"}}}}"#
        ),
    );
    assert!(
        !output.status.success(),
        "workspace.create must reject a near-duplicate Workspace; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("similar Workspace exists"),
        "the guard must explain the near-duplicate; stderr={stderr}"
    );

    // The agent and its original Work item remain intact after the rejected create.
    let projection = load_projection(&fixture);
    assert!(
        projection
            .agents
            .iter()
            .any(|agent| agent.session_id == SESSION),
        "the registered agent must remain in the projection after a rejected create"
    );
}

#[test]
fn workspace_update_then_focus_change_persists() {
    let fixture = fixture();
    register_agent(&fixture);

    assert_ok(
        &run_ws(
            &fixture,
            &format!(
                r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","current_focus":"focus after register"}}}}"#
            ),
        ),
        "workspace.update (focus change)",
    );

    let projection = load_projection(&fixture);
    let agent = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == SESSION)
        .expect("registered agent must exist");
    assert_eq!(
        agent.current_focus.as_deref(),
        Some("focus after register"),
        "current_focus must persist across workspace.update calls"
    );
}

#[test]
fn workspace_update_complete_forward_pair_uses_host_proxy_without_reading_container_authority() {
    let fixture = fixture();
    poison_container_authority_state(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let server = CaptureServer::success();
    let request_json = format!(
        r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","purpose":"Proxy contract coverage","current_focus":"forward sparse intent","summary":"host owns mutation"}}}}"#
    );

    let output = run_ws_raw_with_forward_env(
        &fixture,
        &request_json,
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );
    assert!(
        output.status.success(),
        "a complete forwarding pair must bypass poisoned container authority state: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);

    let captured = server.recv();
    server.assert_no_additional_request();
    assert!(
        captured.authorization == format!("Bearer {FORWARD_TOKEN}"),
        "workspace.update proxy request must use the configured bearer"
    );
    let project_root = fixture
        .project
        .path()
        .canonicalize()
        .expect("canonical project root");
    assert_eq!(
        captured.body,
        serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": SESSION,
            "observation": {
                "cwd": project_root,
                "git_toplevel": project_root,
                "repo_hash": project_scope_hash(fixture.project.path()).as_str(),
                "branch": BRANCH,
            },
            "intent": {
                "summary": "host owns mutation",
                "current_focus": "forward sparse intent",
                "title_summary": "Proxy contract coverage",
            },
        }),
        "the proxy request must contain only the equality claim, runtime observation, and sparse intent"
    );
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "proxy success must not read-repair or mutate container HOME/Project State"
    );
}

#[test]
fn workspace_update_real_host_proxy_mutates_host_authority_with_separate_container_home() {
    for (case, poisoned_container) in [("empty", false), ("poisoned", true)] {
        let fixture = fixture();
        if poisoned_container {
            poison_container_authority_state(&fixture);
        }
        let container_before = container_home_state_snapshot(&fixture);
        let host_home = tempfile::tempdir().expect("Host HOME tempdir");
        register_agent_at_home(host_home.path(), fixture.project.path());

        let project_root = fixture
            .project
            .path()
            .canonicalize()
            .expect("canonical project root");
        let host_state_dir = host_home
            .path()
            .join(".gwt/projects")
            .join(project_scope_hash(&project_root).as_str())
            .join("project-state");
        let host_session_path = host_home
            .path()
            .join(".gwt/sessions")
            .join(format!("{SESSION}.toml"));
        let host_current_path = host_state_dir.join("current.json");
        let host_works_path = host_state_dir.join("works.json");
        let host_journal_path = host_state_dir.join("journal.jsonl");
        let tracked_events_path = project_root.join(".gwt/work/events.jsonl");
        let host_session_before = fs::read(&host_session_path).expect("Host Session before");
        let host_current_before = fs::read(&host_current_path).expect("Host current before");
        let host_works_before = fs::read(&host_works_path).expect("Host works before");
        let tracked_events_before = fs::read(&tracked_events_path).expect("tracked events before");
        assert!(
            !host_journal_path.exists(),
            "{case}: Host journal must start absent"
        );

        let server =
            CaptureServer::real_host(host_home.path(), &project_root, SESSION, FORWARD_TOKEN);
        let output = run_ws_raw_with_forward_env(
            &fixture,
            &format!(
                r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","purpose":"Real Host proxy coverage","current_focus":"real proxy sparse intent","summary":"host owns real mutation"}}}}"#
            ),
            SESSION,
            Some(&server.forward_url),
            Some(FORWARD_TOKEN),
        );
        assert!(
            output.status.success(),
            "{case}: the real Host proxy must commit successfully: {}",
            output_text(&output)
        );
        assert_secret_redacted(&output, FORWARD_TOKEN);

        let captured = server.recv();
        assert_eq!(
            captured.authorization,
            format!("Bearer {FORWARD_TOKEN}"),
            "{case}: the real Host handler must authenticate the forwarding bearer"
        );
        assert_eq!(
            captured.body,
            serde_json::json!({
                "schema_version": 1,
                "claimed_session_id": SESSION,
                "observation": {
                    "cwd": project_root,
                    "git_toplevel": project_root,
                    "repo_hash": project_scope_hash(&project_root).as_str(),
                    "branch": BRANCH,
                },
                "intent": {
                    "summary": "host owns real mutation",
                    "current_focus": "real proxy sparse intent",
                    "title_summary": "Real Host proxy coverage",
                },
            }),
            "{case}: the real mutation must consume the child observation and sparse intent"
        );
        assert_eq!(
            container_home_state_snapshot(&fixture),
            container_before,
            "{case}: proxy success must leave the separate container HOME byte-equivalent"
        );

        assert_eq!(
            fs::read(&host_session_path).expect("Host Session after"),
            host_session_before,
            "{case}: the Host Session ledger is authority and must not be rewritten"
        );
        assert_ne!(
            fs::read(&host_current_path).expect("Host current after"),
            host_current_before,
            "{case}: the authenticated Host current projection must change"
        );
        assert_ne!(
            fs::read(&host_works_path).expect("Host works after"),
            host_works_before,
            "{case}: the authenticated Host WorkItems projection must change"
        );

        let host_projection = load_workspace_projection_from_path(&host_current_path)
            .expect("load Host current")
            .expect("Host current exists");
        let host_agent = host_projection
            .latest_agent_for_session(SESSION)
            .expect("Host Session assignment");
        assert_eq!(
            host_agent.current_focus.as_deref(),
            Some("real proxy sparse intent")
        );
        assert_eq!(
            host_agent.title_summary.as_deref(),
            Some("Real Host proxy coverage")
        );
        let host_works = load_workspace_work_items_from_path(&host_works_path)
            .expect("load Host WorkItems")
            .expect("Host WorkItems exist");
        let host_work = host_works
            .work_items
            .iter()
            .find(|item| item.id == WORK_ID)
            .expect("Host target Work");
        assert_eq!(
            host_work.summary.as_deref(),
            Some("host owns real mutation")
        );

        assert!(
            !host_journal_path.exists(),
            "{case}: a foreign target must not enter the identity-less legacy current journal"
        );
        let response: Value = serde_json::from_slice(&output.stdout).expect("gwtd response JSON");
        assert_ok(&response, "real Host proxy workspace.update");
        let _journal_entry_id = response["output"]
            .as_str()
            .and_then(|value| value.strip_prefix("workspace updated: "))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .expect("Host mutation receipt id");

        let tracked_events_after = fs::read(&tracked_events_path).expect("tracked events after");
        assert!(
            tracked_events_after.starts_with(&tracked_events_before)
                && tracked_events_after.len() > tracked_events_before.len(),
            "{case}: the real Host commit must append exactly through the tracked event surface"
        );
        let appended_event = String::from_utf8(tracked_events_after)
            .expect("tracked events UTF-8")
            .lines()
            .rfind(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<WorkEvent>(line).expect("tracked Work event JSON"))
            .expect("appended tracked Work event");
        assert_eq!(appended_event.work_item_id, WORK_ID);
        assert_eq!(appended_event.kind, WorkEventKind::Update);
        assert_eq!(
            appended_event.summary.as_deref(),
            Some("host owns real mutation")
        );
        assert_eq!(appended_event.agent_session_id.as_deref(), Some(SESSION));
    }
}

#[test]
fn workspace_update_exact_same_home_host_accepts_fresh_canonical_journal_receipt() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let project_root = fixture
        .project
        .path()
        .canonicalize()
        .expect("canonical project root");
    let server =
        CaptureServer::real_host(fixture.home.path(), &project_root, SESSION, FORWARD_TOKEN);
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","purpose":"Exact Host receipt","current_focus":"validate fresh canonical journal","summary":"same HOME Host applied once"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        output.status.success(),
        "an exact same-HOME Host receipt must read back successfully: {}",
        output_text(&output)
    );
    server.recv();
    server.assert_no_additional_request();
    let response: Value = serde_json::from_slice(&output.stdout).expect("gwtd response JSON");
    let receipt_id = response["output"]
        .as_str()
        .and_then(|value| value.strip_prefix("workspace updated: "))
        .map(str::trim)
        .expect("workspace receipt journal id");
    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _home = gwt_core::test_support::ScopedGwtHome::set(fixture.home.path());
    let journal = gwt_core::workspace_projection::load_recent_workspace_journal_entries(
        &project_root,
        usize::MAX,
    )
    .expect("load canonical Host journal");
    assert_eq!(
        journal
            .iter()
            .filter(|entry| entry.id == receipt_id)
            .count(),
        1,
        "the accepted receipt must identify one fresh canonical journal entry"
    );
    assert_eq!(load_projection(&fixture).id, exact.work_id);
}

#[test]
fn workspace_update_exact_same_home_foreign_work_accepts_fresh_work_event_receipt() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let project_root = fixture
        .project
        .path()
        .canonicalize()
        .expect("canonical project root");
    switch_current_projection_away_from_exact_work(
        fixture.home.path(),
        &project_root,
        &exact.work_id,
    );
    assert_ne!(
        load_projection(&fixture).id,
        exact.work_id,
        "fixture target must exercise the non-current Work contract"
    );
    let server =
        CaptureServer::real_host(fixture.home.path(), &project_root, SESSION, FORWARD_TOKEN);
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","purpose":"Foreign Work receipt","current_focus":"validate fresh Work event","summary":"foreign Work Host applied once"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        output.status.success(),
        "an exact foreign-Work Host receipt must use Work event evidence: {}",
        output_text(&output)
    );
    server.recv();
    server.assert_no_additional_request();
    let response: Value = serde_json::from_slice(&output.stdout).expect("foreign response JSON");
    let receipt_id = response["output"]
        .as_str()
        .and_then(|value| value.strip_prefix("workspace updated: "))
        .map(str::trim)
        .expect("foreign receipt evidence id");
    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _home = gwt_core::test_support::ScopedGwtHome::set(fixture.home.path());
    let state_dir = fixture
        .home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(&project_root).as_str())
        .join("project-state");
    let works = load_workspace_work_items_from_path(&state_dir.join("works.json"))
        .expect("load WorkItems after foreign Work receipt")
        .expect("WorkItems exist after foreign Work receipt");
    let delivered = works
        .work_items
        .iter()
        .find(|work| work.id == exact.work_id)
        .expect("foreign target Work")
        .events
        .iter()
        .filter(|event| {
            event.agent_session_id.as_deref() == Some(SESSION)
                && event.summary.as_deref() == Some("foreign Work Host applied once")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        delivered.len(),
        1,
        "the foreign Work mutation must be delivered once"
    );
    assert_eq!(
        delivered[0].id, receipt_id,
        "a foreign receipt must identify its exact append-only Work event"
    );
    assert!(
        !state_dir.join("journal.jsonl").exists(),
        "foreign Work success must preserve the identity-less journal exclusion contract"
    );
}

#[test]
fn workspace_update_current_snapshot_accepts_fresh_event_after_host_switches_current_work() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let project_root = fixture
        .project
        .path()
        .canonicalize()
        .expect("canonical project root");
    let server = CaptureServer::real_host_after_current_switch(
        fixture.home.path(),
        &project_root,
        SESSION,
        FORWARD_TOKEN,
        &exact.work_id,
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","purpose":"Current switch receipt","current_focus":"accept exact fresh event","summary":"Host committed after current switched"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        output.status.success(),
        "a genuine Host event must remain successful when current changes after the authority snapshot: {}",
        output_text(&output)
    );
    server.recv();
    server.assert_no_additional_request();
    assert_eq!(load_projection(&fixture).id, FOREIGN_CURRENT_WORK_ID);
    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _home = gwt_core::test_support::ScopedGwtHome::set(fixture.home.path());
    let state_dir = fixture
        .home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(&project_root).as_str())
        .join("project-state");
    let works = load_workspace_work_items_from_path(&state_dir.join("works.json"))
        .expect("load WorkItems after current-switch Host update")
        .expect("WorkItems exist after current-switch Host update");
    assert_eq!(
        works
            .work_items
            .iter()
            .find(|work| work.id == exact.work_id)
            .expect("exact target Work")
            .events
            .iter()
            .filter(|event| {
                event.agent_session_id.as_deref() == Some(SESSION)
                    && event.summary.as_deref() == Some("Host committed after current switched")
            })
            .count(),
        1,
        "the current-switch outcome must be proven by one exact fresh Work event"
    );
    assert!(
        !state_dir.join("journal.jsonl").exists(),
        "the Host must not synthesize a legacy current journal entry for a foreign target"
    );
}

#[test]
fn workspace_update_foreign_work_rejects_static_receipt_without_fresh_event() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let project_root = fixture
        .project
        .path()
        .canonicalize()
        .expect("canonical project root");
    switch_current_projection_away_from_exact_work(
        fixture.home.path(),
        &project_root,
        &exact.work_id,
    );
    let state_dir = fixture
        .home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(&project_root).as_str())
        .join("project-state");
    fs::write(
        state_dir.join("journal.jsonl"),
        b"malformed current journal",
    )
    .expect("poison the irrelevant current journal");
    let before = workspace_delivery_state_snapshot(&fixture);
    let server = CaptureServer::start(
        StatusCode::OK,
        serde_json::json!({
            "schema_version": 1,
            "work_id": exact.work_id,
            "journal_entry_id": "static-foreign-receipt",
        })
        .to_string(),
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"static foreign receipt must fail"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "a foreign receipt without a fresh Work event must fail closed: {}",
        output_text(&output)
    );
    assert!(
        output_text(&output).contains("one exact new Work event"),
        "the refusal must prove that local authority validation was not skipped: {}",
        output_text(&output)
    );
    server.recv();
    server.assert_no_additional_request();
    assert_eq!(
        workspace_delivery_state_snapshot(&fixture),
        before,
        "a static foreign receipt must not cause a local replay"
    );
}

#[test]
fn workspace_update_foreign_work_rejects_preexisting_matching_event_as_stale() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let project_root = fixture
        .project
        .path()
        .canonicalize()
        .expect("canonical project root");
    switch_current_projection_away_from_exact_work(
        fixture.home.path(),
        &project_root,
        &exact.work_id,
    );
    let request = format!(
        r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","purpose":"Stale foreign event","current_focus":"reject pre-request event evidence","summary":"same foreign intent"}}}}"#
    );
    let seed_server =
        CaptureServer::real_host(fixture.home.path(), &project_root, SESSION, FORWARD_TOKEN);
    let seed_output = run_ws_raw_with_forward_env(
        &fixture,
        &request,
        SESSION,
        Some(&seed_server.forward_url),
        Some(FORWARD_TOKEN),
    );
    assert!(
        seed_output.status.success(),
        "the stale fixture must first persist one genuine foreign Work event: {}",
        output_text(&seed_output)
    );
    seed_server.recv();
    seed_server.assert_no_additional_request();
    let seed_response: Value =
        serde_json::from_slice(&seed_output.stdout).expect("seed foreign response JSON");
    let stale_receipt_id = seed_response["output"]
        .as_str()
        .and_then(|value| value.strip_prefix("workspace updated: "))
        .map(str::trim)
        .expect("seed foreign receipt id")
        .to_string();
    drop(seed_server);

    let before = workspace_delivery_state_snapshot(&fixture);
    let stale_server = CaptureServer::start(
        StatusCode::OK,
        serde_json::json!({
            "schema_version": 1,
            "work_id": exact.work_id,
            "journal_entry_id": stale_receipt_id,
        })
        .to_string(),
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &request,
        SESSION,
        Some(&stale_server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "a pre-request matching Work event must not prove a new Host mutation: {}",
        output_text(&output)
    );
    assert!(
        output_text(&output).contains("one exact new Work event"),
        "the refusal must distinguish stale event evidence: {}",
        output_text(&output)
    );
    stale_server.recv();
    stale_server.assert_no_additional_request();
    assert_eq!(
        workspace_delivery_state_snapshot(&fixture),
        before,
        "reusing a stale foreign event must leave mutation and settlement state byte-equivalent"
    );
}

#[test]
fn workspace_update_partial_forward_pair_fails_without_proxy_or_local_mutation() {
    for (case, include_url, include_token) in
        [("url-only", true, false), ("token-only", false, true)]
    {
        let fixture = fixture();
        register_agent(&fixture);
        let before = mutation_state_snapshot(&fixture);
        let server = CaptureServer::success();
        let output = run_ws_raw_with_forward_env(
            &fixture,
            &format!(
                r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"must not mutate through {case}"}}}}"#
            ),
            SESSION,
            include_url.then_some(server.forward_url.as_str()),
            include_token.then_some(FORWARD_TOKEN),
        );

        assert!(
            !output.status.success(),
            "partial forwarding configuration ({case}) must fail closed: {}",
            output_text(&output)
        );
        assert_secret_redacted(&output, FORWARD_TOKEN);
        server.assert_no_request();
        assert_eq!(
            mutation_state_snapshot(&fixture),
            before,
            "partial forwarding configuration ({case}) must not fall back to local mutation"
        );
    }
}

#[test]
fn workspace_update_managed_session_missing_forward_pair_never_uses_direct_mutation() {
    let fixture = fixture();
    register_agent(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let runtime_path = fixture
        .home
        .path()
        .join(".gwt/runtime/managed-session.json");
    let mut command = gwtd_command(&fixture, SESSION);
    command.env(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
    let mut child = command
        .spawn()
        .expect("run managed gwtd without bridge pair");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(
            format!(
                r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"must not use the standalone direct path"}}}}"#
            )
            .as_bytes(),
        )
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait managed gwtd");

    assert!(
        !output.status.success(),
        "a managed Session without its capability pair must fail closed: {}",
        output_text(&output)
    );
    let diagnostic = output_text(&output).to_ascii_lowercase();
    assert!(
        diagnostic.contains("bridge") && diagnostic.contains("relaunch"),
        "managed missing-pair diagnostic must require relaunch: {diagnostic}"
    );
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "managed missing-pair rejection must not fall back to direct mutation"
    );
}

#[test]
fn workspace_update_session_claim_mismatch_fails_before_proxy_or_local_mutation() {
    const FOREIGN_SESSION: &str = "foreign-explicit-session";

    let fixture = fixture();
    register_agent(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let server = CaptureServer::success();
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{FOREIGN_SESSION}","summary":"must be rejected"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "explicit/ambient Session mismatch must fail: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    server.assert_no_request();
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "Session claim mismatch must be rejected before every mutation"
    );
}

#[test]
fn workspace_update_unsafe_ambient_session_fails_before_proxy_or_ledger_lookup() {
    const UNSAFE_SESSION: &str = "../escaped-session";

    let fixture = fixture();
    let before = mutation_state_snapshot(&fixture);
    let server = CaptureServer::success();
    let output = run_ws_raw_with_forward_env(
        &fixture,
        r#"{"schema_version":1,"operation":"workspace.update","params":{"summary":"must be rejected"}}"#,
        UNSAFE_SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "unsafe ambient Session must fail: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    let diagnostic = output_text(&output).to_ascii_lowercase();
    assert!(
        diagnostic.contains("session")
            && (diagnostic.contains("unsafe") || diagnostic.contains("invalid")),
        "unsafe Session must be rejected at the identifier boundary, not looked up as a ledger path: {diagnostic}"
    );
    server.assert_no_request();
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "unsafe Session rejection must be zero-mutation"
    );
}

#[test]
fn workspace_update_proxy_transport_failure_never_falls_back_locally() {
    let fixture = fixture();
    prepare_exact_ensured_host(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let unreachable_url = reserve_unreachable_forward_url();
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"must remain unchanged"}}}}"#
        ),
        SESSION,
        Some(&unreachable_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "proxy transport failure must fail instead of using local state: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "transport failure must leave local projection/journal/events byte-equivalent"
    );
}

#[test]
fn workspace_update_proxy_response_loss_never_replays_locally_or_to_the_host() {
    let fixture = fixture();
    prepare_exact_ensured_host(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let mut server = DisconnectServer::start();
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"unknown Host outcome must not replay"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "response loss must remain an unknown outcome: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    server.receive();
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "response loss must not trigger a local compatibility continuation"
    );
}

#[test]
fn workspace_update_proxy_response_loss_after_host_apply_delivers_once() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let mut server = ApplyThenDisconnectServer::start(
        fixture.home.path(),
        fixture.project.path(),
        SESSION,
        FORWARD_TOKEN,
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"Host applied before response loss"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "response loss after Host apply must remain an unknown outcome: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    let observation = server.receive();
    assert_eq!(observation.work_id, exact.work_id);

    let events = fs::read_to_string(fixture.project.path().join(".gwt/work/events.jsonl"))
        .expect("tracked events after applied response loss")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<WorkEvent>(line).expect("tracked Work event JSON"))
        .collect::<Vec<_>>();
    let delivered = events
        .iter()
        .filter(|event| {
            event.work_item_id == exact.work_id
                && event.summary.as_deref() == Some("Host applied before response loss")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        delivered.len(),
        1,
        "the Host-applied mutation must append exactly one tracked Work event"
    );

    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _home = gwt_core::test_support::ScopedGwtHome::set(fixture.home.path());
    let journal_path =
        gwt_core::paths::gwt_workspace_journal_path_for_repo_path(fixture.project.path());
    let journal_entries =
        load_recent_workspace_journal_entries_from_path(&journal_path, usize::MAX)
            .expect("load journal after applied response loss");
    assert_eq!(
        journal_entries.len(),
        1,
        "the Host-applied mutation must create exactly one journal entry at {}; entries={journal_entries:?}",
        journal_path.display()
    );
    assert_eq!(journal_entries[0].id, observation.journal_entry_id);
    assert_eq!(
        journal_entries[0].agent_session_id.as_deref(),
        Some(SESSION)
    );
}

#[test]
fn workspace_update_proxy_non_success_never_falls_back_or_leaks_secret() {
    let fixture = fixture();
    register_agent(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let server = CaptureServer::start(
        StatusCode::CONFLICT,
        serde_json::json!({
            "code": "binding_conflict",
            "message": format!("host diagnostic must redact {FORWARD_TOKEN}"),
        })
        .to_string(),
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"must remain unchanged"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "non-success proxy response must fail instead of using local state: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    let captured = server.recv();
    assert!(
        captured.authorization == format!("Bearer {FORWARD_TOKEN}"),
        "workspace.update proxy request must use the configured bearer"
    );
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "non-success proxy response must leave local state byte-equivalent"
    );
}

#[test]
fn workspace_update_invalid_proxy_response_never_falls_back_locally() {
    let fixture = fixture();
    prepare_exact_ensured_host(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let server = CaptureServer::start(
        StatusCode::OK,
        r#"{"schema_version":2,"work_id":"foreign-work","journal_entry_id":"foreign-entry"}"#,
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"must remain unchanged"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "unknown proxy response schema must fail instead of using local state: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    let captured = server.recv();
    assert!(
        captured.authorization == format!("Bearer {FORWARD_TOKEN}"),
        "workspace.update proxy request must use the configured bearer"
    );
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "invalid proxy response must leave local state byte-equivalent"
    );
}

#[test]
fn workspace_update_mismatched_success_receipt_never_replays_or_mutates_locally() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let server = CaptureServer::start(
        StatusCode::OK,
        serde_json::json!({
            "schema_version": 1,
            "work_id": format!("foreign-{}", exact.work_id),
            "journal_entry_id": "foreign-success-receipt"
        })
        .to_string(),
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"mismatched success must fail"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "a success receipt for another Work must fail closed: {}",
        output_text(&output)
    );
    assert!(
        output_text(&output).contains("different Work"),
        "the Work identity check must be the refusal reason: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    server.recv();
    server.assert_no_additional_request();
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "a mismatched success receipt must never trigger local replay"
    );
}

#[test]
fn workspace_update_docker_rejects_success_receipt_for_another_work() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    mark_bound_session_as_docker(&fixture);
    let docker_ensure = run_ws(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.ensure","params":{{"agent_session":"{SESSION}","purpose":"Exact ensured Host fixture","current_focus":"eligible compatibility authority","issue":3412}}}}"#
        ),
    );
    assert_ok(&docker_ensure, "exact existing Docker workspace.ensure");
    assert!(
        docker_ensure["output"].as_str().is_some_and(
            |output| output.contains(&exact.work_id) && output.contains("already-assigned")
        ),
        "Docker receipt fixture must retain the canonical Work: {docker_ensure}"
    );
    let before = mutation_state_snapshot(&fixture);
    let server = CaptureServer::start(
        StatusCode::OK,
        serde_json::json!({
            "schema_version": 1,
            "work_id": format!("foreign-{}", exact.work_id),
            "journal_entry_id": "foreign-docker-success-receipt",
        })
        .to_string(),
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"Docker receipt must retain Work authority"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "Docker must reject a success receipt for another Work: {}",
        output_text(&output)
    );
    assert!(
        output_text(&output).contains("different Work"),
        "the Docker receipt must fail at the Work identity check: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    server.recv();
    server.assert_no_additional_request();
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "a foreign Docker receipt must not mutate local authority"
    );
}

#[test]
fn workspace_update_host_rejects_success_receipt_with_unknown_journal_entry() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let server = CaptureServer::start(
        StatusCode::OK,
        serde_json::json!({
            "schema_version": 1,
            "work_id": exact.work_id,
            "journal_entry_id": "unknown-host-journal-entry",
        })
        .to_string(),
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"Host receipt must prove its journal mutation"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "Host must reject a success receipt without the corresponding journal entry: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    server.recv();
    server.assert_no_additional_request();
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "an unproven Host journal receipt must not trigger local replay"
    );
}

#[test]
fn workspace_update_host_rejects_preexisting_matching_journal_receipt_as_stale() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let project_root = fixture
        .project
        .path()
        .canonicalize()
        .expect("canonical project root");
    let seed_server =
        CaptureServer::real_host(fixture.home.path(), &project_root, SESSION, FORWARD_TOKEN);
    let seed_output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"seed stale journal evidence"}}}}"#
        ),
        SESSION,
        Some(&seed_server.forward_url),
        Some(FORWARD_TOKEN),
    );
    assert!(
        seed_output.status.success(),
        "the stale fixture must first persist a genuine Host journal entry: {}",
        output_text(&seed_output)
    );
    seed_server.recv();
    seed_server.assert_no_additional_request();
    let seed_response: Value =
        serde_json::from_slice(&seed_output.stdout).expect("seed workspace response JSON");
    let stale_receipt_id = seed_response["output"]
        .as_str()
        .and_then(|value| value.strip_prefix("workspace updated: "))
        .map(str::trim)
        .expect("seed journal receipt id")
        .to_string();
    drop(seed_server);

    let before = workspace_delivery_state_snapshot(&fixture);
    let stale_server = CaptureServer::start(
        StatusCode::OK,
        serde_json::json!({
            "schema_version": 1,
            "work_id": exact.work_id,
            "journal_entry_id": stale_receipt_id,
        })
        .to_string(),
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"seed stale journal evidence"}}}}"#
        ),
        SESSION,
        Some(&stale_server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "a pre-request matching journal receipt must be rejected as stale: {}",
        output_text(&output)
    );
    assert!(
        output_text(&output).contains("stale journal receipt evidence"),
        "the refusal must distinguish stale evidence from an unknown receipt: {}",
        output_text(&output)
    );
    stale_server.recv();
    stale_server.assert_no_additional_request();
    assert_eq!(
        workspace_delivery_state_snapshot(&fixture),
        before,
        "a stale Host receipt must not cause local replay"
    );
}

#[test]
fn workspace_update_exact_ensure_required_rejection_uses_one_bound_continuation() {
    let fixture = fixture();
    let exact_identity = register_projectionless_bound_session(&fixture);
    let ensure = run_ws(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.ensure","params":{{"agent_session":"{SESSION}","purpose":"Typed compatibility continuation","current_focus":"ensure before old Host update","issue":3412}}}}"#
        ),
    );
    assert_ok(&ensure, "projectionless workspace.ensure");
    assert!(
        ensure["output"].as_str().is_some_and(
            |output| output.contains("workspace ensured:") && output.contains("created")
        ),
        "the positive fixture must pass through real durable Session bootstrap: {ensure}"
    );
    let events_after_first_ensure = fs::read(fixture.project.path().join(".gwt/work/events.jsonl"))
        .expect("tracked Start event after first ensure");
    let ensure_retry = run_ws(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.ensure","params":{{"agent_session":"{SESSION}","purpose":"Typed compatibility continuation","current_focus":"ensure before old Host update","issue":3412}}}}"#
        ),
    );
    assert_ok(
        &ensure_retry,
        "response-loss-idempotent workspace.ensure retry",
    );
    assert!(
        ensure_retry["output"]
            .as_str()
            .is_some_and(|output| output.contains("already-assigned")),
        "an ensure retry after a potentially lost response must reuse the assignment: {ensure_retry}"
    );
    assert_eq!(
        fs::read(fixture.project.path().join(".gwt/work/events.jsonl"))
            .expect("tracked event after ensure retry"),
        events_after_first_ensure,
        "response-loss retry must not duplicate the ensured Start event"
    );
    let project_root = fixture
        .project
        .path()
        .canonicalize()
        .expect("canonical project root");
    let ensured_projection = load_projection(&fixture);
    let canonical_work_id = ensured_projection
        .latest_agent_for_session(SESSION)
        .and_then(|agent| agent.workspace_id.clone())
        .expect("workspace.ensure canonical Work assignment");
    let events_path = project_root.join(".gwt/work/events.jsonl");
    let events_before = fs::read_to_string(&events_path).expect("tracked events before");
    let server = CaptureServer::start(
        StatusCode::CONFLICT,
        serde_json::json!({
            "code": "workspace_ensure_required",
            "reason": "workspace_ensure_required",
            "message": "old Host still resolves WorkItems from the exact-worktree scope"
        })
        .to_string(),
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","status":"done","summary":"typed compatibility continuation"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        output.status.success(),
        "the exact typed pre-mutation rejection must reach one bound continuation: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    let captured = server.recv();
    server.assert_no_additional_request();
    assert_eq!(captured.authorization, format!("Bearer {FORWARD_TOKEN}"));
    assert_eq!(
        captured.body["claimed_session_id"],
        Value::String(SESSION.to_string())
    );
    assert_eq!(captured.body["intent"]["status_category"], "done");

    let projection = load_projection(&fixture);
    assert!(
        projection.latest_agent_for_session(SESSION).is_some(),
        "bound Session assignment must survive the continuation"
    );
    let state_dir = fixture
        .home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(&project_root).as_str())
        .join("project-state");
    let work_items = load_workspace_work_items_from_path(&state_dir.join("works.json"))
        .expect("load WorkItems after continuation")
        .expect("WorkItems exist after continuation");
    let work = work_items
        .work_items
        .iter()
        .find(|work| work.id == canonical_work_id)
        .expect("bound Work after continuation");
    assert_eq!(work.status_category, WorkspaceStatusCategory::Done);
    assert_eq!(
        work.summary.as_deref(),
        Some("typed compatibility continuation")
    );

    let events_after = fs::read_to_string(&events_path).expect("tracked events after");
    assert!(events_after.starts_with(&events_before));
    let events = events_after
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<WorkEvent>(line).expect("tracked event JSON"))
        .collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == WorkEventKind::Start)
            .count(),
        1,
        "compatibility continuation must not duplicate Start delivery"
    );
    assert_eq!(events.len(), 2, "the original request must be applied once");
    assert_eq!(events[1].kind, WorkEventKind::Done);
    assert_eq!(events[1].work_item_id, canonical_work_id);
    assert_eq!(events[1].agent_session_id.as_deref(), Some(SESSION));
    assert_eq!(
        events[1].status_category,
        Some(WorkspaceStatusCategory::Done)
    );

    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _home = gwt_core::test_support::ScopedGwtHome::set(fixture.home.path());
    let settlement =
        gwt::cli::verification_record::load_work_event_settlement_record(fixture.project.path())
            .expect("load settlement record")
            .expect("Done continuation must reserve settlement");
    assert_eq!(settlement.session_id, SESSION);
    assert!(
        settlement.obligation_open,
        "the terminal compatibility continuation must retain its delivery obligation"
    );
    assert_eq!(
        settlement.execution_binding.as_ref(),
        Some(&exact_identity.execution_binding.identity)
    );
    let journal_path = gwt_core::paths::gwt_workspace_journal_path_for_repo_path(&project_root);
    let session_journal_entries =
        load_recent_workspace_journal_entries_from_path(&journal_path, usize::MAX)
            .expect("load compatibility continuation journal")
            .into_iter()
            .filter(|entry| entry.agent_session_id.as_deref() == Some(SESSION))
            .collect::<Vec<_>>();
    assert_eq!(
        session_journal_entries.len(),
        1,
        "the compatibility continuation must create exactly one Session journal entry"
    );
    let journal_entry = &session_journal_entries[0];
    let response: Value = serde_json::from_slice(&output.stdout).expect("gwtd response JSON");
    let receipt_journal_entry_id = response["output"]
        .as_str()
        .and_then(|value| value.strip_prefix("workspace updated: "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("compatibility continuation receipt journal id");
    assert_eq!(receipt_journal_entry_id, journal_entry.id);
    assert_eq!(
        journal_entry.status_category,
        Some(WorkspaceStatusCategory::Done)
    );
    assert_eq!(journal_entry.agent_session_id.as_deref(), Some(SESSION));
}

#[test]
fn workspace_update_ensure_required_lookalikes_never_replay_locally() {
    let cases = [
        (
            "reason-mismatch",
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "workspace_ensure_required",
                "reason": "authority_mismatch",
                "message": "not the exact pre-mutation outcome"
            })
            .to_string(),
        ),
        (
            "code-mismatch",
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "identity_conflict",
                "reason": "workspace_ensure_required",
                "message": "not the exact pre-mutation outcome"
            })
            .to_string(),
        ),
        (
            "reason-missing",
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "workspace_ensure_required",
                "message": "reason is required"
            })
            .to_string(),
        ),
        (
            "unknown-field",
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "workspace_ensure_required",
                "reason": "workspace_ensure_required",
                "message": "known fields otherwise match",
                "mutation_state": "unknown"
            })
            .to_string(),
        ),
        (
            "malformed-json",
            StatusCode::CONFLICT,
            "{not-json".to_string(),
        ),
        (
            "wrong-status",
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({
                "code": "workspace_ensure_required",
                "reason": "workspace_ensure_required",
                "message": "status is not the proven pre-mutation refusal"
            })
            .to_string(),
        ),
    ];

    for (case, status, body) in cases {
        let fixture = fixture();
        prepare_exact_ensured_host(&fixture);
        let before = mutation_state_snapshot(&fixture);
        let server = CaptureServer::start(status, body);
        let output = run_ws_raw_with_forward_env(
            &fixture,
            &format!(
                r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"must not replay {case}"}}}}"#
            ),
            SESSION,
            Some(&server.forward_url),
            Some(FORWARD_TOKEN),
        );

        assert!(
            !output.status.success(),
            "{case}: a non-exact Host outcome must fail closed: {}",
            output_text(&output)
        );
        assert_secret_redacted(&output, FORWARD_TOKEN);
        if case == "unknown-field" {
            let diagnostic = output_text(&output);
            assert!(
                diagnostic.contains("code=workspace_ensure_required")
                    && diagnostic.contains("bridge_reason=workspace_ensure_required"),
                "rolling-version fields must not erase the preserved machine-readable rejection: {diagnostic}"
            );
        }
        server.recv();
        server.assert_no_additional_request();
        assert_eq!(
            mutation_state_snapshot(&fixture),
            before,
            "{case}: a non-exact Host outcome must not mutate local authority"
        );
    }
}

#[test]
fn workspace_update_without_exact_ensure_fails_before_host_contact() {
    let fixture = fixture();
    register_bound_agent(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let server = CaptureServer::start(
        StatusCode::CONFLICT,
        serde_json::json!({
            "code": "workspace_ensure_required",
            "reason": "workspace_ensure_required",
            "message": "typed refusal without a canonical ensured assignment"
        })
        .to_string(),
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"must require exact ensure authority"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "an unensured authority must fail before Host contact: {}",
        output_text(&output)
    );
    assert!(
        output_text(&output).contains("workspace_ensure_required"),
        "the preflight failure must preserve the machine-readable recovery action: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    server.assert_no_request();
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "an unensured authority must not mutate local state"
    );
}

#[test]
fn workspace_update_exact_rejection_revalidates_the_pre_request_session_identity() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let before = workspace_delivery_state_snapshot(&fixture);
    let server = CaptureServer::start_with_capability_rotation(
        StatusCode::CONFLICT,
        serde_json::json!({
            "code": "workspace_ensure_required",
            "reason": "workspace_ensure_required",
            "message": "rotate the durable capability before returning the typed refusal"
        })
        .to_string(),
        fixture.home.path(),
        SESSION,
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"stale snapshot must not mutate"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "a Session identity changed during the bridge call must fail closed: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    server.recv();
    server.assert_no_additional_request();
    assert_eq!(
        workspace_delivery_state_snapshot(&fixture),
        before,
        "capability rotation may change the Session ledger but no Work delivery surface"
    );
    let rotated = Session::load(
        &fixture
            .home
            .path()
            .join(".gwt/sessions")
            .join(format!("{SESSION}.toml")),
    )
    .expect("load rotated Session fixture");
    assert_eq!(
        rotated
            .execution_binding
            .as_ref()
            .expect("rotated execution binding")
            .capability_generation,
        exact.identity.execution_binding.capability_generation + 1,
        "the server hook must prove the exact pre-request identity became stale"
    );
}

#[test]
fn workspace_update_exact_rejection_rejects_host_to_docker_change_during_bridge() {
    let fixture = fixture();
    prepare_exact_ensured_host(&fixture);
    let before = workspace_delivery_state_snapshot(&fixture);
    let server = CaptureServer::start_with_docker_switch(
        StatusCode::CONFLICT,
        serde_json::json!({
            "code": "workspace_ensure_required",
            "reason": "workspace_ensure_required",
            "message": "switch the Session to Docker before returning the typed refusal",
        })
        .to_string(),
        fixture.home.path(),
        fixture.project.path(),
        SESSION,
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","status":"done","summary":"stale Host runtime must not mutate"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "a Session switched to Docker during the bridge call must fail closed: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    server.recv();
    server.assert_no_additional_request();
    assert_eq!(
        workspace_delivery_state_snapshot(&fixture),
        before,
        "the Docker switch may change the Session ledger but no Work delivery surface"
    );
    let switched = Session::load(
        &fixture
            .home
            .path()
            .join(".gwt/sessions")
            .join(format!("{SESSION}.toml")),
    )
    .expect("load Docker-switched Session fixture");
    assert_eq!(
        switched.runtime_target,
        gwt_agent::LaunchRuntimeTarget::Docker,
        "the response hook must prove Host authority became stale"
    );
}

#[test]
fn workspace_update_never_follows_redirect_to_exact_rejection() {
    let fixture = fixture();
    prepare_exact_ensured_host(&fixture);
    let before = mutation_state_snapshot(&fixture);
    let server = RedirectServer::start();
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","status":"done","summary":"redirected refusal must not authorize continuation"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    let captured = server.recv_source();
    assert_eq!(captured.authorization, format!("Bearer {FORWARD_TOKEN}"));
    server.assert_no_redirect_request();
    assert!(
        !output.status.success(),
        "a redirect response must fail before reaching its target: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "a redirected exact refusal must not authorize local mutation"
    );
}

#[test]
fn workspace_update_exact_rejection_rejects_duplicate_session_projection_rows() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    let project_root = fixture
        .project
        .path()
        .canonicalize()
        .expect("canonical project root");
    let events_path = project_root.join(".gwt/work/events.jsonl");
    let events_before = fs::read(&events_path).expect("tracked events before duplicate");
    let settlement_before = work_event_settlement_state_snapshot(&fixture);
    let server = CaptureServer::start_with_projection_duplicate(
        StatusCode::CONFLICT,
        serde_json::json!({
            "code": "workspace_ensure_required",
            "reason": "workspace_ensure_required",
            "message": "duplicate the same-authority Session row before returning the refusal",
        })
        .to_string(),
        fixture.home.path(),
        &project_root,
        SESSION,
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","status":"done","summary":"ambiguous projection must not mutate"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "duplicate Session projection rows must invalidate the continuation lease: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    server.recv();
    server.assert_no_additional_request();
    assert_eq!(
        fs::read(&events_path).expect("tracked events after duplicate"),
        events_before,
        "an ambiguous projection must not append a Done event"
    );
    assert_eq!(
        work_event_settlement_state_snapshot(&fixture),
        settlement_before,
        "an ambiguous projection must not reserve an orphan terminal settlement"
    );
    let state_dir = fixture
        .home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(&project_root).as_str())
        .join("project-state");
    let work_items = load_workspace_work_items_from_path(&state_dir.join("works.json"))
        .expect("load WorkItems after duplicate rejection")
        .expect("WorkItems exist after duplicate rejection");
    let work = work_items
        .work_items
        .iter()
        .find(|work| work.id == exact.work_id)
        .expect("canonical Work after duplicate rejection");
    assert_ne!(work.status_category, WorkspaceStatusCategory::Done);
}

#[test]
fn workspace_update_exact_ensure_required_rejection_never_continues_for_docker() {
    let fixture = fixture();
    let exact = prepare_exact_ensured_host(&fixture);
    mark_bound_session_as_docker(&fixture);
    let docker_ensure = run_ws(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.ensure","params":{{"agent_session":"{SESSION}","purpose":"Exact ensured Host fixture","current_focus":"eligible compatibility authority","issue":3412}}}}"#
        ),
    );
    assert_ok(&docker_ensure, "exact existing Docker workspace.ensure");
    assert!(
        docker_ensure["output"].as_str().is_some_and(
            |output| output.contains(&exact.work_id) && output.contains("already-assigned")
        ),
        "Docker negative must retain an otherwise exact existing Work: {docker_ensure}"
    );
    let before = mutation_state_snapshot(&fixture);
    let server = CaptureServer::start(
        StatusCode::CONFLICT,
        serde_json::json!({
            "code": "workspace_ensure_required",
            "reason": "workspace_ensure_required",
            "message": "Docker authority stays Host-only"
        })
        .to_string(),
    );
    let output = run_ws_raw_with_forward_env(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","summary":"must not continue for Docker"}}}}"#
        ),
        SESSION,
        Some(&server.forward_url),
        Some(FORWARD_TOKEN),
    );

    assert!(
        !output.status.success(),
        "Docker must not enter the Host compatibility continuation: {}",
        output_text(&output)
    );
    assert_secret_redacted(&output, FORWARD_TOKEN);
    server.recv();
    server.assert_no_additional_request();
    assert_eq!(
        mutation_state_snapshot(&fixture),
        before,
        "Docker typed rejection must remain byte-identical"
    );
}
