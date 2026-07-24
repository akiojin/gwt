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
    io::Write,
    net::TcpListener as StdTcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use axum::{
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
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
        append_workspace_work_event_to_path, load_workspace_projection_from_path,
        load_workspace_work_items_from_path, save_workspace_projection_to_path,
        save_workspace_work_items_projection_to_path, WorkEvent, WorkEventApplyOutcome,
        WorkEventKind, WorkItemsProjection, WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary,
        WorkspaceExecutionContainerRef, WorkspaceProjection, WorkspaceStatusCategory,
    },
};
use serde_json::Value;
use tempfile::TempDir;
use tokio::{net::TcpListener, runtime::Runtime, sync::oneshot};

const SESSION: &str = "ws-cli-session";
const BRANCH: &str = "work/ws-cli";
const WORK_ID: &str = "existing-similar-work";
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
}

#[derive(Clone)]
struct RealHostWorkspaceUpdate {
    home: PathBuf,
    project_root: PathBuf,
    session_id: String,
    bearer_token: String,
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
        Self::start_with_real_host(response_status, response_body, None)
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
        )
    }

    fn start_with_real_host(
        response_status: StatusCode,
        response_body: impl Into<String>,
        real_host: Option<RealHostWorkspaceUpdate>,
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

/// Seed the complete Session-bound mutation target without invoking the
/// `workspace.update` path under test or relying on default synthesis.
fn register_agent(fixture: &Fixture) {
    register_agent_at_home(fixture.home.path(), fixture.project.path());
}

fn register_agent_at_home(home: &Path, project: &Path) {
    let project_root = project.canonicalize().expect("canonical project root");
    let mut session = Session::new(&project_root, BRANCH, AgentId::Codex);
    session.id = SESSION.to_string();
    session.project_state_root = Some(project_root.clone());
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
    register_agent(&fixture);
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
    register_agent(&fixture);
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
